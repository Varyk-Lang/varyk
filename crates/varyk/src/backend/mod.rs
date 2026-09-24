//! Backends (spec 6.4): turn a checked [`HirProgram`] into an artifact.

use crate::hir::HirProgram;

mod rust;
mod rust_expr;

pub use rust::RustBackend;

/// A generated Cargo project, not yet written to disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratedCrate {
    pub package_name: String,
    pub cargo_toml: String,
    /// Each file's path relative to the crate root, and its content.
    pub files: Vec<(String, String)>,
}

/// Takes a checked program and produces a generated crate.
pub trait Backend {
    fn generate(&self, program: &HirProgram, package_name: &str) -> GeneratedCrate;
}
