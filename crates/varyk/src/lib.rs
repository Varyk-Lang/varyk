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
pub mod packages;
pub mod resolve;
#[cfg(test)]
mod test_packages;
pub mod types;

use std::path::{Path, PathBuf};

use varyk_syntax::{FileId, SourceFile, Span};

use borrow::analyze;
use diagnostics::{Diagnostic, codes};
use hir::HirProgram;
use package::{Kind, Package};
use packages::{Graph, Listing, VarykPackage};
pub use resolve::Dependencies;
use resolve::{Packages, resolve_root};
use types::typecheck;

/// A Varyk package of the build, checked (M5b2 spec 4.2), for the driver
/// to generate. It has no single name in Rust: each package that uses it
/// names it by its own key (spec 5).
#[derive(Debug, Clone, PartialEq)]
pub struct CheckedPackage {
    /// The directory holding its `Cargo.toml`, as cargo reports it.
    pub dir: PathBuf,
    /// Its index in [`Graph::varyk_packages`].
    pub graph_index: usize,
    /// Its `[package]` name.
    pub name: String,
    pub version: String,
    /// Its isolated manifest (`Package::isolated_manifest`), which the
    /// crate the driver writes for it is made from (spec 4.3).
    pub manifest: toml::Table,
    /// Its `Cargo.toml` among the sources, where a rustc error in its
    /// crate is shown (spec 4.3).
    pub manifest_file: FileId,
    pub program: HirProgram,
}

/// A checked program and the Varyk packages of its build.
#[derive(Debug, Clone, PartialEq)]
pub struct Checked {
    pub program: HirProgram,
    /// Every Varyk package of the build, each after those it depends on.
    pub packages: Vec<CheckedPackage>,
    /// The first package whose log lines made the program need `varyk-std`
    /// when the program itself did not.
    pub logging_package: Option<String>,
}

/// Checks the program rooted at `entry`, the root of `package`, or a
/// single file when that is `None`: first every Varyk package of
/// `graph`'s build, dependencies first, each once (M5b2 spec 4.2), then
/// the program, which can name the packages its `[dependencies]` reach.
/// Each is resolved (see [`resolve::resolve_root`]), type-checked into
/// HIR (see [`types::typecheck`]), and run through borrow analysis (see
/// [`borrow::analyze`]). Every file it loads is pushed onto `sources`,
/// indexed by `FileId.0`; `entry` takes the next id after the packages'.
///
/// A package that does not pass stops the check: its diagnostics are
/// returned at its own files, each with a note naming the package.
///
/// Reading the entry file is the caller's job: an unreadable path has no
/// `SourceFile` to build, so that error can't flow through here — `cli.rs`
/// handles it as a plain message, before ever calling this function.
pub fn check_file(
    mut entry: SourceFile,
    package: Option<&Package>,
    graph: Option<&Graph>,
    sources: &mut Vec<SourceFile>,
) -> Result<Checked, Vec<Diagnostic>> {
    let first = sources.len() as u32;
    let mut checked: Vec<CheckedPackage> = Vec::new();
    for (index, dependency) in graph
        .map_or(&[][..], Graph::varyk_packages)
        .iter()
        .enumerate()
    {
        let listings = graph.map_or_else(Vec::new, |graph| graph.listings(&dependency.dir));
        let manifest_file = FileId(sources.len() as u32);
        let (program, manifest) = check_dependency(dependency, &checked, &listings, sources)
            .map_err(|diagnostics| {
                let note = format!(
                    "in the package `{}` {}, which this build uses",
                    dependency.name, dependency.version
                );
                diagnostics
                    .into_iter()
                    .map(|mut diagnostic| {
                        // A fix-it would edit the dependency, which is not
                        // this package's to change.
                        diagnostic.fix_it = None;
                        diagnostic.with_note(note.clone())
                    })
                    .collect::<Vec<_>>()
            })?;
        checked.push(CheckedPackage {
            dir: dependency.dir.clone(),
            graph_index: index,
            name: dependency.name.clone(),
            version: dependency.version.clone(),
            manifest,
            manifest_file,
            program,
        });
    }
    let dependency_files = first..sources.len() as u32;
    entry.id = FileId(sources.len() as u32);
    let listings = graph
        .zip(package)
        .map_or_else(Vec::new, |(graph, package)| graph.listings(&package.root));
    let packages = Packages {
        keys: graph.map_or(&[][..], Graph::root_deps),
        checked: &checked,
        dependency: false,
        listings: &listings,
    };
    let mut program =
        check_package(entry, package, packages, sources).map_err(|mut diagnostics| {
            for diagnostic in &mut diagnostics {
                let in_dependency = diagnostic
                    .fix_it
                    .as_ref()
                    .is_some_and(|fix_it| dependency_files.contains(&fix_it.span.file.0));
                if in_dependency {
                    diagnostic.fix_it = None;
                }
            }
            diagnostics
        })?;
    // A program's `main` sets up logging for every package of the build
    // (M5b2 spec 4.2), so a package that logs makes the program log.
    let binary = package.is_none_or(|package| package.kind == Kind::Binary);
    let logger = first_logger(&checked).filter(|_| binary);
    let logging_package = logger
        .filter(|_| !program.uses_std)
        .map(|package| package.name.clone());
    if logger.is_some() {
        program.logs = true;
        program.uses_std = true;
    }
    Ok(Checked {
        program,
        packages: checked,
        logging_package,
    })
}

