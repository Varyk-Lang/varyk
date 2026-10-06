# Milestone 5b4 follow-ups

Items found during the task and branch reviews of milestone 5b4 (the
`varyk-http` route table, route and hook calls, `varyk add http sql`, and a
`&T: Serialize` parameter of a facade) that were deliberately left for
later. None affects the example programs. Items the branch fixed are not
listed.

## Wording

- V0214 for a handler or hook that gets back to the async function adding it says "calls" where the function adds a route.
- V0407 gives its help as note lines rather than a help line.
- A hook adapter that finds no state parameter skips it silently, which cannot happen after the check; a `compile_error!` would match fail-loudly.

## Source map

- The mark of a route call or `App::new` covers the whole compound statement around it: inside an `if` or a `match`, every line of the statement points at the route. The text is right and the line is coarse.
- `SourceMap::mark` repeats the line lookup of its neighbours in `backend/mod.rs`.
- The response adapter in `rust.rs` has `Nothing` and nested `Result` arms that cannot be reached.

## Tests

- The stub router's behavior (hook order, `before_on` on whole segments, a conflict, the fixed 500 body, 404 and 405) is covered only through `http_user`, not by tests of its own.
- Not tested: a wrong argument count to a route call, a body on `delete`, a handler returning `Option of Response` or `Result` of another error type, a route or hook in a `while let` value, an `after` hook with an unrelated third parameter, `varyk-http` under another key, two versions of `varyk-http` in one build.
- No test pins `-> &varyk_std::Error` or a method that returns a bare `varyk_std::Error` beyond their fixtures.
- The 5b4 snapshots are reviewed as a set; the cases are not itemised one to one against the spec's list.
- `varyk add` with several official names stops at the first `cargo add` that fails; that is not tested.

## Documents

- The HTTP examples in `docs/language.md` are not machine-checked: only whole programs are checked alone, and the examples there are fragments.
- `README.md` names no release number for `varyk-http` and the HTTP support; add 0.7.0 once it ships, in the pass after the release.

## Behavior left as the spec allows

- In a build of `varyk-http` itself, every struct its root declares counts as a type the package binds (rule 3), so a root struct `bind` does not accept fails at rustc (V0900); the package's design keeps its root free of other structs; a later fix could limit the set to structs imported from `.rs` modules.

- `varyk add sql serde http` passes `http` to cargo as a plain argument after `sql` (spec 2.9 recognizes official names only at the start), so cargo reads it as a crate name, not the shorthand.
- A route call in the head block of a `for` is accepted and runs once.
