//! The driver (spec 6.4): writes a [`GeneratedCrate`] to its build
//! directory, runs cargo on it, and executes the result.
//!
//! For a single file, everything lives under `target/varyk/` relative to
//! the current directory: one build directory per entry file, and one
//! shared cargo target directory, `target/varyk/cache/`, so dependencies
//! compile once across programs. A package's hidden crate and cache live
//! under its own root instead (M3 spec 2.4, `Package::hidden_crate_dir`).

use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};

use crate::backend::GeneratedCrate;

mod cargo;
pub mod generate;
pub mod init;
pub mod messages;
pub mod publish;

pub use cargo::DriverError;
pub use generate::emit_rust;
pub use messages::Message;
pub use publish::assemble;

/// Root of everything the driver writes for a single file, relative to
/// the current directory.
const VARYK_DIR: &str = "target/varyk";

/// The shared cargo target directory for a single file, relative to the
/// current directory.
pub fn cache_dir() -> PathBuf {
    Path::new(VARYK_DIR).join("cache")
}

/// FNV-1a 64-bit hash of `bytes`, truncated to its low 32 bits. Used
/// instead of [`std::hash::DefaultHasher`], which is not stable across
/// toolchains and would change the build directory and package name for
/// the same entry file on a compiler upgrade.
fn fnv1a_32(bytes: &[u8]) -> u32 {
    const OFFSET_BASIS: u64 = 0xcbf29ce484222325;
    const PRIME: u64 = 0x100000001b3;
    let mut hash = OFFSET_BASIS;
    for &byte in bytes {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(PRIME);
    }
    hash as u32
}

/// `<stem>-<8 hex chars of a hash of the absolute entry path>`: the
/// unsanitized name shared by the build directory and the package name.
fn name_for(entry: &Path) -> String {
    let absolute = std::path::absolute(entry).unwrap_or_else(|_| entry.to_path_buf());
    let hash = fnv1a_32(absolute.to_string_lossy().as_bytes());
    let stem = entry
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_default();
    format!("{stem}-{hash:08x}")
}

/// The build directory for `entry`: `target/varyk/<stem>-<hash>`, relative
/// to the current directory. Two entries with the same stem in different
/// directories get different build directories.
pub fn build_dir_for(entry: &Path) -> PathBuf {
    Path::new(VARYK_DIR).join(name_for(entry))
}

/// The cargo package (and binary) name for `entry`: the build directory's
/// name, sanitized into a crate identifier.
pub fn package_name_for(entry: &Path) -> String {
    sanitize_name(&name_for(entry))
}

/// Lowercases `name` and replaces every character that is not an ASCII
/// letter or digit with `_`; prefixes `v_` if the result would otherwise
/// start with a digit, be empty, or collide with `cache` (the shared
/// build cache directory, spec 2.4), `package` (where `varyk publish`
/// assembles its crate), or a folder of Cargo's own that a program cannot
/// be named after (`deps`, `examples`, `build`, `incremental`) — the ways
/// a sanitized name could fail `package::name_problem` or
/// `package::program_name_problem`. Shared by `package_name_for` (a build
/// directory's name) and `init` (a directory's own name).
pub fn sanitize_name(name: &str) -> String {
    let sanitized: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect();
    if sanitized.is_empty()
        || sanitized.starts_with(|c: char| c.is_ascii_digit())
        || sanitized == "cache"
        || sanitized == "package"
        || crate::package::program_name_problem(&sanitized).is_some()
    {
        format!("v_{sanitized}")
    } else {
        sanitized
    }
}

