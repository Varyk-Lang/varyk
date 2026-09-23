# Varyk: language and compiler design

Date: 2026-09-23.

**Status.** Design for milestone 1. Nothing is implemented yet. Varyk is
experimental and pre-0.1: any syntax, diagnostic code, or command-line flag in
this document may change until a 0.1 release, which is not scheduled.
Discussion happens in the issues of this repository. Until milestone 1 lands,
the design itself is the thing to comment on.

This is the design specification the compiler is built from. Shorter derived
documents, `docs/design.md`, `docs/language.md`, and `docs/open-questions.md`,
are planned and listed in the repository layout below. Until they exist, this
file and `docs/roadmap.md` are the only references.

## 1. Summary

Varyk is a systems programming language with Rust-like safety and Go-like
simplicity. It compiles to Rust and runs on the Rust ecosystem, in the same way
TypeScript compiles to JavaScript and runs on the JavaScript ecosystem.

The name is Lithuanian for "go!" or "go on!", the imperative of *varyti*.
Files use the `.vr` extension. The compiler binary is `varyk`. The compiler is
written in Rust.

The analogy is about the ecosystem relationship, not the grammar. TypeScript
is a superset of JavaScript; Varyk is not a superset of Rust. Rust code lives
in `.rs` files next to Varyk code, and the two build together.

The central idea: Rust-shaped source in which ordinary function calls do not
require the programmer to write borrowing. Ownership, borrowing, and lifetimes
exist inside the compiler and in the safety guarantees. They rarely appear in
everyday code.

Varyk targets services first, the space Go occupies, and standalone binaries
second.

Varyk requires a stable Rust toolchain installed through rustup. `varyk build`
and `varyk run` invoke `cargo`.

## 2. Positioning

This section describes the language Varyk is designed to become. Section 4
lists what milestone 1 implements, and section 8 schedules the rest. Varyk
must be attractive to three audiences at once.

A developer arriving from JavaScript, TypeScript, Go, or Python sees:

- native speed and memory safety without a garbage collector;
- `async`/`await` that looks like JavaScript, with a built-in runtime
  (milestone 3);
- one string type, no reference sigils, no lifetime annotations;
- one toolchain and one package registry, inherited from Cargo and crates.io
  (milestone 2);
- the TypeScript story: adopt gradually, drop to the underlying language when
  needed, publish packages other people consume without knowing the source
  language (milestone 2).

A Rust developer sees:

- Rust syntax and Rust idioms: immutability by default, `let mut`, `struct`,
  and from milestone 2 `impl`, `match`, `Option`, `Result`, and `?`;
- the same safety model: the generated Rust is checked by rustc, and Varyk
  never bypasses it;
- less ceremony for application code;
- `.rs` files next to `.vr` files in the same package, built by the same
  `cargo`;
- Varyk packages are Cargo packages and publish to crates.io as ordinary
  crates (milestone 2).

An AI coding agent sees:

- Rust syntax, so what a model learned from Rust transfers, minus the parts of
  Rust that models most often get wrong: which `&` to write at a call site,
  `&mut` versus `&`, lifetime annotations, `String` versus `&str`;
- no `unsafe` in the surface language, so every generated program is checked
  by rustc;
- structured, machine-readable diagnostics with codes and fix-its, so a
  generate-compile-fix loop has something precise to act on;
- diagnostics that recognize Rust habits (`&user`, `user: &User`, `String`,
  `<'a>`) and say exactly what to change;
- a language reference short enough to fit in a prompt;
- one way to do each thing; `varyk check` for surface errors without invoking
  cargo, `varyk build` for the full check.

### Prior art

The idea of declaring a parameter's passing convention in the signature and
inferring the rest is not new. The closest relatives, and the one-line
difference from each:

- **Mojo** has `read`, `mut`, and `owned` argument conventions with the same
  shape as Varyk's parameter modes, on its own compiler and runtime with
  Python syntax. Varyk targets the Rust ecosystem and Rust syntax.
- **Hylo** (formerly Val) is built on mutable value semantics with `let`,
  `inout`, and `sink` conventions and no first-class references in the surface
  language. It is the closest philosophical relative, with its own compiler.
  Varyk keeps Rust's references underneath and only hides their spelling.
- **Swift** has `inout`, `borrowing`, and `consuming` modifiers, on top of
  reference counting. Varyk has no reference counting.
- **Vale** removes borrow-checker friction with generational references, a
  different runtime model. Varyk changes nothing about the runtime model.
- **Rune** has Rust-like syntax with dynamic typing and reference counting.
  Varyk is statically typed and compiles to native code.
