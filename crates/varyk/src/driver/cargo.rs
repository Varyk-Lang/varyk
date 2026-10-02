//! Invoking cargo on a written-out [`GeneratedCrate`] and reading back the
//! executable it produced, along with every compiler message cargo
//! reported (spec 5, classified by [`super::messages`]).

use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use super::PackageCrate;
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

/// What [`run_cargo`] asks of cargo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Goal {
    /// `cargo build`, with `--release` when `release` is set.
    Build { release: bool },
    /// `cargo test --no-run`: build the test executables, run none (M5a
    /// spec 2.7).
    Test,
}

/// Runs cargo for `goal` on the crate already written at `build_dir`,
/// into the shared target directory `target_dir`, and returns cargo's
/// JSON output (stdout), from which the caller reads the executables it
/// built, and its stderr, alongside every compiler message cargo
/// reported, classified against `map` and `copied` (spec 5). The executables are read from cargo's
/// `compiler-artifact` messages because that is the only reliable way to
/// find them: under a configured target (`CARGO_BUILD_TARGET` or
/// `[build].target`), cargo nests the profile directory under an extra
/// `<triple>/` path component that this cannot predict on its own.
///
/// `--message-format=json` (not `-render-diagnostics`) is required so
/// every diagnostic arrives as a `compiler-message` line with a
/// `rendered` field this can classify and print itself, instead of cargo
/// rendering it straight to stderr.
///
/// A message from the crate of a Varyk package of the build, one of
/// `packages`, is classified as that package's (M5b2 spec 4.3).
///
/// Cargo's output is captured; on failure its stderr and the classified
/// messages are returned in [`DriverError::Cargo`].
pub(super) fn run_cargo(
    build_dir: &Path,
    target_dir: &Path,
    goal: Goal,
    map: &SourceMap<'_>,
    copied: &[(String, PathBuf)],
    packages: &[PackageCrate],
) -> Result<(String, String, Vec<Message>), DriverError> {
    let manifest = std::path::absolute(build_dir.join("Cargo.toml"))?;
    let target_dir = std::path::absolute(target_dir)?;
    let crate_src = std::path::absolute(build_dir)?;

    let mut cargo = Command::new("cargo");
    match goal {
        Goal::Build { .. } => cargo.arg("build"),
        Goal::Test => cargo.arg("test").arg("--no-run"),
    };
    cargo
        .arg("--manifest-path")
        .arg(&manifest)
        .arg("--target-dir")
        .arg(&target_dir)
        .arg("--message-format=json");
    if goal == (Goal::Build { release: true }) {
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
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let classified = classify_messages(stdout.lines(), map, copied, &crate_src, packages);

    if !output.status.success() {
        return Err(DriverError::Cargo {
            stderr,
            messages: classified,
        });
    }
    Ok((stdout, stderr, classified))
}

/// Reads every `compiler-message` line of cargo's `--message-format=json`
/// output and classifies it (spec 5), as one of `packages`' when its
/// `manifest_path` is that package crate's (M5b2 spec 4.3); other reasons
/// (`compiler-artifact`, `build-finished`, ...) are ignored. A V0900 that
/// may follow from an error in a user's `.rs` file says so (see
/// [`messages::after_user_errors`]).
fn classify_messages<'a>(
    lines: impl Iterator<Item = &'a str>,
    map: &SourceMap<'_>,
    copied: &[(String, PathBuf)],
    crate_src: &Path,
    packages: &[PackageCrate],
) -> Vec<Message> {
    let canonical = crate::packages::canonical;
    let manifests: Vec<PathBuf> = packages
        .iter()
        .map(|package| canonical(&package.dir.join("Cargo.toml")))
        .collect();
    let classified = lines
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter(|value| value.get("reason").and_then(|r| r.as_str()) == Some("compiler-message"))
        .flat_map(|value| {
            let package = value
                .get("manifest_path")
                .and_then(|path| path.as_str())
                .map(|path| canonical(Path::new(path)))
                .and_then(|path| manifests.iter().position(|manifest| *manifest == path))
                .and_then(|index| packages.get(index));
            let Some(message) = value.get("message") else {
                return Vec::new();
            };
            match package {
                Some(package) => messages::classify_package(message, package),
                None => messages::classify(message, map, copied, crate_src)
                    .into_iter()
                    .collect(),
            }
        })
        .collect();
    messages::after_user_errors(classified)
}

/// The executable in cargo's JSON `lines`: the `compiler-artifact`
/// message of the `bin` target named `package`. Other artifacts (a
/// dependency's build script, say) may carry an `executable` too.
pub(super) fn executable_from<'a>(
    lines: impl Iterator<Item = &'a str>,
    package: &str,
) -> Option<String> {
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

/// The test executables in cargo's JSON `lines`: the `executable` of
/// every `compiler-artifact` message whose `profile.test` is set, in the
/// order cargo reported them.
pub(super) fn test_executables_from<'a>(lines: impl Iterator<Item = &'a str>) -> Vec<PathBuf> {
    lines
        .filter_map(|line| {
            let message = serde_json::from_str::<serde_json::Value>(line).ok()?;
            if message.get("reason")?.as_str()? != "compiler-artifact"
                || message.get("profile")?.get("test")?.as_bool() != Some(true)
            {
                return None;
            }
            Some(PathBuf::from(message.get("executable")?.as_str()?))
        })
        .collect()
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

    #[test]
    fn test_executables_are_the_artifacts_built_as_tests() {
        let lines = [
            r#"{"reason":"compiler-artifact","target":{"kind":["lib"],"name":"dep"},"profile":{"test":false},"executable":null}"#,
            r#"{"reason":"compiler-artifact","target":{"kind":["bin"],"name":"hello"},"profile":{"test":true},"executable":"/t/debug/deps/hello-1"}"#,
            r#"{"reason":"compiler-artifact","target":{"kind":["bin"],"name":"hello"},"profile":{"test":false},"executable":"/t/debug/hello"}"#,
            r#"{"reason":"build-finished","success":true}"#,
        ];
        assert_eq!(
            test_executables_from(lines.into_iter()),
            [PathBuf::from("/t/debug/deps/hello-1")]
        );
    }
}