/// Writes `generated` under `build_dir`, copies `lock` (the package's
/// `Cargo.lock`, when it has one) in beside it (M3 spec 2.2; never copied
/// back), builds it with cargo into `target_dir`, and returns the
/// executable's path, read from cargo's `compiler-artifact` message, or
/// `None` for a library, which has none. This is the only reliable way to
/// find it: under a configured target (`CARGO_BUILD_TARGET` or
/// `[build].target`), cargo nests the profile directory under an extra
/// `<triple>/` path component that `build` cannot predict on its own.
///
/// Files whose content is already on disk are left untouched, so cargo's
/// freshness check keeps working and concurrent builds of one program do
/// not rewrite each other's files. Cargo's output is captured; on failure
/// its stderr and every classified compiler message are returned in
/// [`DriverError::Cargo`].
///
/// Every compiler message from the run is classified (spec 5) and
/// returned alongside the executable, on success as well as failure, so a
/// build that succeeds with a warning in a copied `.rs` module still
/// reports it (only cargo's own exit status distinguishes failure; a
/// warning never fails the build).
pub fn build(
    generated: &GeneratedCrate,
    build_dir: &Path,
    target_dir: &Path,
    lock: Option<&Path>,
    release: bool,
) -> Result<(Option<PathBuf>, Vec<Message>), DriverError> {
    generate::write_files(build_dir, generated)?;
    generate::sync_lock(lock, &build_dir.join("Cargo.lock"))?;
    let binary = generated
        .files
        .iter()
        .any(|file| file.path == "src/main.rs");
    let map = generated.source_map();
    cargo::run_cargo(
        build_dir,
        target_dir,
        release,
        &generated.package_name,
        binary,
        &map,
        &generated.copied,
    )
}

/// Runs the built program with `args`, handing it this process's stdin,
/// stdout, and stderr, and returns its exit status.
pub fn run(exe: &Path, args: &[String]) -> io::Result<ExitStatus> {
    Command::new(exe).args(args).status()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_stem_in_different_directories_gets_different_names() {
        let interop = Path::new("examples/interop/main.vr");
        let modules = Path::new("examples/modules/main.vr");

        assert_ne!(build_dir_for(interop), build_dir_for(modules));
        assert_ne!(package_name_for(interop), package_name_for(modules));
    }

    /// Pinned so a future change to [`fnv1a_32`] (or a swap for a
    /// different hash) is caught: unlike `DefaultHasher`, this hash must
    /// give the same build directory and package name across toolchains.
    #[test]
    fn fnv1a_32_of_a_fixed_string_is_pinned() {
        assert_eq!(fnv1a_32(b"hello"), 0x80aabd0b);
    }

    #[test]
    fn build_dir_is_the_stem_and_eight_hex_chars_under_target_varyk() {
        let dir = build_dir_for(Path::new("examples/hello.vr"));

        assert_eq!(dir.parent(), Some(Path::new("target/varyk")));
        let name = dir.file_name().unwrap().to_str().unwrap();
        let hash = name.strip_prefix("hello-").expect("name starts with stem");
        assert_eq!(hash.len(), 8);
        assert!(hash.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn package_name_is_the_build_dir_name_sanitized() {
        let entry = Path::new("examples/hello.vr");
        let dir_name = build_dir_for(entry)
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();

        assert_eq!(package_name_for(entry), dir_name.replace('-', "_"));
    }

    #[test]
    fn a_hyphen_in_the_stem_becomes_an_underscore() {
        let name = package_name_for(Path::new("my-Program.vr"));

        assert!(name.starts_with("my_program_"), "{name}");
        assert!(
            name.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'),
            "{name}"
        );
    }

    #[test]
    fn a_leading_digit_gets_a_v_prefix() {
        let name = package_name_for(Path::new("2fast.vr"));

        assert!(name.starts_with("v_2fast_"), "{name}");
    }

    /// A directory name sanitized for `init` must always pass the
    /// manifest's own name rule (`package::name_problem`), including its
    /// edge cases that a plain build-directory name never hits: a
    /// directory literally called `cache` or `package`, one named after a
    /// folder of Cargo's own (`init` makes a program by default), and one
    /// whose name has no ASCII letters or digits at all.
    #[test]
    fn a_sanitized_directory_name_always_passes_the_manifest_name_rule() {
        for name in [
            "My Project",
            "2fast",
            "cache",
            "package",
            "deps",
            "Examples",
            "build",
            "incremental",
            "..",
            "café",
            "___",
        ] {
            let sanitized = sanitize_name(name);
            assert_eq!(
                crate::package::name_problem(&sanitized),
                None,
                "{name:?} -> {sanitized:?}"
            );
            assert_eq!(
                crate::package::program_name_problem(&sanitized),
                None,
                "{name:?} -> {sanitized:?}"
            );
        }
    }
}
