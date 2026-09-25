# Milestone 1 and 2 follow-ups

Items found during the reviews of milestones 1 and 2 that were deliberately
left for later. None affects the example programs. Ordered by value within
each section. Milestone 2 absorbed the representation refactor, the alias
rule, the reserved names, and the `.clone()` fix-it for V0304.

## Programs that pass `check` and fail in rustc (each rare)

- Constant overflow (`255u8 + 1`, `let y: i8 = -128 - 1`) and `1 / 0` are
  rustc deny-by-default lints and pass check; catching them needs constant
  folding.
- V0306 (a later argument changing or giving away a value an earlier
  argument borrows) compares argument roots, so `f(p.a, p.b)` with one
  `mut` parameter is rejected although Rust accepts two different fields,
  and it counts a `let`, an assignment, or a struct field inside a block
  argument as giving the value away even where Rust would only borrow it.
- A copied `.rs` module is compiled as edition 2024, so a file that uses a
  2024 keyword such as `gen` as a name fails in rustc; `r#match` in a `.rs`
  module is reported as an unsupported attribute, which misleads.

## Diagnostics wording

- `/* */`, `1e5`, `1u8`, and `as` get generic messages.
- When a mixed-text `if` is passed to a `mut string` parameter, the
  "not changeable" check is skipped for the whole argument, so a three-way
  `if` with two read-only parameters reports one error instead of two. The
  program is still rejected; the second error appears once the first is
  fixed. Suppress per leaf if this ever bites.
- `x: &'a T` gives V0011 and V0012 with overlapping fix-its; a JSON consumer
  applying both corrupts the text.
- JSON `column` counts bytes while the human `-->` line counts characters,
  so they disagree on non-ASCII lines.
- `let _ = s;` counts as a move of `s`, though Rust does not move there
  (over-strict, not unsound). `pub mod m;` is accepted and `pub` is ignored.
- V0105's "declared here without `pub`" label sits on the `fn` keyword but
  on the struct's name for structs; pick one. V0106 for a missing `main`
  uses a zero-width span at 1:1, which reads as blaming whatever is there.
- `--message-format=json` is tested end to end only for `check`, not for
  `build` or `run`.
- A discarded mixed `if` statement (`if c { s } else { mk_s() };`) is
  rejected with V0304 and a "store it with `let`" hint; harsher than needed.
- `let fn = 1;` gives V0002, not the "Rust keyword" V0001, because milestone
  keywords are separate tokens from reserved words.
- V0305 does not say what to change; for text it could suggest
  `.clone()`, as V0304 does.
- A long message is repeated as the caret label; consider a shorter label
  text.
- V0010 and V0012 keep the spec's "infers references" and "lifetimes are
  inferred" wording, which section 2 of the spec would rather avoid.
- `-> &str`, `&T` in a field type, and `&T` in a `let` annotation get a
  generic V0002 instead of a Rust-habit diagnostic.

## Small code items

- `used_while_lent` and `overlapping_borrows` in `borrow.rs` are two passes
  over the same call arguments for V0306 and could be one.

- `varyk-syntax`: `line_col` returns a bare tuple; `parse_call_args` and
  the macro argument loop duplicate a loop; `Program` and `Item` carry no
  span of their own.
- Resolver: the module-file stop rule is untested; pass 2 relies on pass-1
  push order; duplicate imported names are dropped silently.
- Type checker: implementer-added rules (i64 propagation to both operands,
  duplicate literal field, unary minus on unsigned, lone braces) are
  untested; an `if` branch literal falls back to `i32` before the `else`
  branch is seen.
- Borrow analysis: fields of temporaries are over-strict; nested loops cost
  2^depth walks; aliasing a mixed `let` can yield two V0304s.
- Driver: cargo's stdout is dropped on failure.
- `RustTy` in `interop.rs` and `map_param`/`map_return`/`map_primitive` in
  `resolve.rs` are two passes over one table; `interop` could return the
  `Ty` and `ParamMode` directly, saving about 90 lines and one enum, at the
  cost of rewriting the interop unit tests.
- No automated test proves `run -- args` forwarding or exit-code forwarding,
  since no milestone-1 program reads arguments or exits non-zero.

## Found in milestone 2

Programs `check` handles wrongly or harshly:

- `f()?;` where `f` returns a `Result` whose value is itself a `Result`
  drops the inner error with only rustc's `unused_must_use` warning, which
  the generated crate silences.
- `Ok(x)?` written directly is V0207, since the operand of `?` has no
  expected type.
- Over-strict, never unsound: a `let mut` once assigned a borrowed place
  keeps that root for the whole function; a `for` over a place holds its
  root, so writing a different field inside a `mut self` method is V0307;
  a `match` value mixing an owned string binding with another new value is
  V0304; an owned binding from a matched temporary passed to a `mut`
  parameter through a `match` value is V0303.
- A `pub fn` in an `impl` of a non-`pub` struct whose signature names the
  struct is V0105 ("used by a public item but is not public"), though rustc
  accepts it.

Diagnostics wording:

- V0204 names a module enum's variant without its module path.
- V0303 and V0307 can both fire on one statement.
- V0207's suggested shapes hard-code the binding names `x`, `v`, and `r`.
- The bare-`return` arm-body V0001 carries its fix as message text, not a
  structured fix-it a JSON consumer can apply.
- `mut self: T` is reported as `self: T`; tuple and reference patterns are
  called literal patterns; `0..10..2` in a `for` head gets a generic V0002;
  the `while let` message repeats itself.
- A struct path used as a value gets the "associated functions" message.

Tests:

- No soundness case loops over a `&Vec` or `&mut Vec` head of Copy
  elements; the `match` and `for` copy wording is not pinned by a test.
- No span assertions for `MatchArm`, `Pattern`, `VariantPattern`, or the
  `format!` intrinsic; several rejection tests check only the code.
- The cargo-invoking example tests may contend on the shared cache; one
  unrepeated `run_hello` failure was seen.
- `docs/language.md` does not say `usize` is 64 bits on the supported
  targets.

Code:

- `parser/item.rs`, `parser/expr.rs`, `resolve.rs`, and `types/check.rs`
  are large; match parsing could move to `parser/pattern.rs`, `for` typing
  to `types/check/loops.rs`, and the resolver's tests and cycle check to
  their own files.
- The backend re-derives the parameter mode (`repr()` in `rust_expr.rs`)
  and `MethodRef` modes instead of reading them; "is a pattern binding" has
  two representations (a borrow-analysis side table and `strings.rs`).
- `skip_brace_group` duplicates `skip_generic_args`; built-in variant names
  are mapped to strings in three places; V0203's struct and enum branches
  could merge on `is_compound`; `reserved_type_name` is `pub(crate)` with no outside
  caller; `Origin::Local` is recorded but the alias rule reads roots from
  `refers`.
- `typecheck` relies on diagnostics being non-empty to keep `FnId` indexes
  aligned; a `debug_assert` would make a mismatch loud.
