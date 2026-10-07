# Varyk milestone 5c: time, ids, and bytes

Date: 2026-10-07.

**Status.** Design, not yet implemented. Extends the milestone-1 to
milestone-5b4 specs, `2026-09-23-varyk-design.md` ("M1 §n"),
`2026-09-25-milestone-2-design.md` ("M2 §n"),
`2026-09-26-milestone-3-design.md` ("M3 §n"),
`2026-09-29-milestone-4-design.md` ("M4 §n"),
`2026-09-30-milestone-5a-design.md` ("M5a §n"),
`2026-10-01-milestone-5b1-design.md` ("M5b1 §n"),
`2026-10-02-milestone-5b2-design.md` ("M5b2 §n"),
`2026-10-03-milestone-5b3-design.md` ("M5b3 §n"), and
`2026-10-05-milestone-5b4-design.md` ("M5b4 §n"). Their principles,
ownership rules, and compiler architecture stay in force. Varyk is
experimental and pre-1.0: anything here may change.

This spec is the compiler and `varyk-std`. Section 6 fixes what
`varyk-sql` and `varyk-http` must provide; each gets a short spec of its
own in its repository once this one is approved, as in M5b3 and M5b4.

## 1. Summary and scope

Milestone 5's bar is one golden path: a users API on a database is
`varyk init`, `varyk add http sql`, one file, and `varyk run` away, within
fifteen minutes of `cargo install varyk`, and `varyk build --release`
leaves one native executable. 5b4 made the path; this milestone gives it
what a production API needs that the language lacks, so that the users
API does not ship `created_at` as a string. After 5c the hero's types
read:

```varyk
struct User {
    id: i64,
    name: string,
    created_at: Time,
}
```

and `created_at` is a native `timestamptz` column on Postgres, written by
`db.run("insert into users (name, created_at) values ($1, $2)", user.name, Time::now())`.

5c adds three built-in types, each a reserved type name like `Error`:

1. `Time`, one point in time in UTC with microsecond precision
   (section 2.1);
2. `Uuid`, with `Uuid::new()` making a time-ordered version 7
   (section 2.2);
3. `Bytes`, an immutable run of bytes, base64 in JSON (section 2.3).

Each reaches the places a scalar already reaches where it makes sense:
JSON, `env::parse`, `s.parse()`, trailing SQL values, route parameters,
and `.rs` facade signatures (section 2.4). `varyk-std` holds the three
types (section 5); `varyk-sql` reads and writes them as native columns,
dropping sqlx's `Any` driver to do it, and `varyk-http` reads and writes
bytes bodies (section 6).

### 1.1 What was decided and why

- **One time type.** A point in time in UTC covers `created_at`,
  `updated_at`, and `expires_at`, which are what a service stores. A
  calendar `Date`, a `Duration`, and time zones each double the work
  through every list the compiler keeps for a built-in type, and wait
  for a program that needs them (section 11).
- **Microseconds.** Postgres `timestamptz` and MySQL `DATETIME(6)` hold
  microseconds, so a `Time` that went into the database and came back is
  equal to the one that went in. `Time::now()` is cut to the microsecond
  for the same reason.
- **Built-in types, each its own variant.** The alternatives were one
  "scalar kind" variant shared by `Time` and `Uuid`, a new abstraction
  that two types do not justify and `Bytes` would not fit, and Rust
  structs of `varyk-std` imported through a facade, which cannot reach
  JSON, printing, comparison, map keys, or SQL values without an
  exception in each. A built-in type goes through the hand-written lists
  `Error` went through (section 7), and nothing new is added to drive
  them.
- **`varyk-sql` drops `Any`.** sqlx's `Any` driver has no date or uuid
  kind, so native columns need the concrete pools. The `Any` alternative
  (text through casts written in every query) would have kept the
  roadmap's "native columns" out of milestone 5. Concrete pools also fix
  MySQL's unsigned integers, which `Any` reads wrong today.
- **Base64 in JSON.** It is the convention of protobuf's JSON mapping and
  of most APIs that put bytes in JSON, so a struct with a `Bytes` field
  needs no special handling. A JSON array of numbers is four times
  larger and expected by nobody.
- **`Uuid::new()` makes a version 7.** Time-ordered ids keep a primary
  key index compact, which is the common use. A version 7 id shows when
  it was made, to anyone who reads it; `Uuid::v4()` is the random one,
  asked for by name. The user chose this knowing the trade-off.
- **No crash for an absent value, and none for a range.** Every call that
  can leave the range of `Time` returns a `Result`; integer `+` stops a
  program on overflow (M1), a library call does not. The one panic left
  is the `uuid` crate's, under the `Uuid` constructors: the operating
  system's random source failing, an environment fault, not an absent
  value (section 3).

