//! The resolver: loads the entry file and its `mod` files, collects every
//! module's symbols, resolves signature and field types, and enforces
//! visibility (spec sections 4.1 and 4.5).
//!
//! [`resolve`] owns lexing and parsing of every file, entry included, with
//! the per-file stop rule: a file with syntax errors contributes only those
//! errors, and resolution stops after module loading if any file had them.
//! Expression paths and locals are resolved later, during lowering in the
//! type checker, through [`Symbols::lookup_fn`], [`Symbols::lookup_type`],
//! [`Symbols::lookup_member`], and [`Symbols::resolve_type`].

use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use varyk_syntax::{
    FileId, FixIt, Function, Ident, ImplBlock, Item, Path, Program, SelfMode, SourceFile, Span,
    TypeExpr, VariantFields, parse_source,
};

use crate::builtins::BuiltinId;
use crate::diagnostics::{Diagnostic, codes};
use crate::interop::ImportedModule;
use crate::package::Kind;
use crate::types::{Derives, ParamMode, Ty};

mod attrs;
mod imports;
mod modules;
mod paths;
mod signatures;
#[cfg(test)]
mod tests;
mod uses;
mod visibility;

use attrs::Place;
pub use attrs::{FieldAttrs, HirDefault};
use imports::skipped_note;
use modules::load_modules;
pub use paths::resolve_path;
pub(crate) use paths::{display_path, no_parent, path_text, split_last};
pub(crate) use uses::UseTarget;
pub(crate) use visibility::{is_visible, not_visible};

/// The name given to the entry module. It can never collide with a `mod`
/// name (`crate` is a keyword); a path reaches the entry module only
/// through `crate::` or `super::` (spec 3.1).
pub const ENTRY_MODULE_NAME: &str = "crate";

/// Index into [`Resolved::modules`]; the entry module is always `ModuleId(0)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ModuleId(pub u32);

/// Index into [`Symbols::fns`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FnId(pub u32);

/// Index into [`Symbols::imported`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ImportedFnId(pub u32);

/// Index into [`Symbols::structs`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct StructId(pub u32);

/// Index into [`Symbols::enums`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EnumId(pub u32);

/// A user-declared type: what a type name, a struct literal's path, and
/// an `impl` block resolve to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UserType {
    Struct(StructId),
    Enum(EnumId),
}

impl UserType {
    pub fn ty(self) -> Ty {
        match self {
            UserType::Struct(id) => Ty::Struct(id),
            UserType::Enum(id) => Ty::Enum(id),
        }
    }
}

/// What a module was loaded from.
#[derive(Debug, Clone, PartialEq)]
pub enum ModuleKind {
    /// A `.vr` file (or the entry file), parsed.
    Varyk(Program),
    /// A `.rs` file's source text, copied verbatim into the generated crate.
    Rust(String),
}

/// One loaded module: the entry file or a file named by a `mod` in a
/// `.vr` module (spec 3.1).
#[derive(Debug, Clone, PartialEq)]
pub struct Module {
    pub id: ModuleId,
    /// The `mod` name, or [`ENTRY_MODULE_NAME`] for the entry module.
    pub name: String,
    /// The module that declares this one; `None` for the entry module.
    pub parent: Option<ModuleId>,
    /// Declared `pub mod`; always false for the entry module.
    pub is_pub: bool,
    pub kind: ModuleKind,
    pub file: FileId,
    /// The directory this module's own `mod` declarations resolve in:
    /// the entry file's directory for the entry, else the parent's
    /// directory joined with this module's name, for `shop.vr` and
    /// `shop/mod.vr` alike.
    pub dir: PathBuf,
}

/// A Varyk function's resolved signature.
#[derive(Debug, Clone, PartialEq)]
pub struct FnSig {
    pub name: String,
    pub module: ModuleId,
    pub is_pub: bool,
    /// Name, type, and mode of each parameter (spec 4.2).
    pub params: Vec<(String, Ty, ParamMode)>,
    /// [`Ty::Unit`] when no return type is written.
    pub ret: Ty,
    /// The type of the `impl` block declaring the function; `None` for a
    /// free function (and for one in an `impl` block naming no type of
    /// its file, which is an error).
    pub owner: Option<UserType>,
    /// A method's receiver mode (spec 2.5): `self` is a shared borrow,
    /// `mut self` a mutable one. `None` for a free or associated
    /// function. The receiver is not in `params`.
    pub self_mode: Option<ParamMode>,
    /// Index of the declaring [`Item::Function`] or [`Item::Impl`] in the
    /// module's `Program::items`; see [`Resolved::fn_decl`].
    pub item: usize,
    /// For a function of an `impl` block, its index in
    /// [`ImplBlock::functions`].
    pub member: Option<usize>,
    /// Marked `#[test]` (M5a spec 2.7): run by `varyk test`, never called.
    pub is_test: bool,
    /// The whole declaration, starting at `fn` (or `pub`).
    pub span: Span,
}

/// A `pub fn` imported from a `.rs` module, mapped per the spec 4.5 table
/// (extended by M3 spec 4.4): a free function, or a method or associated
/// function of an imported struct (M3 spec 4.2).
#[derive(Debug, Clone, PartialEq)]
pub struct ImportedSig {
    pub name: String,
    pub module: ModuleId,
    /// The imported struct whose `impl` block declares it; `None` for a
    /// free function.
    pub owner: Option<StructId>,
    /// A method's receiver mode: `&self` is a shared borrow, `&mut self`
    /// a mutable one. `None` for a free or associated function. The
    /// receiver is not in `params`; [`ImportedSig::modes`] puts it first.
    pub self_mode: Option<ParamMode>,
    /// Empty when not `callable`: an uncallable signature is never
    /// type-checked.
    pub params: Vec<(Ty, ParamMode)>,
    /// [`Ty::Unit`] when not `callable`.
    pub ret: Ty,
    /// The parameter position (0 for `self`, as in [`ImportedSig::modes`])
    /// a borrowed `ret` is part of, when lifetime elision roots it there
    /// (M4 spec 2.12); `None` for an owned return.
    pub ret_root: Option<usize>,
    /// The original Rust signature text, for the V0108 diagnostic.
    pub signature: String,
    pub callable: bool,
    /// When not `callable`, or not usable from outside `within`, what to
    /// change, for the V0108 diagnostic.
    pub note: Option<String>,
    /// `Some` when the signature names a type behind a module only this
    /// module (and those inside it) can see: callable only from there
    /// (M3 spec 4.2), since the generated Rust names the type in full.
    pub within: Option<ModuleId>,
    /// When not `callable` because the `.rs` file's glob `use` or macro
    /// could redefine a type the signature names: the file's name, for
    /// the V0108 headline.
    pub redefined_in: Option<String>,
}

impl ImportedSig {
    /// The mode of every argument a call passes, the receiver's first for
    /// a method: the one place borrow analysis and the backend read an
    /// imported call's modes from.
    pub fn modes(&self) -> Vec<ParamMode> {
        self.self_mode
            .into_iter()
            .chain(self.params.iter().map(|(_, mode)| *mode))
            .collect()
    }
}

/// A struct's resolved definition.
#[derive(Debug, Clone, PartialEq)]
pub struct StructDef {
    pub name: String,
    pub module: ModuleId,
    pub is_pub: bool,
    /// In declaration order; a field index is a position in this list.
    pub fields: Vec<FieldDef>,
    /// Imported from a `.rs` module (M3 spec 4.1): checked like any
    /// struct, never emitted.
    pub imported: bool,
    /// For an imported struct, what its `#[derive(..)]` lists (M4 spec
    /// 2.12); `None` for a Varyk one, whose derives follow from its fields
    /// (see `types::derives`).
    pub derives: Option<Derives>,
    /// The whole declaration, starting at `struct` (or `pub`); for an
    /// imported struct, its name in the `.rs` file.
    pub span: Span,
}

/// One field of a resolved struct (spec 3.4). Its declaring module is
/// its struct's own: [`is_visible`] applied to `module` and `is_pub`
/// decides whether a use of it, or a struct literal naming it, is seen
/// from elsewhere.
#[derive(Debug, Clone, PartialEq)]
pub struct FieldDef {
    pub name: String,
    /// [`Ty::Unit`] when `unusable` and not usable anywhere.
    pub ty: Ty,
    pub is_pub: bool,
    /// For a field of an imported struct whose Rust type does not map
    /// (M3 spec 4.1): the type, and what to change.
    pub unusable: Option<Unusable>,
    /// Its attributes (M5a spec 2.2); none for an imported struct's field.
    pub attrs: FieldAttrs,
    /// The whole declaration, starting at the field's name (or `pub`);
    /// for an imported struct's field, its name in the `.rs` file.
    pub span: Span,
}

