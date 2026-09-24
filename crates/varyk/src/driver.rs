//! The driver (spec 6.4): writes a [`GeneratedCrate`] to its build
//! directory, runs cargo on it, and executes the result.
//!
//! Everything lives under `target/varyk/` relative to the current
//! directory: one build directory per entry file, and one shared cargo
//! target directory, `target/varyk/cache/`, so dependencies compile once
//! across programs.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};

use crate::backend::GeneratedCrate;

/// Root of everything the driver writes, relative to the current directory.
const VARYK_DIR: &str = "target/varyk";

/// The shared cargo target directory, relative to the current directory.
const CACHE_DIR: &str = "target/varyk/cache";

/// Why [`build`] failed.
#[derive(Debug)]
pub enum DriverError {
    /// Writing the crate, or spawning cargo, failed.
    Io(io::Error),
    /// Cargo ran and rejected the generated crate; carries cargo's stderr.
    Cargo { stderr: String },
}

impl From<io::Error> for DriverError {
    fn from(err: io::Error) -> Self {
        DriverError::Io(err)
    }
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
/// name, lowercased, with every character other than an ASCII letter or
/// digit replaced by `_`, and prefixed with `v_` if it starts with a digit.
pub fn package_name_for(entry: &Path) -> String {
    let sanitized: String = name_for(entry)
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect();
    if sanitized.starts_with(|c: char| c.is_ascii_digit()) {
        format!("v_{sanitized}")
    } else {
        sanitized
    }
}

/// Writes `generated` under `build_dir`, builds it with cargo into the
/// shared target directory, and returns the executable's path, read from
/// cargo's `compiler-artifact` message. This is the only reliable way to
/// find it: under a configured target (`CARGO_BUILD_TARGET` or
/// `[build].target`), cargo nests the profile directory under an extra
/// `<triple>/` path component that `build` cannot predict on its own.
///
/// Files whose content is already on disk are left untouched, so cargo's
/// freshness check keeps working and concurrent builds of one program do
/// not rewrite each other's files. Cargo's output is captured; on failure
/// its stderr is returned in [`DriverError::Cargo`].
pub fn build(
    generated: &GeneratedCrate,
    build_dir: &Path,
    release: bool,
) -> Result<PathBuf, DriverError> {
    write_if_changed(&build_dir.join("Cargo.toml"), &generated.cargo_toml)?;
    for (relative, content) in &generated.files {
        write_if_changed(&build_dir.join(relative), content)?;
    }

    let manifest = std::path::absolute(build_dir.join("Cargo.toml"))?;
    let target_dir = std::path::absolute(CACHE_DIR)?;

    let mut cargo = Command::new("cargo");
    cargo
        .arg("build")
        .arg("--manifest-path")
        .arg(&manifest)
        .arg("--target-dir")
        .arg(&target_dir)
        .arg("--message-format=json-render-diagnostics");
    if release {
        cargo.arg("--release");
    }
    let output = cargo.stdin(Stdio::null()).output()?;
    if !output.status.success() {
        return Err(DriverError::Cargo {
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        });
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    executable_from(stdout.lines(), &generated.package_name)
        .map(PathBuf::from)
        .ok_or_else(|| DriverError::Cargo {
            stderr: format!(
                "cargo produced no executable\n{}",
                String::from_utf8_lossy(&output.stderr)
            ),
        })
}

/// The executable in cargo's JSON `lines`: the `compiler-artifact`
/// message of the `bin` target named `package`. Other artifacts (a
/// dependency's build script, say) may carry an `executable` too.
fn executable_from<'a>(lines: impl Iterator<Item = &'a str>, package: &str) -> Option<String> {
    lines.into_iter().find_map(|line| {
        let message = serde_json::from_str::<serde_json::Value>(line).ok()?;
        if message.get("reason")?.as_str()? != "compiler-artifact" {
            return None;
        }
        let target = message.get("target")?;
        let is_bin = target
            .get("kind")?
            .as_array()?
            .iter()
            .any(|kind| kind.as_str() == Some("bin"));
        if !is_bin || target.get("name")?.as_str()? != package {
            return None;
        }
        Some(message.get("executable")?.as_str()?.to_string())
    })
}

/// Writes `content` to `path`, creating parent directories, unless the
/// file already holds exactly `content`. The write goes to a temporary
/// file that is then renamed into place, so a concurrent reader never
/// sees a half-written file.
fn write_if_changed(path: &Path, content: &str) -> io::Result<()> {
    if fs::read(path).is_ok_and(|existing| existing == content.as_bytes()) {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut temporary = path.as_os_str().to_owned();
    temporary.push(format!(".tmp{}", std::process::id()));
    let temporary = PathBuf::from(temporary);
    fs::write(&temporary, content)?;
    // On Windows, `rename` fails if `path` already exists; clear it first
    // (ignoring "not found") so the rename below can succeed there too.
    #[cfg(windows)]
    {
        let _ = fs::remove_file(path);
    }
    fs::rename(&temporary, path)
}

/// The generated crate as text for `--emit-rust`: `Cargo.toml` and then
/// every file in order, each after a header line `// ==== <path> ====`.
/// `.rs` files pass through `rustfmt --edition 2024` when it is on the
/// path; if it is absent or fails, the content is used as is.
pub fn emit_rust(generated: &GeneratedCrate) -> String {
    let mut out = String::new();
    let cargo_toml = ("Cargo.toml".to_string(), generated.cargo_toml.clone());
    for (path, content) in std::iter::once(&cargo_toml).chain(&generated.files) {
        let content = if path.ends_with(".rs") {
            rustfmt(content).unwrap_or_else(|| content.clone())
        } else {
            content.clone()
        };
        out.push_str(&format!("// ==== {path} ====\n"));
        out.push_str(&content);
        if !content.ends_with('\n') {
            out.push('\n');
        }
    }
    out
}

/// Formats `source` with `rustfmt --edition 2024` over stdin and stdout;
/// `None` if rustfmt cannot be spawned or reports an error.
fn rustfmt(source: &str) -> Option<String> {
    let mut child = Command::new("rustfmt")
        .args(["--edition", "2024"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    // Write from another thread while this one collects the output, so
    // neither side can block on a full pipe.
    let mut stdin = child.stdin.take()?;
    let source = source.to_string();
    let writer = std::thread::spawn(move || stdin.write_all(source.as_bytes()));
    let output = child.wait_with_output().ok()?;
    let written = writer.join().ok()?;
    if written.is_err() || !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout).ok()
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

    #[test]
    fn executable_is_the_bin_artifact_of_the_package() {
        let lines = [
            r#"{"reason":"compiler-artifact","target":{"kind":["custom-build"],"name":"build-script-build"},"executable":"/t/build/dep/build-script-build"}"#,
            r#"{"reason":"compiler-artifact","target":{"kind":["bin"],"name":"hello"},"executable":"/t/debug/hello"}"#,
            r#"{"reason":"build-finished","success":true}"#,
        ];
        assert_eq!(
            executable_from(lines.into_iter(), "hello").as_deref(),
            Some("/t/debug/hello")
        );
        assert_eq!(executable_from(lines.into_iter(), "other"), None);
    }
}
