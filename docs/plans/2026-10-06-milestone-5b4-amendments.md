# Milestone 5b4 Amendments Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement the 2026-10-06 amendments to milestone 5b4 that `varyk-http`'s own design needs: `Error::with_status` as a Varyk call, the compiler marking `varyk-http`'s own `App`, adapters binding the package's types last, and the stub, fixture, and docs brought in line (errors sent with their status only from 400 to 599, `before` hooks only for matched routes, `before_on` by route pattern, no default in-flight limit).

**Architecture:** A builtin row for `Error::with_status` beside `Error::new`; `mark_http` also marks the package being checked when its name is `varyk-http`; the route and hook adapter writers emit the package-type bindings after the others; the stub fixture's constructors move to Varyk, and its rules follow the amended spec; the docs say the same.

**Tech Stack:** Rust stable, edition 2024, MSRV 1.85; no new dependency.

**Spec:** `docs/specs/2026-10-05-milestone-5b4-design.md`, its 2026-10-06 amendments (sections 2.1, 2.4, 2.6, 4, 5, 6.1, 6.3, 6.5, 7.2, 7.3, 9, 10, 12). The package that needs them is specified in `Varyk-Lang/varyk-http`'s `docs/specs/2026-10-06-varyk-http-design.md`. Read the spec's amended sections before any task.

## Global Constraints

- `AGENTS.md` applies in full: the gate after every task, MSRV 1.85 with no let-chains, minimum viable, fail loudly, stable codes, snapshots reviewed, conventional commits, safe by default, every document current.
- No new diagnostic code; no new mechanism beyond what the amendments name.
- No `unwrap`, `expect`, or other crash-on-absence call in non-test code, the stub's Rust included.
- Work on branch `feat/milestone-5b4` (PR #31, open). At the end the branch is squashed again into one commit for the PR; that is the controller's step, not a task's.
- No attribution lines in commits.

## Review Focus

1. `Error::with_status(400, text)` where `text` is a borrowed `string` parameter: V0304 with the help to `.clone()`, as for `Error::new`. Task 1.
2. A package named `varyk-http` that also depends on another `varyk-http` (two versions): each `App` is marked once, the package's own and the dependency's. Task 2.
3. A handler `async fn h(req: http::Request, id: i64)` on `/x/{id}`: the generated adapter binds `id` before `req`, and the call passes them in the handler's order. Task 3.
4. `http_user` run output after the stub changes: `GET /nowhere` gives 404 with the `after` hook's header, and no line changes that should not. Task 2.

---

## Overview

Four tasks. Task 1 adds `Error::with_status`. Task 2 marks the package's own `App` and brings the stub and `http_user` in line (constructors in Varyk, the 400 to 599 rule, `before` only for matched routes, `before_on` by pattern, a route test in the stub's own `src/tests.vr`). Task 3 orders the adapters' bindings. Task 4 updates the documents and the site hero branch.

## Context

Line numbers are approximate. Paths are under `crates/varyk/src/` unless shown otherwise.

