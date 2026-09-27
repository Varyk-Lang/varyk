# Varyk milestone 3: packages and interop

Date: 2026-09-26.

**Status.** Design, not yet implemented. Extends the milestone-1 spec,
`2026-09-23-varyk-design.md` (cited as "M1 §n"), and the milestone-2 spec,
`2026-09-25-milestone-2-design.md` (cited as "M2 §n"). Their principles,
ownership rules, and compiler architecture stay in force. Varyk is
experimental and pre-0.1: anything here may change.

## 1. Summary and scope

Milestones 1 and 2 made a single `.vr` file, with the modules it declares,
into a running program. Milestone 3 makes Varyk code a Cargo package: a
`Cargo.toml`, dependencies from crates.io, modules at any depth, `use`,
private fields, Rust structs and enums imported from `.rs` modules, plain
`cargo build` through a `build.rs`, and publishing to crates.io as a crate
whose consumers need only cargo. Rust-layer errors are reported at the Varyk
line that produced them.

The one decision that shapes everything else: **Varyk code never names a
crate.** A dependency is used from a `.rs` file in the same package, a
*facade*, which exposes plain functions, structs, and enums that Varyk
imports the way it has imported `.rs` functions since milestone 1. `use`
exists, but only for paths inside the package. This follows M1 principle 6,
escape hatches live in Rust files, and it holds regardless of whether Varyk
ever gets generics: most crate APIs are generic, and a facade is where the
concrete API gets written, as typed wrappers are for untyped JavaScript
libraries. Direct crate `use` from Varyk is an open question, revisited when
traits exist (section 12).

Milestone 3 is an MVP like the two before it. The test for every item was
"does one of the three example packages in section 7 need it"; what failed
is listed in section 6 and scheduled in section 11. Only what this document
lists is supported; anything else is rejected with a diagnostic that names
the construct.

The site at varyk.com stays a milestone-3 roadmap item but is a separate
sub-project in its own repository and is not covered here.

## 2. Packages and the command line

### 2.1 A Varyk package

A Varyk package is a Cargo package whose crate root is `src/main.vr` (a
binary) or `src/lib.vr` (a library), never both. Its `.vr` and `.rs`
modules live under `src/` as section 3 describes. `Cargo.toml` is the only
manifest, as M1 §6.8 decided.

Single-file mode, `varyk run hello.vr`, is unchanged. The two modes are told
apart by one rule: **for a file argument, look for `Cargo.toml` upward from
the file; if the file is that package's `src/main.vr` or `src/lib.vr`, the
package mode applies; anything else is single-file mode.** So
`varyk run src/main.vr` inside a package gets the package's dependencies,
and `varyk run examples/hello.vr` inside a Rust workspace stays single-file
because it is not that workspace's crate root.

`varyk check`, `varyk build`, and `varyk run` with no file argument look
for `Cargo.toml` upward from the current directory and use its crate root.
No `Cargo.toml`, or one with neither root file, is the one-line error "no
Varyk package here; give a file or run inside a package" (for the second,
it names the `Cargo.toml` found and says the package has no `src/main.vr`
or `src/lib.vr`).

A library must not define `main` (V0106) and cannot be `run`; a binary must.
Module names `main` and `lib` stay reserved (V0104).

### 2.2 The manifest

`varyk check` reads `Cargo.toml` with a TOML parser. It never runs cargo,
keeping M1's rule that `check` is cargo-free. The manifest is registered as
a source file so its diagnostics render like any other, with spans where the
parser gives them.

- `[package] edition` must be `"2024"` (V0400). The hidden crate compiles as
  2024; under plain `cargo build` the generated code compiles under the
  user's edition, and the two must agree.
- `[dependencies]` is copied into the hidden crate as written, except that
  relative `path` values are made absolute. Per-dependency `features`
  lists pass through with it.
- A dependency or package field with `workspace = true` is V0401, "not
  supported yet". A Varyk package *inside* a Rust workspace is fine, as in
  milestone 1; only inheritance from the workspace is out.
