//! End-to-end tests for `varyk build` and `varyk run`: every
//! example builds through cargo and prints its expected output (milestone-1
//! spec section 5, milestone-2 spec section 4, milestone-3 spec section 7).
//! These invoke cargo, so they are slower than the rest.

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use common::{empty_dir, example_dir, std_config, varyk, varyk_run_with, varyk_with_env};
use varyk::backend::{Backend, CrateInfo, RustBackend, StdDependency};
use varyk_syntax::{FileId, SourceFile};

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
    assert_runs(
        "examples/interop/main.vr",
        "Hello from Rust, Varyk!\nHello\n",
    );
}

/// An imported Rust struct (M3 spec 4.1, 4.2) over `std` only: a private
/// `Vec<String>` inside, built with `new`, used through `&self` and
/// `&mut self` methods and a `pub` field.
#[test]
fn run_interop_imported_struct() {
    assert_runs(
        "crates/varyk/tests/fixtures/interop/matcher/main.vr",
        "2 true\n2 words\n",
    );
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

// --- The milestone-4 examples (M4 spec 4) -----------------------------------

#[test]
fn run_iterators() {
    assert_runs(
        "examples/iterators.vr",
        "55\n3\ntrue\n2 4 6\napple, banana, cherry\n3\n",
    );
}

#[test]
fn run_words() {
    assert_runs(
        "examples/words.vr",
        "and 1\ncat 2\ndog 1\nran 1\nsaw 1\nthe 3\n6 distinct words\n",
    );
}

#[test]
fn run_patterns() {
    assert_runs(
        "examples/patterns.vr",
        "key a\nclick at the origin\nclick at 3, 4\nquit\nA B lower\ntrue false\n\
         first click at x = 0\n3\n2\n1\nquit\n",
    );
}

#[test]
fn run_getters() {
    assert_runs("examples/getters.vr", "Alice\nBob\n[hello]\nAlice\n[]\n");
}

#[test]
fn run_readings() {
    assert_runs(
        "examples/readings.vr",
        "25 -> 77\nnot a number: abc\n1\n10\n7\ntrue\ntrue\n0\n",
    );
}

#[test]
fn run_text() {
    assert_runs(
        "examples/text.vr",
        "ERROR_disk_full, WARN_low_memory\n2 of 3 lines kept\ntrue\ntrue\nfalse\n\
         second: DEBUG tick\nERROR DISK FULL\nfound WARN low memory\ntrue\n600\ntrue\n\
         42\nbad code: x!\n8080\nhello world\n",
    );
}

/// `json::parse` and `json::stringify` (M5a spec 2.4): `#[rename]` on a
/// field and a variant, a missing `Option` read as `None`, a `#[default]`
/// used, a `#[skip]` field neither read nor written, and a number out of
/// range for its type an `Error` printed with `{}`, not a panic.
#[test]
fn run_json() {
    assert_runs(
        "examples/json.vr",
        "7 ann member\n2 tags, age 18, nickname: false\n\
         {\"id\":7,\"userName\":\"ann\",\"role\":\"member\",\"nickname\":null,\"tags\":[\"a\",\"b\"],\"age\":18}\n\
         error: invalid value: integer `300`, expected u8 at line 1 column 67\n",
    );
}

/// `env::parse` (M5a spec 2.5) run in an empty directory, with the
/// variables the program reads set or removed, and `LOG` and `LOG_FORMAT`
/// removed, so a tester's own environment cannot change the output. The
/// field left to its `#[default]` is `timeout_secs`.
fn run_config_with(set: &[(&str, &str)], remove: &[&str]) -> Output {
    let program = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/config.vr");
    let program = program.to_string_lossy().into_owned();
    let mut removed = vec!["LOG", "LOG_FORMAT"];
    removed.extend(remove);
    varyk_run_with(&["run", &program], &empty_dir("config"), set, &removed)
}

#[test]
fn run_config() {
    let output = run_config_with(
        &[
            ("PORT", "8080"),
            ("DB_URL", "postgres://h/db?sslmode=require"),
            ("MODE", "live"),
        ],
        &["TOKEN", "TIMEOUT_SECS"],
    );
    assert!(output.status.success(), "{}", stderr_of(&output));
    assert_eq!(
        stdout_of(&output),
        "port 8080\ndatabase postgres://h/db?sslmode=require\nmode live\n\
         token set: false\ntimeout 30s\n"
    );
    assert_eq!(stderr_of(&output), "");
}

/// A variable that is not set, and a value that does not read, are an
/// `Error` naming the variable, printed with `{}`.
#[test]
fn run_config_reports_what_it_cannot_read() {
    let output = run_config_with(
        &[("DB_URL", "x"), ("MODE", "live")],
        &["PORT", "TOKEN", "TIMEOUT_SECS"],
    );
    assert!(output.status.success(), "{}", stderr_of(&output));
    assert_eq!(stdout_of(&output), "error: `PORT` is not set\n");
    let output = run_config_with(
        &[("PORT", "abc"), ("DB_URL", "x"), ("MODE", "live")],
        &["TOKEN", "TIMEOUT_SECS"],
    );
    assert!(output.status.success(), "{}", stderr_of(&output));
    assert_eq!(stdout_of(&output), "error: `PORT` is not a number: `abc`\n");
}

/// The four `log` calls (M5a spec 2.6) with `LOG=debug` and `LOG_FORMAT`
/// removed: every line on stderr, the time column dropped, and nothing on
/// stdout.
#[test]
fn run_logging() {
    let program = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/logging.vr");
    let program = program.to_string_lossy().into_owned();
    let output = varyk_run_with(
        &["run", &program],
        &empty_dir("logging"),
        &[("LOG", "debug")],
        &["LOG_FORMAT"],
    );
    assert!(output.status.success(), "{}", stderr_of(&output));
    assert_eq!(stdout_of(&output), "");
    let lines: Vec<String> = stderr_of(&output)
        .lines()
        .map(|line| {
            line.split_once(' ')
                .map_or("", |(_, rest)| rest)
                .to_string()
        })
        .collect();
    assert_eq!(
        lines,
        [
            "DEBUG starting with 3 workers",
            "INFO listening on port 8080",
            "WARN queue is 90 percent full",
            "ERROR request failed: connection refused",
        ]
    );
}

/// A `log` call reads its arguments once whatever the level, as
/// `println!` does (M5a spec 2.6): an argument that changes state runs
/// with `LOG` unset (only `info` and above shown) and with `LOG=debug`.
#[test]
fn log_arguments_run_whatever_the_level() {
    let dir = empty_dir("log-arguments");
    fs::write(
        dir.join("main.vr"),
        "struct Counter { n: i32 }\n\
         impl Counter { fn bump(mut self) -> i32 { self.n = self.n + 1; self.n } }\n\
         fn main() {\n    let mut c = Counter { n: 0 };\n    \
         log::debug(\"bumped to {}\", c.bump());\n    println!(\"n = {}\", c.n);\n}\n",
    )
    .expect("write main.vr");
    for (set, logged) in [
        (&[][..], ""),
        (&[("LOG", "debug")][..], "DEBUG bumped to 1"),
    ] {
        let removed = if set.is_empty() {
            vec!["LOG", "LOG_FORMAT"]
        } else {
            vec!["LOG_FORMAT"]
        };
        let output = varyk_run_with(&["run", "main.vr"], &dir, set, &removed);
        assert!(output.status.success(), "{}", stderr_of(&output));
        assert_eq!(stdout_of(&output), "n = 1\n");
        let stderr = stderr_of(&output);
        let line = stderr
            .trim_end()
            .split_once(' ')
            .map_or("", |(_, rest)| rest);
        assert_eq!(line, logged);
    }
}

/// `users` (M5a spec 6) run from a copy, as its current directory, so its
/// committed `.env` is the only source of configuration: every variable it
/// reads, and `LOG_FORMAT`, is removed from the environment, and `LOG` is
/// set by the `.env` itself. Stdout is the program's; stderr is the log,
/// its time column dropped.
#[test]
fn run_users() {
    let dir = example_dir("users");
    assert!(dir.join(".env").is_file(), "the copy keeps the .env");
    let output = varyk_run_with(
        &["run"],
        &dir,
        &[],
        &["PORT", "DB_URL", "LOG", "LOG_FORMAT"],
    );
    assert!(output.status.success(), "{}", stderr_of(&output));
    assert_eq!(
        stdout_of(&output),
        "added, 1 in the store\nadded, 2 in the store\nrejected: a user needs a name\n\
         rejected: unknown variant `Root`, expected `Admin` or `member` at line 1 column 38\n\
         [{\"id\":1,\"name\":\"ann\",\"role\":\"member\",\"email\":null,\"age\":18},\
         {\"id\":2,\"name\":\"bo\",\"role\":\"Admin\",\"email\":\"bo@example.com\",\"age\":41}]\n"
    );
    let lines: Vec<String> = stderr_of(&output)
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            line.split_once(' ')
                .map_or("", |(_, rest)| rest)
                .to_string()
        })
        .collect();
    assert_eq!(
        lines,
        [
            "INFO serving on port 8080 with postgres://localhost/users?sslmode=disable",
            "INFO now 1 users",
            "INFO now 2 users",
            "WARN rejected a user: a user needs a name",
            "WARN rejected a user: unknown variant `Root`, expected `Admin` or `member` at line 1 column 38",
        ]
    );
    assert!(!package("users").join("target").exists());
}