- **Rust with lints.** A lint cannot remove `&` from a call site, because the
  sigils are semantics, not style. Varyk moves that decision into the
  compiler.

## 3. Principles

In priority order. When two conflict, the earlier one wins.

1. **Rust's safety model, unchanged.** No garbage collector. No implicit
   `Clone`. No implicit deep copy. The compiler inserts exactly one kind of
   allocation, described in section 4.3: a string literal placed into an owned
   slot is converted at that line, and `--emit-rust` shows it. Aliasing rules
   are preserved. The generated Rust is checked by rustc, and Varyk never works
   around rustc with unsafe code.
2. **Newcomer first, human or agent.** Every tie-breaker on the surface
   language goes toward the developer who has never written Rust. AI agents
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
   the line where the literal is written. Moves follow Rust's rules, and the diagnostics for moved values
   are a first-class feature.
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

### Non-goals

- Varyk is not a superset of Rust and does not aim to accept arbitrary Rust
  syntax in `.vr` files.
- No garbage collector, ever.
- No separate package registry, package manifest, or build system. Cargo and
  crates.io are the only ones.
- No inline Rust blocks inside `.vr` files. Rust goes in `.rs` files.
- No commitment to a native backend. The `Backend` trait keeps the option
  open; the roadmap does not promise it.
- Not every Rust niche. Embedded and `no_std` targets are out of scope.

### Stability

Before 0.1, everything may change. Diagnostic codes are stable in one sense
from the start: a code, once assigned, is never reused for a different
meaning, though it may be retired. Command-line flags and the generated Rust
layout have no stability guarantee before 0.1.

## 4. Milestone-1 language surface

Milestone 1 is a minimum viable compiler. Its job is to validate the central
idea with six small programs, not to be complete or polished. Only what is
listed here is supported. Anything else is rejected with a
diagnostic that names the construct. Supported features must behave correctly.
Unsupported features must fail loudly. Nothing may silently mis-compile.

### 4.1 Lexical and syntactic elements

- Comments: `//` line comments.
- Keywords: `fn`, `pub`, `let`, `mut`, `struct`, `mod`, `if`, `else`, `while`,
  `break`, `continue`, `return`, `true`, `false`. Every other Rust keyword and
  reserved word, including the 2024-edition ones, is reserved and produces an
  "unsupported" diagnostic when used, so a Varyk identifier is always a valid
  Rust identifier.
- Items: `fn`, `struct`, `mod name;`, each optionally preceded by `pub`.
  Fields of a `pub struct` are public. Field-level `pub` is milestone 2.
- The entry file must define `fn main()` with no parameters and no return
  value. Modules must not define `main`.
- Types: `bool`, `i8`, `i16`, `i32`, `i64`, `u8`, `u16`, `u32`, `u64`,
  `f32`, `f64`, `string`, and user-declared structs.
- Statements: `let x = e;`, `let x: T = e;`, `let mut` forms of both,
  assignment to a variable or a field, `return`, `while`, `break`,
  `continue`, expression statements.
- Expressions: integer, float, bool, and string literals; identifiers; paths
  such as `greet::hello`; unary `-` and `!`; binary `+ - * / %`, comparisons
  `== != < <= > >=`, `&&` and `||`; calls; field access; struct literals;
  blocks whose tail expression is the block's value; `if`/`else`, which is an
  expression as in Rust.
- Arithmetic and ordering operators are defined for numeric types only, with
  both operands of the same type. `==` and `!=` are defined for primitives and
  `string`. `+` on `string` is not in milestone 1.
- String literals support the escapes `\n`, `\t`, `\\`, `\"`, and `\0`. No
  raw strings and no other escapes in milestone 1.
- `println!` is a compiler intrinsic, not a macro system. The format string is
  checked at compile time: only `{}` placeholders and the `{{` and `}}`
  escapes are accepted in milestone 1, and the number of placeholders must
  match the number of arguments. Arguments must be primitives or `string`;
  structs have no display form yet.

Type inference covers `let` bindings without annotations. Integer and float
literals take their type from context, as in Rust, and fall back to `i32` and
`f64`.

### 4.2 Ownership rules

The compiler lowers the program to an intermediate representation, the HIR
(section 6.3), in which every parameter carries a mode: shared borrow,
mutable borrow, or owned.

- A parameter written `name: T` is a **shared borrow**. The body may read it
  and may not mutate it.
- A parameter written `mut name: T` is a **mutable borrow**. The body may
  mutate it, and the caller observes the mutation.
- Call sites never write `&` or `&mut`. The callee's contract decides the
  passing mode.
