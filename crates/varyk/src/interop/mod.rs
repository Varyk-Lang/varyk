//! Importing public Rust signatures, structs, and enums from `.rs` modules
//! (M1 spec 4.5, M3 spec 4.1-4.4).
//!
//! A `.rs` module is copied verbatim into the generated crate.
//! [`import_rust_module`] parses the module text with `syn` and returns an
//! [`ImportedModule`]: one [`ImportedFn`] per top-level free `pub fn`, one
//! [`ImportedStruct`] per top-level plain-`pub` struct, with the `pub fn`
//! items of its same-file inherent `impl` blocks, and one [`ImportedEnum`]
//! per top-level plain-`pub` enum, with types mapped to [`RustTy`]; and,
//! from anywhere in the file, what its `impl Drop` blocks are for
//! ([`DropScan`]). Everything else in the file — other trait impls,
//! non-`pub` items, `pub(crate)`/`pub(super)` items,
//! `unsafe`/`async`/`const`/`extern` fns, `#[cfg(...)]`-gated items, and
//! macro invocations — is ignored here, per the spec's ignore list. An out-of-line `mod x;` (no `{ ... }` body) fails
//! the whole import: the backend copies only this one file into the
//! generated crate, so the submodule's source would be missing and rustc
//! would reject it (E0583). An inline `mod x { ... }` is left alone; its
//! contents are simply unreachable from Varyk.
//!
//! The resolver turns an [`ImportError`] into a `V0104` diagnostic, at the
//! `mod` declaration for a parse failure and at the offending name in the
//! `.rs` file otherwise, and derives callability: a function is callable when
//! every parameter and the return type map to a Varyk type. A generic
//! function (type or lifetime parameters, or a `where` clause) records its
//! return type as [`RustTy::Opaque`], so it is never callable.

use std::collections::HashSet;
use std::ops::Range;

use proc_macro2::{TokenStream, TokenTree};
use syn::{Item, UseTree};

use crate::types::Derives;

mod drops;
mod items;
mod macros;
mod signatures;
#[cfg(test)]
mod tests;

pub(crate) use items::glob_reason;
pub use macros::MacroRisk;

/// Why a function or method is not imported: it has `#[cfg]` or
/// `#[cfg_attr]` (or its `impl` block has), so it may not exist in the
/// build.
pub const CFG: &str = "behind `#[cfg]`";
/// Why a function or method is not imported: it is a `#[test]`.
pub const TEST: &str = "marked `#[test]`";
/// Why a function or method is not imported: a parameter or its receiver
/// has `#[cfg]`, so it may not have this shape in the build.
pub const CFG_PARAM: &str = "a parameter behind `#[cfg]`";
/// Why a method is not imported: it is a method of a Rust enum.
pub const ENUM_METHOD: &str = "a method of a Rust enum";
/// Why a name is not imported: a `pub use` brings it in from elsewhere.
pub const REEXPORT: &str = "brought in from elsewhere by `pub use`";
use macros::Macros;
use signatures::Names;
pub use signatures::{RustPath, RustTy};
pub use varyk_syntax::SelfMode;

/// A `pub fn` imported from a `.rs` module: a free function, or a method
/// or associated function of an [`ImportedStruct`] (spec 4.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportedFn {
    pub name: String,
    /// Not including the receiver. A receiver Varyk cannot pass (owned
    /// `self`, `self: Box<Self>`, `&'a self`) is the first entry, as
    /// [`RustTy::Opaque`] holding its text, so the method is never
    /// callable.
    pub params: Vec<RustTy>,
    pub ret: RustTy,
    /// `Some(Shared)` for `&self`, `Some(Mutable)` for `&mut self`;
    /// `None` for a free or associated function (or an opaque receiver,
    /// see `params`).
    pub receiver: Option<SelfMode>,
    /// The struct whose `impl` block declares the function; `None` for a
    /// free function.
    pub owner: Option<String>,
    /// The function signature's token text, for diagnostics (spec 4.5:
    /// "unsupported Rust signature" shows the signature).
    pub signature: String,
    /// The parameter position (0 for the receiver) `ret` borrows from,
    /// when `ret` is `&str` or `&S` and lifetime elision roots it there
    /// (M4 spec 2.12); `None` for an owned or opaque return.
    pub ret_root: Option<usize>,
    /// `async fn` (milestone 5b1 spec 2.8): called awaited or started, as
    /// a Varyk async function is.
    pub is_async: bool,
    /// The function has type parameters or a `where` clause.
    pub generic: bool,
    /// Byte range of the function's name in the file.
    pub span: Range<usize>,
    /// The one type parameter Varyk fills at the call from where the
    /// result goes (milestone 5b3 spec 2.1), named as the signature names
    /// it; it is [`RustTy::Param`] in `ret`.
    pub type_param: Option<String>,
    /// Why a generic signature is not callable, in words for a note
    /// ("a `where` clause"); `ret` is then [`RustTy::Opaque`] holding the
    /// signature.
    pub type_param_refused: Option<&'static str>,
}