/// The table rows and `HashMap` (M4 spec 2.7) run as written: a `HashMap`
/// of counts, `parse` in each expected-type position, and `contains` on a
/// `Vec<string>` whose argument is a local named `e`, the name the
/// generated closure uses, printing `false` (a shadowed `e` would compile
/// and print `true`).
#[test]
fn run_table_rows() {
    assert_runs(
        "crates/varyk/tests/fixtures/codegen/tables/main.vr",
        "2 true\ntrue\n7\ntrue\nfalse\nfalse\n9 true false\nHELLO, YOU true\na+b\nfalse true\n",
    );
}

/// A single file whose only mention of `varyk-std` is `Error` in a
/// signature builds: its manifest gets the dependency (M5a spec 1, 5.2).
#[test]
fn run_a_single_file_naming_error_only_in_a_signature() {
    assert_runs(
        "crates/varyk/tests/fixtures/codegen/error_signature/main.vr",
        "true\n",
    );
}

/// `Error::new`, `message`, `{}` and `==` on an `Error`, and `?` on
/// `parse`, whose error names the text (M5a spec 2.3, 2.8).
#[test]
fn run_error_calls() {
    assert_runs(
        "crates/varyk/tests/fixtures/codegen/error_calls/main.vr",
        "literal stored\ntrue\n42\n`abc` is not a number\n",
    );
}

