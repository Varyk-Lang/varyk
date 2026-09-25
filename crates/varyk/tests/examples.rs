//! End-to-end tests for `varyk build` and `varyk run`: every
//! example builds through cargo and prints its expected output (milestone-1
//! spec section 5, milestone-2 spec section 4). These invoke cargo, so they are slower than the rest.

mod common;

use std::process::{Command, Output};

use common::{varyk, varyk_with_env};

fn stdout_of(output: &Output) -> String {
    String::from_utf8(output.stdout.clone()).expect("stdout should be valid utf-8")
}

fn stderr_of(output: &Output) -> String {
    String::from_utf8(output.stderr.clone()).expect("stderr should be valid utf-8")
}

/// Runs `path` and asserts exit 0, exactly `expected` on stdout, and an
/// empty stderr.
fn assert_runs(path: &str, expected: &str) {
    let output = varyk(&["run", path]);
    assert!(
        output.status.success(),
        "status: {:?}, stderr: {}",
        output.status,
        stderr_of(&output)
    );
    assert_eq!(stdout_of(&output), expected);
    assert_eq!(stderr_of(&output), "");
}

#[test]
fn run_hello() {
    assert_runs("examples/hello.vr", "Hello, world!\n");
}

#[test]
fn run_functions() {
    assert_runs("examples/functions.vr", "42\n");
}

#[test]
fn run_structs() {
    assert_runs("examples/structs.vr", "Alice\nAlice\n");
}

#[test]
fn run_borrowing() {
    assert_runs("examples/borrowing.vr", "Alice\nBob\n");
}

#[test]
fn run_modules() {
    assert_runs("examples/modules/main.vr", "49\n");
}

#[test]
fn run_interop() {
    assert_runs("examples/interop/main.vr", "Hello from Rust, Varyk!\n");
}

#[test]
fn run_enums() {
    assert_runs("examples/enums.vr", "3.14\n6\n0\n");
}

#[test]
fn run_methods() {
    assert_runs("examples/methods.vr", "0\n3\n");
}

#[test]
fn run_collections() {
    assert_runs("examples/collections.vr", "3\n10\n20\n30\n60\n2\n");
}

#[test]
fn run_errors() {
    assert_runs("examples/errors.vr", "84\nnot a number: abc\nnone\n");
}

#[test]
fn run_strings() {
    assert_runs(
        "examples/strings.vr",
        "Alice\nAlice\nHello, Alice!\n5\ntrue\n",
    );
}

#[test]
fn run_todo() {
    assert_runs(
        "examples/todo/main.vr",
        "[ ] Buy milk\n[ ] Write spec\n[x] Buy milk\n[ ] Write spec\n1 of 2 done\n",
    );
}

#[test]
fn run_with_trailing_arguments_still_prints_the_greeting() {
    let output = varyk(&["run", "examples/hello.vr", "--", "a", "b"]);
    assert!(output.status.success(), "stderr: {}", stderr_of(&output));
    assert_eq!(stdout_of(&output), "Hello, world!\n");
}

#[test]
fn build_with_emit_rust_prints_the_rust_then_the_executable_path() {
    let output = varyk(&["build", "--emit-rust", "examples/hello.vr"]);
    assert!(output.status.success(), "stderr: {}", stderr_of(&output));

    let stdout = stdout_of(&output);
    let lines: Vec<&str> = stdout.lines().collect();
    assert!(
        lines
            .iter()
            .any(|line| line.starts_with("// ====") && line.contains("src/main.rs")),
        "no header naming src/main.rs in: {stdout}"
    );
    assert!(
        lines.iter().any(|line| line.contains("fn main")),
        "no `fn main` in: {stdout}"
    );
    let last = lines
        .last()
        .expect("stdout should not be empty")
        .replace('\\', "/");
    assert!(
        last.contains("target/varyk/cache/") && last.contains("/debug/"),
        "last line is not the executable path: {last:?}"
    );
}

#[test]
fn build_release_prints_a_release_executable_path() {
    let output = varyk(&["build", "--release", "examples/functions.vr"]);
    assert!(output.status.success(), "stderr: {}", stderr_of(&output));

    let stdout = stdout_of(&output);
    let last = stdout
        .lines()
        .last()
        .expect("stdout should not be empty")
        .replace('\\', "/");
    assert!(
        last.contains("target/varyk/cache/") && last.contains("/release/"),
        "not a release path: {last:?}"
    );
}

/// `rustc -vV`'s `host:` line: the triple for the compiler that built this
/// test binary.
fn host_triple() -> String {
    let output = Command::new("rustc")
        .arg("-vV")
        .output()
        .expect("failed to spawn rustc -vV");
    assert!(output.status.success(), "rustc -vV failed");
    let stdout = String::from_utf8(output.stdout).expect("rustc -vV output should be utf-8");
    stdout
        .lines()
        .find_map(|line| line.strip_prefix("host: "))
        .expect("rustc -vV output should have a host: line")
        .to_string()
}

#[test]
fn run_with_a_configured_target_still_finds_the_executable() {
    let host = host_triple();
    let output = varyk_with_env(
        &["run", "examples/hello.vr"],
        &[("CARGO_BUILD_TARGET", &host)],
    );
    assert!(output.status.success(), "stderr: {}", stderr_of(&output));
    assert_eq!(stdout_of(&output), "Hello, world!\n");
}

#[test]
fn run_on_a_rust_layer_error_exits_one_with_the_note_before_cargo_output() {
    let output = varyk(&[
        "run",
        "crates/varyk/tests/fixtures/rust_layer_error/main.vr",
    ]);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(stdout_of(&output), "");

    let stderr = stderr_of(&output);
    assert!(
        stderr.starts_with(
            "error: the generated Rust did not compile; run with --emit-rust to inspect it\n"
        ),
        "stderr: {stderr}"
    );
    assert!(stderr.contains("mismatched types"), "stderr: {stderr}");
}

#[test]
fn run_on_a_constant_division_by_zero_builds_and_panics_at_run_time() {
    let output = varyk(&["run", "crates/varyk/tests/fixtures/divide_by_zero/main.vr"]);
    assert!(!output.status.success(), "status: {:?}", output.status);
    let stderr = stderr_of(&output);
    assert!(
        !stderr.contains("the generated Rust did not compile"),
        "stderr: {stderr}"
    );
    assert!(stderr.contains("divide by zero"), "stderr: {stderr}");
}
