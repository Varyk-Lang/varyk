# Open questions

This began as section 9 of the [design spec](specs/2026-09-23-varyk-design.md),
copied as is. Later milestone specs add to it, and this file is the collected
list; milestones refer to [roadmap.md](roadmap.md).

Recorded, deliberately unanswered.

- Should local variables require `let mut`? Milestone 1 keeps it, following
  Rust.
- Should mutation be inferred from the body instead of declared on parameters?
- What syntax should explicit ownership transfer use for Varyk-declared
  functions? `move` is a candidate. The async runtime decision depends on it.
- How should owned versus borrowed return values be expressed and inferred?
- How should an explicit string copy be spelled, once method calls exist?
  Answered in milestone 2: `s.clone()`.
- How should lifetime inference work across function boundaries?
- How should Rust traits, generics, and attributes map into Varyk while
  keeping Go-like simplicity?
- Should serde derivation be automatic for every struct, or declared with
  attribute syntax? Milestone 5 (batteries) is gated on this.
- Multi-threaded or current-thread async runtime, and how do `Send` and `Sync`
  failures surface to a writer who never sees those bounds?
- Which Rust spellings, such as `&x` at a call site, should be accepted with a
  warning as a transition aid rather than rejected? Milestone 1 rejects them
  with a fix-it.
- What error-handling ergonomics beyond `Result` and `?` are worth adding?
- Should Rust enums and generic types from `.rs` modules be imported
  automatically, as structs will be in milestone 3?
- Should `match` on an owned local move it, as in Rust, so that its parts
  can be taken out without a copy? Milestone 2 borrows every place it
  matches on, so an owned `Option<Task>` local can only be opened by
  matching on the call that produced it.
- Should Varyk code ever `use` a crate directly, once traits exist? Milestone 3
  plans to reach every crate through a facade `.rs` module in the package;
  that rule is the answer until then, and may remain it. An experiment, listed as unscheduled in the roadmap.
- Will generics ever be Varyk surface syntax? Milestone 3 assumes not and
  designs the facade around that.
- Should a Varyk library be importable from Varyk without a facade, given
  that its public API is already Varyk-shaped?
- The site's hero shows a service as milestone 5 is meant to write it, and
  that example assumes four things no spec has decided yet: standard
  modules `http`, `db`, and `json` reachable without `use`; one shared
  `Error` type so `?` works across them; automatic serde derivation for
  structs; and expected-type inference for a query result. Each is for the
  milestone-5 design to settle, and the site copies the real example once it
  exists.