/// Whether an [`ImportedField`] is `pub` (plain `pub`) or not visible
/// from Varyk (private, `pub(crate)`, `pub(super)`, `pub(in ..)`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldVis {
    Pub,
    Hidden,
}

/// A struct's shape; only `Named` is imported (spec 4.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StructKind {
    Named,
    Tuple,
    Unit,
}

/// One field of an [`ImportedStruct`], in declaration order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportedField {
    pub name: String,
    pub ty: RustTy,
    pub vis: FieldVis,
    /// Byte range of the field's name in the file.
    pub span: Range<usize>,
}

/// A top-level plain-`pub` struct of a `.rs` module (spec 4.1), with the
/// `pub fn` items of its same-file inherent `impl` blocks, merged in
/// source order (spec 4.2). A generic, tuple, or unit struct is still
/// listed, so that naming it can say why it was not imported.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportedStruct {
    pub name: String,
    /// Empty unless `kind` is `Named`.
    pub fields: Vec<ImportedField>,
    pub methods: Vec<ImportedFn>,
    /// The `pub` methods of its inherent `impl` blocks, and every method
    /// of a trait `impl` for it, that are not imported, each with why in
    /// words for a note ("a trait method", "`unsafe`", "behind `#[cfg]`").
    pub skipped_methods: Vec<(String, &'static str)>,
    /// The methods and associated functions of its inherent `impl` blocks
    /// with a restricted `pub(...)`, not imported: each name and its
    /// visibility as written (`pub(crate)`).
    pub restricted_methods: Vec<(String, String)>,
    /// Has type or lifetime parameters, or a `where` clause.
    pub generic: bool,
    /// Why a struct of an importable shape still cannot be used as a
    /// Varyk struct: `#[repr(packed)]`, or no fixed size.
    pub unfit: Option<&'static str>,
    /// Which of `Clone` and `PartialEq` its `#[derive(..)]` lists name
    /// (M4 spec 2.12); a hand-written `impl` is not seen.
    pub derives: Derives,
    pub kind: StructKind,
    /// Byte range of the struct's name in the file.
    pub span: Range<usize>,
}

/// One unit or tuple variant of an [`ImportedEnum`], in declaration
/// order. A named-field variant is listed with an empty `payload`: it
/// makes the whole enum opaque (spec 4.3), so its fields' shape is never
/// used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportedVariant {
    pub name: String,
    pub payload: Vec<RustTy>,
}

/// A top-level plain-`pub` enum of a `.rs` module (spec 4.3). A generic
/// or lifetime-parameterized enum is still listed, so that naming it can
/// say why it was not imported, like [`ImportedStruct`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportedEnum {
    pub name: String,
    pub variants: Vec<ImportedVariant>,
    /// Has type or lifetime parameters, or a `where` clause.
    pub generic: bool,
    /// `Some`, with why, when a variant has named fields or a payload
    /// type that is already known to be unmappable from this file alone
    /// (spec 4.3, 4.4): a bare shape (`HashMap`, a tuple, a reference,
    /// ...) or a type reached through a `use` line. A payload type that
    /// still might not map (a bare name of this file, or a `crate::`
    /// path) is decided later, once every module's types are known; see
    /// `resolve::imports::register`, which can still set this after the
    /// fact.
    pub opaque: Option<String>,
    /// Which of `Clone` and `PartialEq` its `#[derive(..)]` lists name
    /// (M4 spec 2.12); a hand-written `impl` is not seen.
    pub derives: Derives,
    /// Byte range of the enum's name in the file.
    pub span: Range<usize>,
    /// The name of every `fn` of the file's inherent `impl` blocks for
    /// it, which Varyk does not import (spec 4.2 gives methods to structs
    /// only), so that calling one can say so.
    pub methods: Vec<String>,
}

