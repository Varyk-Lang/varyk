//! Turning a parsed file's top-level items into [`ImportedFn`]s,
//! [`ImportedStruct`]s, and [`ImportedEnum`]s, and rejecting what the
//! generated crate cannot satisfy.

use std::collections::HashSet;
use std::ops::Range;

use quote::ToTokens;
use syn::{
    Fields, FnArg, ImplItem, Item, ItemImpl, ReceiverKind, ReturnType, Safety, Signature, UseTree,
    Visibility,
};

use super::signatures::{Names, map_param_type, map_return_type, map_value};
use super::{
    CFG, CFG_PARAM, FieldVis, ImportError, ImportedEnum, ImportedField, ImportedFn, ImportedModule,
    ImportedStruct, ImportedVariant, REEXPORT, RustPath, RustTy, SelfMode, StructKind, TEST,
    ident_name, tidy_signature,
};

/// Builds one [`ImportedFn`] per top-level free `pub fn` item in `items`
/// (plain `pub`, not `pub(crate)`/`pub(super)`/`pub(in ...)`), and one
/// [`ImportedStruct`] per top-level plain-`pub` struct with the `pub fn`
/// items of its inherent `impl` blocks in this file (spec 4.1, 4.2), and
/// one [`ImportedEnum`] per top-level plain-`pub` enum (spec 4.3),
/// skipping `unsafe`/`async`/`const`/`extern` fns, `pub use` re-exports,
/// and `#[cfg(...)]`-gated items, each recorded with why so that naming
/// it can say so. `text` is the file's source, for byte ranges.
pub(super) fn import_items(items: Vec<Item>, names: &Names, text: &str) -> ImportedModule {
    let mut module = ImportedModule::default();
    let mut impls = Vec::new();
    for item in items {
        match item {
            Item::Fn(item_fn) => {
                let name = ident_name(&item_fn.sig.ident);
                let cfg = cfg_reason(&item_fn.attrs, &item_fn.sig);
                if cfg.is_some() || !is_plain_pub(&item_fn.vis) {
                    if let (Some(why), true) = (cfg, is_plain_pub(&item_fn.vis)) {
                        module.skipped_fns.push((name, "function", why));
                    } else if let (Some(vis), None) = (restricted(&item_fn.vis), cfg) {
                        module.restricted_fns.push((name, vis));
                    }
                    continue;
                }
                match why_not_imported(&item_fn.sig) {
                    Some(why) => module.skipped_fns.push((name, "function", why)),
                    None => module.fns.extend(import_fn(&item_fn.sig, None, names)),
                }
            }
            Item::Use(item) if is_plain_pub(&item.vis) => {
                let mut names = HashSet::new();
                let mut glob = false;
                super::collect_use_names(&item.tree, None, &mut names, &mut glob);
                let mut names: Vec<String> = names.into_iter().collect();
                names.sort();
                for name in names {
                    module.skipped_fns.push((name, "item", REEXPORT));
                }
            }
            Item::Struct(item) if has_cfg_attr(&item.attrs) => {
                if is_plain_pub(&item.vis) {
                    module.cfg_types.push(("struct", ident_name(&item.ident)));
                }
            }
            Item::Enum(item) if has_cfg_attr(&item.attrs) => {
                if is_plain_pub(&item.vis) {
                    module.cfg_types.push(("enum", ident_name(&item.ident)));
                }
            }
            Item::Struct(item) => {
                if let Some(vis) = restricted(&item.vis) {
                    let name = ident_name(&item.ident);
                    module.restricted_types.push(("struct", name, vis));
                    continue;
                }
                if !is_plain_pub(&item.vis) {
                    continue;
                }
                let generics = &item.generics;
                let (kind, fields) = match &item.fields {
                    Fields::Named(fields) => (
                        StructKind::Named,
                        fields
                            .named
                            .iter()
                            .filter_map(|field| {
                                let ident = field.ident.as_ref()?;
                                // A `#[cfg]`-gated field may or may not
                                // exist: hidden, so no literal names it or
                                // leaves it out.
                                let public =
                                    is_plain_pub(&field.vis) && !has_cfg_attr(&field.attrs);
                                Some(ImportedField {
                                    name: ident_name(ident),
                                    ty: map_value(&field.ty, names),
                                    vis: if public {
                                        FieldVis::Pub
                                    } else {
                                        FieldVis::Hidden
                                    },
                                    span: name_range(ident, text),
                                })
                            })
                            .collect(),
                    ),
                    Fields::Unnamed(_) => (StructKind::Tuple, Vec::new()),
                    Fields::Unit => (StructKind::Unit, Vec::new()),
                };
                module.structs.push(ImportedStruct {
                    name: ident_name(&item.ident),
                    fields,
                    methods: Vec::new(),
                    skipped_methods: Vec::new(),
                    restricted_methods: Vec::new(),
                    generic: !generics.params.is_empty() || generics.where_clause.is_some(),
                    unfit: unfit_struct(&item),
                    kind,
                    span: name_range(&item.ident, text),
                });
            }
            Item::Enum(item) => {
                if let Some(vis) = restricted(&item.vis) {
                    let name = ident_name(&item.ident);
                    module.restricted_types.push(("enum", name, vis));
                    continue;
                }
                if !is_plain_pub(&item.vis) {
                    continue;
                }
                let generics = &item.generics;
                let mut opaque = None;
                let variants = item
                    .variants
                    .iter()
                    .map(|variant| {
                        let name = ident_name(&variant.ident);
                        // A `#[cfg]`-gated variant may or may not exist.
                        if has_cfg_attr(&variant.attrs) {
                            opaque.get_or_insert_with(|| {
                                format!(
                                    "has a variant, `{name}`, behind `#[cfg]` that may not \
                                     exist in the build"
                                )
                            });
                        }
                        let payload = match &variant.fields {
                            Fields::Named(_) => {
                                opaque.get_or_insert_with(|| named_field_variant_reason(&name));
                                Vec::new()
                            }
                            Fields::Unnamed(fields) => {
                                // A `#[cfg]`-gated field may or may not
                                // exist, and with it the payload's shape.
                                if fields
                                    .unnamed
                                    .iter()
                                    .any(|field| has_cfg_attr(&field.attrs))
                                {
                                    opaque.get_or_insert_with(|| {
                                        format!(
                                            "has a variant, `{name}`, with a field behind \
                                             `#[cfg]` that may not exist in the build"
                                        )
                                    });
                                }
                                let payload: Vec<RustTy> = fields
                                    .unnamed
                                    .iter()
                                    .map(|field| map_value(&field.ty, names))
                                    .collect();
                                if let Some(bad) =
                                    payload.iter().find(|ty| unmapped_at_import_time(ty))
                                {
                                    opaque
                                        .get_or_insert_with(|| unmapped_payload_reason(&name, bad));
                                }
                                payload
                            }
                            Fields::Unit => Vec::new(),
                        };
                        ImportedVariant { name, payload }
                    })
                    .collect();
                module.enums.push(ImportedEnum {
                    name: ident_name(&item.ident),
                    variants,
                    generic: !generics.params.is_empty() || generics.where_clause.is_some(),
                    opaque,
                    span: name_range(&item.ident, text),
                    methods: Vec::new(),
                });
            }
            Item::Impl(item) => impls.push(item),
            Item::Mod(item) if item.content.is_some() => {
                module.inline_mods.push(ident_name(&item.ident));
            }
            _ => {}
        }
    }
    // A `#[cfg]`-gated struct or enum may stand in for one of the same
    // name that is not gated; only a name nothing else takes is noted.
    let declared: HashSet<String> = (module.structs.iter().map(|s| s.name.clone()))
        .chain(module.enums.iter().map(|e| e.name.clone()))
        .chain(
            module
                .restricted_types
                .iter()
                .map(|(_, name, _)| name.clone()),
        )
        .collect();
    module
        .cfg_types
        .retain(|(_, name)| !declared.contains(name));
    let fns: HashSet<String> = (module.fns.iter().map(|f| f.name.clone()))
        .chain(module.restricted_fns.iter().map(|(name, _)| name.clone()))
        .collect();
    module.skipped_fns.retain(|(name, ..)| !fns.contains(name));
    // Every inherent `impl S` block of the file, in source order, adds its
    // `pub fn` items to `S` (spec 4.2: several blocks merge).
    // A method left out for a reason a caller could not guess (a trait
    // method, `unsafe`, `const`, `async`, `#[cfg]`) is recorded with it.
    for item in impls {
        let Some(owner) = inherent_owner(&item) else {
            if let Some(owner) = trait_impl_owner(&item) {
                if let Some(target) = module.structs.iter_mut().find(|s| s.name == owner) {
                    for impl_item in &item.items {
                        if let ImplItem::Fn(method) = impl_item {
                            let name = ident_name(&method.sig.ident);
                            target.skipped_methods.push((name, "a trait method"));
                        }
                    }
                }
            }
            continue;
        };
        if let Some(target) = module.enums.iter_mut().find(|e| e.name == owner) {
            target
                .methods
                .extend(item.items.iter().filter_map(|impl_item| match impl_item {
                    ImplItem::Fn(method) => Some(ident_name(&method.sig.ident)),
                    _ => None,
                }));
            continue;
        }
        let Some(target) = module.structs.iter_mut().find(|s| s.name == owner) else {
            continue;
        };
        let names = names.in_impl(&owner);
        let block_cfg = has_cfg_attr(&item.attrs);
        for impl_item in &item.items {
            let ImplItem::Fn(method) = impl_item else {
                continue;
            };
            if !is_plain_pub(&method.vis) {
                if let (Some(vis), false) = (restricted(&method.vis), block_cfg) {
                    if !has_cfg_attr(&method.attrs) {
                        let name = ident_name(&method.sig.ident);
                        target.restricted_methods.push((name, vis));
                    }
                }
                continue;
            }
            let cfg = if block_cfg {
                Some(CFG)
            } else {
                cfg_reason(&method.attrs, &method.sig)
            };
            if let Some(why) = cfg {
                let name = ident_name(&method.sig.ident);
                target.skipped_methods.push((name, why));
            } else if let Some(why) = why_not_imported(&method.sig) {
                let name = ident_name(&method.sig.ident);
                target.skipped_methods.push((name, why));
            } else {
                target
                    .methods
                    .extend(import_fn(&method.sig, Some(&owner), &names));
            }
        }
    }
    module
}

