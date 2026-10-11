# Language server, stage 1, Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `varyk lsp`, a Language Server Protocol server inside the compiler that shows `varyk check`'s diagnostics on open and on save and offers each fix-it as a quick fix, plus the documents and the `AGENTS.md` rule that keeps the external grammars current.

**Spec:** `docs/specs/2026-10-11-language-server-design.md`, "spec §n". Read it in full before any task.

## Overview

`load` in `cli.rs` splits into `check_target`, which checks and returns diagnostics with their sources instead of printing them, and a thin `load` that prints exactly as before; the file branch of `locate_for` moves into a function that returns a manifest's diagnostics the same way. On top of that, one new module, `crates/varyk/src/lsp.rs`, speaks JSON-RPC with `Content-Length` framing over `serde_json` on standard input and output (no new dependency), finds the program a saved `.vr` file belongs to as `varyk check` would, checks it afresh, publishes the diagnostics per file, and answers `textDocument/codeAction` with the fix-its of the last check while the editor's text and the disk still match what that check read.

The server is one thread and holds no compiler state between checks. A panic in a check becomes a `window/showMessage` and the server goes on. The command-line output of every existing command stays byte for byte the same; the existing tests and snapshots are the proof. Documents follow in the same pull request (spec §8).

## Context

Line numbers are approximate.