/// Everything imported from one `.rs` module.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImportedModule {
    pub fns: Vec<ImportedFn>,
    pub structs: Vec<ImportedStruct>,
    pub enums: Vec<ImportedEnum>,
    /// What the file's `impl Drop` blocks, anywhere in it, are for.
    pub drops: DropScan,
    /// Every top-level plain-`pub` free fn and `pub use` name that is not
    /// imported for a reason a caller could not guess: its kind for a
    /// headline (`"function"`, or `"item"` for a `pub use`) and why, in
    /// words for a note ("behind `#[cfg]`", "`const`", "a `pub use`
    /// re-export").
    pub skipped_fns: Vec<(String, &'static str, &'static str)>,
    /// Every top-level plain-`pub` struct (`"struct"`) and enum (`"enum"`)
    /// behind `#[cfg]`, not imported, whose name no other struct or enum
    /// of the file takes: its kind and name.
    pub cfg_types: Vec<(&'static str, String)>,
    /// Every top-level free fn with a restricted `pub(...)`, not imported:
    /// its name and its visibility as written (`pub(crate)`).
    pub restricted_fns: Vec<(String, String)>,
    /// Every top-level struct (`"struct"`) and enum (`"enum"`) with a
    /// restricted `pub(...)`, not imported: its kind, name, and visibility
    /// as written.
    pub restricted_types: Vec<(&'static str, String, String)>,
    /// Every top-level inline `mod name { ... }`, whose items Varyk does
    /// not read: its name.
    pub inline_mods: Vec<String>,
    /// The first top-level macro call that could define type names (see
    /// [`collect_names`]).
    pub expander: Option<Expander>,
    /// The file's name (`helper.rs`), for notes; set by whoever read the
    /// file, empty until then.
    pub file: String,
}

/// A top-level macro call of a `.rs` file that could define type names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Expander {
    /// How it is written: `make!`.
    pub shown: String,
    /// Its 1-based line.
    pub line: usize,
    pub risk: MacroRisk,
}

/// The `impl Drop` blocks found anywhere in one `.rs` file, for the
/// resolver to mark the enums they are for (see `resolve::imports`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DropScan {
    /// The last path segment of each `impl Drop` block's type.
    pub targets: Vec<String>,
    /// Why every enum may drop, as the end of a sentence starting with
    /// the file's name: some `impl Drop` block's type could not be read as
    /// a path, `Drop` is renamed or shared, or a macro could expand to an
    /// `impl Drop`.
    pub unknown: Option<String>,
    /// Names a `type` alias or a `use ... as` rename introduces, which
    /// may stand for any type.
    pub aliases: Vec<String>,
    /// Every struct and union name declared anywhere in the file.
    pub structs: Vec<String>,
}

/// Why a `.rs` file cannot be imported (spec 4.5): it does not parse, or
/// it parses but names something only it could not bring into the
/// generated crate. Each of the latter carries the name it is about and
/// its byte range in the file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportError {
    /// The file does not parse as Rust: syn's message.
    Parse(String),
    /// An `extern crate` or a `use` names a crate the generated crate
    /// does not depend on.
    Crate { name: String, span: Range<usize> },
    /// An out-of-line `mod name;`: only this file is copied.
    NestedModule { name: String, span: Range<usize> },
    /// `include!`, `include_str!`, or `include_bytes!` (the name, without
    /// `!`): only this file is copied.
    Include { name: String, span: Range<usize> },
}

impl std::fmt::Display for ImportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ImportError::Parse(message) => write!(f, "{message}"),
            ImportError::Crate { name, .. } => write!(f, "uses the crate `{name}`"),
            ImportError::NestedModule { name, .. } => write!(f, "declares a module `{name}`"),
            ImportError::Include { name, .. } => write!(f, "uses `{name}!`"),
        }
    }
}

/// Parses `text` as a Rust file and imports its top-level free `pub fn`
/// items (plain `pub`, not `pub(crate)`/`pub(super)`/`pub(in ...)`), its
/// plain-`pub` structs, and their same-file inherent methods.
///
/// Returns `Err` with a message naming the problem when `text` does not
/// parse as a Rust file (syn's error text, which names the failing token
/// and its location).
pub fn import_rust_module(text: &str) -> Result<ImportedModule, ImportError> {
    import_rust_module_in(text, None)
}

