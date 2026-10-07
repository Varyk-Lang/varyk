//! End-to-end tests for `varyk build` and `varyk run`: every
//! example builds through cargo and prints its expected output (milestone-1
//! spec section 5, milestone-2 spec section 4, milestone-3 spec section 7).
//! These invoke cargo, so they are slower than the rest.

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use common::{empty_dir, example_dir, std_config, varyk, varyk_in, varyk_run_with, varyk_with_env};
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

/// A `Shared` of an imported Rust struct (milestone 5b1 spec 2.6): its
/// `&self` method and `pub` field read through the handle.
#[test]
fn run_shared_imported_struct() {
    assert_runs(
        "crates/varyk/tests/fixtures/interop/shared_matcher/main.vr",
        "2 0\n",
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

/// `Time`, `Uuid`, and `Bytes` (milestone 5c spec 9): a time read with
/// `from_iso` and moved with `add_seconds`, a parsed id printed in
/// lowercase, a struct of the three through JSON and back, and the bytes
/// as text and base64. `Time::now()` and `Uuid::new()` are called and not
/// printed, so the output is fixed.
#[test]
fn run_records() {
    assert_runs(
        "examples/records.vr",
        "0192f0c4-7a3e-7b5c-9d1e-2f3a4b5c6d7e at 2026-10-07T10:00:00Z\n\
         expires at 2026-10-07T11:00:00Z, 3600 seconds later\n\
         {\"id\":\"0192f0c4-7a3e-7b5c-9d1e-2f3a4b5c6d7e\",\"at\":\"2026-10-07T10:00:00Z\",\
         \"data\":\"aGVsbG8sIHdvcmxk\"}\n\
         read back the same: true\n\
         hello, world\n\
         aGVsbG8sIHdvcmxk\n\
         a new upload has its own id: true\n\
         `42` is not a Uuid like 01890a5d-ac96-774b-bcce-b302099a8057\n\
         `2026-10-07 12:00` is not a time like 2026-10-07T12:00:00Z\n",
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

/// A `main` that returns a `Result`: `port` fails on `"x"`, and `?` in
/// `main` passes its error on.
const RESULT_MAIN: &str = "fn port(text: string) -> Result<i64, Error> {\n    \
     if text == \"x\" {\n        return Err(Error::new(\"no port in x\"));\n    }\n    \
     Ok(8080)\n}\n";

/// Writes `main` after [`RESULT_MAIN`] into a fresh directory and runs it
/// there with `LOG` and `LOG_FORMAT` removed.
fn run_result_main(label: &str, main: &str) -> Output {
    let dir = empty_dir(label);
    fs::write(dir.join("main.vr"), format!("{RESULT_MAIN}{main}")).expect("write main.vr");
    varyk_run_with(&["run", "main.vr"], &dir, &[], &["LOG", "LOG_FORMAT"])
}

/// An `Ok` from `main` exits 0, and `?` in `main` gives the value.
#[test]
fn a_main_that_returns_ok_exits_zero() {
    let output = run_result_main(
        "result-main-ok",
        "fn main() -> Result<i64, Error> {\n    let p = port(\"y\")?;\n    \
         println!(\"port {}\", p);\n    Ok(p)\n}\n",
    );
    assert!(output.status.success(), "{}", stderr_of(&output));
    assert_eq!(stdout_of(&output), "port 8080\n");
    assert_eq!(stderr_of(&output), "");
}

/// An `Err` from `main`, here through `?`, exits 1 with its message on
/// stderr, once, and nothing after the `?` runs.
#[test]
fn a_main_that_returns_err_exits_one_with_the_message() {
    let output = run_result_main(
        "result-main-err",
        "fn main() -> Result<i64, Error> {\n    let p = port(\"x\")?;\n    \
         println!(\"port {}\", p);\n    Ok(p)\n}\n",
    );
    assert_eq!(output.status.code(), Some(1), "{}", stderr_of(&output));
    assert_eq!(stdout_of(&output), "");
    assert_eq!(stderr_of(&output), "error: no port in x\n");
}

/// In a program that logs, an `Err` from an async `main` is one
/// error-level log line, and the exit code is 1.
#[test]
fn a_logging_main_logs_its_err_at_error_level() {
    let output = run_result_main(
        "result-main-logs",
        "async fn main() -> Result<bool, Error> {\n    log::info(\"starting\");\n    \
         let p = port(\"x\")?;\n    println!(\"port {}\", p);\n    Ok(true)\n}\n",
    );
    assert_eq!(output.status.code(), Some(1), "{}", stderr_of(&output));
    assert_eq!(stdout_of(&output), "");
    let lines: Vec<String> = stderr_of(&output)
        .lines()
        .map(|line| {
            line.split_once(' ')
                .map_or("", |(_, rest)| rest)
                .to_string()
        })
        .collect();
    assert_eq!(lines, ["INFO starting", "ERROR no port in x"]);
}

/// Started calls overlapping, `Task::all` over ten, an async method
/// (milestone 5b1 spec 6).
#[test]
fn run_tasks() {
    assert_runs(
        "examples/tasks.vr",
        "fetched user 7\nsent receipt to ann\nfast then slow: 7 and 2\n\
         total of ten squares: 385\ndoubled 21 is 42\nslow task done\n",
    );
}

/// `varyk test` runs the `#[test] async fn`s of `examples/tasks.vr`.
#[test]
fn test_tasks() {
    let output = varyk(&["test", "examples/tasks.vr"]);
    let out = stdout_of(&output);
    assert!(output.status.success(), "{out}\n{}", stderr_of(&output));
    assert!(out.contains("test doubles ... ok"), "{out}");
    assert!(out.contains("test counts ... ok"), "{out}");
}

/// `Task::all` with one failure and `?`, and `Task::all_settled` printing
/// every outcome (milestone 5b1 spec 2.5).
#[test]
fn run_fanout() {
    assert_runs(
        "examples/fanout.vr",
        "prices: 12 + 24 + 48\nfailed: no price for item 3\nitem 1: 12\nitem 2: 24\n\
         item 3 failed: no price for item 3\nitem 4: 48\n",
    );
}

/// One `Shared<Config>` handed to 10,000 started tasks.
#[test]
fn run_shared() {
    assert_runs("examples/shared.vr", "10000 tasks, total 50025000\n");
}

/// A single file with an async `main` that sleeps and prints runs, and
/// `varyk test` on it runs its `#[test] async fn` (milestone 5b1 spec
/// 2.2, 2.7): each on `varyk-std`'s runtime, which its manifest depends on.
#[test]
fn run_and_test_an_async_single_file() {
    let dir = empty_dir("async-file");
    fs::write(
        dir.join("main.vr"),
        "async fn twice(n: i64) -> i64 {\n    time::sleep(5).await;\n    n * 2\n}\n\n\
         async fn main() {\n    time::sleep(10).await;\n    println!(\"{}\", twice(21).await);\n}\n\n\
         #[test]\nasync fn doubles() {\n    assert_eq(twice(2).await, 4);\n}\n",
    )
    .expect("write main.vr");
    let output = varyk_run_with(&["run", "main.vr"], &dir, &[], &[]);
    assert!(output.status.success(), "{}", stderr_of(&output));
    assert_eq!(stdout_of(&output), "42\n");
    let output = varyk_run_with(&["test", "main.vr"], &dir, &[], &[]);
    let out = stdout_of(&output);
    assert!(output.status.success(), "{out}\n{}", stderr_of(&output));
    assert!(out.contains("test doubles ... ok"), "{out}");
}

/// A started task that panics, awaited (milestone 5b1 spec 2.4): the panic
/// continues in `main`, so the program stops with a failure and the
/// panic's message on stderr instead of hanging or printing on.
#[test]
fn a_panicking_task_that_is_awaited_fails_the_program() {
    let dir = empty_dir("task-panic");
    fs::write(
        dir.join("main.vr"),
        "async fn crash(n: usize) -> i64 {\n    time::sleep(5).await;\n    let v: Vec<i64> = vec![1, 2];\n    v[n]\n}\n\n\
         async fn main() {\n    let t = crash(7);\n    println!(\"started\");\n    println!(\"{}\", t.await);\n    println!(\"not reached\");\n}\n",
    )
    .expect("write main.vr");
    // On a thread, so that a hang fails the test rather than stalling it.
    let (send, receive) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = send.send(varyk_run_with(&["run", "main.vr"], &dir, &[], &[]));
    });
    let output = receive
        .recv_timeout(std::time::Duration::from_secs(300))
        .expect("the program finishes rather than hang");
    assert!(!output.status.success(), "{}", stdout_of(&output));
    assert_eq!(stdout_of(&output), "started\n");
    let stderr = stderr_of(&output);
    assert!(
        stderr.contains("index out of bounds: the len is 2 but the index is 7"),
        "{stderr}"
    );
}

/// Runs `main.vr` (`text`) in a fresh directory named `name`, on a thread
/// so that a hang fails the test rather than stalling it.
fn run_on_a_thread(name: &str, text: &str) -> std::process::Output {
    let dir = empty_dir(name);
    fs::write(dir.join("main.vr"), text).expect("write main.vr");
    let (send, receive) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = send.send(varyk_run_with(&["run", "main.vr"], &dir, &[], &[]));
    });
    receive
        .recv_timeout(std::time::Duration::from_secs(300))
        .expect("the program finishes rather than hang")
}

