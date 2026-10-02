//! Shared helper for the CLI integration tests: spawns
//! the real `varyk` binary from the workspace root, so relative paths in
//! test arguments (e.g. `examples/hello.vr`) resolve the same way they
//! would for a user running `varyk` from the repository root.

#![allow(dead_code)] // each test binary uses a different subset

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU32, Ordering};

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
        .env("VARYK_STD_PATH", std_path())
        .envs(env.iter().copied())
        .output()
        .expect("failed to spawn the varyk binary")
}

/// Runs `varyk` with `args` from `cwd` and returns its captured output.
pub fn varyk_in(cwd: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_varyk"))
        .args(args)
        .current_dir(cwd)
        .env("VARYK_STD_PATH", std_path())
        .output()
        .expect("failed to spawn the varyk binary")
}

/// Runs `varyk` with `args` from `dir`, with the variables in `set` set
/// and those in `remove` removed from the environment it inherits, and
/// `VARYK_STD_PATH` always set: for programs whose output depends on the
/// environment or the current directory (`env::parse`, `log`, `.env`), so
/// a tester's own variables cannot change it.
pub fn varyk_run_with(args: &[&str], dir: &Path, set: &[(&str, &str)], remove: &[&str]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_varyk"));
    command.args(args).current_dir(dir);
    for name in remove {
        command.env_remove(name);
    }
    command
        .envs(set.iter().copied())
        .env("VARYK_STD_PATH", std_path())
        .output()
        .expect("failed to spawn the varyk binary")
}

/// The workspace's `crates/varyk-std`, which every generated crate a test
/// builds depends on through `VARYK_STD_PATH` until a version is on
/// crates.io (M5a spec 5.3).
pub fn std_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../varyk-std")
        .canonicalize()
        .expect("crates/varyk-std should exist")
}

/// The value of cargo's `--config` that points `varyk-std` at the
/// workspace's copy, for the tests that run plain cargo on a package: the
/// driver is not involved there, so `VARYK_STD_PATH` does nothing (M5a
/// spec 5.3).
pub fn std_config() -> String {
    format!(
        "patch.crates-io.varyk-std.path=\"{}\"",
        std_path().display()
    )
}

/// A fresh, empty directory under `CARGO_TARGET_TMPDIR`, named `label`
/// plus a counter so repeated calls with the same label never collide;
/// for tests (`init`) that build a package from nothing rather
/// than from a fixture.
pub fn empty_dir(label: &str) -> PathBuf {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("packages")
        .join(format!("{label}-{}-{n}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create the empty dir");
    dir
}

/// A fresh copy of the package fixture `tests/fixtures/packages/<case>/`
/// under `CARGO_TARGET_TMPDIR`, so a test that builds it never writes
/// into the source tree. Each call gets its own directory.
pub fn package_dir(case: &str) -> PathBuf {
    fixture_dir("packages", case)
}

/// A fresh copy of the fixture `tests/fixtures/<kind>/<case>/` under
/// `CARGO_TARGET_TMPDIR/<kind>/`, one per call: for a package fixture that
/// is built, or whose package graph is read (M5b2 spec 9), which writes
/// its `target/`.
pub fn fixture_dir(kind: &str, case: &str) -> PathBuf {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(kind)
        .join(case);
    let dest = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join(kind)
        .join(format!("{case}-{}-{n}", std::process::id()));
    let _ = fs::remove_dir_all(&dest);
    copy_dir(&source, &dest);
    dest
}

/// A fresh copy of the example package `examples/packages/<name>/` under
/// `CARGO_TARGET_TMPDIR/examples/<pid>/<n>/<name>/`, one per call, so that building it never
/// writes into the source tree (its `target/`, its `Cargo.lock`) and two
/// test processes in one checkout never clear each other's copy. The
/// example packages it depends on by `path = "../<other>"`, and those
/// they depend on, are copied beside it (M5b2 spec 4.7). The
/// first call in a process removes the directory of any earlier process
/// that has ended (or after an hour, in case its id was reused), and
/// anything else there once it is a day old, as the soundness harness
/// does with its scratch directories.
pub fn example_dir(name: &str) -> PathBuf {
    static ROOT: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    let root = ROOT.get_or_init(|| {
        let parent = Path::new(env!("CARGO_TARGET_TMPDIR")).join("examples");
        sweep(&parent);
        parent.join(std::process::id().to_string())
    });
    // A fresh directory per call: two tests of one process copying the
    // same example must not clear each other's copy.
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let examples = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/packages");
    let parent = root.join(n.to_string());
    let _ = fs::remove_dir_all(&parent);
    let mut copied: Vec<String> = Vec::new();
    let mut next = vec![name.to_string()];
    while let Some(package) = next.pop() {
        if copied.contains(&package) {
            continue;
        }
        copy_dir(&examples.join(&package), &parent.join(&package));
        next.extend(sibling_dependencies(&examples.join(&package)));
        copied.push(package);
    }
    parent.join(name)
}

/// The directory names of the packages the package at `dir` lists under
/// `[dependencies]` with `path = "../<name>"`.
fn sibling_dependencies(dir: &Path) -> Vec<String> {
    let text = fs::read_to_string(dir.join("Cargo.toml")).expect("read the example's Cargo.toml");
    let manifest: toml::Table = text.parse().expect("the example's Cargo.toml is TOML");
    let Some(toml::Value::Table(dependencies)) = manifest.get("dependencies") else {
        return Vec::new();
    };
    dependencies
        .values()
        .filter_map(|dependency| dependency.get("path")?.as_str())
        .filter_map(|path| path.strip_prefix("../"))
        .map(str::to_string)
        .collect()
}

/// Removes from `parent` the directory of every other process that has
/// ended, or that is over an hour old in case its id was reused, and any
/// other entry over a day old; the current process's own is kept.
pub fn sweep(parent: &Path) {
    let hour = std::time::Duration::from_secs(60 * 60);
    let own = std::process::id().to_string();
    for entry in fs::read_dir(parent).into_iter().flatten().flatten() {
        let older_than = |limit| {
            entry
                .metadata()
                .and_then(|meta| meta.modified())
                .is_ok_and(|modified| modified.elapsed().is_ok_and(|age| age > limit))
        };
        let name = entry.file_name().to_string_lossy().into_owned();
        let stale = match name.parse::<u32>() {
            Ok(_) if name != own => !process_running(&name) || older_than(hour),
            Ok(_) => false,
            Err(_) => older_than(24 * hour),
        };
        if stale {
            let _ = fs::remove_dir_all(entry.path());
        }
    }
}

/// Whether a process with id `pid` is running, as `kill -0` tells.
fn process_running(pid: &str) -> bool {
    Command::new("kill")
        .args(["-0", pid])
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

/// Copies the directory `from` to `to`, recursively, leaving out every
/// `target/` directory (build output, never part of a package's source).
fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).expect("create the fixture copy");
    for entry in fs::read_dir(from).expect("read the fixture") {
        let path = entry.expect("read the fixture").path();
        let dest = to.join(path.file_name().expect("a named entry"));
        if path.is_dir() && path.file_name() == Some("target".as_ref()) {
            continue;
        }
        if path.is_dir() {
            copy_dir(&path, &dest);
        } else {
            fs::copy(&path, &dest).expect("copy a fixture file");
        }
    }
}