- An argument passed to a `mut` parameter must be a **mutable place**: a
  `let mut` binding that is not a shared borrowed place (see below), a `mut`
  parameter, a field of a mutable place, or a temporary (a call result, a
  struct literal, or a literal), which nobody else can observe and Rust
  accepts as `&mut`. Passing an immutable `let` binding produces a diagnostic
  whose fix-it adds `mut` to the `let`. Passing a non-`mut` parameter, or a
  binding initialized from one, produces a diagnostic whose fix-it adds `mut`
  to that parameter, with a note that this changes the function's contract
  for its callers.
- A **borrowed place** is a non-Copy parameter, whatever its mode; any
  non-Copy field, whether reached through a parameter or through an owned
  local, because milestone 1 has no partial moves; or a `let` binding
  initialized from either, which inherits the shared or mutable kind of its
  source and is a reference in the generated Rust. Copy-typed parameters,
  fields, and bindings are never borrowed places: reading one yields an owned
  copy. A borrowed place cannot flow into an **owned slot**: a struct field, a
  return value, an imported Rust parameter taken by value, or a `let` binding
  that section 4.3 determines must be owned. The diagnostic names the
  parameter or the struct the field belongs to, and says milestone 2 will
  allow borrowed returns and field moves.
- `let b = a;` moves, as in Rust. Later use of `a` is an error, and the
  diagnostic shows where the move happened. `string` is never `Copy` in Varyk,
  whatever its representation in the generated Rust.
- Copy types (`bool`, integers, floats) are passed by value to read-only
  parameters. For a Copy type this is observably identical to a shared
  borrow, so the HIR records the `Owned` mode and the generated Rust takes a
  plain `i32`. A `mut i32` parameter is a mutable borrow like any other.
- Explicit ownership transfer syntax for Varyk-declared functions does not
  exist in milestone 1. It is an open question.

A note for Rust readers: in Rust, `mut name: T` declares an owned parameter
that the body may rebind. Varyk reuses the keyword for the mutable-borrow
contract. No Varyk-declared parameter takes ownership of a non-Copy value in
milestone 1, so the Rust meaning has nothing to attach to; only imported Rust
signatures produce owned parameters of non-Copy types. This is the one place
a Rust keyword changes meaning, and it is deliberate: mutation of the caller's
value should be visible in the signature.

### 4.3 Strings

`string` is the only string type in the surface language. Writing `String` or
`str` produces a diagnostic pointing at `string`.

Inside the compiler every `string` value has one of two representations,
borrowed (`&str`) or owned (`String`). Sources of string values are:

- a **literal**, which is borrowed and may be converted;
- an **owned value**: a call result, or a `let` binding that is owned;
- a **borrowed place**: a `string` parameter, any `string` field, or a
  `let` binding initialized from either.

**Owned slots** are struct fields, return values, imported Rust parameters of
type `String`, and `let` bindings that need to be owned. A `let` binding needs
to be owned if any value assigned to it is owned, if it is ever passed to a
`mut string` parameter, or if it ever flows into another owned slot. The
representation is computed over the whole function before code generation.

The rules for what may flow into an owned slot:

- a **literal** is converted with `.to_string()` at that expression. This is
  the only allocation Varyk inserts, and it is visible in `--emit-rust`;
- an **owned value** moves, with no allocation, and the source is unusable
  afterwards, whatever its representation;
- a **borrowed place** is an error, with a diagnostic naming the parameter.
  Converting it would be a hidden copy of the string's bytes, which principle
  1 forbids. Milestone 2 adds lifetime inference for borrowed returns and an
  explicit copy spelling.

The table illustrates the rules. The exact reference and dereference
operators in generated code are chosen by type, as section 6.3 describes;
`mut` on a generated `let` mirrors the source.

| Situation                                              | Generated Rust                    | Allocates       |
|--------------------------------------------------------|-----------------------------------|-----------------|
| literal bound by `let`, binding stays borrowed         | `let s = "x";`                    | no              |
| literal bound by `let`, binding needs to be owned      | `let s = "x".to_string();`        | where the literal is written |
| literal into a field, a field assignment, or a return  | `"x".to_string()`                 | where the literal is written |
| owned local into a field, a return, or a `String` parameter | `s`                          | no, moves       |
| parameter or any field into an owned slot              | rejected                          |                 |
| borrowed local or literal to a `string` parameter      | `f(s)`                            | no              |
| owned local to a `string` parameter                    | `f(&s)`                           | no              |
| field to a `string` parameter                          | `f(&user.name)`                   | no              |
| owned local to a `mut string` parameter                | `f(&mut s)`                       | no              |
| field of a mutable place to a `mut string` parameter   | `f(&mut user.name)`               | no              |
| `let` from a field                                     | `let n = &user.name;`             | no              |
| assignment through a `mut string` parameter            | `*name = "x".to_string();`        | where the literal is written |
| `==` between any two strings                           | `a.as_str() == b.as_str()` as needed | no           |