/// The struct a trait `impl` block is for, by the last segment of its
/// type's path (`A`, `crate::ext::A`, `self::A`), matched against this
/// file's structs by the caller: its methods are never imported, only
/// recorded as skipped, so a note can say why.
fn trait_impl_owner(item: &ItemImpl) -> Option<String> {
    item.trait_.as_ref()?;
    let syn::Type::Path(path) = item.self_ty.as_ref() else {
        return None;
    };
    if path.qself.is_some() {
        return None;
    }
    let last = path.path.segments.last()?;
    matches!(last.arguments, syn::PathArguments::None).then(|| ident_name(&last.ident))
}

/// Why a struct cannot be a Varyk struct whatever its fields: Varyk
/// borrows fields, which rustc refuses for a `#[repr(packed)]` struct
/// (E0793), and holds values in locals and `Vec`s, which needs a fixed
/// size (E0277) that a struct whose last field is a slice, `str`, or a
/// trait object does not have.
fn unfit_struct(item: &syn::ItemStruct) -> Option<&'static str> {
    let packed = item.attrs.iter().any(|attr| {
        super::path_is(attr.path(), "repr")
            && attr.meta.require_list().is_ok_and(|list| {
                list.tokens
                    .clone()
                    .into_iter()
                    .any(|tree| matches!(tree, proc_macro2::TokenTree::Ident(i) if i == "packed"))
            })
    });
    if packed {
        return Some("it is `#[repr(packed)]`, so its fields cannot be borrowed");
    }
    let last = item.fields.iter().next_back()?;
    let unsized_ = match &last.ty {
        syn::Type::Slice(_) | syn::Type::TraitObject(_) => true,
        syn::Type::Path(path) => path.qself.is_none() && path.path.is_ident("str"),
        _ => false,
    };
    unsized_.then_some("it has no fixed size: its last field is a slice, `str`, or `dyn` type")
}

