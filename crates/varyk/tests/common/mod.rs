//! Shared helper for the CLI integration tests: spawns
//! the real `varyk` binary from the workspace root, so relative paths in
//! test arguments (e.g. `examples/hello.vr`) resolve the same way they
//! would for a user running `varyk` from the repository root.

use std::path::PathBuf;
use std::process::{Command, Output};

/// Runs `varyk` with `args`, from the workspace root, and returns its
/// captured output.
pub fn varyk(args: &[&str]) -> Output {
    varyk_with_env(args, &[])
}

/// Like [`varyk`], but with additional environment variables set on the
/// spawned process.
pub fn varyk_with_env(args: &[&str], env: &[(&str, &str)]) -> Output {
    let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("workspace root should exist and be canonicalizable");

    Command::new(env!("CARGO_BIN_EXE_varyk"))
        .args(args)
        .current_dir(&workspace_root)
        .envs(env.iter().copied())
        .output()
        .expect("failed to spawn the varyk binary")
}
