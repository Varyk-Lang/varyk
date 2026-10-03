# Varyk milestone 5b3: facades for packages

Date: 2026-10-03.

**Status.** Design, not yet implemented. Extends the milestone-1 to
milestone-5b2 specs, `2026-09-23-varyk-design.md` ("M1 §n"),
`2026-09-25-milestone-2-design.md` ("M2 §n"),
`2026-09-26-milestone-3-design.md` ("M3 §n"),
`2026-09-29-milestone-4-design.md` ("M4 §n"),
`2026-09-30-milestone-5a-design.md` ("M5a §n"),
`2026-10-01-milestone-5b1-design.md` ("M5b1 §n"), and
`2026-10-02-milestone-5b2-design.md` ("M5b2 §n"). Their principles,
ownership rules, and compiler architecture stay in force. Varyk is
experimental and pre-1.0: anything here may change.

The package this milestone exists for, `varyk-sql`, lives in its own
repository, `Varyk-Lang/varyk-sql`, and has its own spec there
(`docs/specs/2026-10-03-varyk-sql-design.md`). This spec is the compiler
side: what a `.rs` facade can say so that such a package can be written
in Varyk with a thin layer of Rust, and nothing in the program that uses
it is Rust.

## 1. Summary and scope

A facade `.rs` module (M3 §1) can name only concrete types, so until now a
package could not offer "read this row into whatever struct you name" or
"take these values, however many". Milestone 5b3 adds four shapes a
`.rs` facade can declare, one thing a `.vr` file can say, and one
convenience for adding a package:

1. a function or method with one type parameter standing for any Varyk
   data type, chosen from where the result goes (section 2.1);
2. a last parameter taking any number of scalar values (section 2.2);
3. a parameter taking only text written in the program (section 2.3);
4. `varyk_std::Error` as the error type of a returned `Result`
   (section 2.4);
5. `pub use` in a `.vr` file, so a package's path can be shorter than the
   path of its modules (section 2.5);
6. `varyk add sql` as a shorthand for the official package (section 2.6).

Together they let `varyk-sql`'s facade declare

```rust
pub async fn one<T: varyk_std::serde::de::DeserializeOwned>(
    &self,
    query: &'static str,
    values: Vec<varyk_std::Value>,
) -> Result<T, varyk_std::Error>
```

and a program write

```varyk
let user: User = db.one("select id, name from users where id = ?", id).await?;
```

Apart from `pub use`, nothing here is a language feature of Varyk
itself: a Varyk function still
has no type parameters, no variadic parameter, and no literal-only
parameter. These are shapes the importer accepts in a `.rs` signature, and
rules for calling them. The facade rule stands: a crate is reached through
a `.rs` module in the same package (M3 §1).

### 1.1 What was decided and why

- **One direction for the type parameter.** Only `DeserializeOwned`, for
  reading a result into a Varyk type. The other direction, `&T:
  Serialize` for sending a Varyk value out, is what an HTTP client needs
  and waits for milestone 5b4. The machinery is the same; adding it
  later is a small change.
- **Scalars only in the value list.** `varyk_std::Value` holds `Null`,
  `Bool`, `Int`, `Float`, and `Text`, and the compiler builds each one
  from the argument's type. A design that sent any data type through
  serde was considered and dropped: serde's conversion can fail in
  principle, and the only ways to handle that failure were a crash (never
  in Varyk), an error on every call, or a silent `Null`, which in a
  database query is wrong data with no error. SQL parameters are scalars,
  and the list grows when a package needs more.
- **Literal-only text is a type the facade asks for**, `&'static str`,
  not a rule about SQL. The compiler knows nothing about queries; it
  knows that this parameter accepts only a string literal.
- **`varyk-sql` is a separate repository**, released on its own, since its
  version is tied to sqlx and the databases as much as to the compiler.
  This repository proves the facade features with a fixture package that
  has no database.

## 2. Language surface

### 2.1 A type parameter chosen from the result

A `pub fn` or method of a `.rs` module may have one type parameter with
one bound, written inline and by full path:

```rust
pub fn one<T: varyk_std::serde::de::DeserializeOwned>(...) -> Result<T, varyk_std::Error>
pub fn first<T: serde::de::DeserializeOwned>(...) -> Result<Option<T>, varyk_std::Error>
pub async fn all<T: serde::de::DeserializeOwned>(...) -> Result<Vec<T>, varyk_std::Error>
```

