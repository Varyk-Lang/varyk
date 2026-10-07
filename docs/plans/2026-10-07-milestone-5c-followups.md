# Milestone 5c follow-ups

Items found during the task and branch reviews of milestone 5c (`Time`,
`Uuid`, and `Bytes` in `varyk-std` and the compiler) that were deliberately
left for later. None affects the example programs. Items the branch fixed
are not listed.

## Wording

- A method call on a `Uuid` says "type `Uuid` has no methods" without pointing to `Uuid::new()`, `v7()`, and `v4()`.
- `t + 3600` on a `Time` is V0200 "expected `Time`, found `i64`", with no hint to use `add_seconds`.
- A `Bytes` query parameter gets V0219's generic "gets no value from the request" message; a `Bytes` path parameter gets the specific one.
- V0210's note for `Bytes` in `env` lists what `env` reads but does not suggest a `string` field and `Bytes::from_base64`.
- V0108's note for a borrowed `Time` or `Uuid` parameter always shows `t: varyk_std::Uuid` as the example.
- A type brought in by a `use` line in a `.rs` file is matched to `varyk_std::Time`, `Uuid`, or `Bytes` by its bare name for the note's suggestion.
- A few lines in `README.md`, `docs/design.md`, and `docs/language.md` run past the files' usual width.

## Code

- `types/check/methods.rs` builds the list of a built-in owner's names on every built-in method call only to detect `Uuid`; a filter on the owner would do.
- The full path of the three types is spelled in two places (`interop/items.rs`, `resolve/signatures.rs`).
- Each crossing from `bytes::Bytes` to `varyk_std::Bytes` wraps the handle in one more `from_owner` owner: one small allocation, never a copy of the bytes (spec 5 accepts it).

## Tests

- The clone tests of `Bytes` prove the buffer is shared, not that nothing is allocated.
- The resolve tests reach `std_type_taken` directly for a `.rs` struct named `Time` or `Uuid`; only a `.rs` enum named `Bytes` goes through the import.
- No runtime round trip of a `None` `Option<Time>` through JSON or `env`.
- No test of `&Vec<varyk_std::Time>` or `&Option<varyk_std::Bytes>` in a facade.
- The closure branch of the borrowed-return note is not tested for `Bytes`, and the soundness templates do not exercise `first` and `last` on a `Vec<Bytes>`.

## Behaviour left as the spec allows

- A facade cannot return a borrowed `Time`, `Uuid`, or `Bytes`, or take `&mut varyk_std::Time` or `&mut varyk_std::Uuid` (V0108): spec 2.5 lists only these forms.
- An `if` giving `Some(b)` or `None` as a trailing value gives `b` away, as `Some(name)` does for a string (V0305 if used after, V0304 if borrowed); a borrowed `Option<Bytes>` parameter is lent through `.as_ref()`.
- `Bytes::from_base64` and `to_text` say what is wrong without repeating the input, which can be as large as a body (spec 3).
- `README.md` names `varyk-sql` and `varyk-http` as working with 0.8; until their 0.8 releases, V0404 refuses `varyk-sql` 0.2 and `varyk-http` 0.1 under varyk 0.8 (spec 6.3).