## 2. Language surface

`Time`, `Uuid`, and `Bytes` join `BUILTIN_TYPE_NAMES`; declaring a type,
enum, or module with one of the names is V0113, as for `Error`. That is a
breaking change for a program that declared one.

### 2.1 `Time`

A point in time in UTC, with microsecond precision, from the year 0000 to
the year 9999, the range RFC 3339 can write. A Copy type.

| Call | Uses the value | Result |
|---|---|---|
| `Time::now()` | none | the current time, cut to the microsecond |
| `Time::from_iso(text)`; `text: string` read | none | `Result<Time, Error>`, by the rules below |
| `Time::from_unix(seconds)`; `seconds: i64` | none | `Result<Time, Error>`; seconds since 1970-01-01T00:00:00Z; an `Err` outside the range |
| `Time::from_unix_micros(n)`; `n: i64` | none | `Result<Time, Error>`; microseconds since 1970-01-01T00:00:00Z; an `Err` outside the range |
| `t.to_iso()` | reads | `string`, the text `{}` prints |
| `t.to_unix()` | reads | `i64` seconds, rounded down, so a time before 1970 gives the second it falls in |
| `t.to_unix_micros()` | reads | `i64` |
| `t.add_seconds(n)`; `n: i64` | reads | `Result<Time, Error>`; `n` may be negative; an `Err` outside the range |
| `t.seconds_since(u)`; `u: Time` | reads | `i64`, `t` minus `u` in whole seconds, rounded toward zero; negative when `u` is later |
| `text.parse()` | reads | `Result<Time, Error>`, as `Time::from_iso` |

