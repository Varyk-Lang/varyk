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
- [x] `varyk init` with a `build.rs` so plain `cargo build` works, and `varyk emit` (superseded in 5b2: no `build.rs`, no stub, and `varyk emit` is removed; `Cargo.toml` names the `.vr` root)
- [x] `varyk publish`: a plain Rust crate with the generated `.rs` and the `.vr` sources included
- [x] Three example packages building and running with expected output; soundness templates for every new construct
- [x] `docs/language.md`, `docs/design.md`, `docs/open-questions.md`, and `docs/roadmap.md` updated
- [ ] Site at varyk.com: the pitch, `borrowing.vr` beside its generated Rust, getting started, install via `cargo install varyk`, the language reference

Done when every item above but the site is checked; the site is a separate
sub-project in its own repository. The full definition of done is section
10 of the spec.

## Milestone 4: closures, iterators, and patterns

Spec: `docs/specs/2026-09-29-milestone-4-design.md`. The language only;
the former milestone-4 tooling and interop items moved to milestones 5
and 6.

- [x] Closures as arguments of built-in calls: untyped parameters, shared captures, never a value
- [x] Iterator chains on stored values: `iter`, `split`, `keys`, `values` sources; `map`, `filter`; `collect`, `count`, `sum`, `any`, `all`, `find`; a chain as a `for` head; items borrowed, copied, or owned
- [x] Borrowed return values inferred from the body, one root parameter, explicit lifetime in the generated Rust only where elision would not name it; the call result an alias of the argument
- [x] `if let` and `while let`
- [x] Enum variants with named fields, nested, literal, and `..=` range patterns, `match` on numbers, `bool`, and strings, exhaustiveness and reachability by Varyk
- [x] `?` on `Option`; expected types flowing through `?`
- [x] The `Option` and `Result` methods (`is_some`, `is_ok`, `is_err`, `unwrap_or`, `map`, `ok_or`, `ok`, `map_err`) and the new `Vec` and `string` rows of the table, `parse` included
- [x] `HashMap` with `get` looked into where it is made
- [x] `as` between number types; `..=` in `for`
- [x] `Clone` and `PartialEq` derived where every field allows, so `.clone()` and `==` work on structs and enums; derive lists read from imported Rust types
- [x] Import of Rust signatures returning `&str` or `&S` where lifetime elision names the root
- [x] Test asserting `docs/language.md` mentions every keyword, built-in type, table call, and diagnostic code
- [x] Six new examples and the `todo`, `interop`, and `matcher` updates building and running with expected output; soundness templates for every new construct
- [x] `docs/language.md`, `docs/design.md`, `docs/open-questions.md`, and `docs/roadmap.md` updated

Done when every item above is checked. The full definition of done is
section 7 of the spec.

## Milestone 5: batteries for services

Milestone 5 is five milestones. The bar for the whole of it is one golden
path: a users API on a database is `varyk init`, `varyk add http sql`, one file,
and `varyk run` away, within fifteen minutes of `cargo install varyk`, and
`varyk build --release` leaves an ordinary native executable. It is met at
the end of 5b4, and promotion waits for it.

### Milestone 5a: data, configuration, logging, and tests

Design: [specs/2026-09-30-milestone-5a-design.md](specs/2026-09-30-milestone-5a-design.md).

- [x] `varyk-std` crate, and the `varyk-std` dependency in every package (`varyk check` reads its version and lock)
- [x] `Error`, one built-in error type with a message, and `parse` returning a `Result`
- [x] Attributes `#[rename]`, `#[default]`, `#[skip]`, and `#[test]`
- [x] Serde derivation for the types a `json` or `env` call reaches
- [x] JSON: `json::parse` and `json::stringify`
- [x] Configuration from the environment and `.env`: `env::parse`
- [x] Logging via tracing: `log::debug`, `info`, `warn`, and `error`
- [x] `varyk test`, with `assert` and `assert_eq`
- [x] `varyk add`, a pass-through to `cargo add`
- [x] Examples `json`, `config`, `logging`, and the `users` package, with `greeting` updated to what `varyk init` writes; `docs/language.md`, `docs/design.md`, `docs/open-questions.md`, and `docs/roadmap.md` updated

### Milestone 5b1: async

Design: [specs/2026-10-01-milestone-5b1-design.md](specs/2026-10-01-milestone-5b1-design.md).

