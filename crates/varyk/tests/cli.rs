//! Integration tests for the `varyk` CLI: `check` through the
//! parser, exit codes, and the human/JSON output streams, exercised by
//! spawning the real binary.

mod common;

use common::varyk;

#[test]
fn check_on_a_valid_file_exits_zero_with_no_output() {
    let output = varyk(&["check", "examples/hello.vr"]);

    assert!(output.status.success(), "status: {:?}", output.status);
    assert!(
        output.stdout.is_empty(),
        "expected no stdout, got: {:?}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(
        output.stderr.is_empty(),
        "expected no stderr, got: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn check_on_a_parse_error_exits_one_with_a_rendered_diagnostic_on_stderr() {
    let output = varyk(&["check", "crates/varyk/tests/fixtures/parse_error/main.vr"]);

    assert_eq!(output.status.code(), Some(1));
    assert!(
        output.stdout.is_empty(),
        "expected no stdout, got: {:?}",
        String::from_utf8_lossy(&output.stdout)
    );

    let stderr = String::from_utf8(output.stderr).expect("stderr should be valid utf-8");
    insta::assert_snapshot!(stderr);
}

#[test]
fn version_flag_prints_the_crate_version() {
    let out = common::varyk(&["--version"]);
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert_eq!(
        stdout.trim(),
        format!("varyk {}", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn check_on_a_missing_file_exits_one_with_a_plain_stderr_message() {
    let output = varyk(&["check", "does/not/exist.vr"]);

    assert_eq!(output.status.code(), Some(1));
    assert!(
        output.stdout.is_empty(),
        "expected no stdout, got: {:?}",
        String::from_utf8_lossy(&output.stdout)
    );

    let stderr = String::from_utf8(output.stderr).expect("stderr should be valid utf-8");
    assert!(
        stderr.starts_with("error: cannot read `does/not/exist.vr`:"),
        "expected a plain unreadable-file message, got: {stderr:?}"
    );
}

#[test]
fn check_on_a_non_vr_file_exits_one_with_a_plain_stderr_message() {
    let output = varyk(&["check", "README.md"]);

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).expect("stderr should be valid utf-8");
    assert_eq!(stderr, "error: expected a `.vr` file, got `README.md`\n");
}

#[test]
fn check_with_json_message_format_emits_one_json_line_with_the_v0002_code() {
    let output = varyk(&[
        "check",
        "crates/varyk/tests/fixtures/parse_error/main.vr",
        "--message-format=json",
    ]);

    assert_eq!(output.status.code(), Some(1));
    assert!(
        output.stderr.is_empty(),
        "expected no stderr, got: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).expect("stdout should be valid utf-8");
    let mut lines = stdout.lines();
    let line = lines.next().expect("expected one JSON line on stdout");
    assert!(
        lines.next().is_none(),
        "expected exactly one JSON line, got: {stdout:?}"
    );

    let value: serde_json::Value = serde_json::from_str(line).expect("line should be valid JSON");
    assert_eq!(value["code"], "V0002");
    // `fn main() { let x = ; }`: the error is at the `;`, column 21.
    assert_eq!(value["line"], 1);
    assert_eq!(value["column"], 21);
    let file = value["file"].as_str().expect("file is a string");
    assert!(file.ends_with("parse_error/main.vr"), "file: {file:?}");
}

#[test]
fn check_passes_on_every_example_entry_file() {
    for entry in [
        "examples/hello.vr",
        "examples/functions.vr",
        "examples/structs.vr",
        "examples/borrowing.vr",
        "examples/modules/main.vr",
        "examples/interop/main.vr",
    ] {
        let output = varyk(&["check", entry]);
        assert!(
            output.status.success(),
            "{entry}: status {:?}, stderr: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