/// [`import_rust_module`] for a module of a package whose
/// `[dependencies]` are `crates` (manifest names), or of a single file when
/// `None`: an `extern crate` or `use` naming a dependency is allowed, since
/// the generated crate has it.
pub fn import_rust_module_in(
    text: &str,
    crates: Option<&[String]>,
) -> Result<ImportedModule, ImportError> {
    let parse_error = |err: syn::Error| ImportError::Parse(err.to_string());
    let file = syn::parse_file(text).map_err(parse_error)?;
    // `include!`, `include_str!` and `include_bytes!` read another file by
    // a path relative to this one, anywhere in the file (an item, a
    // constant, a function body); only this file is copied into the
    // generated crate, so the build would fail with that file missing.
    let tokens = text
        .parse::<TokenStream>()
        .map_err(|err| ImportError::Parse(err.to_string()))?;
    if let Some(ident) = find_include_macro(tokens) {
        return Err(ImportError::Include {
            name: ident_name(&ident),
            span: items::name_range(&ident, text),
        });
    }
    // A file-level `#![cfg(...)]` or `#![cfg_attr(...)]` may configure the
    // whole file out of a normal build; import nothing rather than guess.
    if file
        .attrs
        .iter()
        .any(|attr| path_is(attr.path(), "cfg") || path_is(attr.path(), "cfg_attr"))
    {
        return Ok(ImportedModule {
            drops: drops::scan(&file, &Macros::new(&file)),
            ..ImportedModule::default()
        });
    }

    let macros = Macros::new(&file);
    let (names, expander) = collect_names(&file.items, &macros);

    let mut local = HashSet::new();
    collect_item_names(&file.items, &mut local);
    let dependencies: Option<HashSet<String>> =
        crates.map(|crates| crates.iter().map(|name| name.replace('-', "_")).collect());
    items::check_unsupported_items(&file.items, &local, dependencies.as_ref(), text)?;

    let drops = drops::scan(&file, &macros);
    Ok(ImportedModule {
        drops,
        expander,
        ..items::import_items(file.items, &names, text)
    })
}

/// The name of the first file-including macro invocation (`include`,
/// `include_str`, `include_bytes`) anywhere in `tokens`, walking into
/// every delimited group, a `macro_rules!` body included. The rule is
/// deliberately blunt: a file include can never resolve from the copied
/// file, whether the macro around it is invoked or not, and a local macro
/// that happens to be named `include` is not told apart from the builtin
/// (rare, and not worth the machinery); either way the message names the
/// token so the author can rename or remove it.
fn find_include_macro(tokens: TokenStream) -> Option<proc_macro2::Ident> {
    let mut previous: Option<proc_macro2::Ident> = None;
    for tree in tokens {
        match tree {
            TokenTree::Group(group) => {
                if let Some(name) = find_include_macro(group.stream()) {
                    return Some(name);
                }
                previous = None;
            }
            TokenTree::Punct(punct) if punct.as_char() == '!' => {
                if let Some(ident) = previous.take() {
                    if matches!(
                        ident_name(&ident).as_str(),
                        "include" | "include_str" | "include_bytes"
                    ) {
                        return Some(ident);
                    }
                }
            }
            TokenTree::Ident(ident) => previous = Some(ident),
            _ => previous = None,
        }
    }
    None
}

/// Every name an item in this list declares, plus the names its `use`
/// items bring in: the names in scope for a `use` in this module. An
/// inline module's own items are its scope, not this one's, so they are
/// not collected here; `check_unsupported_items` collects them when it
/// descends.
fn collect_item_names(items: &[Item], names: &mut HashSet<String>) {
    let mut has_glob = false;
    for item in items {
        let ident = match item {
            Item::Mod(item) => &item.ident,
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
            // `extern crate x as y;` puts `y` (else `x`) in scope, so a
            // `use y::..` below is not a crate to look up.
            Item::ExternCrate(item) => match &item.rename {
                Some((_, alias)) => alias,
                None => &item.ident,
            },
            _ => continue,
        };
        names.insert(ident_name(ident));
    }
}

