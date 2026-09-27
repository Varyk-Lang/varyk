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
}

impl CrateInfo {
    /// A single file's crate: `name`, version `0.0.0`, edition 2024, no
    /// dependencies, and an empty `[workspace]` table so an enclosing
    /// workspace does not claim it.
    pub fn single_file(name: String) -> Self {
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
        manifest.insert(
            "dependencies".to_string(),
            toml::Value::Table(toml::Table::new()),
        );
        CrateInfo { name, manifest }
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
        let text = CrateInfo::single_file("hello".to_string())
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
}