/// Why a field of an imported struct cannot be used from Varyk (V0108).
#[derive(Debug, Clone, PartialEq)]
pub struct Unusable {
    /// The field's Rust type as written.
    pub rust_ty: String,
    /// What to change, when there is more to say than the type.
    pub note: Option<String>,
    /// `Some` when the type maps but is behind a module only this module
    /// (and those inside it) can see: the field keeps its type and is
    /// usable from there.
    pub within: Option<ModuleId>,
}

/// An enum's resolved definition (spec 2.2).
#[derive(Debug, Clone, PartialEq)]
pub struct EnumDef {
    pub name: String,
    pub module: ModuleId,
    pub is_pub: bool,
    /// Each variant, in declaration order. Empty (a placeholder) for an
    /// `opaque` enum, whose real shape is never used.
    pub variants: Vec<VariantDef>,
    /// Imported from a `.rs` module (M3 spec 4.3): checked like any enum,
    /// never emitted.
    pub imported: bool,
    /// For an imported enum, what its `#[derive(..)]` lists (M4 spec
    /// 2.12); `None` for a Varyk one, whose derives follow from its
    /// payloads (see `types::derives`).
    pub derives: Option<Derives>,
    /// An enum with an `impl Drop` in some `.rs` module, or one Varyk
    /// cannot rule out: a `match` on a temporary of it only looks inside
    /// it, as on a stored value, since Rust cannot move a payload out of
    /// it (E0509).
    pub drops: Option<DropCause>,
    /// For an imported enum with a named-field variant or an unmappable
    /// payload (M3 spec 4.3): why. The enum itself is still a valid type
    /// (holdable, passable, returnable), but naming a variant to
    /// construct or match it is V0100.
    pub opaque: Option<String>,
    /// The whole declaration, starting at `enum` (or `pub`); for an
    /// imported enum, its name in the `.rs` file.
    pub span: Span,
}

/// Why an enum is treated as having a destructor (see `EnumDef::drops`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DropCause {
    /// An `impl Drop` in a `.rs` file names it.
    Impl,
    /// A `.rs` file might give it one: why, as "`helper.rs` has a macro
    /// whose text mentions `Drop`".
    Unsure(String),
}

impl DropCause {
    /// "runs", or "may run" when Varyk cannot tell, for "which runs code
    /// when it is thrown away".
    pub fn runs(&self) -> &'static str {
        match self {
            DropCause::Impl => "runs",
            DropCause::Unsure(_) => "may run",
        }
    }

    /// The cause, for inside parentheses after "nothing can take it out".
    pub fn aside(&self, owner: &str) -> String {
        match self {
            DropCause::Impl => format!("in Rust terms, `{owner}` implements `Drop`"),
            DropCause::Unsure(why) => {
                format!(
                    "Varyk cannot rule out that `{owner}` runs code when it is thrown away, \
                     because {why}"
                )
            }
        }
    }

    /// The note saying why a `match` on `owner` only borrows it.
    pub fn note(&self, owner: &str) -> String {
        match self {
            DropCause::Impl => format!(
                "in Rust terms, `{owner}` implements `Drop`, so `match` only borrows it and \
                 nothing can be moved out of it"
            ),
            DropCause::Unsure(why) => format!(
                "Varyk cannot rule out that `{owner}` runs code when it is thrown away (in Rust \
                 terms, whether it has an `impl Drop`), because {why}, so `match` only borrows \
                 it and nothing can be moved out of it"
            ),
        }
    }
}

impl EnumDef {
    /// The index of the variant called `name`.
    pub fn variant(&self, name: &str) -> Option<usize> {
        self.variants
            .iter()
            .position(|variant| variant.name == name)
    }
}

/// A variant of an enum (spec 2.2, M4 spec 2.5): its name and what it
/// holds.
#[derive(Debug, Clone, PartialEq)]
pub struct VariantDef {
    pub name: String,
    pub fields: VariantFieldsDef,
    /// `#[rename("key")]` on a unit variant (M5a spec 2.2): the key's text
    /// between the quotes, as written.
    pub rename: Option<String>,
}

/// What a variant holds: values by position, none for a unit variant
/// (`Point` is a `Tuple` of no types, written without `()`), or named
/// fields in declaration order.
#[derive(Debug, Clone, PartialEq)]
pub enum VariantFieldsDef {
    Tuple(Vec<Ty>),
    Named(Vec<(String, Ty)>),
}

impl VariantDef {
    /// The types the variant holds, in declaration order.
    pub fn types(&self) -> Vec<&Ty> {
        match &self.fields {
            VariantFieldsDef::Tuple(types) => types.iter().collect(),
            VariantFieldsDef::Named(fields) => fields.iter().map(|(_, ty)| ty).collect(),
        }
    }

    /// A unit variant: no values, no fields.
    pub fn is_unit(&self) -> bool {
        matches!(&self.fields, VariantFieldsDef::Tuple(types) if types.is_empty())
    }
}

/// A resolved function reference.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Callee {
    Varyk(FnId),
    Imported(ImportedFnId),
    /// An associated function of the built-in table: `Vec::new`.
    Builtin(BuiltinId),
}

/// Why a lookup failed. The type checker turns these into diagnostics at the use
/// site: `Unknown` is V0100, `NotVisible` is V0105.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LookupError {
    /// No such module, or no such item in it.
    Unknown,
    /// The item is not `pub`, and its module is neither the one looking
    /// nor an ancestor of it (spec 3.2). Holds the span
    /// of its declaration, whose start is where a `pub ` fix-it inserts,
    /// and the keyword it starts with (`fn`, `struct`, or `enum`).
    NotVisible { decl: Span, keyword: &'static str },
    /// A module on the way to the item is not `pub` and not visible from
    /// here (spec 3.2): the highest such module.
    PrivateModule { module: ModuleId },
    /// The path starts with `super` in the crate root (V0111); holds the
    /// span of the keyword.
    NoParent { span: Span },
}

/// Every module's functions, imported functions, structs, and enums, and
/// every type's methods and associated functions.
#[derive(Debug, Clone, Default)]
pub struct Symbols {
    pub fns: Vec<FnSig>,
    pub imported: Vec<ImportedSig>,
    pub structs: Vec<StructDef>,
    pub enums: Vec<EnumDef>,
    /// The functions of every type's `impl` blocks, by type then name: a
    /// Varyk function, or an imported one of a `.rs` struct.
    members: HashMap<(UserType, String), Callee>,
    /// The methods of an imported struct that exist in its `.rs` file but
    /// are not imported, by type then name: why, in words for a note.
    skipped_members: HashMap<(UserType, String), &'static str>,
    /// The methods of an imported struct with a restricted `pub(...)` in
    /// its `.rs` file, by type then name: the visibility as written.
    restricted_members: HashMap<(UserType, String), String>,
    /// Per-module name tables, indexed by `ModuleId`.
    scopes: Vec<Scope>,
    /// The package's dependencies, as Rust code names them (`-` as `_`).
    crates: Vec<String>,
}