/// Why a signature is not imported at all, in words for a note: `unsafe`,
/// `const`, `async`, or `extern`; `None` when it is.
fn why_not_imported(sig: &Signature) -> Option<&'static str> {
    if !matches!(sig.safety, Safety::Default) {
        Some("`unsafe`")
    } else if sig.constness.is_some() {
        Some("`const`")
    } else if sig.asyncness.is_some() {
        Some("`async`")
    } else if sig.abi.is_some() {
        Some("`extern`")
    } else {
        None
    }
}

/// The struct an `impl` block is for when it is a plain inherent one:
/// `impl S { .. }` with no trait, no generics, no `unsafe`, and `S` a
/// bare name. A `#[cfg]` on the block is the caller's to check.
fn inherent_owner(item: &ItemImpl) -> Option<String> {
    if item.trait_.is_some()
        || item.unsafety.is_some()
        || !item.generics.params.is_empty()
        || item.generics.where_clause.is_some()
        || item.modifiers.defaultness.is_some()
        || item.modifiers.polarity.is_some()
    {
        return None;
    }
    let syn::Type::Path(path) = item.self_ty.as_ref() else {
        return None;
    };
    if path.qself.is_some() {
        return None;
    }
    path.path.get_ident().map(ident_name)
}

/// The [`ImportedFn`] of signature `sig`, a method or associated function
/// when `owner` is its struct; `None` for an `unsafe`, `async`, `const`,
/// or `extern` fn, which is not imported at all.
fn import_fn(sig: &Signature, owner: Option<&str>, names: &Names) -> Option<ImportedFn> {
    if why_not_imported(sig).is_some() {
        return None;
    }
    let mut receiver = None;
    let mut params = Vec::new();
    for arg in &sig.inputs {
        match arg {
            FnArg::Typed(pat_type) => params.push(map_param_type(&pat_type.ty, names)),
            // Only `&self` and `&mut self` map (spec 4.2); any other
            // receiver is kept as an opaque first parameter, so the
            // method is never callable.
            FnArg::Receiver(found) => match &found.kind {
                ReceiverKind::Reference(_, None, mutability) => {
                    receiver = Some(if mutability.is_some() {
                        SelfMode::Mutable
                    } else {
                        SelfMode::Shared
                    });
                }
                _ => params.insert(0, RustTy::Opaque(found.to_token_stream().to_string())),
            },
        }
    }
    let signature = sig.to_token_stream().to_string();
    let generics = &sig.generics;
    let ret = if !generics.params.is_empty() || generics.where_clause.is_some() {
        // Varyk cannot name a type argument, so a generic function is
        // never callable; an opaque return type says so.
        RustTy::Opaque(signature.clone())
    } else {
        match &sig.output {
            ReturnType::Default => RustTy::Unit,
            ReturnType::Type(_, ty) => map_return_type(ty, names),
        }
    };
    Some(ImportedFn {
        name: ident_name(&sig.ident),
        params,
        ret,
        receiver,
        owner: owner.map(str::to_string),
        signature,
    })
}