**Reading.** `from_iso` and `parse` read RFC 3339: `YYYY-MM-DD`, `T` or
`t`, `HH:MM:SS`, an optional fraction of one to nine digits, and `Z`,
`z`, or an offset `+HH:MM` or `-HH:MM`. The time is converted to UTC, and
digits past the sixth are cut, so a nanosecond time from Go reads. A
leap second (`:60`), a space for the `T`, the compact form with no
dashes or colons, week and ordinal dates, and a time outside the range
after conversion are an `Err` whose message names the text:
`` `2026-10-07 12:00` is not a time like 2026-10-07T12:00:00Z ``.
`iso` names the familiar standard (JavaScript's `toISOString`);
`docs/language.md` says that it means RFC 3339's form of ISO 8601.

**Writing.** `{}`, `to_iso`, JSON, and assertion messages write UTC with
`Z`: `2026-10-07T12:00:00Z`, and a fraction only when the time has one,
with its trailing zeros dropped: `2026-10-07T12:00:00.5Z`,
`2026-10-07T12:00:00.123456Z`.

**Comparing.** `==`, `!=`, `<`, `<=`, `>`, and `>=`; `sort` and
`contains` on a `Vec<Time>`. A `Time` is not a `HashMap` key.

### 2.2 `Uuid`

A 128-bit identifier. A Copy type.

| Call | Uses the value | Result |
|---|---|---|
| `Uuid::new()` | none | a new version 7 id: time-ordered, ordered by creation within one process |
| `Uuid::v7()` | none | the same as `Uuid::new()` |
| `Uuid::v4()` | none | a new random version 4 id |
| `text.parse()` | reads | `Result<Uuid, Error>`; only the 36-character hyphenated form, hex digits in either case |

A version 7 id holds the millisecond it was made in, readable by anyone
who has the id; a version 4 id holds nothing but randomness.
`docs/language.md` says so beside the table, so that a writer who
publishes ids and does not want their creation times known picks
`Uuid::v4()`.

`{}`, JSON, and assertion messages write the lowercase hyphenated form.
`==` and `!=`; `contains` on a `Vec<Uuid>`; a `Uuid` is a `HashMap` key. No ordering: `<` on a `Uuid` is V0200.

### 2.3 `Bytes`

An immutable run of bytes. Not a Copy type: it is owned, moved, and
borrowed exactly as `string` is (M1 §3), and `.clone()` copies the handle
to a shared buffer, not the bytes.

| Call | Uses the value | Result |
|---|---|---|
| `Bytes::from_text(text)`; `text: string` read | none | the UTF-8 bytes of `text`, a copy |
| `Bytes::from_base64(text)`; `text: string` read | none | `Result<Bytes, Error>`, by the JSON rule below |
| `b.to_text()` | reads | `Result<string, Error>`, a copy; an `Err` when the bytes are not UTF-8 |
| `b.to_base64()` | reads | `string`, standard base64 with padding |
| `b.len()` | reads | `usize`, the number of bytes |
| `b.is_empty()` | reads | `bool` |

`==` and `!=`; `contains` on a `Vec<Bytes>`. `{}` on `Bytes` is V0203,
whose note names `to_text` and `to_base64`. A `Bytes` is not a `HashMap`
key and has no ordering.

**JSON.** A `Bytes` is a string of standard base64 with padding (RFC 4648
section 4). Reading is strict: another alphabet, missing padding, or a
character outside the alphabet is an `Err`.

### 2.4 Where the types go

| Use | `Time` | `Uuid` | `Bytes` |
|---|---|---|---|
| A field of a struct `json::parse` or `json::stringify` reaches, a route's body or return value | yes | yes | yes |
| A field of a struct `env::parse` reads | yes | yes | no, V0210 |
| `s.parse()` | yes | yes | no, V0200 |
| A trailing value of a `Vec<varyk_std::Value>` parameter, or an `Option` of one | yes | yes | yes |
| A route's path or query parameter | yes | yes | no, V0219 |
| `.rs` facade signatures (section 2.5) | yes | yes | yes |
| `{}` | yes | yes | no, V0203 |
| `assert_eq` | yes | yes | yes |
| A field of a struct that derives `==` or `.clone()` | yes | yes | yes |

A path or query parameter that does not read as its type is a 400 built
by `varyk-http`, the handler not called, as for an integer today
(M5b4 §3). A trailing value is turned into `Value` by the compiler, as
M5b3 §2.2 sets out; section 6.2 says how `varyk-sql` binds each one. A
`Bytes` passed as a trailing value is borrowed, not moved, as a `string`
is.

### 2.5 Facades

A `.rs` function, method, or `pub` field of an imported struct may name
the three types by their full paths, as it names `varyk_std::Error`:

| Rust | Varyk | Where |
|---|---|---|
| `varyk_std::Time`, `varyk_std::Uuid` | `Time`, `Uuid` | parameters, returns, `pub` fields, the fields of an imported enum's variants, and inside `Option`, `Vec`, and `Result` |
| `&varyk_std::Bytes` | `Bytes`, read | parameters |
| `varyk_std::Bytes` | `Bytes`, owned: the argument moves | parameters, returns, `pub` fields, the fields of an imported enum's variants, and inside `Option`, `Vec`, and `Result` |

`&varyk_std::Time`, `&varyk_std::Uuid`, `&mut varyk_std::Bytes`, and the
types reached through a `use` line rather than the full path are V0108,
with a note giving the accepted form. A type parameter
`T: serde::de::DeserializeOwned` or `&T: Serialize` (M5b3, M5b4 §2.7)
reaches the three types with no change, since they pass the JSON check.
An imported enum whose variants hold the three types is matched in Varyk
as any imported enum is (M3 §4); `varyk-http`'s WebSocket message,
`Text(String)` or `Binary(varyk_std::Bytes)`, is the case that needs it.

### 2.6 Not in milestone 5c

A calendar date, a time of day, a duration type, time zones and local
time, formatting a `Time` by a pattern, a monotonic clock; ordering
`Uuid`s, the nil id, and other versions; indexing, slicing, or building
`Bytes` piece by piece, and `Bytes` from a `Vec<u8>`; `Bytes` in
`env::parse`; `Time` or `Bytes` as a map key; a handler parameter bound
to the raw body (a handler reads it through `http::Request`, section
6.1). `docs/language.md`'s "Not in milestone 5b4" section, renamed
"Not in milestone 5c", names them.

## 3. Safety

- **No absent value and no range stops a program.** Every conversion
  that can fail returns a `Result`. The message of a `Time`, `Uuid`, or
  number conversion names the text or the number; a `Bytes` conversion's
  says what is wrong without repeating the text or the bytes, which can
  be a whole body. `Time::now()`, `Time`'s `to_` calls, `seconds_since`,
  and the `Uuid` constructors cannot fail on a value: `seconds_since` cannot overflow an
  `i64` inside the range, a clock set before 1970 gives `Time::now()` a
  negative time, and a clock outside the range gives its nearest end.
- **One documented panic, the `uuid` crate's.** `Uuid::new()`, `v7()`,
  and `v4()` panic when the operating system cannot give random bytes,
  as Rust's own `HashMap` seeding and the tokio runtime's start do; it
  cannot happen on macOS, Windows, or Linux once the system has booted.
  The clock does not panic: a version 7 id takes its time from the
  reading `Time::now()` makes, not from `uuid`'s own `now_v7`, which
  stops the program when the clock reads before 1970 (section 5). In a
  handler, `varyk-http` contains the panic as a 500 (M5b4 §3); elsewhere
  it stops the program with the crate's message. `docs/language.md` says
  so.
- **Input is checked at the edge.** A `Time`, `Uuid`, or `Bytes` in a JSON
  body, a path, or a query string that does not read as its type is a
  400 built by the package, so a handler's parameters are always values
  of their types.
- **A `Uuid` v7 shows its creation time.** The default is the useful
  one for keys; the private one is asked for by name (section 2.2).
- **No new hidden allocation.** The compiler still inserts exactly the two
  allocations AGENTS.md names. `Bytes::from_text`, `from_base64`,
  `to_text`, and `to_base64` copy because the program called them.
  `Value::from(&b)` for a trailing `Bytes` adds one to a reference count
  and allocates nothing, since `varyk-std` makes every `Bytes` it builds
  in the `bytes` crate's shared form (section 5); `docs/design.md` says
  so beside the two allocations.

## 4. Generated Rust

- Types: `Time` is `::varyk_std::Time`, `Uuid` is `::varyk_std::Uuid`,
  `Bytes` is `::varyk_std::Bytes`.
- An associated call is written in full, as `Error::new` is (M5a):
  `Time::now()` is `::varyk_std::Time::now()`. A method is
  `receiver.name(args)`, as every built-in row is, so Rust lends the
  receiver whether it is a value or already a reference:
  `t.add_seconds(n)`, `b.len()`.
- `text.parse()` into `Time` or `Uuid` is
  `::varyk_std::parse::<::varyk_std::Time>(..)`, through the sealed
  `Parse` trait.
- `==`, `!=`, and the orderings are Rust's operators on the types, which
  implement `Clone`, `Debug`, `PartialEq`, and `Eq`; `Time` and `Uuid`
  also `Copy` and `Display`, `Time` `PartialOrd` and `Ord`, and `Uuid`
  `Hash`.
- `Time` and `Uuid` are passed by value. `Bytes` goes through borrow
  analysis as a struct does (section 7.4): a parameter is
  `&::varyk_std::Bytes`, a field owns, `let c = b` moves, and returning
  part of a parameter gives `&::varyk_std::Bytes`, as for a struct;
  where a borrowed one cannot go (V0304), the fix-it offers `.clone()`,
  as for a `string`.
- A trailing value is `::varyk_std::Value::from(t)` for `Time` and
  `Uuid`; a `Bytes` is lent, `::varyk_std::Value::from(&b)` (or `b` when
  it is already a reference), and an `Option<Bytes>` goes through
  `.as_ref()`, as an `Option<string>` goes through `.as_deref()`.
- A route's path or query parameter of the new types is read through the
  same `param` and `query` calls an integer uses (M5b4 §4).

## 5. `varyk-std`

Three new modules, one per type, and four new dependencies. The crates
behind `Time` and `Uuid` stay private to `varyk-std`, so it can change
them without breaking a package.

| Type | Holds | Built on |
|---|---|---|
| `Time` | an `i64` of microseconds since 1970-01-01T00:00:00Z | `time` (`>= 0.3.45`, which builds on Rust 1.83) to read and write RFC 3339, after `varyk-std` checks the shape of section 2.1 itself, since the crate's parser takes any separator for the `T`, a leap second, and any number of fraction digits; `now()` reads `std::time::SystemTime` |
| `Uuid` | 16 bytes | `uuid` (`>= 1.20`, which builds on Rust 1.63, features `v4` and `v7`); a version 7 id is `Uuid::new_v7(Timestamp::from_unix(&CONTEXT, secs, nanos))`, where `CONTEXT` is one `static` `Mutex<ContextV7>` of `varyk-std`'s, which orders the ids of one process, and the time is `Time::now()`'s reading, a time before 1970 taken as 0, since a version 7 id holds unsigned milliseconds since 1970; `parse` goes through the crate's parser after `varyk-std` checks that the text is 36 characters long, since the crate also reads the simple, the braced, and the URN forms, which section 2.2 refuses |
| `Bytes` | a `bytes::Bytes` | `bytes` (`>= 1.10.1`, the first whose `from_owner` does not leak on `to_vec`); `base64` for the text form |

A `Bytes` that `varyk-std` builds from a `Vec<u8>` (`from_text`,
`from_base64`, serde) is made with `bytes::Bytes::from_owner`, whose
clone only adds to a reference count; a `bytes::Bytes` made from a `Vec`
the plain way allocates its count on its first clone. One made from a
`bytes::Bytes` a package hands over (axum's body) wraps that handle with
`from_owner` the same way, without copying the bytes, so the clone of
every `Bytes` only adds to a count.

The workspace's resolver is version 2, which does not look at
`rust-version`, so its committed `Cargo.lock` holds versions that build
on 1.85 (`time` 0.3.45, `uuid` 1.26, and what they need). A generated
crate is edition 2024, whose resolver picks versions that build on the
user's Rust. CI's 1.85 leg checks both.

What a package's Rust sees, beyond the calls of section 2:
`Uuid::from_bytes([u8; 16])` and `as_bytes`; `Bytes` from and into `bytes::Bytes`, and `&[u8]`
through `AsRef`; `FromStr` for `Time` and `Uuid`, with the rules of
`parse`. `bytes` is the one public dependency, since axum gives a body as
a `bytes::Bytes` and handing it over must not copy it.

**Serde,** written by hand against `varyk_std::serde`, as the derived
code is (M5a):

- Each of the three asks its deserializer for a string
  (`deserialize_str`). A deserializer may answer with raw bytes instead,
  as `varyk-sql` does for a binary column (section 6.2); JSON and `env`
  never do.
- `Time` and `Uuid` serialize as a string in their written form, and
  deserialize from a string (`visit_str`): a JSON number is not a
  `Time`. `Uuid` also takes exactly 16 raw bytes (`visit_bytes`,
  `visit_byte_buf`), how `varyk-sql` hands a 16-byte column to it.
  In `env::parse`, a value that does not read is an error that names
  the variable, as a number's does; `env`'s reader puts the name before
  the type's message.
- `Bytes` serializes as a base64 string. It deserializes from a base64
  string (`visit_str`) or from raw bytes (`visit_bytes`,
  `visit_byte_buf`). JSON never calls the raw ones for a string, so the
  two cannot be confused; the raw ones are how `varyk-sql` hands a
  `bytea` or `BLOB` column to a field (section 6.2).
- `Debug`, which `Value`'s derive needs, prints the written form, and
  `Bytes` as its base64. Assertion messages print with `{}`, not
  `Debug` (M5a), so they show a `Time` or a `Uuid` and not a `Bytes`.

**`Value`** gains `Time(Time)`, `Uuid(Uuid)`, and `Bytes(Bytes)`, with
`From<Time>`, `From<Uuid>`, `From<&Bytes>`, `From<Option<Time>>`,
`From<Option<Uuid>>`, and `From<Option<&Bytes>>`. A new variant breaks a
`match` on `Value`, so `varyk-std` goes to 0.8.

Every program that uses `varyk-std` now builds `time`, `uuid`, `base64`,
and `bytes`; a
program on `varyk-http` built `bytes` already, through axum. Building
them only for a program that uses the types would need a mechanism of
its own (section 11).

## 6. The packages' contract

### 6.1 `varyk-http`

The compiler depends on one thing: `Request::param` and `Request::query`
(M5b4 §6.1) accept `varyk_std::Time` and `varyk_std::Uuid`, for a
required and an `Option` query value. Today's package does that by
implementing its `Plain` trait for the two types, which `FromStr`
(section 5) allows.

`varyk-http`'s own spec (`Varyk-Lang/varyk-http`,
`docs/specs/2026-10-07-varyk-http-0.2-design.md`) adds, through ordinary
facade signatures (section 2.5), each taking a `Bytes` as
`&varyk_std::Bytes`: `req.body_bytes()`, the body without copying it,
and `req.set_body_bytes(b)` for tests; `Response::bytes(b,
content_type)` and `r.body_bytes()`; `part.bytes()` and
`part.content_type()` for a multipart upload; `ws.send_bytes(b)` and
`ws.recv_message()`, giving an enum of a text or a binary message, with
`ws.recv()` staying text; the client's `post_bytes` and `put_bytes`.
Each bytes call has a name of its own because Varyk has no overloading
and the server's `Response` is also the client's. The 0.1 items left
"until a bytes type exists" are then done.

### 6.2 `varyk-sql`

- **Concrete pools.** `Pool` and `Tx` hold one of `SqlitePool`,
  `PgPool`, and `MySqlPool` (and their transactions), each behind its
  existing cargo feature, chosen from the URL's scheme as today. The
  `Any` driver is gone, and with it the README's MySQL unsigned-integer
  warning.
- **Binding** a trailing value:

  | Value | Postgres | MySQL | SQLite |
  |---|---|---|---|
  | `Time` | `timestamptz` | `DATETIME(6)`, in UTC | text of a fixed width, `2026-10-07T12:00:00.000000Z` |
  | `Uuid` | `uuid` | 36-character text | 36-character text |
  | `Bytes` | `bytea` | `BLOB` | `BLOB` |

  SQLite has no time type, and text of a fixed width keeps `order by` and
  `<` in SQL right; the written form of section 2.1 does not, since
  `12:00:00.5Z` sorts before `12:00:00Z`.
- **Reading** a column into a field: a field of the three types asks for
  a string (section 5), so `varyk-sql` hands each column over by the
  column's type. A `timestamptz`, a `timestamp` without a zone (read as
  UTC), and MySQL's `DATETIME` and `TIMESTAMP` go as the written form of
  a `Time`; a `uuid` as the written form of a `Uuid`; text, SQLite's
  times and ids among it, as the text; `bytea`, `BLOB`, `BINARY`, and
  `VARBINARY` raw. The field's deserializer takes what it reads (a `Uuid`
  takes 16 raw bytes) and refuses the rest, an `Error` naming the
  column and not the value.
