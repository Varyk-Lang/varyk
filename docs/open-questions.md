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
- Should `varyk_std::Value` grow `UInt`, `List`, and `Map`, so a trailing
  value may be a `u64`, a `Vec` (Postgres `= ANY($1)`), or a struct (a JSON
  column)? Milestone 5b3 keeps scalars so that no conversion can fail.
- Should a Varyk function be able to declare a literal-only or a variadic
  parameter, so a `.vr` wrapper could forward them?
- Should the facade's type parameter be allowed in parameter position
  (`&T: Serialize`), and in other return shapes (`HashMap<String, T>`)?
  Partly answered in milestone 5b4: yes in parameter position, as one `&T`
  bounded by `Serialize + ?Sized`; other return shapes are still open.
- Should `pub use` re-export a module, several names at once, or an item
  of another package? The last reopens V0115 (milestone 5b2).
- Should `varyk_std::Error` be accepted in parameters and fields, and
  should a facade be able to carry a status or kind on it? Partly answered
  in milestone 5b4: an `Error` carries an optional HTTP status, set by
  `Error::with_status` from Varyk (`varyk-http`'s constructors are Varyk
  over it); a bare `varyk_std::Error` return stays for a facade that
  makes an error of its own; parameters, fields, and a kind are still
  open.
- Should there be a wrapping hook, `app.wrap(f)` with `async fn f(req:
  http::Request, next: http::Next) -> http::Response`? `Next` would be a
  facade struct with one async method, so the cost is small; milestone 5b4
  keeps `before` and `after` until a need shows up that they cannot meet.
- Should routes be grouped as values (`let admin = app.group("/admin")`),
  or does `before_on` cover what groups are for?
- Should a handler be able to bind a header or a cookie by name, as a path
  parameter binds, without naming `http::Request`?
- Should a body bind by content type (XML, form data), with `xml::parse`
  and `xml::stringify` beside `json`? XML goes through `req.body()` and a
  facade until then.
- Should the HTTP client take per-request headers, which needs a request
  builder, or do default headers on `Client` cover a service's needs?
- Should the compiler write an OpenAPI document from the route table and
  the types it binds (`varyk openapi`)? It knows everything the document
  needs.
- Should routes be added to an app a function received, which needs the
  state type to travel with it (`http::App<State>` in the surface)?
- Should the compiler know any package but `varyk-std` and `varyk-http` by
  name, and should a community package be able to offer route binding?
  Function values, if they ever come, would answer the second.
- Should Varyk have a calendar `Date`, a time of day, or a `Duration`, and
  should `time::sleep` take a `Duration`? Milestone 5c has one time type,
  a point in time in UTC, and `add_seconds` and `seconds_since` in whole
  seconds.
- Should a `Time` carry or convert to a time zone, for a service that
  shows local times? Milestone 5c reads an offset and keeps only UTC.
- Should `varyk-std` build `time`, `uuid`, and `base64` only for a program
  that uses the types, through cargo features the driver sets? Milestone
  5c builds them for every program that uses `varyk-std`.
- Should `Uuid`s order, so a `Vec<Uuid>` of version 7 ids sorts by
  creation? Milestone 5c compares them with `==` and `!=` only.
- Should `Bytes` be indexed, sliced, and built from a `Vec<u8>`, and
  should there be a growable buffer? Milestone 5c's `Bytes` is made whole
  from text or base64, or handed over by a package.
- Should the language have a document literal, with values written inline,
  for MongoDB and JSON? What is its type, and does JSON share it? `varyk-mongo`
  builds its documents from structs until this is answered.
- `varyk-mongo` escapes a `$regex` value taken from input, so it matches as
  plain text anywhere in a field. Should there be a form for a prefix search
  from input (`^` then the escaped value), which an index can serve?
- Should `varyk-mongo` have a transaction call that retries? It needs a
  function as a facade argument, and Varyk has no function values.