Passing a borrowed string to an imported Rust parameter of type `&String` is
not possible without an allocation the rules above do not cover, so such
functions are not callable in milestone 1 and the diagnostic suggests `&str`.

This is the one place Varyk breaks Rust's naming convention. The rule is:
lowercase names are built-in types (`i32`, `bool`, `string`), CamelCase names
are library types. Go and TypeScript spell strings in lowercase, and those are
two of the languages section 2 targets.

### 4.4 Return values

In milestone 1, return values are always owned. A literal return converts, an
owned value moves, and returning a borrowed place, such as
`fn name(user: User) -> string { user.name }`, is rejected by the rule in
section 4.2. Lifetime inference for borrowed returns is milestone 2 work, and
the HIR is designed to carry it.

### 4.5 Modules and Rust interop

`mod greet;` in the entry file resolves to `greet.vr` or `greet.rs` in the same
directory. This is Rust's module rule, extended to `.vr` files. If both files
exist, that is an error, and `main` is not a valid module name. Items are
referenced through their module path, as in
`greet::hello(name)`, and the generated Rust spells every such path with a
`crate::` prefix so it resolves the same way from any module.

A `.vr` module is compiled like the entry file, except that it must not define
`main`. Its items are visible to the importer only when declared `pub`.
Milestone 1 supports one level: modules declared in the entry file only, no
nested `mod`, and no `mod.vr` directories. Nesting is milestone 2.

Every module, Varyk or Rust, becomes `src/<name>.rs` in the generated crate,
declared by a `mod <name>;` line in the generated `main.rs`, so the generated
tree mirrors the source tree.

A `.rs` module is copied verbatim into the generated crate and compiled as
edition 2024. In milestone 1 it may use only the standard library, because the
generated crate has no dependencies, and it may not declare nested modules,
because only the one file is copied. Its top-level free `pub fn` items are
parsed with the `syn` crate and imported. Methods, `pub(crate)` and
`pub(super)` items, `unsafe fn`, `async fn`, `const fn`, `#[cfg]`-gated items,
and macro-generated items are ignored. Imported signatures map to Varyk modes
with the inverse of the code-generation mapping, extended with `&T` for Copy
`T`:

| Rust parameter type              | Varyk mode                          |
|----------------------------------|-------------------------------------|
| `bool`, integer and float types  | `Owned`, passed by value            |
| `&T`, `&mut T` for Copy `T`      | shared or mutable borrow of `T`     |
| `&str`                           | shared borrow of `string`           |
| `&mut String`                    | mutable borrow of `string`          |
| `String`                         | owned `string`: an owned argument moves, a literal converts, a borrowed place is rejected |
| `&String`                        | not callable in milestone 1; the diagnostic suggests `&str` |

In milestone 1 the only known types are the primitives and `string`, because
Rust structs declared in `.rs` modules are not imported yet. Return types map
`String` to `string`, primitives to themselves, and `()` to no return value.

A signature using generics, explicit lifetimes, trait objects, references in
return position, or any type Varyk does not know is imported as opaque.
Calling it produces an "unsupported Rust signature" diagnostic that shows the
signature. Opaque functions never cause a failure unless they are called.

### 4.6 Not in milestone 1

Enums, `match`, `for`, closures, traits, `impl` blocks, generics, `Option`,
`Result`, `?`, `Vec`, attributes and derives, `use`, Cargo dependencies,
nested modules, field-level `pub`, an explicit string copy, async, threads,
unsafe, raw pointers, FFI, macros beyond the `println!` intrinsic, a package
manifest, a formatter, a language server, and any backend other than
generated Rust.

## 5. Milestone-1 examples

All six must compile and run with the shown output. They are the integration
tests.

`examples/hello.vr` prints `Hello, world!`.

`examples/functions.vr` defines `fn add(a: i32, b: i32) -> i32 { a + b }`,
calls it with 20 and 22, and prints `42`.

`examples/structs.vr` declares `struct User { name: string, age: i32 }`,
defines `fn print_user(user: User)`, constructs one user, and calls
`print_user(user)` twice. Output is `Alice` twice. The second call must
compile: read-only parameters borrow.

`examples/borrowing.vr`, in full:

