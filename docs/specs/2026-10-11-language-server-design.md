# Varyk language server, stage 1

Date: 2026-10-11.

**Status.** Implemented, not yet released. `varyk lsp` is a Language Server
Protocol server built into the compiler: it shows `varyk check`'s
diagnostics in an editor, on open and on save, and offers each fix-it as a
quick fix. Its clients are the editor extensions of
`Varyk-Lang/varyk-editors` (`docs/specs/2026-10-11-varyk-editors-design.md`
there, "editors §n"), whose highlighting comes from the grammars of that
repository and of `Varyk-Lang/tree-sitter-varyk`
(`docs/specs/2026-10-11-tree-sitter-varyk-design.md` there, "tree-sitter
§n"). This spec also adds the rule that keeps those grammars current with
the compiler. Varyk is experimental and pre-1.0: anything here may change.

## 1. Summary and scope

A developer saves a file in VS Code or Zed; the editor's Varyk extension
runs `varyk lsp`, which checks the program the file belongs to and
underlines every diagnostic at its span, with the message, the notes, and
the code a reader can look up in `docs/language.md`. A diagnostic with a
fix-it offers it as a quick fix.

Stage 1 is diagnostics and quick fixes only. Hover, go-to-definition,
completion, and semantic tokens need a parser that recovers from errors
and a checker that returns what it learned despite them; they stay in
milestone 6 with `varyk fmt`, which needs a richer syntax tree too.

### 1.1 Decisions

- **Inside the compiler, as `varyk lsp`.** One binary to install, and the
  server can never disagree with `varyk check` about a program, because it
  is `varyk check`. Gleam does the same (`gleam lsp`).
- **No new dependency.** Stage 1 reads nine messages and writes two;
  JSON-RPC with `Content-Length` framing over `serde_json`, which the
  compiler already uses, is about 150 lines. `lsp-server` and `lsp-types`
  are the choice to revisit when hover and completion arrive.
- **On open and on save, from disk.** The checker reads modules from disk,
  and on save the disk is what the editor shows. Following edits as they
  are typed needs a layer that serves unsaved buffers to the resolver, and
  a checker fast enough to run per keystroke; neither is stage 1.
- **The command-line output does not change.** The refactor in section 4.3
  splits checking from printing; `varyk check` prints what it printed
  before, byte for byte, and its tests prove it.

## 2. The command

`varyk lsp` takes no arguments besides the global `--message-format`,
which it ignores. It speaks LSP over standard input and output until the
client sends `exit` or closes standard input, and exits 0 after a
`shutdown` request, 1 otherwise, as LSP asks. It is listed in
`varyk --help` and in the command table of `docs/language.md`. The server
is one new module, `crates/varyk/src/lsp.rs`.

Standard output belongs to the protocol. Nothing the server runs may
print there: checking never does (section 9 tests it), and a log line the
server writes for a person goes to standard error, which editors show in
their output panel.

## 3. Protocol

Messages are JSON-RPC 2.0, each framed by a `Content-Length` header and a
blank line; other headers are read and ignored. A body that is not JSON
is logged to standard error and skipped. A header part the server cannot
read (no `Content-Length`, or one that is not a number) leaves no way to
find where the next message starts, so it is logged and ends the server,
as a closed standard input does.

| Message | Kind | What the server does |
|---|---|---|
| `initialize` | request | answers with the capabilities below and `serverInfo` (`varyk`, its version) |
| `initialized` | notification | nothing |
| `textDocument/didOpen` | notification | checks the file's program (section 4) |
| `textDocument/didChange` | notification | marks the document changed since its last save; the changes are not read (section 6) |
| `textDocument/didSave` | notification | checks the file's program |
| `textDocument/didClose` | notification | forgets the document was changed (section 6); its diagnostics stay until the next check |
| `textDocument/codeAction` | request | answers with the quick fixes in range (section 6) |
| `shutdown` | request | answers `null`; later requests get `InvalidRequest` |
| `exit` | notification | exits |

Any other request gets the error `MethodNotFound`, and any other
notification, `$/cancelRequest` among them, is ignored. A
request before `initialize` gets `ServerNotInitialized`.

The capabilities:

- `textDocumentSync`: `openClose: true`, `change: 2` (incremental, the
  smallest messages; only their arrival matters), `save: {}`.
- `codeActionProvider`: `codeActionKinds: ["quickfix"]`.
- `positionEncoding` is left out, which means UTF-16, the protocol's
  default and the one every client supports.

## 4. Checking

### 4.1 When

On `didOpen` and `didSave` of a `file:` URI whose path ends in `.vr`. Other
schemes and extensions are ignored; a `Cargo.toml` or `.rs` save does not
check, and the next `.vr` save picks the change up. A URI's path is
percent-decoded to a file path (a space is `%20`), and a path is
percent-encoded back when diagnostics are published. A source path the
checker spells relative (a dependency's files are shown relative to the
working directory) is made absolute against the server's working
directory first.

### 4.2 What

The program the file belongs to, found as `varyk check` finds it:

- `package::find` from the file's directory upward. When the `Cargo.toml`
  it finds has `src/main.vr` or `src/lib.vr`, the server checks that
  package, as `varyk check` run in its directory does.
- Otherwise a single-file program, whose entry is chosen below.

A `Cargo.toml` without either root is not a package; the file is checked
as a single-file program, which is what a stray `.vr` file next to a Rust
crate wants.
Outside a package nothing names a file's entry. A module of a
single-file program (`examples/modules/math.vr`) checked on its own would
show V0106 for a `main` it is not meant to have, so for a file not named
`main.vr` the server first looks for a `main.vr` in the file's directory
and then in each parent, stopping after the workspace folder that holds
the file, of those the client named in `initialize` (only the file's own
directory when none holds it, so the walk never reaches past what the
user opened), and checks that `main.vr` when it finds one, as `varyk check main.vr`
does; else it checks the file alone. A file the found program does not
reach as a module gets no diagnostics from that check: a module not yet
declared with `mod`, which is checked once it is, or a stray file under a
package or below an unrelated `main.vr`.

### 4.3 How

`load` in `crates/varyk/src/cli.rs` reads the entry, reads the package
graph, runs `check_file`, and checks the build's `varyk-std`, printing any
diagnostics on the way. It splits in two:

- **`check_target`**, which does the same work and returns what `load`
  returns on success, or a `Failure`: diagnostics with the sources they
  point into, or a plain message (the entry cannot be read). It prints
  nothing.
- **`load`**, the existing function, now a wrapper that calls it and
  prints a failure as before.

Once `docs/specs/2026-10-11-std-minimum-design.md` gives `load` the
`varyk-std` rule to apply, `check_target` takes it too, and the server
passes the rule of every command but `varyk publish`.

Locating splits the same way: the file branch of `locate_for`
(`Package::for_file`, then a `Target`) moves into a function that returns
the manifest's diagnostics instead of printing them. Its other branch
cannot serve the server, since it looks upward from the current
directory. The server calls the new function with the package's root file
(`package::root_of`) when section 4.2 found a package, which loads that
package, or else with the entry section 4.2 chose (the `main.vr` it
found, or the saved file), which `for_file` leaves to be checked alone.

The server holds no compiler state between checks: each one reads the
files and the package graph afresh. `cargo metadata` runs on every check
of a package with dependencies, which on save is fast enough; caching it
waits for checks per keystroke.

A failure that is not a diagnostic (the entry cannot be read; a bad
manifest is diagnostics, published at its `Cargo.toml`) is sent as
`window/showMessage` of type warning, with the message `varyk check` would print.

## 5. Diagnostics

Each `Diagnostic` becomes one LSP diagnostic at its primary span's file:

- `range`: the span's byte offsets, converted to line and UTF-16 column
  in the span's `SourceFile` text. A line ends at `\n`; a `\r` before it
  needs nothing special, since a column before it is the same either way
  and a client clamps one past a line's end. An offset past the end of the
  text clamps to the end.
- `severity`: 1 (error); every Varyk diagnostic is an error.
- `source`: `"varyk"`; `code`: the diagnostic's code (`"V0101"`).
- `message`: the diagnostic's message, then each note on its own line as
  `note: …`, the way the terminal shows them.
- `relatedInformation`: one entry per label, at the label's span with its
  text, always; VS Code and Zed both show it, and a client that cannot
  ignores the field, so the server reads no client capability for it.

Diagnostics are published per file, grouped by the file of their primary
span: a `.vr` file, a `.rs` facade, a `Cargo.toml`, or a file of a
dependency package (which `check_file` already notes as such). Every check
publishes, for each file it found diagnostics in, the full list, and for
each file that had diagnostics after the previous check of the same
program and has none now, an empty list. Programs are told apart by their
root directory, or by their entry file's path outside a package (so
saving `math.vr` and saving its `main.vr` are one program). A file checked
as one program leaves every other program that published for it or kept
its fix-its (a `main.vr` or a package appeared around it): each drops the
file, and the file gets an empty list before the new check's lists unless
the new check publishes for it, so a diagnostic from a program that will
not check it again does not stay. A program that was the file checked
alone drops everything its check published, so a V0106 it no longer has
goes too. A file two
programs share (a path dependency's module, checked on its own and as
part of the package that depends on it) shows what the latest check
published for it; both read the same files from disk, so they rarely
disagree, and merging their lists waits for a user who needs it.

