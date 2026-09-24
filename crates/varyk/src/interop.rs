//! Importing public Rust function signatures from `.rs` modules (spec
//! section 4.5).
//!
//! A `.rs` module is copied verbatim into the generated crate; its
//! top-level free `pub fn` items are the only things Varyk can call.
//! [`import_rust_module`] parses the module text with `syn` and returns one
//! [`ImportedFn`] per such item, with its parameter and return types mapped
//! to [`RustTy`]. Everything else in the file — methods, non-`pub` items,
//! `pub(crate)`/`pub(super)` items, `unsafe`/`async`/`const`/`extern` fns,
//! `#[cfg(...)]`-gated items, and macro invocations — is ignored here, per
//! the spec's ignore list. An out-of-line `mod x;` (no `{ ... }` body) fails
//! the whole import: the backend copies only this one file into the
//! generated crate, so the submodule's source would be missing and rustc
//! would reject it (E0583). An inline `mod x { ... }` is left alone; its
//! contents are simply unreachable from Varyk.
//!
//! The resolver turns a parse failure into a `V0104` diagnostic at the
//! `mod` declaration, and derives callability: a function is callable when
//! every parameter and the return type map to a Varyk type. A generic
//! function (type or lifetime parameters, or a `where` clause) records its
//! return type as [`RustTy::Opaque`], so it is never callable.

use std::collections::HashSet;

use proc_macro2::{TokenStream, TokenTree};
use quote::ToTokens;
use syn::{
    FnArg, Item, PathArguments, ReturnType, Safety, Type, TypeReference, UseTree, Visibility,
};

/// A Rust type, mapped to the subset spec section 4.5 assigns a Varyk mode.
///
/// `Ref`/`RefMut` are only ever built over a `Copy` primitive variant
/// (`Bool` or one of the integer/float variants) — a reference to anything
/// else, including `str`/`String`, has its own variant or is [`Opaque`].
///
/// [`Opaque`]: RustTy::Opaque
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RustTy {
    Bool,
    I8,
    I16,
    I32,
    I64,
    U8,
    U16,
    U32,
    U64,
    F32,
    F64,
    /// `&str`.
    Str,
    /// Owned `String`.
    String,
    /// `&mut String`.
    RefMutString,
    /// `&T` for a `Copy` primitive `T`.
    Ref(Box<RustTy>),
    /// `&mut T` for a `Copy` primitive `T`.
    RefMut(Box<RustTy>),
    /// `()`, or a missing return type.
    Unit,
    /// Anything else: generics, explicit lifetimes, trait objects, `impl
    /// Trait`, tuples, arrays, references to non-primitives, paths with
    /// more than one segment, and references in return position. Holds the
    /// original type text via [`ToTokens`].
    Opaque(String),
}

/// A `pub fn` imported from a `.rs` module.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportedFn {
    pub name: String,
    pub params: Vec<RustTy>,
    pub ret: RustTy,
    /// The function signature's token text, for diagnostics (spec 4.5:
    /// "unsupported Rust signature" shows the signature).
    pub signature: String,
}