/// `Task::all` over 40 tasks giving a `Result`, the last of which fails
/// first (milestone 5b1 spec 2.5): it returns that `Err` at once, without
/// waiting for the tasks before it in the list, and cancels them, so none
/// of them prints, though `main` waits past the time they would finish.
#[test]
fn task_all_returns_the_first_err_to_arrive_and_cancels_the_rest() {
    let output = run_on_a_thread(
        "task-all-first-err",
        "async fn check(i: i32) -> Result<i32, string> {\n    if i == 39 {\n        \
         return Err(format!(\"task {} failed\", i));\n    }\n    time::sleep(1000).await;\n    \
         println!(\"task {} finished\", i);\n    Ok(i)\n}\n\n\
         async fn main() {\n    let mut ids: Vec<i32> = Vec::new();\n    for i in 0..40 {\n        \
         ids.push(i);\n    }\n    \
         match Task::all(ids.iter().map(|i| check(i)).collect()).await {\n        \
         Ok(values) => println!(\"all {}\", values.len()),\n        \
         Err(e) => println!(\"failed: {}\", e),\n    }\n    time::sleep(1500).await;\n    \
         println!(\"done\");\n}\n",
    );
    assert!(output.status.success(), "{}", stderr_of(&output));
    assert_eq!(stdout_of(&output), "failed: task 39 failed\ndone\n");
}