The rules the importer applies:

- the type parameter may appear only in the return type, and the return
  type is `Result<T, E>`, `Result<Option<T>, E>`, or `Result<Vec<T>, E>`,
  with `E` the error of section 2.4; `async` is allowed as for any
  imported function (M5b1);
- the bound is `serde::de::DeserializeOwned` or
  `varyk_std::serde::de::DeserializeOwned`; `varyk-std` re-exports
  `serde`, so the second spelling needs no serde dependency, while the
  first needs `serde` in `[dependencies]` for rustc; `use` is not read
  for it, as for any type in a signature (M3 §4.4);
- two type parameters, a `where` clause, another bound, a lifetime
  parameter, or `T` in a parameter or elsewhere in the return keep
  today's rule: the function is imported but cannot be called (V0108),
  with the note saying which of these it is.

At a call, `T` is the type the result is used as, exactly as `json::parse`
finds its type (M5a §2.4): the declared type of the `let`, the parameter
type of the call it is an argument of, the function's return type when it
is returned, or the field's type in a struct literal. The expected type
flows through `?` (M4 §2.6) and through `.await`, so

```varyk
let user: User = db.one("...", id).await?;
let maybe: Option<User> = db.first("...", id).await?;
let users: Vec<User> = db.all("...").await?;
let n: i64 = db.one("select count(*) from users").await?;
```

all fix `T`. With no expected type (`match db.first(...).await? { .. }`,
or a `let` without a type), the call is V0207, the message `json::parse`
gives, with the help to write the type on the `let`. A call in started
position (M5b1 §2.3, no `.await`) has no expected type either and is
refused with V0207 and the note "a call that takes its type from where
its result goes cannot be started; add `.await`".

`T` may be any type a `json::parse` result may be (M5a §2.9): a number,
`bool`, `string`, `Option`, `Vec`, or `HashMap` of those, or a struct or
enum declared in the current package. The type joins the serde reach
analysis as a read type (M5a §2.4, §2.9): its serde derive is generated, the
attributes `#[rename]`, `#[default]`, and `#[skip]` apply with their
read-side checks (V0209), and a struct imported from a `.rs` module or
declared in another package is V0210, as for `json::parse`.

In the generated Rust the type is always written: `::sql::Pool::one::<User>(&db, ...)`
for a method of an imported struct, and after the name of a function
likewise, so rustc never has to infer it.

### 2.2 Any number of values

The last parameter of a `pub fn` or method of a `.rs` module may be
`Vec<varyk_std::Value>`. Varyk code writes that parameter as zero or more
arguments after the others:

```varyk
db.run("delete from users").await?;
db.run("insert into users (name, age) values (?, ?)", name, age).await?;
```

Each such argument is a `bool`, a `string`, `f32`, `f64`, `i8`, `i16`,
`i32`, `i64`, `u8`, `u16`, `u32`, or an `Option` of one of those. `u64`
and `usize` are not accepted, since they do not fit `Int`; write `n as
i64`. Anything else (a struct, a `Vec`, a `HashMap`, a `Result`) is
V0218, "a value of this type cannot be passed here", with the list of
types that can. A bare `None` has no type to take, since a trailing
argument is checked against this list and not against an expected type,
so it is V0207 as elsewhere; write `let missing: Option<i64> = None`
first. A `Vec` of `Value` itself cannot be written, since Varyk code
cannot name `Value` (section 3).

Each argument is read, not given away, as an argument of `json::stringify`
is (M5a §2.4): a number is copied, and a string is copied into the
`Value`. After the call the local is still usable. That string copy is
the hand-over: the callee owns its values (sqlx binds owned values), so
the copy is the cost of sending the value out, as a literal placed into
an owned slot is. It is the second allocation the compiler writes, and
AGENTS.md's "no hidden allocation" rule names it beside the first.

`varyk_std::Value` is

```rust
pub enum Value { Null, Bool(bool), Int(i64), Float(f64), Text(String) }
```

