//! Module paths (spec 3.1): resolving the module prefix of a written path
//! (`crate::`, `self::`, `super::`, or a leading module name declared in
//! the current module, then child modules by name) and rendering paths
//! back to text for diagnostics.

use varyk_syntax::{Ident, Path, PathStart, Span};

use crate::diagnostics::{Diagnostic, codes};

use super::{LookupError, ModuleId, Symbols, UseTarget};

impl Symbols {
    /// `id` itself first, then its parent, and so on up to the crate root,
    /// which comes last.
    pub fn ancestors(&self, id: ModuleId) -> impl Iterator<Item = ModuleId> + '_ {
        std::iter::successors(Some(id), |module| self.scopes[module.0 as usize].parent)
    }

    /// The module declared in `parent` as `mod name;`, if any.
    pub(crate) fn child(&self, parent: ModuleId, name: &str) -> Option<ModuleId> {
        self.scopes[parent.0 as usize].children.get(name).copied()
    }

    /// Whether some module in the crate, anywhere, declares a child module
    /// named `name` (spec 3.3): a `use` leading segment matching one names
    /// a real module the uniform-paths rule cannot reach without
    /// `crate::` (V0111); one matching none is instead taken for a crate
    /// name (V0110).
    pub(crate) fn module_named(&self, name: &str) -> bool {
        self.scopes
            .iter()
            .any(|scope| scope.package.is_none() && scope.children.contains_key(name))
    }

    /// The path from the crate root, without `crate::`, of every module
    /// called `name`: `shop`, `a::shop`.
    pub(crate) fn modules_named(&self, name: &str) -> Vec<&str> {
        self.scopes
            .iter()
            .filter(|scope| scope.package.is_none())
            .filter_map(|scope| scope.children.get(name))
            .map(|id| self.scopes[id.0 as usize].name.as_str())
            .collect()
    }

    /// The module that `path` names, seen from `from`: `crate` is the
    /// root, `self` is `from`, `super` is `from`'s parent, and a path with
    /// no keyword starts at a module declared in `from`; every segment is
    /// then a child module by name. `Unknown` when a segment names no
    /// such module; `NoParent` for `super` in the crate root.
    ///
    /// A path with no keyword also lets its first segment be a `use`
    /// alias of `from`'s own file (spec 3.3), consulted only there: an
    /// alias is never exported, so it never governs a later segment once
    /// the walk has moved into another module. Failing both, it may be a
    /// dependency of the package (M5b2 spec 2.1): a Varyk package's root,
    /// or `Dependency` for one Varyk code cannot name.
    pub(crate) fn module_at(&self, from: ModuleId, path: &Path) -> Result<ModuleId, LookupError> {
        let mut current = match path.leading {
            PathStart::Crate => ModuleId(0),
            PathStart::SelfMod | PathStart::None => from,
            PathStart::Super => {
                self.scopes[from.0 as usize]
                    .parent
                    .ok_or(LookupError::NoParent {
                        span: Span::new(path.span.file, path.span.start, keyword_end(path)),
                    })?
            }
        };
        let mut segments = path.segments.iter();
        if path.leading == PathStart::None {
            if let Some(first) = segments.next() {
                current = match self.child(current, &first.name) {
                    Some(module) => module,
                    None => match self.scopes[from.0 as usize].use_types.get(&first.name) {
                        Some(UseTarget::Module(module)) => *module,
                        _ => self.dependency_root(first)?,
                    },
                };
            }
        }
        for segment in segments {
            current = self
                .child(current, &segment.name)
                .ok_or(LookupError::Unknown)?;
        }
        Ok(current)
    }
}

/// The module `path` names as seen from `from` (spec 3.1): V0111 for
/// `super` in the crate root, V0100 for a segment that names no module,
/// citing the whole path as written.
#[expect(
    clippy::result_large_err,
    reason = "a single diagnostic on a cold path, as `Symbols::resolve_type`"
)]
pub fn resolve_path(
    symbols: &Symbols,
    from: ModuleId,
    path: &Path,
) -> Result<ModuleId, Diagnostic> {
    symbols.module_at(from, path).map_err(|error| match error {
        LookupError::NoParent { span } => no_parent(span),
        LookupError::Dependency { span, dep } => symbols.dependency_error(span, dep),
        _ => Diagnostic::new(
            codes::V0100,
            path.span,
            format!("cannot find module `{}`", path_text(path)),
        ),
    })
}

/// V0111 at the `super` keyword of a path written in the crate root.
pub(crate) fn no_parent(span: Span) -> Diagnostic {
    Diagnostic::new(
        codes::V0111,
        span,
        "`super` goes to the module that declares this one, but this is the entry file, which \
         nothing declares",
    )
    .with_note("to name an item of the entry file, write `crate::` or leave the prefix out")
    .with_note("in Rust terms, the crate root has no parent module")
}

/// `path` as written: its keyword prefix (if any) and its segments,
/// joined by `::` (`crate::shop::cart`).
pub(crate) fn path_text(path: &Path) -> String {
    let mut parts: Vec<&str> = Vec::new();
    match path.leading {
        PathStart::Crate => parts.push("crate"),
        PathStart::SelfMod => parts.push("self"),
        PathStart::Super => parts.push("super"),
        PathStart::None => {}
    }
    parts.extend(path.segments.iter().map(|s| s.name.as_str()));
    parts.join("::")
}

/// `path` followed by `name`: the whole path as written, for a diagnostic
/// about it. Mirrors `TypeExpr::display_name`.
pub(crate) fn display_path(path: &Path, name: &str) -> String {
    format!("{}::{name}", path_text(path))
}

/// Splits `path` into its last segment and the module path before it:
/// `None` for that module path when it is empty (`Shape` alone names
/// something in the current module), and `None` overall for a path with
/// no segments (`crate::` alone). For `shop::cart::Cart::new`, whose
/// `path` is `shop::cart::Cart`, gives `shop::cart` and `Cart`.
pub(crate) fn split_last(path: &Path) -> Option<(Option<Path>, &Ident)> {
    let (last, prefix) = path.segments.split_last()?;
    if path.leading == PathStart::None && prefix.is_empty() {
        return Some((None, last));
    }
    let end = prefix.last().map_or(keyword_end(path), |s| s.span.end);
    let prefix = Path {
        leading: path.leading,
        segments: prefix.to_vec(),
        span: Span::new(path.span.file, path.span.start, end),
    };
    Some((Some(prefix), last))
}

/// Where `path`'s keyword prefix ends.
fn keyword_end(path: &Path) -> u32 {
    let len = match path.leading {
        PathStart::Crate => "crate".len(),
        PathStart::SelfMod => "self".len(),
        PathStart::Super => "super".len(),
        PathStart::None => 0,
    };
    path.span.start + len as u32
}
