# Open questions

This is section 9 of the [design spec](specs/2026-09-23-varyk-design.md),
copied as is; milestones refer to [roadmap.md](roadmap.md).

Recorded, deliberately unanswered.

- Should local variables require `let mut`? Milestone 1 keeps it, following
  Rust.
- Should mutation be inferred from the body instead of declared on parameters?
- What syntax should explicit ownership transfer use for Varyk-declared
  functions? `move` is a candidate. The async runtime decision depends on it.
- How should owned versus borrowed return values be expressed and inferred?
- How should an explicit string copy be spelled, once method calls exist?
- How should lifetime inference work across function boundaries?
- How should Rust traits, generics, and attributes map into Varyk while
  keeping Go-like simplicity?
- Should serde derivation be automatic for every struct, or declared with
  attribute syntax? Milestone 3 is gated on this.
- Multi-threaded or current-thread async runtime, and how do `Send` and `Sync`
  failures surface to a writer who never sees those bounds?
- Which Rust spellings, such as `&x` at a call site, should be accepted with a
  warning as a transition aid rather than rejected? Milestone 1 rejects them
  with a fix-it.
- What error-handling ergonomics beyond `Result` and `?` are worth adding?
- Should Rust enums and generic types from `.rs` modules be imported
  automatically, as structs will be in milestone 2?
