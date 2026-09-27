# Roadmap

The living plan for Varyk. Milestones are ordered; items within a milestone
are not. Design rationale lives in `docs/specs/`. Each milestone gets an
implementation plan in `docs/plans/` when work on it starts.

Varyk is experimental and pre-1.0: anything may change, and a breaking
change bumps the minor version. Nothing here is a release commitment or a
date. An item in progress is marked in its text.

## Milestone 1: compiler skeleton and the borrow-by-default proof

Spec: `docs/specs/2026-09-23-varyk-design.md`.

- [x] Workspace with `varyk-syntax` and `varyk` crates, stable toolchain, MSRV 1.85 checked in CI, edition 2024
- [x] Lexer and recursive-descent parser with spans on every node
- [x] Resolver, type checker, and borrow analysis (modes, places, moves, per-binding string representation) producing HIR
- [x] `println!` intrinsic with placeholder-count check
- [x] `mod name;` resolving to `.vr` or `.rs` modules, `pub` visibility, `crate::` paths
- [x] Import of top-level `pub fn` signatures from `.rs` modules via `syn`
- [x] `RustBackend` writing a Cargo project under `target/varyk/`, with `[workspace]` table, `mod` declarations, crate-wide warning allows, and `--emit-rust`
- [x] Driver invoking `cargo build` with a shared target directory and passing rustc errors through with a note
- [x] Diagnostics with `Vnnnn` codes, fix-its, `annotate-snippets` rendering, `--message-format=json`
- [x] Rust-habit diagnostics: `&x` at call sites, `&T` in parameters, `String`/`str`, lifetimes
- [x] CLI: `varyk check`, `varyk build`, `varyk run`, `--release`
- [x] Six examples building and running with expected output
- [x] Unit tests, `insta` snapshots, integration tests, CI (fmt, clippy, test)
- [x] `README.md`, `docs/design.md`, `docs/language.md`, `docs/open-questions.md`, dual license

Done when every item above is checked. The full definition of done is
section 7 of the spec.

## Milestone 2: enums, matching, methods, and collections

Spec: `docs/specs/2026-09-25-milestone-2-design.md`. The language core only;
the former milestone-2 items on packages and tooling moved to milestones 3
and 4.

- [x] Representation refactor: borrow analysis records each local's final Rust representation, the backend reads it; functions in one `Vec` by `FnId`
- [x] Alias rule: a `let` from a parameter, field, or element, a pattern binding, and a `for` item are aliases; changing or giving away the root while an alias is still used is an error
- [x] Enums with unit and tuple variants, `pub enum`, module paths to variants
- [x] `match` on enums, `Option`, and `Result` with one-level variant, name, and `_` patterns, and Varyk-side exhaustiveness
- [x] `for` over integer ranges and `Vec`
- [x] `impl` blocks with `self` and `mut self` methods and associated functions
- [x] `Option`, `Result`, `Vec` in types, `Some`/`None`/`Ok`/`Err`/`Vec::new()`/`vec!`, expected-type inference for `None`, `Vec::new()`, `Ok`, `Err`
- [x] The standard table: `Vec::new`, `push`, `pop`, `len`, indexing, and `clone` and `len` on strings
- [x] `usize`
- [x] `?` on a `Result` with the function's error type
- [x] `format!` and `vec!` intrinsics
- [x] Module types nameable as `m::Type`, `m::Enum::Variant`, `m::Type::new()`
- [x] New diagnostics (spec section 6) with plain-word messages
- [x] Six examples building and running with expected output; soundness templates for every new construct
- [x] `docs/language.md`, `docs/design.md`, and `docs/open-questions.md` updated

Done when every item above is checked. The full definition of done is
section 7 of the spec.

## Milestone 3: packages and interop

Spec: `docs/specs/2026-09-26-milestone-3-design.md`. Varyk code never names
a crate; a dependency is used from a `.rs` facade in the same package.

- [x] `Cargo.toml` as the package manifest, `src/main.vr` or `src/lib.vr` as the root, edition 2024, `[package.metadata.varyk]` reserved
- [x] Cargo dependencies, reached from `.rs` facades; `Cargo.lock` shared with cargo
- [x] Nested modules and `mod.vr` directories, `pub mod`, and Rust's visibility rule
- [x] `use` for paths inside the package
- [x] Field-level `pub`
- [x] Import of Rust structs, their inherent methods, and enums from `.rs` modules
- [x] Rust-layer errors mapped to Varyk source through a line-level source map (V0900); the user's `.rs` errors and warnings shown at their file; item-level lint allows on generated code
- [x] `varyk init` with a `build.rs` so plain `cargo build` works, and `varyk emit`
- [x] `varyk publish`: a plain Rust crate with the generated `.rs` and the `.vr` sources included
- [x] Three example packages building and running with expected output; soundness templates for every new construct
- [x] `docs/language.md`, `docs/design.md`, `docs/open-questions.md`, and `docs/roadmap.md` updated
- [ ] Site at varyk.com: the pitch, `borrowing.vr` beside its generated Rust, getting started, install via `cargo install varyk`, the language reference

Done when every item above but the site is checked; the site is a separate
sub-project in its own repository. The full definition of done is section
10 of the spec.

## Milestone 4: closures, iterators, and tooling

- [ ] Closures and function types
- [ ] Iterator adapters on `Vec` and `string` (`iter`, `map`, `filter`, `chars`, `split`, `parse`)
- [ ] Borrowed return values inferred from the body, explicit lifetimes in the generated Rust
- [ ] `if let` and `while let`
- [ ] Enum variants with named fields, nested and literal patterns, `match` on numbers and strings, `..=`, `?` on `Option`
- [ ] The `Option` and `Result` methods and the rest of the `Vec` and `string` methods
- [ ] `HashMap`, `Box`, `as`, `clone` on structs and enums
- [ ] `varyk fmt`, a deterministic formatter on `varyk-syntax`
- [ ] Test asserting `docs/language.md` covers every construct the parser accepts
- [ ] Nested modules in `.rs` files
- [ ] Import of Rust tuple and unit structs
- [ ] Derives on imported Rust structs (`Clone`, `PartialEq`, `Debug`) usable from Varyk
- [ ] `.rs` signatures naming Varyk-declared types
- [ ] Direct import of a published Varyk library from Varyk

## Milestone 5: batteries for services

Gated on three open questions in the spec: serde derivation (automatic or
attribute syntax), the async runtime shape, and ownership-transfer syntax.

- [ ] `varyk-std` crate
- [ ] Derivation of serde traits for Varyk structs, per the open question
- [ ] JSON via serde
- [ ] Logging via tracing
- [ ] HTTP server on a proven Rust crate, chosen at that time
- [ ] HTTP client on the same stack
- [ ] Databases through one API; sqlx is the candidate crate
- [ ] Configuration from the environment
- [ ] `varyk test`
- [ ] `async`/`await` on a built-in tokio runtime, `spawn` as a built-in, `Send`/`Sync`/`Pin` kept out of the surface syntax and their failures mapped to Varyk diagnostics

## Milestone 6: tooling and beyond

- [ ] Language server on `varyk-syntax`
- [ ] Decision on a native backend behind the `Backend` trait

## Unscheduled

Declaring generics, traits, and attributes in Varyk code. Each waits on an
open question in the spec.

Using a crate directly from Varyk code, with no facade `.rs` module in
between. Milestone 3 reaches every crate through a facade, because most
crate APIs are generic and Varyk has no generics in its surface; whether
Varyk code should ever `use` a crate directly is an experiment for after
traits exist, recorded in [open-questions.md](open-questions.md). The facade
rule stands until that experiment says otherwise.