/// Why an enum with a named-field variant is opaque (spec 4.3): Varyk
/// imports only unit and tuple variants.
fn named_field_variant_reason(variant: &str) -> String {
    format!("has a variant, `{variant}`, with named fields, which Varyk does not import yet")
}

/// Whether a variant's payload type is already known to be unmappable
/// from this file alone (spec 4.3, 4.4), regardless of what other
/// modules turn out to import: a bare shape Varyk never maps (a
/// `HashMap`, a tuple, a reference, a generic parameter, ...), or a type
/// reached through a `use` line, which Varyk does not follow. A bare
/// name of this file or a `crate::` path may still turn out unmappable,
/// but only once every module's imports are known; see
/// `resolve::imports::register`.
fn unmapped_at_import_time(ty: &RustTy) -> bool {
    matches!(
        ty,
        RustTy::Opaque(_) | RustTy::Named(RustPath::Used(_) | RustPath::Glob(_))
    )
}

/// Why a variant's payload makes its enum opaque, from what is already
/// known at import time (spec 4.3).
fn unmapped_payload_reason(variant: &str, ty: &RustTy) -> String {
    if let RustTy::Named(RustPath::Used(name)) = ty {
        return format!(
            "has a variant, `{variant}`, holding `{name}`, which a `use` line of the Rust file \
             brings in and Varyk does not follow (write the full path, as in \
             `crate::module::{name}`)"
        );
    }
    if let RustTy::Named(RustPath::Glob(name)) = ty {
        return format!(
            "has a variant, `{variant}`, holding `{name}`, which the file's glob `use` \
             (`use ...::*`) could bring in (replace the glob with the names the file needs)"
        );
    }
    format!(
        "has a variant, `{variant}`, holding `{}`, which Varyk cannot use",
        tidy_signature(&ty.text())
    )
}

