# Typed serialize and `varyk add mongo` Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The compiler-side changes the MongoDB package `varyk-mongo` needs: varyk-std's `Time`, `Uuid`, and `Bytes` keep their type when a serializer is not human-readable (released as varyk 0.8.1, before varyk-mongo 0.1.0), the documents of that round, and later the `varyk add mongo` shorthand (merged only after varyk-mongo 0.1.0 is on crates.io).

**Spec:** section 6 of `Varyk-Lang/varyk-mongo`'s `docs/specs/2026-10-09-varyk-mongo-design.md`, "spec §n". Read §6 (and §3.1, which consumes 6.1) before any task. The varyk-sql 0.4 spec (`Varyk-Lang/varyk-sql` `docs/specs/2026-10-09-varyk-sql-0.4-design.md` §4) asks for one roadmap line here too.

## Overview

Three tasks on two branches. Tasks 1 and 2 are one pull request on `fix/typed-serialize` (a `fix:` commit and `docs:` commits, rebase-merged), released as 0.8.1. Task 3 is a second pull request on `fix/varyk-add-mongo`, opened as a draft and merged only after varyk-mongo 0.1.0 is published, so `varyk add mongo` never names a crate someone else could register first.

## Context

Line numbers are approximate; paths are in this repository.

- `crates/varyk-std/src/lib.rs` 6-28: public modules and re-exports (`pub use serde` 24).
- `crates/varyk-std/src/timestamp.rs` 190-194: `impl Serialize for Time` (`collect_str`); `to_unix_micros` 101.
- `crates/varyk-std/src/ids.rs` 95-99: `impl Serialize for Uuid` (`collect_str`); `as_bytes` 59.
- `crates/varyk-std/src/buffer.rs` 93-97: `impl Serialize for Bytes` (base64 through `collect_str`); unit tests 131 on.
- `crates/varyk-std/Cargo.toml` 16-17: `serde` with `derive`, `serde_json`.
- `docs/language.md` 2717: "A facade for a package"; 3041-3042 (the `varyk add` command table), 3140, 3145-3162 (the shorthands and the `not_added` note).
- `crates/varyk/src/cli.rs` 136-141 (the `Add` doc comment, naming only `sql`), 801 (`SHORTHANDS`), tests 1030-1120.
- `crates/varyk/src/resolve/package_items.rs` 29-40 (`not_added` and its doc comment naming `http` and `sql`).
- `docs/roadmap.md` 156 (5b3's "each database's own placeholders, passed through"), 185 ("After 5b4, `varyk-mongo` and `varyk-redis` are the next packages."), "## Unscheduled" 216.
- `docs/open-questions.md`: a flat bullet list, newest at the end.
- `README.md` 24-32 (the packages sentence), 197 (the command table's `varyk add sql` line), 236-263 (Status).

## Development Approach

- TDD: a failing unit test in the module first, then the code, then the gate.
- Task 1's test needs a serializer that is not human-readable: add `serde_test` as a dev-dependency of varyk-std and use its `Configure::compact()` with `assert_ser_tokens` (and `readable()` for the JSON-like path); no hand-written serializer. JSON's output is checked with `serde_json::to_string` as the existing tests do.
- Follow each file's existing comment style; the new module's doc comment says the forms are one-way (`Deserialize` reads text and bytes only, so MessagePack or bincode would not read them back) and that they are the contract a database package's serializer matches on.
- **The gate**, from the repository root, as AGENTS.md and CI: `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `CARGO_TERM_COLOR=always cargo test --workspace`, and `rustup run 1.85 cargo build --workspace` (no let-chains).
- Commits: `fix:` for code, `docs:` for documents, one concern each; no `<` or `>` in subjects; no attribution lines. Never on `main`; push over HTTPS with `gh`'s credential helper; stop at green CI; the maintainer merges.
- Not needed: any change to `Deserialize`, to `Value`, or to the compiler for Task 1; a CHANGELOG edit (release-please writes it).

## Tasks

### Task 1: `Time`, `Uuid`, and `Bytes` keep their type outside JSON

Spec §6.1.

**Files:**
- Modify: `crates/varyk-std/src/lib.rs`, `crates/varyk-std/src/timestamp.rs`, `crates/varyk-std/src/ids.rs`, `crates/varyk-std/src/buffer.rs`, `crates/varyk-std/Cargo.toml`, `Cargo.lock`, `docs/language.md`
- Create: `crates/varyk-std/src/serde_names.rs`

- [ ] add the public module `serde_names` with the constants `TIME = "$__varyk_std_Time"` and `UUID = "$__varyk_std_Uuid"`, documented as the contract and as one-way
- [ ] make `Time`'s `Serialize` call `serialize_newtype_struct(TIME, micros)` with its `i64` Unix microseconds when the serializer is not human-readable, and keep `collect_str` otherwise
- [ ] make `Uuid`'s `Serialize` call `serialize_newtype_struct(UUID, ..)` when not human-readable, around a private wrapper struct whose `Serialize` calls `serialize_bytes` on the 16 bytes (a `&[u8; 16]` would serialize as a tuple)
- [ ] make `Bytes`' `Serialize` call `serialize_bytes` when not human-readable
- [ ] add `serde_test` as a dev-dependency of varyk-std
- [ ] write tests: each of the three serializes as today under `readable()` and as JSON (byte for byte), and in the forms above under `compact()`
- [ ] add one sentence to `docs/language.md` "A facade for a package" naming `varyk_std::serde_names` for facade authors whose serializer is not human-readable
- [ ] commit as `fix: Time, Uuid, and Bytes keep their type outside JSON`
- [ ] gate: the full gate, with `rustup run 1.85 cargo test -p varyk-std` in place of the 1.85 build (a build compiles no dev-dependency, and CI tests on 1.85)

### Task 2: The documents of this round

Spec §6.3; varyk-sql 0.4 spec §4.

**Files:**
- Modify: `docs/roadmap.md`, `docs/open-questions.md`

- [ ] roadmap 156: add that from varyk-sql 0.4 the placeholders are `$1`, `$2`, … on every database (keep the 5b3 record)
- [ ] roadmap 185: turn the sentence into an unchecked item `- [ ] varyk-mongo` linking `https://github.com/Varyk-Lang/varyk-mongo/blob/main/docs/specs/2026-10-09-varyk-mongo-design.md`, keeping "`varyk-redis` is next" as a sentence after it, and keep a lead-in line before it (such as "After 5b4, the next packages:") so the item does not join the "Its own work" list above
- [ ] roadmap Unscheduled: add a new paragraph for a document literal in the language, with values written inline, for MongoDB and JSON
- [ ] open-questions: add the document literal (its type, and whether JSON shares it), a prefix search from input in varyk-mongo, and a transaction call that retries (needs a function as a facade argument)
- [ ] commit as `docs: varyk-mongo on the roadmap, and its open questions`
- [ ] in a separate commit, roadmap Unscheduled: add a new paragraph for "`{name}` capture in `println!` and `format!`, the Rust 1.58 form, inside the `{}` syntax the language has", as `docs: name capture in format strings on the roadmap`
- [ ] gate: the full gate (documents only; the gate confirms nothing else moved)

### Task 3: `varyk add mongo` (second pull request, merged after varyk-mongo 0.1.0)

Spec §6.2. On a new branch `fix/varyk-add-mongo` started from `fix/typed-serialize` after Task 2 (it checks the roadmap item Task 2 adds, and both edit `docs/language.md`); its draft pull request targets `main` and is rebased onto `main` once the first pull request is merged.

**Files:**
- Modify: `crates/varyk/src/cli.rs`, `crates/varyk/src/resolve/package_items.rs`, `crates/varyk/tests/errors.rs`, `docs/language.md`, `README.md`, `docs/roadmap.md`
- Create: `crates/varyk/tests/fixtures/errors/v0101_mongo_not_added/` (`Cargo.toml`, `src/main.vr`) and its snapshot `errors__v0101_mongo_not_added.snap`

- [ ] add `("mongo", "varyk-mongo")` to `SHORTHANDS` and a test following the existing `sql` shorthand tests
- [ ] update the `Add` doc comment to name all three shorthands
- [ ] add `"mongo" => "varyk-mongo"` to `not_added` and update its doc comment; add the fixture `v0101_mongo_not_added` following `v0101_sql_not_added`, its `package_error_case!` line in `tests/errors.rs` (~239), and its snapshot (regenerate only that test with `INSTA_UPDATE=always`, read the `.snap`)
- [ ] `docs/language.md`: name `mongo` beside `http` and `sql` in the command table (3041-3042) and in the shorthand paragraphs (3140, 3145-3162)
- [ ] README: name `mongo` beside `sql` in the command table (197), add varyk-mongo (`varyk add mongo`) to the packages sentence (24-32) and a short paragraph at the top of Status saying it is released
- [ ] roadmap: check the `- [ ] varyk-mongo` item
- [ ] commit as `fix: varyk add mongo adds the MongoDB package`
- [ ] gate: the full gate; open the pull request as a draft whose description says to merge only after varyk-mongo 0.1.0 is on crates.io
