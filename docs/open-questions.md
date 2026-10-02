# Open questions

This began as section 9 of the [design spec](specs/2026-09-23-varyk-design.md),
copied as is. Later milestone specs add to it, and this file is the collected
list; milestones refer to [roadmap.md](roadmap.md).

Recorded, deliberately unanswered.

- Should local variables require `let mut`? Milestone 1 keeps it, following
  Rust.
- Should mutation be inferred from the body instead of declared on parameters?
- What syntax should explicit ownership transfer use for Varyk-declared
  functions? `move` is a candidate. Answered in milestone 5b1: none; the
  arguments of a started call are the one transfer, and parameters still
  borrow.
- How should owned versus borrowed return values be expressed and inferred?
  Answered in milestone 4: inferred from the body, never written; every
  return is new, or every return is part of one read-only parameter.
- How should an explicit string copy be spelled, once method calls exist?
  Answered in milestone 2: `s.clone()`.
- How should lifetime inference work across function boundaries?
  Answered in milestone 4: the result of a call with a borrowed return is
  another name for the argument in the rooted position, under the alias
  rule; one written lifetime in the generated Rust where elision would not
  pick that parameter.
- How should Rust traits, generics, and attributes map into Varyk while
  keeping Go-like simplicity?
- Should serde derivation be automatic for every struct, or declared with
  attribute syntax? Answered in milestone 5a: derived by the compiler only
  for the types a `json` or `env` call reaches; the attributes adjust
  names, defaults, and skipping, never whether to derive.
- Multi-threaded or current-thread async runtime, and how do `Send` and `Sync`
  failures surface to a writer who never sees those bounds? Answered in
  milestone 5b1: multi-threaded; Varyk types are always `Send` and `Sync`,
  and a `.rs` type that is not is V0901.
- Should Varyk infer which functions are async, as it infers borrows, and
  await every call implicitly, with concurrency spelled `go f(x)`? That is
  Go's surface on Rust's runtime; 5b1 keeps waiting explicit.
- Should tasks race, time out, or talk through channels, and in what shape?
- Should tasks change a shared value, and through what (`Mutex`, an actor, a
  channel)?
- Which Rust spellings, such as `&x` at a call site, should be accepted with a
  warning as a transition aid rather than rejected? Milestone 1 rejects them
  with a fix-it.
- What error-handling ergonomics beyond `Result` and `?` are worth adding?
  Partly answered in 5a: one built-in `Error` with a message. Whether it
  should carry a kind or a cause is open, below.
- Should Rust enums and generic types from `.rs` modules be imported
  automatically, as structs will be in milestone 3? Half-answered in
  milestone 3: enums are imported automatically; generic types remain open.
- Should `match` on an owned local move it, as in Rust, so that its parts
  can be taken out without a copy? Milestone 2 borrows every place it
  matches on, so an owned `Option<Item>` local can only be opened by
  matching on the call that produced it.
- Should Varyk code ever `use` a crate directly, once traits exist? Milestone 3
  reaches every crate through a facade `.rs` module in the package; that rule
  is the answer until then, and may remain it. An experiment, listed as
  unscheduled in the roadmap.
- Will generics ever be Varyk surface syntax? Milestone 3 assumes not and
  designs the facade around that.
- Should a Varyk library be importable from Varyk without a facade, given
  that its public API is already Varyk-shaped? Answered in milestone 5b2:
  yes, by its `Cargo.toml` key.
- The site's hero shows a service as milestone 5 is meant to write it, and
  that example assumes four things this spec has not decided: standard
  modules `http`, `db`, and `json` reachable without `use`; one shared
  `Error` type so `?` works across them; automatic serde derivation for
  structs; and expected-type inference for a query result. Each is settled
  by the milestone-5 spec, and the site copies the real example once it
  exists. Answered in part in milestone 5b2: `http` and `db` are packages a
  program adds, named by their keys; `json` stays in `varyk-std`.
- Should closures ever be values, with function types in the surface?
  Milestone 4 says no until the generics and traits question is answered,
  since no Varyk function could take one; closures exist only as arguments
  of built-in calls.
- Should serde derivation follow milestone 4's rule for `Clone` and
  `PartialEq`, automatic wherever the fields allow, or be declared? The
  serde question above, with milestone 4 as an input. Answered in 5a as
  above.
- Should `parse` return a `Result` once milestone 5 has a shared error
  type, so `?` works on it directly? Milestone 4 returned an `Option`.
  Answered in 5a: yes, `parse` returns `Result<T, Error>`.
- Should an `Option` of a borrowed value ever be a first-class value, so
  `get` and `find` could be stored and passed? Milestone 4 opens it only in
  the head of a `match`, `if let`, or `while let`, and copies a number or
  `bool` payload out instead.
- Does Varyk need a character type? `chars()` waits on it.
- Should the standard table of built-in calls move out of the compiler
  into a declaration file in Varyk's own signature vocabulary, so rows are
  added without a compiler change and a facade author can declare shapes
  the importer cannot infer? The milestone-4 spec (section 2.7) says why the
  table exists; where it lives is a later choice.
- Should JSON support enums with data, and in which shape? Milestone 5a
  refuses them and points to a `type` field written explicitly.
- Should `Error` carry a kind or a cause, and should other error types
  convert into it automatically at `?`? Milestone 5a has a message only,
  and no conversions.
- Should Varyk grow more attributes, and should they ever be namespaced?
  Milestone 5a's four are unprefixed because only the compiler defines
  attributes.
- Should a library derive serde for every `pub` type it declares, so its
  users can send its types through JSON? Milestone 5b2 refuses a `json` or
  `env` call on a type from another package (V0210); deriving would make
  every such library depend on `varyk-std`.
- Should code be able to hold a value of a package it does not depend on?
  Milestone 5b2 refuses it (V0115) because the generated Rust sometimes
  writes the type.
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
- Should `varyk publish` verify with its own build instead of cargo's, so
  publishing never compiles a dependency's shipped Rust?
