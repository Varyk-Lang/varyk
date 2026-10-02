# Varyk milestone 5b2: packages

Date: 2026-10-02.

**Status.** Design, not yet implemented. Extends the milestone-1 to
milestone-5b1 specs, `2026-09-23-varyk-design.md` ("M1 §n"),
`2026-09-25-milestone-2-design.md` ("M2 §n"),
`2026-09-26-milestone-3-design.md` ("M3 §n"),
`2026-09-29-milestone-4-design.md` ("M4 §n"),
`2026-09-30-milestone-5a-design.md` ("M5a §n"), and
`2026-10-01-milestone-5b1-design.md` ("M5b1 §n"). Their principles,
ownership rules, and compiler architecture stay in force. Varyk is
experimental and pre-1.0: anything here may change.

## 1. Summary and scope

Milestone 5b2 lets Varyk code use a Varyk package. Until now a package
reached every dependency, a Varyk library included, through a `.rs` facade
(M3 §1). After 5b2:

- a dependency that is a Varyk package is named like a module:
  `units::length::add(a, b)`;
- a package may itself use Varyk packages, to any depth;
- `varyk` compiles every Varyk package it uses from the package's `.vr`
  sources, with the compiler of the person building, and never runs the
  package's build script or the Rust its publisher shipped;
- a Varyk package is built only by `varyk`. Its `Cargo.toml` names its
  `.vr` root, so cargo's tools (`cargo add`, `update`, `tree`, `audit`)
  work on it; `varyk init` writes no `build.rs` and no stub, and
  `varyk emit` is removed.

A Rust crate is still reached through a `.rs` facade, and a published
Varyk crate is still a plain Rust crate for cargo users (M3 §2.6). Those
rules are unchanged.

### 1.1 Why packages come before HTTP and the database

The roadmap had 5b2 as the HTTP server, the HTTP client, and the database,
all inside `varyk-std`. That is replaced. The decisions, taken while
designing this milestone:

- **`varyk-std` stays the small runtime every program has.** It holds what
  the language itself needs and the light things nearly every service uses:
  `Error`, tasks, `json`, `env`, `log`.
- **Anything heavy is a package the writer adds.** SQL, MongoDB, Redis,
  the HTTP server and client. A program that prints a line never compiles
  a web server or a database driver.
- **Official packages are named `varyk-*`, community packages `*-varyk`.**
  `TRADEMARKS.md` already draws this line. Names carry neither `std` nor
  the crate underneath: `varyk-sql`, `varyk-http`, `varyk-mongo`,
  `varyk-redis`. A name is published when the crate has real content.
- **The mechanism comes first.** With packages in place, a battery is
  written once as a package, by this project or by anyone, and needs no
  work in the compiler.

Milestone 5 therefore continues as: 5b2, packages (this document); 5b3,
what a `.rs` facade can say, and `varyk-sql`; 5b4, `varyk-http` and the
golden path. Section 10 records what was agreed for 5b3 and 5b4.

### 1.2 Varyk builds Varyk

Until now a package from `varyk init` carried a `build.rs` and a one-line
stub so that plain `cargo build` could compile it (M3 §2.5). That is
dropped. Varyk code is built by `varyk`, as Go code is built by `go`; a
package is `Cargo.toml`, its `.vr` files, and its `.rs` facades.