- Builtins: `builtins.rs` (`Error::new` row ~630, `message` ~638, `status` ~641; tests ~1245-1260).
- Marking: `resolve/package_items.rs` (`mark_http` ~544, the filter on `Packages.checked` by `HTTP_PACKAGE`, `HttpItems` pushed ~581); `resolve/mod.rs` (`HttpItems` ~522); `test_packages.rs` (`varyk_http()` and `Build::add_dir`).
- Adapters: `backend/rust.rs` (`route_adapter` ~255, `hook_adapter` ~302, `bind` ~378).
- Stub: `tests/fixtures/packages/varyk-http/` (`src/lib.vr`, `src/server.rs` with `covers` ~283, `respond_error` ~549 using `100..=599`, the Rust constructors ~575-604, the hook loop ~185; `src/tests.vr`).
- Fixture program: `tests/fixtures/packages/http_user/src/main.vr` (expected output comment ~21); `tests/examples.rs` (`HTTP_USER_OUTPUT` ~1541, the run and snapshot test ~1575).
- Docs: `docs/language.md` (the `Error` table, the HTTP section ~2000-2200 with the `before`/`after` rows ~2011-2012, the `after` paragraph ~2123, the example with a "listening" line ~1994, the status range ~2181, the two places that call the bare `varyk_std::Error` return the constructors' mechanism ~2275 and ~2619), `docs/roadmap.md` (the `varyk-http` list ~177), `docs/plans/2026-10-05-milestone-5b4-followups.md`; the site: `../varyk.com` branch `hero-5b4`, `templates/partials/home-service.html`.

## Development Approach

- TDD per task: a failing test first, the minimal code, then the gate.
- Builtin rows follow `Error::new`'s row and its tests; the owned text slot is the same shape.
- Stub changes stay the stub's: the stub is a test double of the real package's contract, so it changes only where the amended spec says, and stays small.
- Not needed: any change to how a dependency's `App` is marked; any new diagnostic; any change to `varyk-std`'s code (it already has `with_status`; only its documentation changes, task 1).

## Tasks

### Task 1: `Error::with_status` from Varyk

Spec 2.6 and 7.3 (amended): `Error::with_status(status, text)` beside `Error::new`, `status` a `u16`, `text` an owned `string`.

**Files:**
- Modify: `crates/varyk/src/builtins.rs`, the backend where `Error::new` is emitted (`backend/rust_expr.rs`), `crates/varyk/src/types/check/values.rs` (~361, the message for an unknown `Error::` function), `crates/varyk-std/src/error.rs` (`with_status`'s documentation), `docs/language.md` (the `Error` table)
- Test: `builtins.rs` tests, `types/check/tests.rs`, `tests/fixtures/codegen/error_with_status/` with its snapshot, a V0304 error fixture

**Interfaces:**
- Produces: the builtin `Error::with_status(u16, string) -> Error`, emitted as `::varyk_std::Error::with_status(status, text)`.

- [ ] Write builtin and type tests: `Error::with_status(404, "gone")` types as `Error`; a borrowed `string` parameter as `text` is V0304 with the clone help (Review Focus 1); `text.clone()` is accepted; a status that is not a `u16` is V0200
- [ ] Write codegen fixture `error_with_status` (a function returning `Err(Error::with_status(409, name.clone()))` and one reading `e.status()`) and its snapshot
- [ ] Write the V0304 fixture `v0304_with_status_borrowed_text` with its snapshot and registration
- [ ] Implement the builtin row beside `Error::new` and its emission
- [ ] Make the message for an unknown `Error::` function name both `Error::new(..)` and `Error::with_status(..)`, and update its test (`types/check/tests.rs` ~4161)
- [ ] Rewrite `with_status`'s Rust documentation per the amended spec 5 and 2.6: a status from 400 to 599 is sent with its message, any other as a 500; only Varyk code sets one, and no official package's Rust calls it
- [ ] Add the `Error::with_status` row to `docs/language.md`'s `Error` table, saying a status from 400 to 599 is sent with its message by `varyk-http` and any other status as a 500
- [ ] Run the gate: `cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings && CARGO_TERM_COLOR=always cargo test --workspace`

### Task 2: `varyk-http`'s own `App`, and the stub in line with the amendments

Spec 2.1, 2.4, 2.6, 7.2, 9 (amended): a package named `varyk-http` marks its own root `App`, `Request`, and `Response`; the stub's constructors are Varyk over `Error::with_status`; its `respond_error` sends a status only from 400 to 599; it runs no `before` hook for an unmatched request; its `before_on` tests the matched route's pattern; its `src/tests.vr` adds a route and sends a request.

**Files:**
- Modify: `crates/varyk/src/resolve/package_items.rs` (`mark_http`), `crates/varyk/src/resolve/mod.rs` (`Packages` ~1254 gains the checked package's own name), `crates/varyk/src/lib.rs` (`check_package` ~225 passes `package.name`), `crates/varyk/src/test_packages.rs` (`add_dir` passes its `name`), `crates/varyk/src/backend/rust.rs` (`package_of`/`package_root` ~443, ~856), `crates/varyk/tests/fixtures/packages/varyk-http/src/lib.vr`, `src/server.rs`, `src/tests.vr`, `crates/varyk/tests/fixtures/packages/http_user/src/main.vr` (its expected-output comment), `crates/varyk/tests/examples.rs` (`HTTP_USER_OUTPUT`)
- Test: `types/check/tests.rs` (a `Build` whose checked package is named `varyk-http`), the stub's `varyk test` run in `tests/examples.rs`, the `http_user` run and its snapshot

**Interfaces:**
- Consumes: `Error::with_status` (task 1).
- Produces: `mark_http` marks the root items of the package being checked when its name is `varyk-http`, besides every `varyk-http` it depends on; the stub's `lib.vr` declares `bad_request`, `unauthorized`, `forbidden`, `not_found`, `conflict`, and `error` in Varyk.

- [ ] Write a type test: checking a library named `varyk-http` whose own `src/lib.vr` (or a test module) calls `App::new` and `app.get(..)` on its own `App` accepts them as intrinsics; a library named otherwise does not mark its `App`; a `varyk-http` depending on another `varyk-http` marks both (Review Focus 2)
- [ ] Pass the package's own `[package]` name to `mark_http` through `Packages`, from `check_package` and `Build::add_dir`
- [ ] Implement the own-package marking in `mark_http`; for the package's own `App`, the adapters and registrations write `crate` (the package root, whose `pub use` lines name every contract item), never the module `App` is declared in (`package_of`/`package_root`, `backend/rust.rs` ~443, ~856)
- [ ] Write a backend unit test: an adapter in the package's own code starts `async fn varyk_route_0(varyk_req: crate::Request)` and registers with `crate::route(..)`, with `App` declared in a module other than the root
- [ ] Move the stub's six constructors from `server.rs` to `src/lib.vr` as Varyk functions over `Error::with_status`, remove their `pub use` lines
- [ ] Change the stub's `respond_error` to send `{"error": message}` with the status only from 400 to 599 and the fixed 500 otherwise; its hook loop to run no `before` hook when no route matches; its `covers` to test the matched route's pattern as written
- [ ] Add to the stub's `src/tests.vr` a test that builds an app with one route, sends a request through `app.request`, and asserts the status and body (this proves the own-package marking under `varyk test`)
- [ ] Update `http_user`'s expected output (`GET /nowhere` now 404 with the `after` hook's header) in its comment and in `HTTP_USER_OUTPUT`, run it, regenerate and read its snapshot (Review Focus 4)
- [ ] Run the gate

### Task 3: Package types bound last

Spec 4 (amended): in route and hook adapters, the path, query, body, and state bindings come first and the `bind::<K>()` bindings after them, so a 400 happens before a live handler's early response; the handler call keeps the handler's parameter order.

**Files:**
- Modify: `crates/varyk/src/backend/rust.rs` (`route_adapter`, `hook_adapter`), `crates/varyk/tests/fixtures/packages/http_user/src/main.vr` (a handler whose `http::Request` parameter comes before a path parameter, if none does yet)
- Test: the `http_user` snapshot, a backend unit test of the binding order

**Interfaces:**
- Consumes: the adapter writer from milestone 5b4 task 6.

- [ ] Write a backend unit test: for a handler `(req: http::Request, id: i64)` the emitted lines bind `varyk_1` (`param`) before `varyk_0` (`bind`), and the call passes `varyk_0, varyk_1` in order (Review Focus 3)
- [ ] Implement the two-group order in `route_adapter` (hook adapters bind no package type, so they do not change)
- [ ] Regenerate and read the `http_user` snapshot; the run output does not change
- [ ] Run the gate

### Task 4: The documents and the site hero

Spec 9 and 10 (amended): every document says what the amended spec says.

**Files:**
- Modify: `docs/language.md`, `docs/roadmap.md`, `docs/open-questions.md`, `docs/plans/2026-10-05-milestone-5b4-followups.md` (drop what the amendments fixed, if listed), `../varyk.com/templates/partials/home-service.html` on that repository's branch `hero-5b4`
- Test: `tests/reference.rs`, `tests/cli.rs` (the language reference's programs), the site's build

**Interfaces:**
- Consumes: tasks 1-3.

- [ ] Update `docs/language.md`: the status range (a status from 400 to 599 is sent, any other is a 500); the `before` and `after` rows and the `after` paragraph (a `before` hook runs for every request a route matches; an `after` hook on every response the router makes); `before_on` tested against the matched route's pattern; the in-flight limit off by default; the two places that call the bare `varyk_std::Error` return the constructors' mechanism (the constructors are Varyk over `Error::with_status`; the shape remains for a facade making an error of its own); a live handler may return nothing or `Result<http::Response, Error>`; the HTTP example without its own "listening" line
- [ ] Update `docs/roadmap.md`: the `varyk-http` list (no default in-flight limit, a limit on requests in flight by one call) and the compiler list's `Error` line (a status set by `Error::with_status` and the `http::` constructors written over it)
- [ ] Rewrite the answered half of `docs/open-questions.md`'s `varyk_std::Error` entry: a status is set by `Error::with_status` from Varyk
- [ ] Remove from the 5b4 follow-ups file any item the amendments fixed
- [ ] On `../varyk.com` branch `hero-5b4`, drop the hero's own "listening" log line, and run the site's build check as its CONTRIBUTING says; commit there with a `docs:` prefix, not pushed
- [ ] Run the gate, then `rustup run 1.85 cargo build --workspace`
