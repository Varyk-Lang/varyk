//! Integration tests for the `varyk` CLI: `check` through the
//! parser, exit codes, and the human/JSON output streams, exercised by
//! spawning the real binary.

mod common;

use std::fs;
use std::path::{Path, PathBuf};

use common::{varyk, varyk_with_env};

/// A fresh directory under this test binary's own `CARGO_TARGET_TMPDIR`,
/// for an `emit --out-dir` target that must not collide with another
/// test's.
fn out_dir(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("emit")
        .join(name);
    let _ = fs::remove_dir_all(&dir);
    dir
}

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
        "examples/enums.vr",
        "examples/methods.vr",
        "examples/collections.vr",
        "examples/errors.vr",
        "examples/strings.vr",
        "examples/todo/main.vr",
        "examples/packages/greeting/src/main.vr",
        "examples/packages/matcher/src/main.vr",
        "examples/packages/units/src/lib.vr",
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

#[test]
fn emit_out_dir_writes_the_generated_tree_with_no_inner_attribute() {
    let dir = out_dir("hello");

    let output = varyk(&[
        "emit",
        "examples/hello.vr",
        "--out-dir",
        dir.to_str().expect("utf-8 path"),
    ]);

    assert!(
        output.status.success(),
        "status: {:?}, stderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty());

    let main_rs = fs::read_to_string(dir.join("src/main.rs")).expect("emit wrote src/main.rs");
    assert!(
        !main_rs.trim_start().starts_with("#!["),
        "the root file must carry no inner attribute so it also works via `include!`: {main_rs}"
    );
    assert!(
        main_rs.contains("#[allow(warnings, arithmetic_overflow, unconditional_panic)]"),
        "{main_rs}"
    );
    assert!(
        !dir.join("Cargo.toml").exists(),
        "emit writes only the src/ tree of spec 2.3, not a manifest"
    );
}

#[test]
fn emit_on_a_varyk_error_reports_diagnostics_on_stderr_and_never_runs_cargo() {
    let dir = out_dir("parse_error");
    let empty_path = out_dir("parse_error_no_tools");
    fs::create_dir_all(&empty_path).unwrap();
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap();

    let output = std::process::Command::new(env!("CARGO_BIN_EXE_varyk"))
        .args([
            "emit",
            "crates/varyk/tests/fixtures/parse_error/main.vr",
            "--out-dir",
        ])
        .arg(&dir)
        .current_dir(&workspace_root)
        .env("PATH", &empty_path)
        .output()
        .expect("spawn varyk");

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).expect("stderr is utf-8");
    assert!(stderr.contains("V0002"), "{stderr}");
    assert!(
        !dir.exists(),
        "emit must not write a partial tree on failure"
    );
}

#[test]
fn build_without_cargo_on_the_path_says_cargo_is_missing() {
    let empty_path = out_dir("no_cargo_path");
    fs::create_dir_all(&empty_path).unwrap();

    let output = varyk_with_env(
        &["build", "examples/hello.vr"],
        &[("PATH", empty_path.to_str().expect("utf-8 path"))],
    );

    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8(output.stderr).expect("stderr is utf-8");
    assert_eq!(
        stderr,
        "error: cannot run `cargo`: not found; install Rust from https://rustup.rs\n"
    );
}

/// Two uses of one item without `pub`, or two reads of one private field,
/// are two V0105s but one edit: only the first carries the `pub ` fix-it,
/// so applying every fix-it never writes `pub pub`.
#[test]
fn one_missing_pub_gets_one_fix_it() {
    for fixture in ["two_uses_of_a_private_fn", "two_reads_of_a_private_field"] {
        let path = format!("crates/varyk/tests/fixtures/{fixture}/main.vr");
        let stderr = String::from_utf8(varyk(&["check", &path]).stderr).expect("utf-8");
        assert_eq!(stderr.matches("error[V0105]").count(), 2, "{stderr}");
        assert_eq!(stderr.matches("help: insert `pub `").count(), 1, "{stderr}");
    }
}

/// Every whole program in `docs/language.md` passes `check` (M3 spec 10's
/// reference coverage). A code block whose first line is `// <file>`,
/// `<file>` ending in `.vr` or `.rs`, is that file; a `// main.vr` block
/// closes a program made of it and every file block since the previous
/// one. Any other ```` ```varyk ```` block is a fragment and is not
/// checked, but one defining `main` must be marked.
#[test]
fn every_program_in_the_language_reference_checks() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let text = fs::read_to_string(root.join("docs/language.md")).expect("read language.md");
    let mut files: Vec<(String, String)> = Vec::new();
    let mut block: Option<(String, Vec<&str>)> = None;
    let mut programs = 0;
    for line in text.lines() {
        if let Some((info, lines)) = &mut block {
            if line != "```" {
                lines.push(line);
                continue;
            }
            let file = lines
                .first()
                .and_then(|first| first.strip_prefix("// "))
                .filter(|name| name.ends_with(".vr") || name.ends_with(".rs"));
            let body = lines.join("\n") + "\n";
            match file {
                Some(name) => files.push((name.to_string(), body)),
                None => assert!(
                    info != "varyk" || !body.contains("fn main()"),
                    "a program in language.md is not marked `// main.vr`:\n{body}"
                ),
            }
            if file == Some("main.vr") {
                programs += 1;
                let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
                    .join("language_md")
                    .join(programs.to_string());
                let _ = fs::remove_dir_all(&dir);
                for (name, body) in files.drain(..) {
                    let path = dir.join(name);
                    fs::create_dir_all(path.parent().expect("a directory")).expect("create");
                    fs::write(path, body).expect("write a program file");
                }
                let entry = dir.join("main.vr");
                let output = varyk(&["check", entry.to_str().expect("utf-8 path")]);
                assert!(
                    output.status.success(),
                    "program {programs} of language.md fails check:\n{}",
                    String::from_utf8_lossy(&output.stderr)
                );
            }
            block = None;
        } else if let Some(info) = line.strip_prefix("```") {
            block = Some((info.to_string(), Vec::new()));
        }
    }
    assert!(files.is_empty(), "file blocks after the last `// main.vr`");
    assert!(programs >= 7, "too few programs found: {programs}");
}
