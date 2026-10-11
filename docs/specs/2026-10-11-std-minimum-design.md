# A package's varyk-std line is its minimum varyk

Date: 2026-10-11.

**Status.** Implemented, not yet released. Today a program and every Varyk
package of its build must name `varyk-std` on the compiler's own minor
version (V0404), so every minor release of the compiler, even one that
changes nothing in `varyk-std`, stops every published package from
building until it releases again. This spec turns the line into a
minimum: a package written for varyk 0.8 builds with varyk 0.9, and the
build always uses the compiler's own `varyk-std`. It amends the milestone
5a spec (5.1, 5.2) and the milestone 5b2 spec (4.2, 4.3); the
`Cargo.toml` rule of `AGENTS.md` is unchanged. Varyk is experimental and
pre-1.0: anything here may change.

## 1. Summary and scope

`varyk`, `varyk-syntax`, and `varyk-std` keep one version, released
together. A program or a package says which `varyk-std` it was written
for, `varyk-std = "0.8"`, and that now reads "needs varyk 0.8 or later":

- a compiler of that version or a later one accepts it, and builds it
  with its own `varyk-std`;
- an earlier compiler refuses it (V0404, "this package needs varyk 0.8
  or later");
- `varyk publish` keeps today's rule and today's checks on top of that
  one, the compiler's own minor exactly, because the Rust it ships is built by plain cargo
  with the `varyk-std` its manifests and lock name.

### 1.1 Decisions

- **The `varyk-std` line is the minimum; no second field.** Gleam
  (`gleam = ">= 1.0.0"` in `gleam.toml`) and Elm (`elm-version`) keep a
  separate minimum-compiler field because their standard library is
  versioned apart from the compiler. Varyk's is not: with one linked
  version the line already names the varyk a package was written for, and
  a second field would be a second number that must agree with it.
- **One `varyk-std` per build, the compiler's.** Before 1.0, cargo builds
  `0.8` and `0.9` as two crates whose types do not mix, and an open
  requirement (`>=0.8`) makes cargo pick the newest release, which can be
  another minor again (tried with a stand-in crate: `>=0.5` beside `"0.6"`
  locked 0.6.4 and 0.10.1, two minors). A `[patch]` covers only its own
  minor and a lock cannot join two minors. What works is the manifest: the driver already writes every
  manifest of the build (M5b2 §4.3), so it writes the compiler's
  `varyk-std` into each one.
- **The Varyk code of a package always matches.** The user's compiler
  generates it from the package's `.vr` files, against its own
  `varyk-std`. Only a package's own `.rs` modules are written against an
  older `varyk-std`: if a later one renamed or removed something they call,
  rustc says so at that file (V0900, naming the package), which fails
  loudly. A change of behaviour under the same name is not caught; the
  weekly "Latest varyk" job of each official package, which builds it with
  the newest compiler, is the guard for those.

## 2. The rule

`check_std_dependency` (`crates/varyk/src/package.rs`) keeps every check
it has except the version's: `varyk-std` listed when the program uses it,
from crates.io, not optional, not renamed, a `version` present. The
requirement is still `X.Y` or `X.Y.Z`, optionally with `^`; `~`, `=`,
`>=`, `*`, and lists are refused as today. What changes is the comparison:

- `X.Y.Z` (`X.Y` read as `X.Y.0`) at or below the compiler's version:
  accepted.
- above it: V0404, "this package needs varyk X.Y or later" (or "this
  program …"; `X.Y.Z` when the line names a patch), with the note "this
  compiler is varyk A.B.C; `cargo install varyk` installs the newest" in
  place of the line to write.

The comparison is of version triples in order, so after 1.0 a package on
`0.9` is older than a compiler on `1.2` and builds as any older package
does. The patch counts because a package may need one: varyk-mongo names
`"0.8.1"` for what its `.rs` modules use of `varyk-std` 0.8.1, and a
compiler 0.8.0 that accepted it could build it with `varyk-std` 0.8.0,
from a lock that holds it.

`varyk publish` applies the new comparison and keeps today's checks,
one of them narrowed: the old comparison, equal `(major, minor)`, with its message
("`varyk-std` must be version A.B, the version of this compiler") for a
line on an older minor (a newer one gets the new comparison's),
`locked_older` (a lock older than the
compiler, now counting only a locked `varyk-std` on the compiler's
minor: one on an older minor is most often a package's, which the next
check, run only when this one passes, reports by its cause), and the
second half of `check_build_std` (the oldest
`varyk-std` cargo resolves for the build, from `Graph::std_version`, older
than the compiler). A published crate is built by plain cargo with the
`varyk-std` its lines and its lock name, without varyk to rewrite them: a
Varyk package of its build on an older minor brings a second, older
`varyk-std` whose types do not mix with this one, and a stale lock an
older `varyk-std` than its Rust was generated for. The note of the
resolved-version check names both causes and keeps `cargo update -p
varyk-std`, the help for the lock. `check_build_std` and
`check_std_dependency` take which rule they apply; `load` in `cli.rs`,
which `run_publish` goes through as the other commands do, passes it to
`check_build_std`, and only `run_publish` asks for publish's. A
dependency package's own check (`check_dependency` in `lib.rs`) always
applies the new comparison: under `varyk publish` one on an older minor
is caught by the resolved-version check, since cargo resolves its older
`varyk-std` for the build.

Every other command applies only the new comparison: `locked_older` and
the resolved-version check refuse exactly the builds this spec accepts, an
older lock or a package on an older minor, and the build no longer uses those
versions (section 3).

A dependency package that does not pass its check (`check_dependency` in
`lib.rs`) is most often one written for another varyk. Today its
`varyk-std` line is checked too and adds V0404 when the line is wrong,
on another minor included. Now a version is wrong only when it is newer
than the compiler; when its `(major, minor)` is older than the
compiler's, each of
the package's errors gets one more note: "`units` was written for varyk
0.8; a newer release of it may build with this compiler".

## 3. One varyk-std in every build

`with_local_std` (`crates/varyk/src/backend/mod.rs`) becomes
`with_build_std`: in a manifest that lists `varyk-std` under
`[dependencies]`, it replaces the entry with the compiler's own: the
directory `VARYK_STD_PATH` names when that is set, else `"A.B.C"`, the
compiler's version or a later release cargo counts compatible with it (a
later patch, before 1.0). A single file keeps
`"=A.B.C"` (M5a §5.2); a package build has a lock, and a later patch
already locked is what today's build uses too. Its two callers already
cover every Varyk crate of a build, `varyk test` included, the program's
(`crate_info` in `cli.rs`) and each package's (`package_crates` in
`driver/mod.rs`); a Varyk package under `[dev-dependencies]` is refused
(V0401). `run_publish` reaches it through `crate_info` but keeps only the
generated tree, and `assemble` writes the package's own manifest, so a
published manifest is never rewritten. A plain Rust crate of the build
that depends on `varyk-std` itself keeps its own line; that is cargo's
and rustc's, as any `.rs` code is.

The `varyk-std` entry is replaced whole, as `with_local_std` replaces it
today. `check_std_dependency` lets through every key but `path`, `git`,
`package`, and a true `optional`; `varyk-std` has no features, so
`features` and `default-features` say nothing, and a `registry` is
dropped, as `VARYK_STD_PATH` drops it today.

The package graph is still read from the manifests as written (M5b2
§4.1), and may name two `varyk-std`s; only `varyk publish` reads that.
The build runs cargo without `--locked`, and the graph's lock (the
program's own when no graph is read) is copied into the build on every
build (`build_dirs` in `cli.rs`), so when the
copy holds no `varyk-std` that `"A.B.C"` accepts (no line of the build
on the compiler's minor, or a lock on an older patch), cargo resolves one
again on each build, from the crates.io index, which needs the network
each time. Moving the program's line to the compiler's minor, adding one
if it has none, and running `cargo update -p varyk-std` in its folder
ends it; when its lock already holds two `varyk-std`s (a package on an
older minor), cargo calls that name ambiguous, and `cargo update -p
varyk-std@X.Y.Z`, naming the locked one on the compiler's minor, does it
instead. The program's own `Cargo.lock`, in
its folder, is never written; it keeps what the user's own `cargo` put
there.

## 4. What does not change

- `varyk init` writes `varyk-std` on the compiler's full version, as now,
  so a new package needs that varyk or later; `varyk add` runs `cargo
  add` and writes no `varyk-std` line.
- A single file's crate depends on the compiler's exact `varyk-std`, as
  now.
- `release-please` keeps the three crates linked.
- The official packages' code and manifests need no change. Their "Latest
  varyk" jobs move their `varyk-std` line to the newest minor before
  building, which is still a correct test; only the job's comment, "a new
  minor version of varyk needs a … release that moves `varyk-std` to it",
  becomes false (section 6). `varyk publish` wants the line on the minor
  of the varyk that runs it, and their CI and release jobs install varyk
  `^0.8`, so a package's own release raises its line only when it moves
  that pin, when it wants something new. A program that uses the package
  builds under any later varyk; a library that uses it is published only
  by a varyk on the package's minor (section 2), so a release that moves
  the line still follows each minor varyk, when convenient rather than
  at once.
- The duplicate-package refusal of `packages.rs` (V0401) compares Varyk
  packages of one name and version; two `varyk-std`s in the graph are
  neither Varyk packages nor of one version, so it never fires for them.

## 5. Diagnostics

No new code. V0404 keeps its meaning, a `varyk-std` line this compiler
cannot build with, and keeps three causes (an older minor, a stale lock,
an older resolved `varyk-std`) only under `varyk publish`. Its row in `docs/language.md`,
and its comment in `crates/varyk/src/diagnostics/codes.rs`, are
rewritten to say: missing, from a `path` or `git`, optional, renamed, not
`X.Y` or `X.Y.Z` (optionally `^`), or newer than the compiler; under
`varyk publish`, also any minor but the compiler's, a locked `varyk-std`
on the compiler's minor older than it, or an older `varyk-std` resolved
for the build.

## 6. Documents

- `docs/language.md`: the `varyk-std` paragraph of "Packages" (the rule,
  publish's exact rule, "To upgrade, run `cargo install varyk`", and that
  until the line is moved to the compiler's minor and the lock updated,
  each build asks the crates.io index for the compiler's `varyk-std`
  again, with the commands of section 3), the
  paragraph of "Packages and the build" that says a failing package's
  errors include V0404 and promises a resolved `varyk-std` no older than
  the compiler, and the V0404 row.
- `docs/specs/2026-10-02-milestone-5b2-design.md` and
  `docs/specs/2026-09-30-milestone-5a-design.md`: a line in each
  decisions log pointing here.
- `docs/design.md`: the "Versioning" row, which says the minor and the
  lock are checked by `varyk check`.
- `README.md` if it states the old rule; `docs/roadmap.md`: a line under
  the next milestone, done.
- `CONTRIBUTING.md`, if it tells authors to move their `varyk-std` line
  with every release; the "Versions" section of each official package's
  README, which says each minor varyk release is followed by a release of
  the package and pairs each release with one varyk; and the comment of each official package's `.github/workflows/latest-varyk.yml`
  (in that package's repository).

## 7. Testing and definition of done

Unit tests in `package.rs`, replacing today's minor tests: the compiler's
version, an older patch and a newer patch of its minor, an older minor,
an older major (`0.9` under `1.2`), a newer minor, a newer major, each
with and without `^`; the same under the publish rule, where only a line
on the compiler's minor and no newer than it passes; the stale-lock tests kept under
the publish rule and passing under the other, and a lock whose only older
`varyk-std` is on an older minor passing under both; the old refusals (`~`,
`>=`, a list, `path`, `git`, `optional`, `package`, no `version`) still
refused. The `check_build_std` tests of `packages.rs` stay, as the publish
rule, except that the stale-lock one, whose lock (0.4.0) is on an older
minor than its compiler (0.5.0) and so now falls to the resolved-version
check, moves to a lock on an older patch of the compiler's minor (a
compiler at 0.5.1, a lock at 0.5.0). A unit test in `backend/mod.rs` of the rewrite, given the value to
write rather than reading `VARYK_STD_PATH`, which a test cannot unset
safely: a `varyk-std` line, a string or a table, becomes `"A.B.C"`, and a manifest
without one is unchanged; the end-to-end test below sets the variable,
so it sees only the path.

Error fixtures, run by `varyk check`: `v0404_std_wrong_minor`, whose
`"0.1"` is an older minor and now passes, becomes
`v0404_std_newer_than_compiler`, on `"999.0"`: a release never reaches
it, before or after 1.0, and the text holds neither the compiler's
version nor its minor, which `without_version` in `tests/errors.rs`
replaces (`"0.999"` would read `[minor]99` under 0.9 and change at
0.10). The program lists nothing else, so no graph is read and cargo is
never asked for it; `v0404_std_stale_lock` removed with its case in
`tests/errors.rs` and its entry in `release-please-config.json`;
`v0404_failing_dependency_wrong_std` unchanged (its dependency's line is
a `path`, still refused, which keeps cargo off the network). Fixtures
that name the compiler's own version today keep doing so. The note of
section 2 is a unit test in `lib.rs`, beside the one that checks `trip`
through `Graph::for_tests` and `check_file`, so no cargo runs and no stub
`varyk-std` is needed: a `units` written to a scratch directory, with
`varyk-std = "0.1"` and a function that does not type-check, used by a
program; every diagnostic is V0200, none V0404, and each carries the note
beside the one naming the package.

An end-to-end test (`tests/packages.rs`): a program on the compiler's
minor uses a path package that uses `varyk-std` (it names `Error`), so
its line is checked, and whose `varyk-std` line is an older minor; it
checks, builds, runs, and the generated manifests of both crates name the
directory `VARYK_STD_PATH` names. It needs no release of `varyk-std`: the older minor is a
stub crate named `varyk-std`, version `0.1.0` so that no release bumps or
outgrows it, written by the test, patched in for the graph step by a
renamed entry under `[patch.crates-io]` in the program's
`.cargo/config.toml` (the manifest refuses `[patch]`, and
`with_std_patch` writes its own over it), beside the one `VARYK_STD_PATH`
already adds, so no `varyk-std` comes from crates.io (cargo still reads
the index under a patch and fetches `varyk-std`'s own dependencies, as
every test that builds it does, M5b2 §4.7). The build itself does not
use that patch; cargo's warning about it stays in the stderr the driver
keeps for a failure, so the output does not change. Two publish cases with
`varyk publish --assemble-only`: a package on an older minor is V0404
with publish's message, and the program of the first test is V0404 for
the older `varyk-std` its build resolves.

Done when the gate passes on stable and on the minimum Rust version, the
examples still build and print what they did, and the documents of
section 6 are current.

## 8. Release

A `feat:` commit. It shares the release that makes `varyk` 0.9.0 (with
the Rust 1.88 change), so 0.9.0 is the first compiler that builds packages
written for 0.8 unchanged; the release pull request waits for this.

## 9. Not in this version

Versioning `varyk-std` apart from the compiler; a minimum-varyk field
separate from the `varyk-std` line; a warning when a package is built
with a newer `varyk-std` than it was written for; checking at `varyk
check` time that a package's `.rs` modules still compile against the
compiler's `varyk-std` (that is rustc's job at build time, as for every
`.rs` module).

## 10. Decisions

- 2026-10-11: one version for the three crates stays; a `varyk-std` line
  on an older minor is accepted and built with the compiler's own
  `varyk-std`, written into every manifest of the build; a newer minor is
  V0404; `varyk publish` keeps the exact minor. Reviewed by Fable before
  the spec, which found the publish and resolved-version traps.
- 2026-10-11, review: the comparison is of full versions, so a line that
  names a patch (varyk-mongo's `"0.8.1"`) keeps meaning it; the build
  writes `"A.B.C"`, not `"=A.B.C"`, so a later patch already locked is
  kept as today; `varyk publish` keeps `locked_older` and the
  resolved-version check, since plain cargo builds what it ships with the
  versions its manifests and lock name.
