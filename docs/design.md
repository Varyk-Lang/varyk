# Varyk design notes

Varyk is a small language for backend services, APIs, workers, and
microservices, that compiles to Rust: Go-like application code that ships as
a native binary with Rust's safety, speed, and ecosystem. It runs on the
Rust ecosystem in the same way TypeScript runs on the JavaScript ecosystem;
the analogy is about the ecosystem relationship, not the grammar, and Varyk
is not a superset of Rust. The choice Varyk competes in is the one a team
makes for a service, between Go, TypeScript, Python, and Rust, so Varyk
targets services first, the space Go occupies, and standalone binaries
second. Ownership inference is how Varyk delivers that, not what it sells.

These are the principles Varyk is built on and the decisions made so far,
with the reason for each. The full design, including how the compiler works,
is in [specs/2026-09-23-varyk-design.md](specs/2026-09-23-varyk-design.md),
milestone 2's additions are in
[specs/2026-09-25-milestone-2-design.md](specs/2026-09-25-milestone-2-design.md),
milestone 3's in
[specs/2026-09-26-milestone-3-design.md](specs/2026-09-26-milestone-3-design.md),
milestone 4's in
[specs/2026-09-29-milestone-4-design.md](specs/2026-09-29-milestone-4-design.md),
milestone 5b2's (packages) in
[specs/2026-10-02-milestone-5b2-design.md](specs/2026-10-02-milestone-5b2-design.md),
milestone 5b3's (facades for packages) in
[specs/2026-10-03-milestone-5b3-design.md](specs/2026-10-03-milestone-5b3-design.md),
milestone 5b4's (`varyk-http` and the golden path) in
[specs/2026-10-05-milestone-5b4-design.md](specs/2026-10-05-milestone-5b4-design.md),
and milestone 5c's (time, ids, and bytes) in
[specs/2026-10-07-milestone-5c-design.md](specs/2026-10-07-milestone-5c-design.md).

## Principles

In priority order. When two conflict, the earlier one wins.

1. **Rust's safety model, unchanged.** No garbage collector. No implicit
   `Clone`. No implicit deep copy. The compiler inserts exactly two kinds of
   allocation: a string literal placed into an owned slot (a struct field, a
   return value, a variable that must own its string, or a Rust function
   parameter of type `String`) is converted at that line, and a string
   passed as a trailing value of a facade (milestone 5b3) is copied into
   the value it is handed over in; `--emit-rust` shows both. A `Bytes`
   passed as a trailing value (milestone 5c) is lent, not copied: the
   value adds one to the reference count of its shared buffer and
   allocates nothing. Aliasing rules are preserved. The generated Rust is
   checked by rustc, and Varyk never works around rustc with unsafe code.
2. **The service developer first, human or agent.** Every tie-breaker on
   the surface language goes toward the developer building services who has
   never written Rust. Learnability is a value in its own right: the language and
   its diagnostics should be learnable without a Rust background. AI agents
   are first-class writers of Varyk. Where their needs and human readability
   diverge, human readability wins.
3. **Rust syntax with sigils inferred.** Functions borrow their arguments by
   default. Mutation is declared in the function contract with `mut`.
   References are never written at call sites. Lifetimes are inferred wherever
   the compiler can infer them.
4. **One way to do each thing.** Go's discipline. A small, regular grammar
   with one obvious spelling per idea is the most useful property a language
   can have for both a newcomer and a code generator.
5. **Predictable cost.** Passing a value to a Varyk-declared function never
   allocates. Storing a string literal into an owned slot allocates once, at
   the line where the literal is written. Moves follow Rust's rules, and the
   diagnostics for moved values are a first-class feature.
6. **Gradual adoption.** A Varyk package can contain Rust files and depend on
   Cargo crates. Escape hatches live in Rust files, not in new Varyk syntax.
7. **Reuse Cargo, build only the compiler.** The manifest, dependency
   resolution, registry, lockfile, features, workspaces, caching,
   cross-compilation, code generation, optimization, and the full borrow check
   all come from Cargo and rustc. Varyk owns the front end, the Rust emitter,
   and diagnostics.
8. **Generated Rust is the first backend, not necessarily the last.** The
   front end never depends on the backend.