/// Parses `text` as a Rust file and imports its top-level free `pub fn`
/// items (plain `pub`, not `pub(crate)`/`pub(super)`/`pub(in ...)`).
///
/// Returns `Err` with a message naming the problem when `text` does not
/// parse as a Rust file (syn's error text, which names the failing token
/// and its location).
pub fn import_rust_module(text: &str) -> Result<Vec<ImportedFn>, String> {
    let file = syn::parse_file(text).map_err(|err| err.to_string())?;
    // `include!`, `include_str!` and `include_bytes!` read another file by
    // a path relative to this one, anywhere in the file (an item, a
    // constant, a function body); only this file is copied into the
    // generated crate, so the build would fail with that file missing.
    if let Some(name) =
        find_include_macro(text.parse::<TokenStream>().map_err(|err| err.to_string())?)
    {
        return Err(format!(
            "uses `{name}!`, which is not supported: only the module file itself is copied"
        ));
    }
    // A file-level `#![cfg(...)]` or `#![cfg_attr(...)]` may configure the
    // whole file out of a normal build; import nothing rather than guess.
    if file
        .attrs
        .iter()
        .any(|attr| path_is(attr.path(), "cfg") || path_is(attr.path(), "cfg_attr"))
    {
        return Ok(Vec::new());
    }

    // Names the file's own top-level items give to a type or alias. A
    // signature that spells one of these bare names means that local item,
    // not any same-named primitive or `std` type this module maps by
    // spelling (spec 4.5's mapping is by *resolved* type, and a shadowed
    // name never resolves to the thing it looks like).
    let shadowed = collect_shadowed_names(&file.items);

    let mut local = HashSet::new();
    collect_item_names(&file.items, &mut local);
    check_unsupported_items(&file.items, &local)?;

    let mut imported = Vec::new();
    for item in file.items {
        let Item::Fn(item_fn) = item else {
            continue;
        };
        if !is_plain_pub(&item_fn.vis) {
            continue;
        }
        if !matches!(item_fn.sig.safety, Safety::Default)
            || item_fn.sig.asyncness.is_some()
            || item_fn.sig.constness.is_some()
            || item_fn.sig.abi.is_some()
        {
            continue;
        }
        if has_cfg_attr(&item_fn.attrs) {
            continue;
        }

        let params = item_fn
            .sig
            .inputs
            .iter()
            .filter_map(|arg| match arg {
                FnArg::Typed(pat_type) => Some(map_param_type(&pat_type.ty, &shadowed)),
                // A free function never takes `self`; skip defensively
                // rather than panic if syn ever hands one back.
                FnArg::Receiver(_) => None,
            })
            .collect();
        let signature = item_fn.sig.to_token_stream().to_string();
        let generics = &item_fn.sig.generics;
        let ret = if !generics.params.is_empty() || generics.where_clause.is_some() {
            // Varyk cannot name a type argument, so a generic function is
            // never callable; an opaque return type says so.
            RustTy::Opaque(signature.clone())
        } else {
            match &item_fn.sig.output {
                ReturnType::Default => RustTy::Unit,
                ReturnType::Type(_, ty) => map_return_type(ty, &shadowed),
            }
        };

        imported.push(ImportedFn {
            name: ident_name(&item_fn.sig.ident),
            params,
            ret,
            signature,
        });
    }

    Ok(imported)
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
        path_is(attr.path(), "cfg")
            || path_is(attr.path(), "cfg_attr")
            || path_is(attr.path(), "test")
    })
}

/// The identifier of a type written as a single bare path segment with no
/// generic arguments (`i32`, `String`) — `None` for anything qualified,
/// multi-segment, or with `<...>` arguments (`Vec<i32>`, `std::fmt::Debug`,
/// `<T as Trait>::Assoc`).
fn plain_path_ident(ty: &Type) -> Option<String> {
    let Type::Path(type_path) = ty else {
        return None;
    };
    if type_path.qself.is_some() || type_path.path.leading_colon.is_some() {
        return None;
    }
    let [segment] = type_path.path.segments.iter().collect::<Vec<_>>()[..] else {
        return None;
    };
    if !matches!(segment.arguments, PathArguments::None) {
        return None;
    }
    Some(ident_name(&segment.ident))
}

/// [`plain_path_ident`], but `None` when that name is one the file's own
/// top-level items give to a type or alias — such a name never resolves to
/// a primitive or `std` type this module maps by spelling.
fn bare_ident(ty: &Type, shadowed: &HashSet<String>) -> Option<String> {
    let name = plain_path_ident(ty)?;
    if shadowed.contains(&name) {
        None
    } else {
        Some(name)
    }
}

/// Every bare name a signature can spell that this module maps to a Varyk
/// type by spelling: the primitives, `String`, and `str` (only meaningful
/// inside `&str`). A glob `use` can bring in a same-named item from
/// anywhere, so its presence makes all of these conservatively shadowed.
const MAPPED_TYPE_NAMES: [&str; 13] = [
    "bool", "i8", "i16", "i32", "i64", "u8", "u16", "u32", "u64", "f32", "f64", "String", "str",
];

/// Rejects, anywhere in `items` (recursing into inline modules), what the
/// generated crate cannot satisfy: an out-of-line `mod x;` (only this file
/// is copied), and an `extern crate` or a `use` whose root names a crate
/// other than the built-in ones (the generated crate has no dependencies).
/// `local` holds every item name the file declares, so `use m::X` for a
/// local module `m` passes.
fn check_unsupported_items(items: &[Item], local: &HashSet<String>) -> Result<(), String> {
    let dependency = |name: &str| {
        format!(
            "depends on the crate `{name}`, which is not supported: the generated crate has no dependencies"
        )
    };
    for item in items {
        match item {
            Item::Mod(item_mod) => match &item_mod.content {
                Some((_, items)) => check_unsupported_items(items, local)?,
                None => {
                    return Err(format!(
                        "declares a nested module `{}`, which is not supported: only the module file itself is copied",
                        item_mod.ident
                    ));
                }
            },
            Item::ExternCrate(item_crate) => {
                let name = ident_name(&item_crate.ident);
                if !is_builtin_crate(&name) {
                    return Err(dependency(&name));
                }
            }
            Item::Use(item_use) => {
                let mut roots = Vec::new();
                use_roots(&item_use.tree, &mut roots);
                for root in roots {
                    if !is_builtin_crate(&root)
                        && !matches!(root.as_str(), "crate" | "self" | "super" | "Self")
                        && !local.contains(&root)
                    {
                        return Err(dependency(&root));
                    }
                }
            }
            _ => {}
        }
    }
    Ok(())
}

