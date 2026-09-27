# Varyk design notes

Varyk is a programming language for backend services with Rust-like safety
and Go-like simplicity. It compiles to Rust and runs on the Rust ecosystem,
in the same way TypeScript compiles to JavaScript and runs on the JavaScript
ecosystem; the analogy is about the ecosystem relationship, not the grammar,
and Varyk is not a superset of Rust. Varyk targets services first, the space
Go occupies, and standalone binaries second.

These are the principles Varyk is built on and the decisions made so far,
with the reason for each. The full design, including how the compiler works,
is in [specs/2026-09-23-varyk-design.md](specs/2026-09-23-varyk-design.md),
milestone 2's additions are in
[specs/2026-09-25-milestone-2-design.md](specs/2026-09-25-milestone-2-design.md),
and milestone 3's in
[specs/2026-09-26-milestone-3-design.md](specs/2026-09-26-milestone-3-design.md).

## Principles

In priority order. When two conflict, the earlier one wins.

1. **Rust's safety model, unchanged.** No garbage collector. No implicit
   `Clone`. No implicit deep copy. The compiler inserts exactly one kind of
   allocation: a string literal placed into an owned slot (a struct field, a
   return value, a variable that must own its string, or a Rust function
   parameter of type `String`) is converted at that line, and `--emit-rust`
   shows it. Aliasing rules are preserved. The generated Rust is checked by
   rustc, and Varyk never works around rustc with unsafe code.
2. **Newcomer first, human or agent.** Every tie-breaker on the surface
   language goes toward the developer building services who has never
   written Rust. Learnability is a value in its own right: the language and
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

## Stability

Varyk is experimental and pre-1.0: anything may change before 1.0; a
breaking change bumps the minor version. Diagnostic codes are stable in one
sense from the start: a code, once assigned, is never reused for a different
meaning, though it may be retired. Command-line flags and the layout of the
generated Rust have no stability guarantee before 1.0.

## Decisions log