/// The first of `checked` that writes log lines.
fn first_logger(checked: &[CheckedPackage]) -> Option<&CheckedPackage> {
    checked.iter().find(|package| package.program.logs)
}

/// Loads `dependency`, a Varyk package of the build, through its own
/// manifest and checks it with the packages `checked` before it, then its
/// `varyk-std` line against its own use of it (its lock is not read, M5b2
/// spec 4.2). Its files are shown relative to the working directory when
/// they are under it. Returns its program and its isolated manifest.
fn check_dependency(
    dependency: &VarykPackage,
    checked: &[CheckedPackage],
    listings: &[Listing],
    sources: &mut Vec<SourceFile>,
) -> Result<(HirProgram, toml::Table), Vec<Diagnostic>> {
    let manifest_file = FileId(sources.len() as u32);
    let manifest = shown(&dependency.dir).join("Cargo.toml");
    let package = package::load_dependency(&manifest, sources, dependency.source.is_none())?;
    let text = std::fs::read_to_string(&package.entry).map_err(|err| {
        vec![Diagnostic::new(
            codes::V0104,
            Span::new(manifest_file, 0, 0),
            format!("cannot read `{}`: {err}", package.entry.display()),
        )]
    })?;
    let entry = SourceFile::new(FileId(sources.len() as u32), package.entry.clone(), text);
    let packages = Packages {
        keys: &dependency.deps,
        checked,
        dependency: true,
        listings,
    };
    let program = match check_package(entry, Some(&package), packages, sources) {
        Ok(program) => program,
        Err(mut errors) => {
            // A package that does not pass is most often one written for
            // another Varyk version (spec 4.2), so its `varyk-std` line is
            // checked too, whatever the package uses.
            if package.dependencies.contains_key("varyk-std") {
                errors.extend(package::check_std_dependency(
                    &package,
                    true,
                    env!("CARGO_PKG_VERSION"),
                ));
            }
            return Err(errors);
        }
    };
    let problems =
        package::check_std_dependency(&package, program.uses_std, env!("CARGO_PKG_VERSION"));
    if problems.is_empty() {
        Ok((program, package.isolated_manifest()))
    } else {
        Err(problems)
    }
}

/// Checks one package, or a single file when `package` is `None`, with
/// the Varyk packages of `packages`.
fn check_package(
    entry: SourceFile,
    package: Option<&Package>,
    packages: Packages<'_>,
    sources: &mut Vec<SourceFile>,
) -> Result<HirProgram, Vec<Diagnostic>> {
    let crates: Vec<String> = package
        .map(|package| package.dependencies.keys().cloned().collect())
        .unwrap_or_default();
    let optional: Vec<String> = package
        .map(|package| {
            package
                .dependencies
                .iter()
                .filter(|(_, value)| {
                    value.get("optional").and_then(toml::Value::as_bool) == Some(true)
                })
                .map(|(key, _)| key.clone())
                .collect()
        })
        .unwrap_or_default();
    let dependencies = package.map(|package| Dependencies {
        crates: &crates,
        dev: &package.dev_dependencies,
        optional: &optional,
    });
    let kind = package.map_or(Kind::Binary, |package| package.kind);
    resolve_root(entry, kind, dependencies, packages, sources)
        .and_then(|resolved| typecheck(resolved, sources))
        .and_then(|hir| analyze(hir, sources))
        .map_err(without_repeated_fix_its)
}