/// Every bare name a signature can spell that this module maps to a Varyk
/// type by spelling: the primitives, `String`, `str` (only meaningful
/// inside `&str`), and the generic `Vec`, `Option`, and `Result`. A glob
/// `use` can bring in a same-named item from anywhere, so its presence
/// makes all of these conservatively shadowed.
const MAPPED_TYPE_NAMES: [&str; 17] = [
    "bool", "i8", "i16", "i32", "i64", "u8", "u16", "u32", "u64", "usize", "f32", "f64", "String",
    "str", "Vec", "Option", "Result",
];

/// The names a file's top-level items give to types, which decide what a
/// bare name in a signature or field means (spec 4.4):
///
/// - `types`: every `struct`/`enum`/`union`/`trait`/`type` alias name, an
///   item of this file;
/// - `used`: every name a `use` brings into scope (its last path segment,
///   or the `as` name for a rename), which Varyk does not follow;
/// - `glob`: some `use` is a glob (`use ...::*`), which could bring in any
///   name;
/// - `expands`: some top-level macro call could write items Varyk cannot
///   see (see [`macros`]); the first such call is returned, as written
///   (`make!`), with its line and why, for a note;
/// - `shadowed`: every name that no longer means a primitive or `std`
///   type this module maps by spelling: all of the above plus module
///   names and, when there is a glob or such a macro call, every
///   [`MAPPED_TYPE_NAMES`] name — a glob or a macro expansion introduces
///   no single name of its own, but it could define any of them, and
///   there is no way to tell without resolving the glob's target or
///   expanding the macro.
fn collect_names(items: &[Item], macros: &Macros) -> (Names, Option<Expander>) {
    let mut names = Names::default();
    let mut expander = None;
    for item in items {
        let ident = match item {
            Item::Macro(item) => {
                if item.ident.is_none() && expander.is_none() {
                    if let Some(risk) = macros.risk(&item.mac, true) {
                        let path = &item.mac.path;
                        let name = path
                            .segments
                            .last()
                            .map_or_else(String::new, |seg| ident_name(&seg.ident));
                        let line = path
                            .segments
                            .first()
                            .map_or(0, |seg| seg.ident.span().start().line);
                        expander = Some(Expander {
                            shown: format!("{name}!"),
                            line,
                            risk,
                        });
                    }
                }
                continue;
            }
            Item::Struct(item) => &item.ident,
            Item::Enum(item) => &item.ident,
            Item::Union(item) => &item.ident,
            Item::Trait(item) => &item.ident,
            Item::Type(item) => &item.ident,
            Item::Mod(item) => {
                names.shadowed.insert(ident_name(&item.ident));
                continue;
            }
            Item::Use(item) => {
                collect_use_names(&item.tree, None, &mut names.used, &mut names.glob);
                continue;
            }
            _ => continue,
        };
        names.types.insert(ident_name(ident));
    }
    names.shadowed.extend(names.types.iter().cloned());
    names.shadowed.extend(names.used.iter().cloned());
    names.expands = expander.is_some();
    if names.glob || names.expands {
        names
            .shadowed
            .extend(MAPPED_TYPE_NAMES.iter().map(|name| name.to_string()));
    }
    (names, expander)
}

/// Collects the name(s) a `use` tree brings into scope, per
/// [`collect_names`], and sets `has_glob` if it contains a glob.
/// `parent` is the path segment before `tree`: a `self` in a group
/// (`use m::X::{self}`) binds that parent's name.
pub(super) fn collect_use_names(
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

/// Tidies `syn`'s `to_token_stream().to_string()` spacing for a
/// diagnostic: `Vec < String >` becomes `Vec<String>`. Used wherever a
/// raw signature or type's token text (kept for its exact spelling)
/// reaches a message a person reads.
pub fn tidy_signature(tokens: &str) -> String {
    let mut out = String::new();
    let mut prev = "";
    for token in tokens.split_whitespace() {
        // A group prints as one piece (`(value`), so compare the edges.
        let glue_to_prev = token.starts_with([',', ':', ';', ')', '>', '(', '<'])
            || prev.ends_with(['(', '<', '&'])
            || prev.ends_with("::");
        if !out.is_empty() && !glue_to_prev {
            out.push(' ');
        }
        out.push_str(token);
        prev = token;
    }
    out
}
