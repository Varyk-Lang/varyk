# Milestone 1 follow-ups

Items found during milestone 1's reviews that were deliberately left for
milestone 2. None affects the six example programs. Ordered by value.

## Do first in milestone 2

- Record the final per-local Rust representation in borrow analysis and have
  the backend read it, instead of the backend re-deriving it (`scan_block`,
  `scan_expr`, `let_repr` in `backend/rust_expr.rs`). Add `module` to
  `HirFunction` and keep functions in one `Vec` indexed by `FnId`, removing
  the three ways of finding a function by id. Borrowed returns will multiply
  representation decisions, so do this before them.
- Alias bindings: a `let` made from a parameter or a field is another name
  for the same value. `varyk check` accepts programs that use the original
  and the alias together, and rustc then rejects them (E0502, E0505). Add an
  invalidation check reusing the move-checker state.
- A field of a call result as a branch of a string `if`
  (`if c { mk().name } else { "x" }`) gets past the mixed-text rule,
  because the backend treats every field access as a place.

## Programs that pass `check` and fail in rustc (each rare)

- Constant overflow (`255u8 + 1`, `let y: i8 = -128 - 1`) and `1 / 0` are
  rustc deny-by-default lints and pass check; catching them needs constant
  folding.
- V0306 (a later argument changing or giving away a value an earlier
  argument borrows) compares argument roots, so `f(p.a, p.b)` with one
  `mut` parameter is rejected although Rust accepts two different fields,
  and it counts a `let`, an assignment, or a struct field inside a block
  argument as giving the value away even where Rust would only borrow it.
- A local named `None`, `Some`, `Ok`, or `Err`, or a struct named `String`
  or after a primitive (`struct u8`), passes check and rustc rejects it,
  because the generated code then shadows Rust's own names. Reserve them.

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
- V0304 and V0305 do not say what to change, since milestone 1 has no
  explicit copy spelling to suggest.
- A long message is repeated as the caret label; consider a shorter label
  text.
- V0010 and V0012 keep the spec's "infers references" and "lifetimes are
  inferred" wording, which section 2 of the spec would rather avoid.
- `-> &str`, `&T` in a field type, and `&T` in a `let` annotation get a
  generic V0002 instead of a Rust-habit diagnostic.

## Documentation

- A literal mixed with a call result in an `if` passed to a `string`
  parameter allocates (`.to_string()` on the literal); add to the allocation
  list.

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