#[test]
fn run_with_trailing_arguments_still_prints_the_greeting() {
    let output = varyk(&["run", "examples/hello.vr", "--", "a", "b"]);
    assert!(output.status.success(), "stderr: {}", stderr_of(&output));
    assert_eq!(stdout_of(&output), "Hello, world!\n");
}

/// The Rust sample in `README.md`, the fenced block after
/// `<!-- emit-rust: examples/borrowing.vr -->`, is exactly the backend's
/// `src/main.rs` for that example, before any rustfmt: `--emit-rust`
/// formats it only when rustfmt is installed, and the minimal 1.85
/// toolchain has none.
#[test]
fn the_readme_rust_sample_is_the_current_backend_output() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let readme = fs::read_to_string(root.join("README.md")).expect("read README.md");
    let marker = "<!-- emit-rust: examples/borrowing.vr -->\n```rust\n";
    let start = readme
        .find(marker)
        .expect("the README marks its Rust sample")
        + marker.len();
    let sample = &readme[start..start + readme[start..].find("```").expect("a closing fence")];

    let path = root.join("examples/borrowing.vr");
    let text = fs::read_to_string(&path).expect("read examples/borrowing.vr");
    let mut sources = Vec::new();
    let entry = SourceFile::new(FileId(0), path, text);
    let program = varyk::check_file(entry, varyk::package::Kind::Binary, None, &mut sources)
        .unwrap_or_else(|diagnostics| panic!("borrowing should check: {diagnostics:#?}"));
    let std = StdDependency::for_program(program.uses_std);
    let generated =
        RustBackend.generate(&program, &CrateInfo::single_file("borrowing".into(), std));
    let main_rs = generated
        .files
        .iter()
        .find(|file| file.path == "src/main.rs")
        .expect("a src/main.rs");
    assert_eq!(sample, main_rs.text, "README.md sample differs");
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

/// The fixture's type error is in `broken.rs`, a copied `.rs` module: M3
/// spec 5 says this is the user's own Rust, so it is shown at their file
/// and line, verbatim, and never behind M1's "generated Rust did not
/// compile" note (which is only for a rustc message this cannot map back
/// to source at all).
#[test]
fn run_on_a_rust_layer_error_shows_it_at_the_users_file_and_line() {
    let output = varyk(&[
        "run",
        "crates/varyk/tests/fixtures/rust_layer_error/main.vr",
    ]);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(stdout_of(&output), "");

    let stderr = stderr_of(&output);
    assert!(
        stderr.contains("crates/varyk/tests/fixtures/rust_layer_error/broken.rs:2"),
        "stderr: {stderr}"
    );
    assert!(stderr.contains("mismatched types"), "stderr: {stderr}");
    assert!(
        !stderr.contains("the generated Rust did not compile"),
        "stderr: {stderr}"
    );
}