with `From` for each of the Varyk types above, with a string as `&str`,
and for `Option` of each (`None` is `Null`), so the generated Rust is
`vec![::varyk_std::Value::from(id), ::varyk_std::Value::from(name.as_str())]`
for an owned `name`, `::varyk_std::Value::from("Ada")` for a literal,
and `::varyk_std::Value::from(maybe_name.as_deref())` for an
`Option<string>`: a string is always handed over borrowed, and the
`From` makes the copy. No conversion can fail. Which `From` the backend
reaches is decided by the argument's type, so an `i32` becomes `Int` and
an `f32` `Float` without a cast in Varyk code.

A `Vec<varyk_std::Value>` anywhere but last, or a `varyk_std::Value`
alone as a parameter or in a return, is V0108 with a note.

### 2.3 Text written in the program

A parameter of type `&'static str` in a `pub fn` or method of a `.rs`
module is imported as a parameter that takes only a string literal. A
call passes a literal, with the usual escapes, and nothing else:

```varyk
db.one("select id, name from users where id = ?", id)   // fine
db.one(query, id)                                        // V0217
db.one(format!("... where id = {}", id))                 // V0217
```

V0217 reads "this argument must be text written in the program", with the
note "`one` takes only literal text, so no input can reach it", and, when
the argument is a `format!`, the help "pass the values after the text
instead". The message says nothing about SQL; `varyk-sql`'s documentation
does.

A `&'static str` anywhere else in a signature (a return, a field, inside
`Option`) keeps today's rule, V0108. A Varyk function cannot declare a
literal-only parameter of its own, so a `.vr` wrapper cannot pass one
through; `varyk-sql` therefore puts every call that takes a query in its
`.rs` facade, and the check happens at the program's call site.

### 2.4 `varyk_std::Error` in a signature

A `.rs` signature may name `varyk_std::Error`, by that full path, as the
error type of a returned `Result` whose `Ok` type is any type the
"Calling Rust" table admits (`Result<Pool, varyk_std::Error>`,
`Result<u64, ..>`, `Result<bool, ..>`), or one of the shapes of section
2.1. Varyk has no `()`, so `Result<(), ..>` stays V0108 as today; a
facade returns `bool` or a count where Rust would return nothing. Varyk
sees it as its own `Error` (M5a §2.3), so `?` works on the call's result
in a function returning `Result<_, Error>`, and `match` opens it with
`Err(e)` and reads `e.message()`.

`varyk_std::Error` in a parameter, a field, or elsewhere waits for a
package that needs it (section 10). A bare `Error` from a `use` keeps the
"write the full path" refusal of M3 §4.

A program holding such a result already needs `varyk-std` (`names_error`,
M5b2 §7.4, counts every local and return). A call to an imported
signature naming any `varyk_std::` path of this section (`Error`,
`Value`, the serde bound) counts too, whether the signature is in the
program's own `.rs` module or reached through a dependency package,
because the call site writes `::varyk_std::Value::from` and `T`'s serde
derive in the program's crate: a package without the line gets V0404's
help (M5a §5.1), and a single file's manifest gains it (M5a §5.2).

### 2.5 `pub use`

A `.vr` file may re-export one item of its own package:

```varyk
// src/lib.vr of varyk-sql
pub mod db;
pub use db::connect;
pub use db::connect_with;
pub use db::Pool;
pub use db::Tx;
```

The rules:

- the path names a function, struct, or enum (not a module) of the same
  package, through `pub` modules and items, declared in a `.vr` file or
  imported from a `.rs` module of the package; `crate::` and `self::` work
  as in a `use` (M3 §3.3);
- the item then has two names: its own path, and the module the `pub use`
  is in, so a program using the package writes `sql::connect(url)` and
  `sql::Pool` (and may still write `sql::db::connect`); the generated
  Rust keeps writing the item's own path, `::sql::db::connect`, which is
  valid since every module on it is `pub`;
- a name already declared or imported in the module is V0103, as for a
  `use`;
- an item that is not `pub` all the way is V0105, and the visibility rule
  of M5b2 §2.3 is unchanged: the module the item comes from is `pub`, as
  `pub mod db;` above, so every type the item names stays visible to
  outside users;
- `pub use` of a module, with braces, with a glob, with `as`, or of an
  item of another package is V0001, "not supported yet".

A plain `use` is unchanged.

### 2.6 `varyk add sql`

`varyk add` (M5a §5.2, M5b2 §4.6) is a pass-through to `cargo add`. It
gains a table of shorthands for official packages, with one entry for
now (`http` joins it in 5b4):

| Shorthand | Runs |
|---|---|
| `sql` | `cargo add varyk-sql --rename sql` |