/// `dir` relative to the working directory when it is under it, so a
/// dependency's files, which cargo reports by absolute path, are shown as
/// the program's are; else `dir` itself.
fn shown(dir: &Path) -> PathBuf {
    std::env::current_dir()
        .ok()
        .and_then(|cwd| {
            packages::canonical(dir)
                .strip_prefix(packages::canonical(&cwd))
                .ok()
                .map(Path::to_path_buf)
        })
        .unwrap_or_else(|| dir.to_path_buf())
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

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::package::tests::TempDir;
    use crate::packages::DepTarget;

    /// Copies the example package `name` into `dir`, leaving out
    /// `target/`.
    fn copy_example(name: &str, dir: &Path) {
        fn copy(from: &Path, to: &Path) {
            fs::create_dir_all(to).expect("create the copy");
            for entry in fs::read_dir(from).expect("read the example") {
                let path = entry.expect("read the example").path();
                let Some(file_name) = path.file_name() else {
                    continue;
                };
                if path.is_dir() && file_name != "target" {
                    copy(&path, &to.join(file_name));
                } else if path.is_file() {
                    fs::copy(&path, to.join(file_name)).expect("copy a file");
                }
            }
        }
        let examples = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/packages");
        copy(&examples.join(name), &dir.join(name));
    }

    /// `trip`, rewritten to call no `log` and nothing async, checked with
    /// `route` and `units` beside it, from copies.
    fn trip_without_its_own_logging() -> Checked {
        let dir = TempDir::new("trip_logs");
        for name in ["units", "route", "trip"] {
            copy_example(name, &dir.0);
        }
        let trip = dir.write(
            "trip/src/main.vr",
            "fn main() {\n    let legs = vec![route::leg(\"Home\", \"Mill\", 800)];\n    let sum = units::length::add(route::total(legs), units::length::Meters { value: 0 });\n    println!(\"total {} meters\", sum.value);\n}\n",
        );
        let package = |name: &str, deps| VarykPackage {
            dir: dir.0.join(name),
            name: name.to_string(),
            version: "0.1.0".to_string(),
            source: None,
            deps,
        };
        let graph = Graph::for_tests(
            vec![
                package("units", Vec::new()),
                package("route", vec![("units".to_string(), DepTarget::Varyk(0))]),
            ],
            vec![
                ("route".to_string(), DepTarget::Varyk(1)),
                ("units".to_string(), DepTarget::Varyk(0)),
                ("varyk_std".to_string(), DepTarget::Rust),
            ],
        );
        let mut sources = Vec::new();
        let manifest = dir.0.join("trip/Cargo.toml");
        let program = package::load(&manifest, &mut sources)
            .unwrap_or_else(|diagnostics| panic!("{diagnostics:#?}"));
        let text = fs::read_to_string(&trip).expect("read trip");
        let entry = SourceFile::new(FileId(sources.len() as u32), trip, text);
        check_file(entry, Some(&program), Some(&graph), &mut sources)
            .unwrap_or_else(|diagnostics| panic!("{diagnostics:#?}"))
    }

    #[test]
    fn a_program_logs_and_uses_std_when_a_package_of_its_build_logs() {
        let checked = trip_without_its_own_logging();
        let route = &checked.packages[1];
        assert_eq!(route.name, "route");
        assert!(route.program.logs, "`route` logs a stop it cannot read");
        assert!(checked.program.logs);
        assert!(checked.program.uses_std);
        // `units` neither logs nor uses `varyk-std`.
        assert!(!checked.packages[0].program.logs);
        assert!(!checked.packages[0].program.uses_std);
    }
}
