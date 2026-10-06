# Working on Varyk

This file is for anyone, human or AI agent, making changes to this
repository. It is public and short on purpose: the rules that keep the
project coherent, and where to find everything else.

## What this is

Varyk is an experimental programming language for backend services that
compiles to Rust. The compiler is a Rust workspace: `crates/varyk-syntax`
(lexer, parser, AST), `crates/varyk` (diagnostics, resolver, type
checker, borrow analysis, Rust backend, cargo driver, CLI), and
`crates/varyk-std` (the runtime that generated programs call: errors,
JSON, configuration, logging, the async runtime and tasks). The design lives
in `docs/specs/`; read the current spec before changing what the language
accepts or how it compiles.

## The gate

Every change must pass, from a clean tree:

```sh
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

CI runs the same on stable and on the minimum supported Rust version, 1.85.
Do not use language features newer than 1.85 (let-chains, for example).
The programs under `examples/`, single files and the packages in
`examples/packages/`, must keep building and printing their expected
output; `crates/varyk/tests/examples.rs` checks that.

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
  generated crate as it is. `varyk check` reads its public functions,
  structs, methods, and enums, and rejects the few things that can never
  build there (a crate the package does not depend on, a file include, a
  nested module file); it does not validate
  the Rust inside, and it is not meant to. `varyk build` is the authority
  for Rust code, and a `.rs` file that rustc rejects is a bug in that file.
  Do not extend the importer to chase further ways Rust code can fail.
- **`Cargo.toml` is cargo's job the same way.** `varyk check` reads what
  Varyk needs from it, refuses what no crate Varyk builds could honor (a
  fixed list of keys, workspace inheritance, `[patch]`, `[lints]`, extra
  targets, ...), and checks the shape of what it accepts; the syntax of
  values it does not interpret is cargo's to check. Every crate Varyk
  builds uses the package's own manifest (`Package::isolated_manifest`), so
  cargo's verdict is the same under `varyk build` and `varyk publish`, and
  a manifest cargo rejects is a bug in that manifest. Do not chase further
  ways cargo can reject a manifest.
- **The compiler knows `varyk-http` by crate name.** Varyk code reaches a
  crate only through a `.rs` facade in the same package, and the compiler
  knows no package by name but `varyk-std` and one other: when a build
  holds the package whose crate name is `varyk-http`, under any key, or
  when the package being checked is that package, the route and hook calls
  on its `App` are intrinsics, checked against their handlers and compiled
  to adapters that name only the package's items, never axum. What the
  compiler names and checks of the package is section 6 of
  `docs/specs/2026-10-05-milestone-5b4-design.md`; a change there is a
  change to both repositories. Do not extend this to another package
  without a spec.
- **No hidden allocation.** The compiler inserts exactly two allocations:
  a string literal placed into an owned slot, and a string passed as a
  trailing value of a facade's `Vec<varyk_std::Value>` parameter, which
  `Value::from(&str)` copies as it hands the value over. Never solve an
  ownership problem by emitting `.clone()` or `.to_string()` on anything
  else.
- **Safe by default.** Varyk is for services that face the network, so
  security is a design constraint, not a later pass. What runs is what a
  reader can review: do not design a path where Rust that nobody reads,
  such as generated Rust shipped by someone else or a build script of a
  dependency's own, runs in a user's build. A battery keeps data apart
  from code (a value travels beside a SQL query, never pasted into its
  text) and keeps the message of an internal failure away from the client.
  Where the easy way and the safe way differ, the safe way is the default
  and the other is asked for by name.
- **Diagnostics have stable codes** (`crates/varyk/src/diagnostics/codes.rs`).
  A code is never reused for another meaning. A new diagnostic needs a
  fixture under `crates/varyk/tests/fixtures/errors/<code>_<slug>/`, a
  registration in `crates/varyk/tests/errors.rs`, and a line in the code
  list at the end of `docs/language.md`. Messages are written in plain
  words for someone who has never programmed; Rust vocabulary goes in a
  note, not the headline.
- **Keep every document current.** When a change alters what a document
  describes, update that document in the same pull request: `README.md`
  (status, milestone, version, install, usage, features, links),
  `docs/language.md` (any change to what the compiler accepts),
  `docs/design.md`, `docs/roadmap.md`, `docs/open-questions.md`,
  `CONTRIBUTING.md`, or any other doc the change touches. Read the docs
  against the change before every pull request.
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
release pull request; merging that is the release decision: it tags
`varyk-vX.Y.Z`, `varyk-syntax-vX.Y.Z`, and `varyk-std-vX.Y.Z` and publishes
the three crates, with no
further approval. Never create release tags by hand, except one the
release workflow's `tags` job reports missing, at its release commit. Keep
`<` and `>` out of commit subjects, pull request titles, and
`BREAKING CHANGE:` footers (`Option of T`, not `Option<T>`), which CI
checks: release-please reads the release pull request as HTML, and an
unclosed tag there leaves crates untagged. Details and the full prefix
table are in `CONTRIBUTING.md`.

## Where things are

- `crates/varyk/src/` `package.rs` reads `Cargo.toml`; `resolve/` loads
  the module tree and resolves names, `use`, and visibility; `interop/`
  imports `.rs` modules with `syn`; `packages.rs` asks cargo for the
  package graph and finds the Varyk packages of a build; `driver/` writes
  the generated tree, one crate per Varyk package, and runs cargo, `init`,
  and `publish`
- `docs/specs/` the design the compiler is built from, with its decisions log
- `docs/language.md` the one-page reference, including every diagnostic code
- `docs/roadmap.md` what comes next; `docs/plans/` implementation plans and
  the follow-ups left from each milestone
  (`2026-09-23-milestone-1-followups.md`,
  `2026-09-26-milestone-3-followups.md`,
  `2026-09-29-milestone-4-followups.md`,
  `2026-10-05-milestone-5b4-followups.md`)
- `docs/open-questions.md` design questions deliberately not yet answered
- `TRADEMARKS.md`, `LICENSE-MIT`, `LICENSE-APACHE`; the security policy is the
  organization's shared one, shown in the Security tab