#[derive(Debug, Clone, Default)]
struct Scope {
    /// How a "did you mean" note names this module from the crate root:
    /// `crate` for the root, `shop::cart` for a nested module.
    name: String,
    parent: Option<ModuleId>,
    /// Declared `pub mod`; false for the root.
    is_pub: bool,
    /// The `mod` declaration in the parent, where a `pub ` fix-it
    /// inserts; `None` for the root.
    decl: Option<Span>,
    /// The modules declared here, by name.
    children: HashMap<String, ModuleId>,
    fns: HashMap<String, Callee>,
    /// Structs and enums, which share Rust's type namespace.
    types: HashMap<String, UserType>,
    /// A `.rs` module's `pub` struct or enum that was not imported at all
    /// (M3 spec 4.1, 4.3, generic or the wrong shape): its kind (`struct`
    /// or `enum`) and why, for the note of a V0101 naming it.
    skipped: HashMap<String, (&'static str, String)>,
    /// A `.rs` module's free function with a restricted `pub(...)`, not
    /// imported: its visibility as written, for the V0105 naming it.
    restricted: HashMap<String, String>,
    /// A `.rs` module's plain-`pub` free function or `pub use` name that
    /// was not imported (`const`, behind `#[cfg]`, a re-export, ...): its
    /// kind for the headline and why, for the V0100 naming it.
    skipped_fns: HashMap<String, (&'static str, &'static str)>,
    /// A `.rs` module's top-level inline `mod`s, whose items Varyk does
    /// not read, for the V0100 naming one.
    inline_mods: HashSet<String>,
    /// For a `.rs` module with an item macro that could define type names:
    /// the note of an unusable signature, field, or payload it affects.
    expander: Option<String>,
    /// For a `.rs` module, its file's name (`ext.rs`), for messages.
    file: String,
    /// This file's `use` aliases of a module or type (spec 3.3), sharing
    /// the type namespace with `types` and `children`; consulted by
    /// lookups for the first segment of a path (or a bare name) before
    /// `types`. Never populated for another module, since a `use` is
    /// private to the file that wrote it. Always holds `UseTarget::Module`
    /// or `UseTarget::Type`, never `Fn` (that is `use_fns`, a separate
    /// namespace, so a module/type alias and a function alias of the same
    /// name can coexist, as they do in Rust).
    use_types: HashMap<String, UseTarget>,
    /// This file's `use` aliases of a function (spec 3.3), sharing the
    /// value namespace with `fns`. Always holds `UseTarget::Fn`.
    use_fns: HashMap<String, UseTarget>,
    /// Every `use` declaration's own resolved target, in the order its
    /// `Item::Use` appears in the file: `use_types`/`use_fns` are keyed by
    /// the introduced local name, which two declarations may share across
    /// namespaces (spec 2.1), so emission (one line per declaration, not
    /// per name) reads its target from here instead.
    use_order: Vec<UseTarget>,
}

impl Symbols {
    /// Looks up `name` (or `path::name`, spec 3.1) as seen from module
    /// `from`. An item of a module that is not `from` or an ancestor must
    /// be `pub`, and so must every module on the way that is not visible
    /// from `from` (spec 3.2); imported Rust functions are always `pub`.
    pub fn lookup_fn(
        &self,
        from: ModuleId,
        module: Option<&Path>,
        name: &str,
    ) -> Result<Callee, LookupError> {
        // A bare name (no path segment before it) may be a `use` alias of
        // `from`'s own file (spec 3.3); already visibility-checked when it
        // was registered, so it returns straight away.
        if module.is_none() {
            if let Some(UseTarget::Fn(callee)) = self.scopes[from.0 as usize].use_fns.get(name) {
                return Ok(*callee);
            }
        }
        let target = self.target_module(from, module)?;
        let callee = *self.scopes[target.0 as usize]
            .fns
            .get(name)
            .ok_or(LookupError::Unknown)?;
        // An imported Rust function is always `pub`, but its module may
        // not be.
        let private = match callee {
            Callee::Varyk(id) => {
                let sig = &self.fns[id.0 as usize];
                (!sig.is_pub).then_some((sig.span, "fn"))
            }
            Callee::Imported(_) | Callee::Builtin(_) => None,
        };
        match visibility::check_visible(self, from, target, private) {
            Some(err) => Err(err),
            None => Ok(callee),
        }
    }

    /// Looks up a struct or enum `name` (or `path::name`) as seen from
    /// module `from` (spec 2.10, 3.1), visible by the rule of
    /// [`Self::lookup_fn`].
    pub fn lookup_type(
        &self,
        from: ModuleId,
        module: Option<&Path>,
        name: &str,
    ) -> Result<UserType, LookupError> {
        // Same `use`-alias shortcut as `lookup_fn`.
        if module.is_none() {
            if let Some(UseTarget::Type(found)) = self.scopes[from.0 as usize].use_types.get(name) {
                return Ok(*found);
            }
        }
        let target = self.target_module(from, module)?;
        let found = *self.scopes[target.0 as usize]
            .types
            .get(name)
            .ok_or(LookupError::Unknown)?;
        let (is_pub, decl, keyword) = match found {
            UserType::Struct(id) => {
                let def = &self.structs[id.0 as usize];
                (def.is_pub, def.span, "struct")
            }
            UserType::Enum(id) => {
                let def = &self.enums[id.0 as usize];
                (def.is_pub, def.span, "enum")
            }
        };
        let private = (!is_pub).then_some((decl, keyword));
        match visibility::check_visible(self, from, target, private) {
            Some(err) => Err(err),
            None => Ok(found),
        }
    }

    /// Looks up a struct by name in module `from` itself.
    pub fn lookup_struct(&self, from: ModuleId, name: &str) -> Option<StructId> {
        match self.scopes[from.0 as usize].types.get(name) {
            Some(UserType::Struct(id)) => Some(*id),
            _ => None,
        }
    }

    /// Looks up a method or associated function `name` of type `owner` as
    /// seen from module `from` (spec 2.5), visible by the rule of
    /// [`Self::lookup_fn`] applied to the module declaring it.
    pub fn lookup_member(
        &self,
        from: ModuleId,
        owner: UserType,
        name: &str,
    ) -> Result<Callee, LookupError> {
        let found = *self
            .members
            .get(&(owner, name.to_string()))
            .ok_or(LookupError::Unknown)?;
        let (module, private) = match found {
            Callee::Varyk(id) => {
                let sig = &self.fns[id.0 as usize];
                (sig.module, (!sig.is_pub).then_some((sig.span, "fn")))
            }
            // An imported method is always `pub`, but its module may not
            // be.
            Callee::Imported(id) => (self.imported[id.0 as usize].module, None),
            Callee::Builtin(_) => unreachable!("a type's members are never built-ins"),
        };
        match visibility::check_visible(self, from, module, private) {
            Some(err) => Err(err),
            None => Ok(found),
        }
    }

    /// The note of a V0100 for a member `name` of `owner` that is not
    /// found, when the `.rs` file has it but Varyk did not import it.
    pub fn skipped_member_note(&self, owner: UserType, name: &str) -> Option<String> {
        if let Some(vis) = self.restricted_members.get(&(owner, name.to_string())) {
            return Some(format!(
                "`{name}` is `{vis}` in the Rust file; Varyk imports only plain `pub`, so write \
                 `pub fn {name}` in the Rust file"
            ));
        }
        let why = *self.skipped_members.get(&(owner, name.to_string()))?;
        let owner = match owner {
            UserType::Struct(id) => &self.structs[id.0 as usize].name,
            UserType::Enum(id) => &self.enums[id.0 as usize].name,
        };
        if why == crate::interop::ENUM_METHOD {
            let param = owner
                .chars()
                .next()
                .map_or_else(String::new, |c| c.to_ascii_lowercase().to_string());
            return Some(format!(
                "`{name}` exists in the Rust file, but Varyk does not import methods of Rust \
                 enums yet; add a `pub fn {name}({param}: &{owner})` instead"
            ));
        }
        if why == crate::interop::CFG_PARAM {
            return Some(format!(
                "a parameter of `{name}` is behind `#[cfg]`, so `{name}` may not exist with this \
                 shape in the build; Varyk does not import such methods"
            ));
        }
        let fix = if why == "a trait method" {
            format!("add a `pub fn {name}` to a plain `impl {owner}` block")
        } else if why == crate::interop::CFG || why == crate::interop::TEST {
            "such a method may not exist in the build".to_string()
        } else {
            format!("call it from a plain `pub fn` of another name in an `impl {owner}` block")
        };
        Some(format!(
            "`{name}` exists in the Rust file but is {why}; Varyk does not import such methods; \
             {fix}"
        ))
    }

    /// V0101 at `span` when the type `name` of module `path` (written
    /// `full`) is a name a `pub use` of a `.rs` module brings in, which
    /// Varyk does not import; `None` otherwise.
    fn reexported_type(
        &self,
        from: ModuleId,
        path: &Path,
        name: &str,
        full: &str,
        span: Span,
    ) -> Option<Diagnostic> {
        let target = self.target_module(from, Some(path)).ok()?;
        let scope = &self.scopes[target.0 as usize];
        let (_, why) = scope.skipped_fns.get(name)?;
        if *why != crate::interop::REEXPORT {
            return None;
        }
        Some(
            Diagnostic::new(
                codes::V0101,
                span,
                format!("Varyk cannot use the Rust type `{full}`"),
            )
            .with_note(format!(
                "`{name}` is not imported because it is a `pub use` re-export in `{}`; write the \
                 type's own `pub struct` in that file, or, if it comes from another module of \
                 this package, name that module",
                scope.file
            )),
        )
    }

