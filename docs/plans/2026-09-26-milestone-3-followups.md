# Milestone 3 follow-ups

Items found during the reviews of milestone 3 that were deliberately left
for later, each checked still open when milestone 3 closed. None affects
the example programs.

## Language and diagnostics

- `pub(crate) use a::B;` gives two V0001s, one for `pub(crate)` and one for `pub use`.
- `super::super::x` is not syntax (one leading keyword only), though Rust allows it.
- A type needing both `pub` and `pub mod` reports the struct first and the module only on the next run.
- A path into an inline `mod` of a `.rs` file gets the "Varyk cannot look into it" note only in a call (`h::inner::f()`); `use h::inner::f;` and a type `h::inner::S` get a bare V0100/V0101.
- A glob `use dep::*;` bringing in a module named `std` or `core` does not stop Varyk trusting a `std::name!` macro path in that `.rs` file; harmless in practice, since rustc rejects such a file anyway (E0659, `std` is ambiguous) before Varyk's check could matter.

## Interop limits

- `use`, `extern crate`, and `mod x;` inside function bodies are not checked by the importer; rustc reports them at the user's `.rs` line.
- A `use` whose first segment is a prelude or primitive name (`use Option::Some as S;`, `use u8 as Byte;`) is rejected as an unknown crate; write the full path.
- A module brought in by a glob `use` or defined by a macro is rejected as an unknown crate when used as a `use` root; write its full path.

- The file-include rule is blunt: `include!`/`include_str!`/`include_bytes!` anywhere in a `.rs` file is V0104, inside a never-invoked `macro_rules!` body too, and a local macro that happens to be named `include` is not told apart from the builtin.

- A dependency's derive or attribute (proc) macro can write an `impl Drop` or define a type name that Varyk does not see; rustc then rejects the build.
- Methods of Rust enums are not imported (V0100 says so).
- `Result<(), String>`, `Option<()>`, and `Vec<()>` cannot be mapped; `()` works only as a bare return type.
- A struct whose last field is a struct with an unsized last field is imported; rustc rejects it in most shapes anyway.
- A function called through a `use ... as` alias is named by its original name in borrow diagnostics.
- `uncallable_note` tells a method's receiver by `receiver.contains("self")`.

## Driver and CLI

- Cargo target tables (`[lib]`, `[[bin]]`, ...) and the `auto*`/`default-run` keys are refused outright; a `[lib] crate-type = ["cdylib"]` or a renamed library target would need them.

- `src/bin/`, `examples/`, `tests/`, and `benches/` in a package are V0401; `varyk test` (milestone 5) is where Rust integration tests would come back.

- `[package] workspace = "path"` is checked to name a directory with a `[workspace]` manifest, not that the root lists this package as a member; cargo reports that at the package root.

- A hand-edited `build.rs` (beyond the include stub `varyk init` writes) is not detected; plain cargo runs it, the crates Varyk builds do not.

- `varyk publish` skips a `README*`/`LICENSE*` at the root that is a symbolic link (only files named by `readme`/`license-file` may be links, and only to files inside the package).
- rustc paths on Windows (`src\\util.rs`) are not matched by the source map or the copied-path rewrite; CI is Linux-only.
- Inside a workspace, cargo reads `[profile.*]` from the root; the crates Varyk builds read the package's own.

- `[lints]` is V0401: applying it only to the user's `.rs` modules would need lint attributes on the generated `mod` lines (and the `varyk init` stub path still applies the manifest's lints crate-wide). A custom `build` script is V0401 too; the hidden and assembled crates run none.

- `[patch]` is V0401, in the package and in an enclosing workspace's root manifest. Carrying it into the hidden crate needs cargo's workspace-membership rules (`members` globs, `exclude`, transitive `path` dependencies, `package.workspace`) so the effective table is the one cargo would use; a first cut with the `glob` crate was tried and rolled back as too much for the MVP.

- A stale `Cargo.lock` stays in the hidden crate after the user deletes theirs.
- The reserved package names `cache` and `package` are compared case-sensitively; `name = "Cache"` shares cargo's target directory on a case-insensitive file system.
- Run from a subdirectory of a package, diagnostics show absolute paths.
- Package fixture directories with a `Cargo.toml` under `crates/varyk/tests/fixtures/` are silently left out of `cargo package`.
- `push_emit` reads a copied `.rs` with `unwrap_or_default`, so an IO error empties its `--emit-rust` section.
- `rewrite_copied_paths` rewrites paths in rustc's text by unanchored substring replace.
- The published manifest is re-sorted and loses its comments.
- A readme both named in the manifest and matched at the root is copied twice (harmless).
- A run-time panic in a copied `.rs` names the generated path; a non-snake-case `.rs` module name warns at a generated path.
- Non-error rustc messages in generated files are dropped, an internal compiler error included.

## Code structure

- Four module-path walkers do the same walk; `private_in_public` and `hidden_type` duplicate each other.
- `write_if_changed` and `copy_if_changed` duplicate the compare, temporary file, and rename logic.
- Long functions: `collect_symbols` (~330 lines), `package::load` (~215), `import_items` (~135); `resolve/mod.rs` is 1600 lines, `cli.rs` 765.
- A `TempDir` test helper is copied in `package.rs`, `generate.rs`, `init.rs`, and `publish.rs`.
- `-` to `_` crate-name normalisation is spelled in three places; the root file list (`src/main.vr`, `src/lib.vr`) in three.
- Binary versus library is decided two ways (the driver by path, elsewhere by `Kind`); exit-code forwarding is repeated.
- `HirModuleKind::Rust.source` is never read; `emit_rust` reads the file again.
- `use_target_path` is used only by tests.

## Tests

- No test that `remove_stale` removes a directory it leaves empty.
- No unit test for a mixed-case package name.
- The soundness harness detects a crash by the text "panicked" and reads cargo's JSON by substring.
- The language reference test only runs `check` on its programs; it never compiles their `.rs` blocks with rustc.
- `span-locations` makes `proc-macro2`, `quote`, and `syn` compile twice in a clean build.