/// `Task::all` and `Task::all_settled` give the results in the order of
/// the list, whatever order the tasks finish in, and an empty list gives
/// an empty `Vec` (milestone 5b1 spec 2.5). An empty `vec![]` of tasks
/// has no type to take, so the empty list is a collected `map` over an
/// empty `Vec`.
#[test]
fn task_all_keeps_the_order_of_the_list_and_an_empty_one_gives_nothing() {
    let output = run_on_a_thread(
        "task-all-order",
        "async fn slow(n: i64) -> i64 {\n    time::sleep((50 - n * 10) as u64).await;\n    n\n}\n\n\
         async fn check(n: i64) -> Result<i64, string> {\n    time::sleep((50 - n * 10) as u64).await;\n    \
         if n == 2 {\n        return Err(\"two\");\n    }\n    Ok(n)\n}\n\n\
         async fn main() {\n    let ns = Task::all(vec![slow(1), slow(2), slow(3)]).await;\n    \
         println!(\"{} {} {}\", ns[0], ns[1], ns[2]);\n    \
         let rs = Task::all_settled(vec![check(1), check(2), check(3)]).await;\n    \
         for r in rs {\n        match r {\n            Ok(n) => println!(\"ok {}\", n),\n            \
         Err(e) => println!(\"err {}\", e),\n        }\n    }\n    \
         let none: Vec<i64> = vec![];\n    \
         let a = Task::all(none.iter().map(|n| slow(n)).collect()).await;\n    \
         let b = Task::all_settled(none.iter().map(|n| check(n)).collect()).await;\n    \
         match Task::all(none.iter().map(|n| check(n)).collect()).await {\n        \
         Ok(c) => println!(\"{} {} {}\", a.len(), b.len(), c.len()),\n        \
         Err(e) => println!(\"{}\", e),\n    }\n}\n",
    );
    assert!(output.status.success(), "{}", stderr_of(&output));
    assert_eq!(stdout_of(&output), "1 2 3\nok 1\nerr two\nok 3\n0 0 0\n");
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

/// `Bytes` passed, stored, moved, returned, and cloned builds and runs
/// (milestone 5c spec 2.3).
#[test]
fn run_bytes_ownership() {
    assert_runs(
        "crates/varyk/tests/fixtures/codegen/bytes_ownership/main.vr",
        "5\n5\naGVsbG8=\n5\n3\n2\n",
    );
}

/// Trailing values of each admitted type build and run (milestone 5b3
/// spec 2.2): `Time`, `Uuid`, and an `Option<Time>` by value, a `Bytes`
/// lent from a local, a parameter, and a borrowed return, an
/// `Option<Bytes>` looked through, and an `if` lent whole (milestone 5c
/// spec 2.4, 4).
#[test]
fn run_trailing_values() {
    assert_runs(
        "crates/varyk/tests/fixtures/codegen/trailing_values/main.vr",
        "17 54 42 41 user 7 Ada\nnick\n75 82 3 true\n36\n",
    );
}

/// Every call of `Time`, `Uuid`, and `Bytes` builds and gives what spec
/// 2.1 to 2.3 say (milestone 5c); the `Uuid` constructors and `now` are
/// called but not printed, since their values differ on every run.
#[test]
fn run_time_ids_bytes() {
    assert_runs(
        "crates/varyk/tests/fixtures/codegen/time_ids_bytes/main.vr",
        "2026-10-07T10:00:00.5Z\n\
         `2026-10-07 12:00` is not a time like 2026-10-07T12:00:00Z\n\
         1970-01-01T00:00:00Z\n\
         1970-01-01T00:00:01.5Z\n\
         `253402300800` seconds since 1970 is out of range for a time, which runs from the \
         year 0000 to the year 9999\n\
         true\n\
         2001-09-09T01:46:40Z 1000000000 1000000000000000\n\
         2001-09-09T01:46:00Z\n\
         2001-09-09T01:47:00Z\n\
         1000000000 1000000000\n\
         6 false aMOpbGxv\n\
         6 héllo\n\
         true 0\n\
         hi\n\
         the text is not standard base64 with padding\n\
         the bytes are not UTF-8 text: byte 0 starts no character\n",
    );
}

/// Comparing, printing, parsing, copying, and keys of `Time`, `Uuid`, and
/// `Bytes` run as spec 2.1 to 2.4 say (milestone 5c): times compared and
/// sorted, a `Uuid` read from uppercase text printed lowercase, a one-entry
/// `HashMap<Uuid, string>` (one, since `keys` has no fixed order), and
/// `Bytes` compared and cloned; and `varyk test` runs its `#[test]`, which
/// `varyk run` leaves out, with `assert_eq` on two `Time`s.
#[test]
fn run_and_test_records_compare() {
    let path = "crates/varyk/tests/fixtures/codegen/records_compare/main.vr";
    assert_runs(
        path,
        "true true false true\n\
         false true\n\
         1970-01-01T00:16:40Z 1970-01-01T00:25:00Z 1970-01-01T00:33:20Z\n\
         true\n\
         0192f0c4-7a3e-7b5c-9d1e-2f3a4b5c6d7e\n\
         first\n\
         true\n\
         true\n\
         true\n\
         2026-10-07T11:00:00.25Z\n\
         true false\n\
         true\n\
         true\n",
    );
    let output = varyk(&["test", path]);
    let out = stdout_of(&output);
    assert!(output.status.success(), "{out}\n{}", stderr_of(&output));
    assert!(out.contains("test times_compare ... ok"), "{out}");
}

/// `Time`, `Uuid`, and `Bytes` through JSON (milestone 5c spec 2.4): a
/// record written, printed, read back, and equal to the first; a number
/// for a `Time` and base64 without its padding for a `Bytes` are errors.
#[test]
fn run_records_json() {
    assert_runs(
        "crates/varyk/tests/fixtures/codegen/records_json/main.vr",
        "{\"at\":\"2026-10-07T10:00:00.5Z\",\"id\":\"0192f0c4-7a3e-7b5c-9d1e-2f3a4b5c6d7e\",\
         \"data\":\"aGVsbG8=\",\"seen\":\"2026-10-07T10:00:00.5Z\",\
         \"ids\":[\"0192f0c4-7a3e-7b5c-9d1e-2f3a4b5c6d7e\",\"6f1c2a3b-4d5e-4f60-8a7b-9c0d1e2f3a4b\"]}\n\
         true\n\
         error: invalid type: integer `1791374400`, expected a time like \
         2026-10-07T12:00:00Z at line 1 column 17\n\
         error: the text is not standard base64 with padding at line 1 column 14\n",
    );
}

/// `records_env` run in an empty directory, with `START` and `OWNER` set
/// or removed, so a tester's own environment or a `.env` cannot change the
/// output, following `run_config_with`.
fn run_records_env_with(set: &[(&str, &str)], remove: &[&str]) -> Output {
    let program = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/codegen/records_env/main.vr")
        .to_string_lossy()
        .into_owned();
    varyk_run_with(&["run", &program], &empty_dir("records-env"), set, remove)
}

/// `env::parse` reads a `Time` and an `Option<Uuid>` (milestone 5c spec
/// 2.4): both printed, the id `None` once its variable is removed, and a
/// time that does not read an error naming its variable.
#[test]
fn run_records_env() {
    let output = run_records_env_with(
        &[
            ("START", "2026-10-07T12:00:00+02:00"),
            ("OWNER", "0192F0C4-7A3E-7B5C-9D1E-2F3A4B5C6D7E"),
        ],
        &[],
    );
    assert!(output.status.success(), "{}", stderr_of(&output));
    assert_eq!(
        stdout_of(&output),
        "start 2026-10-07T10:00:00Z\nowner 0192f0c4-7a3e-7b5c-9d1e-2f3a4b5c6d7e\n"
    );
    assert_eq!(stderr_of(&output), "");
    let output = run_records_env_with(&[("START", "2026-10-07T12:00:00Z")], &["OWNER"]);
    assert!(output.status.success(), "{}", stderr_of(&output));
    assert_eq!(stdout_of(&output), "start 2026-10-07T12:00:00Z\nno owner\n");
    let output = run_records_env_with(&[("START", "2026-10-07 12:00")], &["OWNER"]);
    assert!(output.status.success(), "{}", stderr_of(&output));
    assert_eq!(
        stdout_of(&output),
        "error: `START`: `2026-10-07 12:00` is not a time like 2026-10-07T12:00:00Z\n"
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

/// A single file whose `.rs` module names `varyk_std::` only in
/// functions it never calls builds: rustc compiles the module whole, so
/// the program depends on `varyk-std` (milestone 5b3 spec 2.4).
#[test]
fn run_a_single_file_whose_rust_module_names_varyk_std_uncalled() {
    assert_runs(
        "crates/varyk/tests/fixtures/interop/std_uncalled/main.vr",
        "42\n",
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
    let program = varyk::check_file(entry, None, None, &mut sources)
        .unwrap_or_else(|diagnostics| panic!("borrowing should check: {diagnostics:#?}"))
        .program;
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
    let dir = empty_dir("json_run");
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

/// `trip` (M5b2 spec 6) uses `route` and `units`, and `route` uses
/// `units`: `varyk check` on a copy, with both packages copied beside it,
/// checks all three.
#[test]
fn trip_checks_with_route_and_units_beside_it() {
    let dir = example_dir("trip");
    for package in ["route", "units"] {
        let sibling = dir.parent().expect("a parent").join(package);
        assert!(sibling.join("src/lib.vr").is_file(), "{sibling:?}");
    }
    let output = varyk_in(&dir, &["check"]);
    assert!(
        output.status.success(),
        "status: {:?}, stderr: {}",
        output.status,
        stderr_of(&output)
    );
}

/// What `trip` prints on stdout (M5b2 spec 6).
const TRIP_OUTPUT: &str = "total 1700 meters\n\
                           longest Mill to Lake, 900 meters\n\
                           stop Lake, 12 minutes\n\
                           no stop\n\
                           about 21 minutes on foot\n";

/// Runs `varyk <args>` in `dir` with `LOG` and `LOG_FORMAT` removed, so
/// the default level and format apply whatever the tester's own are.
fn varyk_logged(dir: &Path, args: &[&str]) -> Output {
    varyk_run_with(args, dir, &[], &["LOG", "LOG_FORMAT"])
}

fn assert_ok(output: &Output) {
    assert!(
        output.status.success(),
        "status: {:?}\nstdout: {}\nstderr: {}",
        output.status,
        stdout_of(output),
        stderr_of(output)
    );
}

/// `text` with every absolute path the driver writes for a copy of an
/// example under `parent` shown as `[examples]`, and the workspace's
/// `varyk-std` as `[varyk-std]`.
fn redacted(text: &str, parent: &Path) -> String {
    let parent = parent.canonicalize().expect("the copy's parent");
    text.replace(&common::std_path().display().to_string(), "[varyk-std]")
        .replace(&parent.display().to_string(), "[examples]")
}

/// `trip` runs under `varyk run` (M5b2 spec 4.3): the driver writes a
/// crate for `route` and one for `units`, the program's `main` starts
/// logging because `route` logs, though `trip` never calls `log`, and
/// `route`'s warning reaches stderr.
#[test]
fn trip_runs_with_route_s_log_line() {
    let dir = example_dir("trip");
    let output = varyk_logged(&dir, &["run"]);
    assert_ok(&output);
    assert_eq!(stdout_of(&output), TRIP_OUTPUT);
    let stderr = stderr_of(&output);
    assert!(stderr.contains("cannot read a stop"), "stderr: {stderr}");

    let parent = dir.parent().expect("a parent");
    let read = |path: &str| {
        fs::read_to_string(dir.join(path)).unwrap_or_else(|err| panic!("{path}: {err}"))
    };
    let main = read("target/varyk/trip/src/main.rs");
    assert!(main.contains("::varyk_std::start();"), "{main}");
    insta::assert_snapshot!("trip_main_rs", main);
    insta::assert_snapshot!(
        "route_lib_rs",
        read("target/varyk/packages/route-0.1.0/src/lib.rs")
    );
    insta::assert_snapshot!(
        "trip_cargo_toml",
        redacted(&read("target/varyk/trip/Cargo.toml"), parent)
    );
    insta::assert_snapshot!(
        "route_cargo_toml",
        redacted(
            &read("target/varyk/packages/route-0.1.0/Cargo.toml"),
            parent
        )
    );
    insta::assert_snapshot!(
        "units_cargo_toml",
        redacted(
            &read("target/varyk/packages/units-0.1.0/Cargo.toml"),
            parent
        )
    );
}

/// The modification time of every file under `dir`, by path.
fn mtimes(dir: &Path) -> std::collections::BTreeMap<PathBuf, std::time::SystemTime> {
    let mut found = std::collections::BTreeMap::new();
    let mut next = vec![dir.to_path_buf()];
    while let Some(dir) = next.pop() {
        for entry in fs::read_dir(&dir).expect("read a directory").flatten() {
            let path = entry.path();
            if path.is_dir() {
                next.push(path);
            } else {
                let modified = entry.metadata().and_then(|meta| meta.modified());
                found.insert(path, modified.expect("a modification time"));
            }
        }
    }
    found
}

/// A second build with no change writes no file and compiles nothing:
/// every package crate and every library cargo made is as the first build
/// left it. A change to `units` then reaches `trip` through `route` too
/// (M5b2 spec 4.3).
#[test]
fn trip_rebuilds_a_changed_units_and_nothing_unchanged() {
    let dir = example_dir("trip");
    assert_ok(&varyk_logged(&dir, &["build"]));
    // The graph's own directory is `check`'s, rewritten as cargo reads it.
    let written = [
        "target/varyk/packages/route-0.1.0",
        "target/varyk/packages/units-0.1.0",
        "target/varyk/trip/src",
        "target/varyk/cache/debug/deps",
    ];
    let all = || -> Vec<_> { written.iter().map(|path| mtimes(&dir.join(path))).collect() };
    let before = all();
    assert_ok(&varyk_logged(&dir, &["build"]));
    assert!(
        before == all(),
        "a build with no change wrote or compiled something"
    );

    let length = dir.parent().expect("a parent").join("units/src/length.vr");
    let text = fs::read_to_string(&length).expect("read length.vr");
    let changed = text.replace("a.value + b.value", "a.value + b.value * 2");
    assert_ne!(text, changed);
    fs::write(&length, changed).expect("write length.vr");
    let output = varyk_logged(&dir, &["run"]);
    assert_ok(&output);
    // `route::total` doubles each leg, then `trip` adds nothing.
    assert!(
        stdout_of(&output).starts_with("total 3400 meters\n"),
        "stdout: {}",
        stdout_of(&output)
    );
}

/// `varyk build` right after `varyk add --path ../units`, before any code
/// names `units`, builds: the new dependency is a Varyk package crate.
#[test]
fn a_program_builds_right_after_adding_a_varyk_package() {
    let units = example_dir("units");
    let parent = units.parent().expect("a parent");
    assert_ok(&varyk_in(parent, &["init", "app"]));
    let app = parent.join("app");
    // `init` pins `varyk-std` to the compiler's version, which a release
    // pull request names before crates.io has it: use the local one.
    let std = std_config();
    assert_ok(&varyk_run_with(
        &[
            "add",
            "units",
            "--path",
            "../units",
            "--offline",
            "--config",
            &std,
        ],
        &app,
        &[("CARGO_TERM_COLOR", "never")],
        &[],
    ));
    assert_ok(&varyk_in(&app, &["build"]));
}

/// `varyk test` in `trip` runs `trip`'s own tests, which may call into
/// `route`, and never `route`'s (M5b2 spec 2.3).
#[test]
fn varyk_test_runs_the_program_s_tests_and_not_a_package_s() {
    let dir = example_dir("trip");
    let parent = dir.parent().expect("a parent");
    let append = |path: &Path, text: &str| {
        let mut source = fs::read_to_string(path).expect("read a source file");
        source.push_str(text);
        fs::write(path, source).expect("write a source file");
    };
    append(
        &dir.join("src/main.vr"),
        "\n#[test]\nfn totals_one_leg() {\n    \
         let legs = vec![route::leg(\"A\", \"B\", 3)];\n    \
         assert_eq(route::total(legs).value, 3);\n}\n",
    );
    append(
        &parent.join("route/src/lib.vr"),
        "\n#[test]\nfn never_runs_from_trip() {\n    assert(false);\n}\n",
    );
    let output = varyk_logged(&dir, &["test"]);
    assert_ok(&output);
    let stdout = stdout_of(&output);
    assert!(stdout.contains("test totals_one_leg ... ok"), "{stdout}");
    assert!(!stdout.contains("never_runs_from_trip"), "{stdout}");
}

/// A package named `route-planner`, listed under that key and named
/// `route_planner::` in Varyk, builds and runs: its crate's library target
/// is `route_planner`, since cargo refuses a `-` there (M5b2 Review Focus
/// 3).
#[test]
fn a_package_with_a_hyphen_in_its_name_builds_and_runs() {
    let dir = example_dir("trip");
    let parent = dir.parent().expect("a parent");
    let edit = |path: &Path, from: &str, to: &str| {
        let text = fs::read_to_string(path).expect("read a file");
        assert!(text.contains(from), "{path:?}: {text}");
        fs::write(path, text.replace(from, to)).expect("write a file");
    };
    edit(
        &parent.join("route/Cargo.toml"),
        "name = \"route\"",
        "name = \"route-planner\"",
    );
    edit(
        &dir.join("Cargo.toml"),
        "route = { path",
        "route-planner = { path",
    );
    edit(&dir.join("src/main.vr"), "route::", "route_planner::");
    let output = varyk_logged(&dir, &["run"]);
    assert_ok(&output);
    assert_eq!(stdout_of(&output), TRIP_OUTPUT);
    let lib = fs::read_to_string(dir.join("target/varyk/packages/route-planner-0.1.0/Cargo.toml"))
        .expect("the package crate's manifest");
    assert!(lib.contains("name = \"route_planner\""), "{lib}");
}

/// A program that lists `units` as `u = { package = "units" }` names its
/// types `::u::..`, `route::total`'s result included (M5b2 spec 5).
#[test]
fn a_renamed_dependency_names_its_types_by_the_key() {
    let dir = example_dir("trip");
    let edit = |path: &Path, from: &str, to: &str| {
        let text = fs::read_to_string(path).expect("read a file");
        assert!(text.contains(from), "{path:?}: {text}");
        fs::write(path, text.replace(from, to)).expect("write a file");
    };
    edit(
        &dir.join("Cargo.toml"),
        "units = { path = \"../units\" }",
        "u = { path = \"../units\", package = \"units\" }",
    );
    let main = dir.join("src/main.vr");
    edit(&main, "units::", "u::");
    // A `Vec` keeps the type written in the Rust (spec 2.3).
    edit(
        &main,
        "    let best",
        "    let totals = vec![route::total(legs)];\n    \
         println!(\"{} totals\", totals.len());\n    let best",
    );
    let output = varyk_logged(&dir, &["build", "--emit-rust"]);
    assert_ok(&output);
    let stdout = stdout_of(&output);
    assert!(
        stdout.contains("let totals: Vec<::u::length::Meters> = "),
        "{stdout}"
    );
    let output = varyk_logged(&dir, &["run"]);
    assert_ok(&output);
    assert!(
        stdout_of(&output).contains("total 1700 meters\n1 totals\n"),
        "{}",
        stdout_of(&output)
    );
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

/// `greeting` runs through `varyk run`, the one way a Varyk package is
/// built (M5b2 spec 1.2), with the output the example promises.
#[test]
fn greeting_runs_through_varyk() {
    let dir = example_dir("greeting");
    let main = dir.join("src/main.vr");
    assert_runs(main.to_str().expect("utf-8 path"), GREETING_OUTPUT);
}

/// `matcher` (spec 7.2) wraps `regex-lite` in a `.rs` facade, fetched
/// from crates.io as `varyk-std`'s own dependencies are for every program
/// that uses it (M5a spec 5.3). The deliberate unused variable in
/// `text.rs` warns at the user's file, never at a generated path.
#[test]
fn matcher_runs_through_its_facade_and_shows_the_rust_warning() {
    let dir = example_dir("matcher");
    let main = dir.join("src/main.vr");
    // A program whose only dependency is a Rust crate still checks, now
    // that `check` reads the package graph (M5b2 Review Focus 1).
    let checked = varyk(&["check", main.to_str().expect("utf-8 path")]);
    assert!(
        checked.status.success(),
        "status: {:?}, stderr: {}",
        checked.status,
        stderr_of(&checked)
    );
    assert!(
        dir.join("target/varyk/packages/graph/Cargo.toml").is_file(),
        "check read no graph"
    );
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
    let program = varyk::check_file(entry, Some(&package), None, &mut sources)
        .unwrap_or_else(|diagnostics| panic!("units should check: {diagnostics:#?}"))
        .program;
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

/// Pins the key list of M5b2 spec 4.4: the manifest `cargo package` writes
/// for the crate `varyk publish` assembles is one a dependency may have,
/// on the toolchain running the tests. No `--no-verify`, so cargo unpacks
/// the crate under `target/package/`; `units` has no dependencies, so
/// nothing needs the network.
#[test]
fn the_manifest_cargo_package_writes_is_one_a_dependency_may_have() {
    let dir = example_dir("units");
    let mut sources = Vec::new();
    let package = varyk::package::load(&dir.join("Cargo.toml"), &mut sources).expect("loads");
    let text = fs::read_to_string(&package.entry).expect("read src/lib.vr");
    let entry = SourceFile::new(FileId(sources.len() as u32), package.entry.clone(), text);
    let program = varyk::check_file(entry, Some(&package), None, &mut sources)
        .unwrap_or_else(|diagnostics| panic!("units should check: {diagnostics:#?}"))
        .program;
    let info = CrateInfo {
        name: package.name.clone(),
        manifest: package.generated_manifest(),
        std_dependency: None,
    };
    let generated = RustBackend.generate(&program, &info);
    let dest = varyk::driver::publish::assemble(&package, &generated).expect("assembles");

    cargo_in(&dest, &["package", "--offline", "--quiet"]);

    let unpacked = dest.join("target/package/units-0.1.0/Cargo.toml");
    let mut sources = Vec::new();
    if let Err(diagnostics) = varyk::package::load_dependency(&unpacked, &mut sources, false) {
        let text = fs::read_to_string(&unpacked).unwrap_or_default();
        panic!("cargo's manifest was refused: {diagnostics:#?}\n{text}");
    }
}

/// Assembles `units` the way `varyk publish --assemble-only` does and
/// returns the assembled crate's directory.
fn assembled_units() -> PathBuf {
    let dir = example_dir("units");
    let output = varyk_in(&dir, &["publish", "--assemble-only"]);
    assert!(output.status.success(), "{}", stderr_of(&output));
    dir.join(stdout_of(&output).trim_end())
}

/// A Varyk program with a `path` dependency on `crate_dir`, named `units`,
/// in a fresh directory; it prints a length from the dependency.
fn program_using_units(crate_dir: &Path) -> PathBuf {
    let dir = empty_dir("uses-assembled-units");
    fs::create_dir_all(dir.join("src")).expect("create src");
    fs::write(
        dir.join("Cargo.toml"),
        format!(
            "[package]\nname = \"uses_units\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n\
             [[bin]]\nname = \"uses_units\"\npath = \"src/main.vr\"\n\n\
             [dependencies]\nunits = {{ path = \"{}\" }}\n",
            crate_dir.display()
        ),
    )
    .expect("write Cargo.toml");
    fs::write(
        dir.join("src/main.vr"),
        "fn main() {\n    let a = units::length::Meters { value: 3 };\n    \
         let b = units::length::Meters { value: 4 };\n    \
         println!(\"{} meters\", units::length::add(a, b).value);\n}\n",
    )
    .expect("write main.vr");
    dir
}

/// A Varyk program depending on the assembled `units` by path builds and
/// runs under `varyk` (M5b2 spec 3).
#[test]
fn a_varyk_program_runs_against_the_assembled_units() {
    let program = program_using_units(&assembled_units());
    let output = varyk_in(&program, &["run"]);
    assert!(output.status.success(), "{}", stderr_of(&output));
    assert_eq!(stdout_of(&output), "7 meters\n");
}

/// Under `varyk` nothing an assembled crate ships runs (M5b2 spec 4.5):
/// changed generated Rust and a failing build script, which plain cargo
/// would run, have no effect.
#[test]
fn shipped_rust_and_build_scripts_have_no_effect_under_varyk() {
    let crate_dir = assembled_units();
    let marker = crate_dir.join("build-script-ran");
    let _ = fs::remove_file(&marker);
    let manifest = crate_dir.join("Cargo.toml");
    let text = fs::read_to_string(&manifest).expect("read the assembled manifest");
    assert!(text.contains("build = false"), "{text}");
    fs::write(&manifest, text.replace("build = false", "")).expect("write the manifest");
    fs::write(
        crate_dir.join("build.rs"),
        format!(
            "fn main() {{\n    std::fs::write(r\"{}\", \"ran\").unwrap();\n    panic!(\"build script ran\");\n}}\n",
            marker.display()
        ),
    )
    .expect("write build.rs");
    for file in ["src/lib.rs", "src/length.rs"] {
        let path = crate_dir.join(file);
        assert!(
            path.is_file(),
            "{} is not in the assembled crate",
            path.display()
        );
        fs::write(&path, "compile_error!(\"shipped Rust compiled\");\n").expect("tamper");
    }

    let program = program_using_units(&crate_dir);
    let output = varyk_in(&program, &["run"]);
    assert!(output.status.success(), "{}", stderr_of(&output));
    assert_eq!(stdout_of(&output), "7 meters\n");
    assert!(!marker.exists(), "the shipped build script ran");
}

/// The manifest `varyk publish --assemble-only` writes for `route`: the
/// generated root as the target, no build script, and `units` with its
/// version kept and its path made absolute (redacted here).
#[test]
fn route_publish_manifest() {
    let dir = example_dir("route");
    let output = varyk_in(&dir, &["publish", "--assemble-only"]);
    assert!(output.status.success(), "{}", stderr_of(&output));
    let dest = dir.join(stdout_of(&output).trim_end());
    let manifest = fs::read_to_string(dest.join("Cargo.toml")).expect("read the manifest");
    let units = dir.join("../units");
    let manifest = manifest.replace(&units.display().to_string(), "<UNITS>");
    assert!(manifest.contains("<UNITS>"), "{manifest}");
    // The release pull request bumps `route`'s `varyk-std` line with the
    // compiler, so the snapshot keeps no version of it.
    let std_line = format!("varyk-std = \"{}\"", env!("CARGO_PKG_VERSION"));
    assert!(manifest.contains(&std_line), "{manifest}");
    let manifest = manifest.replace(&std_line, "varyk-std = \"[version]\"");
    insta::assert_snapshot!("route_publish_cargo_toml", manifest);
}

// --- The milestone-5b3 fixture packages (M5b3 spec 8) -----------------------
//
// `store` is a Varyk library whose `.rs` facade has the shapes of
// `varyk-sql` with no database; `store_user` is a program that uses it.
// They live under `tests/fixtures/packages/`, and run here rather than in
// `tests/packages.rs` because their `serde_json` and `varyk-std` lines
// need the registry.

/// What `store_user` prints (its `src/main.vr` says the same).
const STORE_USER_OUTPUT: &str = "Ada 36 countess 9.5 true\n\
                                 2 users: Ada, Bo\n\
                                 Bo has no nick\n\
                                 nothing stored under user/9\n\
                                 committed true, then: the batch is already committed\n\
                                 Cy is in, Di is not\n\
                                 3 users, 3 counted\n";

/// `store_user` runs under `varyk run` and prints what it should: every
/// facade shape of 5b3 through a dependency package, by the names its
/// `pub use` lines give. `varyk test` in the `store` copied beside it runs
/// the library's own test, which pins `varyk test` on a library.
#[test]
fn store_user_runs_and_store_s_test_passes() {
    let dir = common::package_copy("tests/fixtures/packages", "store_user");
    let output = varyk_logged(&dir, &["run"]);
    assert_ok(&output);
    assert_eq!(stdout_of(&output), STORE_USER_OUTPUT);

    let read = |path: &str| {
        fs::read_to_string(dir.join(path)).unwrap_or_else(|err| panic!("{path}: {err}"))
    };
    let main = read("target/varyk/store_user/src/main.rs");
    for written in [
        "::store::kv::Store::one::<User>(db, \"user/1\", vec![::varyk_std::Value::from(1)])",
        "::store::kv::Store::all::<User>(&db, \"user/\", vec![])",
        "::store::kv::Store::first::<User>(&db, \"user/2\"",
        "::store::kv::Store::one::<i64>(&db, \"count\", vec![])",
        "::varyk_std::Value::from(name.as_str())",
        "::varyk_std::Value::from(nick.as_deref())",
        "::store::kv::Batch::commit(&mut batch).await",
    ] {
        assert!(main.contains(written), "{written} is not in:\n{main}");
    }
    let lib = read("target/varyk/packages/store-0.1.0/src/lib.rs");
    for written in [
        "pub use crate::kv::open;",
        "pub use crate::kv::Store;",
        "pub use crate::kv::Batch;",
    ] {
        assert!(lib.contains(written), "{written} is not in:\n{lib}");
    }

    let store = dir.parent().expect("a parent").join("store");
    let output = varyk_logged(&store, &["test"]);
    assert_ok(&output);
    let stdout = stdout_of(&output);
    assert!(
        stdout.contains("test tests::one_reads_what_put_stored ... ok"),
        "{stdout}"
    );
    assert!(stdout.contains("1 passed"), "{stdout}");
}

/// The stub `varyk-http` (milestone 5b4 spec 6), which every HTTP test
/// builds on, checks and passes its own tests, so rustc compiles its whole
/// facade here and not first under a program's routes; one adds a route to
/// its own `App` and sends a request (spec 2.1).
#[test]
fn the_varyk_http_stub_s_test_passes() {
    let dir = common::package_copy("tests/fixtures/packages", "varyk-http");
    let output = varyk_logged(&dir, &["test"]);
    assert_ok(&output);
    let stdout = stdout_of(&output);
    assert!(
        stdout.contains("test tests::an_empty_response_is_a_204 ... ok"),
        "{stdout}"
    );
    assert!(
        stdout.contains("test tests::a_route_answers_a_request ... ok"),
        "{stdout}"
    );
    assert!(stdout.contains("2 passed"), "{stdout}");
}

/// The fixture package `facade_types` passes its own tests under `varyk
/// test` (milestone 5c spec 2.5): its `.rs` facade takes and gives
/// `Time`, `Uuid`, and `Bytes`, holds them in `pub` fields, and gives an
/// enum whose variant holds a `Bytes`.
#[test]
fn facade_types_s_test_passes() {
    let dir = common::package_copy("tests/fixtures/packages", "facade_types");
    let output = varyk_logged(&dir, &["test"]);
    assert_ok(&output);
    let stdout = stdout_of(&output);
    for name in [
        "a_time_goes_through_the_facade",
        "an_id_goes_through_the_facade",
        "bytes_are_lent_and_given",
        "an_upload_holds_the_three",
        "a_binary_message_holds_bytes",
    ] {
        assert!(
            stdout.contains(&format!("test tests::{name} ... ok")),
            "{stdout}"
        );
    }
    assert!(stdout.contains("5 passed"), "{stdout}");
}

/// What `http_user` prints (its `src/main.vr` says the same).
const HTTP_USER_OUTPUT: &str = "GET /users/1 -> 200 {\"id\":1,\"name\":\"Ada\"}\n\
     GET /users/9 -> 404 {\"error\":\"not found\"}\n\
     GET /users/abc -> 400 {\"error\":\"the path parameter `id` cannot be read from `abc`\"}\n\
     GET /users/-1 -> 500 {\"error\":\"internal error\"}\n\
     GET /names/Bo -> 200 {\"id\":2,\"name\":\"Bo\"}\n\
     GET /search?prefix=A&exact=false -> 200 [\"Ada\"]\n\
     GET /search?prefix=A -> 400 {\"error\":\"the query parameter `exact` is missing\"}\n\
     GET /search?prefix=Bo&exact=true -> 200 [\"Bo\"]\n\
     GET /search?exact=true -> 400 {\"error\":\"an exact search needs a prefix\"}\n\
     GET /orders/0192e1c4-5b8a-7c3d-9e4f-0123456789ab?since=2026-10-07T12:00:00Z -> 200 order 0192e1c4-5b8a-7c3d-9e4f-0123456789ab since 2026-10-07T12:00:00Z\n\
     GET /orders/0192e1c4-5b8a-7c3d-9e4f-0123456789ab -> 200 order 0192e1c4-5b8a-7c3d-9e4f-0123456789ab\n\
     GET /orders/42 -> 400 {\"error\":\"the path parameter `id` cannot be read from `42`\"}\n\
     POST /users -> 201 {\"id\":3,\"name\":\"Cy\"}\n\
     POST /users -> 400 {\"error\":\"a user needs a name\"}\n\
     POST /users -> 400 {\"error\":\"the body `user` is not valid: missing field `name` at line 1 column 13\"}\n\
     PUT /users/2 -> 200 {\"id\":2,\"name\":\"Bo (renamed)\"}\n\
     GET /echo -> 200 GET /echo\n\
     GET /echo/7 -> 200 GET /echo/7 with 7\n\
     GET /echo/abc -> 400 {\"error\":\"the path parameter `id` cannot be read from `abc`\"}\n\
     GET /ping -> 204\n\
     DELETE /ping -> 204\n\
     GET /admin/users -> 200 [{\"id\":1,\"name\":\"Ada\"},{\"id\":2,\"name\":\"Bo\"}]\n\
     GET /users/1 -> 401 {\"error\":\"an `x-key` header is needed\"}\n\
     GET /admin/users -> 403 {\"error\":\"forbidden\"}\n\
     GET /nowhere -> 404, with x-users 2\n\
     served on port 3000\n";

/// `http_user` runs under `varyk run` beside the stub `varyk-http`
/// (milestone 5b4 spec 4, 9): one adapter per route, lending the
/// handler's parameters by mode and answering by its return shape, every
/// request sent through `app.request`, and `serve` the stub's `Ok(true)`.
/// The message of an error with no status reaches the log, not the
/// client, under `varyk run` and under `varyk test`. Its generated
/// `main.rs` is the adapters' snapshot.
#[test]
fn http_user_answers_requests_through_its_routes() {
    let dir = common::package_copy("tests/fixtures/packages", "http_user");
    let output = varyk_logged(&dir, &["run"]);
    assert_ok(&output);
    assert_eq!(stdout_of(&output), HTTP_USER_OUTPUT);
    let stderr = stderr_of(&output);
    assert!(
        stderr.contains("the user store refused a negative id"),
        "stderr: {stderr}"
    );

    let main = fs::read_to_string(dir.join("target/varyk/http_user/src/main.rs"))
        .expect("the generated main.rs");
    assert!(!main.contains("axum"), "{main}");
    insta::assert_snapshot!("http_user_main_rs", main);

    // Its async test starts logging too, so the 500's message reaches
    // stderr under `varyk test` (milestone 5b4 spec 7.5).
    let output = varyk_logged(&dir, &["test"]);
    assert_ok(&output);
    let stdout = stdout_of(&output);
    assert!(
        stdout.contains("test a_negative_id_is_a_500 ... ok"),
        "{stdout}"
    );
    assert!(stdout.contains("1 passed"), "{stdout}");
    let stderr = stderr_of(&output);
    assert!(
        stderr.contains("the user store refused a negative id"),
        "stderr: {stderr}"
    );
}

/// A program that calls a facade method of `store` whose signature names
/// `varyk_std::` needs `varyk-std` itself, since the call site writes
/// `::varyk_std::Value::from` in the program's crate; one that only names
/// `store`'s items does not (M5b3 spec 2.4). Neither names `Error`, so
/// only the call counts.
#[test]
fn a_program_needs_varyk_std_at_a_call_through_store_and_not_before() {
    let dir = common::package_copy("tests/fixtures/packages", "store_user");
    let manifest = dir.join("Cargo.toml");
    let text = fs::read_to_string(&manifest).expect("read the manifest");
    // Any version: the release pull request bumps the line.
    let without: String = text
        .lines()
        .filter(|line| !line.starts_with("varyk-std = "))
        .map(|line| format!("{line}\n"))
        .collect();
    assert_ne!(text, without, "{text}");
    fs::write(&manifest, without).expect("write the manifest");
    let main = dir.join("src/main.vr");

    fs::write(
        &main,
        "fn main() {\n    let db = store::open();\n    println!(\"opened\");\n}\n",
    )
    .expect("write main.vr");
    let output = varyk_logged(&dir, &["run"]);
    assert_ok(&output);
    assert_eq!(stdout_of(&output), "opened\n");

    fs::write(
        &main,
        "fn main() {\n    let mut db = store::open();\n    println!(\"{}\", db.put(\"k\", 1).is_ok());\n}\n",
    )
    .expect("write main.vr");
    let output = varyk_in(&dir, &["check"]);
    assert!(!output.status.success(), "{}", stdout_of(&output));
    let stderr = stderr_of(&output);
    assert!(
        stderr.contains("V0404") && stderr.contains("needs `varyk-std`"),
        "{stderr}"
    );
}
