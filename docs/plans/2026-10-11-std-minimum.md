# A package's varyk-std line is its minimum varyk: Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Spec:** `docs/specs/2026-10-11-std-minimum-design.md` ("spec §n"). Read it in full before any task; it is the authority where this plan is brief, and every message, note, and test case it names is to be followed as written.

## Overview

Today V0404 holds every program and every Varyk package of a build to the compiler's own minor, so each minor release of varyk breaks every published package until it releases again. This change turns the `varyk-std` line into a minimum: a line at or below the compiler's version is accepted, a newer one is V0404 ("this package needs varyk X.Y or later"), and the build always uses the compiler's own `varyk-std`, written into every manifest of the build by `with_build_std` (the renamed `with_local_std`). `varyk publish` keeps today's exact-minor rule, a narrowed stale-lock check, and the resolved-version check, because plain cargo builds what it ships.

A dependency package that fails its check and whose line is on an older minor gets one more note on each error, "`units` was written for varyk 0.8; a newer release of it may build with this compiler". No new diagnostic code. Two V0404 fixtures change (one renamed to a newer-than-compiler case, one removed), the documents of spec §6 are updated, and the change ships as one `feat:` commit in the 0.9.0 release (spec §8). The documents of spec §6 that live in the official packages' own repositories get one small `docs:` pull request per package, opened after the compiler's, to merge after the varyk release that ships the rule.

## Context

Line numbers are approximate.