9. **Advanced features must justify their complexity.** Go-like simplicity is
   the bar for anything added to the surface language.

## Packages and batteries

`varyk-std` stays the small runtime every program has: what the language
itself needs and the light things nearly every service uses (`Error`,
tasks, `json`, `env`, `log`, and `Time`, `Uuid`, and `Bytes`). Anything heavy is a package the writer adds:
SQL, MongoDB, Redis, the HTTP server and client. A program that prints a
line never compiles a web server or a database driver. Official packages
are named `varyk-*` and community packages `*-varyk` (`TRADEMARKS.md`
draws the line), with neither `std` nor the crate underneath in the name.
The mechanism comes first: with packages in place, a battery is written
once as a package, by this project or by anyone, and needs no work in the
compiler. The one exception is `varyk-http`, which the compiler knows by
crate name (milestone 5b4): checking a route against its handler needs a
function named as an argument, which nothing else in the language has.

Varyk code is built by `varyk`, as Go code is built by `go`. A package is
`Cargo.toml`, its `.vr` files, and its `.rs` facades; it has no `build.rs`
and no stub, and `varyk` compiles every Varyk package a build uses from its
`.vr` sources, never from Rust a publisher shipped. Cargo still has to
read the package (`cargo add`, `cargo update`, `cargo tree`, `cargo
audit`, dependency bots), and it refuses a manifest with no target, so the
manifest names the `.vr` root as the target. Plain `cargo build` does not
build a source package, but it builds a published Varyk crate, which
carries its generated Rust, so a Rust project can depend on one as on any
crate.

## Non-goals

- Varyk is not a superset of Rust and does not aim to accept arbitrary Rust
  syntax in `.vr` files.
- No garbage collector, ever.
- No separate package registry, package manifest, or build system. Cargo and
  crates.io are the only ones.
- No inline Rust blocks inside `.vr` files. Rust goes in `.rs` files.
- No commitment to a native backend. The compiler keeps the option open; the
  roadmap does not promise it.
- Not every Rust niche. Embedded and `no_std` targets are out of scope.
- Not for kernels, database engines, custom allocators, or borrow-heavy
  libraries; those stay in `.rs` files in the same build.
- Not a road to all of Rust. Varyk does not add a mechanism so that a
  program which belongs in Rust can be written in Varyk; the `.rs` file
  beside it is the answer, and "use Varyk until you need Rust" is the
  promise.

## Stability

Varyk is experimental and pre-1.0: anything may change before 1.0; a
new feature or a breaking change bumps the minor version. Diagnostic codes are stable in one
sense from the start: a code, once assigned, is never reused for a different
meaning, though it may be retired. Command-line flags and the layout of the
generated Rust have no stability guarantee before 1.0.

## Decisions log

