# Varyk milestone 5a: data, config, logging, and tests

Date: 2026-09-30.

**Status.** Design, not yet implemented. Extends the milestone-1 to
milestone-4 specs, `2026-09-23-varyk-design.md` ("M1 §n"),
`2026-09-25-milestone-2-design.md` ("M2 §n"),
`2026-09-26-milestone-3-design.md` ("M3 §n"), and
`2026-09-29-milestone-4-design.md` ("M4 §n"). Their principles, ownership
rules, and compiler architecture stay in force. Varyk is experimental and
pre-1.0: anything here may change.

## 1. Summary and scope

The roadmap's milestone 5, batteries for services, is split in three
(section 10): **5a**, this spec, the synchronous batteries; **5b1**,
`async`/`await` and `spawn`; **5b2**, HTTP and the database. The golden
path of the roadmap, a users API on a database, is reached at the end of
5b2; 5a builds everything that path needs that is not concurrency or I/O.

Milestone 5a adds:

- a shared error type, `Error`, so `?` works across every standard call;
- JSON: `json::parse` and `json::stringify` on any struct whose fields
  allow it, with serde's traits derived by the compiler where they are
  used;
- configuration: `env::parse` fills a struct from the environment, with a
  `.env` file as the fallback;
- logging: `log::debug`, `info`, `warn`, `error`, on tracing, text or JSON
  lines;
- tests: `#[test]` functions, `assert` and `assert_eq`, and `varyk test`;
- `varyk add`, a pass-through to `cargo add`;
- four attributes, the first in Varyk: `#[rename]`, `#[default]`,
  `#[skip]`, and `#[test]`;
- `varyk-std`, a new crate in this workspace that the generated Rust uses
  for all of the above.

A **`varyk-std` call** in this spec is any call this milestone adds
(`Error::new`, `message`, the `json`, `env`, and `log` calls) and
`s.parse()`, whose `Error` is made by `varyk-std`. The table rows of
milestones 2 and 4 on `Vec`, `string`, and the rest are not: they stay
plain Rust. `assert` and `assert_eq` are not either; they become Rust's
own macros. A program **uses `varyk-std`** when it makes a `varyk-std`
call or names `Error` anywhere, in a signature, a field, or a type
argument; the generated Rust then refers to `varyk_std`.

Of the three questions milestone 5 was gated on (M1 §9), 5a answers serde
derivation (section 2.2). The async runtime shape and ownership-transfer
syntax are 5b1's and move there.

Two decisions shape the milestone. First, **the standard surface lives in
the standard table** (M4 §2.7). `json::parse` and `json::stringify` are
generic in Rust and Varyk has no generics, so they are table rows typed
from the expected type, exactly as `s.parse()` is; every other standard
call joins them, so the whole surface uses one existing mechanism. Second,
**`varyk-std` is the one crate Varyk code reaches without a facade**. The
M3 rule stands for every other crate: Varyk code never names one, and a
dependency is used from a `.rs` facade in the package.

Milestone 5a is an MVP like the four before it. The test for every item
was "does one of the examples in section 6 need it"; what failed is listed
in section 2.11. Only what this document lists is supported; anything else
is rejected with a diagnostic that names the construct.

## 2. Language surface additions

A program using all of it:

```
enum Role { Admin, Member }

struct User {
    id: i32,
    #[rename("userName")]
    user_name: string,
    role: Role,
    nickname: Option<string>,
    #[skip]
    password_hash: Option<string>,
}

struct Config {
    #[default(8080)]
    port: u16,
    database_url: string,
}

fn load(body: string) -> Result<User, Error> {
    let u: User = json::parse(body)?;
    if u.id < 0 {
        return Err(Error::new("id must not be negative"));
    }
    Ok(u)
}

fn main() {
    let loaded: Result<Config, Error> = env::parse();
    let cfg = match loaded {
        Ok(c) => c,
        Err(e) => {
            log::error("bad config: {}", e.message());
            return;
        }
    };
    log::info("listening on {}", cfg.port);
}

#[test]
fn parses_a_user() {
    let u = load("{\"id\": 1, \"userName\": \"ann\", \"role\": \"Admin\"}");
    assert(u.is_ok());
}
```

### 2.1 Lexical elements

`#` becomes a token. It was rejected by the lexer with "attributes (`#`)
are not supported in Varyk yet" and now starts an attribute, `#[name]` or
`#[name(literal)]` (section 2.2). `#!` and any other use of `#` stay
errors. `json`, `env`, `log`, and `Error` become reserved names (section
2.10). No new keywords.