    /// V0101 at `span` when `path` (written `full`) names a `.rs` struct
    /// or enum Varyk did not import (M3 spec 4.1, 4.3), saying why, V0105
    /// when it names a `.rs` function with a restricted `pub(...)`, or
    /// V0100 saying why when it names another `.rs` function or `pub use`
    /// Varyk did not import; `None` when it names no such item.
    pub fn skipped_item(
        &self,
        from: ModuleId,
        module: Option<&Path>,
        name: &str,
        full: &str,
        span: Span,
    ) -> Option<Diagnostic> {
        let target = self.target_module(from, module).ok()?;
        let scope = &self.scopes[target.0 as usize];
        if scope.inline_mods.contains(name) {
            return Some(
                Diagnostic::new(
                    codes::V0100,
                    span,
                    format!(
                        "`{full}` is a module inside the Rust file `{}`, and Varyk cannot look \
                         into it",
                        scope.file
                    ),
                )
                .with_note(format!(
                    "Varyk reads only the top-level items of a Rust file; move what you need \
                     out of `mod {name}` to the top level of `{}`, or into a `.rs` file of its own",
                    scope.file
                )),
            );
        }
        if let Some(vis) = scope.restricted.get(name) {
            return Some(
                Diagnostic::new(
                    codes::V0105,
                    span,
                    format!(
                        "`{full}` is `{vis}` in the Rust file; Varyk imports only plain `pub` \
                         functions"
                    ),
                )
                .with_note(format!(
                    "write `pub fn {name}` in the Rust file to use it here"
                )),
            );
        }
        if let Some((kind, why)) = scope.skipped_fns.get(name) {
            let note = if *why == crate::interop::CFG_PARAM {
                format!(
                    "a parameter of `{name}` is behind `#[cfg]`, so `{name}` may not exist with \
                     this shape in the build; Varyk does not import such {kind}s"
                )
            } else {
                format!(
                    "`{name}` exists in the Rust file but is {why}; Varyk does not import such \
                     {kind}s"
                )
            };
            return Some(
                Diagnostic::new(codes::V0100, span, format!("cannot find {kind} `{full}`"))
                    .with_note(note),
            );
        }
        let (kind, why) = scope.skipped.get(name)?;
        Some(
            Diagnostic::new(
                codes::V0101,
                span,
                format!("Varyk cannot use the Rust {kind} `{full}`"),
            )
            .with_note(skipped_note(kind, name, why)),
        )
    }

    /// Resolves a written type as seen from module `from`: a primitive
    /// name, `string`, a struct or enum visible from `from` (through a
    /// module, `m::User`, when it is `pub`), or `Option`, `Result`, or
    /// `Vec` with the right number of type arguments (spec 2.6). `String`
    /// and `str` are V0107 with a fix-it to `string`; an unknown name or a
    /// wrong argument count is V0101.
    #[expect(
        clippy::result_large_err,
        reason = "a single diagnostic on a cold path; the type checker pushes it straight into its list"
    )]
    pub fn resolve_type(&self, ty: &TypeExpr, from: ModuleId) -> Result<Ty, Diagnostic> {
        let name = ty.name.name.as_str();
        let span = ty.name.span;
        if let Some((arity, shape)) = generic_shape(name).filter(|_| ty.path.is_none()) {
            if ty.args.len() != arity {
                return Err(Diagnostic::new(
                    codes::V0101,
                    ty.span,
                    format!(
                        "`{name}` takes {} type{}, written `{shape}`",
                        if arity == 1 { "one" } else { "two" },
                        if arity == 1 { "" } else { "s" },
                    ),
                ));
            }
            let mut args = Vec::new();
            for arg in &ty.args {
                args.push(self.resolve_type(arg, from)?);
            }
            if let (Some(key), true) = (args.first(), name == "HashMap") {
                if !key.is_map_key() {
                    return Err(Diagnostic::new(
                        codes::V0101,
                        ty.args[0].span,
                        "a `HashMap` key must be of an integer type, `bool`, or `string`",
                    )
                    .with_note(
                        "a key is compared and hashed to find its value; in Rust terms, the key \
                         type must implement `Eq` and `Hash`, which floats and Varyk's own \
                         types do not",
                    ));
                }
            }
            let mut args = args.into_iter().map(Box::new);
            let mut next = || args.next().unwrap_or_else(|| Box::new(Ty::Unit));
            return Ok(match name {
                "Option" => Ty::Option(next()),
                "Vec" => Ty::Vec(next()),
                "HashMap" => Ty::HashMap(next(), next()),
                _ => Ty::Result(next(), next()),
            });
        }
        if !ty.args.is_empty() {
            return Err(Diagnostic::new(
                codes::V0001,
                ty.span,
                format!(
                    "`{name}` takes no type arguments; only `Option`, `Result`, `Vec`, and \
                     `HashMap` do"
                ),
            ));
        }
        if let Some(path) = &ty.path {
            let full = display_path(path, name);
            return match self.lookup_type(from, Some(path), name) {
                Ok(found) => Ok(found.ty()),
                Err(LookupError::Unknown) => Err(self
                    .reexported_type(from, path, name, &full, ty.span)
                    .or_else(|| self.skipped_item(from, Some(path), name, &full, ty.span))
                    .unwrap_or_else(|| {
                        Diagnostic::new(codes::V0101, ty.span, format!("unknown type `{full}`"))
                    })),
                Err(LookupError::NotVisible { decl, keyword }) => {
                    Err(not_visible("type", &full, ty.span, decl, keyword))
                }
                Err(LookupError::PrivateModule { module }) => {
                    Err(self.private_module(module, "type", &full, ty.span))
                }
                Err(LookupError::NoParent { span }) => Err(no_parent(span)),
            };
        }
        if let Some(prim) = Ty::from_primitive_name(name) {
            return Ok(prim);
        }
        // No struct or enum can take the name (V0113), so it is always
        // the standard one (M5a spec 2.3).
        if name == "Error" {
            return Ok(Ty::Error);
        }
        if name == "String" || name == "str" {
            return Err(Diagnostic::new(
                codes::V0107,
                span,
                format!("`{name}` is not a Varyk type; Varyk's string type is `string`"),
            )
            .with_fix_it(FixIt {
                span,
                replacement: "string".to_string(),
            }));
        }
        self.lookup_type(from, None, name)
            .map(UserType::ty)
            .map_err(|_| {
                let mut diagnostic =
                    Diagnostic::new(codes::V0101, span, format!("unknown type `{name}`"));
                if let Some(note) = self.did_you_mean(from, name) {
                    diagnostic = diagnostic.with_note(note);
                }
                diagnostic
            })
    }

    /// Modules other than `from` that declare a type or a function named
    /// `name`, for [`Self::did_you_mean`], each as a path usable from
    /// `from`: from the root, `shop::cart`; from anywhere else,
    /// `crate::shop::cart`.
    fn modules_declaring(&self, from: ModuleId, name: &str) -> Vec<String> {
        self.scopes
            .iter()
            .enumerate()
            .filter(|&(id, scope)| {
                id as u32 != from.0
                    && (scope.types.contains_key(name) || scope.fns.contains_key(name))
            })
            .map(|(id, scope)| {
                if id == 0 || from == ModuleId(0) {
                    scope.name.clone()
                } else {
                    format!("{ENTRY_MODULE_NAME}::{}", scope.name)
                }
            })
            .collect()
    }

    /// A "did you mean `module::name`?" note naming up to two other
    /// modules that declare a type or a function called `name`: `None`
    /// when none do. For an unknown type (V0101) or an unknown value or
    /// associated call whose leading segment was taken for a module
    /// (V0100), written bare instead of through its module.
    pub fn did_you_mean(&self, from: ModuleId, name: &str) -> Option<String> {
        let modules = self.modules_declaring(from, name);
        if modules.is_empty() {
            return None;
        }
        let paths: Vec<String> = modules
            .iter()
            .take(2)
            .map(|module| format!("`{module}::{name}`"))
            .collect();
        Some(format!("did you mean {}?", paths.join(" or ")))
    }

    /// `from` itself for an unqualified name; otherwise the module `module`
    /// names from `from`.
    fn target_module(
        &self,
        from: ModuleId,
        module: Option<&Path>,
    ) -> Result<ModuleId, LookupError> {
        match module {
            None => Ok(from),
            Some(path) => self.module_at(from, path),
        }
    }
}

/// The argument count and written shape of `Option`, `Result`, `Vec`, and
/// `HashMap`; `None` for any other name.
fn generic_shape(name: &str) -> Option<(usize, &'static str)> {
    match name {
        "Option" => Some((1, "Option<T>")),
        "Vec" => Some((1, "Vec<T>")),
        "Result" => Some((2, "Result<T, E>")),
        "HashMap" => Some((2, "HashMap<K, V>")),
        _ => None,
    }
}