- **A `Uuid` is text on MySQL and SQLite,** readable in a SQL shell and
  in SQLite tools, the same as in JSON; a program writes `char(36)` or
  `text`.
- **`None` is an untyped `NULL` on Postgres,** so the server takes its
  type from the query, and the casts `varyk-sql` 0.2 asks for a value
  that may be `None` go, but for one at a parameter's first use where
  nothing gives it a type (`$1 is null`), which Postgres still needs.
- `varyk-sql`'s own spec (`Varyk-Lang/varyk-sql`,
  `docs/specs/2026-10-07-varyk-sql-0.3-design.md`) has the rest.

### 6.3 Versions and releases

The compiler reserves three names (`feat!`), and `Value` gains three
variants (`varyk-std` 0.8). The order: varyk 0.8.0 with `varyk-std`
0.8.0; then `varyk-sql` 0.3 and `varyk-http` 0.2, each needing
`varyk-std` 0.8. A program on varyk 0.8 that still has `varyk-http` 0.1
or `varyk-sql` 0.2 is refused by V0404, since their `varyk-std`
requirement is not the compiler's version; no new check is needed.

## 7. Compiler changes

File and line anchors are approximate.

### 7.1 Types (`crates/varyk/src/types.rs`, `builtins.rs`)

- `Ty::Time`, `Ty::Uuid`, `Ty::Bytes`; `Time` and `Uuid` in `is_copy`
  and `copy_in_rust`; `Uuid` in `is_map_key`; the three in
  `BUILTIN_TYPE_NAMES` and in whatever marks a program as using
  `varyk-std` (`has_error` and its callers).