The rename makes the dependency key `sql`, so code writes `sql::connect`.
(Without it the key would be `varyk-sql`, which begins with `varyk_` and
cannot be named, M5b2 §2.1; the V0100 help already shows the rename.)

The rule: when the first argument is a shorthand, it is replaced by its
row, and every later argument goes to cargo as written (`varyk add sql
--features postgres`), except another shorthand, which is refused with
"add one official package per `varyk add` call". When the first
argument is not a shorthand, the whole call is passed through exactly as
today, so `varyk add varyk-sql --rename sql` still works.

### 2.7 Not in milestone 5b3

A type parameter in a parameter position (`&T: Serialize`), two type
parameters, a `where` clause, or another bound; `Value` variants for
`u64`, lists, and maps; a struct or list as a trailing value;
`varyk_std::Error` outside a returned `Result`; a Varyk function with a
literal-only or variadic parameter; `pub use` of a module, with braces,
globs, or `as`, or across packages; several shorthands in one `varyk
add` (5b4's `varyk add http sql` lifts this, as one `cargo add` run per
shorthand, since `--rename` takes one crate); naming `Value` from Varyk
code; everything of 5b4.

## 3. Safety

- **No conversion can fail.** `Value::from` is total for every type
  section 2.2 admits, so the generated call has no error path that Varyk
  did not write. This is why the list is scalars only.
- **Literal-only text is checked at the call site**, where the program's
  input is visible to the compiler, not inside the package. A query that
  is built from input does not compile; the values go after the text, and
  the database binds them.
- **The borrow rules hold across the new shapes.** A trailing argument is
  read, never moved; the result of a `T`-returning call is a new value,
  owned by the caller, since `DeserializeOwned` gives an owned `T`.
- **Nothing new is Rust-visible from Varyk.** `Value` is not a Varyk type
  and cannot be named, held, or returned; a package's `.rs` sees it, a
  program never does.

## 4. Generated Rust

- A call with a type parameter writes it in a turbofish after the name:
  `::sql::Pool::one::<User>(&db, "...", vec![..]).await`.
- Trailing values become one `vec![..]` of `::varyk_std::Value::from(..)`
  calls, an empty `vec![]` when there are none.
- A literal-only argument is the literal, as any string literal is
  written today.
- A `pub use` is `pub use crate::db::connect;` in the package's generated
  root, so the generated crate's API matches Varyk's view for Rust
  users; a Varyk path through the re-export is written as the item's own
  path (`::sql::db::connect`), as every path is today.
- The error type needs nothing: `varyk_std::Error` is already the Rust of
  `Error`.

## 5. `varyk-std`

One addition, `varyk_std::Value` (section 2.2), in a new module
`value.rs`, re-exported at the crate root beside `Error`. `From` impls
for `bool`, `&str`, `f32`, `f64`, `i8`, `i16`, `i32`, `i64`, `u8`,
`u16`, `u32`, and for `Option` of each of those (`Option<&str>`
included): exactly what the backend writes, nothing more. `Value`
derives `Debug`,
`Clone`, and `PartialEq`. Nothing else in `varyk-std` changes; no new
dependency.

## 6. Compiler changes

Line numbers are approximate.

### 6.1 Interop (`crates/varyk/src/interop/`)

- `items.rs` (~441-457): a single type parameter with the bound of
  section 2.1 is recorded on the signature instead of making the return
  `Opaque`; any other generic shape keeps `Opaque` with a reason for the
  note.
- `signatures.rs` (`map_reference` ~305, `map_value` ~335,
  `generic_std` ~258): `&'static str` as a new `RustTy::Literal` in
  parameter position; `Vec<varyk_std::Value>` as `RustTy::Values` in last
  position; the path `varyk_std::Error` (and `varyk_std::serde::..` for
  the bound) as a known path, mapped to `Error`; the type parameter's
  name as `RustTy::Param` where it appears.
- `mod.rs` (~56): the `.rs` file's `pub use` is still skipped (REEXPORT);
  nothing changes there.

### 6.2 Resolve

- `resolve/signatures.rs` (`Mapper::sig` ~28, `param` ~85, `ret` ~96,
  `uncallable_note` ~214): `Literal` becomes a `string` parameter with a
  literal-only flag (not a new `ParamMode`, which borrow analysis and the
  backend share); `Values` becomes a variadic marker on the
  signature; `Param` in the allowed return shapes gives a signature with
  a type hole filled at the call; the notes of sections 2.1-2.3 for the
  refused shapes.
