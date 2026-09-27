//! Invoking cargo on a written-out [`GeneratedCrate`] and reading back the
//! executable it produced, along with every compiler message cargo
//! reported (spec 5, classified by [`super::messages`]).

use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use super::messages::{self, Message};
use crate::backend::SourceMap;

/// Why [`super::build`] failed.
#[derive(Debug)]
pub enum DriverError {
    /// Writing the crate failed.
    Io(io::Error),
    /// Cargo could not be started (not installed, or not on the path).
    Spawn(io::Error),
    /// Cargo ran and rejected the generated crate; carries cargo's raw
    /// stderr (the fallback when no compiler message was reported at
    /// all, e.g. a linker failure) and every compiler message cargo
    /// reported, classified (spec 5).
    Cargo {
        stderr: String,
        messages: Vec<Message>,
    },
}

impl From<io::Error> for DriverError {
    fn from(err: io::Error) -> Self {
        DriverError::Io(err)
    }
}

/// Runs `cargo build` on the crate already written at `build_dir`, into
/// the shared target directory `target_dir`, and returns the executable's
/// path, read from cargo's `compiler-artifact` message (`None` for a
/// library, `binary` false, which has none), alongside every compiler
/// message cargo reported, classified against `map` and `copied` (spec
/// 5). This is the only reliable way to find the executable: under a
/// configured target (`CARGO_BUILD_TARGET` or `[build].target`), cargo
/// nests the profile directory under an extra `<triple>/` path component
/// that this cannot predict on its own.
///
/// `--message-format=json` (not `-render-diagnostics`) is required so
/// every diagnostic arrives as a `compiler-message` line with a
/// `rendered` field this can classify and print itself, instead of cargo
/// rendering it straight to stderr.
///
/// Cargo's output is captured; on failure its stderr and the classified
/// messages are returned in [`DriverError::Cargo`].
pub(super) fn run_cargo(
    build_dir: &Path,
    target_dir: &Path,
    release: bool,
    package_name: &str,
    binary: bool,
    map: &SourceMap<'_>,
    copied: &[(String, PathBuf)],
) -> Result<(Option<PathBuf>, Vec<Message>), DriverError> {
    let manifest = std::path::absolute(build_dir.join("Cargo.toml"))?;
    let target_dir = std::path::absolute(target_dir)?;
    let crate_src = std::path::absolute(build_dir)?;

    let mut cargo = Command::new("cargo");
    cargo
        .arg("build")
        .arg("--manifest-path")
        .arg(&manifest)
        .arg("--target-dir")
        .arg(&target_dir)
        .arg("--message-format=json");
    if release {
        cargo.arg("--release");
    }
    // Cargo finds `.cargo/config.toml`, and rustup a `rust-toolchain`
    // file, by walking up from the current directory, not from
    // `--manifest-path`; run from inside the package tree so the
    // package's own settings apply wherever `varyk` was invoked from.
    let output = cargo
        .current_dir(&crate_src)
        .stdin(Stdio::null())
        .output()
        .map_err(DriverError::Spawn)?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let classified = classify_messages(stdout.lines(), map, copied, &crate_src);

    if !output.status.success() {
        return Err(DriverError::Cargo {
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            messages: classified,
        });
    }

    if !binary {
        return Ok((None, classified));
    }
    match executable_from(stdout.lines(), package_name) {
        Some(exe) => Ok((Some(PathBuf::from(exe)), classified)),
        // Cargo exited 0 but named no binary artifact for this package: an
        // anomaly, not a compile error, so `classified` (built from the
        // same successful run) is the only messages worth carrying, not
        // an empty list.
        None => Err(DriverError::Cargo {
            stderr: format!(
                "cargo produced no executable\n{}",
                String::from_utf8_lossy(&output.stderr)
            ),
            messages: classified,
        }),
    }
}

/// Reads every `compiler-message` line of cargo's `--message-format=json`
/// output and classifies it (spec 5); other reasons (`compiler-artifact`,
/// `build-finished`, ...) are ignored. A V0900 that may follow from an
/// error in a user's `.rs` file says so (see
/// [`messages::after_user_errors`]).
fn classify_messages<'a>(
    lines: impl Iterator<Item = &'a str>,
    map: &SourceMap<'_>,
    copied: &[(String, PathBuf)],
    crate_src: &Path,
) -> Vec<Message> {
    let classified = lines
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter(|value| value.get("reason").and_then(|r| r.as_str()) == Some("compiler-message"))
        .filter_map(|value| value.get("message").cloned())
        .filter_map(|message| messages::classify(&message, map, copied, crate_src))
        .collect();
    messages::after_user_errors(classified)
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

#[cfg(test)]
mod tests {
    use super::*;

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