- `builtins.rs`: an `Owner` for each type, kept apart from the `time`
  module's owner; `Owner::of`; the `Shape`s the rows need; the rows of
  sections 2.1 to 2.3; `uses_std`; `ElementRule::Ordered` gains `Time`,
  `ElementRule::Comparable` gains the three.

### 7.2 Resolve (`crates/varyk/src/resolve/`)

- `resolve_type` maps the three names; `std_type_taken` reserves them
  (V0113).
- `resolve/signatures.rs` and `interop/signatures.rs`: `std_item`
  recognises `varyk_std::Time`, `Uuid`, and `Bytes`; `RustTy` gains them;
  the mapper accepts the forms of section 2.5, the fields of an imported
  enum's variants among them, and refuses the others with V0108 and its
  note; `names_std` lists them.
- The exhaustive matches over `Ty` in `package_items.rs` and `borrow.rs`.

### 7.3 Checks (`crates/varyk/src/types/`)

- `check/values.rs`: `path_owner` maps `Time`, `Uuid`, and `Bytes`; the
  "no function" messages name each type's calls.
- `check.rs`: orderings accept `Time` (V0200 for the others, with the
  note naming `Time`); `{}` accepts `Time` and `Uuid` (V0203 for `Bytes`,
  with its note); `is_value_type` accepts the three and their `Option`s,
  and V0218's note lists them; `assert_eq` shows `Time` and `Uuid` in
  its message, as it shows what `{}` prints.