Cargo still has to read the package: `cargo add` (behind `varyk add`),
`cargo update` (which V0404's help names), `cargo tree`, `cargo audit`,
dependency bots, and cargo's own resolution of a package's dependents all
parse its manifest, and cargo refuses a manifest with no target. So the
manifest names the `.vr` root as the target:

```toml
[[bin]]
name = "trip"
path = "src/main.vr"
```

or, for a library, `[lib]` with `path = "src/lib.vr"`. Every cargo tool
then works on the package. Plain `cargo build` does not: rustc reads the
`.vr` file as Rust and stops on Varyk's syntax. `docs/language.md` says a
Varyk package is built with `varyk build`.

Plain cargo still builds a *published* Varyk crate, which carries its
generated Rust (section 4.5), so a Rust project can depend on one as on
any crate.

This removes the build-script path that would otherwise have to be made
safe (section 3), and every file in a package is now one the writer
wrote. Varyk has no users yet whose packages this breaks.

Milestone 5b2 is an MVP like the milestones before it. Only what this
document lists is supported; anything else is rejected with a diagnostic
that names the construct.

## 2. Language surface

A program using a package, with `units = { path = "../units" }` in its
`Cargo.toml`:

```
use units::length::Meters;

fn main() {
    let a = Meters { value: 3 };
    let b = units::length::Meters { value: 4 };
    let sum = units::length::add(a, b);
    println!("{} meters", sum.value);
}
```

There is no new syntax. One kind of name is new: the name of a package.

### 2.1 Naming a package

The name of a package in Varyk code is its key in `[dependencies]`, with
each `-` read as `_`, which is cargo's rule for the crate's name in Rust.
So `route-planner = "1"` is `route_planner::`, and
`sql = { package = "varyk-sql", version = "0.5" }` is `sql::`. The writer
of `Cargo.toml` chooses the name, and two packages cannot share one.

The first name of a path, in an expression, a type, a pattern, or a
`use`, is looked up in this order:

1. a module declared in the current module, or a `use` alias, as today;
2. a standard module or type (`json`, `Vec`, ...), as today;
3. a dependency of the package the code is in.

So a local module wins over a package of the same name, and a package
whose key is a standard name, a keyword, or begins with `varyk_` cannot be
named: rename it in `Cargo.toml` with `package = ".."`. The help of
V0113, of V0100, and of the V0111 below shows that line when the name it
reports is also a dependency. A dependency key of `std`, `core`, or
`alloc` is V0401 in any package, since it hides the Rust standard library
that the generated Rust uses.

After the package's name the path continues as a `crate::` path would
inside that package: `units::length::add` is `crate::length::add` there.
`use` takes a package path under the rules of M3 §3.3. `use units;` alone
is V0111, as a lone module name is today, since the name is already in
scope.

Step 3 applies to a `use` and to a path of two or more names; a single
name is never a package. Today's V0111 for a `use` whose leading name is
a module declared elsewhere in the package still comes first.

### 2.2 What a package is

A dependency is a Varyk package when its directory has `src/lib.vr`. It
may come from a `path`, a registry, or `git`; cargo finds it (section
4.1).

Any other dependency is a Rust crate. Naming one in Varyk code, in a path
or a `use`, is V0110, whose help is the facade rule of M3 §1. A Varyk
program (`src/main.vr`, no `src/lib.vr`) is not a package in this sense.

Only `[dependencies]` is read. A package under `[dev-dependencies]` or
`[target.'cfg(..)'.dependencies]` (V0401 already) cannot be named. Naming
a dependency marked `optional` is V0401, "an optional dependency cannot
be named yet", whatever it is: cargo leaves one that no feature turns on
out of the graph, so Varyk cannot tell what it is.

### 2.3 What is visible

Everything a package marks `pub` and that Rust's visibility rule lets a
user of the crate reach (M3 §3.2): `pub` functions, structs with their
`pub` fields, enums, methods, and associated functions, in `src/lib.vr`
and in its `pub mod`s. The `.rs` modules of the package are part of it,
imported under the rules of M3 §4 and later.

In a library, V0105's rule that a `pub` item may not name a type some of
its users cannot see (M3 §3.2) counts the library's outside users too. So
a `pub fn` in `src/lib.vr` that returns a type from a private module is
V0105 in the library itself, since a user in another crate could not name
that type and the generated Rust sometimes writes it (a `let` holding an
`Option`, `Result`, `Vec`, or `HashMap`, `backend/rust.rs` ~589). The rule
covers the library's `.rs` modules too: a `pub` function, method, or
field of a `.rs` module that outside users can reach, and that names a
type they cannot reach, is V0105.

A package's items behave as Varyk items do, because they are Varyk items:

- parameters borrow, and `mut` parameters and `mut self` change the
  caller's value;
- a function returning part of a parameter gives an alias (M4 §3);
- an async function is awaited or started (M5b1 §2.3);
- an enum is matched with every pattern of M4, exhaustiveness included;
- `.clone()` and `==` work where the type's fields allow (M4 §2.10).

What a package cannot be given from outside:

- an `impl` block. An `impl` names a type declared in the same file
  (V0001, as today);
- JSON or environment conversion (section 2.5).

`#[test]` functions of a package are not visible, and `varyk test` runs
only the tests of the package it is run in.

### 2.4 Packages that use packages

A package names its own dependencies by the same rules. `trip` may use
`route`, and `route` may use `units`, each through its own `Cargo.toml`.

Each package sees only its own dependencies. A value whose type is
declared in a package the code's package does not depend on is V0115:

```
let legs = route::legs();          // Vec<route::Leg>, fine
let m = route::total(legs);        // units::length::Meters
```

The second line is V0115 in a package that depends on `route` and not on
`units`. The help is the line to add to `Cargo.toml`. The rule is
over-strict and sound: Rust could hold such a value without naming its
type, but the generated Rust writes the type of some `let`s (section 2.3),
and one rule is easier to learn than a list of places.

When cargo resolves two versions of one package, they are two packages.
A value of `units` 1.0 from `route` is not a value of the `units` 2.0 the
program depends on; that is V0115 too, and its note names both versions.

### 2.5 JSON and configuration across packages

Serde is derived where a type is declared (M5a §7.5), and a package is
compiled without knowing who uses it. So a `json` or `env` call on a type
declared in another package, or holding one, is V0210, with the note
"`Stop` is declared in the package `route`" and the help to convert it
there instead (`pub fn stop_json(stop: Stop) -> string`).

Whether a library should derive serde for every `pub` type it declares is
an open question (section 11).

### 2.6 Not in milestone 5b2

Re-exports (`pub use`), so a package's path is the path of its modules;
shorthands in `varyk add` (`varyk add sql`); a Varyk package under
`[dev-dependencies]`, marked `optional`, or reached through a Rust crate;
reading a package's `[features]`; skipping the compile of a trusted
package; a summary file so `check` need not read a package's sources;
rustc errors in a package mapped back to its `.vr` lines; a Varyk package
used from source by a plain Rust project; everything of 5b3 and 5b4
(section 10).

## 3. Safety

`AGENTS.md` gains the rule "Safe by default" with this milestone. For
packages it means one guarantee:

> When `varyk` builds your program, the Rust of every Varyk package in
> the build is exactly that package's hand-written `.rs` modules and what
> your own compiler generates from its `.vr` files. Nothing else in the
> package is compiled or run.

Without it, a published crate could carry clean `.vr` files beside
generated Rust that does something else, or a build script that does, and
a reviewer reading the Varyk would never see it.

It holds because the driver builds each Varyk package itself (section
4.3), into a crate it writes, and refuses a Varyk package that cargo
would build on its own (section 4.1):

- the manifest is the package's own, with `build = false`, so the
  package's `build.rs` never runs, and its target set by the driver to
  the generated root, so no `[lib] path` in the package's manifest is
  followed;
- `src/` holds the generated tree and the `.rs` modules the package's
  `mod` declarations load, and nothing else;
- a module file present as both `.vr` and `.rs` in a dependency is loaded
  from the `.vr`, and the `.rs`, which is what a publisher's assembly put
  there (section 4.5), is ignored. The root's `src/lib.rs` is ignored the
  same way. In the package being built, both is V0104, as today, except
  for the root: a `src/main.rs` or `src/lib.rs` beside the `.vr` root, a
  stub left from an earlier `varyk init`, is ignored there too.

What the guarantee does not cover, on purpose: the package's own `.rs`
modules, the crates it depends on, and their build scripts and macros.
Those are Rust, visible as files or as lines of `Cargo.toml`, and they
are the reader's and rustc's to judge (`AGENTS.md`, "The Rust layer is
rustc's job"). A Rust project that depends on a published Varyk crate
with plain cargo trusts its shipped Rust as it trusts any crate.

## 4. Packages and the build

### 4.1 The package graph

Cargo decides which version of each dependency a build uses and where its
files are. Varyk asks it, in `check`, `build`, `run`, `test`, and
`publish` alike, whenever `[dependencies]` or `[dev-dependencies]` holds
anything but `varyk-std`, so that a program that passes `check` builds.
A package with no other dependency never runs cargo in `check` (M3
§2.2).

The driver writes the package's isolated manifest (M3 §2.2) to
`target/varyk/packages/graph/`, puts the package's lock beside it as it
does for a build (copied when there is one, removed when there is none),
and runs `cargo metadata
--format-version 1 --filter-platform <host>` there, with the host that
`rustc -vV` reports. The filter keeps cargo from downloading crates for
other platforms; it never hides a Varyk package of the build, since
those are reached only through plain `[dependencies]`, and the refusals
below are made on the host's graph. The manifest keeps the package's own
target, the `.vr` root (section 4.6), as a relative path: cargo accepts
it although no file is there. The package's own directory is never
written to (M3 never copies the lock back). The lock cargo leaves beside
that manifest is the one the build uses (section 4.3), so the graph Varyk
checked is the graph cargo builds.

From the answer Varyk reads each package's directory, name, and version,
and which package each dependency key of each package resolved to. The
**Varyk packages of the build** are those reached through
`[dependencies]` from the program, and from the Varyk packages so
reached. Each is loaded from its directory, checked (section 4.2), and
built by the driver (section 4.3), whether or not any code names it.
Every other package is left to cargo.

Two things are refused, each V0401, "not supported yet", at the
program's `Cargo.toml`, with a message that names the package and what
reaches it:

- a Varyk package reached any other way: through `[dev-dependencies]`,
  as an `optional` dependency a feature turned on, or through a Rust
  crate, even when it is also reached the supported way. Cargo would
  compile it itself, a source package's `.vr` as Rust and a published
  one from its shipped Rust, outside the guarantee of section 3;
- a Varyk package of the build with the same name and version as
  another package the build reaches by `path` or builds as one, `units`
  0.1.0 from a registry beside `units` 0.1.0 by `path`. The driver makes
  every Varyk package a `path` package (section 4.3), and cargo refuses
  two of one name and version. (Two by `path` cargo refuses already, as
  V0405.)

Telling a Rust crate from a Varyk package takes the graph, so the V0110
for naming a Rust crate now comes after cargo ran, and offline may be a
V0405 instead.

Cargo may download what is not on the machine. If it fails, the error is
V0405 with cargo's message.

No cache is kept; one is added if measurements ask for it. A single file
has no `Cargo.toml` and no packages.

### 4.2 Checking a package

Each Varyk package of the build is checked as a program of its own, with
its own `Cargo.toml`, before the code that uses it, and the Varyk
packages it depends on before it. The whole check runs (resolve, types,
borrow analysis), because what a caller needs is partly inferred from
bodies: whether a function returns part of a parameter (M4 §3). A
package is loaded and checked once per package cargo resolved (by its
directory), however many paths reach it, so `units` reached from `trip`
and through `route` gives one `Meters`.

Its `pub` items then enter the user's tables as imported items, the
mechanism of M3 §4.1, carrying what a caller needs:

- for a function or method: each parameter's mode, whether it is async,
  and the parameter its return value is part of, if any;
- for a struct or enum: its fields or variants in full, which fields are
  `pub`, and whether it has `Clone` and `PartialEq`.

The backend emits nothing for an imported item in the user's crate.

Logging is set up by the program's `main` when the program logs (M5a
§7.5). A program now counts as logging, and so as using `varyk-std`,
when any Varyk package of the build calls `log`, so a package's log
lines are not lost in a program that writes none itself.

If a package does not pass, its diagnostics are shown at its own files,
each with the note "in the package `units` 0.1.0, which this build
uses", and checking stops there. The usual cause is a package
written for another version of Varyk; then one of the diagnostics is the
V0404 on the package's own `varyk-std` line, which says so.

For a package that is a dependency:

- `Cargo.lock` beside it is ignored. Only the lock of the package being
  built decides versions, so the lock rule of V0404 is not applied to
  it. Instead, when any package in the build uses `varyk-std`, the
  version the `varyk-std` lines of the program and of its Varyk packages
  resolve to must be no older than the compiler's: V0404 at the
  program's own `varyk-std` line or, when it has none, at its
  `Cargo.toml`, with `cargo update -p varyk-std` as help. It is not
  reported again when the program's own lock rule already reported it.
- Its manifest may be a published one (section 4.4).

### 4.3 Building

`varyk build`, `run`, and `test` write the program's own crate as before
(M3 §2.3), and before running cargo they write one crate for each Varyk
package of the build under `target/varyk/packages/<name>-<version>/`:

- `Cargo.toml`: the package's isolated manifest (M3 §2.2: `build =
  false`, paths made absolute, an empty `[workspace]`), with its target
  set to the generated root, `src/lib.rs`, named after the package;
- `src/`: the package's generated tree and its `.rs` modules, as for a
  program (M3 §2.3).

In every manifest the driver writes, the program's and the packages',
each `[dependencies]` entry on a Varyk package is replaced by a `path` to
the crate written for it, keeping the key, so the generated Rust's crate
names (section 5) still hold. The entry keeps `package`, `features`, and
`default-features`, and drops `version`, `git`, `branch`, `tag`, `rev`,
and `registry`. The program's hidden manifest, whose target is the
generated root as before, is the one cargo builds; all others are reached
from it, and the lock is the one section 4.1 left.

Files are written only when their content changes, and files no longer
generated are removed, so cargo recompiles a package only when it
changed. Every package is generated again on every build, which is
cheap; a change to `units` therefore reaches `route` and `trip`. A
directory left under `target/varyk/packages/` by a package no longer in
the build is harmless and stays.

A rustc error inside a package's crate is reported as V0900 with rustc's
own message, naming the package; it has no Varyk line, since the source
map of M3 §5 covers only the program's own crate. When the file is one
of the package's own `.rs` modules, the note says the error is in that
package's Rust and does not ask for a Varyk bug report. Warnings from a
package's crate are not shown.

### 4.4 Published manifests

A published Varyk crate's manifest differs from a source package's.
`varyk publish` sets `build = false` and the target to `src/lib.rs`
(section 4.5), and `cargo publish` then rewrites the file: it adds `name`
to `[lib]`, sets `autolib`, `autobins`, `autoexamples`, `autotests`, and
`autobenches` to `false`, and adds `readme = false`, among others.
`varyk check` refuses the `auto*` keys today (V0401).

In a dependency these are accepted when each says what is shown here:
`build`, if present, `false`; the target's `path` `src/lib.rs` and its
`name`, if present, the package's name with `-` as `_`; the `auto*` keys,
if present, `false`.
Anything else is the error it is today. The driver names the target
itself (section 4.3), so these keys are only read, never followed. The
exact list is pinned by a test that runs `cargo package` on the crate
`varyk publish --assemble-only` makes of `units` and loads the result, on
stable and on 1.85. A key a later cargo adds is refused until Varyk
learns it, with the note that the package may have been published with a
newer cargo.

### 4.5 `varyk publish`

As in M3 §2.6, the assembled crate holds the isolated manifest, with
`build = false`; the generated tree, so `src/lib.rs` is the generated
root; the `.vr` files beside the files they produced; and the readme and
license files. One change to its manifest: the target names the
generated root (`src/lib.rs` or `src/main.rs`) in place of the `.vr`
root.

A `path` dependency on a Varyk package keeps its `version`, with its path
made absolute as `isolated_manifest` does. Cargo drops the `path` and
keeps the `version` when it publishes, so the dependency needs a
`version` (`varyk add --path` writes one), and that package is published
first, as with any Rust crate.

A cargo user builds the shipped Rust. A `varyk` user's compiler reads the
`.vr` files and ignores it (section 3).

### 4.6 `varyk init`, `varyk add`, and `varyk emit`

`varyk init` writes `Cargo.toml`, with the target table of section 1.2,
`.gitignore`, and `src/main.vr` (or `src/lib.vr`); no `build.rs` and no
stub. Its message says to run `varyk run` (or `varyk build` for a
library), no longer `cargo run`. `varyk add` stays a plain pass-through to
`cargo add`.

The target table becomes the one `varyk check` accepts, and requires, in
a package: `[[bin]]` with `name` the package's name and `path =
"src/main.vr"`, or `[lib]` with `path = "src/lib.vr"` and, if present,
`name` the package's name with `-` as `_`, and no other key. A package
without it is V0406, whose help is the lines to add; any other target
table stays V0402. V0406 is for the package being built. A dependency
may have the published target instead (section 4.4), and one with no
target at all is refused by cargo first, as V0405.

`varyk emit` is removed: the build script was its only caller.
`--emit-rust` stays for looking at the generated Rust.

The `build` key (`package.rs` ~355) may only be left out or `false`. No
crate Varyk builds runs a build script, so `build = "build.rs"` joins
every other script as V0401. `links` stays V0401; its note now says that
every crate Varyk builds has `build = false`.

### 4.7 Building this repository

M5a §5.3 made the generated crate's `varyk-std` a `path` dependency under
`VARYK_STD_PATH`. A Varyk package in the same build has its own
`varyk-std` line, which would bring in a second copy from the registry,
and then two `Error` types. So under `VARYK_STD_PATH`:

- M5a's `path` swap is applied to every manifest the driver builds from,
  the program's and each package crate's (section 4.3), so a build needs
  nothing new;
- the graph manifest (section 4.1), which reaches the packages' own
  manifests, carries `[patch.crates-io] varyk-std = { path =
  "<VARYK_STD_PATH>" }` instead. Cargo asks the registry's index when it
  resolves under a patch, so reading the graph of a package with a
  `varyk-std` line needs the index, as building `varyk-std`'s own
  dependencies already needs the registry.

The example harness copies one package to a temporary directory; for the
examples of section 6 it copies the packages they depend on beside it.

Nothing new is released in 5b2: there is no official package yet.

## 5. Generated Rust

| Varyk | Rust |
|---|---|
| `units::length::add(a, b)` | `::units::length::add(&a, &b)` |
| `units::length::Meters { value: 3 }` | `::units::length::Meters { value: 3 }` |
| `use units::length::Meters;` | `use ::units::length::Meters;` |
| `fn f(m: units::length::Meters)` | `fn f(m: &::units::length::Meters)` |

The crate name is the dependency key with `-` as `_` (section 2.1), which
is the name rustc knows it by: the driver names each package's library
target after the package (section 4.3), and the key renames it. A leading
`::` keeps a local module of the same name out of the way.

Inside a package its own items are `crate::` paths, as today, since the
package is compiled as its own crate. A type of a package is written
with the key under which the package whose code is being generated
depends on its declaring package, not the key of a package it came
through: in `trip`, `route::total`'s result is `::units::length::Meters`
when `trip` lists `units`, and `::u::length::Meters` when it lists it as
`u = { package = "units" }`.

## 6. Milestone-5b2 examples

Under `examples/packages/`, each with its expected output in
`tests/examples.rs`:

- `route`, a new library that depends on `units` by path. It declares
  `Leg` with a field of type `units::length::Meters`; `total`, which adds
  the legs with `units::length::add`; `longest`, which returns one of its
  parameter's legs, so a borrowed return crosses a package; and
  `parse_stop`, which reads its own `Stop` from JSON and logs a line
  when it fails, so it uses `varyk-std`.
- `trip`, a new program that depends on `route` and `units`. It builds
  legs, prints the total and the longest, matches on a result of
  `route::parse_stop`, and awaits one async function of `route`. It
  calls `log` nowhere itself, and the test sees `route`'s line.
- `units`, unchanged in its `.vr` files. Its `consumer`, plain Rust built
  by plain cargo, still builds against the published form.

`greeting`, `units`, and `users` lose their `build.rs` and stub. Every
example package gains the target table of section 4.6, which V0406
requires and `greeting` must have to equal what `varyk init` writes; so
do the `Cargo.toml` files under `tests/fixtures/` and the manifests
written out in `package.rs`'s unit tests. `route` and `trip` join the
`extra-files` of release-please for their `varyk-std` lines (M5a §5.3).

## 7. Compiler changes

Line numbers are approximate.

### 7.1 The graph (`crates/varyk/src/packages.rs`, new)

- Writes the manifest of section 4.1, runs `cargo metadata`, and reads
  its JSON with `serde_json`, already a dependency of the driver.
- Tells Varyk packages from Rust crates (`src/lib.vr`), finds the Varyk
  packages of the build, and applies the two V0401 rules of section 4.1.
- The resolved-`varyk-std` rule of section 4.2 (V0404).

### 7.2 Package (`package.rs`)

- A way to load a package as a dependency: a published manifest accepted
  (section 4.4), and the lock check of `check_std_dependency` (~633,
  `locked_older` ~730) left out.
- The `build` rule of section 4.6 (~352), the new note on `links`
  (~226), and V0401 for a dependency keyed `std`, `core`, or `alloc`
  (section 2.1).
- The target table of section 4.6 accepted and required (V0406). The
  graph manifest keeps it (section 4.1); the hidden crate, the package
  crates, and the assembly replace it with the generated root.
- `name_problem` (~780) reserves `packages` beside `cache` and
  `package`, since a package of that name would have its hidden crate at
  `target/varyk/packages/`; `sanitize_name` in `driver/mod.rs`, which
  `varyk init` uses, and its guard test follow.

### 7.3 Resolve

- `resolve/paths.rs` (`module_at`, ~68): step 3 of section 2.1 after the
  child module and the alias.
- `resolve/uses.rs` (`leading_name_error`, `is_known_crate_name`,
  ~676-718): a dependency that is a Varyk package is no longer V0110; a
  Rust crate is, in a path as well as a `use`; an `optional` one is
  V0401; the rename help of section 2.1 on V0100, V0111, and V0113.
- `resolve/modules.rs` (`find_file`, ~173): in a dependency, a `.vr`
  file wins over a `.rs` file of the same module (section 3). Nothing
  reads a `.rs` file beside the `.vr` root today, so ignoring a leftover
  stub needs no change.
- A package's `pub` items enter `Symbols` as imported functions, structs,
  and enums (`resolve/mod.rs`, `ImportedSig` ~149, the `imported` flags
  ~213 and ~267), each with the package it belongs to and its path there.
  An imported enum declared in a package's `.vr` keeps every variant,
  named fields included; one declared in a package's `.rs` module keeps
  the rules of M3 §4.3.
- `lib.rs` (`check_file`, ~40): checks the Varyk packages of the build,
  dependencies first, before the program.

### 7.4 Types and borrow analysis

- `names_error` (`types/check.rs` ~222) skips imported items, so a
  package's struct holding an `Error` does not make its user need
  `varyk-std`.
- V0115 where an expression's type names a struct or enum of a package
  the current package does not depend on.
- V0210 for a type declared in another package (section 2.5).
- V0105 in a library counts its outside users, for its `.rs` items as
  well (section 2.3).
- The program's `logs` (`types/check/values.rs` ~466), and with it
  `uses_std`, is also true when a Varyk package of the build logs
  (section 4.2).
- An imported function from a package carries `ret_root` (`hir.rs`
  ~184), as an imported `.rs` function does (`ImportedSig` ~171), so the
  alias rule applies to its result.

### 7.5 Backend

- A path to an item of a package is written from the crate name of the
  dependency (section 5), in `rust_type` and wherever a `crate::` path is
  written today (`backend/rust.rs` ~104, ~410).

### 7.6 Driver and command line

- `driver/mod.rs` (`build`, ~129): the package crates of section 4.3,
  the dependency rewrite, writing a file only when it changed, and the
  graph's lock copied into the hidden crate (section 4.1).
- `backend/mod.rs` (`with_local_std`, ~113): section 4.7.
- `driver/init.rs`: the target table, no `build.rs` (its template is
  deleted) and no stub (`stub()`, ~22, removed).
- `driver/publish.rs` (`assemble`, ~40): the manifest change of section
  4.5.
- `cli.rs`: `varyk emit` (`run_emit`, ~568) is removed, with
  `driver::generate`'s `emit`-only checks.
- `driver/messages.rs`: a rustc error in a package's crate is V0900
  naming the package (section 4.3); its warnings are dropped.

## 8. Diagnostics

New codes, each with a fixture, a registration in `tests/errors.rs`, and a
row in `docs/language.md`:

| Code | Meaning |
|---|---|
| V0115 | a value whose type is declared in a package this package does not depend on, or in another version of one it does |
| V0405 | cargo could not say which packages the build uses; the message carries cargo's own |
| V0406 | a package whose `Cargo.toml` does not name its `.vr` root as its target |

Reused: V0401 for a dependency keyed `std`, `core`, or `alloc`, for
naming an `optional` dependency, for a Varyk package reached in a way
section 4.1 refuses, for a Varyk package sharing its name and version
with another `path` package, and for a `build` script; V0110 for a Rust
crate named in a path or a `use`;
V0100 and V0113, with a help line showing the rename, for a package whose
key cannot be a name; V0105 for an item of a package that is not `pub`,
and for a `pub` item of a library naming a type its outside users cannot
reach; V0111 for `use units;`; V0210 for JSON on a type declared in
another package; V0001 for an `impl` of a package's type; V0404, at the
package's own manifest, for a package written for another version, and
at the program's for an older resolved `varyk-std` (section 4.2); V0900
for a rustc error in a package's crate.

Headlines in plain words, for example V0115: "this is a `Meters` from the
package `units`, which this package does not list", with the line to add
as its help.

## 9. Testing and definition of done

- Unit tests for the graph, from recorded `cargo metadata` output: Varyk
  packages told from crates, renamed keys, two versions of one package,
  a package that uses a package, each refused way of reaching a Varyk
  package, two packages of one name and version, and a resolved
  `varyk-std` older than the compiler's (V0404).
- Resolver tests: the order of section 2.1; `crate::` inside a package
  meaning that package; a non-`pub` item; a Rust crate named in a path;
  a dependency module present as `.vr` and `.rs` loaded from the `.vr`.
- `insta` snapshots of the generated Rust of `trip` and `route`, and of
  the manifests the driver writes for them.
- Soundness templates (M2 §7) across a package boundary: a borrowed
  return, a `mut` parameter, a struct literal, a `match` on an enum with
  named fields, a method, and an async function awaited and started.
- Integration, in `tests/examples.rs` and `tests/packages.rs`:
  - `trip` builds and prints under `varyk run`;
  - `units` in published form builds under plain cargo (the `consumer`),
    and under `varyk` from a Varyk program that depends on the assembled
    crate;
  - changed Rust in that assembled crate's `src/`, and a `build.rs` added
    to it that would fail, with `build = false` removed from its manifest
    so plain cargo would run it, have no effect under `varyk`;
  - after a change to `units`, building `trip` again picks it up through
    `route`, and a second build with no change recompiles no package;
  - `cargo package` on the assembled `units` gives a manifest that loads
    as a dependency (section 4.4), on stable and 1.85;
  - an `insta` snapshot of the manifest `varyk publish --assemble-only`
    writes for `route`: the target `src/lib.rs`, `build = false`, and
    the `units` dependency with its `version` kept (its absolute path
    redacted);
  - `varyk build` after `varyk add --path ../units`, before any code
    names `units`, builds;
  - `varyk init` writes three files, and `varyk add --path ../units`
    works in `route`.
- Fixtures, each run from a temporary copy as the examples are, since
  asking cargo for the graph writes under the package, and none with a
  `varyk-std` line, so none needs the network (section 4.7): V0115 (both
  cases); V0405 (a `path` dependency whose
  directory is missing); V0406 (no target table); V0110 in a path (to a
  local `path` Rust crate); V0401 for a Varyk package under
  `[dev-dependencies]`, for one a local Rust crate depends on, for an
  `optional` dependency named, and for a dependency keyed `std`; V0210
  across packages; V0105 for a library's `pub fn`
  returning a type of a private module and for a library `.rs` function
  returning a type of a private `.rs` module; and an error inside a
  dependency with its note. The existing fixture
  `v0104_rust_crate_dev_dependency`, whose `serde` would now send `check`
  to the registry, takes a local `path` crate and runs from a copy too.
- Removed: the tests of `varyk emit` (in `tests/cli.rs` and
  `tests/packages.rs`) and of plain `cargo build` on a source package
  (`greeting_runs_the_same_through_varyk_and_through_cargo` in
  `tests/examples.rs`). `cargo_in` and `std_config` stay for the tests
  that still use them, the `units` consumer among them.
- `docs/language.md` gains a "Packages" section (sections 2 to 4 here)
  and the new codes; widens the V0105 and V0401 rows; replaces "`varyk
  init` and plain `cargo build`" with what section 4.6 says; removes
  `varyk emit` from the command table and the text; drops "never runs
  cargo" from `check`; says that naming a Rust crate asks cargo; and
  updates "`varyk add`". `README.md` drops its promise of plain `cargo
  build`. `docs/design.md` gains the ecosystem shape of section 1.1,
  section 1.2, and the decisions of section 12; `docs/open-questions.md`
  and `docs/roadmap.md` follow sections 10 and 11. `AGENTS.md` gains the
  `packages.rs` line under "Where things are", and its "`Cargo.toml` is
  cargo's job" paragraph drops plain `cargo build` of a source package;
  the "Safe by default" rule lands with the spec. The V0401 notes on
  `build` and `links` and the `isolated_manifest` doc comment follow
  section 4.6.

Done when every example in section 6 and every earlier example builds and
prints its expected output, the tests above pass, and the gate of
`AGENTS.md` passes on stable and 1.85.

## 10. Roadmap changes

Milestone 5's golden path gains one step: a users API on a database is
`varyk init`, `varyk add http sql`, one file, and `varyk run` away. It is
met at the end of 5b4.

The 5b2 entry becomes:

- A Varyk package named from Varyk code by its `Cargo.toml` key, to any
  depth
- Varyk packages compiled by `varyk` from their `.vr` sources, never from
  shipped Rust or a build script
- Packages built only by `varyk`: `Cargo.toml` names the `.vr` root, no
  `build.rs` or stub from `varyk init`, `varyk emit` removed
- Examples `route` and `trip`; `docs/language.md`, `docs/design.md`,
  `docs/open-questions.md`, and `docs/roadmap.md` updated

"Direct import of a published Varyk library from Varyk" leaves
milestone 6: this is it. The milestone-3 entry "`varyk init` with a
`build.rs` so plain `cargo build` works, and `varyk emit`" gains a note
that 5b2 removed it.

New entries, with what was agreed while designing 5b2:

**Milestone 5b3: facades and `varyk-sql`.**

- A `.rs` function with one type parameter standing for any Varyk data
  type (serde's `Serialize` or `DeserializeOwned`), chosen from the
  argument or from where the result goes
- A `.rs` function whose last parameter takes any number of data values
- A `.rs` parameter of type `&'static str` taking only text written in
  the program
- `varyk_std::Error` in a `.rs` signature
- `pub use` in a `.vr` file, and `varyk add` shorthands for official
  packages
- `varyk-sql` on sqlx: SQLite, Postgres, and MySQL, each a cargo feature;
  `sql::connect(url)`, and on a pool `one`, `first`, `all`, and `run`,
  each taking the query and its values; rows read into structs by column
  name; each database's own placeholders, passed through; the query text
  a literal, so a query built from input is a compile error; no secret in
  an error message

The former item "`.rs` signatures naming Varyk-declared types" is dropped
to "Unscheduled": the type parameter covers what a facade needs.

**Milestone 5b4: `varyk-http` and the golden path.**

- An HTTP server with an explicit route table (`app.get("/users/{id}",
  get_user)`); a handler's parameters bound by name to the route, by type
  to the JSON body and to shared state; the route checked against the
  handler by `varyk check`; the return value as the response, `None` as
  404
- `Error` carrying an optional status set by constructors
  (`http::bad_request(..)`); an error without one is a 500 whose message
  is logged and not sent
- An HTTP client in the same package
- The `users` API on a database, and the fifteen-minute path

The agent evaluation follows 5b4. `varyk-mongo` and `varyk-redis` are
listed after it as the next packages.

## 11. Open questions

Answered, marked so in `docs/open-questions.md`:

- "Should a Varyk library be importable from Varyk without a facade?"
  Yes, by its `Cargo.toml` key (section 2).
- The standard modules of the site's hero: `http` and `db` are packages a
  program adds, named by their keys; `json` stays in `varyk-std`.

Added:

- Should a library derive serde for every `pub` type it declares, so its
  users can send its types through JSON (section 2.5)? It would make
  every such library depend on `varyk-std`.
- Should code be able to hold a value of a package it does not depend on
  (section 2.4)?
- Should a builder be able to skip compiling a package it trusts, an
  official one for example, and use its shipped Rust?
- Should a package ship a summary of its `pub` items, so `varyk check`
  need not read and analyse its sources?
- After 1.0, how is a package written for one version of the compiler
  kept working with the next?
- Should a Varyk package be usable under `[dev-dependencies]`, from
  tests, or when a Rust crate in the build depends on it too?
- Should a plain Rust project be able to use a Varyk package from source,
  not only in its published form?

## 12. Decisions

Appended to the decisions log of M1 §10.

| Decision | Choice | Why |
|---|---|---|
| Milestone 5 split (5b2) | 5b2 packages; 5b3 facades and `varyk-sql`; 5b4 `varyk-http` and the golden path | batteries become packages, so the mechanism comes first; each slice testable on its own |
| Batteries | `varyk-std` is the small runtime; anything heavy is a package | a program compiles only what it uses; anyone can write a battery |
| Package names | `varyk-*` official, `*-varyk` community; no `std`, no backing crate in the name | `TRADEMARKS.md` already draws the line; the crate underneath may change |
| Name in code | the `[dependencies]` key | the writer chooses it; no second naming scheme; it is the crate name in the generated Rust |
| What a package is | a dependency with `src/lib.vr` | no marker to forget or forge |
| How a package is compiled | by the driver, from its `.vr` and its `.rs` modules, into a crate it writes | shipped Rust and build scripts could differ from the Varyk a reviewer reads |
| Who builds Varyk | `varyk` only; no `build.rs` or stub in a package; `varyk emit` removed | fewer files, and no build-script path to make safe; published crates still serve cargo users |
| A package's target | `Cargo.toml` names the `.vr` root (`[lib] path = "src/lib.vr"`) | cargo's tools need a target to read the manifest; no Rust file needed |
| Published form | M3's: generated Rust with the `.vr` beside it, the target naming the generated root | cargo users and docs.rs need no `varyk` |
| A Varyk package reached another way | refused, V0401: `[dev-dependencies]`, `optional`, through a Rust crate | cargo would build it itself, outside the guarantee; one rule, no exceptions |
| Reading a package | the full check on its sources, its `pub` items imported | borrowed returns are inferred from bodies |
| When Varyk asks cargo for the graph | in every command, when any dependency besides `varyk-std` is listed; on a manifest in its own directory | a program that passes `check` builds; the graph is the build's own |
| JSON on another package's type | refused, V0210; convert it in its own package | serde is derived where a type is declared |
| A type from a package not depended on | refused, V0115 | the generated Rust must be able to name it |