```varyk
struct User {
    name: string,
}

fn rename(mut user: User) {
    user.name = "Bob";
}

fn print_user(user: User) {
    println!("{}", user.name);
}

fn main() {
    let mut user = User {
        name: "Alice",
    };

    print_user(user);
    rename(user);
    print_user(user);
}
```

Output is `Alice` then `Bob`. The generated Rust, in source order and with
Varyk visibility mapped one to one (`pub` stays `pub`, everything else is
private):

```rust
struct User {
    name: String,
}

fn rename(user: &mut User) {
    user.name = "Bob".to_string();
}

fn print_user(user: &User) {
    println!("{}", user.name);
}

fn main() {
    let mut user = User { name: "Alice".to_string() };
    print_user(&user);
    rename(&mut user);
    print_user(&user);
}
```

`examples/modules/main.vr` declares `mod math;` and calls
`math::square(7)` where `examples/modules/math.vr` defines
`pub fn square(x: i32) -> i32 { x * x }`. Output is `49`.

`examples/interop/main.vr` declares `mod greet;` and calls
`greet::hello(name)` where `examples/interop/greet.rs` defines
`pub fn hello(name: &str) -> String`. Output is `Hello from Rust, Varyk!`.

## 6. Compiler architecture

### 6.1 Planned repository layout

```text
varyk/
├── Cargo.toml                  workspace; edition 2024 in each crate manifest
├── rust-toolchain.toml         pinned stable toolchain for building the compiler
├── LICENSE-MIT, LICENSE-APACHE dual license, Rust ecosystem convention
├── README.md
├── .gitignore
├── crates/
│   ├── varyk-syntax/           spans, tokens, lexer, parser, AST. No semantics.
│   └── varyk/                  library plus the `varyk` binary
│       ├── src/
│       │   ├── diagnostics/    Diagnostic type, codes, renderer adapter, JSON output
│       │   ├── resolve.rs      names, modules, imported Rust signatures
│       │   ├── types.rs        type checking, inference, string representation
│       │   ├── borrow.rs       parameter modes, moves, places, mutability contracts
│       │   ├── hir.rs          lowered program with semantic decisions
│       │   ├── interop.rs      syn-based import of `.rs` signatures
│       │   ├── backend/        Backend trait, RustBackend
│       │   ├── driver.rs       build directory, cargo invocation
│       │   └── main.rs         CLI
│       └── tests/              integration tests that build and run the examples
├── examples/                   the six programs above
└── docs/
    ├── design.md               principles and decisions, derived from this spec
    ├── language.md             one-page language reference, kept in sync with the compiler
    ├── open-questions.md       section 9 of this spec
    ├── roadmap.md              living milestone tracker; section 8 is its starting point
    ├── plans/                  implementation plans, one per milestone
    └── specs/                  design specs, this file first
```

`varyk-syntax` is separate so a formatter and language server can reuse it
without depending on semantic analysis. A `varyk-std` crate for JSON, logging,
HTTP, and the async runtime arrives in milestone 3.

### 6.2 Pipeline

```text
source (.vr)
  → lexer            tokens with spans
  → parser           AST, recursive descent, Pratt for expressions
  → resolver         names, `mod` declarations, imported .rs signatures
  → type checker     types, inference, string representation per binding
  → borrow analysis  parameter modes, places, mutability contracts, whole-variable moves
  → HIR              semantic decisions made explicit
  → Backend trait    RustBackend: Cargo project under target/varyk/<name>/
  → cargo build      rustc does code generation and the full borrow check
  → native executable
```

Every stage from the lexer to the HIR is deterministic and testable without
cargo, given an in-memory set of source files. `varyk check` stops before the
backend.

### 6.3 Key data structures

The AST mirrors the syntax and carries a `Span` on every node. Parameters carry
a `mutable: bool` from the `mut` keyword. The AST does not encode Rust
constructs. The parser recognizes `&`, `&mut`, and lifetime syntax in the
positions listed under "Rust habits" in section 6.6 only to report the
diagnostic, then continues as if the sigil were absent.

The HIR records decisions, not syntax. The critical one:

```rust
pub enum ParamMode {
    SharedBorrow,
    MutableBorrow,
    Owned,
}
```

Every function in the HIR, whether declared in Varyk or imported from a `.rs`
module, has a mode per parameter. Imported functions also keep their original
Rust signature, for use in diagnostics. Call sites in the HIR carry the
resolved callee. Every `string`-typed binding in the HIR carries its
representation, borrowed or owned, and every place expression records whether
it is a borrowed place and whether it is a mutable place; a `mut` parameter is
both.