- `check/methods.rs`: `parse` targets gain `Time` and `Uuid` (V0200 for
  `Bytes`); `derived_clone` accepts the three, so `.clone()` on a `Time`
  or `Uuid` reaches the V0100 a number gets, its message naming them.
- `derives.rs`: `convertible` accepts the three for JSON, `env_readable`
  accepts `Time` and `Uuid`, `ENV_FIELDS` and the JSON note list them,
  `ty_name` names them.
- `check/routes.rs`: a path parameter and `is_query` accept `Time` and
  `Uuid`, and V0219's text lists them.

### 7.4 Borrow analysis (`crates/varyk/src/borrow.rs`)

`Bytes` is a compound type (`Ty::is_compound`), as `Error` is: its Rust
type is `::varyk_std::Bytes` owned and `&::varyk_std::Bytes` borrowed,
with none of `string`'s `&str` forms, so the rules that name
`Ty::String` for its `&str` and `String` forms do not take it. A
struct's rules give section 2.3: borrowed parameter, owned field, move on
`let`, and a returned part of a parameter. A trailing `Bytes` is read,
not moved, and `clone_fix_it`, which adds `.clone()` to a V0304 on a
`string` only, adds it on a `Bytes` too. `Time`
and `Uuid` take the Copy rules.

### 7.5 Backend (`crates/varyk/src/backend/`)