/// Why a bare `name` in a file with a glob `use` is not mapped (spec 4.4).
pub(crate) fn glob_reason(name: &str) -> String {
    format!(
        "this Rust file has a glob `use` (`use ...::*`), which could bring in its own \
         `{name}`; replace the glob with the names the file needs"
    )
}

/// The byte range of `ident` in `text`, or an empty range at the start
/// when the parser's positions do not line up with `text` (a leading
/// byte-order mark or `#!` line shifts them).
pub(super) fn name_range(ident: &proc_macro2::Ident, text: &str) -> Range<usize> {
    let range = ident.span().byte_range();
    let name = ident.to_string();
    if text.get(range.clone()) == Some(name.as_str()) {
        range
    } else {
        0..0
    }
}

/// A restricted visibility as written (`pub(crate)`, `pub(in crate::a)`);
/// `None` for plain `pub` and for private.
fn restricted(vis: &Visibility) -> Option<String> {
    let Visibility::Restricted(vis) = vis else {
        return None;
    };
    let path = vis.path.to_token_stream().to_string().replace(' ', "");
    Some(match vis.in_token {
        Some(_) => format!("pub(in {path})"),
        None => format!("pub({path})"),
    })
}

/// True for plain `pub`; false for private, `pub(crate)`, `pub(super)`,
/// and `pub(in ...)`.
fn is_plain_pub(vis: &Visibility) -> bool {
    matches!(vis, Visibility::Public(_))
}

/// True when any attribute is `#[cfg(...)]`, `#[cfg_attr(...)]`, or
/// `#[test]` (a bare `#[cfg]`/`#[cfg_attr]` attribute is vanishingly rare
/// and not worth special-casing; matching the path covers the general form
/// found in practice, including `#[cfg(test)]` and
/// `#[cfg_attr(test, ...)]`; `#[test]` is skipped so test functions are
/// never imported as callable).
fn has_cfg_attr(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|attr| {
        super::path_is(attr.path(), "cfg")
            || super::path_is(attr.path(), "cfg_attr")
            || super::path_is(attr.path(), "test")
    })
}

/// Why a function or method may not exist, or not with this shape, in
/// the build: [`CFG`] for a `#[cfg]`/`#[cfg_attr]` on it, [`TEST`] for
/// `#[test]`, [`CFG_PARAM`] for one on a parameter or the receiver;
/// `None` when it has none of these.
fn cfg_reason(attrs: &[syn::Attribute], sig: &Signature) -> Option<&'static str> {
    if attrs.iter().any(|attr| attr.path().is_ident("test")) {
        Some(TEST)
    } else if has_cfg_attr(attrs) {
        Some(CFG)
    } else if inputs_have_cfg(sig) {
        Some(CFG_PARAM)
    } else {
        None
    }
}

/// True when a parameter or the receiver of `sig` has a `#[cfg]` (see
/// [`has_cfg_attr`]): the function's parameter list may differ in the
/// build, so it is not imported.
fn inputs_have_cfg(sig: &Signature) -> bool {
    sig.inputs.iter().any(|input| match input {
        FnArg::Typed(param) => has_cfg_attr(&param.attrs),
        FnArg::Receiver(receiver) => has_cfg_attr(&receiver.attrs),
    })
}

