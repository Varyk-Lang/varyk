# Milestone 4 follow-ups

Items found during the reviews of milestone 4 that were deliberately left
for later. None affects the example programs.

## Over-strict

Each of these is rejected though rustc would accept the Rust it stands for.

- `match res::open() { Some(h) => keep(h), _ => {} }` with an enum that runs code when it is thrown away (an `impl Drop`) inside an `Option` temporary is V0304.
- `n.insert(n.len(), n[0])` on a `Vec<i32>` is V0306, though two-phase borrows accept it.
- A field of a borrowed-return call on a stored value as an argument (`trim(u.first().name)`, `reg::word(r.head().key)`) is V0001 "made right here", which rustc accepts and whose wording is wrong. A field of a borrowed call as a `match` head (`match obj.info().kind`) stays V0001 by design.
- `any` and `all` over owned items treat the closure parameter as look-only (V0304 on `o.ok_or(3)`), though Rust passes the item by value.
- `Some(t) == o` with a borrowed `t` is V0304, though a comparison needs no ownership.
- `3000000000 as i64` is V0200, because the literal is typed `i32` before the cast.
- `o.map(|n| .. o ..)` on a stored `Option<i32>` is V0305, though rustc accepts it (the `Option` is Copy).
- `let mut out = r::refnum(v[0]); for x in v { out = r::refnum(x); }` is V0301 "given by a call, and cannot be changed", while `let mut best = "none"; best = nm(u)` is accepted.
- `let m = u.name.trim(); u.nick = "y";` followed by a use of `m` is V0307, though rustc accepts it (the borrow is only of `u.name`).
- `r::refnum(3)` with a literal argument is V0001 "made right here", though rustc promotes `&3` to a static.
- `let q = o;` for a stored `Option<i32>` inside a loop is V0305, though the `Option` is Copy in Rust (the spec copies only numbers and `bool`).
- `{ let own = "y"; out = own.trim(); }` is V0304 "only exists inside this block", though the text is a literal.
- `r::first_word(match s.len() { 3 => s, _ => "q" })` is V0001 "made right here", though one arm is a stored place.

## Wording

- `for v in m.values() { m.insert("z", v.clone()); }` blames "`v` is another name for part of `m`" instead of giving the loop message.
- A changed `map` capture in a `for` head says "goes over part of `other`" instead of "head reads `other`"; capture changes reuse the label "the loop goes over this until it ends".
- `Rootless` returns and rootless `filter`/`find` parameters reuse "belongs to someone else".
- V0203's headline says "copied" where the spec says "cloned".
- In a recursive group the recursive call is labelled "this return is new" and reads as a mixed V0304; the "calls itself" message in `returns.rs` is never reached, because mixed returns are checked before recursion.
- V0001 for a field of a call's result as a head (`v0001_field_of_call_as_head`, `patterns.rs`) says "a part of a value that is not stored anywhere", though the value may be stored; say "store the call's result with `let` first".
- Two V0206 headlines show `Option<_>`, which is Rust vocabulary; say "an `Option`".
- V0304 on a loop result says "this value is kept inside `n`" where it is part of `n`.

## Diagnostics

- Within one function, the return diagnostic comes after the function's other diagnostics (the order across functions is right).
- One mistake can give V0301 and V0307 together (`names.iter().map(|x| { names.push("1"); x }).count()`).
- `Some(text.parse()?)` is V0207 rather than V0206.
- `1 as Vec<i32>` gives "comparisons cannot be chained": the cast's type is read as a bare name and `<` as a comparison.
- `docs/language.md` promises an error naming what is not supported, but a tuple `(1, 2)`, struct shorthand `P { a }`, and `let Some(x) = o else {..}` give plain V0002.
- `|x| return true` is V0002 "expected an expression"; only the block form `|x| { return true; }` gives the documented V0001.
- The V0208 span for `match Some(v.iter())` covers the whole head, not the chain.
- `let w = v.iter();` reports V0208 only after an unrelated V0200 in the same function is fixed.
- `loop` says "not supported in Varyk yet", though `docs/language.md` lists it as left out by design.
- `HashSet<i32>` and `Box<i32>` give "takes no type arguments" rather than naming what is unsupported.
- V0304 on storing a borrowed `Option<string>` parameter into a field has no "insert `.clone()`" help, though the same error on a `string` parameter does.
- Inside a closure, `let mut q = p; q = "z";` with `p` a borrowed item names `p` rather than `q` as what cannot change, and suggests changing the source, which a closure cannot.
- V0200 on `for id in 1..=3` passed to a `u32` parameter could suggest a typed bound (`let last: u32 = 3;`) or `id as u32`.

## Code structure

- `returns.rs` has two diagnostic tables (`ClosureRoots::diagnostic` and `Report::diagnostic`) that duplicate helpers.
- The backend picks special forms by method-name strings.
- `is_place_or_rooted` repeats `is_place`.
- `chains.rs::source_receiver` and `head_reads` both walk to the source.
- The `Str|Ref` root filter is derived in three places.
- `Owner::of` repeats `Subst` updates.

## Tests

- No error fixture where an imported rooted call triggers V0307 or V0304.
- The one-root versus several-roots choice of a chain `for` variable's `Bound.root` is not tested.
- Two `derives.rs` test helpers fail with unclear panics.
- The fix for Copy `for` variables has no committed test for the `let`-then-return form or the loop over a borrowed-return call.
- `reference.rs` checks only that each code appears in `docs/language.md`, not that it has a `| Vxxxx |` row.
- Three `packages.rs` tests that run plain `cargo build` in a temporary package fail when `CARGO_TARGET_DIR` is exported, because they expect the output under the package's own `target/`.

## Performance

- A literal compared inside a closure (`filter(|x| x.nick == Some("n"))`) allocates per item under the literal-into-owned-slot rule.
