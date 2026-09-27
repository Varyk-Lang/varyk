//! `varyk`: the Varyk compiler library.
//!
//! Lowers `.vr` sources through the resolver, type checker, and borrow
//! analysis into HIR, then hands the HIR to a backend that generates and
//! builds a Rust crate.

pub mod backend;
pub mod borrow;
pub mod builtins;
pub mod cli;
pub mod diagnostics;
pub mod driver;
pub mod hir;
pub mod interop;
pub mod package;
pub mod resolve;
pub mod types;

use varyk_syntax::SourceFile;

use borrow::analyze;
use diagnostics::Diagnostic;
use hir::HirProgram;
use package::Kind;
pub use resolve::Dependencies;
use resolve::resolve_root;
use types::typecheck;

/// Resolves the program rooted at `entry`, a crate root of `kind` whose
/// package has `dependencies`, or a single file when that is `None`
/// (see [`resolve::resolve_root`]),
/// type-checks it into HIR (see [`types::typecheck`]), and runs borrow
/// analysis over it (see [`borrow::analyze`]),
/// pushing every file it loads onto `sources`, indexed by `FileId.0`; the
/// caller builds `entry` with `FileId(sources.len())`.
///
/// Reading the entry file is the caller's job: an unreadable path has no
/// `SourceFile` to build, so that error can't flow through here — `cli.rs`
/// handles it as a plain message, before ever calling this function.
pub fn check_file(
    entry: SourceFile,
    kind: Kind,
    dependencies: Option<Dependencies<'_>>,
    sources: &mut Vec<SourceFile>,
) -> Result<HirProgram, Vec<Diagnostic>> {
    resolve_root(entry, kind, dependencies, sources)
        .and_then(|resolved| typecheck(resolved, sources))
        .and_then(|hir| analyze(hir, sources))
        .map_err(without_repeated_fix_its)
}

/// `diagnostics` with each fix-it kept only on the first diagnostic that
/// carries it: two uses of one item without `pub` are two diagnostics,
/// but one edit, and applying every fix-it must not write `pub pub`.
fn without_repeated_fix_its(mut diagnostics: Vec<Diagnostic>) -> Vec<Diagnostic> {
    let mut seen = Vec::new();
    for diagnostic in &mut diagnostics {
        if let Some(fix_it) = diagnostic.fix_it.take() {
            if !seen.contains(&fix_it) {
                seen.push(fix_it.clone());
                diagnostic.fix_it = Some(fix_it);
            }
        }
    }
    diagnostics
}