- `crates/varyk/src/package.rs` 871-952 `check_std_dependency` (line checks, the minor comparison, `locked_older`); 954-992 `check_build_std` (resolved-version check and its note); 995-1018 `version_triple`, `requirement_matches`; 1020-1043 `locked_older`; tests 1639-1768 (`std_check`, `accepted`, `STD_LOCK`, the minor and lock tests).
- `crates/varyk/src/packages.rs` 120-125 `Graph::std_version`; 233-242 `resolved_std`; 111-118 `Graph::for_tests`; tests 920-1008 (`build_std` helper, the resolved, stale-lock, and accepted tests, all at compiler 0.5.0 against `STD`'s 0.4.0).
- `crates/varyk/src/lib.rs` 176-225 `check_dependency` (the failing branch re-checks the `varyk-std` line, 200-213); 82-160 `check_file` (adds "in the package `units` 0.1.0, which this build uses", 108-121); tests 290-374 (`copy_example`, `trip_without_its_own_logging` through `Graph::for_tests`).
- `crates/varyk/src/cli.rs` 316-370 `load` (calls `check_build_std` at 346); callers of `load` at 298, 461, 573, 760 (`run_publish`, 751); 607-636 `crate_info` (calls `with_local_std`); 507-527 `build_dirs` (the lock copy, unchanged).
- `crates/varyk/src/backend/mod.rs` 67 `STD_PATH_VAR`; 92-107 `local_std`, `path_value`; 109-122 `with_local_std`; 125-139 `with_std_patch` (unchanged); 209 the test module.
- `crates/varyk/src/driver/mod.rs` 149-182 `package_crates` (calls `with_local_std`); 135-147 `package_dirs` (`target/varyk/packages/<name>-<version>`); `driver/publish.rs` 40 `assemble` (writes the package's own manifest, unchanged).
- `crates/varyk/src/diagnostics/codes.rs` 202-205, V0404's comment.
- `crates/varyk/tests/errors.rs` 113-118 `without_version`; 558-560 the `v0404_std_*` cases; 575-601 `v0404_failing_dependency_wrong_std` (unchanged).
- `crates/varyk/tests/fixtures/errors/v0404_std_wrong_minor/`, `v0404_std_stale_lock/`, and their snapshots under `crates/varyk/tests/snapshots/`.
- `release-please-config.json` 39-43, the `v0404_std_stale_lock` extra-files entry.
- `crates/varyk/tests/packages.rs` 723-765 (a test that writes a package by hand with `empty_dir`, the pattern for the end-to-end test); 939-968 the publish section and `assemble_package`; 1191-1212 `publish_assemble_only_...`; `crates/varyk/tests/common/mod.rs` `varyk_in`, `empty_dir`, `std_path`.
- `docs/language.md` 2971-2990 "Packages and the build" (failing package, V0404, resolved `varyk-std`); 3171-3185 the `varyk-std` paragraph of "Packages"; 3286 the V0404 row.
- `docs/design.md` 196, the "Versioning" row; `docs/roadmap.md` 211 "Milestone 6"; `docs/specs/2026-09-30-milestone-5a-design.md` 647 and `docs/specs/2026-10-02-milestone-5b2-design.md` 832, the decisions tables.
- Sibling repositories (`../varyk-http`, `../varyk-sql`, `../varyk-mongo`): the "Versions" section of each `README.md` (729, 478, 642), which says each minor varyk release is followed by a package release; the header comment of each `.github/workflows/latest-varyk.yml` (3-6), which says a new minor varyk needs a release that moves `varyk-std`.

## Development Approach

- TDD: in each task, write or change the unit tests first (spec §7 lists them), watch them fail, then change the code.
- One rule switch, two values: add a small public enum in `package.rs` (build rule, publish rule) that `check_std_dependency` and `check_build_std` take as a new last argument; `load` in `cli.rs` takes it and passes it on, and only `run_publish` passes the publish rule; `check_dependency` in `lib.rs` always passes the build rule (spec §2).
- Parse a requirement once: turn `requirement_matches` into a function that returns the requirement's version triple (`X.Y` read as `X.Y.0`, `^` stripped, the same digits-only parse), and compare triples in order, as `version_triple` and `resolved_std` already do. The `X.Y` or `X.Y.Z` in the new message is the requirement as written without its `^`.
- The rewrite is testable without the environment: `with_build_std` reads `VARYK_STD_PATH` and hands the value to write to an inner function the unit test calls, following `path_value` and `local_std` in `backend/mod.rs` (spec §7: a test cannot unset the variable safely).
- Fixture changes follow the `package_error_case!` pattern in `tests/errors.rs`; rename the fixture and its snapshot with `git mv` where the content carries over, and delete the snapshots of removed or renamed cases so no orphan `.snap` is left.
- The end-to-end test writes its packages with `empty_dir`, following `a_call_to_a_facade_naming_varyk_std_error_needs_the_varyk_std_line` in `tests/packages.rs`; it writes the program's line from `env!("CARGO_PKG_VERSION")` at run time, so no new release-please entry is needed. The new error fixture's `"999.0"` is never touched by a release either.
- Messages are plain words (AGENTS.md); copy the spec's wording for the newer-than-compiler message and note, publish's kept message, and the dependency note.
- Not needed: a new diagnostic code (spec §5); any change to `with_std_patch`, `Graph::read`, `build_dirs`, `assemble`, `varyk init`, `varyk add`, or the single-file `"=A.B.C"` dependency (spec §3, §4); a warning for a newer `varyk-std` than a package was written for, or checking a package's `.rs` against the compiler's `varyk-std` (spec §9); a CHANGELOG edit (release-please writes it).
- Outside this repository (spec §6): the package READMEs and `latest-varyk.yml` comments are Task 7, a `docs:` pull request in each package repository; their code, manifests, and the job's steps do not change (spec §4).
- Commits: a working commit per task, local only; no task pushes or opens a pull request. After the branch's review loops, the orchestrator squashes the compiler branch into one `feat:` commit (spec §8) and pushes and opens the pull requests (Task 8); they are left at green CI, never merged by the implementer.

## Tasks

### Task 1: The line is a minimum; publish keeps the exact minor

Spec §2, §5, §7 (unit tests in `package.rs` and `packages.rs`, error fixtures).

**Files:**
- Modify: `crates/varyk/src/package.rs`, `crates/varyk/src/packages.rs`, `crates/varyk/src/lib.rs`, `crates/varyk/src/cli.rs`, `crates/varyk/src/diagnostics/codes.rs`, `crates/varyk/tests/errors.rs`, `release-please-config.json`
- Rename: `crates/varyk/tests/fixtures/errors/v0404_std_wrong_minor/` to `v0404_std_newer_than_compiler/`
- Create: `crates/varyk/tests/snapshots/errors__v0404_std_newer_than_compiler.snap`
- Delete: `crates/varyk/tests/fixtures/errors/v0404_std_stale_lock/`, `crates/varyk/tests/snapshots/errors__v0404_std_stale_lock.snap`, `crates/varyk/tests/snapshots/errors__v0404_std_wrong_minor.snap`

- [ ] write tests in `package.rs`, replacing today's minor tests: under the build rule, the compiler's version, an older and a newer patch of its minor, an older minor, an older major (`0.9` under `1.2`), a newer minor, and a newer major, each with and without `^`; only the newer ones are V0404 with the new message and note
- [ ] write tests: the same lines under the publish rule, where only a line on the compiler's minor and no newer than it passes; an older minor gets publish's kept message, a newer line the new one
- [ ] write tests: the stale-lock tests kept under the publish rule and passing under the build rule; a lock whose only older `varyk-std` is on an older minor passes under both
- [ ] keep the old refusals (`~`, `=`, `>=`, `*`, a list, `path`, `git`, `optional`, `package`, no `version`) refused under both rules, and the missing-line and not-used tests as they are
- [ ] add the rule enum and take it in `check_std_dependency` and `check_build_std`
- [ ] replace the minor comparison with the ordered triple comparison: above the compiler is V0404 "this program/package needs varyk X.Y or later" with the note "this compiler is varyk A.B.C; `cargo install varyk` installs the newest" in place of the line to write
- [ ] under the publish rule only, keep the equal-`(major, minor)` check with today's message for a line on an older minor, and run `locked_older`, narrowed to a locked `varyk-std` on the compiler's minor older than the compiler
- [ ] in `check_build_std`, run the resolved-version check only under the publish rule, and rewrite its note to name both causes (a Varyk package of the build on an older minor, a stale lock), keeping `cargo update -p varyk-std`
- [ ] update the doc comments of both functions and of `locked_older`
- [ ] `packages.rs` tests: pass the publish rule to every `check_build_std` call; move the stale-lock test to a compiler at 0.5.1 with a lock at 0.5.0 (give `build_std` the compiler as an argument); add one assertion that the build rule accepts the older resolved `varyk-std`
- [ ] `lib.rs` `check_dependency`: pass the build rule to both `check_std_dependency` calls (the note is Task 2)
- [ ] `cli.rs`: give `load` the rule and pass it to `check_build_std`; `run_publish` passes the publish rule, every other caller the build rule
- [ ] rewrite V0404's comment in `codes.rs` to spec §5's list
- [ ] rename the fixture `v0404_std_wrong_minor` to `v0404_std_newer_than_compiler` with `varyk-std = "999.0"`, and its `package_error_case!` line in `tests/errors.rs`
- [ ] remove the fixture `v0404_std_stale_lock`, its case in `tests/errors.rs`, its snapshot, and its entry in `release-please-config.json`
- [ ] regenerate only `v0404_std_newer_than_compiler`'s snapshot with `INSTA_UPDATE=always`, read it (the new message on `999.0`, the note with `[version]`), and delete the old `wrong_minor` snapshot
- [ ] gate: `cargo test -p varyk` passes without `INSTA_UPDATE`, with every other V0404 snapshot unchanged (`v0404_failing_dependency_wrong_std` included)

### Task 2: A failing package written for an older varyk says so

Spec §2 (last paragraph), §7 (the `lib.rs` unit test).

**Files:**
- Modify: `crates/varyk/src/lib.rs`, `crates/varyk/src/package.rs`

- [ ] write a test in `lib.rs` beside `a_program_logs_and_uses_std_when_a_package_of_its_build_logs`: a `units` written to a `TempDir` with `varyk-std = "0.1"` and a function that does not type-check, used by a program, checked through `Graph::for_tests` and `check_file`; every diagnostic is V0200, none V0404, and each carries the new note beside "in the package `units` 0.1.0, which this build uses"
- [ ] add a `pub(crate)` helper in `package.rs` that gives the `(major, minor)` of a package's `varyk-std` requirement, string or table `version`, through Task 1's requirement parse
- [ ] in `check_dependency`'s failing branch, when that `(major, minor)` is older than the compiler's, add "`<name>` was written for varyk X.Y; a newer release of it may build with this compiler" to each error, after the V0404 re-check, and update the branch's comment
- [ ] gate: `cargo test -p varyk` (the `v0404_failing_dependency_wrong_std` snapshot unchanged: its line is a `path`, with no version)

### Task 3: Every manifest of the build names the compiler's varyk-std

Spec §3, §7 (the `backend/mod.rs` unit test).

**Files:**
- Modify: `crates/varyk/src/backend/mod.rs`, `crates/varyk/src/cli.rs`, `crates/varyk/src/driver/mod.rs`

- [ ] write tests in `backend/mod.rs`, against the inner function given the value: a `varyk-std` line as a string and as a table (with `features` and `registry`) each becomes `"A.B.C"`; a manifest without the line, or without `[dependencies]`, is unchanged
- [ ] rename `with_local_std` to `with_build_std`: replace a `[dependencies]` `varyk-std` entry whole with the `VARYK_STD_PATH` directory when set, else the string `"A.B.C"` of `CARGO_PKG_VERSION` (no `=`), through the inner function
- [ ] switch both callers, `crate_info` in `cli.rs` and `package_crates` in `driver/mod.rs`, and rewrite their doc comments and `with_build_std`'s (a published manifest is never rewritten; a plain Rust crate keeps its own line)
- [ ] gate: `cargo test -p varyk`

### Task 4: End to end, an older-minor package builds; publish refuses it

Spec §7 (the end-to-end test and the two publish cases).

**Files:**
- Modify: `crates/varyk/tests/packages.rs`

- [ ] write a helper that builds, under one `empty_dir`: a stub crate named `varyk-std` at version `0.1.0` with an empty `src/lib.rs`; a library package `units` whose `src/lib.vr` names `Error` (so its line is checked) with `varyk-std = "0.1"`; and a program on the compiler's version (from `env!("CARGO_PKG_VERSION")`) that uses `units` by `path`, with a `.cargo/config.toml` holding a renamed `[patch.crates-io]` entry (`package = "varyk-std"`) pointing at the stub
- [ ] write test: the program checks, builds, and runs with the expected output, and the generated manifests at `target/varyk/<name>/Cargo.toml` and `target/varyk/packages/units-0.1.0/Cargo.toml` both name the `varyk-std` path `common::std_path()` gives
- [ ] write test: `varyk publish --assemble-only` inside `units` is V0404 with publish's message ("`varyk-std` must be version ..., the version of this compiler")
- [ ] write test: `varyk publish --assemble-only` inside the program is V0404 for the older `varyk-std` its build resolves (`0.1.0`)
- [ ] gate: `cargo test -p varyk --test packages`

### Task 5: The documents

Spec §6.

**Files:**
- Modify: `docs/language.md`, `docs/design.md`, `docs/roadmap.md`, `docs/specs/2026-09-30-milestone-5a-design.md`, `docs/specs/2026-10-02-milestone-5b2-design.md`; `README.md` and `CONTRIBUTING.md` only if they state the old rule

- [ ] `docs/language.md` "Packages", the `varyk-std` paragraph (~3171): the minimum rule, publish's exact minor, lock, and resolved-version checks, "To upgrade, run `cargo install varyk`", and that until the line is on the compiler's minor and the lock updated each build asks the crates.io index for the compiler's `varyk-std` again, with `cargo update -p varyk-std` and, when the lock holds two, `cargo update -p varyk-std@X.Y.Z` (spec §3)
- [ ] `docs/language.md` "Packages and the build" (~2980): a failing package written for an older varyk gets the new note instead of V0404; the resolved-`varyk-std` promise holds under `varyk publish` only, and the build uses the compiler's own `varyk-std`
- [ ] `docs/language.md` V0404 row (~3286): spec §5's list
- [ ] `docs/design.md` "Versioning" row: the line is the minimum varyk, a newer one refused by `varyk check`, the minor and lock checked by `varyk publish`
- [ ] add a row to the decisions table of each of the 5a and 5b2 specs pointing to `2026-10-11-std-minimum-design.md`
- [ ] `docs/roadmap.md`: a checked line under "Milestone 6" for a package's `varyk-std` line as its minimum varyk
- [ ] read `README.md` and `CONTRIBUTING.md` against the change and edit only a sentence that states the old rule or tells authors to move their line with every release (at the time of writing, neither does)
- [ ] gate: `cargo test -p varyk` (the language-reference and examples tests read `docs/language.md` and `examples/`)

### Task 6: The full gate

**Files:**
- none, unless the gate finds something

- [ ] run `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, and `CARGO_TERM_COLOR=always cargo test --workspace` from a clean tree, without `INSTA_UPDATE`
- [ ] if a snapshot moved, confirm it belongs to a test meant to change (only `v0404_std_newer_than_compiler` is new), read it, and rerun the gate without the variable; `git status` shows no orphan `.snap` and no other file changed
- [ ] confirm `release-please-config.json` names no removed fixture and no manifest of this change needs an entry
- [ ] run `rustup run 1.88 cargo build --workspace --locked`
- [ ] gate: all of the above pass; nothing is pushed

### Task 7: The official packages' documents

Spec §6 (the last bullet), §4. In each of `../varyk-http`, `../varyk-sql`, and `../varyk-mongo`, after Task 6.

**Files:**
- Modify, in each of the three repositories: `README.md` (the "Versions" section), `.github/workflows/latest-varyk.yml` (the header comment only)

- [ ] branch from an up-to-date `main` in each repository
- [ ] rewrite the "Versions" prose: from the newest release on, the package's `varyk-std` line is the oldest varyk it builds with, and a program that uses it builds under any later varyk, so a minor varyk release no longer needs a package release at once; one that moves the line still follows, when convenient, since `varyk publish` of a library that uses the package wants the line on its own minor (spec §4); earlier releases were each written for the one varyk the table names; keep varyk-mongo's 0.8.1 sentence
- [ ] in the table, only the newest row reads as a minimum ("0.8 or later"; varyk-mongo's "0.8.1 or later"), the release varyk 0.9.0 reads under the rule; older rows keep their one varyk, since they predate the rule and the language changed between those minors (varyk-sql's `feat!: require varyk 0.7`), so "or later" would overpromise for them
- [ ] rewrite the `latest-varyk.yml` comment's sentence that a new minor varyk needs a release moving `varyk-std`: the job checks the package still builds and passes with the newest varyk, so a red run says a fix is due
- [ ] commit once, locally, with a `docs:` subject (no attribution, no `<` or `>`)
- [ ] gate: in each repository, `git diff --stat main` names only `README.md` and `.github/workflows/latest-varyk.yml`

### Task 8: Pull requests, after the branch's review loops

Run by the orchestrator, not a task subagent, once the review loops on the finished branches are done. Spec §8.

**Files:**
- none

- [ ] squash `std-minimum` into one `feat:` commit; grep it and each package commit for attribution lines and `<` or `>` in subjects
- [ ] push `std-minimum` over HTTPS and open the compiler pull request under the `feat:` title
- [ ] then push each package branch over HTTPS and open its `docs:` pull request; its body links the compiler pull request and says to merge it only after the varyk release that ships this rule (0.9.0)
- [ ] gate: every pull request's CI (the package repositories' commit-message check included) is green; none is merged
