# Milestone 5a Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give Varyk the synchronous service batteries: one `Error` type, JSON, configuration from the environment, logging, tests, and `varyk add`, through a new `varyk-std` crate.

**Architecture:** The pipeline is unchanged. Every new call is a row of the standard table in `builtins.rs`, typed from the expected type where it is generic, and emitted as an absolute `::varyk_std` path; `varyk-std` is a plain Rust crate in the workspace that implements the rows. Serde's traits are derived per direction only on the types a `json` or `env` call reaches, computed beside the existing `Clone`/`PartialEq` judgement in `types/derives.rs`. Attributes are parsed on five AST nodes, validated in the resolver, and read by the checker and backend.

**Tech Stack:** Rust stable, edition 2024, MSRV 1.85. New crate `varyk-std` with serde (derive), serde_json, tracing, tracing-subscriber (`fmt` only, default features off: the JSON and text formats are Varyk's own `FormatEvent`, without colour). No new dependencies in `varyk` or `varyk-syntax`.

**Spec:** `docs/specs/2026-09-30-milestone-5a-design.md`. "Spec 2.4" refers to its sections, "M4 §n" to the earlier specs. Read it before any task.

## Global Constraints

- `AGENTS.md` applies in full (the gate after every task, MSRV 1.85 with no let-chains, minimum viable, fail loudly, no hidden allocation, stable codes with a fixture and a `docs/language.md` row, snapshots reviewed, conventional commits). Before any push, `rustup run 1.85 cargo build --workspace` passes.
- MVP: spec 2.11 is the cut list; implement nothing on it. No `unwrap` or `expect` anywhere, `varyk-std` included.
- New codes exactly: V0112, V0113, V0114, V0209, V0210, V0404. Narrowed: V0203 (`{}` accepts `Error`). Reused: V0200, V0202, V0206, V0207 (spec 8).
- Generated code names `varyk-std` only as `::varyk_std::…` (spec 7.5).
- `.env` is never written into the process environment; `varyk-std` never calls `std::env::set_var` (spec 2.5).
- `varyk check` never runs cargo (M3 §2.2). Every new example has an inline expected string in `crates/varyk/tests/examples.rs`.
- Tests build against `crates/varyk-std` through `VARYK_STD_PATH` or cargo's `--config patch.crates-io.varyk-std.path=…`, never crates.io (spec 5.3).

## Review Focus

Inputs the spec implies but no example exercises, most likely to bite first. Each has a test in the task that owns the code.

1. A `.env` value containing `=` (`DATABASE_URL=postgres://h/db?sslmode=require`) reads as everything after the first `=`, and a `.env` line `export PORT=1` is a malformed-line `Error` naming line and text, not a silent skip. Task 1.
2. `{"age": 300}` into a `u8` field, and `"12abc"` for an env `u16`, are an `Error` with a readable message, never a panic. Tasks 1 and 5.
3. A single file whose only mention of `varyk-std` is `fn f() -> Result<i32, Error> { Ok(1) }` builds (the dependency is added), and a single file with no `varyk-std` use has no `varyk-std` line in its generated manifest. Task 2.
4. A struct with a `#[skip]` non-`Option` field and no default is fine when only `json::stringify` reaches it, and V0209 only when a `parse` does. Task 5.
5. `json::stringify` of a `HashMap` is valid JSON whatever the key order; no example prints one, because the order is not stable. Task 5 (a round-trip unit test) and task 9 (example review).

---

## Overview

Nine tasks. Task 1 builds `varyk-std` on its own with unit tests and wires it into the workspace and release files. Task 2 adds `Error` and turns `parse` into a `Result`, which is the first use of `varyk-std` from generated code, and so also brings the single-file dependency, `VARYK_STD_PATH`, and the test-harness plumbing. Task 3 does the package side: V0404, `varyk init`, `varyk add`, the `greeting` example, and the plain-cargo tests. Task 4 adds attribute syntax and the resolver's checks, with the attributes still inert. Tasks 5, 6, 7, and 8 deliver JSON, configuration, logging, and tests, each with its example. Task 9 adds the `users` package and finishes the documentation.

## Context

Line numbers are approximate. Paths are under `crates/` unless shown otherwise.

- Syntax: `varyk-syntax/src/lexer.rs` (`#` rejected at 416, test `hash_names_attributes` 782; identifiers may start with `_` at 103); `token.rs` (`Bang` 72, no `Hash`); `ast.rs` (`Function` 122, `StructDecl` 149, `FieldDecl` 161, `EnumDecl` 172, `EnumVariant` 183, `VariantFields` 192, `VariantField` 202, `ImplBlock` 212, literal exprs 385-391, `Unary{Neg}` 329/410, `Path` 405, `Call` 428, `Intrinsic{name, format, args}` 529, pattern `Literal` 606); `parser/item.rs` (`parse_item` 30, `parse_function` 83, `parse_struct` 340, `parse_field` 370, `parse_enum` 499, `parse_enum_variant` 532, `parse_variant_field` 585, `parse_impl` 644); `parser/expr.rs` (`parse_unary` 202, macro detection 383, `parse_macro_like` 559, `parse_dotted_path` 687, `path_and_name` 710, test `path_call` 1206).
- Resolver: `varyk/src/resolve/mod.rs` (`Symbols::lookup_fn` 476, `target_module` 884, `reserved_type_name` 1027, `reserved_value_name` 1041, `taken` 1051, `collect_symbols` 1073 with its reserved checks 1142-1384, `push_fn` 1457, `check_entry_main` 1661); `resolve/modules.rs` (`load_modules` 23, `check_name` 87 with reserved module names 137); `resolve/uses.rs` (`register` 113 with reserved checks 155-168, `resolve_use_path` 345); `resolve/paths.rs` (`module_at` 55).
- Table and types: `varyk/src/builtins.rs` (`Owner` 11, `Shape` 111 with `OptionOfExpected` 126 made a type at 228, `Builtin` 315, `TABLE` 422, `parse` row 486, `BuiltinId::path` 576, `lookup` 611, `names` 624); `types.rs` (`Ty` 9, `BUILTIN_TYPE_NAMES` 32); `types/derives.rs` (`Derives` 20, `Trait` 39, `compute` 107, `solve` 116, `can_clone`/`can_compare` 238/243), called at `types/check.rs` 46.
- Checker: `types/check.rs` (`result_try_operand` 249, Intrinsic arm 777, Try arm 792, `==` check 1062, `call` 1146 with `lookup_fn` 1199, `format_args` 1496 with V0202 1531-1558 and V0203 1575/1586, `blocked` 1688); `types/check/methods.rs` (`method_call` 25, `OptionOfExpected` branch 167, `parsed_type` 368 with V0206 392 and V0207 hints 395/410, `try_` 468); `types/check/values.rs` (`path_owner` 92, `vec_lit` 597, `type_hole` 645).
- HIR: `varyk/src/hir.rs` (`HirStruct` 115 with `derives` 125, `HirEnum` 130 with `derives` 143, `HirFunction` 148).
- Backend: `varyk/src/backend/rust.rs` (`generate` 33 with the manifest 34, `emit_struct` 194, `emit_enum` 211, `write_derives` 243, function emission 365); `backend/mod.rs` (`CrateInfo` 43, `single_file` 55); `backend/rust_expr.rs` (Call arm 356, `method_call` 775, the `parse` special case 876, `path_call` 885, `format_call` 907, `callee` 997 with builtin paths 1032).
- Driver and CLI: `varyk/src/driver/generate.rs` (`write_files` 15, `sync_lock` 172, a one-way copy of the package's lock into the hidden crate); `driver/cargo.rs` (`DriverError` 14, `run_cargo` 52); `driver/mod.rs` (`build` 129, `run` 156); `driver/init.rs` (`files` 44, `cargo_toml` 63); `driver/templates/gitignore.txt`; `driver/publish.rs` (`assemble` 40, `write_manifest` 88); `driver/messages.rs` (`classify` 102); `varyk/src/cli.rs` (`Command` 79, dispatch 153-180, `run_build` 327, `run_run` 351, `generate_and_build` 395, `crate_info` 450 choosing `isolated_manifest` or `CrateInfo::single_file`); `varyk/src/package.rs` (`dependencies` 37, per-dependency checks 320, lock location 586, `isolated_manifest` 968).
- Diagnostics: `diagnostics/codes.rs` (consts 12-105, `ALL` 109 with its completeness test 120).
- Tests: `tests/common/mod.rs` (`varyk` 15, `varyk_with_env` 21, `varyk_in` 36, `package_dir` 62, `example_dir` 84); `tests/errors.rs` (`error_case!` 57, `package_error_case!` 69, registrations 79-256); `tests/examples.rs` (`assert_runs` 26, `package` 529, `cargo_in` 537, `greeting_keeps_the_files_varyk_init_writes` 580, greeting under cargo 595); `tests/packages.rs` (`cargo_build_and_run` 534 expecting `dir/target/debug` at 548, used by the three `CARGO_TARGET_DIR` tests at 555, 577, 1234; other plain cargo runs 932, 954, 996, 1316, 1330, 1343); `tests/soundness.rs` (`PRELUDE` 45, `MUST_PASS` 1080, `MUST_REJECT` 1283, `run` 2067, hand-written generated-crate manifests 3545 and 3722; 2634 is a Varyk package's own manifest); `tests/codegen.rs` (`generate_str` 33, `main_rs` 70, `hello_cargo_toml` 84); `tests/reference.rs` (table calls 64, codes 71); `tests/cli.rs` (`check_passes_on_every_example_entry_file` 127).
- `parse` today: `examples/text.vr` 37 and 42, `examples/readings.vr` 22; 17 `Option<..> = x.parse()` sites across `types/check/tests.rs` (3187, 3289, 3306, 3441), `tests/fixtures/codegen/{tables,lookups}/main.vr`, `soundness.rs` (888, 1191), `docs/language.md` (444-453); error fixtures `v0207_parse_nothing_expected`, `v0207_parse_then_ok_or`, `v0206_parse_question_in_result_function`.
- Release: `release-please-config.json` (`release-as` 3, packages 7-16, `linked-versions` 22-29); `.release-please-manifest.json`; `.github/workflows/release-please.yml` (publish job 23, crates 36-43); root `Cargo.toml` (`members` 3); `.github/workflows/ci.yml` (MSRV job 30).
- Docs: `docs/language.md` (Types 247, Calls on built-in types 393, Literals 522, Passing errors on 938, Printing 977, Functions 995, Not in milestone 4 1136, The command line 1454, init 1511, publish 1577, Error codes 1602); `docs/roadmap.md` milestone 5; `docs/open-questions.md`; `docs/design.md` decisions log.

## Development Approach

- TDD per task: failing test, minimal code, the task's gate. Unit tests beside the code; snapshots and integration tests under `crates/varyk/tests/`.
- Every new call is a `Builtin` row plus a soundness template plus a `docs/language.md` line, following the `parse` row (`builtins.rs` 486) and its expected-type shape `OptionOfExpected`; module calls (`json::`, `env::`, `log::`) and `Error::new` are rows under new `Owner`s, reached from a path call the way `Vec::new` is (`values.rs` 92).
- Diagnostics: each new code gets fixtures under `tests/fixtures/errors/v<code>_<case>/` registered in `tests/errors.rs`, with plain-word headlines (spec 8).
- Derives follow the existing fixed point in `types/derives.rs`: convertibility is a third and fourth trait judged the same way, and the reached sets are a walk from each call site.
- Generated names: helper functions the backend adds (the `#[default]` functions) are named `varyk_default_<type>_<field>`; task 4 reserves the `varyk_` prefix for items so no Varyk name can collide (a one-line addition to spec 2.10 in that task).
- Not needed and why: a `.env` crate (`dotenvy` writes the process environment and has its own messages; the reader is small), `envy` (it lower-cases variable names, which breaks the `#[rename]` rule, so `env::parse` uses a small serde `Deserializer` that reads the struct's field list), the importer for `varyk-std` (spec 1, approach 1). Tests that build a `varyk-std` program resolve serde and tracing from crates.io, as the `matcher` test already does for its crate; update the `examples.rs` comment that calls `matcher` the one test that fetches from crates.io.

## Tasks

### Task 1: The `varyk-std` crate

The runtime half, built and tested alone (spec 4, 2.3-2.6, 2.8, 5.3).

**Files:**
- Create: `crates/varyk-std/Cargo.toml`, `crates/varyk-std/src/{lib.rs,error.rs,json.rs,env.rs,dotenv.rs,log.rs,parse.rs}`
- Modify: root `Cargo.toml` (members), `release-please-config.json`, `.release-please-manifest.json`, `.github/workflows/release-please.yml`, `README.md` and `AGENTS.md` (the crate list)
- Test: unit tests in each module

**Interfaces:**
- Produces: `varyk_std::Error` (`new(String) -> Error`, `message(&self) -> &str`, `Display`, `Clone`, `PartialEq`, `Debug`); `varyk_std::json::parse<T: DeserializeOwned>(&str) -> Result<T, Error>`; `varyk_std::json::stringify<T: Serialize + ?Sized>(&T) -> String`; `varyk_std::env::parse<T: DeserializeOwned>() -> Result<T, Error>`; `varyk_std::parse<T: Parse>(&str) -> Result<T, Error>` with a sealed `Parse` trait for every Varyk number type and `bool`; `varyk_std::start()`; `pub use serde; pub use tracing;`; `crate::dotenv::load(dir) -> Result<Vec<(String, String)>, Error>` and a process-wide `OnceLock` of the current directory's `.env`

- [ ] Create the crate (edition 2024, `rust-version = "1.85"`, version equal to the workspace crates, description and license as the others) and add it to the workspace members
- [ ] Write `Error` with its tests (`Display` is the message; `==` compares messages)
- [ ] Write `parse` with tests: `"42"` into `i32`, `"abc"` into `i32` gives "`abc` is not a number", `"yes"` into `bool` gives "`yes` is not `true` or `false`", `"300"` into `u8` is an error
- [ ] Write the `.env` reader with tests: `KEY=value`, blank and `#` lines skipped, single and double quotes stripped, a value containing `=`, a missing file as no entries, `export PORT=1` and a line without `=` as an `Error` naming the line number and text (review focus 1)
- [ ] Write `env::parse` as a small serde `Deserializer` over the struct's field list, each field read from its upper-cased key, first from a lookup function then from the `.env` entries, with tests through an injected lookup (never the process environment): lookup order, a missing variable naming it, `Option` missing as `None`, a bad number naming variable and value, `bool`, a unit enum by its key, a renamed field
- [ ] Write `json::parse`/`stringify` with tests: a struct round trip, a missing `Option` key, an unknown key ignored, a number out of range as an `Error` (review focus 2), a `HashMap` round trip compared as values, not text (review focus 5); `stringify` returns `String` and its unreachable error branch returns the error's text, with a comment that the checker admits only types that cannot fail
- [ ] Write `start()` and its pure helpers with tests: level from `LOG` (case-insensitive; unset is `info`; a bad value or unreadable `.env` gives one warning line and `info`), `LOG_FORMAT=json` choosing JSON lines, text without colour; a small custom `FormatEvent` for both formats, since tracing-subscriber's own JSON uses `timestamp` and `fields.message` and its text adds the target: text is `<time> <LEVEL> <message>`, JSON has exactly `time`, `level`, `message`, each tested on a captured writer; `start()` itself reads the process environment and the `.env` cache and installs the subscriber, and is not unit tested
- [ ] Add `crates/varyk-std` to `release-please-config.json` (package entry and `linked-versions`), `.release-please-manifest.json`, and the publish job before `varyk-syntax`; remove the stale `release-as: 0.2.0`
- [ ] Gate: `cargo test -p varyk-std`; the full gate

### Task 2: `Error`, `parse` as a `Result`, and the single-file dependency

The first generated use of `varyk-std`, end to end for single files (spec 2.3, 2.8, 2.10 for `Error`, 5.2, 5.3).

**Files:**
- Modify: `varyk/src/types.rs`, `builtins.rs`, `types/derives.rs` (`Error` can be cloned and compared), `types/check.rs`, `types/check/methods.rs`, `types/check/values.rs`, `hir.rs`, `resolve/mod.rs`, `backend/{mod,rust,rust_expr}.rs`, `driver/generate.rs`, `cli.rs`, `diagnostics/codes.rs`; `tests/common/mod.rs`, `tests/soundness.rs`, `tests/codegen.rs`, `tests/errors.rs`, `tests/examples.rs`, `tests/packages.rs`, `types/check/tests.rs`, the codegen fixtures `tables` and `lookups`; `examples/text.vr`, `examples/readings.vr`; `docs/language.md`
- Test: checker unit tests, `tests/codegen.rs`, `tests/errors.rs`, `tests/soundness.rs`, `tests/examples.rs`

**Interfaces:**
- Consumes: task 1's `varyk_std::{Error, parse}`
- Produces: `Ty::Error` (in `BUILTIN_TYPE_NAMES`); `Owner::Error` with rows `new` and `message`; `HirProgram.uses_std: bool`, set when any signature, field, local, or type argument is `Error` or any `varyk-std` row is called (spec 1); `CrateInfo.std_dependency: Option<StdDependency>` where `StdDependency { Version(String), Path(PathBuf) }`, and `CrateInfo::single_file(name, std_dependency)` taking it, with one helper `StdDependency::for_program(uses_std) -> Option<StdDependency>` (the compiler's `CARGO_PKG_VERSION`, or `VARYK_STD_PATH` when set) used by `cli.rs::crate_info` and by the direct callers in `tests/codegen.rs` 22, `tests/examples.rs` 210, `tests/soundness.rs` 3760, and the unit test at `backend/mod.rs` 152; the struct literals `CrateInfo { name, manifest }` at `cli.rs` 452, `tests/examples.rs` 657, and `tests/packages.rs` 792 gain `std_dependency: None`; V0113 with its first case

- [ ] Write checker tests: `Error::new("x")` and `e.message()` type; `println!("{}", e)` accepted (V0203 narrowed); `==` on `Error`; a struct holding an `Error` keeps `Clone` and `==`; `let n: i32 = s.parse()?` in a function returning `Result<i32, Error>`; `let r: Result<i32, Error> = s.parse()` then `r.ok()`; `?` on it in a function returning `Result<_, string>` is V0206; a struct or enum named `Error`, and a `.rs` `pub struct Error`, are V0113
- [ ] Implement `Ty::Error`, the rows (`new` takes an owned `string`, so a literal is the one allowed allocation and a stored `string` moves; `message` returns part of its receiver, a rooted result as M4's `trim` is), `parse` returning `Result<T, Error>` (rename `OptionOfExpected` to what it now is), V0203's `Error` case, V0113 for the name `Error`, and `uses_std`; update the V0207 and V0206 hints at `methods.rs` 395-410 to the two-statement `Result` form
- [ ] Backend: `Error::new(s)` as `::varyk_std::Error::new(s)`, `e.message()` as a borrowed call, `parse` as `::varyk_std::parse::<T>(&s)` replacing the `.parse::<T>().ok()` case at `rust_expr.rs` 876, `Error` in types as `::varyk_std::Error`
- [ ] Driver: `CrateInfo::single_file` writes `varyk-std = "=<version>"` when `std_dependency` is set, and a `path` dependency instead when `VARYK_STD_PATH` is set; for a package, `VARYK_STD_PATH` replaces the `varyk-std` entry of `isolated_manifest`'s output; `varyk publish` never applies it
- [ ] Test harness: `tests/common/mod.rs` sets `VARYK_STD_PATH` to the workspace's `crates/varyk-std` in `varyk_with_env` and `varyk_in`; the soundness harness's hand-written generated-crate manifests (`soundness.rs` 3545 and 3722; not 2634, which is a Varyk package's own manifest) gain a `varyk-std` path dependency
- [ ] Codegen snapshots: a single file using `Error` only in a signature has the dependency line, `hello.vr` has none (review focus 3); `parse` and `Error` calls; the manifest text has `CARGO_PKG_VERSION` replaced by a fixed placeholder before `assert_snapshot!` (the `insta` filters feature is not enabled and would add `regex`), so a release bump never changes a snapshot
- [ ] Update every `parse` site `grep -rn "parse()" crates examples docs` finds to the `Result` form (Context lists the known ones; `borrow/returns/tests.rs` 221 uses `text.parse()?` in functions returning `Option`, and `soundness.rs` 961 is another), and every place that shows the generated `.parse::<T>().ok()`: the inline asserts at `tests/codegen.rs` 744-747 and the snapshots `codegen__tables_main_rs`, `codegen__lookups_main_rs`, and `codegen__readings_main_rs`; the tables fixture's `doubled` (`text.parse()?` in an `Option` function) is rewritten, with `run_table_rows` (`tests/examples.rs` 176) updated if its output changes, the three `parse` error fixtures and their snapshots, `examples/text.vr` and `readings.vr` with their expected output unchanged or updated, and the soundness `parse` templates; add must-build templates for `Error::new`, `message`, and `?` on `parse`
- [ ] Add a run test for review focus 3 (a single file using `Error` only in a signature builds and runs)
- [ ] Register V0113 in `codes.rs` with fixtures for a struct named `Error` and a `.rs` `pub struct Error`
- [ ] `docs/language.md`: `Error`, the `parse` row and notes, V0113 and the V0203 row
- [ ] Gate: `cargo test --workspace`

### Task 3: Packages, `varyk init`, and `varyk add`

The package side of the dependency (spec 5.1-5.3) and the `CARGO_TARGET_DIR` follow-up.

**Files:**
- Modify: `varyk/src/package.rs`, `cli.rs`, `driver/init.rs`, `driver/templates/gitignore.txt`, `driver/mod.rs` (the `add` pass-through), `diagnostics/codes.rs`; `examples/packages/greeting/{Cargo.toml,.gitignore}`, delete `examples/packages/greeting/Cargo.lock`; `release-please-config.json` (`extra-files`); `tests/examples.rs`, `tests/packages.rs`, `tests/errors.rs`, `tests/cli.rs`; `docs/language.md`; `docs/plans/2026-09-29-milestone-4-followups.md`
- Test: `package.rs` unit tests, `tests/errors.rs` package cases, `tests/packages.rs`, `tests/examples.rs`

**Interfaces:**
- Consumes: `HirProgram.uses_std` (task 2)
- Produces: `package::check_std_dependency(&Package, uses_std: bool, compiler_version: &str) -> Vec<Diagnostic>`, called with `CARGO_PKG_VERSION` and tested with fixed versions so a release bump never breaks a test returning V0404s, run after type checking wherever `check` diagnostics are gathered for a package; `Command::Add { args: Vec<String> }`

- [ ] Write unit tests for `check_std_dependency`: missing; `"0.2"`, `"0.2.0"`, `"^0.2"`, and a table with `version` accepted for compiler 0.2.x; `"0.1"`, `"~0.2"`, `"=0.2.0"`, `">=0.2"`, `"*"`, a `path`, a `git`, `optional = true`, and `package = "x"` refused; a lock with `varyk-std` older than the compiler refused with the `cargo update -p varyk-std` help; no lock accepted; a package that does not use `varyk-std` never checked
- [ ] Implement it with the requirement grammar of spec 5.1 by hand (no `semver` crate) and the lock read with `toml`; the fix (the line to write, or `cargo update -p varyk-std`) is a note, since `package_error_case!` expects no fix-it (`errors.rs` 69); add V0404 and package error fixtures for missing, wrong minor, and stale lock, the stale-lock fixture's `Cargo.toml` carrying an empty `[workspace]` table so it is its own root and its own lock is read (`package.rs` 446, 582)
- [ ] Update `init_in_an_empty_dir_writes_the_five_files_named_after_the_directory` (`packages.rs` 452) to expect `"/target\n.env\n"`
- [ ] `varyk init`: `cargo_toml` writes `varyk-std = "<full version>"` under `[dependencies]`; `gitignore.txt` gains `.env`; update `greeting`'s `Cargo.toml` and `.gitignore` to match, delete its `Cargo.lock`, and add `examples/packages/greeting/Cargo.toml` to `extra-files` with the `toml` updater at `$.dependencies.varyk-std` (confirm the path form against release-please's config schema, and with a dry run where a token is available, before committing)
- [ ] Plain-cargo tests: `cargo_in` (`examples.rs` 537) and every plain `cargo` command in `packages.rs` pass `--config patch.crates-io.varyk-std.path="<workspace>/crates/varyk-std"`; the greeting plain-cargo run already works on a copy (`example_dir`), so no `Cargo.lock` is written into the tree
- [ ] Fix the follow-up: `cargo_build_and_run` (`packages.rs` 534) finds the binary through `CARGO_TARGET_DIR` when it is set, so the tests at 555, 577, and 1234 pass with it exported; strike the item from the milestone-4 follow-ups file
- [ ] `varyk add`: run `cargo add` with the arguments in the package directory, output and exit code passed through; test on a temporary package with no `varyk-std` line adding a path dependency, offline
- [ ] `docs/language.md`: `varyk init`'s new line, `varyk add`, upgrading, and the V0404 row
- [ ] Gate: `cargo test --workspace`

### Task 4: Attributes and reserved names

Attributes parse and are validated; they have no effect until tasks 5 to 8 (spec 2.1, 2.2 placement rules, 2.7 test shape, 2.10).

**Files:**
- Modify: `varyk-syntax/src/{lexer.rs,token.rs,ast.rs,lib.rs}`, `varyk-syntax/src/parser/item.rs`; `varyk/src/resolve/{mod,modules,uses}.rs`, `hir.rs`, `types/check.rs` (HIR structs and functions built at 83, 97, 318; the call check at 1199), `diagnostics/codes.rs`; `tests/errors.rs`; `docs/specs/2026-09-30-milestone-5a-design.md` (2.10: the `varyk_` prefix); `docs/language.md`
- Test: inline parser tests, resolver unit tests, `tests/errors.rs`

**Interfaces:**
- Produces: token `Hash`; `ast::Attribute { name: Ident, arg: Option<AttrArg>, span }` with `AttrArg { Str(String), Int { text: String, negative: bool }, Float { text: String, negative: bool }, Bool(bool) }`; `attrs: Vec<Attribute>` on every item kind of `Item` (`ast.rs` 99), on `FieldDecl`, `EnumVariant`, `VariantField`, and on impl methods (which are `Function`); resolved `hir::FieldAttrs { rename: Option<String>, default: Option<HirDefault>, skip: bool }` on struct fields, `rename: Option<String>` on variants, `is_test: bool` on `HirFunction`, where `HirDefault { Int(i128), Float(f64), Str(String), Bool(bool) }`

- [ ] Parser tests: `#[rename("userName")]` on its own line and inline before a field, stacked `#[skip] #[default(1)]`, `#[default(-1)]`, `#[default(1.5)]`, `#[default(true)]`, `#[test]` before `fn`, attributes before a variant, a variant field, and a method; `#!` stays a lexer error, and a stray `#` elsewhere is a parser error naming where attributes may go; `hash_names_attributes` (`lexer.rs` 782) is rewritten
- [ ] Implement the token, the AST, and the parsing: an attribute list before every item, field, variant, variant field, and method
- [ ] Resolver tests and implementation for V0112: `#[derive(Clone)] struct` (unknown name, with the `derive` note), any attribute on a struct, enum, `impl`, `mod`, or `use`, an unknown name on a field, `#[test]` on a field, `#[skip]` on a variant, `#[rename]` on a data variant or a variant field, any attribute on a method, the same attribute twice, a missing or unexpected argument (`#[rename]`, `#[default]`, `#[skip(1)]`, `#[test(1)]`); a literal of the wrong kind (`#[rename(1)]`) is V0209, below
- [ ] V0113 widened: modules or module files named `json`, `env`, `log`; functions named `assert` or `assert_eq`; `use json;` and `use json::parse;`; any item, method, or module whose name starts with `varyk_` (add this line to spec 2.10 and the V0113 row)
- [ ] V0114, shape half: a `#[test]` function with parameters or a return type, and a call to a `#[test]` function (the call check lives where `call` resolves the callee, `types/check.rs` 1199)
- [ ] V0209, the part that holds on every struct: a `#[rename]` literal that is not a string, a `#[default]` literal that does not fit its field (`"x"` on an `i32`, `300` on a `u8`), `#[default]` on an `Option`, enum, or struct field, an empty `#[rename("")]`; register V0209 with fixtures; the checks that depend on reachability (a parsed `#[skip]` with no default, unique keys) come in task 5
- [ ] Fill the resolved attribute fields in HIR
- [ ] Fixtures for each V0112, V0113, and V0114 case above; `docs/language.md` gains an "Attributes" section, the V0112, V0114, and V0209 rows, and the widened V0113 row (`tests/reference.rs` needs a row for every registered code)
- [ ] Gate: `cargo test -p varyk-syntax`; `cargo test --workspace`

### Task 5: JSON

`json::parse`, `json::stringify`, convertibility, and the derives (spec 2.2 semantics, 2.4, 2.9).

**Files:**
- Modify: `varyk/src/builtins.rs`, `types/derives.rs`, `types/check.rs`, `types/check/methods.rs`, `types/check/values.rs`, `hir.rs`, `backend/{rust,rust_expr}.rs`, `diagnostics/codes.rs`; `tests/{codegen,errors,soundness,examples,cli}.rs`; `docs/language.md`
- Create: `examples/json.vr`
- Test: derives and checker unit tests, codegen snapshots, `tests/examples.rs`

**Interfaces:**
- Consumes: task 4's `FieldAttrs`, variant `rename`; task 2's `Ty::Error` and expected-type machinery
- Produces: `Owner::Json` rows `parse` (expected type) and `stringify`; a separate `serde: Serde { serialize: bool, deserialize: bool }` field on `HirStruct` and `HirEnum`, meaning "reached in this direction and convertible", computed after checking from the call sites; `Derives` (also the derive list read from `.rs` files, `interop/items.rs` 333) and its tests are left unchanged; a `Convertible` judgement in `derives.rs` returning the blocking part for V0210; the reachability-dependent V0209 checks as one function over reached types, reused by task 6

- [ ] Write derives tests: a nested struct reached from `stringify` marks both types `serialize` only; `parse` marks `deserialize`; an enum with data reached gives V0210 naming the variant, with the `type`-field note; `HashMap<i32, _>` and `Error` and a `.rs` type give V0210; a type containing itself through a `Vec` is judged without looping; an unreached type with an enum-with-data field is fine
- [ ] Write V0209 tests for the reachability-dependent cases: a parsed `#[skip]` non-`Option` field with no default, two fields renamed to the same key, a renamed variant equal to another; review focus 4 (skip without default accepted when only `stringify` reaches it)
- [ ] Implement the rows (V0207 with the `let u: User = json::parse(..)?` help when no expected type), the reached-set walk from each call's type, convertibility, and V0209/V0210
- [ ] Backend: `::varyk_std::json::parse::<T>(&text)` and `::varyk_std::json::stringify(&value)`; on reached types, `#[derive(::varyk_std::serde::Serialize)]` and/or `Deserialize` with `#[serde(crate = "::varyk_std::serde")]`; `#[serde(rename = "…")]` on fields and variants; `#[serde(skip)]`; `#[serde(default = "varyk_default_<type>_<field>")]` with the function emitted beside the type, a string default written as a literal into an owned `String` (the one allowed allocation)
- [ ] Codegen snapshots of each derive combination; soundness must-build templates for each row and attribute, must-reject none needed beyond V0210 fixtures
- [ ] Write `examples/json.vr` per spec 6 (no printed `HashMap`), its `run_json` test, and add it to `check_passes_on_every_example_entry_file`
- [ ] Fixtures for the new V0209 cases and V0210; `docs/language.md` "JSON" section, the two rows, the V0210 row, and the V0209 row widened
- [ ] Gate: `cargo test --workspace`

### Task 6: Configuration

`env::parse` (spec 2.5).

**Files:**
- Modify: `varyk/src/builtins.rs`, `types/derives.rs`, `types/check/*.rs`, `backend/rust_expr.rs`; `tests/common/mod.rs`; `tests/{codegen,errors,soundness,examples,cli}.rs`; `docs/language.md`
- Create: `examples/config.vr`
- Test: checker unit tests, snapshots, `tests/examples.rs`

**Interfaces:**
- Consumes: task 5's reached-set walk (as `deserialize`), convertibility, and V0209 checks; task 1's `varyk_std::env::parse`
- Produces: `Owner::Env` row `parse`

- [ ] Write checker tests: `let c: Config = env::parse()?` accepted; a nested struct, a `Vec`, or a `HashMap` field is V0210 naming the field, but not when skipped; two fields whose upper-cased keys match (`user_name` and a field renamed `USER_NAME`) are V0209; no expected type is V0207; the `match` head form needs the typed `let` (spec 2.4)
- [ ] Add one helper to `tests/common/mod.rs`, `varyk_run_with(args, dir, set, remove)`, that runs `varyk` in a given directory with variables set and removed and `VARYK_STD_PATH` always set; tasks 7 and 9 use it too
- [ ] Implement the row and the env-shape check on top of task 5; backend `::varyk_std::env::parse::<T>()`
- [ ] Write `examples/config.vr` per spec 6 and `run_config`, with the variables set through `varyk_run_with` and one left to its `#[default]`; the test removes (`env_remove`) every variable the program reads that it does not set, and `LOG` and `LOG_FORMAT`, so a tester's own environment cannot change the output
- [ ] Fixtures, soundness template, `docs/language.md` "Configuration" section and row
- [ ] Gate: `cargo test --workspace`

### Task 7: Logging

The four `log` calls and `start()` (spec 2.6).

**Files:**
- Modify: `varyk/src/builtins.rs`, `types/check.rs` (`format_args`), `hir.rs`, `backend/{rust,rust_expr}.rs`; `tests/{codegen,soundness,examples,cli}.rs`; `docs/language.md`
- Create: `examples/logging.vr`
- Test: checker tests, snapshots, `tests/examples.rs`

**Interfaces:**
- Produces: `Owner::Log` rows `debug`, `info`, `warn`, `error`, checked by `format_args` like `println!`; `HirProgram.logs: bool`

- [ ] Write checker tests: placeholder count mismatch is V0202; a first argument that is not a string literal (`log::info(msg)`) is V0202 saying the text must be written in quotes, as `println!`'s is; `{}` on a struct is V0203; on `Error` accepted; the call's value is unit
- [ ] Implement the rows through `format_args` and `logs`
- [ ] Backend: `::varyk_std::tracing::info!("…", args)` for each level; `::varyk_std::start();` as the first statement of `main` only when `logs` is set; snapshot both with and without a `log` call
- [ ] Write `examples/logging.vr` and `run_logging` with `LOG=debug` and `LOG_FORMAT` removed from the environment, comparing stderr line by line after dropping the time column; a program without `log` calls keeps an empty stderr (every existing `assert_runs`)
- [ ] Soundness template, `docs/language.md` "Logging" section and rows
- [ ] Gate: `cargo test --workspace`

### Task 8: Tests and `varyk test`

`assert`, `assert_eq`, and the command (spec 2.7).

**Files:**
- Modify: `varyk/src/builtins.rs`, `types/check.rs`, `backend/{rust,rust_expr}.rs`, `driver/{mod,cargo}.rs`, `cli.rs`; `tests/{codegen,errors,packages,cli}.rs`; `docs/language.md`
- Create: `crates/varyk/tests/fixtures/packages/failing_test/` (a package with one passing and one failing test)
- Test: checker tests, snapshots, `tests/packages.rs`

**Interfaces:**
- Consumes: `HirFunction.is_test` (task 4)
- Produces: rows `assert` and `assert_eq`, whose HIR call node carries a `location: String` (`src/store.vr:12`) that the checker fills from the source map, since the backend sees only byte spans (`backend/mod.rs` 82); `driver::test(...)`; `Command::Test { file: Option<PathBuf> }`

- [ ] Write checker tests: `assert(x > 1)` and `assert_eq(a, b)` in a test accepted; outside a test V0114; `assert_eq` on differing types V0200, on a struct without `==` V0203
- [ ] Implement the rows and the V0114 case
- [ ] Backend: `#[test]` passes through; `assert` becomes `::std::assert!(cond, "assertion failed at {location}")`, by its full path as every std macro is (`rust_expr.rs` 903-906), and `assert_eq` uses `::std::assert!` too; `assert_eq` evaluates each operand once, as Rust's own `assert_eq!` does, by matching on references to both (`match (&a, &b) { (l, r) => assert!(..) }`, the names chosen by the compiler and the operands evaluated before them), with the comparison written through the existing `==` emission (`rust_expr.rs` `==` normalisation) and both values in the message when their type prints with `{}`, otherwise the file and line only
- [ ] `varyk test [file]`: generate as for build, then `cargo test --no-run --message-format=json` through the existing `run_cargo` mapping (`driver/cargo.rs` 52), so rustc errors map as for build; then run each test executable (the artifacts with `profile.test` true) with inherited stdio, returning the first non-zero exit code
- [ ] Integration tests: the fixture package's run exits non-zero and names the failing test and the Varyk line; a single file with a passing test exits zero
- [ ] Soundness must-build templates for `assert` and `assert_eq` inside a `#[test]` function, built with `cargo test --no-run`; add `assert` to the macro-hijacking `ext.rs` case in `MUST_RUN_TREES` (`soundness.rs` 2603)
- [ ] Fixtures for the V0114 cases; snapshots; `docs/language.md` "Tests" section and `varyk test` in "The command line"
- [ ] Gate: `cargo test --workspace`

### Task 9: The `users` package, documentation, and the definition of done

**Files:**
- Create: `examples/packages/users/{Cargo.toml,.gitignore,.env,src/main.vr,src/store.vr}`
- Modify: `tests/examples.rs`, `tests/packages.rs` (`varyk test` on `users`), `tests/cli.rs`, `tests/reference.rs` if needed; `docs/language.md` (opening line, "Not in milestone 5a" replacing "Not in milestone 4", the complete codes table), `docs/design.md` (spec 12 decisions), `docs/open-questions.md` (spec 11), `docs/roadmap.md` (spec 10), `README.md`
- Test: `tests/examples.rs`, `tests/packages.rs`, `tests/reference.rs`

- [ ] Add `examples/packages/users/Cargo.toml` to `extra-files` beside `greeting`'s (task 3)
- [ ] Write `users` per spec 6: `Config` from a committed `.env` (its `.gitignore` without `.env`, no `Cargo.lock`, `varyk-std` at the full current version), users parsed from JSON, one rejected with an `Error`, logging, the store printed as JSON from a `Vec`; `store.vr` holds `#[test]` functions
- [ ] `run_users` in `examples.rs` runs in a copy made by `example_dir("users")` (`examples.rs` 523-527; confirm it copies `.env`) as its current directory and with every variable `users` reads, and `LOG` and `LOG_FORMAT`, removed from the environment so only the committed `.env` applies, checks stdout, and checks stderr with the time column dropped; `varyk test` on `users` passes in `packages.rs`, also on an `example_dir` copy, so no test writes `target/` into the source tree
- [ ] Confirm `tests/reference.rs` covers the new table rows, types, and codes, and fix any gap in `docs/language.md`
- [ ] Update `docs/open-questions.md`, `docs/roadmap.md` (split milestone 5, check off 5a), `docs/design.md`, and `README.md` per spec 10-12
- [ ] Gate: `cargo test --workspace`; `rustup run 1.85 cargo build --workspace`; `cargo clippy --workspace --all-targets -- -D warnings`; `cargo fmt --check`; `cargo test --workspace` again with `CARGO_TARGET_DIR` exported

---

After task 9, run the whole-branch review-and-fix loop: at least five rounds, at most eight, a fresh reviewer each round, MVP fixes only, stopping early when a round finds nothing. `parse` returning a `Result` breaks existing programs, so the merge title is `feat!:` and the next release bumps the minor version.