- `crates/varyk/src/cli.rs`: module doc listing the commands 1-27; `Cli` 53 (global `--message-format`); `Command` 85; `run` 169 (dispatch); `Target` 195; `locate` 208 and `locate_for` 214 (the `Some(path)` branch at ~249 calls `Package::for_file`; the `expect` at ~261 goes away with the split); `run_check` 297; `Loaded` 306; `load` 316-385 (unreadable entry 318-324, `Graph::read` 325-333, `check_file` 337, `check_build_std` 345, the logging note 353-363); `emit_diagnostics` 1011.
- `crates/varyk/src/lib.rs`: `check_file` 82 (drops fix-its in dependency files at ~105 and ~140); `shown` 263 (a dependency's files spelled relative to the working directory).
- `crates/varyk/src/package.rs`: `find` 94; `root_of` 105; `load` 115; `check_build_std` 961; `Package::for_file` 1387; `tests::TempDir` (used by `lib.rs` tests) for temporary trees in unit tests.
- `crates/varyk/src/diagnostics/mod.rs`: `source_file` 29 (panics on an unknown id; the server must not use it); `Label` 41; `Diagnostic` 50 (`code`, `message`, `span`, `labels`, `notes`, `fix_it`).
- `crates/varyk/src/diagnostics/render.rs` 1-80: how notes and labels are shown in the terminal (`= note:` lines).
- `crates/varyk-syntax/src/span.rs` 10: `Span` (byte offsets, `FileId`); `source.rs` 43: `SourceFile` (`path`, `text`); `error.rs` 25: `FixIt` (`span`, `replacement`).
- `crates/varyk/src/types/check.rs` ~1382: V0200 for `+` on strings, with a note and a `format!` fix-it (the integration test's type error).
- `crates/varyk/src/resolve/visibility.rs` ~597: `declared_without_pub`, V0105's `pub ` fix-it at the declaration in another module; fixture `crates/varyk/tests/fixtures/two_uses_of_a_private_fn/`.
- `crates/varyk/tests/common/mod.rs`: `varyk_in` 37 (spawns `CARGO_BIN_EXE_varyk`), `empty_dir` 89 (a fresh directory under `CARGO_TARGET_TMPDIR`, below the repository's workspace `Cargo.toml`, which has no `src/main.vr`, so files there are single-file programs).
- `crates/varyk/tests/cli.rs` 22-120: tests of `check`'s output, `--version`, and the plain messages; 198 `one_missing_pub_gets_one_fix_it`.
- Docs: `README.md` "Try it" 170 (main commands ~191-199), "Documents" 314-331; `docs/language.md` "Not in milestone 5c" 1992 ("a language server" ~2082), "The command line" 3039 (table 3041-3052), "Error codes" 3219; `docs/roadmap.md` milestone 6 211-218 (the language server line 214); `docs/design.md` decisions log 127; `AGENTS.md` "Keep every document current" 95, "Where things are" 127; `CONTRIBUTING.md` "Where things are" 25.
- Precondition: the std-minimum change (`docs/specs/2026-10-11-std-minimum-design.md`) is implemented and its pull request opened first, and this branch is rebased onto `std-minimum` before Task 1, so `load` and `check_build_std` already take its rule argument (normal or publish, `run_publish` alone asking for publish); the line numbers above are from before that rebase.

## Development Approach

- TDD where it fits: the pure parts (framing, positions, URIs, the diagnostic's LSP form, titles, range overlap, entry choice) get failing unit tests in `lsp.rs`'s `#[cfg(test)] mod tests` first, as spec §9 lists them; the session as a whole is the integration test of Task 8.
- The refactor of Task 1 adds no test: the existing `check`, `build`, `run`, `test`, and `publish` tests and snapshots passing unchanged is its proof (spec §4.3). Do not regenerate any snapshot.
- The server is written against `impl BufRead` and `impl Write`, so unit tests drive it with byte buffers, or call its per-message handler on a `Server` they set up when the disk must change between two messages (Task 5's second check, Task 6's changed file); `run` hands it locked standard input and output.
- JSON is built and read with `serde_json::Value` and the `json!` macro, as `diagnostics/json.rs` does; no typed protocol structs, no `lsp-types` or `lsp-server` (spec §1.1).
- No `unwrap`, `expect`, or indexing that can panic in `lsp.rs`: a malformed request gets an `InvalidParams` error response, a malformed notification a line on standard error (spec §7). Look a `FileId` up with `sources.get`, never `source_file`, and skip what it cannot find.
- Nothing in the server writes to standard output except the frame writer; log lines use `eprintln!`.
- Unit tests that need a tree on disk (entry choice, staleness) follow `package::tests::TempDir`.
- Not needed: a thread, an async runtime, caching of the package graph, reading unsaved buffers, client-capability negotiation beyond what spec §3 lists, Windows drive-letter URIs (CI runs on Linux; spec §4.1 names only percent-coding), any change to `varyk-syntax`, any new diagnostic code.
- The std-minimum rule (already in `load`, see the Context precondition) touches only `check_target` and the server: `check_target` takes the rule argument `load` has, which `load` passes through (so `varyk publish` keeps the publish rule), and the server passes the normal (non-publish) rule.
- Commits: per-task commits are working commits; the pull request is squash-merged as one `feat:` commit (spec §10).
- Tasks 2-4 build helpers that Task 5 first calls from `serve`, so their gates run tests only and dead-code warnings in between are expected; never silence them with `allow`. From Task 5 on every helper is called, so the gates of Tasks 1 and 5-8 run clippy with `-D warnings` too.

## Tasks

### Task 1: Split checking and locating from printing

Spec §4.3. Output of every command unchanged.

**Files:**
- Modify: `crates/varyk/src/cli.rs`

- [ ] add a `pub(crate) enum Failure` with a diagnostics variant (the diagnostics and the `Vec<SourceFile>` they point into) and a message variant (the text `varyk check` prints after `error: `)
- [ ] move the body of `load` into `pub(crate) fn check_target(target) -> Result<(Loaded, Target), Failure>` that prints nothing and takes the std-minimum rule argument, which `load` passes through
- [ ] make `load` a wrapper that calls `check_target` and prints a `Failure` exactly as before (`error: cannot read ...` on stderr; diagnostics through `emit_diagnostics`), returning `ExitCode::FAILURE`
- [ ] move the file branch of `locate_for` (`Package::for_file`, then the `Target`) into `pub(crate) fn target_for_file(path) -> Result<Target, Failure>` with no `expect`; `locate_for` keeps the `.vr` extension check and the no-file branch and prints a returned failure as before
- [ ] make `Loaded` and `Target` `pub(crate)`, with `Target`'s fields (`entry`, `package`, `sources`) `pub(crate)` for the server
- [ ] gate: `cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test -p varyk` passes with no snapshot changed (`git status` shows no `.snap` edits)

### Task 2: Add framing and the message loop

Spec §2, §3, §7 (no `unwrap`).

**Files:**
- Create: `crates/varyk/src/lsp.rs`
- Modify: `crates/varyk/src/lib.rs` (`pub mod lsp;`)

- [ ] add a frame reader over `impl BufRead`: header lines until a blank line, `Content-Length` required, other headers ignored, end of input reported as such, a missing or non-numeric length an error that ends the server
- [ ] add a frame writer over `impl Write` that writes `Content-Length`, the blank line, the body, and flushes
- [ ] add `pub fn serve(input, output) -> ExitCode` with a `Server` state (initialized, shut down, workspace folders, per-program state added later) that loops message by message; a body that is not JSON is logged and skipped
- [ ] answer `initialize` with the capabilities of spec §3 (`textDocumentSync` `openClose`/`change: 2`/`save: {}`, `codeActionProvider` with `quickfix`, no `positionEncoding`) and `serverInfo` (`varyk`, `CARGO_PKG_VERSION`); keep the `workspaceFolders` URIs it names (Task 3 turns them into paths)
- [ ] answer `shutdown` with `null`; later requests get `InvalidRequest` (-32600); `exit` ends the loop, as does end of input; exit code 0 after `shutdown`, 1 otherwise
- [ ] a request before `initialize` gets `ServerNotInitialized` (-32002); an unknown request `MethodNotFound` (-32601); unknown notifications, `$/cancelRequest` among them, and `initialized` are ignored; the `textDocument/*` handlers are stubs filled by later tasks (`codeAction` answers an empty list)
- [ ] write tests: one message; two messages in one read; a header other than `Content-Length`; a bad length ends the server; a body that is not JSON is skipped; end of input; initialize answer shape; request before initialize; unknown method; request after shutdown; exit codes 0 and 1
- [ ] gate: `cargo test -p varyk lsp`

### Task 3: Convert positions and URIs

Spec §4.1, §5 (`range`).

**Files:**
- Modify: `crates/varyk/src/lsp.rs`

- [ ] add a byte offset to LSP position conversion over a file's text: line counted at `\n`, character in UTF-16 units from the line start, offset past the end clamped to the end, never slicing inside a character
- [ ] add `file:` URI to path (percent-decoded; other schemes give none) and path to URI (percent-encoded, `/` kept)
- [ ] make a relative source path absolute against the working directory before encoding it
- [ ] write tests: ASCII; a two-byte character before the offset (one unit); a character outside the basic plane (two units); `\r\n` line ends; an offset at and past the end; a space round-trips as `%20`; a non-ASCII name round-trips
- [ ] gate: `cargo test -p varyk lsp`

### Task 4: Convert a diagnostic to its LSP form

Spec §5.

**Files:**
- Modify: `crates/varyk/src/lsp.rs`

- [ ] convert a `Diagnostic` with its sources into the LSP diagnostic: `range` from the primary span (Task 3), `severity` 1, `source` `"varyk"`, `code`, `message` followed by one `note: …` line per note, `relatedInformation` with one entry per label (its file's URI, range, and text)
- [ ] group a check's diagnostics by the URI of their primary span's file, keeping their order; a diagnostic whose file id is not in the sources is skipped, not a panic
- [ ] write tests: a diagnostic with two notes and a label in another file to its LSP form; grouping of diagnostics in two files
- [ ] gate: `cargo test -p varyk lsp`

### Task 5: Check and publish per program

Spec §4.1, §4.2, §4.3 (the server's side), §5 (publishing).

**Files:**
- Modify: `crates/varyk/src/lsp.rs`

- [ ] on `didOpen` and `didSave` of a `file:` URI ending in `.vr`, choose the program: `package::find` from the file's directory, and when `package::root_of` finds a root there, that root file; else, for a file not named `main.vr`, the nearest `main.vr` in the file's directory and each parent, stopping after the workspace folder that holds the file (only the file's own directory when none does); else the file itself
- [ ] check it with `cli::target_for_file` then `cli::check_target`, reading everything afresh
- [ ] key each program by its package root, or by its entry path outside a package, and keep the URIs it last published diagnostics for
- [ ] publish `textDocument/publishDiagnostics` with the full list for every URI that has diagnostics now, and an empty list for every URI of the previous check of the same program that has none now; a successful check clears them all
- [ ] send a message failure (the entry cannot be read) as `window/showMessage` type 2 (warning) with the text `varyk check` prints
- [ ] write tests: entry choice for a package file, a stray `Cargo.toml` without roots (single file), a module beside a `main.vr`, a `main.vr` above the workspace folder not taken, a file named `main.vr` taken as is; a second check that fixes a file publishes an empty list for it
- [ ] gate: `cargo clippy -p varyk --all-targets -- -D warnings && cargo test -p varyk lsp`

### Task 6: Offer quick fixes behind the staleness guard

Spec §6.

**Files:**
- Modify: `crates/varyk/src/lsp.rs`

- [ ] keep, per program, each published diagnostic that has a fix-it: its document URI, its LSP form, the fix-it's file path, its byte range, its replacement, and that file's text as the check read it (from the check's sources)
- [ ] track documents changed since their last save: `didChange` marks, `didOpen` marks when its text differs from the file on disk, `didSave` and `didClose` unmark
- [ ] answer `textDocument/codeAction` with one action per kept diagnostic of that document whose range overlaps or touches the requested one: `kind` `quickfix`, `isPreferred`, `diagnostics` with its LSP form, and a `WorkspaceEdit` with one `TextEdit` in the fix-it's file, its range converted against the kept text
- [ ] offer none while the diagnostic's document or the fix-it's file is marked changed, or while the fix-it's file on disk differs from the kept text; answer an empty list then
- [ ] title it `Replace with `…``, `Insert `…`` (empty range), or `Remove this` (empty replacement), a replacement over 40 characters or spanning lines shortened to the first 40 characters of its first line and `…`
- [ ] write tests: the five titles of spec §9; overlap and touch of a cursor; no action after `didChange`; no action when the file on disk changed; an action whose edit is in another file than its diagnostic
- [ ] gate: `cargo clippy -p varyk --all-targets -- -D warnings && cargo test -p varyk lsp`

### Task 7: Guard checks against panics and add `varyk lsp`

Spec §2, §7.

**Files:**
- Modify: `crates/varyk/src/lsp.rs`, `crates/varyk/src/cli.rs`, `crates/varyk/tests/cli.rs`

- [ ] run each check through one small guard that takes the check as a closure and calls it under `std::panic::catch_unwind` (with `AssertUnwindSafe`); on a panic publish nothing, keep the program's previous state, and send `window/showMessage` type 1 with the spec §7 text
- [ ] write test: the guard given a panicking closure sends the error message and leaves the program's state as it was (no compiler panic is known to trigger, so the closure stands in for one)
- [ ] add the `Lsp` subcommand to `Command` (doc comment: speaks the Language Server Protocol on standard input and output, for editors), dispatch it in `run` to `lsp::serve` on locked stdin and stdout, ignoring `--message-format`; add it to the module doc of `cli.rs` and to `Command`'s doc
- [ ] write test in `tests/cli.rs`: `varyk --help` lists `lsp`, and `varyk lsp --help` exits 0 (the VS Code extension probes it)
- [ ] gate: `cargo fmt --check && cargo clippy -p varyk --all-targets -- -D warnings && cargo test -p varyk`

### Task 8: Write the session test

Spec §9.

**Files:**
- Create: `crates/varyk/tests/lsp.rs`

- [ ] write integration test (session in a temporary package under `common::empty_dir`, made with `varyk init` through `common::varyk_in` as `tests/cli.rs` ~296 does, since a manifest needs the target table (V0406) and `init` writes it with a `varyk-std` line the compiler accepts; no dependency besides `varyk-std`, so no `cargo metadata`): initialize with the temporary directory as workspace folder; open a file with V0200, its `didOpen` text the file as on disk (else no action is offered), and expect code, range, and the note; code action at it gives the `format!` edit; a `didChange` without save gives no actions; write the fix, save, expect an empty list; then write and save modules where a use of an item declared without `pub` gives V0105, and a code action there gives an edit in the declaring module; shutdown, exit, status 0 (V0105 is a resolver error, and resolving stops before types are checked, so this step comes only after the V0200 file is fixed)
- [ ] write integration test: save a single file outside any package and expect its diagnostics; open a module beside a single-file program's `main.vr` and expect no V0106
- [ ] write integration test: a request before `initialize` and an unknown method get their errors
- [ ] read the child's frames on a thread with a timeout so a silent server fails rather than hangs, and parse every byte of stdout as frames, which proves checking prints nothing there (spec §2); to show nothing was published (no V0106), send a request right after the notification and expect its answer as the next frame, since the server handles messages in order
- [ ] gate: `cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`

### Task 9: Update the documents and add the grammar rule

Spec §8.

**Files:**
- Modify: `AGENTS.md`, `CONTRIBUTING.md`, `README.md`, `docs/language.md`, `docs/roadmap.md`, `docs/design.md`

- [ ] `AGENTS.md`: add the "The grammars are documents too" bullet after "Keep every document current", word for word from spec §8; in "Where things are" add `lsp.rs` (the `varyk lsp` server) and the repositories `Varyk-Lang/varyk-editors` and `Varyk-Lang/tree-sitter-varyk`
- [ ] `CONTRIBUTING.md` "Where things are": add the two repositories, pointing to `AGENTS.md` for the rule
- [ ] `README.md`: `varyk lsp` in the main commands of "Try it"; a short "Editors" part there (the VS Code and Zed extensions, JetBrains through TextMate bundles, `varyk lsp` for any other LSP editor); this spec in "Documents"
- [ ] `docs/language.md`: `varyk lsp` in the command table; an "Editors" section after "The command line" and its subsections, before "Error codes", saying what the server does and that it checks on open and save; in "Not in milestone 5c" replace "a language server" with hover and completion
- [ ] `docs/roadmap.md` milestone 6: split the language server line into "stage 1: diagnostics and quick fixes on save" (checked) and "hover, go-to-definition, completion; needs error recovery in the parser" (open)
- [ ] `docs/design.md` decisions log: a row for the language server (`varyk lsp` in the compiler, checking on open and save, no new dependency)
- [ ] gate: `cargo test -p varyk --test cli --test reference` (the programs of `docs/language.md` still pass `check`, and its code still names every keyword and diagnostic code)

### Task 10: Run the final gate

Spec §9 definition of done.

**Files:** none

- [ ] run `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, and `CARGO_TERM_COLOR=always cargo test --workspace` from a clean tree
- [ ] run `rustup run 1.88 cargo build --workspace --locked`
- [ ] by hand: pipe an `initialize` request framed with `Content-Length` into `cargo run -p varyk -- lsp` and see its capabilities and `serverInfo` on stdout; follow with `shutdown` and `exit` and see status 0
- [ ] read every document of Task 9 against the finished code; grep the branch for attribution lines before pushing
- [ ] gate: all of the above green