## 6. Quick fixes

The server keeps the fix-its of the last check of each program, with the
diagnostic they belong to. A `textDocument/codeAction` request for a
document answers with one action per diagnostic published there that has
a fix-it and whose range overlaps or touches the requested range (a
cursor is an empty range). A fix-it may edit another file than the one
its diagnostic is in: a missing `pub` is added at the item's
declaration, in the module that declares it.

- `kind`: `"quickfix"`; `isPreferred`: `true`;
- `diagnostics`: the LSP diagnostic it fixes;
- `edit`: a `WorkspaceEdit` with one `TextEdit` in the fix-it's file,
  replacing the fix-it's range, converted as in section 5 against that
  file's text, with its replacement;
- `title`: `Replace with `…``, `Insert `…`` (an empty range), or
  `Remove this` (an empty replacement); a replacement longer than 40
  characters or spanning lines is shortened to its first 40 characters
  of its first line and `…`.

Diagnostics in a dependency package carry no fix-it (`check_file` drops
them), so no action ever edits a dependency.

A fix-it's range is a byte range of the file as last checked, so it is
offered only while the editor's text is that file: when the diagnostic's
document or the fix-it's file is open and changed after its last save (a
`didChange` since its last save or open, or a `didOpen` whose text differs from the file
on disk, as a restored unsaved buffer does), there is no action until it
is saved again; and when the fix-it's file on disk is no longer the text
the check read (a module changed outside the editor, or a `Cargo.toml`,
whose changes the client never sends), there is no action until the next
check. An edit at stale offsets
would silently corrupt the text.