/// The resolver's output: every loaded module and every module's symbols.
#[derive(Debug, Clone)]
pub struct Resolved {
    pub modules: Vec<Module>,
    pub symbols: Symbols,
    pub entry: ModuleId,
    /// Whether the entry is a program's root or a library's (M3 spec 2.1).
    pub kind: Kind,
}

impl Resolved {
    /// The AST of a Varyk function.
    pub fn fn_decl(&self, id: FnId) -> &Function {
        let sig = &self.symbols.fns[id.0 as usize];
        let ModuleKind::Varyk(program) = &self.modules[sig.module.0 as usize].kind else {
            unreachable!("a Varyk function always lives in a Varyk module");
        };
        match (&program.items[sig.item], sig.member) {
            (Item::Function(function), None) => function,
            (Item::Impl(block), Some(member)) => &block.functions[member],
            _ => unreachable!("FnSig::item indexes a function, or an impl block with `member`"),
        }
    }
}

/// Resolves the program rooted at `entry`, a single file or a binary
/// package's root with no dependencies; see [`resolve_root`].
pub fn resolve(
    entry: SourceFile,
    sources: &mut Vec<SourceFile>,
) -> Result<Resolved, Vec<Diagnostic>> {
    resolve_root(entry, Kind::Binary, None, sources)
}

/// A package's dependency names, as its manifest writes them.
#[derive(Debug, Clone, Copy)]
pub struct Dependencies<'a> {
    /// `[dependencies]`: the crates a `.rs` module may use.
    pub crates: &'a [String],
    /// `[dev-dependencies]`: only tests may use them, which a message
    /// about a `.rs` module naming one says.
    pub dev: &'a [String],
}

/// Resolves the program rooted at `entry`, a crate root of `kind` (a
/// binary must define `main`, a library must not) whose package has
/// `dependencies` (a `use` naming one of its crates is V0110), or a
/// single file when `dependencies` is `None`.
///
/// Pushes `entry` onto `sources` (the caller builds it with
/// `FileId(sources.len())`, so its id is its index), then every module
/// file, each with the next `FileId`. Returns every diagnostic collected,
/// or the resolved modules and symbols if there were none.
pub fn resolve_root(
    entry: SourceFile,
    kind: Kind,
    dependencies: Option<Dependencies<'_>>,
    sources: &mut Vec<SourceFile>,
) -> Result<Resolved, Vec<Diagnostic>> {
    let entry_file = entry.id;
    let entry_path = entry.path.clone();
    sources.push(entry);
    let program = parse(sources.last().expect("just pushed"))?;

    let mut diagnostics = Vec::new();
    let mut modules = vec![Module {
        id: ModuleId(0),
        name: ENTRY_MODULE_NAME.to_string(),
        parent: None,
        is_pub: false,
        kind: ModuleKind::Varyk(program),
        file: entry_file,
        dir: entry_path
            .parent()
            .unwrap_or(std::path::Path::new(""))
            .to_path_buf(),
    }];
    let mut imports = Vec::new();
    if !load_modules(
        dependencies,
        sources,
        &mut modules,
        &mut imports,
        &mut diagnostics,
    ) {
        return Err(diagnostics);
    }

    let crates = dependencies.map_or(&[][..], |dependencies| dependencies.crates);
    let symbols = collect_symbols(&modules, imports, crates, &mut diagnostics);
    check_entry_main(&modules[0], kind, &mut diagnostics);

    if diagnostics.is_empty() {
        Ok(Resolved {
            modules,
            symbols,
            entry: ModuleId(0),
            kind,
        })
    } else {
        Err(diagnostics)
    }
}

/// Lexes and parses `file`, converting any syntax errors.
fn parse(file: &SourceFile) -> Result<Program, Vec<Diagnostic>> {
    let (program, errors) = parse_source(file);
    if errors.is_empty() {
        Ok(program)
    } else {
        Err(errors.into_iter().map(Diagnostic::from).collect())
    }
}

/// V0103 at `span` for a struct, enum, or module named after a built-in
/// type or one of the reserved names of spec 2.1: these share Rust's type
/// namespace with the primitives, `String`, `Option`, `Result`, and `Vec`,
/// so a user item with one of those names would capture them in rustc
/// (`mod String;` makes every `String` a module).
/// `Error` is V0113 instead: the standard error type's name.
pub(crate) fn reserved_type_name(name: &str, span: Span) -> Option<Diagnostic> {
    if name == "Error" {
        return Some(error_name_taken(span));
    }
    let primitive = Ty::from_primitive_name(name).is_some() || matches!(name, "string" | "str");
    if primitive {
        return Some(taken(name, "a built-in type", span));
    }
    reserved_value_name(name, span)
}

/// V0103 at `span` for a function, method, variant, field, parameter, or
/// `let` named with one of the reserved names of spec 2.1 (`Some`, `None`,
/// `Ok`, `Err`, `Option`, `Result`, `Vec`, `String`, and M4's `HashMap`):
/// the generated Rust would hide Rust's own (`let None = x;` is a
/// pattern). The primitive names stay usable here; Rust keeps values and
/// types apart.
pub(crate) fn reserved_value_name(name: &str, span: Span) -> Option<Diagnostic> {
    let owner = match name {
        "Some" | "None" => "`Option`",
        "Ok" | "Err" => "`Result`",
        "Option" | "Result" | "Vec" | "HashMap" | "String" => "a built-in type",
        _ => return None,
    };
    Some(taken(name, owner, span))
}

/// V0113 at `span` for a struct, enum, module, `use` alias, or `.rs`
/// struct or enum named `Error`, the standard error type (M5a spec 2.10).
pub(crate) fn error_name_taken(span: Span) -> Diagnostic {
    Diagnostic::new(
        codes::V0113,
        span,
        "the name `Error` is already taken by the standard error type",
    )
    .with_note(
        "every standard call that can fail gives an `Error`, so a type of your own needs \
         another name, such as `ParseError` or `Problem`",
    )
}

/// V0113 at `span` for a module, struct, or enum named `json`, `env`, or `log`, the
/// standard modules (M5a spec 2.10).
pub(crate) fn std_module_taken(name: &str, span: Span) -> Option<Diagnostic> {
    if !is_std_module(name) {
        return None;
    }
    Some(
        Diagnostic::new(
            codes::V0113,
            span,
            format!("the name `{name}` is already taken by the standard `{name}` module"),
        )
        .with_note(format!(
            "the standard `{name}` module is reached by its path wherever it is used, as in \
             `{name}::..`, so a module, struct, or enum of your own needs another name"
        )),
    )
}

/// `json`, `env`, and `log`, the standard modules (M5a spec 2.10).
pub(crate) fn is_std_module(name: &str) -> bool {
    matches!(name, "json" | "env" | "log")
}

/// V0113 at `span` for a function named `assert` or `assert_eq`, the
/// standard checks of a test (M5a spec 2.7, 2.10).
fn std_fn_taken(name: &str, span: Span) -> Option<Diagnostic> {
    if !matches!(name, "assert" | "assert_eq") {
        return None;
    }
    Some(
        Diagnostic::new(
            codes::V0113,
            span,
            format!("the name `{name}` is already taken by the standard check `{name}`"),
        )
        .with_note(
            "`assert` and `assert_eq` check results inside a `#[test]` function, so a \
             function of your own needs another name",
        ),
    )
}

/// V0113 at `span` for an item, method, module, or `use` name starting
/// with `varyk_`, kept for the functions the compiler adds to the
/// generated Rust (M5a spec 2.10).
pub(crate) fn varyk_prefix_taken(name: &str, span: Span) -> Option<Diagnostic> {
    if !name.starts_with("varyk_") {
        return None;
    }
    Some(
        Diagnostic::new(
            codes::V0113,
            span,
            format!("the name `{name}` starts with `varyk_`"),
        )
        .with_note(
            "names starting with `varyk_` are kept for what Varyk adds to the Rust it writes, \
             so this needs another name",
        ),
    )
}

fn taken(name: &str, owner: &str, span: Span) -> Diagnostic {
    Diagnostic::new(
        codes::V0103,
        span,
        format!("the name `{name}` is already taken by {owner}"),
    )
}

/// V0103 at `span`, labelling the first definition at `first`.
fn duplicate(name: &str, span: Span, first: Span) -> Diagnostic {
    Diagnostic::new(
        codes::V0103,
        span,
        format!("the name `{name}` is defined more than once"),
    )
    .with_label(first, format!("`{name}` first defined here"))
}