/// The crates every generated crate can name without a dependency.
fn is_builtin_crate(name: &str) -> bool {
    matches!(name, "std" | "core" | "alloc" | "self")
}

/// The first path segment of every path in a `use` tree.
fn use_roots(tree: &UseTree, roots: &mut Vec<String>) {
    match tree {
        UseTree::Path(path) => roots.push(ident_name(&path.ident)),
        UseTree::Name(name) => roots.push(ident_name(&name.ident)),
        UseTree::Rename(rename) => roots.push(ident_name(&rename.ident)),
        UseTree::Glob(_) => {}
        UseTree::Group(group) => {
            for tree in &group.items {
                use_roots(tree, roots);
            }
        }
    }
}

/// Every name an item declares, in this list and in every inline module
/// under it, plus the names its `use` items bring in.
fn collect_item_names(items: &[Item], names: &mut HashSet<String>) {
    let mut has_glob = false;
    for item in items {
        let ident = match item {
            Item::Mod(item) => {
                if let Some((_, items)) = &item.content {
                    collect_item_names(items, names);
                }
                &item.ident
            }
            Item::Struct(item) => &item.ident,
            Item::Enum(item) => &item.ident,
            Item::Union(item) => &item.ident,
            Item::Trait(item) => &item.ident,
            Item::TraitAlias(item) => &item.ident,
            Item::Type(item) => &item.ident,
            Item::Fn(item) => &item.sig.ident,
            Item::Const(item) => &item.ident,
            Item::Static(item) => &item.ident,
            Item::Macro(item) => match &item.ident {
                Some(ident) => ident,
                None => continue,
            },
            Item::Use(item) => {
                collect_use_names(&item.tree, None, names, &mut has_glob);
                continue;
            }
            _ => continue,
        };
        names.insert(ident_name(ident));
    }
}

/// The names a file's top-level items introduce that could shadow a
/// primitive or `std` type name this module maps by spelling: every
/// `struct`/`enum`/`union`/`trait`/`type` alias name, every name a `use`
/// brings into scope (its last path segment, or the `as` name for a
/// rename), and, if any `use` is a glob (`use ...::*`) or any item is a
/// macro invocation (other than a `macro_rules!` definition), every
/// [`MAPPED_TYPE_NAMES`] name — a glob or a macro expansion introduces no
/// single name of its own, but it could define any of them, and there is
/// no way to tell without resolving the glob's target or expanding the
/// macro, so all of them are shadowed conservatively.
fn collect_shadowed_names(items: &[Item]) -> HashSet<String> {
    let mut names = HashSet::new();
    let mut has_glob = false;
    for item in items {
        match item {
            Item::Macro(item) if !path_is(&item.mac.path, "macro_rules") => has_glob = true,
            Item::Struct(item) => {
                names.insert(ident_name(&item.ident));
            }
            Item::Enum(item) => {
                names.insert(ident_name(&item.ident));
            }
            Item::Union(item) => {
                names.insert(ident_name(&item.ident));
            }
            Item::Trait(item) => {
                names.insert(ident_name(&item.ident));
            }
            Item::Type(item) => {
                names.insert(ident_name(&item.ident));
            }
            Item::Mod(item) => {
                names.insert(ident_name(&item.ident));
            }
            Item::Use(item) => collect_use_names(&item.tree, None, &mut names, &mut has_glob),
            _ => {}
        }
    }
    if has_glob {
        names.extend(MAPPED_TYPE_NAMES.iter().map(|name| name.to_string()));
    }
    names
}

/// Collects the name(s) a `use` tree brings into scope, per
/// [`collect_shadowed_names`], and sets `has_glob` if it contains a glob.
/// `parent` is the path segment before `tree`: a `self` in a group
/// (`use m::X::{self}`) binds that parent's name.
fn collect_use_names(
    tree: &UseTree,
    parent: Option<&proc_macro2::Ident>,
    names: &mut HashSet<String>,
    has_glob: &mut bool,
) {
    match tree {
        UseTree::Path(path) => collect_use_names(&path.tree, Some(&path.ident), names, has_glob),
        UseTree::Name(name) => {
            let ident = match parent {
                Some(parent) if name.ident == "self" => parent,
                _ => &name.ident,
            };
            names.insert(ident_name(ident));
        }
        UseTree::Rename(rename) => {
            names.insert(ident_name(&rename.rename));
        }
        UseTree::Glob(_) => *has_glob = true,
        UseTree::Group(group) => {
            for tree in &group.items {
                collect_use_names(tree, parent, names, has_glob);
            }
        }
    }
}