/// Rejects, anywhere in `items` (recursing into inline modules), what the
/// generated crate cannot satisfy: an out-of-line `mod x;` (only this file
/// is copied), and an `extern crate` or a `use` whose root names a crate
/// other than the built-in ones and, in a package, its `dependencies`
/// (`None` for a single file, whose generated crate has none). `local`
/// holds every item name this module declares, so `use m::X` for a local
/// module `m` passes; an inline module is checked against its own names,
/// as Rust resolves a `use` there. An `extern crate` is checked against
/// crates only.
pub(super) fn check_unsupported_items(
    items: &[Item],
    local: &HashSet<String>,
    dependencies: Option<&HashSet<String>>,
    text: &str,
) -> Result<(), ImportError> {
    let is_crate =
        |name: &str| is_builtin_crate(name) || dependencies.is_some_and(|deps| deps.contains(name));
    let dependency = |ident: &proc_macro2::Ident| ImportError::Crate {
        name: ident_name(ident),
        span: name_range(ident, text),
    };
    // Names `extern crate` brings in, aliases included, are crate names:
    // reachable with a leading `::` too, unlike this module's other items.
    let extern_names: HashSet<String> = items
        .iter()
        .filter_map(|item| match item {
            Item::ExternCrate(item) => Some(ident_name(
                item.rename.as_ref().map_or(&item.ident, |(_, alias)| alias),
            )),
            _ => None,
        })
        .collect();
    for item in items {
        match item {
            Item::Mod(item_mod) => match &item_mod.content {
                Some((_, items)) => {
                    let mut inner = HashSet::new();
                    super::collect_item_names(items, &mut inner);
                    check_unsupported_items(items, &inner, dependencies, text)?
                }
                None => {
                    return Err(ImportError::NestedModule {
                        name: ident_name(&item_mod.ident),
                        span: name_range(&item_mod.ident, text),
                    });
                }
            },
            Item::ExternCrate(item_crate) => {
                let name = ident_name(&item_crate.ident);
                if !is_crate(&name) && !is_declarable_builtin(&name) {
                    return Err(dependency(&item_crate.ident));
                }
            }
            Item::Use(item_use) => {
                let mut roots = Vec::new();
                use_roots(&item_use.tree, &mut roots);
                // `use ::x::..` can only name a crate; without the leading
                // `::`, `x` may be an item of this module (or a keyword).
                let absolute = item_use.leading_colon.is_some();
                for ident in roots {
                    let root = ident_name(ident);
                    let allowed = is_crate(&root)
                        || extern_names.contains(&root)
                        || (!absolute
                            && (matches!(root.as_str(), "crate" | "self" | "super" | "Self")
                                || local.contains(&root)));
                    if !allowed {
                        return Err(dependency(ident));
                    }
                }
            }
            _ => {}
        }
    }
    Ok(())
}

/// The crates every generated crate can name in a `use` without a
/// dependency: the extern prelude of a 2024 crate. `alloc` and
/// `proc_macro` are not in it; they need an `extern crate` first, which
/// then puts the name in scope like any item.
fn is_builtin_crate(name: &str) -> bool {
    matches!(name, "std" | "core")
}

/// The crates `extern crate` may name without a dependency.
fn is_declarable_builtin(name: &str) -> bool {
    matches!(name, "std" | "core" | "alloc" | "proc_macro" | "self")
}

/// The first path segment of every path in a `use` tree.
fn use_roots<'t>(tree: &'t UseTree, roots: &mut Vec<&'t proc_macro2::Ident>) {
    match tree {
        UseTree::Path(path) => roots.push(&path.ident),
        UseTree::Name(name) => roots.push(&name.ident),
        UseTree::Rename(rename) => roots.push(&rename.ident),
        UseTree::Glob(_) => {}
        UseTree::Group(group) => {
            for tree in &group.items {
                use_roots(tree, roots);
            }
        }
    }
}