- [x] `async fn`, `.await`, and started calls on a built-in multi-threaded tokio runtime, with `Send`, `Sync`, and `Pin` kept out of the surface
- [x] `Task<T>`: cancelled when dropped, `detach`, `Task::all`, and `Task::all_settled`
- [x] `Shared<T>` for a read-only struct held by many tasks
- [x] `time::sleep`, and `pub async fn` imported from `.rs` modules
- [x] Examples `tasks`, `fanout`, and `shared`; `docs/language.md`, `docs/design.md`, `docs/open-questions.md`, and `docs/roadmap.md` updated

### Milestone 5b2: packages

Design: [specs/2026-10-02-milestone-5b2-design.md](specs/2026-10-02-milestone-5b2-design.md).
HTTP and the database move to packages in 5b3 and 5b4 below: `varyk-std`
stays small, and anything heavy is a package the writer adds.

- [x] A Varyk package named from Varyk code by its `Cargo.toml` key, to any depth
- [x] Varyk packages compiled by `varyk` from their `.vr` sources, never from shipped Rust or a build script
- [x] Packages built only by `varyk`: `Cargo.toml` names the `.vr` root, no `build.rs` or stub from `varyk init`, `varyk emit` removed
- [x] The examples' Rust stops calling `expect`: `Matcher::new` in `examples/packages/matcher/src/text.rs` returns a `Result` the Varyk code handles, and no example package carries the `build.rs` that `varyk init` used to write, so the examples follow the rule that nothing stops a program because a value is absent
- [x] Examples `route` and `trip`; `docs/language.md`, `docs/design.md`, `docs/open-questions.md`, and `docs/roadmap.md` updated

### Milestone 5b3: facades and `varyk-sql`

- [ ] A `.rs` function with one type parameter standing for any Varyk data type (serde's `Serialize` or `DeserializeOwned`), chosen from the argument or from where the result goes
- [ ] A `.rs` function whose last parameter takes any number of data values
- [ ] A `.rs` parameter of type `&'static str` taking only text written in the program
- [ ] `varyk_std::Error` in a `.rs` signature
- [ ] `pub use` in a `.vr` file, and `varyk add` shorthands for official packages
- [ ] `varyk-sql` on sqlx: SQLite, Postgres, and MySQL, each a cargo feature; `sql::connect(url)`, and on a pool `one`, `first`, `all`, and `run`, each taking the query and its values; rows read into structs by column name; each database's own placeholders, passed through; the query text a literal, so a query built from input is a compile error; no secret in an error message

### Milestone 5b4: `varyk-http` and the golden path

- [ ] An HTTP server with an explicit route table (`app.get("/users/{id}", get_user)`); a handler's parameters bound by name to the route, by type to the JSON body and to shared state (a `Shared<T>`); the route checked against the handler by `varyk check`; the return value as the response, `None` as 404
- [ ] `Error` carrying an optional status set by constructors (`http::bad_request(..)`); an error without one is a 500 whose message is logged and not sent
- [ ] An HTTP client in the same package
- [ ] The `users` API on a database, and the fifteen-minute path: `varyk init`, `varyk add http sql`, one file, and `varyk run`
- [ ] An agent evaluation: the examples written by a model from `docs/language.md` alone, pass rates published, before any page claims that agents write Varyk well

After 5b4, `varyk-mongo` and `varyk-redis` are the next packages.

## Milestone 6: tooling and beyond

- [ ] `varyk fmt`, a deterministic formatter on `varyk-syntax` (moved from milestone 4; needs comment-preserving syntax)
- [ ] Language server on `varyk-syntax`
- [ ] Nested modules in `.rs` files
- [ ] Import of Rust tuple and unit structs
- [ ] `Debug` on imported Rust structs, with a `{:?}` placeholder
- [ ] Decision on a native backend behind the `Backend` trait

## Unscheduled

`.rs` signatures naming Varyk-declared types, which the type parameter of
milestone 5b3 makes unnecessary for what a facade needs (moved from 5b2).

Declaring generics, traits, and attributes in Varyk code. Each waits on an
open question in the spec.

TOML, beside the rest of the cuts listed under "Not in milestone 5b2" in
[language.md](language.md): nothing on the golden path needs it, and the
same machinery adds it later.

`Box`, until a program needs a recursive type that `Vec` or `HashMap`
cannot hold, and
the rest of the milestone-4 cut list (spec section 2.13): a character type,
closures as values, tuples, `HashSet`, and the remaining iterator adapters.

Using a crate directly from Varyk code, with no facade `.rs` module in
between. Milestone 3 reaches every crate through a facade, because most
crate APIs are generic and Varyk has no generics in its surface; whether
Varyk code should ever `use` a crate directly is an experiment for after
traits exist, recorded in [open-questions.md](open-questions.md). The facade
rule stands until that experiment says otherwise.