| Decision | Choice | Why |
|---|---|---|
| Name | Varyk | Lithuanian for "go!", the imperative of *varyti*; unclaimed on crates.io, npm, and PyPI at the time of writing, with no repository of that name on GitHub; does not contain "Rust" |
| Extension | `.vr` | short, "var" mnemonic, unclaimed |
| String type | `string`, lowercase, one type | newcomer first; owned versus borrowed is a compiler decision |
| String allocation | only a literal placed into an owned slot converts, at that line | one predictable allocation, visible in `--emit-rust`; no hidden copies |
| Borrowed values | a value a function only borrows cannot be stored into a struct or returned, for now | the alternative is a hidden copy; lifetime inference comes later |
| Assignment | Rust move semantics; `string` is never `Copy` | preserves Rust's model; diagnostics carry the burden |
| Parameter passing | borrow by default, `mut` for mutable borrow | keeps Rust's ownership model while taking its bookkeeping out of everyday code |
| Copy types | numbers and `bool` are passed by value | observably identical to a borrow, no indirection |
| Modules | `mod name;` resolves to `.vr` or `.rs`, `pub` for visibility, one file per module in the generated crate | Rust's rule; Varyk and Rust modules are symmetric |
| Rust interop | `.rs` files as modules, plus Cargo crates; no inline Rust blocks | the TypeScript and JavaScript model; one grammar per file |
| Manifest | `Cargo.toml` | reuse Cargo entirely; no Varyk manifest |
| Build orchestration | `varyk` drives `cargo` first; milestone 3 added the `build.rs` that `varyk init` writes for plain `cargo build` | diagnostics stay under Varyk's control |
| Generated code formatting | the compiler emits readable code; rustfmt only for display | builds must not depend on rustfmt |
| Rust-layer errors | a line-level source map reports a rejection of Varyk's own generated code as a Varyk diagnostic, V0900, at the Varyk line responsible; a rejection inside a user's own `.rs` module passes through verbatim at their file | the two-layer model's promised net (M1 §6.5): generated-code failures read in Varyk's terms, the user's own Rust stays in the user's terms (M3 §5) |
| rustc warnings | `#[allow(warnings, arithmetic_overflow, unconditional_panic)]` on every generated item, none on copied `.rs` files, and none on `mod` lines but `#[allow(non_snake_case)]` for a `.vr` module whose name is not in snake case (M3) | one tree for all three build destinations, works under `include!`, the user's own `.rs` warnings stay visible; verified with rustc |
| Diagnostics renderer | `annotate-snippets` | maintained by the Rust project; no renderer to own |
| Concurrency | Rust async, JavaScript surface, built-in runtime | the Rust ecosystem is already async |
| License | MIT or Apache-2.0 | Rust ecosystem convention |
| Learnability | designed to be learnable by a developer building services who has never written Rust; plain-word diagnostics | backend developers coming from Go, TypeScript, or Python come first, then developers coming from Rust, then AI agents |
| AI agents | first-class writers, humans win on conflicts | Rust knowledge transfers; the removed syntax is where models fail; structured diagnostics close the loop |
| Milestone 2 scope | language core only; packages, closures, and tooling later | four independent areas do not fit one MVP; a real program needs enums and collections before it needs dependencies |
| MVP cuts | borrowed returns, struct variants, nested and literal patterns, `Option`/`Result` methods, most `Vec` and `string` methods, `..=`, `?` on `Option`, and field `pub` (added in milestone 3) deferred | no example needs them; `match` is the one way to look inside an `Option` or `Result`; one-level patterns make exhaustiveness exact |
| String copy | `s.clone()`, strings only | Rust's own spelling; the cost is visible at the call; no implicit clone anywhere |
| Closures | deferred to milestone 4 | without generics no Varyk function can take one; they pay off only with iterator adapters |
| String joining | `format!` only; `+` rejected with a fix-it | one spelling, allocation visible at the call; Rust's `+` consumes its left side |
| Borrowed returns | deferred to milestone 4; `.clone()` is the milestone 2 answer | a performance feature, not a capability; removes lifetime emission and return classification from the MVP |
| Matching a place | never moves; non-Copy bindings are aliases, Copy ones are copied at arm entry; a temporary is owned | borrow by default, applied to `match` and `for` |
| Exhaustiveness | checked by Varyk: every variant or a catch-all | plain-word diagnostics before cargo runs; exact, since patterns are one level deep |
| Standard types | `Option`, `Result`, `Vec` with a hand-written table of six methods and indexing | no generics in the surface; the table is small, explicit, and sound by construction |
| Lengths and indexes | `usize` added, no casts | matches Rust and rustc's types; `as` is lossy and can wait |
| Type holes | `None`, `Vec::new()`, an empty `vec![]`, `Ok`, `Err` take the expected type or ask for an annotation | the integer-literal rule, no backward inference |
| Crates from Varyk | never named; used through `.rs` facades in the package | most crate APIs are generic; a facade is where the concrete API is written; keeps check-passes-means-compiles; principle 6 |
| `use` | package paths only; `use` lines are emitted as canonical `crate::` paths and every use site is written with its full path, so the aliases never affect the generated Rust (M3, amended from "verbatim"); no crates, braces, globs, variants | convenience without committing the keyword to a crate-import meaning before traits exist |
| Build model | hidden crate for `varyk build`; `build.rs` + `include!` stub from `init` for plain cargo; assembled plain crate for publish | Varyk keeps its diagnostics; cargo tooling works unchanged; consumers need no `varyk` |
| Edition | 2024 required in package mode | the hidden crate and plain cargo must compile the same code the same way |
| Package mode detection | a file argument that is a package's crate root | no flag; `varyk run src/main.vr` gets the dependencies |
| Visibility | Rust's rule for modules, items, and fields; `pub mod` added; no `pub(crate)` | generated Rust must resolve; one rule to teach |
| Field privacy | private unless `pub`; breaking for M2 cross-module field reads | libraries need invariants; matches imported Rust structs; breaking, so the next release is 0.1.0 |
| Rust import | structs with named fields, same-file inherent methods, unit-and-tuple enums; derives ignored | what a facade needs and nothing more; opaque otherwise, as M1 |
| Nested `.rs` modules | cut | a facade is one file; removes the `syn` tree walk |
| `check` and cargo | `check` never runs cargo, package mode included | M1 rule kept; TOML is parsed directly |
