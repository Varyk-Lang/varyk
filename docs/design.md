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
and milestone 4's in
[specs/2026-09-29-milestone-4-design.md](specs/2026-09-29-milestone-4-design.md).

## Principles

In priority order. When two conflict, the earlier one wins.

1. **Rust's safety model, unchanged.** No garbage collector. No implicit
   `Clone`. No implicit deep copy. The compiler inserts exactly one kind of
   allocation: a string literal placed into an owned slot (a struct field, a
   return value, a variable that must own its string, or a Rust function
   parameter of type `String`) is converted at that line, and `--emit-rust`
   shows it. Aliasing rules are preserved. The generated Rust is checked by
   rustc, and Varyk never works around rustc with unsafe code.
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
| Milestone 4 scope | language only; `varyk fmt` and the interop leftovers to milestones 5 and 6 | two independent areas, as in milestone 2; the formatter needs comment-preserving syntax work of its own |
| Chains | one expression from source to terminal; unfinished chains have no type; a source needs a stored receiver or a string literal | the iterator types have no Varyk spelling; nothing is lost, since a chain cannot be observed before it ends |
| Items | borrowed, copies, or owned, decided at the source and by `map`; `collect` needs owned or copies | mirrors `for` and `match` on places and temporaries; a `Vec` of borrowed values has no Varyk type |
| Look-inside results | `get` and `find` on borrowed items open only in a `match`/`if let`/`while let` head; on a number or `bool` payload they are a plain `Option` of a copy | an `Option<&T>` has no Varyk spelling; the bindings are aliases exactly as in a `match` on what the payload is part of, so nothing new is needed; numbers copy everywhere else |
| Number items | a chain over stored numbers or `bool` copies them at the source | the same copy `for` and `match` already make; downstream closures take plain values |
| `parse` | returns `Option<T>`, `T` from the expected type | Rust's error type has no Varyk name and no more information; `ok_or` gives the `Result` |
| `unwrap`, `expect` | never added, a principle rather than a cut | a call that stops the program on an absent value defeats the purpose of a language for services; `match`, `if let`, `?`, `unwrap_or` cover every use |
| Derives | `Clone` and `PartialEq` automatic where every field allows, `Debug` not | zero run-time cost, `.clone()` stays the one visible copy, `==` on enums is everyday code; `{:?}` does not exist |
| `HashMap` | in the table; `get` and `insert`, no indexing, no direct `for` | Rust's map cannot be assigned through an index; pairs need tuples |
| `as` | number types only | the `usize` friction of M2 §2.7 was real; anything else has a method |
| Reachability | a useless arm is an error | the check is free once exhaustiveness is exact; rustc's warning is silenced in generated code and an unreachable arm is a bug |
| Chars | no `char` type, no `chars()` | services split and trim strings; a character type is its own design |
| Standard calls | a table of declared signatures, never pass-through of unknown Rust methods and never signatures read from `std` | Varyk must know each call's type, receiver mode, borrow, and allocation to write the Rust and keep plain-word errors; `std` signatures need generics and traits to read; a facade is the pass-through |
