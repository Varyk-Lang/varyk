//! `varyk`: the Varyk compiler library.
//!
//! Lowers `.vr` sources through the resolver, type checker, and borrow
//! analysis into HIR, then hands the HIR to a backend that generates and
//! builds a Rust crate.

pub mod backend;
pub mod borrow;
pub mod cli;
pub mod diagnostics;
pub mod driver;
pub mod hir;
pub mod interop;
pub mod resolve;
pub mod types;

use varyk_syntax::SourceFile;

use borrow::analyze;
use diagnostics::Diagnostic;
use hir::HirProgram;
use resolve::resolve;
use types::typecheck;

/// Resolves the program rooted at `entry` (see [`resolve::resolve`]),
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
    sources: &mut Vec<SourceFile>,
) -> Result<HirProgram, Vec<Diagnostic>> {
    resolve(entry, sources)
        .and_then(typecheck)
        .and_then(analyze)
}