/// Builds the symbol tables: first every name (so types can refer to
/// types declared later or in any order), then every `impl` block's
/// functions attached to its type, then field, payload, and signature
/// types.
fn collect_symbols(
    modules: &[Module],
    imports: Vec<(ModuleId, ImportedModule)>,
    crates: &[String],
    diagnostics: &mut Vec<Diagnostic>,
) -> Symbols {
    let mut symbols = Symbols {
        crates: crates.iter().map(|name| name.replace('-', "_")).collect(),
        scopes: modules
            .iter()
            .map(|module| Scope {
                name: module.name.clone(),
                parent: module.parent,
                is_pub: module.is_pub,
                decl: mod_decl(modules, module),
                ..Scope::default()
            })
            .collect(),
        ..Symbols::default()
    };
    // Children and the "did you mean" names, parents first: modules are
    // loaded breadth-first, so a parent's name is final before its
    // children's.
    for module in &modules[1..] {
        let parent = module.parent.expect("only the entry module has no parent");
        let parent_scope = &mut symbols.scopes[parent.0 as usize];
        parent_scope.children.insert(module.name.clone(), module.id);
        if parent != ModuleId(0) {
            let name = format!("{}::{}", parent_scope.name, module.name);
            symbols.scopes[module.id.0 as usize].name = name;
        }
    }
    // Every `impl` block with its module and the id of its first function.
    let mut impls: Vec<(ModuleId, &ImplBlock, u32)> = Vec::new();

    // Pass 1: names. Duplicates still get a table entry (so their types
    // are checked too) but not a scope entry.
    for module in modules {
        let ModuleKind::Varyk(program) = &module.kind else {
            continue;
        };
        let is_entry = module.id == ModuleId(0);
        let scope = &mut symbols.scopes[module.id.0 as usize];
        let mut first_fn: HashMap<&str, Span> = HashMap::new();
        let mut first_type: HashMap<&str, Span> = HashMap::new();
        // Modules share Rust's type namespace with structs and enums, so
        // a type named like a module loaded here is a duplicate (rustc
        // E0428).
        for item in &program.items {
            let Item::Mod(decl) = item else {
                continue;
            };
            if scope.children.contains_key(&decl.name.name) {
                first_type.entry(&decl.name.name).or_insert(decl.name.span);
            }
        }

        for (index, item) in program.items.iter().enumerate() {
            match item {
                Item::Function(function) => {
                    let name = &function.name;
                    if !is_entry && name.name == "main" {
                        diagnostics.push(Diagnostic::new(
                            codes::V0106,
                            header(function),
                            "a module must not define `main`",
                        ));
                    }
                    let id = push_fn(&mut symbols.fns, function, module.id, index, None);
                    let placed = attrs::placed(&function.attrs, Place::Function, diagnostics);
                    if !placed.is_empty() {
                        symbols.fns[id.0 as usize].is_test = true;
                        check_test_shape(function, diagnostics);
                    }
                    diagnostics.extend(reserved_value_name(&name.name, name.span));
                    diagnostics.extend(std_fn_taken(&name.name, name.span));
                    diagnostics.extend(varyk_prefix_taken(&name.name, name.span));
                    match first_fn.entry(&name.name) {
                        Entry::Occupied(first) => {
                            diagnostics.push(duplicate(&name.name, name.span, *first.get()));
                        }
                        Entry::Vacant(slot) => {
                            slot.insert(name.span);
                            scope.fns.insert(name.name.clone(), Callee::Varyk(id));
                        }
                    }
                }
                Item::Struct(decl) => {
                    let name = &decl.name;
                    // Reported, but still registered: the later passes walk
                    // the struct declarations in the same order by id.
                    diagnostics.extend(reserved_type_name(&name.name, name.span));
                    diagnostics.extend(std_module_taken(&name.name, name.span));
                    diagnostics.extend(varyk_prefix_taken(&name.name, name.span));
                    attrs::placed(&decl.attrs, Place::Struct, diagnostics);
                    let id = StructId(symbols.structs.len() as u32);
                    symbols.structs.push(StructDef {
                        name: name.name.clone(),
                        module: module.id,
                        is_pub: decl.is_pub,
                        imported: false,
                        derives: None,
                        fields: Vec::new(),
                        span: decl.span,
                    });
                    match first_type.entry(&name.name) {
                        Entry::Occupied(first) => {
                            diagnostics.push(duplicate(&name.name, name.span, *first.get()));
                        }
                        Entry::Vacant(slot) => {
                            slot.insert(name.span);
                            scope.types.insert(name.name.clone(), UserType::Struct(id));
                        }
                    }
                }
                Item::Enum(decl) => {
                    let name = &decl.name;
                    diagnostics.extend(reserved_type_name(&name.name, name.span));
                    diagnostics.extend(std_module_taken(&name.name, name.span));
                    diagnostics.extend(varyk_prefix_taken(&name.name, name.span));
                    attrs::placed(&decl.attrs, Place::Enum, diagnostics);
                    let id = EnumId(symbols.enums.len() as u32);
                    // Variant names are known now; pass 2 fills in their
                    // payload types (and pass 1b already needs the names).
                    let variants = decl
                        .variants
                        .iter()
                        .map(|variant| VariantDef {
                            name: variant.name.name.clone(),
                            fields: VariantFieldsDef::Tuple(Vec::new()),
                            rename: None,
                        })
                        .collect();
                    symbols.enums.push(EnumDef {
                        name: name.name.clone(),
                        module: module.id,
                        is_pub: decl.is_pub,
                        variants,
                        imported: false,
                        derives: None,
                        drops: None,
                        opaque: None,
                        span: decl.span,
                    });
                    match first_type.entry(&name.name) {
                        Entry::Occupied(first) => {
                            diagnostics.push(duplicate(&name.name, name.span, *first.get()));
                        }
                        Entry::Vacant(slot) => {
                            slot.insert(name.span);
                            scope.types.insert(name.name.clone(), UserType::Enum(id));
                        }
                    }
                }
                Item::Impl(block) => {
                    attrs::placed(&block.attrs, Place::Impl, diagnostics);
                    let first = symbols.fns.len() as u32;
                    for (member, function) in block.functions.iter().enumerate() {
                        let name = &function.name;
                        attrs::placed(&function.attrs, Place::Method, diagnostics);
                        diagnostics.extend(reserved_value_name(&name.name, name.span));
                        diagnostics.extend(varyk_prefix_taken(&name.name, name.span));
                        push_fn(&mut symbols.fns, function, module.id, index, Some(member));
                    }
                    impls.push((module.id, block, first));
                }
                Item::Mod(decl) => {
                    attrs::placed(&decl.attrs, Place::Mod, diagnostics);
                }
                // `use` introduces no symbol of its own in this pass: it
                // names an existing one, so it is resolved once every
                // module's own names (and, further below, its imports) are
                // known, in `uses::register`.
                Item::Use(decl) => {
                    attrs::placed(&decl.attrs, Place::Use, diagnostics);
                }
            }
        }
    }

    // Pass 1b: every `impl` block's functions join their type's table.
    // Several blocks for one type share it.
    let mut first_member: HashMap<(UserType, &str), Span> = HashMap::new();
    for (module, block, first) in impls {
        let type_name = &block.type_name;
        let Some(&owner) = symbols.scopes[module.0 as usize].types.get(&type_name.name) else {
            diagnostics.push(Diagnostic::new(
                codes::V0001,
                type_name.span,
                format!(
                    "an `impl` block must name a struct or enum declared in the same file, and this file declares no `{}`",
                    type_name.name
                ),
            ));
            continue;
        };
        for (offset, function) in block.functions.iter().enumerate() {
            let id = FnId(first + offset as u32);
            symbols.fns[id.0 as usize].owner = Some(owner);
            let name = &function.name;
            // `E::Make` must mean one thing: a function named after one of
            // the enum's own variants would be unreachable behind it.
            if let UserType::Enum(enum_id) = owner {
                let def = &symbols.enums[enum_id.0 as usize];
                if def.variant(&name.name).is_some() {
                    let owner_text = format!("a variant of `{}`", def.name);
                    diagnostics.push(taken(&name.name, &owner_text, name.span));
                    continue;
                }
            }
            match first_member.entry((owner, &name.name)) {
                Entry::Occupied(first) => {
                    diagnostics.push(duplicate(&name.name, name.span, *first.get()));
                }
                Entry::Vacant(slot) => {
                    slot.insert(name.span);
                    symbols
                        .members
                        .insert((owner, name.name.clone()), Callee::Varyk(id));
                }
            }
        }
    }

    imports::register(&mut symbols, modules, imports);

    // Pass 1c: `use` declarations (spec 3.3). Every module's own fns,
    // types, children, and imports are known now, in any order across
    // files, so a `use` can name any of them; pass 2 (right below) then
    // sees every alias while resolving field and signature types.
    uses::register(&mut symbols, modules, diagnostics);

    // Pass 2: types, in declaration order. Tables were filled in the same
    // order, so a running index finds each item's entry.
    let (mut next_fn, mut next_struct, mut next_enum) = (0, 0, 0);
    // Every resolved field or payload type with the span it was written
    // at, per struct and per enum, for the recursive-type check below.
    let mut struct_parts: Vec<Vec<(Ty, Span)>> = Vec::new();
    let mut enum_parts: Vec<Vec<(Ty, Span)>> = Vec::new();
    for module in modules {
        let ModuleKind::Varyk(program) = &module.kind else {
            continue;
        };
        for item in &program.items {
            match item {
                Item::Function(function) => {
                    let reach = symbols.fn_reach(FnId(next_fn as u32));
                    let (params, ret) =
                        resolve_signature(&symbols, function, module.id, reach, diagnostics);
                    let sig = &mut symbols.fns[next_fn];
                    sig.params = params;
                    sig.ret = ret;
                    next_fn += 1;
                }
                Item::Impl(block) => {
                    for function in &block.functions {
                        let reach = symbols.fn_reach(FnId(next_fn as u32));
                        let (params, ret) =
                            resolve_signature(&symbols, function, module.id, reach, diagnostics);
                        let sig = &mut symbols.fns[next_fn];
                        sig.params = params;
                        sig.ret = ret;
                        next_fn += 1;
                    }
                }
                Item::Struct(decl) => {
                    let mut seen: HashMap<&str, Span> = HashMap::new();
                    let mut fields = Vec::new();
                    let mut parts = Vec::new();
                    for field in &decl.fields {
                        let name = &field.name;
                        diagnostics.extend(reserved_value_name(&name.name, name.span));
                        if let Some(&first) = seen.get(name.name.as_str()) {
                            diagnostics.push(duplicate(&name.name, name.span, first));
                        }
                        seen.entry(&name.name).or_insert(name.span);
                        let placed = attrs::placed(&field.attrs, Place::Field, diagnostics);
                        match symbols.resolve_type(&field.ty, module.id) {
                            Ok(ty) => {
                                let attrs = field_attrs(&placed, &ty, &field.ty, diagnostics);
                                // Only a `pub` field's type must be as
                                // widely visible as its callers (spec
                                // 3.4): a private field may have a
                                // private type, as in Rust.
                                if field.is_pub {
                                    diagnostics.extend(symbols.private_in_public(
                                        &ty,
                                        field.ty.span,
                                        &decl.name.name,
                                        symbols.reach(module.id, decl.is_pub),
                                    ));
                                }
                                parts.push((ty.clone(), field.span));
                                fields.push(FieldDef {
                                    name: name.name.clone(),
                                    ty,
                                    is_pub: field.is_pub,
                                    unusable: None,
                                    attrs,
                                    span: field.span,
                                });
                            }
                            Err(diagnostic) => diagnostics.push(diagnostic),
                        }
                    }
                    symbols.structs[next_struct].fields = fields;
                    struct_parts.push(parts);
                    next_struct += 1;
                }
                Item::Enum(decl) => {
                    let mut seen: HashMap<&str, Span> = HashMap::new();
                    let mut variants = Vec::new();
                    let mut parts = Vec::new();
                    for variant in &decl.variants {
                        let name = &variant.name;
                        diagnostics.extend(reserved_value_name(&name.name, name.span));
                        if let Some(&first) = seen.get(name.name.as_str()) {
                            diagnostics.push(duplicate(&name.name, name.span, first));
                        }
                        seen.entry(&name.name).or_insert(name.span);
                        let place = match &variant.fields {
                            VariantFields::Unit => Place::UnitVariant,
                            VariantFields::Tuple(_) | VariantFields::Named(_) => Place::DataVariant,
                        };
                        let rename = attrs::placed(&variant.attrs, place, diagnostics)
                            .into_iter()
                            .find_map(|attr| attrs::rename(attr, diagnostics));
                        if let VariantFields::Named(fields) = &variant.fields {
                            for field in fields {
                                attrs::placed(&field.attrs, Place::VariantField, diagnostics);
                            }
                        }
                        // Each field name, where one is written.
                        let written: Vec<(Option<&Ident>, &TypeExpr)> = match &variant.fields {
                            VariantFields::Unit => Vec::new(),
                            VariantFields::Tuple(types) => {
                                types.iter().map(|ty| (None, ty)).collect()
                            }
                            VariantFields::Named(fields) => fields
                                .iter()
                                .map(|field| (Some(&field.name), &field.ty))
                                .collect(),
                        };
                        let mut field_seen: HashMap<&str, Span> = HashMap::new();
                        let mut resolved = Vec::new();
                        for (field, written) in written {
                            if let Some(field) = field {
                                diagnostics.extend(reserved_value_name(&field.name, field.span));
                                if let Some(&first) = field_seen.get(field.name.as_str()) {
                                    diagnostics.push(duplicate(&field.name, field.span, first));
                                }
                                field_seen.entry(&field.name).or_insert(field.span);
                            }
                            match symbols.resolve_type(written, module.id) {
                                Ok(ty) => {
                                    if decl.is_pub {
                                        diagnostics.extend(symbols.private_in_public(
                                            &ty,
                                            written.span,
                                            &decl.name.name,
                                            symbols.reach(module.id, decl.is_pub),
                                        ));
                                    }
                                    parts.push((ty.clone(), written.span));
                                    resolved.push((field, ty));
                                }
                                Err(diagnostic) => diagnostics.push(diagnostic),
                            }
                        }
                        let fields = match &variant.fields {
                            VariantFields::Named(_) => VariantFieldsDef::Named(
                                resolved
                                    .into_iter()
                                    .filter_map(|(field, ty)| Some((field?.name.clone(), ty)))
                                    .collect(),
                            ),
                            VariantFields::Unit | VariantFields::Tuple(_) => {
                                VariantFieldsDef::Tuple(
                                    resolved.into_iter().map(|(_, ty)| ty).collect(),
                                )
                            }
                        };
                        variants.push(VariantDef {
                            name: name.name.clone(),
                            fields,
                            rename,
                        });
                    }
                    symbols.enums[next_enum].variants = variants;
                    enum_parts.push(parts);
                    next_enum += 1;
                }
                Item::Mod(_) => {}
                Item::Use(_) => {}
            }
        }
    }
    // Imported structs and enums (each numbered after every Varyk one of
    // its kind) contain no Varyk type, so they close no cycle.
    struct_parts.resize(symbols.structs.len(), Vec::new());
    enum_parts.resize(symbols.enums.len(), Vec::new());
    struct_parts.extend(enum_parts);
    check_recursive_types(&symbols, &struct_parts, diagnostics);
    symbols
}