- `resolve/uses.rs` (`register` ~119, `resolve_use_path` ~399) and
  `varyk-syntax/src/parser/item.rs` (~121): `pub use` parsed as a `use`
  with a visibility, registered as a re-export of the item it resolves
  to; V0001 for the forms of section 2.5; the package's exported items
  (`resolve/mod.rs`, `ImportedSig` ~173) include re-exports under the
  re-exporting module's path.

### 6.3 Types

- `types/check.rs` (`arguments` ~1563) and `types/check/methods.rs`
  (~119): a literal-text parameter accepts only `ExprKind::String`
  (V0217); a variadic signature accepts any number of trailing
  arguments after the fixed ones, each checked against the type list of
  section 2.2 (V0218); the count error V0201 counts the fixed parameters
  only and says "at least".
- `types/check/values.rs` (`read_type` ~585, `convert_call` ~491): a
  call with a type hole takes `T` from `expected` as `json::parse` does,
  with the same V0207, and adds `T` to the read reach set (V0210 for a
  Rust or foreign type).
- `types/check/asyncs.rs` (`async_call` ~127, `await_expr` ~51): the
  expected type already reaches the call through `.await` and `?`
  (`methods.rs` `try_` wraps it in `Result`); a test pins it. A call
  with a type hole in started position is V0207 with the note of
  section 2.1.
- `names_error` (`check.rs` ~230): a call to an imported signature
  naming any `varyk_std::` path of section 2 counts, through a
  dependency package as well (section 2.4).
- HIR: the call records the chosen `T`, the split between fixed and
  trailing arguments, and each trailing argument's scalar type.

### 6.4 Backend (`crates/varyk/src/backend/rust_expr.rs`)

- `callee` (~1180) writes the turbofish from the recorded `T`, using
  `rust_type`.
- `args` (~768) writes the trailing `vec![..]` of `Value::from`, passing
  a string argument as `&str` (`Need::Str`, as today), an `Option<string>`
  as `.as_deref()`, and the rest by value (section 2.2).
- `backend/rust.rs` writes `pub use crate::..;` for each re-export in a
  module (section 4); paths are written as today.

### 6.5 Command line

- `cli.rs` (`run_add` ~798): the shorthand table and the refusal of
  section 2.6; the `cargo add` argument list is built by a pure function
  the tests call.

## 7. Diagnostics

New codes, each with a fixture, a registration in `tests/errors.rs`, and a
row in `docs/language.md`:

| Code | Meaning |
|---|---|
| V0217 | an argument to a literal-text parameter that is not a string literal |
| V0218 | a trailing value of a type that cannot be passed as a value |

Reused: V0108, with a new note, for a generic shape, a `&'static str`, a
`Vec<varyk_std::Value>`, or a `varyk_std::Error` where section 2 does not
admit it; V0207 for a type-parameter call with no expected type, a
started one included; V0210
for a `T` that is a Rust or foreign type; V0209 for the attribute checks
of a read type; V0201 for too few fixed arguments; V0001 for the `pub
use` forms of section 2.5; V0105 and V0103 for `pub use` visibility and
clashes.

## 8. Testing and definition of done

- Interop unit tests: each accepted signature shape of sections 2.1-2.4,
  and each refused one with its note.
- Resolver tests: `pub use` of a `.vr` item, of a `.rs` item, through
  `crate::` and `self::`; each V0001 form; V0103 on a clash; V0105 on a
  re-export of a non-`pub` item.
- Type tests: `T` from a `let`, a parameter, a return, a field, through
  `?` and `.await`, and V0207 without and for a started call; each
  trailing type, and V0218 for
  a struct, a `Vec`, and a `u64`; V0217 for a variable, a `format!`, and a
  parameter.
- `insta` snapshots of the generated Rust for a call with a turbofish,
  with zero and three trailing values, and of a package root with
  re-exports.
- Soundness templates (M2 §7): a trailing string argument still usable
  after the call; a `T` result owned and changeable.