### 2.2 Attributes

An attribute is `#[name]` or `#[name(literal)]`, written before a struct
field, an enum variant, or a top-level function, on its own line or on the
same line. Several may stack. There are four:

| Attribute | Where | Meaning |
|---|---|---|
| `#[rename("key")]` | struct field, unit variant | the name used in JSON and the environment, instead of the Varyk name |
| `#[default(literal)]` | struct field | the value used when the key or variable is missing |
| `#[skip]` | struct field | never written by `stringify`, never read by `parse` |
| `#[test]` | top-level function | a test, run by `varyk test` (section 2.7) |

Rules:

- Any other name (`#[derive(Clone)]`, `#[serde(..)]`, `#[inline]`) is an
  error naming the four; `derive` gets a note that `.clone()`, `==`, and
  JSON are automatic.
- An attribute in any other place (on a struct, an `impl` method, a data
  variant, a field of a variant) is an error saying where it may go. The
  same attribute twice on one item is an error.
- `#[rename]` takes a non-empty string literal. After renaming, the keys of
  the fields that are not skipped, or of the variants, of a type that a
  `json` or `env` call reaches must be unique.
- `#[default]` takes an integer, float, string, or `bool` literal, a
  number optionally preceded by `-`, that fits the field's type, which
  must be a number, `string`, or `bool`. It is an error on an `Option`
  field, which is already `None` when missing, and on any other type. A
  string default is a literal placed into an owned slot, the one
  allocation the compiler inserts (M1 §4.3).
- A `#[skip]` field that a `parse` reaches must have a `#[default]` or be
  an `Option`; `#[default]` on a skipped field gives its value on parse.
- The attributes on a type that no `json` or `env` call reaches have no
  effect and are not an error; they are still checked for placement and
  literal fit.

### 2.3 `Error`

`Error` is a built-in type: a message.

| Call | Types | Meaning |
|---|---|---|
| `Error::new(text)` | `text: string`, owned | a new error |
| `e.message()` | returns `string`, part of `e` | the message |

`{}` in `println!`, `format!`, and the `log` calls prints the message.
`Error` can be cloned and compared with `==`, so a struct holding one
keeps its derives (M4 §2.10). It cannot be turned into JSON.

Every failing `varyk-std` call returns `Result<T, Error>`. The `?` rule
of M2 §2.8 is unchanged: `?` on a `Result<_, Error>` needs a function
returning `Result<_, Error>`. A function's own error enum converts with
`map_err` and a function of the writer's that gives its text
(`r.map_err(|e| Error::new(describe(e)))`, where `describe` matches on
the enum), since `{}` does not print an enum; there is no automatic
conversion.

### 2.4 JSON

| Call | Types | Meaning |
|---|---|---|
| `json::parse(text)` | `text: string`, read; returns `Result<T, Error>` | reads `T` from JSON text |
| `json::stringify(value)` | `value: T`, read; returns `string`, new | writes `value` as compact JSON |

`T` for `json::parse` comes from the expected type, in every position
where M4 §2.6 and §2.7 give one to `s.parse()`: a typed `let`, an argument,
a return, a field, and through `?`. The head of a `match` and the receiver
of a method are not among them, so a result opened with `match` is bound
first (`let r: Result<User, Error> = json::parse(body);`). Without an
expected type it is V0207, whose help shows `let u: User =
json::parse(..)?;`.

`T` must be convertible (section 2.9). The mapping:

- a struct is an object; keys are field names or their `#[rename]`, in
  declaration order on write; unknown keys are ignored on read;
- a missing key is `None` for an `Option` field, the `#[default]` value if
  there is one, and otherwise an error naming the key;
- `None` is written as `null`, and `null` reads as `None`;
- a unit-only enum is a string, the variant name or its `#[rename]`;
- `Vec` is an array, `HashMap<string, V>` an object;
- numbers, `bool`, and `string` are themselves; a number out of range for
  its type is an error on read.

`stringify` cannot fail on a convertible type, so it returns a `string`,
not a `Result`. Pretty printing is not in 5a.

### 2.5 Configuration

| Call | Types | Meaning |
|---|---|---|
| `env::parse()` | returns `Result<T, Error>` | reads `T` from the environment |