/// The `mod` declaration of `module` in its parent's file; `None` for
/// the root.
fn mod_decl(modules: &[Module], module: &Module) -> Option<Span> {
    let ModuleKind::Varyk(program) = &modules[module.parent?.0 as usize].kind else {
        return None;
    };
    program.items.iter().find_map(|item| match item {
        Item::Mod(decl) if decl.name.name == module.name => Some(decl.span),
        _ => None,
    })
}

/// Appends the signature shell of `function`, declared by item `item` of
/// `module` (at `member` inside an `impl` block); pass 2 fills in its
/// types and pass 1b its owner.
fn push_fn(
    fns: &mut Vec<FnSig>,
    function: &Function,
    module: ModuleId,
    item: usize,
    member: Option<usize>,
) -> FnId {
    let id = FnId(fns.len() as u32);
    fns.push(FnSig {
        name: function.name.name.clone(),
        module,
        is_pub: function.is_pub,
        params: Vec::new(),
        ret: Ty::Unit,
        owner: None,
        self_mode: match function.self_mode {
            SelfMode::None => None,
            SelfMode::Shared => Some(ParamMode::SharedBorrow),
            SelfMode::Mutable => Some(ParamMode::MutableBorrow),
        },
        item,
        member,
        is_test: false,
        span: function.span,
    });
    id
}

