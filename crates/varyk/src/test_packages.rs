//! For tests: the Varyk packages of a build, checked from the fixtures
//! under `tests/fixtures/resolve/packages/` without cargo (M5b2 spec
//! 4.2), and programs checked with them.

use std::path::PathBuf;

use varyk_syntax::{FileId, SourceFile};

use crate::CheckedPackage;
use crate::borrow::analyze;
use crate::diagnostics::Diagnostic;
use crate::hir::HirProgram;
use crate::package::Kind;
use crate::packages::DepTarget;
use crate::resolve::{Dependencies, Packages, Resolved, resolve_root};
use crate::types::typecheck;

/// What a dependency key of a test package names.
#[derive(Clone, Copy)]
pub(crate) enum Dep<'a> {
    /// The package fixture of this name, added to the build already.
    Package(&'a str),
    Rust,
    /// Marked `optional`, naming the package fixture of this name.
    Optional(&'a str),
}

/// The packages checked so far and every file they read.
#[derive(Default)]
pub(crate) struct Build {
    pub sources: Vec<SourceFile>,
    pub checked: Vec<CheckedPackage>,
}

/// A package's `[dependencies]` as the resolver takes them: the keys,
/// the optional ones, and what each key resolved to.
struct Keys {
    crates: Vec<String>,
    optional: Vec<String>,
    keys: Vec<(String, DepTarget)>,
}

fn fixture(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/resolve/packages")
        .join(rel)
}

impl Build {
    fn keys(&self, deps: &[(&str, Dep<'_>)]) -> Keys {
        let index = |name: &str| {
            self.checked
                .iter()
                .position(|checked| checked.name == name)
                .unwrap_or_else(|| panic!("add `{name}` to the build first"))
        };
        let mut keys = Keys {
            crates: Vec::new(),
            optional: Vec::new(),
            keys: Vec::new(),
        };
        for (key, dep) in deps {
            keys.crates.push(key.to_string());
            let target = match dep {
                Dep::Package(name) => DepTarget::Varyk(index(name)),
                Dep::Optional(name) => {
                    keys.optional.push(key.to_string());
                    DepTarget::Varyk(index(name))
                }
                Dep::Rust => DepTarget::Rust,
            };
            keys.keys.push((key.replace('-', "_"), target));
        }
        keys
    }

    /// Resolves `entry`, a root of `kind` whose package, named `name`
    /// when it is a library, has `deps`, with the packages checked so far.
    fn resolve_entry(
        &mut self,
        entry: SourceFile,
        kind: Kind,
        deps: &[(&str, Dep<'_>)],
        name: Option<&str>,
    ) -> Result<Resolved, Vec<Diagnostic>> {
        let keys = self.keys(deps);
        let dependencies = Dependencies {
            crates: &keys.crates,
            dev: &[],
            optional: &keys.optional,
        };
        let packages = Packages {
            keys: &keys.keys,
            checked: &self.checked,
            dependency: name.is_some(),
            listings: &[],
            name,
        };
        resolve_root(entry, kind, Some(dependencies), packages, &mut self.sources)
    }

    fn entry(&self, path: PathBuf) -> SourceFile {
        let text = std::fs::read_to_string(&path).expect("read the fixture");
        SourceFile::new(FileId(self.sources.len() as u32), path, text)
    }

    /// Checks the package fixture `name`, a library whose package has
    /// `deps`, as a dependency of the build, and adds it.
    pub(crate) fn add(&mut self, name: &str, deps: &[(&str, Dep<'_>)]) {
        self.add_dir(name, fixture(name), deps);
    }

    /// Checks the library package `name` in `dir`, whose package has
    /// `deps`, as a dependency of the build, and adds it.
    pub(crate) fn add_dir(&mut self, name: &str, dir: PathBuf, deps: &[(&str, Dep<'_>)]) {
        self.try_add_dir(name, dir, deps)
            .unwrap_or_else(|diagnostics| panic!("`{name}` should check: {diagnostics:#?}"));
    }

    /// [`Build::add_dir`], giving the package's diagnostics when it does
    /// not check, and nothing added.
    pub(crate) fn try_add_dir(
        &mut self,
        name: &str,
        dir: PathBuf,
        deps: &[(&str, Dep<'_>)],
    ) -> Result<(), Vec<Diagnostic>> {
        let entry = self.entry(dir.join("src/lib.vr"));
        let program = self
            .resolve_entry(entry, Kind::Library, deps, Some(name))
            .and_then(|resolved| typecheck(resolved, &self.sources))
            .and_then(|hir| analyze(hir, &self.sources))?;
        self.checked.push(CheckedPackage {
            dir,
            graph_index: self.checked.len(),
            name: name.to_string(),
            version: "0.1.0".to_string(),
            manifest: toml::Table::new(),
            manifest_file: FileId(0),
            program,
        });
        Ok(())
    }

    /// Resolves `text` as the `src/main.vr` of a program with `deps`.
    pub(crate) fn resolve(
        &mut self,
        text: &str,
        deps: &[(&str, Dep<'_>)],
    ) -> Result<Resolved, Vec<Diagnostic>> {
        let entry = SourceFile::new(FileId(self.sources.len() as u32), "dummy/src/main.vr", text);
        self.resolve_entry(entry, Kind::Binary, deps, None)
    }

    /// Resolves the program fixture `name` (its `src/main.vr`) with `deps`.
    pub(crate) fn resolve_fixture(
        &mut self,
        name: &str,
        deps: &[(&str, Dep<'_>)],
    ) -> Result<Resolved, Vec<Diagnostic>> {
        let entry = self.entry(fixture(name).join("src/main.vr"));
        self.resolve_entry(entry, Kind::Binary, deps, None)
    }

    /// Checks `text` as the `src/main.vr` of a program with `deps`,
    /// through borrow analysis.
    pub(crate) fn check(
        &mut self,
        text: &str,
        deps: &[(&str, Dep<'_>)],
    ) -> Result<HirProgram, Vec<Diagnostic>> {
        self.resolve(text, deps)
            .and_then(|resolved| typecheck(resolved, &self.sources))
            .and_then(|hir| analyze(hir, &self.sources))
    }
}

/// A build holding `units`, and `route`, which depends on it.
pub(crate) fn units_and_route() -> Build {
    let mut build = Build::default();
    build.add("units", &[]);
    build.add("route", &[("units", Dep::Package("units"))]);
    build
}

/// A build holding the stub `varyk-http` of `tests/fixtures/packages/`
/// (milestone 5b4 spec 6), which depends on `varyk-std`.
pub(crate) fn varyk_http() -> Build {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/packages/varyk-http");
    let mut build = Build::default();
    build.add_dir("varyk-http", dir, &[("varyk-std", Dep::Rust)]);
    build
}