/// The name of the first file-including macro invocation (`include`,
/// `include_str`, `include_bytes`) anywhere in `tokens`, walking into
/// every delimited group.
fn find_include_macro(tokens: TokenStream) -> Option<String> {
    let mut previous: Option<String> = None;
    for tree in tokens {
        match tree {
            TokenTree::Group(group) => {
                if let Some(name) = find_include_macro(group.stream()) {
                    return Some(name);
                }
                previous = None;
            }
            TokenTree::Punct(punct) if punct.as_char() == '!' => {
                if let Some(name) = previous.take() {
                    if matches!(name.as_str(), "include" | "include_str" | "include_bytes") {
                        return Some(name);
                    }
                }
            }
            TokenTree::Ident(ident) => previous = Some(ident_name(&ident)),
            _ => previous = None,
        }
    }
    None
}

/// Whether `path` is the single identifier `name`, raw spelling included.
fn path_is(path: &syn::Path, name: &str) -> bool {
    path.get_ident()
        .is_some_and(|ident| ident_name(ident) == name)
}

/// The name an identifier spells, with any raw prefix stripped: Rust
/// treats `r#String` and `String` as the same name, and so must every
/// comparison here.
fn ident_name(ident: &proc_macro2::Ident) -> String {
    let name = ident.to_string();
    name.strip_prefix("r#").map_or(name.clone(), str::to_string)
}

/// Maps a bare (non-reference) primitive type name to its [`RustTy`].
fn primitive_ty(ty: &Type, shadowed: &HashSet<String>) -> Option<RustTy> {
    Some(match bare_ident(ty, shadowed)?.as_str() {
        "bool" => RustTy::Bool,
        "i8" => RustTy::I8,
        "i16" => RustTy::I16,
        "i32" => RustTy::I32,
        "i64" => RustTy::I64,
        "u8" => RustTy::U8,
        "u16" => RustTy::U16,
        "u32" => RustTy::U32,
        "u64" => RustTy::U64,
        "f32" => RustTy::F32,
        "f64" => RustTy::F64,
        _ => return None,
    })
}

fn is_named(ty: &Type, name: &str, shadowed: &HashSet<String>) -> bool {
    bare_ident(ty, shadowed).as_deref() == Some(name)
}

fn type_to_text(ty: &Type) -> String {
    ty.to_token_stream().to_string()
}

/// Maps a reference type (`&T` / `&mut T`) appearing in parameter position.
/// A reference carrying an explicit lifetime is always [`RustTy::Opaque`],
/// per spec 4.5's "explicit lifetimes ... imported as opaque".
fn map_reference(reference: &TypeReference, whole: &Type, shadowed: &HashSet<String>) -> RustTy {
    if reference.lifetime.is_some() {
        return RustTy::Opaque(type_to_text(whole));
    }
    let inner = reference.elem.as_ref();
    // `&str` is the table's shared-borrow-of-string case; `&mut str` isn't
    // in the table at all (mutable string slices are not part of the
    // spec's mapping), so it falls through to the generic Opaque case
    // below rather than being conflated with `&str`.
    if is_named(inner, "str", shadowed) && reference.mutability.is_none() {
        return RustTy::Str;
    }
    // `&mut String` maps; a shared `&String` falls through to Opaque below
    // (not callable), and the type checker suggests `&str`.
    if is_named(inner, "String", shadowed) && reference.mutability.is_some() {
        return RustTy::RefMutString;
    }
    match primitive_ty(inner, shadowed) {
        Some(prim) if reference.mutability.is_some() => RustTy::RefMut(Box::new(prim)),
        Some(prim) => RustTy::Ref(Box::new(prim)),
        None => RustTy::Opaque(type_to_text(whole)),
    }
}

/// Maps a non-reference type, shared by parameter and return-type mapping.
fn map_non_reference(ty: &Type, shadowed: &HashSet<String>) -> RustTy {
    if let Some(prim) = primitive_ty(ty, shadowed) {
        return prim;
    }
    if is_named(ty, "String", shadowed) {
        return RustTy::String;
    }
    if matches!(ty, Type::Tuple(tuple) if tuple.elems.is_empty()) {
        return RustTy::Unit;
    }
    RustTy::Opaque(type_to_text(ty))
}