- A fixture package under `crates/varyk/tests/fixtures/packages/store/`:
  a Varyk library whose `.rs` facade is an in-memory store with the
  shapes of `varyk-sql`'s `Pool` and `Tx` (a `T`-returning method in each
  of the three return shapes, a `&'static str` parameter, trailing values,
  `Result<_, varyk_std::Error>`, an async method, and a `mut self`
  method), one `#[test]` function, and `pub use` lines in its
  `src/lib.vr`; its facade reads `T` through `serde_json`, listed in its
  `Cargo.toml` as M3 requires of any crate a facade uses. Beside it,
  `fixtures/packages/store_user/` is a program with `store = { path =
  "../store" }` that builds and prints the expected output under `varyk
  run`, and `varyk test` in `store` runs its test, pinning `varyk test`
  on a library. Both run in `tests/examples.rs` from a copy of the two
  directories made together (an `example_dir` that takes a root), as the
  example packages do (M5b2 §9), since their `serde_json` and
  `varyk-std` lines need the registry and `tests/packages.rs` runs
  without the network. Both manifests carry a `varyk-std` line, so each
  gets an `extra-files` entry in `release-please-config.json`, as
  `route` and `trip` have, or the next release pull request breaks the
  test.
- `tests/cli.rs`: the `cargo add` arguments for `varyk add sql`, `varyk
  add sql --features postgres`, the refusal of a second shorthand, and a
  plain crate name, without running cargo.
- The language-reference test (M4 §7) covers V0217 and V0218.
- `docs/language.md` ("Calling Rust" gains the four signature shapes;
  "Modules" gains `pub use`; "Packages" gains the shorthand and points to
  `varyk-sql`), `docs/design.md`, `docs/open-questions.md`,
  `docs/roadmap.md`, and AGENTS.md (the second allocation, section 2.2)
  updated.
- CI green, including the 1.85 build; no `unwrap`, `expect`, or other
  crash-on-absence call added.

Done when every item of the roadmap's 5b3 list that belongs to this
repository is checked; `varyk-sql`'s own item is checked when its first
release is on crates.io.

## 9. Roadmap changes

The 5b3 list becomes:

- a `.rs` function or method with one type parameter standing for any
  Varyk data type, chosen from where the result goes;
- a `.rs` function whose last parameter takes any number of scalar values
  (`varyk_std::Value`);
- a `.rs` parameter of type `&'static str` taking only text written in the
  program;
- `varyk_std::Error` as the error type of a `.rs` function's result;
- `pub use` in a `.vr` file, and `varyk add sql`;
- `varyk-sql`, in its own repository, on sqlx: SQLite, Postgres, and
  MySQL, each a cargo feature; `connect` and `connect_with`, `migrate`,
  `begin` and `commit`, and on a pool or a transaction `one`, `first`,
  `all`, and `run`, each
  taking the query and its values; rows read into structs by column name;
  each database's own placeholders, passed through; the query text a
  literal, so a query built from input is a compile error; no secret in
  an error message.

The `Serialize` half of the first item moves to 5b4, where the HTTP
client needs it.

## 10. Open questions

Added to `docs/open-questions.md`:

- Should `Value` grow `UInt`, `List`, and `Map`, so a trailing value may
  be a `u64`, a `Vec` (Postgres `= ANY($1)`), or a struct (a JSON
  column)? 5b3 keeps scalars so that no conversion can fail.
- Should a Varyk function be able to declare a literal-only or a variadic
  parameter, so a `.vr` wrapper could forward them?
- Should the facade's type parameter be allowed in parameter position
  (`&T: Serialize`), and in other return shapes (`HashMap<String, T>`)?
  The first is planned for 5b4.
- Should `pub use` re-export a module, several names at once, or an item
  of another package? The last reopens V0115 (M5b2 §2.4).
- Should `varyk_std::Error` be accepted in parameters and fields, and
  should a facade be able to carry a status or kind on it? 5b4's
  `http::bad_request` is the first need.

## 11. Decisions

Taken with the user on 2026-10-03:

- `varyk-sql` lives in its own repository, released with release-please,
  not in this one.
- The variadic parameter is `Vec<varyk_std::Value>`; `Value` is scalars
  only, built by the compiler without serde, so nothing can fail and no
  failure is hidden as `Null`.
- The fixture package stays in the test tree, not in `examples/`; the
  user-facing example is `varyk-sql`'s own.
- A single pool setting, `max_connections`, through a second connect
  function; everything else in `varyk-sql` is described in its spec.
