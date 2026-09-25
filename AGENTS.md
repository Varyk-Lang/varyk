# Working on Varyk

This file is for anyone, human or AI agent, making changes to this
repository. It is public and short on purpose: the rules that keep the
project coherent, and where to find everything else.

## What this is

Varyk is an experimental programming language that compiles to Rust. The
compiler is a Rust workspace: `crates/varyk-syntax` (lexer, parser, AST) and
`crates/varyk` (diagnostics, resolver, type checker, borrow analysis, Rust
backend, cargo driver, CLI). The design lives in `docs/specs/`; read the
current spec before changing what the language accepts or how it compiles.

## The gate

Every change must pass, from a clean tree:

```sh
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

CI runs the same on stable and on the minimum supported Rust version, 1.85.
Do not use language features newer than 1.85 (let-chains, for example).
The twelve programs under `examples/` must keep building and printing their
expected output; `crates/varyk/tests/examples.rs` checks that.

## Rules that are not obvious from the code

- **Minimum viable.** Prefer the smallest change that fixes the problem.
  Do not add mechanisms, abstractions, flags, or dependencies that no
  example, no required diagnostic, and no documented behavior needs. If a
  fix wants a new mechanism, open an issue first.
- **Fail loudly.** Anything the language does not support gets a named
  diagnostic, never a panic and never silently wrong Rust. A Varyk program
  that passes `varyk check` should compile under rustc; when you find one
  that does not, fix the check, not the docs.
- **The Rust layer is rustc's job.** A `.rs` module is copied into the
  generated crate as it is. `varyk check` reads its public function
  signatures and rejects the few things that can never build there (a
  dependency, a file include, a nested module file); it does not validate
  the Rust inside, and it is not meant to. `varyk build` is the authority
  for Rust code, and a `.rs` file that rustc rejects is a bug in that file.
  Do not extend the importer to chase further ways Rust code can fail.
- **No hidden allocation.** The compiler inserts exactly one allocation, a
  string literal placed into an owned slot. Never solve an ownership
  problem by emitting `.clone()` or `.to_string()` on anything else.
- **Diagnostics have stable codes** (`crates/varyk/src/diagnostics/codes.rs`).
  A code is never reused for another meaning. A new diagnostic needs a
  fixture under `crates/varyk/tests/fixtures/errors/<code>_<slug>/`, a
  registration in `crates/varyk/tests/errors.rs`, and a line in the code
  list at the end of `docs/language.md`. Messages are written in plain
  words for someone who has never programmed; Rust vocabulary goes in a
  note, not the headline.
- **Keep the reference in sync.** `docs/language.md` changes in the same
  pull request as any change to what the compiler accepts.
- **Snapshots are reviewed content.** Regenerate with `INSTA_UPDATE=always`
  only for the tests you meant to change, read the new `.snap` files, and
  make sure the gate passes without the variable afterwards.
- **Identifiers are ASCII** and every Rust keyword is reserved, so a Varyk
  name is always a valid Rust name in generated code.

## Commits and releases

Every commit on `main` has a conventional prefix: `feat:` and `fix:` bump
the version and go in the changelog; `docs:`, `test:`, `ci:`, `chore:`,
`refactor:` do not; `!` after the prefix marks a breaking change. One
concern per commit: squash-merge a single-concern pull request under a
conventional title, rebase-merge a multi-concern one with a conventional
commit per concern. release-please turns the history on `main` into a
release pull request; merging that tags `varyk-vX.Y.Z` and
`varyk-syntax-vX.Y.Z` and publishes both crates after approval in the
`release` environment. Never create release tags by hand. Details and the
full prefix table are in `CONTRIBUTING.md`.

## Where things are

- `docs/specs/` the design the compiler is built from, with its decisions log
- `docs/language.md` the one-page reference, including every diagnostic code
- `docs/roadmap.md` what comes next; `docs/plans/` implementation plans and
  the follow-ups left from each milestone
- `docs/open-questions.md` design questions deliberately not yet answered
- `SECURITY.md`, `TRADEMARKS.md`, `LICENSE-MIT`, `LICENSE-APACHE`