/// Maps a parameter type.
fn map_param_type(ty: &Type, shadowed: &HashSet<String>) -> RustTy {
    match ty {
        Type::Reference(reference) => map_reference(reference, ty, shadowed),
        _ => map_non_reference(ty, shadowed),
    }
}

/// Maps a return type. A reference in return position is always
/// [`RustTy::Opaque`] (spec 4.5), even `-> &str`, unlike in parameter
/// position.
fn map_return_type(ty: &Type, shadowed: &HashSet<String>) -> RustTy {
    match ty {
        Type::Reference(_) => RustTy::Opaque(type_to_text(ty)),
        _ => map_non_reference(ty, shadowed),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(text: &str) -> ImportedFn {
        let mut imported = import_rust_module(text).expect("should parse");
        assert_eq!(
            imported.len(),
            1,
            "expected exactly one imported fn from: {text}"
        );
        imported.remove(0)
    }

    fn none(text: &str) {
        let imported = import_rust_module(text).expect("should parse");
        assert!(
            imported.is_empty(),
            "expected no imported fns from: {text}, got {imported:?}"
        );
    }

    // --- Import table (spec 4.5): parameter types -----------------------

    #[test]
    fn param_bool() {
        let f = one("pub fn f(x: bool) {}");
        assert_eq!(f.params, vec![RustTy::Bool]);
    }

    #[test]
    fn param_integers() {
        let f = one("pub fn f(a: i8, b: i16, c: i32, d: i64, e: u8, g: u16, h: u32, i: u64) {}");
        assert_eq!(
            f.params,
            vec![
                RustTy::I8,
                RustTy::I16,
                RustTy::I32,
                RustTy::I64,
                RustTy::U8,
                RustTy::U16,
                RustTy::U32,
                RustTy::U64,
            ]
        );
    }

    #[test]
    fn param_floats() {
        let f = one("pub fn f(a: f32, b: f64) {}");
        assert_eq!(f.params, vec![RustTy::F32, RustTy::F64]);
    }

    #[test]
    fn param_shared_ref_to_copy_primitive() {
        let f = one("pub fn f(x: &i32) {}");
        assert_eq!(f.params, vec![RustTy::Ref(Box::new(RustTy::I32))]);
    }

    #[test]
    fn param_mut_ref_to_copy_primitive() {
        let f = one("pub fn f(x: &mut i32) {}");
        assert_eq!(f.params, vec![RustTy::RefMut(Box::new(RustTy::I32))]);
    }

    #[test]
    fn param_str_ref() {
        let f = one("pub fn f(x: &str) {}");
        assert_eq!(f.params, vec![RustTy::Str]);
    }

    #[test]
    fn param_mut_string_ref() {
        let f = one("pub fn f(x: &mut String) {}");
        assert_eq!(f.params, vec![RustTy::RefMutString]);
    }

    #[test]
    fn param_owned_string() {
        let f = one("pub fn f(x: String) {}");
        assert_eq!(f.params, vec![RustTy::String]);
    }

    #[test]
    fn param_shared_string_ref() {
        let f = one("pub fn f(x: &String) {}");
        assert_eq!(f.params, vec![RustTy::Opaque("& String".to_string())]);
    }

    // --- Return types ------------------------------------------------------

    #[test]
    fn return_string() {
        let f = one("pub fn f() -> String { String::new() }");
        assert_eq!(f.ret, RustTy::String);
    }

    #[test]
    fn return_primitive() {
        let f = one("pub fn f() -> i32 { 0 }");
        assert_eq!(f.ret, RustTy::I32);
    }

    #[test]
    fn return_missing_is_unit() {
        let f = one("pub fn f() {}");
        assert_eq!(f.ret, RustTy::Unit);
    }

    #[test]
    fn return_explicit_unit() {
        let f = one("pub fn f() -> () {}");
        assert_eq!(f.ret, RustTy::Unit);
    }

    #[test]
    fn return_reference_is_opaque() {
        let f = one("pub fn f() -> &'static str { \"\" }");
        assert!(matches!(f.ret, RustTy::Opaque(_)));
    }

    // --- Opaque parameter cases, signature text preserved -------------------

    #[test]
    fn param_generic_type_is_opaque() {
        let f = one("pub fn f<T>(x: T) {}");
        assert_eq!(f.params, vec![RustTy::Opaque("T".to_string())]);
    }

    #[test]
    fn param_vec_is_opaque() {
        let f = one("pub fn f(x: Vec<i32>) {}");
        match &f.params[0] {
            RustTy::Opaque(text) => assert_eq!(text, "Vec < i32 >"),
            other => panic!("expected Opaque, got {other:?}"),
        }
    }

    #[test]
    fn param_option_is_opaque() {
        let f = one("pub fn f<T>(x: Option<T>) {}");
        match &f.params[0] {
            RustTy::Opaque(text) => assert_eq!(text, "Option < T >"),
            other => panic!("expected Opaque, got {other:?}"),
        }
    }

    #[test]
    fn param_lifetime_ref_is_opaque() {
        let f = one("pub fn f<'a>(x: &'a str) {}");
        match &f.params[0] {
            RustTy::Opaque(text) => assert_eq!(text, "& 'a str"),
            other => panic!("expected Opaque, got {other:?}"),
        }
    }

    #[test]
    fn param_trait_object_is_opaque() {
        let f = one("pub fn f(x: &dyn Foo) {}");
        assert!(matches!(&f.params[0], RustTy::Opaque(_)));
    }

    #[test]
    fn param_impl_trait_is_opaque() {
        let f = one("pub fn f(x: impl Foo) {}");
        assert!(matches!(&f.params[0], RustTy::Opaque(_)));
    }

    #[test]
    fn param_tuple_is_opaque() {
        let f = one("pub fn f(x: (i32, i32)) {}");
        match &f.params[0] {
            RustTy::Opaque(text) => assert_eq!(text, "(i32 , i32)"),
            other => panic!("expected Opaque, got {other:?}"),
        }
    }

    #[test]
    fn param_array_is_opaque() {
        let f = one("pub fn f(x: [i32; 4]) {}");
        assert!(matches!(&f.params[0], RustTy::Opaque(_)));
    }

    #[test]
    fn param_ref_to_non_primitive_is_opaque() {
        let f = one("pub fn f(x: &Foo) {}");
        match &f.params[0] {
            RustTy::Opaque(text) => assert_eq!(text, "& Foo"),
            other => panic!("expected Opaque, got {other:?}"),
        }
    }

    #[test]
    fn param_multi_segment_path_is_opaque() {
        let f = one("pub fn f(x: std::string::String) {}");
        assert!(matches!(&f.params[0], RustTy::Opaque(_)));
    }

    /// `&mut str` is not in the spec 4.5 table (which only has `&str`);
    /// it must not be conflated with `&str`.
    #[test]
    fn param_mut_str_ref_is_opaque() {
        let f = one("pub fn f(x: &mut str) {}");
        assert!(matches!(&f.params[0], RustTy::Opaque(_)));
    }

    #[test]
    fn generic_function_has_an_opaque_return() {
        let f = one("pub fn zero<T: Default>() -> i32 { 0 }");
        assert!(matches!(&f.ret, RustTy::Opaque(_)), "{f:?}");
        let f = one("pub fn zero<T>() -> i32 where T: Default { 0 }");
        assert!(matches!(&f.ret, RustTy::Opaque(_)), "{f:?}");
        let f = one("pub fn plain() -> i32 { 0 }");
        assert_eq!(f.ret, RustTy::I32);
    }

    // --- Shadowed type names: a local item can shadow a mapped name --------

    /// A module-local `struct` named after a primitive shadows it: the
    /// bare name no longer resolves to the primitive, so it must not be
    /// mapped as one (or the generated crate fails to compile, since the
    /// call site would pass a Varyk-primitive value where the local struct
    /// type is expected).
    #[test]
    fn struct_shadowing_a_primitive_name_is_opaque() {
        let f = one("pub struct i32; pub fn f(_: i32) {}");
        assert!(matches!(&f.params[0], RustTy::Opaque(_)), "{f:?}");
    }

    /// Same shadowing hazard via a local `type` alias reusing `String`.
    #[test]
    fn type_alias_shadowing_string_is_opaque() {
        let f = one("type String = Vec<u8>; pub fn f(x: String) {}");
        assert!(matches!(&f.params[0], RustTy::Opaque(_)), "{f:?}");
    }

    /// A `use` that renames an import onto a mapped name is the same
    /// hazard: `i32` now names whatever was imported, not the primitive.
    #[test]
    fn use_rename_shadowing_a_primitive_name_is_opaque() {
        let f = one("use std::string::String as i32; pub fn f(x: i32) {}");
        assert!(matches!(&f.params[0], RustTy::Opaque(_)), "{f:?}");
    }

    /// An ordinary module with no shadowing items maps names exactly as
    /// before: an unrelated `use` and struct don't affect `i32`/`String`.
    #[test]
    fn ordinary_module_without_shadowing_is_unaffected() {
        let f = one("use std::collections::HashMap; \
             struct Point { x: i32, y: i32 } \
             pub fn f(a: i32, b: String) -> i32 { a }");
        assert_eq!(f.params, vec![RustTy::I32, RustTy::String]);
        assert_eq!(f.ret, RustTy::I32);
    }

    /// A glob `use` could bring in a same-named `String` from anywhere, so
    /// every mapped name becomes opaque conservatively, making the function
    /// uncallable (V0108 at the resolver).
    #[test]
    fn glob_use_makes_mapped_names_opaque() {
        let f = one("use crate::types::*; pub fn f(_: String) {}");
        assert!(matches!(&f.params[0], RustTy::Opaque(_)), "{f:?}");
    }

    /// An item-position macro invocation could expand to a same-named
    /// `String` item, so every mapped name becomes opaque conservatively.
    #[test]
    fn item_macro_invocation_makes_mapped_names_opaque() {
        let f =
            one("macro_rules! ty { () => { pub struct String; } } ty!(); pub fn f(_: String) {}");
        assert!(matches!(&f.params[0], RustTy::Opaque(_)), "{f:?}");
    }

    /// A `macro_rules!` definition alone introduces no item, so it does
    /// not shadow anything.
    #[test]
    fn macro_rules_definition_does_not_shadow_mapped_names() {
        let f = one("macro_rules! ty { () => { pub struct String; } } pub fn f(_: String) {}");
        assert_eq!(f.params, vec![RustTy::String]);
    }

    /// `include!` splices in a file that is never copied next to the
    /// module, so the import is rejected rather than left to fail in rustc.
    #[test]
    fn include_macro_is_rejected() {
        let err = import_rust_module("include!(\"defs.rs\"); pub fn f() {}").unwrap_err();
        assert!(err.contains("`include!`"), "{err}");
    }

    /// The same for `include_str!`/`include_bytes!` nested anywhere: in a
    /// constant or inside a function body.
    #[test]
    fn nested_include_str_and_include_bytes_are_rejected() {
        let err = import_rust_module("pub fn f() -> &'static str { include_str!(\"a.txt\") }")
            .unwrap_err();
        assert!(err.contains("`include_str!`"), "{err}");
        let err = import_rust_module("const B: &[u8] = include_bytes!(\"a.bin\"); pub fn f() {}")
            .unwrap_err();
        assert!(err.contains("`include_bytes!`"), "{err}");
    }

    /// Raw spellings name the same things: `r#include_str!` is rejected
    /// like `include_str!`, and `struct r#String` shadows `String`.
    #[test]
    fn raw_identifiers_are_normalized() {
        let err = import_rust_module("pub fn f() -> &'static str { r#include_str!(\"a.txt\") }")
            .unwrap_err();
        assert!(err.contains("`include_str!`"), "{err}");
        let f = one("pub struct r#String; pub fn f(_: String) {}");
        assert!(matches!(&f.params[0], RustTy::Opaque(_)), "{f:?}");
        let f = one("pub fn r#f(_: r#i32) {}");
        assert_eq!((f.name.as_str(), &f.params), ("f", &vec![RustTy::I32]));
    }

    /// An identifier merely named `include` (a function, say) is not a
    /// macro invocation and is left alone.
    #[test]
    fn plain_include_identifier_is_not_rejected() {
        let f = one("fn include() {} pub fn f() { include() }");
        assert_eq!(f.name, "f");
    }

    /// A file with no glob `use` is unaffected: an ordinary named `use`
    /// still maps `String` normally.
    #[test]
    fn non_glob_use_does_not_shadow_mapped_names() {
        let f = one("use std::collections::HashMap; pub fn f(_: String) {}");
        assert_eq!(f.params, vec![RustTy::String]);
    }

    // --- Ignore list ---------------------------------------------------

    #[test]
    fn ignores_pub_crate() {
        none("pub(crate) fn f() {}");
    }

    #[test]
    fn ignores_pub_super() {
        none("pub(super) fn f() {}");
    }

    #[test]
    fn ignores_method_in_impl() {
        none("struct S; impl S { pub fn f(&self) {} }");
    }

    #[test]
    fn ignores_unsafe_fn() {
        none("pub unsafe fn f() {}");
    }

    #[test]
    fn ignores_async_fn() {
        none("pub async fn f() {}");
    }

    #[test]
    fn ignores_const_fn() {
        none("pub const fn f() {}");
    }

    #[test]
    fn ignores_extern_fn() {
        none(r#"pub extern "C" fn f() {}"#);
    }

    #[test]
    fn ignores_cfg_gated() {
        none("#[cfg(test)] pub fn f() {}");
    }

    #[test]
    fn file_level_cfg_imports_nothing() {
        none("#![cfg(test)] pub fn f() {}");
        none("#![cfg_attr(test, allow(dead_code))] pub fn f() {}");
    }

    /// A raw-spelled attribute is the same attribute.
    #[test]
    fn ignores_raw_cfg_gated() {
        none("#[r#cfg(test)] pub fn f() {}");
        none("#![r#cfg(test)] pub fn f() {}");
    }

    /// `extern crate` names a dependency the generated crate does not
    /// have, so the import is rejected; the built-in crates are fine.
    #[test]
    fn extern_crate_is_rejected_except_builtin_crates() {
        let err = import_rust_module("extern crate serde; pub fn f() {}").unwrap_err();
        assert!(err.contains("`serde`"), "{err}");
        let f = one("extern crate std; extern crate core; extern crate alloc; pub fn f() {}");
        assert_eq!(f.name, "f");
    }

    /// A `use` rooted at a crate the generated crate does not depend on is
    /// rejected like `extern crate`; local modules and built-in crates pass.
    #[test]
    fn use_of_external_crate_is_rejected() {
        let err = import_rust_module("use serde::Serialize; pub fn f() {}").unwrap_err();
        assert!(err.contains("`serde`"), "{err}");
        let err =
            import_rust_module("mod m { use ::serde::Serialize; } pub fn f() {}").unwrap_err();
        assert!(err.contains("`serde`"), "{err}");
        let f = one(
            "mod types { pub struct S; } use types::S; use std::fmt; use crate::types::S as T; use self::{types::S as U}; pub fn f() {}",
        );
        assert_eq!(f.name, "f");
    }

    /// An out-of-line `mod inner;` is rejected inside an inline module too.
    #[test]
    fn nested_out_of_line_module_is_rejected() {
        let err = import_rust_module("mod outer { mod inner; } pub fn f() {}").unwrap_err();
        assert!(err.contains("`inner`"), "{err}");
    }

    /// `use m::String::{self}` binds `String`, and an inline `mod String`
    /// takes the name too; both make the mapped name opaque.
    #[test]
    fn grouped_self_import_and_inline_module_shadow_mapped_names() {
        let f = one(
            "mod types { pub struct String; } use types::String::{self}; pub fn f(_: String) {}",
        );
        assert!(matches!(&f.params[0], RustTy::Opaque(_)), "{f:?}");
        let f = one("mod String {} pub fn f(_: String) {}");
        assert!(matches!(&f.params[0], RustTy::Opaque(_)), "{f:?}");
    }

    #[test]
    fn ignores_cfg_attr_gated() {
        none("#[cfg_attr(not(test), cfg(test))] pub fn f() {}");
    }

    #[test]
    fn ignores_test_fn() {
        none("#[test] pub fn f() {}");
    }

    #[test]
    fn ignores_private_fn() {
        none("fn f() {}");
    }

    #[test]
    fn ignores_macro_rules() {
        none("macro_rules! f { () => {} }");
    }

    // --- Parse failure ---------------------------------------------------

    #[test]
    fn parse_failure_returns_named_error() {
        let err = import_rust_module("pub fn f(").expect_err("should fail to parse");
        assert!(!err.is_empty());
    }

    // --- Nested modules --------------------------------------------------

    /// An out-of-line `mod inner;` cannot be honored, because the backend
    /// copies only this one file into the generated crate; the whole import
    /// fails rather than silently dropping the declaration.
    #[test]
    fn out_of_line_mod_declaration_is_an_error() {
        let err = import_rust_module("mod inner;\npub fn f() {}")
            .expect_err("should fail: nested module cannot be copied");
        assert!(err.contains("inner"), "{err}");
    }

    /// An inline `mod inner { ... }` has nothing missing from the copied
    /// file, so it is simply ignored, same as any other non-`fn` item.
    #[test]
    fn inline_mod_is_ignored() {
        let f = one("mod inner { pub fn helper() {} }\npub fn f() {}");
        assert_eq!(f.name, "f");
    }

    // --- The one real input in the repository ---------------------------

    #[test]
    fn imports_greet_rs() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../examples/interop/greet.rs"
        );
        let text = std::fs::read_to_string(path).expect("examples/interop/greet.rs should exist");
        let f = one(&text);
        assert_eq!(f.name, "hello");
        assert_eq!(f.params, vec![RustTy::Str]);
        assert_eq!(f.ret, RustTy::String);
    }
}
