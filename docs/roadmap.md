# Roadmap

The living plan for Varyk. Milestones are ordered; items within a milestone
are not. Design rationale lives in `docs/specs/`. Each milestone gets an
implementation plan in `docs/plans/` when work on it starts.

Varyk is experimental and pre-0.1. Nothing here is a release commitment or a
date. An item in progress is marked in its text.

## Milestone 1: compiler skeleton and the borrow-by-default proof

Spec: `docs/specs/2026-09-23-varyk-design.md`.

- [ ] Workspace with `varyk-syntax` and `varyk` crates, pinned stable toolchain, edition 2024
- [ ] Lexer and recursive-descent parser with spans on every node
- [ ] Resolver, type checker with per-binding string representation, and borrow analysis (modes, places, moves) producing HIR
- [ ] `println!` intrinsic with placeholder-count check
- [ ] `mod name;` resolving to `.vr` or `.rs` modules, `pub` visibility, `crate::` paths
- [ ] Import of top-level `pub fn` signatures from `.rs` modules via `syn`
- [ ] `RustBackend` writing a Cargo project under `target/varyk/`, with `[workspace]` table, `mod` declarations, crate-wide warning allows, and `--emit-rust`
- [ ] Driver invoking `cargo build` with a shared target directory and passing rustc errors through with a note
- [ ] Diagnostics with `Vnnnn` codes, fix-its, `annotate-snippets` rendering, `--message-format=json`
- [ ] Rust-habit diagnostics: `&x` at call sites, `&T` in parameters, `String`/`str`, lifetimes
- [ ] CLI: `varyk check`, `varyk build`, `varyk run`, `--release`
- [ ] Six examples building and running with expected output
- [ ] Unit tests, `insta` snapshots, integration tests, CI (fmt, clippy, test)
- [ ] `README.md`, `docs/design.md`, `docs/language.md`, `docs/open-questions.md`, dual license

Done when every item above is checked. The full definition of done is
section 7 of the spec.

## Milestone 2: packages, nested modules, enums, and control flow

- [ ] `Cargo.toml` as the package manifest, `[package.metadata.varyk]`
- [ ] Cargo dependencies and `use`
- [ ] Nested modules and `mod.vr` directories
- [ ] Field-level `pub`
- [ ] Enums, `match`, `for`, `impl` blocks
- [ ] Using generic standard types (`Option`, `Result`, `Vec`) without declaring generics, and `?`
- [ ] Closures
- [ ] Lifetime inference for borrowed return values
- [ ] An explicit string copy spelling
- [ ] Import of Rust structs from `.rs` modules
- [ ] Rust-layer errors mapped to Varyk source through a source map; per-file handling of rustc warnings
- [ ] Test asserting `docs/language.md` covers every construct the parser accepts
- [ ] `varyk init` with a `build.rs` so plain `cargo build` works
- [ ] Publishing a Varyk library to crates.io with generated `.rs` included
- [ ] `varyk fmt`, a deterministic formatter on `varyk-syntax`

## Milestone 3: batteries for services

Gated on three open questions in the spec: serde derivation (automatic or
attribute syntax), the async runtime shape, and ownership-transfer syntax.

- [ ] `varyk-std` crate
- [ ] Derivation of serde traits for Varyk structs, per the open question
- [ ] JSON via serde
- [ ] Logging via tracing
- [ ] HTTP server on a proven Rust crate, chosen at that time
- [ ] `async`/`await` on a built-in tokio runtime, `spawn` as a built-in, `Send`/`Sync`/`Pin` kept out of the surface syntax and their failures mapped to Varyk diagnostics

## Milestone 4: tooling and beyond

- [ ] Language server on `varyk-syntax`
- [ ] Decision on a native backend behind the `Backend` trait

## Unscheduled

Declaring generics, traits, and attributes in Varyk code. Each waits on an
open question in the spec.