The backend knows the generated Rust type of every HIR expression: a borrowed
place is `&T` or `&mut T`, an owned value is `T`, a borrowed string is `&str`
and an owned one `String`. Each context (an argument of a given mode, a `let`
initializer, an assignment target, an operand, a field value, a return) has a
required type, and the backend inserts `&`, `&mut`, `*`, or `.as_str()` to
make the expression's type match it, relying on Rust's reborrowing where a
reference already has the right shape. This one rule replaces a case list:
`f(&user.name)` for a field passed by shared borrow, `*x = 1` for assignment
through a `mut i32` parameter, `f(name)` for a `mut string` parameter
forwarded where a `&str` is needed, and both operands of a string comparison
normalized to `&str`. Snapshot tests pin the exact output.

### 6.4 Backend and generated project

`Backend` is a trait with one job: take a `HirProgram` and produce an artifact
or a diagnostic. `RustBackend` writes:

```text
target/varyk/<name>/
├── Cargo.toml       package name, edition 2024, empty [workspace] table, no dependencies
└── src/
    ├── main.rs      generated from the entry .vr file; declares `mod math;` and `mod greet;`
    ├── math.rs      generated from a .vr module
    └── greet.rs     .rs modules copied verbatim
```

`target/varyk/` is relative to the current working directory. `<name>` is the
entry file's stem followed by a short hash of its absolute path, so two files
called `main.vr` in different directories never share a build directory. The
package name, and therefore the binary name, is `<name>` sanitized to a valid
crate identifier. The empty `[workspace]` table keeps cargo from treating the
shadow crate as a member of any Rust workspace that contains the current
directory, which is the normal situation when adopting Varyk inside an
existing Rust project.

Cargo is invoked with a shared target directory, `target/varyk/cache/`, so
dependencies compile once across programs. The executable is at
`target/varyk/cache/<profile>/<package name>`. `varyk build` builds the debug
profile by default and accepts `--release`; integer overflow behaves as in
Rust for the chosen profile.

The generated crate begins with `#![allow(dead_code, unused_variables,
unused_mut)]`, because generated code triggers these warnings routinely and
they mean nothing to a Varyk writer. In milestone 1 this also covers copied
`.rs` modules; per-file handling is milestone 2.

The generated Rust is emitted by the backend in a deterministic, readable
style, and builds do not depend on rustfmt being installed. `--emit-rust`
prints every `.rs` file in the generated crate to stdout, each preceded by a
header line naming the file, passed through rustfmt when rustfmt is on the
path, and then continues the build.

### 6.5 Two-layer safety

Varyk's own passes check what they understand: parameter modes, mutability
contracts, mutable and borrowed places, whole-variable moves, unsupported
constructs, string spellings, and the string rules of section 4.3. The full
borrow check belongs to rustc.

In milestone 1, when rustc rejects generated code, cargo's output is shown as
is, after one line from Varyk saying that the error comes from the generated
Rust and where that Rust can be inspected with `--emit-rust`. None of the six
examples reach this path, and Varyk's own checks catch the common mistakes
before cargo runs.

Milestone 2 adds a source map from generated lines to Varyk spans and reports
Rust-layer errors as Varyk diagnostics at the Varyk location, keeping errors
in copied `.rs` modules in the user's own terms. Improving the wording of
those errors, case by case, is ongoing work after that.

### 6.6 Diagnostics

Diagnostics are a first-class deliverable. Every error carries a file, a span,
a line, a column, a message, and optionally labels, notes, and a fix-it. Each
diagnostic kind has a code of the form `V` followed by four digits, such as
`V0012`, with the stability rule from section 3. The list of codes lives at
the end of `docs/language.md`.

Rendering uses the `annotate-snippets` crate, maintained by the Rust project
and used by cargo, behind Varyk's own `Diagnostic` type. The renderer can be
swapped without touching the passes.

The same `Diagnostic` values are emitted as JSON, one object per line, when
`--message-format=json` is given. Each object carries the code, severity,
message, file, span with line and column, labels, notes, and fix-its. This is
the interface for editors and for AI agents, and it is part of milestone 1.

**Output streams.** Human-readable diagnostics go to stderr; JSON diagnostics
go to stdout, one object per line. `--emit-rust` output and the executable
path go to stdout. `run` prints no executable path and hands both streams to
the program once the build is done.

**Rust habits.** A writer who knows Rust, human or model, will reach for Rust
spellings that Varyk infers. Each of these gets a dedicated diagnostic that
explains the Varyk rule and offers a fix-it:

- `&x` or `&mut x` at a call site: "Varyk infers references; remove the `&`";
- `&T` or `&mut T` in a parameter type: point at `name: T` or `mut name: T`;
- `String` or `str`: point at `string`;
- lifetime syntax such as `<'a>` or `&'a T`: "lifetimes are inferred".