/// Under `--message-format=json`, the same `.rs` error is one JSON
/// object on stdout, at the user's own file.
#[test]
fn a_rust_layer_error_in_json_mode_is_an_object_at_the_users_file() {
    let output = varyk(&[
        "--message-format=json",
        "build",
        "crates/varyk/tests/fixtures/rust_layer_error/main.vr",
    ]);
    assert_eq!(output.status.code(), Some(1));

    let stdout = stdout_of(&output);
    let first = stdout.lines().next().expect("one object per line");
    let value: serde_json::Value = serde_json::from_str(first).expect("a JSON object");
    assert_eq!(
        value["file"],
        "crates/varyk/tests/fixtures/rust_layer_error/broken.rs"
    );
    assert_eq!(value["line"], 2);
    assert_eq!(value["level"], "error");
    assert_eq!(value["rustc_code"], "E0308");
    assert_eq!(value["message"], "mismatched types");
    assert!(value["notes"].is_array(), "{value}");
    for line in stdout.lines() {
        serde_json::from_str::<serde_json::Value>(line).expect("every line is JSON");
    }
    assert!(
        !stderr_of(&output).contains("mismatched types"),
        "the error is not also printed as text"
    );
}

/// Under `run`, stdout is the program's own: Varyk's JSON objects (here a
/// warning from a `.rs` module) go to stderr instead, so the program's
/// output stays exactly what it printed.
#[test]
fn run_in_json_mode_keeps_stdout_for_the_program() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("json_run")
        .join(std::process::id().to_string());
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create the program directory");
    fs::write(
        dir.join("main.vr"),
        "mod noisy;\n\nfn main() {\n    println!(\"{}\", noisy::one());\n}\n",
    )
    .expect("write main.vr");
    fs::write(
        dir.join("noisy.rs"),
        "pub fn one() -> i32 {\n    let unused = 2;\n    1\n}\n",
    )
    .expect("write noisy.rs");
    let entry = dir.join("main.vr");
    let output = varyk(&[
        "--message-format=json",
        "run",
        entry.to_str().expect("a UTF-8 path"),
    ]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr_of(&output));
    assert_eq!(stdout_of(&output), "1\n");
    let stderr = stderr_of(&output);
    let warning = stderr
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .find(|value| value["level"] == "warning")
        .unwrap_or_else(|| panic!("a JSON warning on stderr: {stderr}"));
    assert!(
        warning["file"]
            .as_str()
            .is_some_and(|f| f.ends_with("noisy.rs")),
        "{warning}"
    );

    // A failed build under `run`: nothing on stdout either.
    let output = varyk(&[
        "--message-format=json",
        "run",
        "crates/varyk/tests/fixtures/rust_layer_error/main.vr",
    ]);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(stdout_of(&output), "");
    let stderr = stderr_of(&output);
    let first = stderr.lines().next().expect("one object per line");
    let value: serde_json::Value = serde_json::from_str(first).expect("a JSON object");
    assert_eq!(value["rustc_code"], "E0308");
}

/// `use_it.rs` calls `defs::add` with one argument too many: the
/// diagnostic's primary span is in `use_it.rs`, but rustc's "function
/// defined here" note points at a *different* copied module, `defs.rs`
/// (fix round 1: this used to leak `src/defs.rs`, the internal tree
/// path, since only the primary span's file was rewritten).
#[test]
fn run_on_a_cross_module_rust_layer_error_rewrites_every_copied_path() {
    let output = varyk(&[
        "run",
        "crates/varyk/tests/fixtures/rust_layer_cross_module/main.vr",
    ]);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(stdout_of(&output), "");

    let stderr = stderr_of(&output);
    assert!(
        stderr.contains("crates/varyk/tests/fixtures/rust_layer_cross_module/use_it.rs"),
        "stderr: {stderr}"
    );
    assert!(
        stderr.contains("crates/varyk/tests/fixtures/rust_layer_cross_module/defs.rs"),
        "stderr: {stderr}"
    );
    assert!(!stderr.contains("src/defs.rs"), "stderr: {stderr}");
    assert!(!stderr.contains("src/use_it.rs"), "stderr: {stderr}");
}