`T` comes from the expected type, as in section 2.4. It must be a struct
whose fields, except skipped ones, are numbers, `bool`, `string`,
unit-only enums, or `Option`s of those; a nested struct, `Vec`, or
`HashMap` field is an error naming the field.

Each field is read from the variable named by the field's key (its name,
or its `#[rename]`) in upper case: `database_url` reads `DATABASE_URL`.
Two fields whose variables would be the same are an error. For each
field, in order:

1. a variable set in the process environment wins;
2. otherwise, the value in `.env`, if the file has the variable;
3. otherwise, the `#[default]` value, `None` for an `Option`, or an
   error naming the variable ("`DATABASE_URL` is not set").

A value is read as its field's type: a number from its text, a `bool`
from `true` or `false`, a unit enum from its key, a `string` as written.
A value that does not read is an error naming the variable and the value
("`PORT` is not a number: `abc`").

`.env` is read from the current directory, once, on first use. It is a
list of `KEY=value` lines; blank lines and lines starting with `#` are
skipped, and a value may be wrapped in single or double quotes. A missing
file is not an error; a malformed line is, naming the line. `.env` is
never written into the process environment. Variable expansion
(`${OTHER}`) and several files (`.env.local`) are not in 5a.

### 2.6 Logging

| Call | Meaning |
|---|---|
| `log::debug(format, args..)` | a debug line |
| `log::info(format, args..)` | an info line |
| `log::warn(format, args..)` | a warning line |
| `log::error(format, args..)` | an error line |

The format string and arguments follow `println!` exactly (M1 §4.1),
including V0202 for a placeholder count that does not match. The calls
return nothing and read their arguments.

Lines go to stderr. `LOG` and `LOG_FORMAT` are read like any variable
(section 2.5, steps 1 and 2). `LOG` sets the level, `debug`, `info`,
`warn`, `error`, or `off`, and `info` when unset. `LOG_FORMAT=json` writes
one JSON object per line instead of text; any other value is text. A text
line, without colour, is the time, the level, and the message; a JSON line
has the keys `time`, `level`, and `message`.

Logging is set up by the generated `main` of a program that makes a `log`
call (section 7.5). Setup never stops the program: a bad `LOG` value or a
`.env` that cannot be read gives one warning line on stderr, and logging
goes on at `info` in text. `env::parse` still reports the bad `.env` as
its `Error`. A library's `log` calls go wherever the program using it
sends them, and nowhere under plain Rust without a subscriber.

### 2.7 Tests

A top-level function marked `#[test]` is a test. It takes no parameters,
returns nothing, and is not called from anywhere; each of these is an
error. It may be `pub` or not, in any module of the program.

Two calls exist only inside a test:

| Call | Types | Meaning |
|---|---|---|
| `assert(cond)` | `cond: bool` | the test fails if `cond` is false |
| `assert_eq(a, b)` | `a`, `b` of one type that `==` accepts | the test fails if `a != b` |

Outside a `#[test]` function both are an error: a call that stops the
program has no place in service code (the M4 principle behind leaving out
`unwrap`). A failure names the source line; `assert_eq` also shows both
values when their type prints with `{}`.

`varyk test [file]` builds the program as tests and runs them, for a
package or a single file, as `varyk build` does. Its output and exit code
are cargo's test runner's, with Rust errors mapped as in `varyk build`
(M3 §5).

### 2.8 `parse`

