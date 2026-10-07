# Milestone 5c Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add the built-in types `Time`, `Uuid`, and `Bytes` to the compiler and `varyk-std`: their calls, their reach into comparison, printing, `parse`, JSON, `env`, `.rs` facades, trailing values, and route parameters, so the users API stores `created_at: Time` and not a string.

**Architecture:** `varyk-std` gains the three types, hand-written serde, `Parse` and `FromStr`, and three `Value` variants (task 1). The compiler gives each type its own `Ty` variant and walks it through the hand-written lists `Error` went through: resolve, the builtins table, the checks, derives, borrow analysis, interop, and the backend (tasks 2 to 7). Nothing new drives them: no new mechanism, no new code, no new HIR node. The stub `varyk-http` implements its `Plain` trait for `Time` and `Uuid`, which is all route parameters need.

**Tech Stack:** Rust stable, edition 2024, MSRV 1.85. New dependencies of `varyk-std` only: `time` (`>= 0.3.45`), `uuid` (`>= 1.20`, features `v4` and `v7`), `bytes` (`>= 1.10.1`), and `base64`. None in `varyk` or `varyk-syntax`.

**Spec:** `docs/specs/2026-10-07-milestone-5c-design.md` ("spec 2.1" refers to its sections; "M5b4 §n" and the like to earlier specs). Read the spec before any task; it is the authority where this plan is brief.

## Global Constraints