#[test]
fn a_rust_module_warning_builds_and_the_generated_rust_has_no_inner_attribute() {
    let output = varyk(&[
        "build",
        "--emit-rust",
        "crates/varyk/tests/fixtures/rust_warning/main.vr",
    ]);
    assert!(output.status.success(), "stderr: {}", stderr_of(&output));

    let stdout = stdout_of(&output);
    // Item-level `#[allow(..)]` attributes replace M1's crate-wide
    // `#![allow(..)]` (spec 2.3); the root file has no inner attribute,
    // so the same tree works as a crate root and inside `include!`.
    assert!(
        !stdout.contains("#!["),
        "an inner attribute remains: {stdout}"
    );
    assert!(
        stdout.contains("#[allow("),
        "no item-level attribute: {stdout}"
    );

    // The warning in the copied `.rs` module passes through at the
    // user's path and line (spec 5); it never names a generated-file path.
    let stderr = stderr_of(&output);
    assert!(
        stderr.contains("crates/varyk/tests/fixtures/rust_warning/unused.rs:2:9"),
        "stderr: {stderr}"
    );
    assert!(
        !stderr.contains("target/varyk"),
        "stderr names a generated path: {stderr}"
    );
}

/// A `.rs` module nested under a `.vr` module is not covered by any
/// generated lint attribute: no `mod` line carries one (spec 2.3, 5), so
/// its warning shows at the user's path and a deny-level lint in it
/// fails the build instead of panicking at run time.
#[test]
fn a_rust_module_under_a_varyk_module_keeps_its_warnings_and_deny_lints() {
    let output = varyk(&[
        "build",
        "crates/varyk/tests/fixtures/nested_rust_warning/main.vr",
    ]);
    let stderr = stderr_of(&output);
    assert!(output.status.success(), "stderr: {stderr}");
    assert!(
        stderr.contains("unused variable: `unused`")
            && stderr.contains("crates/varyk/tests/fixtures/nested_rust_warning/outer/ext.rs:2:9"),
        "stderr: {stderr}"
    );
    assert!(
        !stderr.contains("target/varyk"),
        "stderr names a generated path: {stderr}"
    );

    let output = varyk(&[
        "build",
        "crates/varyk/tests/fixtures/nested_rust_overflow/main.vr",
    ]);
    let stderr = stderr_of(&output);
    assert!(!output.status.success(), "stderr: {stderr}");
    assert!(
        stderr.contains("this arithmetic operation will overflow")
            && stderr
                .contains("crates/varyk/tests/fixtures/nested_rust_overflow/outer/ext.rs:2:18"),
        "stderr: {stderr}"
    );

    // The one lint about a `mod` line itself, a module name not in snake
    // case, is allowed on that line, so generated code stays silent.
    let output = varyk(&[
        "build",
        "crates/varyk/tests/fixtures/upper_module_name/main.vr",
    ]);
    let stderr = stderr_of(&output);
    assert!(output.status.success(), "stderr: {stderr}");
    assert!(!stderr.contains("warning"), "stderr: {stderr}");
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

// --- The milestone-3 example packages (M3 spec 7) ---------------------------
//
// Each lives under `examples/packages/`, outside the compiler's workspace.
// The tests build a copy of each ([`example_dir`]), never the source tree.

/// `examples/packages/<name>`, absolute; only read, never built.
fn package(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/packages")
        .join(name)
}

/// Runs `cargo <args>` in `dir` with `VARYK` set to the `varyk` under
/// test, and asserts it succeeds.
fn cargo_in(dir: &Path, args: &[&str]) -> Output {
    let output = Command::new(env!("CARGO"))
        .args(args)
        .args(["--config", &std_config()])
        .current_dir(dir)
        .env("VARYK", env!("CARGO_BIN_EXE_varyk"))
        .output()
        .expect("failed to spawn cargo");
    assert!(
        output.status.success(),
        "cargo {args:?} in {dir:?}: {:?}\nstderr: {}",
        output.status,
        stderr_of(&output)
    );
    output
}

/// Each test process, and each call, builds its own copy of an example,
/// so two `cargo test` runs in one checkout, or two tests of one run,
/// never clear each other's files.
#[test]
fn an_example_copy_is_private_to_this_process() {
    let dir = example_dir("units");
    let pid = std::process::id().to_string();
    assert_eq!(
        dir.parent()
            .and_then(Path::parent)
            .and_then(Path::file_name),
        Some(pid.as_ref()),
        "{dir:?}"
    );
    assert!(dir.join("src/lib.vr").is_file(), "{dir:?}");
    // Every call is a copy of its own: another test's copy of `units` is
    // never cleared by this one.
    let other = example_dir("units");
    assert_ne!(dir, other);
    assert!(dir.join("src/lib.vr").is_file(), "{dir:?}");
}

const GREETING_OUTPUT: &str = "== Greetings ==\nHello, Ada!\nHello, Grace!\n== 2 greeted ==\n";

/// `greeting` is `varyk init greeting` committed as is (spec 7.1), plus
/// its own modules; `src/main.vr` is the one `init` file it replaces.
#[test]
fn greeting_keeps_the_files_varyk_init_writes() {
    let dir = package("greeting");
    for (path, content) in varyk::driver::init::files("greeting", false) {
        if path == Path::new("src/main.vr") {
            continue;
        }
        let committed = fs::read_to_string(dir.join(&path)).expect("an init file");
        assert_eq!(committed, content, "{path:?} differs from `varyk init`");
    }
}

/// `greeting` builds two ways with the same output (spec 7.1): `varyk run`
/// through the hidden crate, and plain `cargo run` through `build.rs` and
/// the `include!` stub.
#[test]
fn greeting_runs_the_same_through_varyk_and_through_cargo() {
    let dir = example_dir("greeting");
    let main = dir.join("src/main.vr");
    assert_runs(main.to_str().expect("utf-8 path"), GREETING_OUTPUT);

    let output = cargo_in(&dir, &["run", "--quiet"]);
    assert_eq!(stdout_of(&output), GREETING_OUTPUT);
}

/// `matcher` (spec 7.2) wraps `regex-lite` in a `.rs` facade, fetched
/// from crates.io as `varyk-std`'s own dependencies are for every program
/// that uses it (M5a spec 5.3). The deliberate unused variable in
/// `text.rs` warns at the user's file, never at a generated path.
#[test]
fn matcher_runs_through_its_facade_and_shows_the_rust_warning() {
    let dir = example_dir("matcher");
    let main = dir.join("src/main.vr");
    let output = varyk(&["run", main.to_str().expect("utf-8 path")]);
    assert!(
        output.status.success(),
        "status: {:?}, stderr: {}",
        output.status,
        stderr_of(&output)
    );
    assert_eq!(
        stdout_of(&output),
        "apple: word, 0 digit runs\n\
         42: the number 42, 1 digit runs\n\
         route 66 or 101: word, 2 digit runs\n\
         2 of 3 contain digits\n\
         true\n"
    );

    let stderr = stderr_of(&output).replace('\\', "/");
    assert!(
        stderr.contains("unused variable: `trimmed`"),
        "stderr: {stderr}"
    );
    assert!(stderr.contains("/matcher/src/text.rs:"), "stderr: {stderr}");
    assert!(!stderr.contains("target/varyk"), "stderr: {stderr}");
}

/// `units` (spec 7.3): the publish assembly, through the library API so
/// nothing reaches the registry, then `cargo package` on it, then
/// `consumer/`, a plain Rust binary with a `path` dependency on the
/// assembled crate.
#[test]
fn units_assembles_packages_and_serves_a_cargo_only_consumer() {
    let dir = example_dir("units");
    let mut sources = Vec::new();
    let package = varyk::package::load(&dir.join("Cargo.toml"), &mut sources).expect("loads");
    let text = fs::read_to_string(&package.entry).expect("read src/lib.vr");
    let entry = SourceFile::new(FileId(sources.len() as u32), package.entry.clone(), text);
    let program = varyk::check_file(
        entry,
        package.kind,
        Some(varyk::Dependencies {
            crates: &[],
            dev: &[],
        }),
        &mut sources,
    )
    .unwrap_or_else(|diagnostics| panic!("units should check: {diagnostics:#?}"));
    let info = CrateInfo {
        name: package.name.clone(),
        manifest: package.isolated_manifest(),
        std_dependency: None,
    };
    let generated = RustBackend.generate(&program, &info);
    let dest = varyk::driver::publish::assemble(&package, &generated).expect("assembles");
    assert_eq!(dest, dir.join("target/varyk/package/units"));

    cargo_in(&dest, &["package", "--no-verify", "--quiet"]);

    let output = cargo_in(&dir.join("consumer"), &["run", "--quiet"]);
    assert_eq!(stdout_of(&output), "7 meters\n");
}
