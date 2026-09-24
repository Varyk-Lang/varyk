# Varyk design notes

These are the principles Varyk is built on and the decisions made so far,
with the reason for each. The full design, including how the compiler works,
is in [specs/2026-09-23-varyk-design.md](specs/2026-09-23-varyk-design.md).

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
   language goes toward someone who has never programmed, then toward the
   developer who has never written Rust. AI agents are first-class writers of
   Varyk. Where their needs and human readability diverge, human readability
   wins.
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

## Stability

Before 0.1, everything may change. Diagnostic codes are stable in one sense
from the start: a code, once assigned, is never reused for a different
meaning, though it may be retired. Command-line flags and the layout of the
generated Rust have no stability guarantee before 0.1.

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
| Build orchestration | `varyk` drives `cargo` first, a `build.rs` later | diagnostics stay under Varyk's control |
| Generated code formatting | the compiler emits readable code; rustfmt only for display | builds must not depend on rustfmt |
| Rust-layer errors | shown as rustc reports them for now, mapped to Varyk source later | mapping is real work and not needed to prove the idea |
| rustc warnings | silenced for the whole generated crate for now, per file later | generated code warns routinely; per-file filtering can wait |
| Diagnostics renderer | `annotate-snippets` | maintained by the Rust project; no renderer to own |
| Concurrency | Rust async, JavaScript surface, built-in runtime | the Rust ecosystem is already async |
| License | MIT or Apache-2.0 | Rust ecosystem convention |
| Learnability | designed to be learnable without a programming background; plain-word diagnostics | the audience Varyk is meant to grow, not only Rust or JavaScript developers |
| AI agents | first-class writers, humans win on conflicts | Rust knowledge transfers; the removed syntax is where models fail; structured diagnostics close the loop |