`s.parse()` returns `Result<T, Error>` instead of `Option<T>`; the types
it reads and where `T` comes from are unchanged (M4 §2.7). The error's
message names the text and the type ("`abc` is not a number", "`yes`
is not `true` or `false`"). `?` now
works on it in a function returning `Result<_, Error>`. The old `Option`
is two statements, since an expected type does not reach a method's
receiver: `let r: Result<i32, Error> = text.parse(); let n = r.ok();`.
This is a breaking change.

### 2.9 Convertible types

A type is convertible, for `json` in the direction it is used, when it is:

- a number type, `bool`, or `string`;
- `Option<T>` or `Vec<T>` of a convertible `T`;
- `HashMap<string, V>` of a convertible `V`;
- a Varyk struct whose fields, except skipped ones, are convertible;
- a Varyk enum whose variants all carry no data.

Anything else is an error at the `json` or `env` call, naming the type and
the part in the way: an enum with data (the note suggests a unit enum
field named `type` with `#[rename("type")]` and `Option` fields for the
data), a `HashMap` with a non-`string` key, `Error`, or a Rust type from a
`.rs` module. The check follows nested types, including a type that
contains itself through a `Vec`.

### 2.10 Reserved names

`json`, `env`, and `log` cannot name a module, a struct, or an enum (a
struct `json` with associated functions would be taken for the module's
`json::` paths), and `Error` cannot name a
struct, enum, or imported Rust type, and `assert` and `assert_eq` cannot
name a function, anywhere in the program: `mod json;`, a module file
`json.vr` or `json.rs`, and `pub struct Error` in a `.rs` module are
errors. The standard modules are reached only by a path at the call,
`json::parse(..)`; `use json::parse;` and `use json;` are errors too.
Local variables and fields may use the names; `log::info(..)` is a path
and never a local.

No function, method, struct, enum, module, or `use` name may start with
`varyk_`: the prefix is kept for what the compiler adds to the generated
Rust, such as the associated function `<Type>::varyk_default_<field>`
behind a `#[default]` (section 7.5).

### 2.11 Not in milestone 5a

TOML; pretty JSON; JSON for enums with data; `flatten`, aliases, and
custom formats; structured log fields (`log::info("x", id = 1)`); log
targets and spans; `.env` variable expansion and several `.env` files;
kinds or a cause on `Error`; automatic conversion into `Error` from other
error types; attributes on structs, methods, and variants with data;
`#[default]` on `Option`, enum, or struct fields; `varyk_std::Error` in
`.rs` signatures (section 10, with the 5b2 interop item); async, HTTP, and
the database.

## 3. Ownership rules

No new rules. The calls fit the existing ones:

- `json::parse`, `json::stringify`, `e.message()`, the `log` calls, and
  `assert`/`assert_eq` read their arguments, as `println!` does.
- `Error::new` takes an owned `string`: a literal allocates under the
  literal rule, a stored `string` moves.
- `e.message()` returns part of `e`, an alias under M2 §3.1, as a
  borrowed-return call does (M4 §3.1).
- `json::parse`, `env::parse`, and `s.parse()` return new values.

The compiler inserts no allocation beyond the literal rule: the JSON
text, the parsed values, and the log lines are made by `varyk-std` and
serde, as a facade's results are.

## 4. The `varyk-std` crate

`crates/varyk-std`, a new library crate in the workspace, published with
`varyk` and `varyk-syntax` under one version (section 5.1). It holds:

- `Error`: a message, with `Display`, `Clone`, `PartialEq`, and the
  conversions from the errors of serde_json, env reading, and `parse`;
- `json::{parse, stringify}` on serde_json;
- `env::parse` and the `.env` reader (section 2.5), filling the struct
  through serde without writing to the process environment;
- `parse`, the `s.parse()` wrapper that makes its `Error`;
- `start()`, which sets up tracing-subscriber from `LOG` and `LOG_FORMAT`
  (section 2.6);
- re-exports of `serde` and `tracing`, so the generated Rust names only
  `varyk_std`.

Its dependencies are serde, serde_json, tracing, and tracing-subscriber;
anything else is chosen in the plan and must be justified by a row here.
It obeys the repository rules, the MSRV among them, and never calls
`unwrap` or `expect`.

## 5. Packages and the dependency

### 5.1 Versioning

`varyk-std` always has the compiler's version: release-please links the
three crates. A package uses it as a plain dependency:

```toml
[dependencies]
varyk-std = "0.3"
```

When the program uses `varyk-std` (section 1), `varyk check` requires:

- `varyk-std` in `[dependencies]`, as a version string or a table with a
  `version` and no `path` or `git`, whose requirement is `0.MINOR` or
  `0.MINOR.PATCH`, optionally with a leading `^`, and whose `MINOR` is the
  compiler's: `"0.3"`, `"0.3.1"`, or `"^0.3"` for compiler 0.3.x. Any other
  requirement (`~`, `=`, `>=`, `*`, a list) is refused, as are `optional`
  and `package` on it, since either leaves no `varyk_std` to build with;
- if `Cargo.lock` (M3 §2.2) locks `varyk-std`, a version no older than the
  compiler's.

Each failure is V0404, and its help is the fix: the line to write, or
`cargo update -p varyk-std`. Upgrading Varyk is `cargo install varyk` and
then whatever `varyk check` asks for. A package that depends on
`varyk-std` without using it is fine.

### 5.2 Single files and new packages

A single file has no `Cargo.toml`, so when it uses `varyk-std` its
generated manifest depends on `varyk-std` at exactly the compiler's
version (`"=0.3.0"`); a file without one has no dependency, as today.
This keeps the M3 rule: a package's crates use the package's own
manifest; a single file has none.

`varyk init` writes the `varyk-std` line into the new `Cargo.toml`, with
the compiler's full version (`varyk-std = "0.3.0"`), and adds `.env` to
the new `.gitignore`.

`varyk add` runs `cargo add` with its arguments in the package directory
and passes its output and exit code through.

### 5.3 Building this repository

Until a version is published, the examples and tests must build against
`crates/varyk-std` in this workspace. When `VARYK_STD_PATH` is set, the
driver makes the generated crate's `varyk-std` dependency a `path`
dependency on that directory, for single files and packages alike;
`varyk publish` never does. The test suite sets it; it is not documented
for users. The tests that run plain cargo on a package from `varyk init`,
in `packages.rs` and in `examples.rs` (`cargo_in`, used on `greeting`),
pass cargo `--config patch.crates-io.varyk-std.path="…"` instead, since
the driver is not involved there. The soundness harness
(`tests/soundness.rs`), which writes its own manifest, gives it a
`varyk-std` path dependency on `crates/varyk-std`.

Releasing: `varyk-std` joins `release-please-config.json` as a package and
a member of the `linked-versions` group, `.release-please-manifest.json`,
and the publish steps of `.github/workflows/release-please.yml`, published
first since nothing depends on it at build time.

The example packages that carry a `varyk-std` line, `users` and
`greeting` (which must equal what `varyk init` writes, by
`greeting_keeps_the_files_varyk_init_writes` in `examples.rs`), have the
full current version (`"0.2.0"` until the next release) and are listed in
`extra-files` with release-please's `toml` updater (`jsonpath:
"$.dependencies.varyk-std"`), which needs no marker comment in the file,
so the release pull request bumps them with the crates; the plan confirms
the paths and the bump with a release-please dry run. Neither commits a
`Cargo.lock`: `greeting`'s is removed, since a committed lock goes stale
at every bump, and for `users` would fail the lock check. The
`release-as: 0.2.0` pin, left from the last release, is removed so the
next one can bump.

## 6. Milestone-5a examples

New single files, each with its expected output in `tests/examples.rs`:

- `examples/json.vr`: a `User` with `#[rename]`, `#[skip]`, `#[default]`,
  an `Option` field, a unit enum, and a `Vec`; `json::parse` of a literal,
  field reads, `json::stringify`, and a parse error printed with `{}`.
- `examples/config.vr`: `env::parse` into a `Config` with a `#[default]`,
  an `Option`, and a unit enum; run by the harness with its variables set.
- `examples/logging.vr`: the four levels with placeholders; run with
  `LOG=debug`, stderr compared with the time removed from each line.

New package `examples/packages/users`: an in-memory users store, the
5a stand-in for the golden path, with no committed `Cargo.lock` (section
5.3) and a `.gitignore` without `.env`. `src/main.vr` reads a `Config` with
`env::parse` from a `.env` committed in the package (the test runs
`varyk run` with the package as its current directory), parses users from
JSON, rejects an invalid one with an `Error`, logs, and prints the store
as JSON; `src/store.vr` holds the store and its `#[test]` functions, run by
`varyk test` in the test suite.

Updated for section 2.8: every example that calls `parse`
(`text.vr`, `readings.vr`, and any other).

## 7. Compiler changes

Line numbers are approximate.

### 7.1 Syntax (`varyk-syntax`)

- `lexer.rs` (~416): `#` becomes a token.
- `parser/item.rs`: an attribute list before a top-level item, an `impl`
  method, a struct field, an enum variant, and a field of a variant; each
  attribute keeps its name, optional literal, and span. Placement is the
  resolver's to check.
- The AST gains the attribute list on those five nodes.

### 7.2 Resolve

- Attribute names and placement (section 2.2) and the `#[test]` shape and
  calls (section 2.7), V0112 and V0114.
- The reserved names of section 2.10, V0113.
- `json::`, `env::`, and `log::` paths and `Error` resolve to the
  standard table.

### 7.3 Types

- `builtins.rs`: rows for `Error::new`, `message`, `json::parse`,
  `json::stringify`, `env::parse`, the four `log` calls, `assert`, and
  `assert_eq`; `s.parse()` returns `Result<T, Error>`. Module calls are a
  new owner beside `Vec`, `string`, and the rest.
- `Ty` gains `Error`.
- `types/derives.rs` gains the convertibility judgement (section 2.9) and
  two collected sets, the types that need `Serialize` and those that need
  `Deserialize`, following nested fields from each `json` and `env` call.
- The attribute literals, `#[skip]` on parsed types, and unique keys
  (section 2.2), V0209; unconvertible types, V0210.

### 7.4 Borrow analysis

Only table rows (section 3); no new analysis.

### 7.5 Backend

- Standard calls as absolute `::varyk_std` paths, so a module of the
  program named `varyk_std` cannot shadow them, with the type written
  where Rust needs it: `::varyk_std::json::parse::<User>(&body)`.
- Derives of `Serialize` and `Deserialize` only on the collected types,
  through `::varyk_std::serde` with
  `#[serde(crate = "::varyk_std::serde")]`;
  `rename`, `skip`, and `default`, the last through a small generated
  function per field.
- `log` calls as `::varyk_std::tracing` macros; `::varyk_std::start();` as
  the first statement of `main` when the program makes a `log` call, so a
  program without one writes nothing extra to stderr.
- `#[test]` passes through; `assert` becomes `assert!(cond, ..)` and
  `assert_eq` becomes `assert!(a == b, ..)`, since Varyk types have no
  `Debug`. The message names the Varyk file and line of the call, and for
  `assert_eq` both values when they print with `{}`.

### 7.6 Driver and package

- `package.rs`: the V0404 checks of section 5.1.
- `driver/`: `varyk test` (cargo test in the generated crate, errors
  mapped as for build), `varyk add`, the single-file dependency, and
  `VARYK_STD_PATH` (section 5).
- `driver/init.rs`: the `varyk-std` line in the manifest it writes;
  `driver/templates/gitignore.txt`: `.env`.
- `cli.rs`: the `test` and `add` commands.
- Release files (section 5.3): `release-please-config.json`,
  `.release-please-manifest.json`, `.github/workflows/release-please.yml`,
  and the workspace `Cargo.toml` members.

## 8. Diagnostics

New codes, each with a fixture, a registration in `tests/errors.rs`, and a
row in `docs/language.md`:

| Code | Meaning |
|---|---|
| V0112 | an unknown attribute, one in a place it cannot go, or the same one twice |
| V0113 | a module, struct, or enum named `json`, `env`, or `log`, a type named `Error`, a function named `assert` or `assert_eq`, a `use` of a standard module, or a function, method, struct, enum, module, or `use` name starting with `varyk_` |
| V0114 | a `#[test]` function with parameters or a return type, called or imported with `use`, or named `main` in the entry file; `assert` or `assert_eq` outside a test |
| V0209 | a `#[rename]` or `#[default]` literal that does not fit, `#[default]` on a field type it cannot go on, a parsed `#[skip]` field with no default, or two keys or variables that are the same |
| V0210 | a type that cannot go through `json` or `env` at this call, naming the part in the way |
| V0404 | the `varyk-std` dependency missing, from a `path` or `git` source, of another minor version, or locked older than the compiler |

Reused: V0202 for `log` placeholders; V0203, narrowed so `{}` accepts
`Error`, and with V0200 for `assert_eq` operands; V0206 for `?` with
another error type; V0207 for `json::parse` and `env::parse` with no
expected type.

Headlines in plain words, for example V0210: "`Payment` cannot be turned
into JSON", with the label "`Card` carries data" and the note on the unit
enum pattern.

## 9. Testing and definition of done

- Unit tests for each table row, the convertibility judgement and its
  collected sets, attribute placement and literals, and the V0404 checks.
- `insta` snapshots of the generated Rust: derives with `rename`, `skip`,
  and `default`; `start()`; typed `parse` calls; `assert_eq`.
- Soundness templates (M2 §7) for every new call.
- `varyk-std` unit tests: the `.env` reader, the lookup order, key upper
  casing, value errors, and the log level and format choice, without
  touching the process environment.
- Integration: `varyk test` on `users` passing, and on a fixture package
  with a failing test, exit code and output checked; `varyk add` of a path
  dependency to a temporary package with no `varyk-std` line, offline
  (`cargo add` resolves the whole graph, and `varyk-std` is not published
  yet).
- The milestone-4 follow-ups this milestone fixes: `parse` returning a
  `Result`, and the three `packages.rs` tests that fail when
  `CARGO_TARGET_DIR` is exported. Other follow-ups only if an example here
  hits them.
- `docs/language.md` gains sections on attributes, `Error`, JSON,
  configuration, logging, and tests, and the new codes;
  `docs/open-questions.md`, `docs/roadmap.md` (section 10), `README.md`,
  and `AGENTS.md` (the new crate) are updated.

Done when every example in section 6 and every earlier example builds and
prints its expected output, `varyk test` passes on `users`, and the gate
of `AGENTS.md` passes on stable and 1.85.

## 10. Roadmap changes

Milestone 5 becomes three milestones; the golden-path bar stays with the
whole of milestone 5 and is met at the end of 5b2.

- **5a, data, config, logging, and tests** (this spec): `varyk-std`,
  serde derivation, JSON, configuration from the environment, logging,
  `varyk test`, `varyk add`.
- **5b1, async**: the async runtime shape and ownership-transfer syntax
  gates; `async`/`await` on a built-in tokio runtime; `spawn`; `Send`,
  `Sync`, and `Pin` kept out of the surface.
- **5b2, HTTP and the database**: HTTP server and client, databases
  through one API, `.rs` signatures naming Varyk-declared types and
  `varyk_std::Error`, and the agent evaluation.
- **Unscheduled:** TOML, beside the rest of section 2.11.

## 11. Open questions

Answered, marked so in `docs/open-questions.md`:

- "Should serde derivation be automatic for every struct, or declared with
  attribute syntax?" and its milestone-4 companion: derived by the
  compiler only for the types a `json` or `env` call reaches; the
  attributes adjust names, defaults, and skipping, never whether to
  derive.
- "Should `parse` return a `Result` once milestone 5 has a shared error
  type?" Yes (section 2.8).
- "What error-handling ergonomics beyond `Result` and `?` are worth
  adding?" Partly: one `Error` type with a message.

Added:

- Should JSON support enums with data, and in which shape? 5a refuses
  them and points to a `type` field.
- Should `Error` carry a kind or a cause, and should other error types
  convert into it automatically at `?`?
- Should Varyk grow more attributes, and should they ever be namespaced?
  5a's four are unprefixed because only the compiler defines attributes.

Moved to 5b1: the async runtime shape and ownership-transfer syntax.

## 12. Decisions

Appended to the decisions log of M1 §10.

| Decision | Choice | Why |
|---|---|---|
| Milestone 5 split | 5a data, config, logging, tests; 5b1 async; 5b2 HTTP and database | three subsystems; each slice testable on its own |
| Standard surface | rows in the standard table, `varyk-std` as the implementation | the calls are generic or format-checked, which the importer cannot read; one existing mechanism |
| Crate access | `varyk-std` reached without a facade; every other crate still through one | the M3 rule stays meaningful; the standard modules are Varyk's own |
| Names | `json::parse` and `json::stringify` | JavaScript's names; `parse` already means text into a value |
| Error | one built-in `Error` with a message | `?` across every standard call without conversions, as Go and JavaScript have one error type |
| Serde derivation | derived only where a `json` or `env` call reaches, per direction | no serde in programs that do not use it; the error lands at the call |
| Attributes | `#[rename]`, `#[default]`, `#[skip]`, `#[test]`, unprefixed, a fixed list | real APIs need renamed and keyword keys, config needs defaults, secrets need skipping; only the compiler defines attributes |
| Enums in JSON | unit-only, as strings; data enums refused | a designer writes a `type` field explicitly; no Rust-shaped format leaks into APIs |
| Configuration | `env::parse` into a struct; environment, then `.env`, then default | Node dotenv's order; one mechanism with JSON; `.env` never written to the process |
| Logging | four calls in `println!` form on tracing; `LOG`, `LOG_FORMAT=json` | readable in a terminal, ingestible in production, no code change between them |
| Tests | `#[test]` functions in place; `assert` only in tests | maps to Rust's `#[test]` and `cargo test`; stopping the program is for tests only |
| Versioning | a version range in `Cargo.toml`, minor and lock checked by `varyk check` | upgrades are `cargo update`; a mismatch is a diagnostic, never a rustc error |
| Single files | depend on `varyk-std` at the compiler's exact version | there is no manifest to respect |
| TOML | not in 5a | nothing on the golden path needs it; the same machinery adds it later |