`rust_type` for the three; the associated calls in full beside
`Error`'s in `rust_expr.rs`; the `parse` turbofish; `Value::from` of a
lent `Bytes`, and `.as_ref()` for an `Option<Bytes>`.

## 8. Diagnostics

No new code. Each new refusal is an existing code whose row in
`docs/language.md` and whose comment in `codes.rs` gain the case:

| Code | New case |
|---|---|
| V0100 | `.clone()` on a `Time` or `Uuid`, which are copied on use |
| V0101 | `Time` or `Bytes` as a `HashMap` key; the message lists `Uuid` among the key types |
| V0108 | a facade form section 2.5 refuses |
| V0113 | a type, enum, or module named `Time`, `Uuid`, or `Bytes` |
| V0200 | `<` and the other orderings on a `Uuid` or `Bytes`; `sort` on them; `parse` into `Bytes`; its list of `parse` targets gains `Time` and `Uuid` |
| V0203 | `{}` on `Bytes`; its list of what `{}` prints gains `Time` and `Uuid` |
| V0210 | `Bytes` in a struct `env::parse` reads |
| V0218 | its list of trailing value types gains the three |
| V0219 | a `Bytes` path or query parameter; its list gains `Time` and `Uuid` |
| V0304 | no new case; its fix, `.clone()` for text, is given for a `Bytes` too |

If the plan finds a case none of these covers, it gets a new code with a
fixture, a registration in `tests/errors.rs`, and a row in
`docs/language.md`, as AGENTS.md asks.

## 9. Testing and definition of done

- `varyk-std` unit tests: RFC 3339 reading (offsets converted to UTC,
  nine fraction digits cut to six, lowercase `t` and `z`, the year 0000
  and 9999 bounds before and after the offset, a leap second, a space,
  ten fraction digits, the compact form, and a week date refused) and
  writing (no fraction, a
  fraction with its trailing zeros dropped); `from_unix`,
  `from_unix_micros`, and `add_seconds` at the edges of the range;
  `to_unix` rounding down before 1970; `seconds_since` rounding toward
  zero; `Uuid` versions, the ordering of version 7 ids made in a row
  and of two made from a clock reading before 1970, and
  parsing only the hyphenated form; strict base64 both ways and
  `to_text` on bytes that are not UTF-8; serde both ways for the three,
  including `Bytes` from raw bytes, a `Uuid` from 16 raw bytes and not
  from 15, a JSON number refused as a `Time`, and an `env::parse` error
  for a `Time` naming its variable;
  `Value::from` for each type and each `Option`.
- Interop unit tests: each accepted facade form of section 2.5,
  including an imported enum with a `Bytes` variant matched in Varyk,
  and each refused one with its note.
- Type tests: every row of the tables of sections 2.1 to 2.4, accepted
  and refused, and each case of section 8.
- `insta` snapshots of the generated Rust for a program using each
  type's calls, a struct of the three through JSON, a trailing value of
  each, a `Bytes` parameter, field, move, and cloned return, and a route
  with a `Uuid` path parameter and an `Option<Time>` query parameter.
- Soundness templates (M2 §7): `Bytes` in each position the `string`
  templates cover, in the must-build and the must-reject programs.
- Fixture packages: a facade fixture whose `.rs` takes and returns the
  three types; the stub `varyk-http` fixture (M5b4 §9) with its `Plain`
  implemented for `Time` and `Uuid`, and `http_user` binding a `Uuid`
  path parameter through it.
- A new example, `examples/records.vr`, with output fixed for
  `tests/examples.rs`: values made by `Time::from_iso` and parsed
  `Uuid`s (it calls `Time::now()` and `Uuid::new()` without printing
  them), `add_seconds` and `seconds_since`, a struct of the three
  written with `json::stringify` and read back, and `to_base64` and
  `to_text`.
- The language-reference test (M4 §7) passes with the three names in
  `docs/language.md`.