/// The checked attributes of a struct field of type `ty`, written as
/// `written`, from those [`attrs::placed`] kept.
fn field_attrs(
    placed: &[&varyk_syntax::Attribute],
    ty: &Ty,
    written: &TypeExpr,
    diagnostics: &mut Vec<Diagnostic>,
) -> FieldAttrs {
    let mut attrs = FieldAttrs::default();
    for attr in placed {
        match attr.name.name.as_str() {
            "rename" => attrs.rename = attrs::rename(attr, diagnostics),
            "default" => attrs.default = attrs::default(attr, ty, written, diagnostics),
            "skip" => attrs.skip = true,
            _ => {}
        }
    }
    attrs
}

/// V0114 for a `#[test]` function with parameters or a return type (M5a
/// spec 2.7): the test runner calls it with nothing and keeps nothing.
fn check_test_shape(function: &Function, diagnostics: &mut Vec<Diagnostic>) {
    if function.params.is_empty() && function.return_type.is_none() {
        return;
    }
    diagnostics.push(
        Diagnostic::new(
            codes::V0114,
            header(function),
            "a test must take no parameters and return nothing",
        )
        .with_note(
            "`varyk test` runs each test on its own, giving it nothing, and a test checks its \
             results with `assert` and `assert_eq` instead of returning them",
        ),
    );
}

/// V0109 for every struct or enum that contains itself, directly or
/// through other structs' fields and enums' payloads, `Option` and
/// `Result` arguments included, since Rust lays all of them out inline
/// (rustc E0072); a `Vec` or a `HashMap` holds its elements elsewhere and
/// breaks the cycle (spec 2.2, M4 spec 2.7). `parts[node]` lists a node's
/// field or payload types with their spans, structs first (node `id`) and
/// then enums (node `structs.len() + id`). A depth-first walk reports each cycle once, at
/// the part that closes it.
fn check_recursive_types(
    symbols: &Symbols,
    parts: &[Vec<(Ty, Span)>],
    diagnostics: &mut Vec<Diagnostic>,
) {
    #[derive(Clone, Copy, PartialEq)]
    enum State {
        Unvisited,
        OnStack,
        Done,
    }

    struct Walk<'a> {
        symbols: &'a Symbols,
        parts: &'a [Vec<(Ty, Span)>],
        state: Vec<State>,
        stack: Vec<usize>,
        diagnostics: &'a mut Vec<Diagnostic>,
    }

    impl Walk<'_> {
        fn name(&self, node: usize) -> &str {
            let structs = self.symbols.structs.len();
            if node < structs {
                &self.symbols.structs[node].name
            } else {
                &self.symbols.enums[node - structs].name
            }
        }

        /// The nodes `ty` lays out inline.
        fn inline(&self, ty: &Ty, out: &mut Vec<usize>) {
            match ty {
                Ty::Struct(id) => out.push(id.0 as usize),
                Ty::Enum(id) => out.push(self.symbols.structs.len() + id.0 as usize),
                Ty::Option(inner) => self.inline(inner, out),
                Ty::Result(ok, err) => {
                    self.inline(ok, out);
                    self.inline(err, out);
                }
                _ => {}
            }
        }

        fn visit(&mut self, node: usize) {
            self.state[node] = State::OnStack;
            self.stack.push(node);
            let parts = self.parts;
            for (ty, span) in &parts[node] {
                let mut targets = Vec::new();
                self.inline(ty, &mut targets);
                targets.dedup();
                for target in targets {
                    match self.state[target] {
                        State::Unvisited => self.visit(target),
                        State::OnStack => self.report(target, *span),
                        State::Done => {}
                    }
                }
            }
            self.stack.pop();
            self.state[node] = State::Done;
        }

        fn report(&mut self, target: usize, span: Span) {
            let start = self
                .stack
                .iter()
                .position(|&s| s == target)
                .expect("an on-stack type is on the stack");
            let cycle: Vec<&str> = self.stack[start..]
                .iter()
                .chain(std::iter::once(&target))
                .map(|&node| self.name(node))
                .collect();
            let message = if target < self.symbols.structs.len() {
                "a struct cannot contain itself"
            } else {
                "an enum cannot contain itself"
            };
            let diagnostic = Diagnostic::new(codes::V0109, span, message)
                .with_note(format!("the cycle is {}", cycle.join(" -> ")))
                .with_note(
                    "a `Vec` or `HashMap` holds its elements elsewhere, so holding the value \
                     in one breaks the cycle",
                );
            self.diagnostics.push(diagnostic);
        }
    }

    let mut walk = Walk {
        symbols,
        parts,
        state: vec![State::Unvisited; parts.len()],
        stack: Vec::new(),
        diagnostics,
    };
    for node in 0..parts.len() {
        if walk.state[node] == State::Unvisited {
            walk.visit(node);
        }
    }
}

/// Resolves a Varyk function's parameter and return types. Parameter
/// modes follow spec 4.2: `mut` is a mutable borrow; otherwise a Copy type
/// is owned and anything else a shared borrow. A `pub` function's types
/// must be nameable everywhere in its `reach` (V0105).
fn resolve_signature(
    symbols: &Symbols,
    function: &Function,
    module: ModuleId,
    reach: ModuleId,
    diagnostics: &mut Vec<Diagnostic>,
) -> (Vec<(String, Ty, ParamMode)>, Ty) {
    let mut seen: HashMap<&str, Span> = HashMap::new();
    let mut params = Vec::new();
    for param in &function.params {
        let name = &param.name;
        if let Some(&first) = seen.get(name.name.as_str()) {
            diagnostics.push(duplicate(&name.name, name.span, first));
        }
        seen.entry(&name.name).or_insert(name.span);
        match symbols.resolve_type(&param.ty, module) {
            Ok(ty) => {
                if function.is_pub {
                    if let Some(diagnostic) =
                        symbols.private_in_public(&ty, param.ty.span, &function.name.name, reach)
                    {
                        diagnostics.push(diagnostic);
                    }
                }
                let mode = if param.mutable {
                    ParamMode::MutableBorrow
                } else if ty.is_copy() {
                    ParamMode::Owned
                } else {
                    ParamMode::SharedBorrow
                };
                params.push((name.name.clone(), ty, mode));
            }
            Err(diagnostic) => diagnostics.push(diagnostic),
        }
    }
    let ret = match &function.return_type {
        None => Ty::Unit,
        Some(written) => match symbols.resolve_type(written, module) {
            Ok(ty) => {
                if function.is_pub {
                    if let Some(diagnostic) =
                        symbols.private_in_public(&ty, written.span, &function.name.name, reach)
                    {
                        diagnostics.push(diagnostic);
                    }
                }
                ty
            }
            Err(diagnostic) => {
                diagnostics.push(diagnostic);
                Ty::Unit
            }
        },
    };
    (params, ret)
}

/// A binary's entry module must define `fn main()` with no parameters
/// and no return type (spec 4.1); a library's must not define `main` (M3
/// spec 2.1).
fn check_entry_main(entry: &Module, kind: Kind, diagnostics: &mut Vec<Diagnostic>) {
    let ModuleKind::Varyk(program) = &entry.kind else {
        return;
    };
    let main = program.items.iter().find_map(|item| match item {
        Item::Function(function) if function.name.name == "main" => Some(function),
        _ => None,
    });
    if kind == Kind::Library {
        if let Some(function) = main {
            diagnostics.push(
                Diagnostic::new(
                    codes::V0106,
                    header(function),
                    "a library must not define `main`",
                )
                .with_note("`src/lib.vr` is a library; a program starts from `src/main.vr`"),
            );
        }
        return;
    }
    match main {
        None => diagnostics.push(Diagnostic::new(
            codes::V0106,
            Span::new(entry.file, 0, 0),
            "the entry file must define `fn main()`",
        )),
        Some(function) if !function.params.is_empty() || function.return_type.is_some() => {
            diagnostics.push(Diagnostic::new(
                codes::V0106,
                header(function),
                "`main` must take no parameters and return nothing",
            ));
        }
        Some(function) if function.attrs.iter().any(|attr| attr.name.name == "test") => {
            diagnostics.push(
                Diagnostic::new(
                    codes::V0114,
                    header(function),
                    "`main` is where the program starts and cannot be a test",
                )
                .with_note("put the test in a function of its own, with another name"),
            );
        }
        Some(_) => {}
    }
}

/// From the start of the declaration through the function's name
/// (`fn main`, or `pub fn main`).
fn header(function: &Function) -> Span {
    Span::new(
        function.span.file,
        function.span.start,
        function.name.span.end,
    )
}