## 7. Robustness

- **One thread.** Messages are handled in order; a check runs to the end
  before the next message is read. A check takes well under a second on
  the examples, and it runs on save.
- **A panic is a compiler bug, not a dead server.** Each check runs under
  `std::panic::catch_unwind`. A panic publishes nothing new, sends
  `window/showMessage` of type error ("the Varyk compiler crashed while
  checking this program; please report it with the file that caused it"),
  and the server goes on. The panic message itself reaches standard
  error through the default hook.
- **No `unwrap`.** A malformed message is an error response or a log
  line, never a panic.

## 8. Documentation and the rule that keeps the grammars current

The grammars that highlight Varyk live outside this repository: the
TextMate grammar in `varyk-editors` (editors §3), which the website copies
into `varyk.com/syntaxes/varyk.json`, and the Tree-sitter grammar in
`tree-sitter-varyk`. Their CI parses this repository's programs and
compares its keyword list every night (editors §7, tree-sitter §5), so a
grammar that falls behind goes red within a day; the rule below makes
catching up part of the change that caused it.

`AGENTS.md` gains a bullet after "Keep every document current":

- **The grammars are documents too.** The syntax of Varyk is described in
  `docs/language.md`, the TextMate grammar of `Varyk-Lang/varyk-editors`
  (which the website copies to `varyk.com/syntaxes/varyk.json`), and the
  Tree-sitter grammar and queries of `Varyk-Lang/tree-sitter-varyk`. A
  change to what the lexer or parser accepts (a keyword, a literal form,
  an operator, a comment form, a new item, statement, expression,
  pattern, or type) updates `docs/language.md` in the same pull request
  and comes with pull requests to those repositories; a red nightly drift
  check there is a bug in the grammar.

and "Where things are" gains the two repositories and `varyk lsp`'s
module. `CONTRIBUTING.md`'s "Where things are" gains the two repositories
too; the rule itself stays in `AGENTS.md`, to which `CONTRIBUTING.md`
already points.

Other documents:

- `README.md`: `varyk lsp` in the main commands of "Try it", and a short
  "Editors" part there: the two extensions, JetBrains through TextMate
  bundles, and `varyk lsp` for any other editor that speaks LSP; and
  this spec in "Documents".
- `docs/language.md`: `varyk lsp` in the command table, a short
  "Editors" section after "The command line" saying what the server
  does and when it checks, and "Not in milestone 5c" naming hover and
  completion where it now says "a language server".
- `docs/roadmap.md`, milestone 6: the language server line splits into
  "stage 1: diagnostics and quick fixes on save" (done) and "hover,
  go-to-definition, completion; needs error recovery in the parser" (open).
- `docs/design.md`: a row in the decisions log (the server is `varyk
  lsp` in the compiler, checking on save); `docs/open-questions.md`
  gains nothing.

## 9. Testing and definition of done

Unit tests in the server module:

- frame reading and writing: one message, two in one read, a header
  other than `Content-Length`, a bad length, a body that is not JSON,
  end of input;
- offset to position: ASCII, a multibyte character before the offset (two
  UTF-8 bytes, one UTF-16 unit), a character outside the basic plane (four
  bytes, two units), `\r\n` line ends, an offset at the end of the text;
- URI to path and back: a space, a non-ASCII name;
- a `Diagnostic` with notes and labels to its LSP form;
- fix-it titles: replace, insert, remove, a long replacement, a
  multi-line one.

An integration test, `crates/varyk/tests/lsp.rs`, starts the built binary
(`CARGO_BIN_EXE_varyk`) in a temporary package and plays a session:
initialize; open a file with a type error and expect its diagnostic with
code, range, and notes; ask for code actions at a fix-it and get its edit;
ask at a use of an item declared without `pub` and get an edit in the
declaring module;
send a change without saving and get no actions there;
write the fixed file, save, and expect an empty list for it; save a
single file outside any package and expect its diagnostics; open a module
beside a single-file program's `main.vr` and expect no V0106; shutdown and
exit with status 0. A second test feeds a request before `initialize` and
an unknown method and expects their errors. The package test runs offline:
it has no dependency besides `varyk-std`, so no `cargo metadata` runs.

The existing `varyk check` tests and snapshots pass unchanged, which is
the proof the refactor in section 4.3 kept the output.

Done when the gate passes from a clean tree (`cargo fmt --check`, clippy
with `-D warnings`, `cargo test --workspace`), the workspace builds on
the minimum Rust version (`rustup run 1.88 cargo build --workspace`), and
the documents of section 8 are current.
Trying the server in the editors is the extensions' own definition of
done (editors §7), so this spec does not wait on another repository.

## 10. Release

A `feat:` commit. The next release is 0.9.0 (Rust 1.88 as the minimum,
and the `varyk-std` rule of `docs/specs/2026-10-11-std-minimum-design.md`);
this ships in it when it lands before that release, and otherwise in the
next minor, whose number then takes 0.9.0's place below and in editors §4.
`varyk-std` and `varyk-syntax` move with it, the three versions being
linked. Under the `varyk-std` rule a compiler release needs no release of
`varyk-http`, `varyk-sql`, or `varyk-mongo`: their `varyk-std` lines on
0.8 build with it unchanged. The VS
Code extension starts the server only when `varyk lsp --help` succeeds
(editors §4); the Zed extension does not
check, and an older `varyk` fails on `lsp` in Zed's log (editors §5).

## 11. Not in this version

Following unsaved edits; hover, go-to-definition, completion, semantic
tokens (Zed and VS Code both read them now, but the TextMate and
Tree-sitter grammars already colour everything stage 1 could), formatting; watching files the editor does not save; caching the
package graph; a JetBrains plugin to start the server (editors §6); rustc's
warnings about `.rs` modules, which only `varyk build` produces.

## 12. Decisions

- 2026-10-11: the server is `varyk lsp` in the compiler, stage 1 is
  diagnostics and quick fixes on open and save, with no new dependency.
- 2026-10-11: the grammars live in `Varyk-Lang/tree-sitter-varyk` and
  `Varyk-Lang/varyk-editors`, kept current by their nightly drift checks
  and the `AGENTS.md` rule of section 8.
