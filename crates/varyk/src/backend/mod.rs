//! Backends (spec 6.4): turn a checked [`HirProgram`] into an artifact.

use std::path::PathBuf;

use varyk_syntax::Span;

use crate::hir::HirProgram;

mod rust;
mod rust_expr;
mod writer;

pub use rust::RustBackend;
pub use writer::{GeneratedFile, Writer};

/// A generated Cargo project, not yet written to disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratedCrate {
    pub package_name: String,
    pub cargo_toml: String,
    /// Every generated `.rs` file, each with its own line-level source
    /// map (spec 5).
    pub files: Vec<GeneratedFile>,
    /// Every copied `.rs` module: its tree path (e.g. `src/greet.rs`) and
    /// the user's file to copy it from byte-for-byte (spec 2.3).
    pub copied: Vec<(String, PathBuf)>,
}

impl GeneratedCrate {
    /// A read-only view over every generated file's line-level source map
    /// (spec 5), for mapping a rustc message back to Varyk source.
    pub fn source_map(&self) -> SourceMap<'_> {
        SourceMap { files: &self.files }
    }
}

/// The crate the generated tree is built as (M3 spec 2.4): its name,
/// and the whole `Cargo.toml` it is built with, a package's isolated
/// manifest (`Package::isolated_manifest`, the same one `varyk publish`
/// writes, so the two builds cannot disagree) or a single file's minimal
/// one.
#[derive(Debug, Clone, PartialEq)]
pub struct CrateInfo {
    /// The crate's name; the same as `manifest`'s `package.name`, kept
    /// here because the generated tree is named after it before any
    /// manifest is written.
    pub name: String,
    pub manifest: toml::Table,
    /// The `varyk-std` dependency a single file's manifest was given (M5a
    /// spec 5.2); `None` for a file that does not use `varyk-std` and for
    /// a package, whose own manifest names it.
    pub std_dependency: Option<StdDependency>,
}

/// Where a single file's crate gets `varyk-std` from (M5a spec 5.2, 5.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StdDependency {
    /// Exactly this version, from crates.io: the compiler's own.
    Version(String),
    /// The crate in this directory, from `VARYK_STD_PATH`, which the test
    /// suite sets to build against the workspace's `crates/varyk-std`.
    Path(PathBuf),
}

/// The environment variable naming a local `varyk-std` to build against
/// (M5a spec 5.3); not documented for users.
pub const STD_PATH_VAR: &str = "VARYK_STD_PATH";

impl StdDependency {
    /// The dependency of a program that uses `varyk-std` (`uses_std`), or
    /// `None`: the directory `VARYK_STD_PATH` names when it is set, else
    /// the compiler's version.
    pub fn for_program(uses_std: bool) -> Option<StdDependency> {
        if !uses_std {
            return None;
        }
        Some(match local_std() {
            Some(path) => StdDependency::Path(path),
            None => StdDependency::Version(env!("CARGO_PKG_VERSION").to_string()),
        })
    }

    /// The dependency's `Cargo.toml` value: `"=0.2.0"`, or `{ path = ".." }`.
    fn value(&self) -> toml::Value {
        match self {
            StdDependency::Version(version) => toml::Value::String(format!("={version}")),
            StdDependency::Path(path) => path_value(path),
        }
    }
}

/// The directory `VARYK_STD_PATH` names, when it is set and not empty.
fn local_std() -> Option<PathBuf> {
    std::env::var_os(STD_PATH_VAR)
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
}

/// `{ path = ".." }` for `path`.
fn path_value(path: &std::path::Path) -> toml::Value {
    let mut table = toml::Table::new();
    table.insert(
        "path".to_string(),
        toml::Value::String(path.to_string_lossy().into_owned()),
    );
    toml::Value::Table(table)
}

/// A package's isolated manifest with its `varyk-std` dependency, if it
/// has one, replaced by a `path` dependency on the directory
/// `VARYK_STD_PATH` names, when that is set (M5a spec 5.3). `varyk
/// publish` never calls this.
pub fn with_local_std(mut manifest: toml::Table) -> toml::Table {
    let Some(path) = local_std() else {
        return manifest;
    };
    if let Some(toml::Value::Table(dependencies)) = manifest.get_mut("dependencies") {
        if dependencies.contains_key("varyk-std") {
            dependencies.insert("varyk-std".to_string(), path_value(&path));
        }
    }
    manifest
}