- `[target.'cfg(..)'.dependencies]` is V0401 as well.
- `[lib]`, `[[bin]]`, and the other target tables, and the `auto*` and
  `default-run` keys, are V0402 and V0401 (amended: a target table with a
  `path` used to be the one error, then a growing set of rules told a
  harmless root table from one that broke the build; a package for a small
  service needs none of them, so none is read: the roots are fixed at
  `src/main.vr` and `src/lib.vr`, found by cargo's defaults). The hidden crate and the crate `publish` assembles are built with one
  manifest, the package's own with `build = false`, the layout keys
  (`include`, `exclude`, `workspace`) dropped, paths made absolute, and an empty `[workspace]`
  table (amended: the hidden manifest used to be synthesized from a few
  fields, and every other field, `[features]`, `[profile.*]`,
  `rust-version`, was a way for `build` and `publish` to disagree). For a
  workspace member, the root's `Cargo.lock`, `[profile.*]`, and `resolver`
  are taken, as cargo takes them (amended: a member has no lock of its
  own, and cargo ignores a member's profiles).
  `check` accepts a fixed list of top-level tables and `[package]` keys and
  refuses any other key as V0401 (amended: unsupported keys used to pass
  `check` and surface in `build` or `publish` one at a time; an MVP for
  small services can say "not yet").
  `[lints]` and a custom `build` script are V0401 (amended: lints apply to
  the generated Rust too, which cannot opt out of a `deny`; no crate Varyk
  builds runs a build script).
  `[patch]`, and cargo's older `[replace]`, in the package or in the root manifest of the nearest
  enclosing workspace, is V0401 (amended: the hidden crate built the
  unpatched source; carrying the patch correctly means reproducing cargo's
  workspace-membership rules, deferred to the follow-ups). `links` is
  V0401 (amended: it needs a build script the assembled crate does not
  carry) and a proc-macro or bin `[lib]`, by `proc-macro = true` or `crate-type`, is
  V0402 (amended: a proc-macro crate may export only macros).
  `[dev-dependencies]`, `[build-dependencies]`, and `[profile.*]` are
  carried through as written (amended: they used to be ignored by the
  hidden crate; `check` reads none of them), except that `workspace = true`
  anywhere is V0401 (amended: the crates Varyk builds stand outside the
  workspace, so an inherited entry would break their manifest).
- `[package.metadata.varyk]` is the reserved home for Varyk settings and
  defines no keys yet.
- `Cargo.lock`, when it sits beside `Cargo.toml`, is copied into the hidden
  crate before cargo runs, so `varyk build` and `cargo build` resolve the
  same versions. It is never copied back. A lock at a workspace root is not
  searched for.

### 2.3 One generated tree

One routine produces the complete `src/` tree of a Rust crate from a
package or a single file:

- every `.vr` file becomes a `.rs` file at the mirrored path (`src/main.vr`
  → `src/main.rs`, `src/shop/cart.vr` → `src/shop/cart.rs`, `src/shop/mod.vr`
  → `src/shop/mod.rs`);
- every `.rs` module is copied verbatim to its mirrored path;
- every generated item (`fn`, `struct`, `enum`, `impl`, `use`) carries
  `#[allow(warnings, arithmetic_overflow, unconditional_panic)]`; no `mod`
  line does, since a lint attribute there would also cover any `.rs` module
  nested below it (a `.vr` module whose name is not snake case gets only
  `#[allow(non_snake_case)]` on its `mod` line).

The item-level attributes replace M1's crate-wide `#![allow(..)]`. Lint
attributes on items cover the item's body, the `warnings` group covers every
warn-level lint so generated code is silent under any driver, and the two
deny-by-default lints are named because a group cannot include them; the
program still panics at run time on overflow or division by zero as M1 §6.4
specifies. Copied `.rs` files carry no attribute, so the user's Rust warns as
Rust normally does (section 5). Because the root file has no inner
attribute, the same tree works as a crate root and inside `include!`.

The output directory is cleared before writing, so a deleted module leaves
no stale file. The tree is written to three places and is byte-identical in
all of them.

### 2.4 `varyk build` and `varyk run`

Unchanged in shape: the tree goes into a hidden crate and cargo builds it
there. In package mode the hidden crate is `<package root>/target/varyk/
<package name>/`, its `Cargo.toml` is generated from the user's (name,
version, edition 2024, the copied `[dependencies]` and `[features]`, an
empty `[workspace]` table), and `Cargo.lock` is copied in. The binary
therefore has the package's name. The shared cargo target directory stays
`target/varyk/cache/`, now under the package root. The user's `build.rs` and
stub files (section 2.5) play no part: the hidden crate has its own root.

Single-file mode keeps M1's `target/varyk/<stem>-<hash>/` under the current
directory.

Dependencies compile once for `varyk build` and once for `cargo build`,
because the two use different target directories. Sharing cargo's own
`target/` would make two crate roots with one package name collide on the
binary path; the cost is recorded, not solved.

### 2.5 `varyk init`, `varyk emit`, and plain `cargo build`

`varyk init [dir] [--lib]` writes a package that both `varyk` and plain
`cargo` can build:

```text
Cargo.toml        name from the directory, version 0.1.0, edition 2024, empty [dependencies]
.gitignore        /target
build.rs          runs `varyk emit --out-dir $OUT_DIR/varyk`; see below
src/main.rs       ::std::include!(::std::concat!(::std::env!("OUT_DIR"), "/varyk/src/main.rs"));
src/main.vr       hello world
```

The stub's path has `src/` in it because `emit` writes the tree of section
2.3 under `src/` in the directory it is given. With `--lib`, `src/lib.rs`
and `src/lib.vr` replace the last two. The
`build.rs` runs the binary named by the `VARYK` environment variable, or
`varyk` on the path, prints `rerun-if-changed=src` and
`rerun-if-env-changed=VARYK`, and fails with one clear line when the binary
is not found. `init` refuses to run if any file it would write exists, and
lists them, so adopting Varyk inside an existing Rust project means copying
the two or three files by hand; there is no merge.

`varyk emit --out-dir DIR` writes the tree of section 2.3 to `DIR` after a
full check. It is the hook `build.rs` calls and never invokes cargo, so
there is no recursion. `varyk build --emit-rust` stays as the stdout view.

Under plain `cargo build`, a Varyk error fails the build script, and cargo
prints the script's stderr, which is Varyk's normal rendering. Rustc errors
in that mode show `OUT_DIR` paths and are not mapped; the build-script
failure message suggests `varyk build` for Varyk-side reporting (section 5).
The stub is one line and carries no attribute, which is why section 2.3 puts
the lint allows on items. rust-analyzer handles `include!` of `OUT_DIR` as it
does for other code generators.

### 2.6 `varyk publish`

`varyk publish [-- cargo args]` runs the full check, assembles a plain Rust
crate at `<package root>/target/varyk/package/<name>/`, and runs
`cargo publish` there with the forwarded arguments (`--dry-run`,
`--allow-dirty`, `--token` and the rest are cargo's, and Varyk does not
interpret them). The assembled crate holds:

- the user's `Cargo.toml` with `build = false` and the same `[dependencies]`
  rewrite as section 2.2;
- the tree of section 2.3, so `src/main.rs` or `src/lib.rs` is the generated
  root, not the stub;
- the `.vr` sources at their places in `src/`, for readers;
- the files the manifest names (`readme`, `license-file`) and any `README*`
  and `LICENSE*` at the package root.

The published crate has no `build.rs` and needs no `varyk`: a consumer adds
it like any crate, and `cargo install` of a Varyk binary works the same way.
This is the TypeScript model of M1 §6.8: compiled output ships, sources
ride along. A Varyk package consuming a published Varyk library reaches it
through a facade like any crate; direct import is an open question
(section 12).

## 3. Modules, `use`, and visibility

### 3.1 Nested modules

Any `.vr` file may declare `mod child;`, not only the crate root. Resolution
is Rust's: from `src/shop.vr`, `mod cart;` finds `src/shop/cart.vr` or
`src/shop/cart.rs`; from `src/shop/mod.vr` it finds the same. A directory
module is `shop/mod.vr` when there is no `shop.vr`. A module present as both
`.vr` and `.rs`, or as both `shop.vr` and `shop/mod.vr`, is V0104, as the
twin rule of M1 §4.5. Generated files mirror the tree (section 2.3), so the
emitted `mod` lines need no `#[path]`.

Paths grow with depth: `shop::cart::Cart`, `crate::shop::cart::Cart`, and
from inside `shop`, `self::cart::Cart` and from `cart`, `super::Order`.
`super` in the crate root is V0111.

A `.rs` module still may not declare modules (V0104, unchanged from M1): a
facade is one file. Nested `.rs` trees are scheduled in section 11.

### 3.2 Visibility

One rule, Rust's: **a module, item, or field without `pub` is visible in the
module that declares it and in that module's descendants.** A path is
usable only if every module on it is visible from the user by the same rule.
`pub mod child;` is new and means what it means in Rust. In a library, the
`pub` items reachable through `pub mod` chains from `lib.vr` are the crate's
public API; a `pub` item inside a non-`pub` module never leaves the crate.

Milestones 1 and 2 declared every module in the crate root, where this rule
and the old one agree, so no existing program changes meaning except for
fields (section 3.4). Rust's `pub(crate)` and `pub(super)` are not added.
Violations are V0105, widened: a private module ("write `pub mod cart;`"),
a private item, a private field.

### 3.3 `use`

`use path;` and `use path as name;` are items, allowed in any `.vr` file in
any order among the other items. The path starts with `crate::`, `self::`,
`super::`, or the name of a module declared in the same file (Rust's uniform
paths), and ends at a module, struct, enum, or function, Varyk-declared or
imported from a `.rs` module. It introduces a local alias in the resolver.
In the generated Rust, `use` lines are emitted as canonical `crate::` paths;
every use site is written with its full path, so the aliases never affect the
generated Rust.

Not allowed: braces, globs, a path ending at an enum variant (patterns stay
one level, `Shape::Circle`), and any leading name that is not a module in
this file or one of the three keywords. A leading name that is a crate,
`std` included, is V0110, whose note explains the facade pattern: "Varyk code
does not use crates directly; call it from a `.rs` module in this package".
A `use` name that clashes with an item declared in the same file is V0103.

### 3.4 Field-level `pub`

```text
pub struct User {
    pub name: string,
    age: i32,
}
```

Fields follow the visibility rule of section 3.2: private unless `pub`,
visible in the declaring module and its descendants, which includes the
struct's own methods. Reading, assigning, or naming a private field from
elsewhere is V0105 with a note offering both ways out, `pub` on the field or
a `pub fn new`, because making every field public is often the wrong fix. A
struct literal from another module needs every field visible; the same code,
with literal-specific wording.

This breaks the milestone-2 rule "all fields of a `pub struct` are public"
for code that reads fields across modules; pre-0.1 allows it, and the
milestone-2 examples are updated. Enum variants stay public with their enum.

## 4. Rust import

M1 §4.5 imported top-level `pub fn` signatures from `.rs` modules with
`syn`. Milestone 3 extends the same importer to structs, inherent methods,
and enums, and lets imported types appear in signatures.

### 4.1 Structs

A non-generic `pub struct` with named fields in a `.rs` module becomes a
Varyk struct type `m::S`, entered in the same tables as a Varyk-declared
struct and flagged as imported, so the checker and borrow analysis treat it
identically and the backend emits no definition for it.

- A `pub` field whose type maps (section 4.4) is a field like any other:
  readable, assignable, and subject to the alias rules of M2 §3.1.
- A private or `pub(crate)` field is invisible, by the rule of section 3.2.
- A `pub` field whose type does not map is present but unusable: touching it
  is V0108, naming the Rust type.
- A struct literal from Varyk needs every field visible and mappable;
  otherwise V0105 with the constructor note.

Structs with generic or lifetime parameters, tuple structs, and unit structs
are not imported; naming one is V0101 with a note saying why.

This is the facade pattern: `pub struct Matcher { re: Regex }` with
`pub fn new(pattern: &str) -> Matcher` and
`pub fn is_match(&self, s: &str) -> bool` imports fully, because the
private field is invisible and both signatures map.

### 4.2 Methods

`pub fn` items in inherent `impl S` blocks in the same `.rs` file as `S`
import as methods and associated functions, with `Self` resolved to `S`.
Multiple `impl` blocks merge. Receivers `&self` and `&mut self` map to
Varyk's `self` and `mut self`; parameters use the M1 mode table extended by
section 4.4; return types must map. Anything else stays opaque and is V0108
when called, showing the signature: owned `self`, a borrowed return such as
`-> &str` (borrowed returns are milestone 4; facade authors return
`String`), generic methods, trait implementations, and the M1 exclusions
(`unsafe`, `async`, `const`, `#[cfg]`, macro-generated).

### 4.3 Enums

A non-generic `pub enum` whose variants are unit or tuple variants with
mappable payloads imports like a Varyk enum: nameable, constructible,
matchable with M2's one-level patterns and exhaustiveness. An enum with a
named-field variant or an unmappable payload is imported as an opaque type:
holdable, passable, returnable, but not constructible or matchable, and
naming a variant is V0100 with a note saying why.

### 4.4 Types in signatures and fields

The M1 table gains rows, and every row applies to fields as well as
parameters:

| Rust type | Varyk |
|---|---|
| `S`, `&S`, `&mut S` for an imported struct or enum `S` | owned, shared borrow, mutable borrow |
| `Vec<T>`, `Option<T>`, `Result<T, E>` of mappable types | the M2 types, by value or borrowed like `String` |
| `usize` | `usize` (from M2 §2.11) |

Type paths inside a `.rs` file resolve two ways: a bare name to an item in
the same file, and a full `crate::…` path to an item in another `.rs`
module. A type reached through a `use` line in the `.rs` file is not
followed; the field or signature is unmappable, with the note "write the
full path". A `crate::` path to a Varyk-declared type is not followed either
in this milestone (section 11).

### 4.5 Derives and attributes

Ignored. An imported struct that derives `Copy` moves in Varyk terms,
over-strict and sound. `clone`, `==`, and `{:?}` stay as M2 left them.

## 5. Rust-layer errors and warnings

The two-layer model of M1 §6.5 stands; this section is its promised net.

The backend records a line-level source map per generated file: each emitted
line carries the span of the Varyk node it came from. The driver runs cargo
with `--message-format=json` and reads every compiler message.

- A message whose primary span is in a generated file becomes a Varyk
  diagnostic, V0900, at the mapped Varyk span, carrying rustc's message and
  its code, a note that Varyk's own checks should have caught this and it
  should be reported, and the `--emit-rust` pointer. Warnings in generated
  files are dropped; section 2.3 should already have silenced them.
- A message whose primary span is in a copied `.rs` file is the user's own
  Rust. The copy is verbatim, so rustc's rendered text is shown as is, with
  the path rewritten to the user's file. Errors and warnings both pass
  through, in package and single-file mode.
- Anything unmapped falls back to cargo's text with M1's one-line note.

No program in the examples or tests reaches V0900 except the test that feeds
the driver a fabricated cargo message; the soundness sweep of M2 §7 is what
keeps it that way.

## 6. Not in milestone 3

Cut because no example needs them, scheduled in section 11: nested modules
in `.rs` files; tuple and unit struct import; Rust structs deriving traits
Varyk could use (`Clone`, `PartialEq`, `Debug`); `.rs` signatures naming
Varyk-declared types; direct import of a published Varyk library from Varyk;
`pub(crate)` and `pub(super)`; `use` with braces or globs; `use` of a
variant; re-exports (`pub use`) in either direction; Cargo workspace
inheritance; custom target paths; reading `[features]`; sharing cargo's
`target/` between `varyk build` and `cargo build`; `varyk init` into an
existing project; `varyk` subcommands mirroring `cargo test`, `cargo doc`, or
`cargo add`, which work unchanged through cargo once `init` has run.

Not in this milestone by design: everything in M2 §2.12's "by design" list,
and the milestone-4 and later items of the roadmap.

## 7. Milestone-3 examples

Three packages under `examples/packages/`, excluded from the compiler's
workspace in the root `Cargo.toml` so cargo treats them as their own
packages; each ignores its own `target/`. The single-file examples of
milestones 1 and 2 stay, with `structs.vr` and `todo/` updated for field
`pub`.

### 7.1 `greeting`, a binary built two ways

The output of `varyk init greeting`, committed as is (a test asserts the
template still produces it), plus:

```text
src/main.vr        mod text; use crate::text::case::title; a Greeter struct with
                   one pub field and one private field and a pub fn new
src/text/mod.vr    pub mod case; pub fn greeting(name: string) -> string
src/text/case.vr   pub fn title(word: string) -> string
```

CI runs `varyk run` and, with `VARYK` pointing at the freshly built compiler,
`cargo run`, and asserts the same output from both. Exercises: package mode,
nested and directory modules, `pub mod`, `use`, `super`-free deep paths,
field privacy, `init`, `build.rs`, `emit`.

### 7.2 `matcher`, a facade over a crate

`Cargo.toml` depends on one small registry crate, chosen in the plan under
the constraint that it compiles in a few seconds cold. `src/text.rs` wraps
it:

```text
pub struct Matcher { re: <crate type> }
impl Matcher {
    pub fn new(pattern: &str) -> Matcher
    pub fn is_match(&self, s: &str) -> bool
    pub fn count(&self, s: &str) -> usize
}
pub enum Kind { Word, Number(i32) }
pub fn classify(s: &str) -> Kind
```

`src/main.vr` builds a `Matcher`, calls its methods on a `Vec` of strings,
and matches on `Kind`. Exercises: dependencies, `Cargo.lock`, struct and
method import with a private field, enum import, imported types in
signatures, item-level lint allows leaving `text.rs` warnings visible.

### 7.3 `units`, a published library and a cargo-only consumer

`varyk init units --lib`, with `src/lib.vr` declaring `pub mod length;` and
`length.vr` a `pub struct Meters { pub value: i32 }` and
`pub fn add(a: Meters, b: Meters) -> Meters`. CI runs the publish assembly
and `cargo package` on the result, then builds `consumer/`, a plain Rust
binary with a `path` dependency on the assembled directory, which calls
`units::length::add` and prints the result. Exercises: libraries, `pub`
API surface, publish assembly, the no-`varyk` consumer.

## 8. Compiler changes

The pipeline of M1 §6.2 is unchanged. Three files are split first, as the
opening task of their areas, with no behavior change: `driver.rs` becomes
`driver/{mod,generate,cargo,init,publish}.rs`; `resolve.rs` (2,100 lines,
flagged in the milestone-2 ledger) gets `resolve/{modules,visibility}.rs`
split out; `interop.rs` becomes `interop/{mod,signatures,structs,enums}.rs`.

**New `package.rs`.** Finds the manifest upward, parses it with the `toml`
crate at a version within MSRV 1.85, and returns a `Package`: root file and
kind, dependency table, lock path, edition, plus the diagnostics of
section 2.2. `check` uses it; nothing else parses TOML.

**`varyk-syntax`.** `pub mod`, `mod` in any file, `use path [as name];` as
an item, `pub` on fields. AST: `Item::Use`, a `pub` flag on `Field`.

**Resolver.** A module tree of arbitrary depth with parent links; `use`
aliases resolved before user names in their file; visibility as one function,
"is the declaring module an ancestor-or-self of the user, or is the thing
`pub` and every module on the path visible"; imported structs and enums in
the same tables as declared ones with an `imported` flag; V0105 widened,
V0110, V0111 new.

**Types and checker.** Imported structs and enums are ordinary `Ty::Struct`
and `Ty::Enum`; fields carry visibility and an `unusable` marker for
unmappable Rust types; `main` required only for binaries.

**Interop.** Structs, same-file inherent impls with `Self`, enums, the
extended type table, two-way path resolution inside `.rs` files, per-field
and per-variant opaque marking.

**HIR.** `HirStruct` and `HirEnum` gain `imported: bool`; fields gain `pub`;
`HirModule` gains `parent` and `pub`; a `Use` item.

**Borrow analysis.** Unchanged; imported types go through the existing
struct and enum paths.

**Backend.** Emits `pub mod`, `use`, `pub` fields, `#[allow(warnings,
arithmetic_overflow, unconditional_panic)]` on every generated item, no
definitions for imported types, and a line-level source map per file.

**Driver.** The generation routine of section 2.3 and its three callers;
the hidden crate for a package (section 2.4); `emit`, `init`, and publish
assembly; cargo JSON reading, mapping, V0900, and `.rs` passthrough
(section 5).

**CLI.** No-argument forms, `emit`, `init`, `publish`; "run on a library"
and "init would overwrite" as plain CLI errors, not diagnostics.

## 9. Diagnostics

New codes, following M1 §6.6. Every message is in plain words, names what
to change, and gives the Rust term in a note.

| Code | Meaning |
|---|---|
| V0100 (widened) | also a variant of an opaque imported enum, saying why it is opaque |
| V0101 (widened) | also a Rust struct or enum Varyk did not import (generic, tuple, or unit), saying why |
| V0103 (widened) | also a `use` name that clashes with an item in the same file |
| V0104 (widened) | also `shop.vr` beside `shop/mod.vr`; and a `.rs` file that parses but uses a crate not in `[dependencies]`, declares a module of its own, or uses `include!`, each at that line of the `.rs` file |
| V0105 (widened) | also a private module ("write `pub mod`"), a private field, and a struct literal with fields you cannot see, with the `pub`-or-constructor note |
| V0106 (widened) | also `main` defined in `lib.vr` |
| V0108 (widened) | also a field whose Rust type Varyk cannot use, and a type reached through a `use` in the `.rs` file |
| V0110 | `use` of a crate (`std` included); Varyk code reaches crates through a `.rs` module in the package |
| V0111 | a `use` path Varyk cannot follow: a variant, `super` in the crate root, a leading name that is not a module in this file (braces, globs, and `pub(crate)` are syntax and stay V0001, naming the construct) |
| V0400 | the package's edition is not 2024 |
| V0401 | `workspace = true`, a target-specific dependency table, `links`, or `[patch]` |
| V0402 | a Cargo target table; roots are `src/main.vr` and `src/lib.vr`, found by cargo's defaults |
| V0900 | the generated Rust was rejected by rustc, at the Varyk line; carries rustc's message and asks for a report |

## 10. Testing and definition of done

The posture of M1 §7 and M2 §7 holds: unit tests per pass, `insta`
snapshots for generated Rust and rendered diagnostics, integration tests
that build and run every example, CI with `fmt`, `clippy -D warnings`, and
`test`, on stable and 1.85.

- Package-mode tests use `path` dependencies on tiny fixture crates; no
  test reaches the network. Only the `matcher` example uses a registry
  crate.
- The `greeting` example is asserted equal to a fresh `varyk init` output
  for the files `init` writes.
- The publish test runs the assembly and `cargo package`; `cargo publish
  --dry-run` is not used because it contacts the registry.
- V0900 is tested by feeding the driver a fabricated cargo JSON message; a
  second test puts a type error in a fixture `.rs` module and asserts the
  report lands at the user's file and line.
- A fixture `.rs` module with a warning asserts passthrough; the same
  program's generated files produce none.
- Each diagnostic in section 9 has a test asserting code and span and a
  rendered snapshot.
- Soundness templates (M2 §7) are extended for field privacy, `use`,
  nested-module visibility, and imported structs and enums, in the
  must-build and must-reject sets. If the harness cannot hold multi-file
  templates, the imported-type cases go in integration tests and the plan
  says so.
- The deferred minors of the milestone-2 ledger are the first review
  round's input, per the project's review-loop practice.

Milestone 3 is done when the three example packages build and run in CI
with their expected output, `greeting` builds both ways, the `units`
consumer builds without `varyk`, every construct in sections 2 to 5 either
works or produces a named diagnostic, the tests above pass, and
`docs/language.md`, `docs/design.md`, `docs/open-questions.md`, and
`docs/roadmap.md` reflect this spec.

## 11. Roadmap changes

Milestone 3 in `docs/roadmap.md` is rewritten to this spec's items, with the
site kept as its own line. Moved to milestone 4: nested modules in `.rs`
files; tuple and unit struct import; `.rs` signatures naming Varyk types;
direct import of a published Varyk library; derives on imported structs.
Milestones 5 and 6 are unchanged.

## 12. Open questions

Added to M1 §9 and `docs/open-questions.md`:

- Should Varyk code ever `use` a crate directly, once traits exist? The
  facade rule of section 1 is the answer until then, and may remain it.
- Will generics ever be Varyk surface syntax? Milestone 3 assumes not and
  designs the facade around that.
- Should a Varyk library be importable from Varyk without a facade, given
  that its public API is already Varyk-shaped?

Half-answered from M2 §9: Rust enums from `.rs` modules are imported
automatically (section 4.3); generic types remain open.

## 13. Decisions

Appended to the decisions log of M1 §10.

| Decision | Choice | Why |
|---|---|---|
| Crates from Varyk | never named; used through `.rs` facades in the package | most crate APIs are generic; a facade is where the concrete API is written; keeps check-passes-means-compiles; M1 principle 6 |
| `use` | package paths only; emitted as canonical `crate::` paths, every use site written with its full path (amended: not verbatim, so the aliases never affect the generated Rust); no crates, braces, globs, variants | convenience without committing the keyword to a crate-import meaning before traits exist |
| Build model | hidden crate for `varyk build`; `build.rs` + `include!` stub from `init` for plain cargo; assembled plain crate for publish | Varyk keeps its diagnostics; cargo tooling works unchanged; consumers need no `varyk` |
| Lint allows | `#[allow(warnings, arithmetic_overflow, unconditional_panic)]` per generated item, never on a `mod` line (amended: a `mod` line's attribute reached `.rs` modules nested below) | one tree for all three destinations, works under `include!`, user `.rs` warnings and deny-level lints stay in force wherever the file sits; verified with rustc |
| Edition | 2024 required in package mode | the hidden crate and plain cargo must compile the same code the same way |
| Package mode detection | a file argument that is a package's crate root | no flag; `varyk run src/main.vr` gets the dependencies |
| Visibility | Rust's rule for modules, items, and fields; `pub mod` added; no `pub(crate)` | generated Rust must resolve; one rule to teach |
| Field privacy | private unless `pub`; breaking for M2 cross-module field reads | libraries need invariants; matches imported Rust structs; pre-0.1 |
| Rust import | structs with named fields, same-file inherent methods, unit-and-tuple enums; derives ignored | what a facade needs and nothing more; opaque otherwise, as M1 |
| Nested `.rs` modules | cut | a facade is one file; removes the `syn` tree walk |
| Rust-layer errors | line-level map, V0900 for generated files, verbatim passthrough for `.rs` files | the net under two-layer safety; user Rust stays in user terms |
| `check` and cargo | `check` never runs cargo, package mode included | M1 rule kept; TOML is parsed directly |