- Documents, in the same pull request: `docs/language.md` (a new
  "Time, ids, and bytes" section; the Copy types; the `parse` row; the
  JSON mapping; `env`; "Calling Rust" with the forms of section 2.5;
  trailing values; route parameters; the "Not in milestone 5b4" section
  renamed for 5c, with its 5c line removed and section 2.6 added; the
  code rows of section 8),
  `docs/design.md` (the reference count beside the two allocations, and
  `varyk-std`'s dependencies), `docs/roadmap.md` (section 10),
  `docs/open-questions.md` (section 11), `README.md` (features and
  status), and `AGENTS.md` (the `varyk-http` contract is section 6 of
  the 5b4 spec with section 6.1 of this one, and the three types in its
  list of what `varyk-std` holds).
- CI green, including the 1.85 build; no `unwrap`, `expect`, or other
  crash-on-absence call added.

Done when every item of the roadmap's 5c compiler list is checked. The
items for `varyk-sql` and `varyk-http` are checked at their releases, and
milestone 5's bar is met when the `varyk-http` demo's `User` has a
`created_at: Time` read through `varyk-sql`. The demo runs on SQLite, as
its tests do; the native Postgres and MySQL columns are proved by
`varyk-sql`'s own CI on all three databases.

## 10. Roadmap changes

The 5c entry links this spec and becomes two lists under one heading.

The compiler:

- `Time`: `now`, `from_iso`, `from_unix`, `from_unix_micros`, `to_iso`,
  `to_unix`, `to_unix_micros`, `add_seconds`, `seconds_since`, `parse`,
  comparison, RFC 3339 in JSON and `env`
- `Uuid`: `new` (version 7), `v7`, `v4`, `parse`, a map key, the
  hyphenated form in JSON and `env`
- `Bytes`: `from_text`, `from_base64`, `to_text`, `to_base64`, `len`,
  `is_empty`, base64 in JSON
- The three in trailing values, route parameters (`Time` and `Uuid`),
  and facade signatures; `varyk-std` 0.8
- The example `records`, and the docs

The packages, each in its own repository:

- `varyk-sql`: concrete pools in place of `Any`; the three types as
  native columns
- `varyk-http`: `Time` and `Uuid` path and query parameters; bytes
  bodies, uploads, binary WebSocket messages, and the client's bytes;
  `created_at: Time` in the `users` demo

The milestone-5 introduction's "It is met when 5b4 and 5c are done"
stays. The "Unscheduled" paragraph that points to "Not in milestone
5b4" in `docs/language.md` follows the section's new name.

## 11. Open questions

Added to `docs/open-questions.md`:

- Should Varyk have a calendar `Date`, a time of day, or a `Duration`,
  and should `time::sleep` take a `Duration`?
- Should a `Time` carry or convert to a time zone, for a service that
  shows local times?
- Should `varyk-std` build `time`, `uuid`, and `base64` only for a
  program that uses the types, through cargo features the driver sets?
- Should `Uuid`s order, so a `Vec<Uuid>` of version 7 ids sorts by
  creation?
- Should `Bytes` be indexed, sliced, and built from a `Vec<u8>`, and
  should there be a growable buffer?

## 12. Decisions

Taken with the user on 2026-10-07:

- One time type, a point in time in UTC; no `Date`, `Duration`, or time
  zones in 5c.
- `varyk-sql` drops sqlx's `Any` driver for concrete pools in 5c, so the
  types are native columns.
- `Bytes` is base64 in JSON, with `to_base64` and `from_base64` beside
  it.
- `Uuid::new()` makes a version 7, with `Uuid::v7()` and `Uuid::v4()` by
  name; the user chose the default knowing a version 7 id shows its
  creation time.
- Built-in types, each its own variant, over a shared scalar variant or
  imported Rust structs (section 1.1).
- `Time::from_iso` and `to_iso` beside `parse` and `{}`, since
  `from_iso` needs no type from where its result goes; `from_unix` and
  `to_unix` in seconds, the meaning the word has elsewhere, with
  `from_unix_micros` and `to_unix_micros` for a round trip that loses
  nothing.
- A failure of the operating system's random source under `Uuid::new()`
  is the `uuid` crate's panic, documented, rather than a `Result` on every
  id; the `uuid` crate's own random source is used, with no call of
  `varyk-std`'s own.
- The compiler and `varyk-std` are this spec; `varyk-sql` and
  `varyk-http` each get a spec of their own under section 6, written
  before this one's plan.
- `varyk-sql` stores a `Uuid` as 36-character text on MySQL and SQLite,
  and binds `None` as an untyped `NULL` on Postgres.
- `varyk-http` receives binary WebSocket messages through a new
  `recv_message`, giving an imported enum, so section 2.5 admits the
  three types in an imported enum's variants; `recv` stays text.
- The bar's demo runs on SQLite; the native columns of Postgres and
  MySQL are proved in `varyk-sql`'s CI.