impl CrateInfo {
    /// A single file's crate: `name`, version `0.0.0`, edition 2024, an
    /// empty `[workspace]` table so an enclosing workspace does not claim
    /// it, and no dependencies but `varyk-std` when `std_dependency` is
    /// set (M5a spec 5.2).
    pub fn single_file(name: String, std_dependency: Option<StdDependency>) -> Self {
        let mut package = toml::Table::new();
        package.insert("name".to_string(), toml::Value::String(name.clone()));
        package.insert(
            "version".to_string(),
            toml::Value::String("0.0.0".to_string()),
        );
        package.insert(
            "edition".to_string(),
            toml::Value::String("2024".to_string()),
        );
        let mut manifest = toml::Table::new();
        manifest.insert("package".to_string(), toml::Value::Table(package));
        manifest.insert(
            "workspace".to_string(),
            toml::Value::Table(toml::Table::new()),
        );
        let mut dependencies = toml::Table::new();
        if let Some(dependency) = &std_dependency {
            dependencies.insert("varyk-std".to_string(), dependency.value());
        }
        manifest.insert("dependencies".to_string(), toml::Value::Table(dependencies));
        CrateInfo {
            name,
            manifest,
            std_dependency,
        }
    }
}

/// Takes a checked program and produces a generated crate.
pub trait Backend {
    fn generate(&self, program: &HirProgram, info: &CrateInfo) -> GeneratedCrate;
}

/// A read-only view over a [`GeneratedCrate`]'s generated files, for
/// mapping a rustc diagnostic's file and line back to the Varyk span that
/// produced it.
pub struct SourceMap<'a> {
    files: &'a [GeneratedFile],
}

impl SourceMap<'_> {
    /// The Varyk span that produced `path`'s line `line` (1-based, as
    /// rustc's JSON messages report it). `None` when `path` is not one of
    /// this crate's generated files, `line` is out of range, or the line
    /// is structural and carries no span.
    pub fn lookup(&self, path: &str, line: u32) -> Option<Span> {
        let file = self.files.iter().find(|file| file.path == path)?;
        let index = line.checked_sub(1)?;
        file.lines.get(index as usize).copied().flatten()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use varyk_syntax::FileId;

    fn span(start: u32, end: u32) -> Span {
        Span::new(FileId(0), start, end)
    }

    fn crate_with(files: Vec<GeneratedFile>) -> GeneratedCrate {
        GeneratedCrate {
            package_name: "p".to_string(),
            cargo_toml: String::new(),
            files,
            copied: Vec::new(),
        }
    }

    #[test]
    fn lookup_finds_the_span_at_a_1_based_line_in_the_named_file() {
        let mut writer = Writer::new("src/main.rs");
        writer.line(0, "fn main() {", Some(span(0, 10)));
        writer.line(1, "let x = 1;", Some(span(15, 25)));
        writer.line(0, "}", None);
        let generated = crate_with(vec![writer.finish()]);

        let map = generated.source_map();

        assert_eq!(map.lookup("src/main.rs", 1), Some(span(0, 10)));
        assert_eq!(map.lookup("src/main.rs", 2), Some(span(15, 25)));
    }

    #[test]
    fn lookup_is_none_for_a_structural_line_an_unknown_file_or_an_out_of_range_line() {
        let mut writer = Writer::new("src/main.rs");
        writer.line(0, "fn main() {", Some(span(0, 10)));
        writer.line(0, "}", None);
        let generated = crate_with(vec![writer.finish()]);

        let map = generated.source_map();

        assert_eq!(map.lookup("src/main.rs", 2), None, "a structural line");
        assert_eq!(map.lookup("src/other.rs", 1), None, "an unknown file");
        assert_eq!(map.lookup("src/main.rs", 99), None, "an out-of-range line");
        assert_eq!(map.lookup("src/main.rs", 0), None, "line 0 is not 1-based");
    }

    #[test]
    fn a_single_file_manifest_has_no_dependencies() {
        let text = CrateInfo::single_file("hello".to_string(), StdDependency::for_program(false))
            .manifest
            .to_string();
        let manifest: toml::Table = text.parse().expect("valid TOML");

        assert_eq!(
            manifest["package"]["name"].as_str(),
            Some("hello"),
            "{text}"
        );
        assert_eq!(manifest["package"]["version"].as_str(), Some("0.0.0"));
        assert_eq!(manifest["package"]["edition"].as_str(), Some("2024"));
        assert!(manifest["workspace"].as_table().unwrap().is_empty());
        assert!(manifest["dependencies"].as_table().unwrap().is_empty());
    }

    #[test]
    fn a_single_file_using_varyk_std_depends_on_it_exactly_or_by_path() {
        let version = StdDependency::Version("0.3.1".to_string());
        let info = CrateInfo::single_file("p".to_string(), Some(version.clone()));
        assert_eq!(info.std_dependency, Some(version));
        let text = info.manifest.to_string();
        let manifest: toml::Table = text.parse().expect("valid TOML");
        assert_eq!(
            manifest["dependencies"]["varyk-std"].as_str(),
            Some("=0.3.1"),
            "{text}"
        );

        let local = StdDependency::Path(PathBuf::from("/w/crates/varyk-std"));
        let text = CrateInfo::single_file("p".to_string(), Some(local))
            .manifest
            .to_string();
        let manifest: toml::Table = text.parse().expect("valid TOML");
        assert_eq!(
            manifest["dependencies"]["varyk-std"]["path"].as_str(),
            Some("/w/crates/varyk-std"),
            "{text}"
        );
    }
}