- `AGENTS.md` applies in full: the gate after every task, MSRV 1.85 with no let-chains, minimum viable, fail loudly, stable codes, snapshots reviewed, conventional commits, safe by default, no hidden allocation. Before any push, `rustup run 1.85 cargo build --workspace` passes and `CARGO_TERM_COLOR=always cargo test --workspace` passes.
- No new diagnostic code. Every new refusal reuses the code spec 8 names: V0100, V0101, V0108, V0113, V0200, V0203, V0210, V0218, V0219, and V0304 (its `.clone()` fix-it only). Each amended case updates the code's `codes.rs` comment and its `docs/language.md` row in the task that adds it. If a case none of them covers turns up, stop and ask; spec 8's last paragraph then applies.
- Spec 2.6 is the cut list; implement nothing on it (no `Date`, `Duration`, zones, `Uuid` ordering, `Bytes` indexing or `Vec<u8>` conversion, `Bytes` in `env`, `Time` or `Bytes` as a map key, a raw-body handler parameter).
- No `unwrap`, `expect`, or other crash-on-absence call in non-test code, fixtures' Rust included. The one documented panic is the `uuid` crate's, when the operating system's random source fails under `Uuid::new()`, `v7()`, or `v4()`. The clock never panics: a version 7 id is `Uuid::new_v7(Timestamp::from_unix(&CONTEXT, secs, nanos))` from `Time::now()`'s reading, a time before 1970 taken as 0, never `uuid`'s `now_v7`.
- Generated Rust exactly: `::varyk_std::Time`, `::varyk_std::Uuid`, `::varyk_std::Bytes`; associated calls in full (`::varyk_std::Time::now()`); methods as `receiver.name(args)`; `parse` as `::varyk_std::parse::<::varyk_std::Time>(..)`; a trailing value as `::varyk_std::Value::from(t)`, a `Bytes` lent (`&b`, or `b` when already a reference), an `Option<Bytes>` through `.as_ref()`. The compiler still inserts exactly the two allocations `AGENTS.md` names.
- Dependencies exactly: `time` `>= 0.3.45` (which builds on Rust 1.83), `uuid` `>= 1.20` with `v4` and `v7`, `bytes` `>= 1.10.1` (the first whose `from_owner` does not leak on `to_vec`), `base64`. The workspace resolver stays version 2; the committed `Cargo.lock` holds `time` 0.3.45 and `uuid` 1.26 and dependencies that build on 1.85. `bytes` is the one public dependency; `time` and `uuid` never appear in `varyk-std`'s public API.
- `varyk-std` goes to 0.8.0 through release-please from the breaking commit; no version is edited by hand. Every new manifest with a `varyk-std` line gets an `extra-files` entry in `release-please-config.json` in the task that adds it.
- Work on the branch `feat/milestone-5c`: the `varyk-sql` 0.3 and `varyk-http` 0.2 plans build against it by that name, from a local checkout (and `varyk-sql`'s CI from a checkout of the pushed branch), and need tasks 1 to 7 on it; push it once task 7's gate passes. Its compiler and `varyk-std` stay numbered 0.7.0 until the release, so those plans keep their `varyk-std = "0.7"` lines until 0.8.0 is on crates.io.
- Commits: one conventional commit per task on the branch; the pull request is squashed as `feat!:` with a `BREAKING CHANGE:` footer naming the three reserved type names and the three new `Value` variants. No `<` or `>` in subjects or the footer (`Option of Bytes`, not the bracketed form). No attribution lines in commits or the pull request.

## Review Focus

Inputs the spec implies but no listed test would otherwise exercise, most likely to bite first. Each has a test in the task that owns the code.

1. An offset that moves a time across the range's ends after conversion to UTC (`9999-12-31T23:30:00-01:00`, `0000-01-01T00:30:00+01:00`) is an `Err` naming the text, while the same instants written in UTC inside the range read. Task 1.
2. Admitting `Time` to the orderings must not admit it to arithmetic: `t + u` and `t - u` stay V0200, since the gate at `types/check.rs` ~1331 serves both. Task 3.
3. A `HashMap<Uuid, i64>` is a valid map, but inside a struct `json::stringify` reaches it stays V0210 with the map-key note, while `HashMap<string, Uuid>` goes through JSON. Task 4.
4. A `Bytes` the function only borrows (returned from a borrowed parameter, or bound by a `match` on a borrowed imported enum) stored in a field or passed to an owned `varyk_std::Bytes` parameter is V0304 with the `.clone()` fix-it, and the fixed program builds. Task 6.
5. An `Option<Bytes>` trailing value that is a borrowed parameter builds through `.as_ref()` without moving it; an `if` giving `Some(b)` or `None` builds and gives `b` away, as `Some(name)` does for a string (V0305 if used after, V0304 if borrowed). Task 7.

---

## Overview

Eight tasks. Task 1 is `varyk-std` alone and builds on its own. Task 2 adds the three types and every call of spec 2.1 to 2.3 to the compiler, with the reference section that the language-reference test needs for the new names. Task 3 adds comparison, printing, `parse`, `.clone()`, `sort`, `contains`, and the `Uuid` map key. Task 4 opens JSON and `env`. Task 5 opens `.rs` facades, imported enums included, with a fixture package. Task 6 proves `Bytes` ownership with the soundness templates, which use task 5's facade forms. Task 7 adds trailing values and route parameters, with the stub's `Plain` and `http_user`. Task 8 adds `examples/records.vr` and the remaining documents, and checks the release.

## Context

Line numbers are approximate. Paths are under `crates/varyk/src/` unless shown otherwise.

- Types: `types.rs` (`Ty` 9 with `Error` 29, `BUILTIN_TYPE_NAMES` 45, `is_copy` 114, `copy_in_rust` 123, `is_compound` 135, `has_error` 152, `is_map_key` 205).
- Builtins: `builtins.rs` (`Owner` 11, where `Owner::Time` 28 is the `time` module; `Owner::name` 46; `Owner::of` 67; `Shape` 157 and `Shape::ty` ~285; `ElementRule` 342 with `accepts` and `wanted`; `row` 398 and `borrowed_row` 432; `TABLE` 492-742 with `Error`'s rows ~634 and `time::sleep` ~701; `Builtin::uses_std` 764; tests ~854).
- Resolve: `resolve/mod.rs` (`resolve_type` 973, the `HashMap` key check ~1021, `Error` mapped ~1078, `std_type_taken` 1407); `resolve/signatures.rs` (`sig` 28 with `names_std`, `param` 115, `ret` 129, `value` 149, `named` 185 with the `use`-line note 188, `names_std` 357); `resolve/package_items.rs` (exhaustive matches over `Ty` ~89 and ~711); `resolve/imports.rs` (variant payloads through `Mapper::field` ~312).
- Interop: `interop/signatures.rs` (`RustTy` 21, `text` 81, `StdItem` 153, `std_item` 171, `map_reference` 376, `map_value` 416 mapping `varyk_std::Error` ~442); `interop/items.rs` (enum variants ~136-185, `unmapped_at_import_time` ~750, `unmapped_payload_reason` ~759).
- Checks: `types/check.rs` (`uses_std` 49 and ~214, `names_error` ~236-264, `binary` ~1270 with `==` ~1296 and the numeric gate ~1331, `assert_call` ~1573 with `show` ~1622, `arguments` ~1650 with V0218 ~1720, `format_args` ~1964 with its V0203 arms ~2035-2075, `is_value_type` ~2508); `types/check/values.rs` (`path_owner` 104, "no function" messages ~335-360); `types/check/methods.rs` (method not found ~122, `derived_clone` 324, `clone_call` ~350, `parsed_type` ~455); `types/check/routes.rs` (`PARAMETERS` 116, `is_query` 184, `bindings` ~863 with the path parameter rule ~890, V0219 ~1004).
- Derives: `types/derives.rs` (`blocking` ~214, `ty_name` 317, notes ~470-545 with `Part::Other` ~524, `ENV_FIELDS` 546, `env_readable` 553, `convertible` 629, `walk` ~648, unit tests ~1048).
- Borrow: `borrow.rs` (`with_trailing` 219, trailing values ~1103, `mixed` ~2224); `borrow/slots.rs` (`clone_fix_it` 150, `string` only); its callers in `borrow/moves.rs` ~202 and `borrow/returns.rs` ~470.
- Backend: `backend/rust.rs` (`rust_type` 622, `param_type` 663, route bindings ~397); `backend/rust_expr.rs` (`Need` 52 with `OptStr` 63, the block-like `OptStr` case ~213, `OptStr` bridged ~392, `values` ~878, `parse` ~1047, `callee` ~1322 with the in-full owner list ~1373).
- `varyk-std`: `crates/varyk-std/Cargo.toml`; `src/lib.rs` (module list and re-exports); `src/parse.rs` (sealed `Parse`); `src/value.rs` (`Value` and its `From` impls); `src/env.rs` (the per-variable `Value` reader ~172, `deserialize_any` ~186, `number_error` ~160); `src/json.rs`; `src/error.rs`; `src/time.rs` (the `time` module, `sleep` only).
- Tests: `tests/codegen.rs` (`generate_path` 30, `a_single_file_naming_error_only_in_a_signature_depends_on_varyk_std` 110, `trailing_values_to_a_vec_of_values` 361, `error_status` ~841); `tests/examples.rs` (`assert_runs` 26, `run_readings` 161, `run_table_rows` ~514 running a codegen fixture, `store_user_runs_and_store_s_test_passes` 1483, `the_varyk_http_stub_s_test_passes` ~1528, `http_user_answers_requests_through_its_routes` 1578); `tests/errors.rs` (`error_case!` 75, `http_error_case!` 155, V0113 cases ~313, V0219 cases ~453); `tests/soundness.rs` (`PRELUDE` 45, `EXT_RS` 442, `MUST_PASS` 1250, `MUST_REJECT` 1539, `MUST_REJECT_ITEMS` 2209); `tests/reference.rs` (type names 57, table calls 64, codes 71); `types/check/tests.rs` (`every_builtin_type_name_resolves_as_a_type` 132, `mod packages` 5373, `mod http` 5562); `test_packages.rs` (`add` 109, `add_dir` 115, `varyk_http` 188); `interop/tests.rs` (`std_item` ~1678, `varyk_std_error_maps_by_its_full_path` 1711).
- Fixtures: `tests/fixtures/packages/varyk-http/src/server.rs` (`Plain` 67); `tests/fixtures/packages/http_user/src/main.vr`; `tests/fixtures/packages/store/src/kv.rs` (`scalar` ~152, an exhaustive `match` on `varyk_std::Value`); `tests/fixtures/codegen/trailing_values/`.
- Docs: `docs/language.md` (Types 293, Copy types ~391, the `parse` row ~493, Printing 1141, Functions and parameters 1159, JSON 1627, Configuration 1729, Strings 1807, "Not in milestone 5b4" 1835 with its 5c line ~1881, HTTP 1928 with route parameters ~2033-2070, Calling Rust 2227 with the `Value` row ~2276, Rust enums 2450, A facade for a package 2539, error codes 3026 with rows ~3038-3081); `docs/roadmap.md` (milestone 5 intro ~106, 5c ~187, Unscheduled TOML line ~214); `docs/design.md` (allocations ~35 and ~129); `docs/open-questions.md`; `README.md` (~269); `AGENTS.md` (`varyk-std`'s list 13, the `varyk-http` rule ~63-70); `release-please-config.json`.

## Development Approach

- TDD per task: a failing test first, the minimal code, then the task's gate. Unit tests beside the code; fixtures, snapshots, and integration tests under `crates/varyk/tests/`.
- A built-in type follows `Error` through every list (spec 7): `Ty::Error` in `types.rs`, `Owner::Error` and its rows in `builtins.rs`, its arm in `path_owner`, `resolve_type`, `std_type_taken`, `ty_name`, `rust_type`, and the in-full owner list of `callee`. Let rustc's exhaustiveness errors find the remaining matches over `Ty` and `RustTy`.
- `Bytes` is compound as `Error` is (spec 7.4): the struct rules give it a borrowed parameter, an owned field, a move on `let`, and a borrowed return; the rules that name `Ty::String` for `&str` do not take it.
- Each generated-Rust check pairs an `insta` snapshot of a fixture under `tests/fixtures/codegen/` with an `assert_runs` test in `tests/examples.rs`, following `tables` (`run_table_rows` ~514), so rustc builds what `check` accepts. Route programs use the `http_user` package snapshot instead (M5b4 plan, Development Approach); route type tests use `varyk_http()` and fixtures `http_error_case!`.
- An `Option<Bytes>` trailing value follows `Need::OptStr` (`as_deref`) as a new `Need::OptRef` (`as_ref`).
- `varyk-std`'s serde follows the derived code's convention: written against `serde` as re-exported, with `deserialize_str` and a visitor; raw-bytes cases are tested with serde's own value deserializers, not `varyk-sql`.
- Each task updates the parts of `docs/language.md` it changes, since `tests/reference.rs` needs every new type name and table call in a code span; task 8 does the rest.
- A message whose list of types grows (V0100's copied types, V0101's key types, `sort`'s element types, V0203's printed forms, `ENV_FIELDS`, V0218's value types, V0219's parameter types) changes existing snapshots, such as `v0100_clone_number`, `v0101_hash_map_float_key`, `v0200_sort_floats`, `v0203_print_struct`, the `v0210_env_*` ones, `v0218_*`, and `v0219_struct_path_parameter`, and the type tests that match that text: the task that grows the list regenerates only those snapshots with `INSTA_UPDATE=always`, reads each, and updates the tests' strings.
- Not needed: any `varyk-sql` or `varyk-http` code beyond the stub's `Plain` lines (each package has its own spec and repository); a new HIR node, driver change, or `packages.rs` change; cargo features to build the new dependencies only when used (spec 11 leaves it open); `Default` for `Uuid`; `Display`, `Hash`, or ordering for `Bytes`; ordering for `Uuid`; a `bytes` re-export; any change to `Value`'s existing variants.

## Tasks

### Task 1: `Time`, `Uuid`, and `Bytes` in `varyk-std`

Spec 2.1 to 2.3 (behaviour), 3, and 5: the three types, their text forms, serde, `Parse` and `FromStr`, the `Value` variants, and the dependencies pinned for 1.85.

**Files:**
- Create: `crates/varyk-std/src/timestamp.rs` (`Time`), `crates/varyk-std/src/ids.rs` (`Uuid`), `crates/varyk-std/src/buffer.rs` (`Bytes`); named apart from the existing `time` module and from the `time`, `uuid`, and `bytes` crates
- Modify: `crates/varyk-std/Cargo.toml` (dependencies; description gains time, ids, and bytes), `Cargo.lock`, `crates/varyk-std/src/lib.rs`, `crates/varyk-std/src/parse.rs`, `crates/varyk-std/src/value.rs`, `crates/varyk-std/src/env.rs`, `crates/varyk/tests/fixtures/packages/store/src/kv.rs`
- Test: unit tests in the three new modules, `value.rs`, and `env.rs`

**Interfaces:**
- Produces `varyk_std::Time` (an `i64` of microseconds since 1970-01-01T00:00:00Z; `Copy`, `Clone`, `Debug`, `PartialEq`, `Eq`, `PartialOrd`, `Ord`, `Display`, `FromStr` with `Err = Error`, `Parse`, `Serialize`, `Deserialize`): `now() -> Time`, `from_iso(&str)`, `from_unix(i64)`, `from_unix_micros(i64)`, each `-> Result<Time, Error>`; taking `self` by value, as clippy's `wrong_self_convention` asks of a Copy type: `to_iso(self) -> String`, `to_unix(self) -> i64`, `to_unix_micros(self) -> i64`, `add_seconds(self, i64) -> Result<Time, Error>`, `seconds_since(self, Time) -> i64`.
- Produces `varyk_std::Uuid` (16 bytes; `Copy`, `Clone`, `Debug`, `PartialEq`, `Eq`, `Hash`, `Display`, `FromStr`, `Parse`, serde): `new()`, `v7()`, `v4()`, `from_bytes([u8; 16])`, `as_bytes(&self) -> &[u8; 16]`.
- Produces `varyk_std::Bytes` (a `bytes::Bytes`; `Clone`, `Debug`, `PartialEq`, `Eq`, serde; not `Copy`, no `Display`): `from_text(&str) -> Bytes`, `from_base64(&str) -> Result<Bytes, Error>`, `to_text(&self) -> Result<String, Error>`, `to_base64(&self) -> String`, `len(&self) -> usize`, `is_empty(&self) -> bool`; `From<bytes::Bytes> for Bytes`, `From<Bytes> for bytes::Bytes`, `AsRef<[u8]>`.
- Produces `Value::Time(Time)`, `Value::Uuid(Uuid)`, `Value::Bytes(Bytes)` with `From<Time>`, `From<Uuid>`, `From<&Bytes>`, `From<Option<Time>>`, `From<Option<Uuid>>`, `From<Option<&Bytes>>`.

- [ ] Add the four dependencies to `varyk-std` (`time` with the features RFC 3339 parsing and formatting need, `uuid` with `v4` and `v7`, `bytes`, and `base64` 0.22), pin `Cargo.lock` to `time` 0.3.45 and a `uuid` 1.26 release with `cargo update --precise`, and check `rustup run 1.85 cargo build -p varyk-std`
- [ ] Write `timestamp.rs` tests for reading: offsets converted to UTC, nine fraction digits cut to six, lowercase `t` and `z`, the year 0000 and 9999 bounds before and after an offset (Review Focus 1), and a leap second, a space for `T`, ten fraction digits, the compact form, and a week date refused with spec 2.1's message
- [ ] Write `timestamp.rs` tests for writing (no fraction; a fraction with trailing zeros dropped), `from_unix`, `from_unix_micros`, and `add_seconds` at both ends of the range, `to_unix` rounding down before 1970, `seconds_since` rounding toward zero, and `now` cut to the microsecond
- [ ] Implement `Time`: check spec 2.1's shape by hand before `time`'s RFC 3339 parser, convert to UTC, cut past six digits, refuse outside 0000-01-01T00:00:00Z to 9999-12-31T23:59:59.999999Z; `Display` and `Debug` both write the written form (spec 5); every range error names the text or the number; `now` reads `SystemTime`, a clock before 1970 giving a negative time and one outside the range its nearest end
- [ ] Write `ids.rs` tests: versions of `new`, `v7`, and `v4`; ids made in a row by `v7` ordered; two made from a clock reading before 1970 ordered; only the 36-character hyphenated form parses (either case; simple, braced, and URN refused); written lowercase
- [ ] Implement `Uuid` with one `static` `Mutex<ContextV7>` and the timestamp from `Time::now()`'s reading (Global Constraints), through one private helper that makes a version 7 id from a given reading, which `new` and `v7` call and the before-1970 test calls directly, `parse` checking the length before the crate's parser, `Display` and `Debug` both writing the lowercase hyphenated form (spec 5), and `#[allow(clippy::new_without_default)]` on `new` with a comment that a default id would hide a random one
- [ ] Write `buffer.rs` tests: strict base64 both ways (standard alphabet with padding; missing padding, the URL alphabet, and a stray character refused), `to_text` on bytes that are not UTF-8, `len`, `is_empty`, and a clone sharing the buffer
- [ ] Implement `Bytes`, every one `varyk-std` builds from a `Vec<u8>` or receives as a `bytes::Bytes` made with `bytes::Bytes::from_owner`, and `Debug` writing base64
- [ ] Write serde tests through `json::parse` and `json::stringify` and serde's value deserializers: each type both ways, a JSON number refused as a `Time`, `Bytes` from raw bytes, a `Uuid` from 16 raw bytes and not from 15
- [ ] Implement serde by hand: each asks `deserialize_str`; `Time` and `Uuid` serialize their written form and read `visit_str`; `Uuid` also `visit_bytes` and `visit_byte_buf` of exactly 16; `Bytes` serializes base64 and reads `visit_str`, `visit_bytes`, and `visit_byte_buf` (the last without copying)
- [ ] Write the `env` test: a `Time` field and an `Option<Uuid>` field read, and a bad `Time` gives an error naming its variable; make `env`'s per-variable reader put `` `NAME`: `` before the message a field's own deserializer gives
- [ ] Add `Parse` and `FromStr` for `Time` and `Uuid` (sealed as today) and widen `Parse`'s doc comment
- [ ] Add the `Value` variants and `From` impls with tests for each type and each `Option`, the doc saying `From<&Bytes>` adds to a reference count and allocates nothing
- [ ] Export the three types from `lib.rs`
- [ ] Give `store`'s `scalar` (`kv.rs` ~152) arms for the three new variants (the written form as a JSON string, `Bytes` as its base64), so the fixture builds against the new `Value`
- [ ] Run the gate: `cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`, then `rustup run 1.85 cargo build --workspace`

### Task 2: The three types in the compiler, and their calls

Spec 2 (reserved names), 2.1 to 2.3 (the call tables), 4, 7.1, 7.2 (resolve), and 7.5: the types resolve, are reserved, and every call of the three tables types and compiles. Comparison, printing, and the rest come in task 3.

**Files:**
- Modify: `crates/varyk/src/types.rs`, `crates/varyk/src/builtins.rs`, `crates/varyk/src/resolve/mod.rs`, `crates/varyk/src/resolve/package_items.rs`, `crates/varyk/src/types/check.rs` (`names_error`), `crates/varyk/src/types/check/values.rs`, `crates/varyk/src/types/derives.rs` (`ty_name`, `walk`), `crates/varyk/src/backend/rust.rs`, `crates/varyk/src/backend/rust_expr.rs` (`callee`), `crates/varyk/src/diagnostics/codes.rs` (V0113 comment), `docs/language.md`
- Create: `crates/varyk/tests/fixtures/codegen/time_ids_bytes/main.vr`, `crates/varyk/tests/fixtures/errors/v0113_struct_named_time/`, `crates/varyk/tests/fixtures/errors/v0113_rs_enum_named_bytes/` (with `ext.rs`)
- Test: `builtins.rs` tests, `resolve/tests.rs`, `types/check/tests.rs`, `tests/codegen.rs`, `tests/examples.rs`, `tests/errors.rs`

**Interfaces:**
- Consumes: task 1's API.
- Produces: `Ty::Time`, `Ty::Uuid`, `Ty::Bytes` (unit variants); `Time` and `Uuid` in `Ty::is_copy`, `Bytes` in `Ty::is_compound`; `Ty::has_error` renamed `Ty::has_std_type` and true for the three, with `names_error` renamed `names_std_type`; `Owner::TimeType` (named `Time`; `Owner::Time` stays the `time` module), `Owner::Uuid`, `Owner::Bytes`, mapped by `Owner::of` and in `uses_std`; `Shape::{I64, Time, Uuid, Bytes, ResultOfTime, ResultOfBytes, ResultOfString}`; one row per call of spec 2.1 to 2.3 (read `string` parameters as `ReadString`, receivers `Reads`); `walk` refusing the three as `Part::Other` until task 4.

- [ ] Write `builtins.rs` tests: each owner's names in order, each row's receiver, parameters, and result, and `uses_std` for each
- [ ] Write type tests: every call of the three tables types as the table says, on a local, a field, and a parameter receiver; a call the type lacks is V0100 listing its calls; a wrong argument type is V0200 and a wrong count V0201; a `Time` where a `Uuid` is expected is V0200
- [ ] Write resolve tests: the three names resolve with no declaration; a struct, enum, module, `use`, or `.rs` struct or enum named each is V0113 with a note suggesting another name
- [ ] Write the V0113 fixtures with snapshots, and register them
- [ ] Write codegen fixture `time_ids_bytes` calling every call (results shown through `to_iso`, numbers, `bool`, and `string`, since `{}` on the types comes in task 3; `Time::now()`, `Uuid::new()`, `v7()`, and `v4()` called but not printed) with its snapshot, and an `assert_runs` test following `run_table_rows`
- [ ] Write the codegen test: a single file naming `Time` only in a struct field depends on `varyk-std`, following `codegen.rs` ~110
- [ ] Implement the `Ty` variants, `BUILTIN_TYPE_NAMES`, `is_copy`, `is_compound`, the renamed `has_std_type` and `names_std_type`, `resolve_type` beside `Error` (~1078), `std_type_taken`, `ty_name`, the exhaustive matches (`package_items.rs` ~89 and ~711, `walk`), and `rust_type`
- [ ] Implement the owners, `Owner::of`, the shapes, the rows, `uses_std`, `path_owner` (~111), the "no function" messages naming each type's calls, and the three owners in `callee`'s in-full list
- [ ] Add to `docs/language.md`: the three rows of the Types table; a new "Time, ids, and bytes" section after Strings with the three call tables, spec 2.1's reading and writing rules and what `iso` means, spec 2.2's paragraph on what a version 7 id shows beside its table, and spec 3's paragraph on the `uuid` panic; the V0113 row; the V0113 comment in `codes.rs`
- [ ] Run the gate: `cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`

### Task 3: Comparing, printing, parsing, copying, and keys

Spec 2.1 to 2.4 (the comparing lines, `{}`, `assert_eq`, `parse`, derived `==` and `.clone()`), 7.1 (`is_map_key`, `ElementRule`), 7.3 (`check.rs`, `methods.rs`), and 8 (V0100, V0101, V0200, V0203).

**Files:**
- Modify: `crates/varyk/src/types.rs` (`is_map_key`), `crates/varyk/src/resolve/mod.rs` (V0101 message ~1021), `crates/varyk/src/builtins.rs` (`ElementRule`), `crates/varyk/src/types/check.rs` (`binary`, `format_args`, `assert_call`), `crates/varyk/src/types/check/methods.rs` (`derived_clone`, `clone_call`, `parsed_type`), `crates/varyk/src/diagnostics/codes.rs`, `docs/language.md`
- Create: `crates/varyk/tests/fixtures/codegen/records_compare/main.vr`; error fixtures `v0100_clone_on_time`, `v0101_time_map_key`, `v0200_ordering_on_uuid`, `v0200_parse_into_bytes`, `v0203_print_bytes`
- Test: type tests, `builtins.rs` tests, `tests/codegen.rs`, `tests/examples.rs`, `tests/errors.rs`

**Interfaces:**
- Consumes: the types, owners, and rows of task 2.
- Produces: `Ty::is_map_key` true for `Uuid`; `ElementRule::Ordered` accepting `Time` and `ElementRule::Comparable` the three, with `wanted` naming them; `parse` targets `Time` and `Uuid`; `AssertKind::Eq { show }` true for `Time` and `Uuid`.

- [ ] Write type tests for operators: `<`, `<=`, `>`, `>=` on `Time` give `bool`; on `Uuid` or `Bytes` V0200 whose note names numbers and `Time`; `t + u` and `t - u` stay V0200 (Review Focus 2); `==` and `!=` on each of the three
- [ ] Write type tests for printing: `{}` on `Time` and `Uuid` accepted; on `Bytes` V0203 whose note names `to_text` and `to_base64`; the V0203 note of other types lists `Time` and `Uuid` among printed forms; `assert_eq` shows `Time` and `Uuid` and not `Bytes`
- [ ] Write type tests for `parse` into `Time` and `Uuid`, and into `Bytes` as V0200 listing the targets
- [ ] Write type tests for copying: `.clone()` on `Time` or `Uuid` is V0100 naming them as copied on use; on `Bytes` accepted; a struct with a field of each derives `==` and `.clone()`
- [ ] Write type tests for `Vec` and maps: `sort` on `Vec<Time>` accepted and on `Vec<Uuid>` or `Vec<Bytes>` V0200; `contains` on a `Vec` of each; `HashMap<Uuid, V>` accepted, `HashMap<Time, V>` and `HashMap<Bytes, V>` V0101 listing `Uuid` among key types
- [ ] Write the error fixtures with snapshots, and register them
- [ ] Write codegen fixture `records_compare` with its snapshot and `assert_runs` test: times compared and sorted; `{}` of a `Time` and of a `Uuid` parsed from uppercase text (printed lowercase); `contains` and `==` with a borrowed `Bytes` parameter against an owned local; a `HashMap<Uuid, string>` of one entry through `insert`, `get`, `contains_key`, and `keys` (one, since `keys` has no fixed order and the output is fixed); `b.clone()`; and a `#[test]` with `assert_eq` on two `Time`s, which `varyk run` does not build, so the test also runs `varyk test` on the fixture and sees it pass, following `run_and_test_an_async_single_file` (`tests/examples.rs` ~352)
- [ ] Implement: orderings for `Time` only (the arithmetic gate unchanged), the `{}` arm and notes, `show`, `parsed_type`, `derived_clone` and the V0100 wording, the `ElementRule`s, `is_map_key`, and the V0101 text
- [ ] Update `docs/language.md` (Copy types ~391, and the lists that name numbers and `bool` as copies or as comparable, such as chain items ~772 and `==` ~659; the `HashMap` row of the Types table; the `parse` row ~493; Printing; comparison in the new section; rows V0100, V0101, V0200, V0203) and the four `codes.rs` comments
- [ ] Run the gate: `cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`

### Task 4: JSON and `env`

Spec 2.4 (JSON and `env` rows), 5 (serde), 7.3 (`derives.rs`), and 8 (V0210).

**Files:**
- Modify: `crates/varyk/src/types/derives.rs` (`walk`, `env_readable`, `ENV_FIELDS`, the JSON note of `Part::Other`), `crates/varyk/src/diagnostics/codes.rs` (V0210 comment), `docs/language.md` (JSON ~1627, Configuration ~1729, V0210 row)
- Create: `crates/varyk/tests/fixtures/codegen/records_json/main.vr`; `crates/varyk/tests/fixtures/codegen/records_env/main.vr` (the `env::parse` program, run only by `examples.rs`, no snapshot); error fixtures `v0210_bytes_from_env`, `v0210_uuid_map_key_through_json`
- Test: `derives.rs` unit tests (~1048), type tests, `tests/codegen.rs`, `tests/examples.rs`, `tests/errors.rs`

**Interfaces:**
- Consumes: task 1's serde and `env` reader, task 2's `walk` arm.
- Produces: `convertible` accepting the three, alone and inside `Option`, `Vec`, structs, and `HashMap<string, _>`; `env_readable` accepting `Time`, `Uuid`, and their `Option`s; `Bytes` in `env` refused as `Part::Other` (V0210).

- [ ] Write `derives.rs` tests: each type convertible alone and in an `Option`, a `Vec`, and a struct; `env_readable` accepts `Time`, `Uuid`, and `Option`s of them; a `Bytes` field in `env` is refused with `ENV_FIELDS` listing `Time` and `Uuid`; a `HashMap<Uuid, i64>` field of a JSON-reached struct is refused with the map-key note and `HashMap<string, Uuid>` accepted (Review Focus 3)
- [ ] Write the two V0210 fixtures with snapshots, and register them
- [ ] Write codegen fixture `records_json` with its snapshot and `assert_runs` test: a struct with `Time`, `Uuid`, `Bytes`, `Option<Time>`, and `Vec<Uuid>` fields written with `json::stringify`, printed, read back with `json::parse`, and compared with `==`; a number for a `Time` and unpadded base64 for a `Bytes` printing their errors
- [ ] Write `records_env` and an `examples.rs` test running it following `run_config_with` (~196: `varyk_run_with` in an `empty_dir`, the program's variables set or removed, so a tester's own environment or a `.env` cannot change the output): a struct with a `Time` and an `Option<Uuid>` read by `env::parse` prints both, and reads the id as `None` when its variable is removed; a bad `Time` prints the error naming its variable
- [ ] Implement `walk`, `env_readable`, `ENV_FIELDS`, and the JSON note
- [ ] Update `docs/language.md` (the JSON mapping: RFC 3339 text in UTC, the lowercase hyphenated id, standard base64 with padding read strictly; Configuration; V0210 row) and the V0210 comment
- [ ] Run the gate: `cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`

### Task 5: The three types in `.rs` facades and imported enums

Spec 2.5, 7.2 (`std_item`, `RustTy`, the mapper, `names_std`), and 8 (V0108).

**Files:**
- Modify: `crates/varyk/src/interop/signatures.rs` (`RustTy`, `text`, `StdItem`, `std_item`, `map_value`), `crates/varyk/src/interop/items.rs` (`unmapped_payload_reason` ~759, the `use`-line reason of a variant payload), `crates/varyk/src/resolve/signatures.rs` (`param`, `value`, `named`, `names_std`), `crates/varyk/src/diagnostics/codes.rs` (V0108 comment), `release-please-config.json`, `docs/language.md` (Calling Rust ~2227, Rust enums ~2450, A facade for a package ~2539, V0108 row)
- Create: `crates/varyk/tests/fixtures/packages/facade_types/` (`Cargo.toml` with `[lib] path = "src/lib.vr"` and a `varyk-std` line, following `store`; `src/lib.vr`; `src/ext.rs`; `src/tests.vr`); `crates/varyk/tests/fixtures/interop/serde_std_types/` (`main.vr`, `ext.rs`); error fixture `v0108_reference_to_time` (with `ext.rs`)
- Test: `interop/tests.rs`, `resolve/tests.rs`, `types/check/tests.rs` (`mod packages`), `tests/examples.rs`, `tests/errors.rs`

**Interfaces:**
- Consumes: tasks 2 to 4.
- Produces: `RustTy::{Time, Uuid, Bytes}` and `StdItem::{Time, Uuid, Bytes}` for `varyk_std::Time`, `varyk_std::Uuid`, `varyk_std::Bytes` written in full (a leading `::` allowed); `Time` and `Uuid` by value as parameters (owned), returns, `pub` fields, variant payloads, and inside `Option`, `Vec`, and `Result`; `&varyk_std::Bytes` a parameter `(Ty::Bytes, ParamMode::SharedBorrow)`; `varyk_std::Bytes` owned in the same places as `Time`; V0108 with a note giving the accepted form for `&varyk_std::Time`, `&varyk_std::Uuid`, `&mut varyk_std::Bytes`, and any of the three reached through a `use` line; each counted by `names_std`; the fixture's facade items named below, which task 6 reuses.

- [ ] Write interop tests: `std_item` recognises the three with and without a leading `::` and refuses generic arguments; `map_value` gives the new `RustTy`s; an enum `Message { Text(String), Binary(varyk_std::Bytes) }` is not opaque, and the same enum holding `Bytes` through `use varyk_std::Bytes` is opaque with a reason naming `varyk_std::Bytes`
- [ ] Write resolve tests: each accepted form of spec 2.5 maps to its type and mode; each refused form is V0108 with its note; a signature naming one sets `names_std`
- [ ] Write a type test in `mod packages`: a program calls `facade_types`' `size`, `keep`, and a function returning `Option<varyk_std::Time>`, and matches its `Message`, with the package added by `Build::add_dir` from `tests/fixtures/packages/facade_types` with `("varyk-std", Dep::Rust)`, following `varyk_http()` (`test_packages.rs` ~188), since `Build::add` reads only `tests/fixtures/resolve/packages/`
- [ ] Write fixture `serde_std_types` (an `ext.rs` with a `decode` whose type parameter is `varyk_std::serde::de::DeserializeOwned` and an `encode` taking `&T` with `T: varyk_std::serde::Serialize + ?Sized`; a `main.vr` passing each a struct holding the three) and a type test on it through `check_path`, following `a_serialize_parameter_takes_any_type_json_writes` (`types/check/tests.rs` ~1311): both calls check with no change (spec 2.5)
- [ ] Write the V0108 fixture with its snapshot, and register it
- [ ] Write `facade_types`: `src/ext.rs` with functions taking and returning `varyk_std::Time` and `varyk_std::Uuid` (alone, in `Option`, `Vec`, and `Result<_, varyk_std::Error>`), `size(b: &varyk_std::Bytes) -> usize`, `keep(b: varyk_std::Bytes) -> varyk_std::Bytes`, a struct `Upload` with `pub` fields of the three and a constructor, and `Message` with `next(binary: bool) -> Message`; `src/tests.vr` with `#[test]`s calling each and matching `Message::Binary(b)` to read `b.len()`
- [ ] Add an `examples.rs` test running `varyk test` on a `package_copy` of `facade_types`, following `the_varyk_http_stub_s_test_passes` (~1528), and its `extra-files` entry
- [ ] Implement the `RustTy` and `StdItem` variants, `std_item`, `map_value`, the `param` and `value` arms with the refused forms' notes, the `use`-line note in `named` and the `use`-line reason in `unmapped_payload_reason` (a variant holding `Bytes` brought in by `use varyk_std::Bytes`) each naming the full path for the three, and `names_std`
- [ ] Update `docs/language.md` (the Calling Rust table rows of spec 2.5; a Rust enum's variants may hold the three; the V0108 row) and the V0108 comment
- [ ] Run the gate: `cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`

### Task 6: `Bytes` ownership and soundness

Spec 2.3 (owned, moved, and borrowed as `string` is), 4, 7.4, 8 (V0304), and 9 (soundness templates).

**Files:**
- Modify: `crates/varyk/src/borrow/slots.rs` (`clone_fix_it`), `crates/varyk/src/borrow.rs` (only where a soundness case shows a rule missing `Bytes`), `crates/varyk/src/diagnostics/codes.rs` (V0304 comment), `crates/varyk/tests/soundness.rs`, `docs/language.md` (Functions and parameters ~1159, V0304 row)
- Create: `crates/varyk/tests/fixtures/codegen/bytes_ownership/main.vr` (with `ext.rs`); error fixtures `v0304_bytes_from_parameter_into_field` and `v0304_bytes_from_a_borrowed_enum` (with `ext.rs` holding a `Message` enum with a `Binary(varyk_std::Bytes)` variant and an owned `keep(b: varyk_std::Bytes)`)
- Test: `borrow/tests.rs`, `tests/soundness.rs`, `tests/codegen.rs`, `tests/examples.rs`, `tests/errors.rs`

**Interfaces:**
- Consumes: `Ty::Bytes` compound (task 2), the facade forms and the `Message` shape (task 5).
- Produces: `clone_fix_it` adding `.clone()` for `Ty::Bytes` as for `Ty::String`.

- [ ] Write borrow tests: a `Bytes` parameter is a shared borrow written `&::varyk_std::Bytes`; a field owns; a use after `let c = b` is the error a `string` gets; returning the parameter or part of it is a borrowed return
- [ ] Write borrow tests for Review Focus 4: a `Bytes` returned from a borrowed parameter and stored in a field (through `check_str`), and one bound by a `match` on a borrowed imported enum and passed to an owned `varyk_std::Bytes` parameter (through `check_path` on `v0304_bytes_from_a_borrowed_enum`, following `passing_an_immutable_let_to_an_imported_mut_reference_is_v0302`), are each V0304 with the `.clone()` fix-it
- [ ] Write the two V0304 fixtures (`fix_it: true`) with their snapshots, and register them
- [ ] Add to the soundness `PRELUDE` a `Bytes` maker, reader, and struct field, and to `EXT_RS` a `&varyk_std::Bytes` and an owned `varyk_std::Bytes` parameter; add `Bytes` cases in each position the `string` cases cover to `MUST_PASS` and `MUST_REJECT`
- [ ] Write codegen fixture `bytes_ownership` with its snapshot and `assert_runs` test: a parameter, a field, a move, a cloned return, a returned parameter, and the fixed Review Focus 4 programs building
- [ ] Implement `clone_fix_it` for `Bytes`, and any rule the soundness cases show treating `Bytes` as other than a struct
- [ ] Update `docs/language.md` (a `Bytes` is passed, stored, and moved as a `string` is, and `.clone()` copies the handle; V0304's fix names `Bytes`) and the V0304 comment
- [ ] Run the gate: `cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`

### Task 7: Trailing values and route parameters

Spec 2.4 (trailing value and route rows), 4, 6.1, 7.3 (`is_value_type`, `routes.rs`), 7.5, 8 (V0218, V0219), and 9 (the stub and `http_user`).

**Files:**
- Modify: `crates/varyk/src/types/check.rs` (`is_value_type`, V0218 text ~1720), `crates/varyk/src/types/check/routes.rs` (path parameter rule, `is_query`, `PARAMETERS`, the "add a parameter" note), `crates/varyk/src/backend/rust_expr.rs` (`Need`, `values`, the block-like case, the bridge), `crates/varyk/src/diagnostics/codes.rs` (V0218, V0219 comments), `crates/varyk/tests/fixtures/packages/varyk-http/src/server.rs`, `crates/varyk/tests/fixtures/packages/http_user/src/main.vr`, `crates/varyk/tests/fixtures/codegen/trailing_values/main.vr`, `docs/language.md` (the `Value` row ~2276, HTTP route parameters ~2033-2070, V0218 and V0219 rows)
- Create: `http_error_case!` fixtures `v0219_bytes_path_parameter`, `v0219_bytes_query_parameter`
- Test: type tests (`mod http` with `varyk_http()`), `tests/codegen.rs` (`trailing_values_to_a_vec_of_values`), `tests/examples.rs` (a run of `trailing_values`, `http_user`'s output and snapshot), `tests/errors.rs`

**Interfaces:**
- Consumes: task 1's `Value::from` impls, task 6's `Bytes` ownership.
- Produces: `is_value_type` accepting the three and their `Option`s; `Need::OptRef`, an `Option<&T>` from an `Option<T>` through `.as_ref()`, mirroring `Need::OptStr`; `Binding::Path` and `Binding::Query` for `Time` and `Uuid`; `impl Plain` for `varyk_std::Time` and `varyk_std::Uuid` in the stub.

- [ ] Write type tests: each of the three and each `Option` as a trailing value accepted; a `Bytes` local passed is still usable afterwards; a `Vec<Bytes>` is V0218 whose text lists the three; a `Uuid` and a `Time` path parameter and a `Time`, `Option<Time>`, `Uuid`, and `Option<Uuid>` query parameter bind; a `Bytes` path or query parameter is V0219; a route whose body and return value are structs holding the three is accepted (spec 2.4's first row)
- [ ] Write the two V0219 fixtures with snapshots, and register them
- [ ] Extend `trailing_values` (its `ext.rs` unchanged) with a `Time`, a `Uuid`, an `Option<Time>`, a `Bytes` local (`&b`), a `Bytes` parameter and a borrowed `Bytes` return (`b`), an `Option<Bytes>` local and parameter (`.as_ref()`), and an `if` giving `Some(b)` or `None` (Review Focus 5); assert each form in `codegen.rs`, review its regenerated snapshot, and add an `assert_runs` test
- [ ] Implement `is_value_type` and the V0218 text; in `values`, a `Bytes` lent through `Need::Shared { binding: false }` and an `Option<Bytes>` through the new `Need::OptRef`, which also gets the block-like case and the bridge
- [ ] Implement the route rules and texts in `routes.rs` (path parameters and `is_query` accept `Time` and `Uuid`; `PARAMETERS` and V0219's messages list them)
- [ ] Implement `Plain` for `varyk_std::Time` and `varyk_std::Uuid` in the stub, its doc comment updated
- [ ] Give `http_user` a route with a `Uuid` path parameter and an `Option<Time>` query parameter, sent a request with each, one with a malformed id (a 400, the handler not called), and one without the query; update the expected output and review the regenerated `http_user_main_rs` snapshot
- [ ] Update `docs/language.md` (the `Value` row: the three and their `Option`s, a `Bytes` lent and not copied; route path and query parameter types; V0218 and V0219 rows) and the two `codes.rs` comments
- [ ] Run the gate: `cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`

### Task 8: The `records` example, the documents, and the release check

Spec 9 (the example, the documents, done), 10, and 11.

**Files:**
- Create: `examples/records.vr`
- Modify: `crates/varyk/tests/examples.rs` (`run_records`), `docs/language.md`, `docs/design.md`, `docs/roadmap.md`, `docs/open-questions.md`, `README.md`, `AGENTS.md`
- Test: `tests/examples.rs`, `tests/reference.rs`

**Interfaces:**
- Consumes: everything above.

- [ ] Write `examples/records.vr` as spec 9 says (times from `Time::from_iso`, parsed `Uuid`s, `Time::now()` and `Uuid::new()` called and not printed, `add_seconds` and `seconds_since`, a struct of the three through `json::stringify` and back, `to_base64` and `to_text`) and `run_records` with its fixed output, following `run_readings` (~161)
- [ ] Rename "Not in milestone 5b4" in `docs/language.md` to "Not in milestone 5c", drop its line on dates, times, UUIDs, and bytes (~1881), add spec 2.6's list, and fix every link to the old heading across the docs
- [ ] Update `docs/design.md`: the reference count of a trailing `Bytes` beside the two allocations (~35, ~129), and a decisions row for the three built-in types and `varyk-std`'s new dependencies
- [ ] Update `docs/roadmap.md` as spec 10 says (the 5c entry links the spec and becomes two lists; the compiler items checked; the package items left unchecked; the Unscheduled pointer follows the renamed section) and `docs/open-questions.md` with spec 11's five questions
- [ ] Update `README.md` (features and status) and `AGENTS.md` (`varyk-std` holds time, ids, and bytes; the `varyk-http` contract is section 6 of the 5b4 spec with section 6.1 of this one)
- [ ] Read every document against the change, as spec 9's list and `AGENTS.md` ask
- [ ] Check every `release-please-config.json` entry, `facade_types`' included, by bumping `varyk-std`'s version in a scratch copy and applying each `extra-files` jsonpath, then running the suite
- [ ] Search the diff for `unwrap`, `expect`, and other crash-on-absence calls outside tests
- [ ] Run the gate: `cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`, then `rustup run 1.85 cargo build --workspace` and `CARGO_TERM_COLOR=always cargo test --workspace`
