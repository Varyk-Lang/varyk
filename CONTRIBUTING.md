# Contributing

Varyk is experimental and pre-1.0: anything may change, and a breaking
change bumps the minor version. Contributions are welcome; small, focused
pull requests are the easiest to review. Varyk has one maintainer, so
reviews are best effort and a pull request may wait a while; a reminder
after two weeks is welcome.

## Build and test

You need a stable Rust toolchain through [rustup](https://rustup.rs).
Before opening a pull request, run the same checks CI runs:

```sh
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

The minimum supported Rust version is 1.85 and CI checks it.

See also `AGENTS.md`, the short list of rules that keep the code coherent,
written for people and AI agents alike.

## Where things are

- `docs/specs/` holds the design the compiler is built from; read it before
  changing semantics. Decisions that are settled are in its decisions log.
- `docs/language.md` is the one-page reference and must change in the same
  pull request as any change to what the compiler accepts. A whole
  program in it is a set of code blocks each starting with a `// <file>`
  line, ending with `// main.vr`; `crates/varyk/tests/cli.rs` checks each.
- `docs/roadmap.md` says what comes next; `docs/plans/` holds the
  implementation plans and the follow-ups left from each milestone.
- Diagnostics have stable codes (`crates/varyk/src/diagnostics/codes.rs`).
  A code is never reused for a different meaning. A new diagnostic needs a
  fixture under `crates/varyk/tests/fixtures/errors/<code>_<slug>/`, a
  registration in `crates/varyk/tests/errors.rs`, and a line in the code
  list at the end of `docs/language.md`.

## Commit messages and releases

Releases are cut by [release-please](https://github.com/googleapis/release-please)
from the commit history on `main`, so every commit that lands on `main`
carries a conventional prefix. A pull request with one concern is
squash-merged with a conventional title; a pull request with several
concerns is rebase-merged with one conventional commit per concern.

| Prefix | Use it for | Effect on the release |
|---|---|---|
| `feat:` | a new capability | bumps the version, listed in the changelog |
| `fix:` | a bug fix | bumps the version, listed in the changelog |
| `docs:` | documentation only | no bump, not listed |
| `test:` | tests only | no bump, not listed |
| `ci:` | workflows and release automation | no bump, not listed |
| `chore:` | maintenance, dependencies, manifests | no bump, not listed |
| `refactor:` | code change with no behavior change | no bump, not listed |

A `!` after the prefix (`feat!:`) marks a breaking change. Before 1.0,
`feat` and `fix` bump the patch version and a breaking change bumps the
minor version.

Keep `<` and `>` out of commit subjects and `BREAKING CHANGE:` footers:
write `Option of T`, not `Option<T>`. release-please copies them into the
release pull request, reads that body as HTML, and an unclosed `<T>` hides
the sections after it, so their crates are published but never tagged, and
the next release pull request proposes a version that has already shipped.
The `Commit messages` check enforces this on every pull request, and the
release workflow's `tags` job fails if a released version has no tag; such
a tag is then created by hand at its release commit, the one exception to
release-please making every tag.

release-please opens a release pull request on `main` with the version
bumps and `CHANGELOG.md`. Merging that pull request is the release decision:
it creates the tags `varyk-vX.Y.Z`, `varyk-syntax-vX.Y.Z`, and
`varyk-std-vX.Y.Z` and the GitHub releases, and the publish job then uploads
the three crates to crates.io with no further approval. The crates.io token is a secret of the `release`
environment, which only workflow runs from protected branches (`main`) may
use; the environment has no required reviewer, so nothing waits on a click
after the merge.

## Keeping it small

Varyk is small on purpose. Prefer the smallest change that fixes the
problem, and open an issue first for anything that adds a language feature,
so it can be weighed against the spec's principles.

## Licensing of contributions

Unless you explicitly state otherwise, any contribution intentionally
submitted for inclusion in Varyk by you, as defined in the Apache-2.0
license, shall be dual licensed under MIT or Apache-2.0, without any
additional terms or conditions. The name "Varyk" is a trademark and is not
covered by those licenses; see `TRADEMARKS.md`.

## Reporting bugs

Open an issue with the smallest `.vr` program that shows the problem and
the output of `varyk check` or `varyk build --emit-rust`. For security
problems, see the security policy in the repository's Security tab, shared by every Varyk-Lang repository.