Required in milestone 1:

- unsupported construct, naming the construct;
- unsupported Rust signature, showing the signature;
- mutation through a non-`mut` parameter, with a fix-it that adds `mut` to the
  parameter;
- assignment to an immutable `let` binding, with a fix-it that adds `mut`;
- passing an immutable binding or a non-`mut` parameter to a `mut` parameter,
  with the fix-its from section 4.2;
- a borrowed place flowing into an owned slot, naming the parameter or
  struct;
- use after move, with a label at the move;
- the Rust-habit diagnostics listed above, each with a fix-it;
- type mismatch, unknown name, unknown field, wrong argument count;
- `println!` placeholder count mismatch, and `{}` applied to a struct.

### 6.7 Command-line interface

```text
varyk check <file.vr>                          parse and analyze; never runs cargo
varyk build <file.vr> [--release] [--emit-rust] generate and build; prints the executable path
varyk run   <file.vr> [--release] [-- args...]  build, then execute, forwarding the exit code
```

Every command accepts `--message-format=json` to emit structured diagnostics
instead of the human renderer.

`check` is the fast loop and must stay fast. The build directory lives under
`target/varyk/` and generated Rust never lands in the source tree.

### 6.8 Packages and Cargo

A Varyk package is a Cargo package. From milestone 2 on, its manifest is
`Cargo.toml`, its dependencies are declared the Cargo way, `cargo add` works
unchanged, and any Varyk-specific settings go under Cargo's
`[package.metadata.varyk]` table. There is no separate Varyk manifest.

Milestone 1 supports only single-entry mode: the entry `.vr` file plus the
`.vr` and `.rs` modules it declares. This is the degenerate package with no
manifest.

`varyk` drives `cargo`: it transpiles into a shadow crate under `target/varyk/`
and invokes cargo there, keeping diagnostics under Varyk's control. The
alternative, a `build.rs` that lets plain `cargo build` drive Varyk so existing
Cargo tooling needs no Varyk-aware steps, arrives in milestone 2 as a
`varyk init` template; it requires the `varyk` binary on the path of whoever
builds the package.

Publishing follows the TypeScript model: a Varyk library is published to
crates.io with its generated `.rs` files included, so consumers need only
cargo, exactly as TypeScript libraries publish compiled JavaScript to npm.

## 7. Testing and definition of done

- Unit tests per pass: tokenization, spans, function and parameter parsing,
  `mut` parameters, struct parsing, expression precedence, type inference,
  string representation, mode assignment, move detection, place checking,
  signature import mapping.
- Snapshot tests with `insta` for generated Rust and for rendered diagnostics,
  taken on the backend's own output so they do not depend on rustfmt.
- Integration tests under `crates/varyk/tests/` that build and run all six
  examples and compare stdout.
- CI runs `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D
  warnings`, and `cargo test --workspace`.

Milestone 1 is done when: the workspace compiles; every path the six examples
exercise is fully implemented; unsupported syntax produces a diagnostic rather
than a panic; the tests above pass; and the README, `docs/design.md`,
`docs/language.md`, `docs/open-questions.md`, and both license files exist.

**Language reference.** `docs/language.md` describes the whole supported
surface on one page: syntax, types, the ownership rules, the string rules, and
the diagnostics a writer is likely to meet, ending with the list of codes. It
is deliberately short enough to paste into a prompt and is updated in the same
change as any surface addition. A test that checks it against the parser is
milestone 2.

## 8. Roadmap

`docs/roadmap.md` is the living version of this section and uses the same
milestone titles.

- **Milestone 1: compiler skeleton and the borrow-by-default proof.** This
  spec: six examples, modules, interop proof.
- **Milestone 2: packages, nested modules, enums, and control flow.**
  `Cargo.toml` as package manifest, Cargo dependencies, `use`, nested modules
  and `mod.vr` directories, field-level `pub`, enums, `match`, `for`, `impl`
  blocks, using generic standard types (`Option`, `Result`, `Vec`) without
  declaring generics, `?`, closures, lifetime inference for borrowed returns,
  an explicit string copy spelling, Rust struct import, Rust-layer errors
  mapped to Varyk source through a source map, per-file handling of rustc
  warnings, a test that `docs/language.md` covers the parser, `varyk init`
  with a `build.rs` so plain `cargo build` works, publishing to crates.io with
  generated `.rs` included, and `varyk fmt`, a deterministic formatter on top
  of `varyk-syntax`.