| Decision | Choice | Why |
|---|---|---|
| Name | Varyk | Lithuanian for "go!", the imperative of *varyti*; unclaimed on crates.io, npm, and PyPI at the time of writing, with no repository of that name on GitHub; does not contain "Rust" |
| Extension | `.vr` | short, "var" mnemonic, unclaimed |
| String type | `string`, lowercase, one type | newcomer first; owned versus borrowed is a compiler decision |
| String allocation | only a literal placed into an owned slot converts, at that line; from 5b3, a string trailing value is also copied, by `Value::from`, as it is handed over; a `Bytes` trailing value (5c) only adds to a reference count, since `varyk-std` builds every `Bytes` in the `bytes` crate's shared form | predictable allocations, visible in `--emit-rust`; no hidden copies |
| Borrowed values | a value a function only borrows cannot be stored into a struct or returned, for now | the alternative is a hidden copy; lifetime inference comes later |
| Assignment | Rust move semantics; `string` is never `Copy` | preserves Rust's model; diagnostics carry the burden |
| Parameter passing | borrow by default, `mut` for mutable borrow | keeps Rust's ownership model while taking its bookkeeping out of everyday code |
| Copy types | numbers and `bool` are passed by value, and from 5c `Time` and `Uuid` | observably identical to a borrow, no indirection |
| Modules | `mod name;` resolves to `.vr` or `.rs`, `pub` for visibility, one file per module in the generated crate | Rust's rule; Varyk and Rust modules are symmetric |
| Rust interop | `.rs` files as modules, plus Cargo crates; no inline Rust blocks | the TypeScript and JavaScript model; one grammar per file |
| Manifest | `Cargo.toml` | reuse Cargo entirely; no Varyk manifest |
| Build orchestration | `varyk` drives `cargo` first; a package's `Cargo.toml` names its `.vr` root, and only `varyk` builds a Varyk package (milestone 5b2 removed `build.rs`, the stub, and `varyk emit`) | diagnostics stay under Varyk's control |
| Generated code formatting | the compiler emits readable code; rustfmt only for display | builds must not depend on rustfmt |
| Rust-layer errors | a line-level source map reports a rejection of Varyk's own generated code as a Varyk diagnostic, V0900, at the Varyk line responsible; a rejection inside a user's own `.rs` module passes through verbatim at their file | the two-layer model's promised net (M1 §6.5): generated-code failures read in Varyk's terms, the user's own Rust stays in the user's terms (M3 §5) |
| rustc warnings | `#[allow(warnings, arithmetic_overflow, unconditional_panic)]` on every generated item, none on copied `.rs` files, and none on `mod` lines but `#[allow(non_snake_case)]` for a `.vr` module whose name is not in snake case (M3) | one tree for all three build destinations, works under `include!`, the user's own `.rs` warnings stay visible; verified with rustc |
| Diagnostics renderer | `annotate-snippets` | maintained by the Rust project; no renderer to own |
| Concurrency | Rust async, JavaScript surface, built-in runtime (settled in 5b1 below) | the Rust ecosystem is already async |
| License | MIT or Apache-2.0 | Rust ecosystem convention |
| Learnability | designed to be learnable by a developer building services who has never written Rust; plain-word diagnostics | backend developers coming from Go, TypeScript, or Python come first, then developers coming from Rust, then AI agents |
| AI agents | first-class writers, humans win on conflicts | agents already write good code and the slow part is reviewing it; a small, concrete language with one way to do each thing keeps agent output reviewable, rustc checks its safety, and structured diagnostics close the loop |
| Milestone 2 scope | language core only; packages, closures, and tooling later | four independent areas do not fit one MVP; a real program needs enums and collections before it needs dependencies |
| MVP cuts | borrowed returns, struct variants, nested and literal patterns, `Option`/`Result` methods, most `Vec` and `string` methods, `..=`, `?` on `Option`, and field `pub` (added in milestone 3) deferred | no example needs them; `match` is the one way to look inside an `Option` or `Result`; one-level patterns make exhaustiveness exact |
| String copy | `s.clone()`, strings only | Rust's own spelling; the cost is visible at the call; no implicit clone anywhere |
| Closures | milestone 4: only as arguments of built-in calls; never a value; shared captures only; one untyped parameter | no generics means no Varyk function can take one; shared captures keep chains free of new borrow-flow rules; the item type is always known |
| String joining | `format!` only; `+` rejected with a fix-it | one spelling, allocation visible at the call; Rust's `+` consumes its left side |
| Borrowed returns | inferred in milestone 4: every return part of one read-only parameter; one lifetime written when elision would not name it; the call result is an alias of the argument | closes M1 §4.4 without new syntax; getters stop copying; two roots, a `mut` root, and mixing stay errors, over-strict and sound |
| Matching a place | never moves; non-Copy bindings are aliases, Copy ones are copied at arm entry; a temporary is owned | borrow by default, applied to `match` and `for` |
| Exhaustiveness | checked by Varyk: every variant or a catch-all | plain-word diagnostics before cargo runs; exact, since patterns are one level deep |
| Standard types | `Option`, `Result`, `Vec` with a hand-written table of six methods and indexing | no generics in the surface; the table is small, explicit, and sound by construction |
| Lengths and indexes | `usize` added, no casts | matches Rust and rustc's types; `as` is lossy and can wait |
| Type holes | `None`, `Vec::new()`, an empty `vec![]`, `Ok`, `Err` take the expected type or ask for an annotation | the integer-literal rule, no backward inference |
| Crates from Varyk | a Rust crate is never named; used through `.rs` facades in the package (a Varyk package is named, from 5b2) | most crate APIs are generic; a facade is where the concrete API is written; keeps check-passes-means-compiles; principle 6 |
| `use` | package paths only; `use` lines are emitted as canonical `crate::` paths and every use site is written with its full path, so the aliases never affect the generated Rust (M3, amended from "verbatim"); no crates, braces, globs, variants | convenience without committing the keyword to a crate-import meaning before traits exist |
| Build model | hidden crate for `varyk build`; assembled plain crate for publish (no `build.rs` or stub in a source package, from 5b2) | Varyk keeps its diagnostics; consumers need no `varyk` |
| Edition | 2024 required in package mode | the hidden crate and plain cargo must compile the same code the same way |
| Package mode detection | a file argument that is a package's crate root | no flag; `varyk run src/main.vr` gets the dependencies |
| Visibility | Rust's rule for modules, items, and fields; `pub mod` added; no `pub(crate)` | generated Rust must resolve; one rule to teach |
| Field privacy | private unless `pub`; breaking for M2 cross-module field reads | libraries need invariants; matches imported Rust structs; breaking, so the next release is 0.1.0 |
| Rust import | structs with named fields, same-file inherent methods, unit-and-tuple enums; derives ignored | what a facade needs and nothing more; opaque otherwise, as M1 |
| Nested `.rs` modules | cut | a facade is one file; removes the `syn` tree walk |
| `check` and cargo | `check` parses `Cargo.toml` itself and runs cargo only to learn the package graph, when a package lists a dependency besides `varyk-std` (5b2) | a program that passes `check` builds; a package with no other dependency never needs cargo |
| Milestone 4 scope | language only; `varyk fmt` and the interop leftovers to milestones 5 and 6 | two independent areas, as in milestone 2; the formatter needs comment-preserving syntax work of its own |
| Chains | one expression from source to terminal; unfinished chains have no type; a source needs a stored receiver or a string literal | the iterator types have no Varyk spelling; nothing is lost, since a chain cannot be observed before it ends |
| Items | borrowed, copies, or owned, decided at the source and by `map`; `collect` needs owned or copies | mirrors `for` and `match` on places and temporaries; a `Vec` of borrowed values has no Varyk type |
| Look-inside results | `get` and `find` on borrowed items open only in a `match`/`if let`/`while let` head; on a number or `bool` payload they are a plain `Option` of a copy | an `Option<&T>` has no Varyk spelling; the bindings are aliases exactly as in a `match` on what the payload is part of, so nothing new is needed; numbers copy everywhere else |
| Number items | a chain over stored numbers or `bool` copies them at the source | the same copy `for` and `match` already make; downstream closures take plain values |
| `parse` | returns `Option<T>`, `T` from the expected type (amended in 5a: returns `Result<T, Error>`) | Rust's error type has no Varyk name and no more information; `ok_or` gives the `Result` |
| `unwrap`, `expect` | never added, a principle rather than a cut | a call that stops the program on an absent value defeats the purpose of a language for services; `match`, `if let`, `?`, `unwrap_or` cover every use |
| Derives | `Clone` and `PartialEq` automatic where every field allows, `Debug` not | zero run-time cost, `.clone()` stays the one visible copy, `==` on enums is everyday code; `{:?}` does not exist |
| `HashMap` | in the table; `get` and `insert`, no indexing, no direct `for` | Rust's map cannot be assigned through an index; pairs need tuples |
| `as` | number types only | the `usize` friction of M2 §2.7 was real; anything else has a method |
| Reachability | a useless arm is an error | the check is free once exhaustiveness is exact; rustc's warning is silenced in generated code and an unreachable arm is a bug |
| Chars | no `char` type, no `chars()` | services split and trim strings; a character type is its own design |
| Standard calls | a table of declared signatures, never pass-through of unknown Rust methods and never signatures read from `std` | Varyk must know each call's type, receiver mode, borrow, and allocation to write the Rust and keep plain-word errors; `std` signatures need generics and traits to read; a facade is the pass-through |
| Milestone 5 split (5a) | 5a data, config, logging, tests; 5b1 async; 5b2 HTTP and database | three subsystems; each slice testable on its own |
| Standard surface | rows in the standard table, `varyk-std` as the implementation | the calls are generic or format-checked, which the importer cannot read; one existing mechanism |
| Crate access | `varyk-std` reached without a facade; every other crate still through one | the M3 rule stays meaningful; the standard modules are Varyk's own |
| Names | `json::parse` and `json::stringify` | JavaScript's names; `parse` already means text into a value |
| Error | one built-in `Error` with a message | `?` across every standard call without conversions, as Go and JavaScript have one error type |
| Serde derivation | derived only where a `json` or `env` call reaches, per direction | no serde in programs that do not use it; the error lands at the call |
| Attributes | `#[rename]`, `#[default]`, `#[skip]`, `#[test]`, unprefixed, a fixed list | real APIs need renamed and keyword keys, config needs defaults, secrets need skipping; only the compiler defines attributes |
| Enums in JSON | unit-only, as strings; data enums refused | a designer writes a `type` field explicitly; no Rust-shaped format leaks into APIs |
| Configuration | `env::parse` into a struct; environment, then `.env`, then default | Node dotenv's order; one mechanism with JSON; `.env` never written to the process |
| Logging | four calls in `println!` form on tracing; `LOG`, `LOG_FORMAT=json` | readable in a terminal, ingestible in production, no code change between them |
| Tests | `#[test]` functions in place; `assert` only in tests | maps to Rust's `#[test]` and `cargo test`; stopping the program is for tests only |
| Versioning | a version range in `Cargo.toml`, minor and lock checked by `varyk check` | upgrades are `cargo update`; a mismatch is a diagnostic, never a rustc error |
| Single files | depend on `varyk-std` at the compiler's exact version | there is no manifest to respect |
| TOML | not in 5a | nothing on the golden path needs it; the same machinery adds it later |
| Waiting (5b1) | explicit `async fn` and `.await` | familiar to JavaScript and Rust writers and to models; a waiting point is visible |
| Starting (5b1) | a call without `.await` starts a task at once; no `spawn` | JavaScript's eager promises; two unawaited calls really overlap |
| Runtime (5b1) | multi-threaded tokio inside `varyk-std` | tokio, the HTTP server, and the database need `Send` anyway; tasks spread over cores |
| Ownership transfer (5b1) | a started call's arguments are owned slots; no new syntax | the only place a value outlives its function; parameters still borrow |
| Dropped tasks (5b1) | cancelled; `detach` to keep running | no work leaks past the code that started it; a forgotten `.await` is an error, not a background job |
| Waiting on many (5b1) | `Task::all` (first `Err` cancels the rest) and `Task::all_settled` (every outcome) | `Promise.all` and `Promise.allSettled`; two names keep cancellation visible |
| Names (5b1) | `Task` reserved; `todo`'s `Task` renamed | `Task::all` reads as in other languages; a type with associated functions follows `Vec::new()` |
| Shared values (5b1) | `Shared<T>` of a struct, read-only, `Arc` underneath, in parameters and `let`s only | one value for thousands of tasks without a copy each; the application state of `varyk-http` |
| Where tasks live (5b1) | made by a `let`, `.detach()`, `vec!`, or collected `map`; only awaited, detached, or given to `Task::all` or `Task::all_settled` | no task is dropped by accident or moved out of a borrowed place |
| Async recursion (5b1) | refused | Rust needs a boxed future for it; nothing in the examples recurses |
| Concurrency model (5b1) | a task is a Rust future spawned on the tokio runtime `varyk_std::run` starts; `Task<T>` wraps its join handle and aborts on drop; every Varyk type is `Send` and `Sync`, so a `.rs` type that is not is refused at build (V0901) | the model runs on the Rust ecosystem unchanged, and the writer never sees `Send`, `Sync`, or `Pin` |
| Milestone 5 split (5b2) | 5b2 packages; 5b3 facades and `varyk-sql`; 5b4 `varyk-http` and the golden path | batteries become packages, so the mechanism comes first; each slice testable on its own |
| Batteries (5b2) | `varyk-std` is the small runtime every program has (`Error`, tasks, `json`, `env`, `log`); anything heavy is a package the writer adds | a program compiles only what it uses; anyone can write a battery |
| Package names (5b2) | `varyk-*` official, `*-varyk` community; no `std`, no backing crate in the name | `TRADEMARKS.md` already draws the line; the crate underneath may change |
| Name in code (5b2) | the `[dependencies]` key, `-` read as `_` | the writer chooses it; no second naming scheme; it is the crate name in the generated Rust |
| What a package is (5b2) | a dependency with `src/lib.vr` | no marker to forget or forge |
| How a package is compiled (5b2) | by the driver, from its `.vr` and its `.rs` modules, into a crate it writes | shipped Rust and build scripts could differ from the Varyk a reviewer reads |
| Who builds Varyk (5b2) | `varyk` only; no `build.rs` or stub in a package; `varyk emit` removed | fewer files, and no build-script path to make safe; published crates still serve cargo users |
| A package's target (5b2) | `Cargo.toml` names the `.vr` root (`[lib] path = "src/lib.vr"`) | cargo's tools (`add`, `update`, `tree`, `audit`) need a target to read the manifest; no Rust file needed |
| Published form (5b2) | generated Rust with the `.vr` beside it, the target naming the generated root | cargo users and docs.rs need no `varyk` |
| A Varyk package reached another way (5b2) | refused, V0401: `[dev-dependencies]`, `optional`, through a Rust crate | cargo would build it itself, outside the guarantee; one rule, no exceptions |
| Reading a package (5b2) | the full check on its sources, its `pub` items imported | borrowed returns are inferred from bodies |
| Asking cargo for the graph (5b2) | in every command, when any dependency besides `varyk-std` is listed, on a manifest in its own directory | a program that passes `check` builds; the graph is the build's own |
| JSON on another package's type (5b2) | refused, V0210; convert it in its own package | serde is derived where a type is declared |
| A type from a package not depended on (5b2) | refused, V0115 | the generated Rust must be able to name it |
| Facades for packages (5b3) | a `.rs` signature may take a type parameter filled from where the result goes, a last `Vec<varyk_std::Value>` written as trailing values, literal-only `&'static str` text, and `varyk_std::Error`; `Value` is scalars only, built by the compiler without serde; `varyk-sql` lives in its own repository | a package such as `varyk-sql` keeps its Rust in a facade, and its users write no Rust; with scalars no conversion can fail, so no failure is hidden as `Null` or turned into a crash; `varyk-sql`'s version follows sqlx and the databases as much as the compiler |
| HTTP (5b4) | `varyk-http`, in its own repository, on axum with tower-http, its client on reqwest with rustls; the compiler knows it by crate name, under any key: the route and hook calls on its `App` are intrinsics, each route checked against its handler and compiled to one adapter that names only the package's items, never axum; a handler's parameters bound by name to the path and query, by type to the body, the state, and the package's own types; its return value is the response; `Error` gains an optional status, and an error without one is a 500 whose message is logged, not sent | Varyk has no function values, so some part of a route table must be known to the compiler, and knowing one package by name is smaller than a general mechanism; axum is in the top tier of TechEmpower round 23, runs on the tokio runtime `varyk-std` already starts, has the middleware a real API needs, and is maintained by the tokio organisation Varyk already bets on; the compiler writes every adapter with concrete types, so axum's trait errors never reach a writer; with one binding rule for the package's own types, WebSockets, server-sent events, and uploads need no compiler change |
| Time, ids, and bytes (5c) | `Time` (a point in time in UTC, in microseconds), `Uuid` (`Uuid::new()` a version 7, `Uuid::v4()` the random one by name), and `Bytes` (immutable, base64 in JSON) are built-in types, each its own variant in the compiler; the three reach JSON, comparison, trailing values, and facade signatures, and `Time` and `Uuid` also `env`, `parse`, printing, and route parameters; `varyk-std` holds them on `time`, `uuid`, `bytes`, and `base64`, with `bytes` its one public dependency, and every program that uses `varyk-std` builds the four | a users API stores `created_at` as a time, not a string; a built-in type goes through the lists `Error` went through, where a shared scalar variant would be a new abstraction `Bytes` does not fit and an imported Rust struct could not reach JSON, printing, or comparison without an exception in each; microseconds are what Postgres and MySQL hold, so a time read back equals the one written; axum gives a body as a `bytes::Bytes`, and handing it over must not copy it |