- **Milestone 3: batteries for services.** `varyk-std`: JSON via serde,
  logging via tracing, an HTTP server on a proven Rust crate chosen at that
  time, and `async`/`await` on a built-in tokio runtime. Milestone 3 is gated
  on three open questions in section 9: how Varyk structs derive serde's
  traits, the async runtime shape, and ownership-transfer syntax.
- **Milestone 4: tooling and beyond.** Language server on top of
  `varyk-syntax`, and a decision on a native backend.
- **Unscheduled, pending open questions.** Declaring generics, traits, and
  attributes in Varyk code.

**Concurrency direction.** Varyk keeps Rust's async model and JavaScript's
surface: `async fn` and `.await`, a built-in runtime, and `spawn` as a
built-in. `Send`, `Sync`, and `Pin` are kept out of the surface syntax; they
are still enforced by rustc, and their failures must be mapped to Varyk
diagnostics. Whether the runtime is multi-threaded, which makes `spawn`
require owned captures and therefore depends on the ownership-transfer
syntax, or current-thread, which avoids `Send` entirely, is an open question
to be decided before milestone 3. Varyk does not adopt Go-style colorless
concurrency: the Rust crates it builds on are already async.

## 9. Open questions

Recorded, deliberately unanswered.

- Should local variables require `let mut`? Milestone 1 keeps it, following
  Rust.
- Should mutation be inferred from the body instead of declared on parameters?
- What syntax should explicit ownership transfer use for Varyk-declared
  functions? `move` is a candidate. The async runtime decision depends on it.
- How should owned versus borrowed return values be expressed and inferred?
- How should an explicit string copy be spelled, once method calls exist?
- How should lifetime inference work across function boundaries?
- How should Rust traits, generics, and attributes map into Varyk while
  keeping Go-like simplicity?
- Should serde derivation be automatic for every struct, or declared with
  attribute syntax? Milestone 3 is gated on this.
- Multi-threaded or current-thread async runtime, and how do `Send` and `Sync`
  failures surface to a writer who never sees those bounds?
- Which Rust spellings, such as `&x` at a call site, should be accepted with a
  warning as a transition aid rather than rejected? Milestone 1 rejects them
  with a fix-it.
- What error-handling ergonomics beyond `Result` and `?` are worth adding?
- Should Rust enums and generic types from `.rs` modules be imported
  automatically, as structs will be in milestone 2?

## 10. Decisions log

| Decision | Choice | Why |
|---|---|---|
| Name | Varyk | Lithuanian for "go!"; unclaimed on crates.io, npm, and PyPI at the time of writing, with no repository of that name on GitHub; does not contain "Rust" |
| Extension | `.vr` | short, "var" mnemonic, unclaimed |
| String type | `string`, lowercase, one type | newcomer first; owned versus borrowed is a compiler decision |
| String allocation | only a literal into an owned slot converts, at that line | one predictable allocation, visible in `--emit-rust`; no hidden copies |
| Borrowed places | cannot flow into owned slots in milestone 1 | the alternative is a hidden clone; lifetime inference comes in milestone 2 |
| Assignment | Rust move semantics; `string` never `Copy` | preserves Rust's model; diagnostics carry the burden |
| Parameter passing | borrow by default, `mut` for mutable borrow | the central idea (section 1) |
| Copy types | `Owned` mode, passed by value | observably identical to a borrow, no indirection |
| Modules | `mod name;` resolves to `.vr` or `.rs`, `pub` for visibility, one file per module in the generated crate | Rust's rule; Varyk and Rust modules are symmetric |
| Rust interop | `.rs` files as modules, plus Cargo crates; no inline Rust blocks | TypeScript/JavaScript model; one grammar per file |
| Manifest | `Cargo.toml` | reuse Cargo entirely; no Varyk manifest |
| Build orchestration | `varyk` drives `cargo` first, `build.rs` later | diagnostics stay under Varyk's control |
| Generated code formatting | backend emits readable code; rustfmt only for display | builds must not depend on rustfmt |
| Rust-layer errors | passed through as is in milestone 1, mapped to Varyk source in milestone 2 | the six examples never hit them; mapping is real work and not needed to prove the idea |
| rustc warnings | silenced crate-wide in milestone 1, per file in milestone 2 | generated code warns routinely; per-file filtering is not MVP |
| Diagnostics renderer | `annotate-snippets` | maintained by the Rust project; no renderer to own |
| Concurrency | Rust async, JavaScript surface, built-in runtime | ecosystem is already async |
| License | MIT or Apache-2.0 | Rust ecosystem convention |
| AI agents | first-class writers, humans win on conflicts | Rust knowledge transfers; the removed syntax is where models fail; structured diagnostics close the loop |
