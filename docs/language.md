# The Varyk language

This page describes everything Varyk accepts today, in milestone 5b1 of an
experimental, pre-1.0 language (see [roadmap.md](roadmap.md) for what comes
next). Anything not described here is rejected with an error that names
what is not supported. For the reasons behind the design, see
[design.md](design.md).

Varyk code is compiled to Rust. You do not need to know Rust to read this
page. Notes marked "For Rust readers" say what the generated Rust looks like
and can be skipped.

## Files and modules

A Varyk program is a file ending in `.vr`. This is the entry file, and it
must define a function called `main` that takes nothing and returns nothing.
The program starts there.

```varyk
// main.vr
fn main() {
    println!("Hello, world!");
}
```

A program can also be a package: a directory with a `Cargo.toml` (the file
Rust's build tool, cargo, reads) and a `src/` directory whose entry file is
`src/main.vr` (a program) or `src/lib.vr` (a library), never both. A library
must not define `main` and cannot be run. `Cargo.toml` says the package's
name (ASCII letters, digits, `-`, and `_`, starting with a letter or `_`, not
`cache` or `package`, which Varyk's build directories use, and for a program
not `deps`, `examples`, `build`, or `incremental` in any case, which cargo's
do) and version, and `edition = "2024"` is required:

```toml
[package]
name = "greeting"
version = "0.1.0"
edition = "2024"

[dependencies]
regex = "1"
```

`[dependencies]` names Rust crates the package uses, and its `.rs` modules
can call them (Varyk code cannot, see `use` below). A `path` to a crate on
disk may be relative to `Cargo.toml`. Taking a setting from a Cargo
workspace (`workspace = true`), dependencies for only some platforms
(`[target.'cfg(..)'.dependencies]`), and any Cargo target table (`[lib]`,
`[[bin]]`, `[[test]]`, ...) are errors: a package has the one root cargo
finds by its defaults. `check` reads a fixed set of `Cargo.toml` keys, the ones a
small service needs (the tables `package`, `workspace`, `dependencies`,
`dev-dependencies`, `build-dependencies`, `target`, `features`, `profile`,
`patch`, `replace`, `lints`, and `badges`, and under `[package]` the
standard keys cargo documents, apart from the target-layout ones, `auto*`
and `default-run`); any other key is V0401,
"not supported yet", so nothing unknown can pass `check` and surprise
`build` or `publish`. Also errors are `links`, which needs a build script
the published crate does not carry, a `build` script other than the `build.rs` that `varyk init` writes (which only includes the generated Rust; a `build.rs` with more in it is not noticed, and the crates Varyk builds do not run it) or `build = false`, which would keep plain `cargo build` from running that script, `[lints]`, which would apply
to the generated Rust too (put `#[deny(..)]` or `#[warn(..)]` attributes in
the `.rs` file instead), and `[patch]` or `[replace]`, in the package or in
the root manifest of an enclosing workspace, which would not apply to the
crate `varyk build` makes. That crate, and the one `varyk publish`
assembles, are built with your `Cargo.toml` as it is, apart from what the
generated layout needs: `build = false`, no `autobins`, `autolib`,
`include`, `exclude`, `default-run`, or `workspace` key, an empty
`[workspace]` table, and relative `path`s in the dependency tables made
absolute. So `[features]`, `[profile.*]`, `rust-version`, and the rest mean
to Varyk exactly what they mean to cargo. For a workspace member (an
ancestor `Cargo.toml` with a `[workspace]` table that does not `exclude` the
package, or the directory `[package] workspace` names), the root's
`Cargo.lock`, `[profile.*]`, and `resolver` are used, as cargo uses them. `check` reads `[dev-dependencies]`
only for its names (a `.rs` module using one is told to move it to
`[dependencies]`). A `Cargo.lock` beside `Cargo.toml` is
used when building, so Varyk builds the same crate versions cargo would.

Which one a file is follows one rule: look for `Cargo.toml` in the file's
directory and then upward; if the file is that package's `src/main.vr` or
`src/lib.vr`, it is the package, and anything else is a program of its own.
So `varyk run src/main.vr` inside a package builds the package, and
`varyk run examples/hello.vr` inside a Rust project stays a single file.

A program can be split into modules. `mod math;` in the entry file loads the
module `math` from `math.vr` or `math.rs` in the same directory, or from
`math/mod.vr`. A `.rs` file is Rust code; see "Calling Rust" below. Having
more than one of these files is an error, and a module cannot be called
`main` or `lib`. A module of the entry file cannot be called `bin` either,
because cargo treats the files in `src/bin/` as extra programs; a `bin`
further down is fine. These names are refused in any capitalization
(`Main`, `LIB`, `Bin`), since on a file system that ignores case, as macOS
and Windows do by default, `Main.rs` is the same file as `main.rs`.

Any `.vr` module can declare modules of its own, to any depth. The files of
a module's modules live in a directory named after it: `mod cart;` in
`shop.vr` loads `shop/cart.vr` or `shop/cart.rs`, and so does `mod cart;` in
`shop/mod.vr` (use `shop/mod.vr` when `shop` is a directory and you want no
`shop.vr` beside it; having both is an error). A `.rs` module cannot declare
modules. `pub mod cart;` declares a public module.

A path names an item through modules, as deep as they go: `shop::cart::Cart`
starts at a module declared in the current file, `crate::shop::cart::Cart`
starts at the entry file, `self::cart::Cart` at the current module, and
`super::Order` at the module that declares the current one (`super` in the
entry file is an error). Paths work the same way for types, calls, struct
literals, enum variants, and patterns.

`use path;` makes a local alias for a module, struct, enum, or function
inside the package, so you can write a shorter name afterward; `use path as
name;` gives it a different local name instead. The path starts the same way
a path anywhere else does: `crate::`, `self::`, `super::`, or a module
declared in the current file, never a name another `use` made (write
`use a::g as gg;`, not `use a as aa;` then `use aa::g as gg;`). With a
module `shop` holding a module `cart`:

```varyk
// shop.vr
pub mod cart;
```

```varyk
// shop/cart.vr
pub struct Cart {
    pub n: i32,
}

impl Cart {
    pub fn new() -> Cart {
        Cart { n: 0 }
    }
}
```

```varyk
// main.vr
mod shop;

use crate::shop::cart::Cart;
use shop::cart;

fn main() {
    let a = Cart::new();
    let b = cart::Cart::new();
    println!("{} {}", a.n, b.n);
}
```

A `use` cannot end at an enum variant (write `Shape::Circle`, not a `use`
for `Circle` alone) or at a function of a type (call `Cart::new()`). Nor
can it start with the name of a crate: Varyk code does not use crates
directly, so `use std::collections::HashMap;` is an error even though
`std` is built into Rust, and so is `use regex::Regex;` in a package whose
`[dependencies]` names `regex`; call the crate from a `.rs` module in the
package instead (see "Calling Rust" below). A leading name this compiler
does not recognize as a crate is reported as an unknown module instead, with
a note in case it was meant to be one. A `use` name that is already taken by
something of the same kind (a type or module, or a function) declared in
the same file, or by a built-in type or one of `Some`,
`None`, `Ok`, `Err`, `Option`, `Result`, or `Vec`, is an error too. When
the name is both a function and a struct or enum (or a module) where it is
declared, the `use` brings in all of them, so any of them can be the one
that is already taken.

An item or module declared without `pub` can be used in the module that
declares it and in the modules inside that one, and nowhere else: a private
function of `shop` works in `shop/cart.vr`, but not in the entry file or in
another module beside `shop`. A path works only if every module on it can be
used from where it is written, so a `pub fn` inside `mod cart;` (not
`pub mod`) of `shop` is out of reach of the entry file; write `pub mod cart;`
to open it. A `pub` function, struct field, or enum variant cannot name a
type that some of its users could not name themselves, such as a type of
`shop`'s private module `cart` in a `pub fn` of `shop`; make the module
`pub mod`, or the function private. From outside a module, use its `pub`
items through the module name:

```varyk
// math.vr
pub fn square(x: i32) -> i32 {
    x * x
}
```

```varyk
// main.vr
mod math;

fn main() {
    println!("{}", math::square(7));
}
```

A top-level item is a function (`fn`), a struct (`struct`), an enum
(`enum`), an `impl` block, a module declaration (`mod name;`), or a `use`,
each optionally preceded by `pub` (except `impl` and `use`). A struct's
fields follow the same visibility rule as everything else, each on its own
(see "Types" below); all variants of a `pub enum` are public. Only the entry
file of a program may define `main`.

A `pub struct` or `pub enum` of a module is named from the file that imports
it through the module name, as a type and in a struct literal:

```varyk
// m.vr
pub struct User {
    pub name: string,
}
```

```varyk
// main.vr
mod m;

fn make() -> m::User {
    m::User { name: "Bob" }
}

fn main() {
    println!("{}", make().name);
}
```

## Comments, keywords, and reserved words

A comment starts with `//` and runs to the end of the line.

The keywords are `fn`, `pub`, `let`, `mut`, `struct`, `mod`, `if`, `else`,
`while`, `break`, `continue`, `return`, `enum`, `impl`, `match`, `for`, `in`,
`self`, `crate`, `super`, `use`, `as`, `async`, `await`, `true`, and
`false`. `self`, `crate`, and `super` also start a path: `self::`,
`crate::`, and `super::`. `as` names a `use` (`use path as name;`) and
converts a number (`n as i64`, see
[Expressions and operators](#expressions-and-operators)). `async` comes
before `fn` and `await` after a `.` (see
[Async functions and tasks](#async-functions-and-tasks)); no keyword can be
used as a name.

Every other Rust keyword is reserved: `const`, `dyn`, `extern`, `loop`, `move`, `ref`, `Self`, `static`, `trait`, `type`,
`unsafe`, `where`, `abstract`, `become`, `box`, `do`, `final`, `gen`,
`macro`, `override`, `priv`, `try`, `typeof`, `unsized`, `virtual`, and
`yield`. Using one is an error that says the construct is not supported
yet. None of them can be used as a name.

`Some`, `None`, `Ok`, `Err`, `Option`, `Result`, `Vec`, `HashMap`, and
`String` are reserved names: no function, method, struct, enum, variant, field, module,
parameter, or `let` name can be any of them, because the generated Rust
would then hide Rust's own. A struct, enum, or module also cannot take a
built-in type's name (`i32`, `string`, `str`, and so on); a function, field,
or local can (`let string = "x";` is fine). `Error`, the standard error
type, and `Task` and `Shared`, the standard types of async code, cannot
name a struct, enum, module, or `use`, nor a `pub` struct or enum of a
`.rs` module (V0113). `json`, `env`, `log`, and `time`, the
standard modules, cannot name a module, a struct, an enum, or a `use`, and `use json;` or
`use json::parse;` is an error: a standard module is reached by its path
where it is used. `assert` and `assert_eq` cannot name a function, and no
function, method, struct, enum, module, or `use` name can start with
`varyk_`, which is kept for what Varyk adds to the Rust it writes (all
V0113). A local or a field may use any of these names.

## Types

| Type | What it holds |
|---|---|
| `bool` | `true` or `false` |
| `i8`, `i16`, `i32`, `i64` | whole numbers that can be negative, in 8, 16, 32, or 64 bits |
| `u8`, `u16`, `u32`, `u64` | whole numbers that cannot be negative |
| `usize` | a whole number that cannot be negative: a length, an index, or a count |
| `f32`, `f64` | numbers with a fractional part |
| `string` | text |
| a struct you declare | a group of named fields |
| an enum you declare | one of several variants, each with its own values |
| `Option<T>`, `Result<T, E>`, `Vec<T>` | Rust's standard types, written as in Rust and nested freely |
| `HashMap<K, V>` | values of type `V` found by keys of type `K`, an integer type, `bool`, or `string` |
| `Error` | the standard error: a message, given by every standard call that can fail |
| `Shared<T>` | a handle many tasks read one struct `T` through; written only as a parameter's or a `let`'s type (see [Shared](#shared)) |

A struct declares its fields and their types. A field is private unless
marked `pub`, and follows the same rule as any other item without `pub`:
usable in the module that declares the struct and the modules inside it,
and nowhere else. Making the struct itself `pub` does not make its fields
`pub`; the two are independent, as in Rust:

```varyk
pub struct User {
    pub name: string,
    age: i32,
}
```

A struct value is written with its name and every field, in any order:
`User { name: "Alice", age: 30 }`; a literal written outside the struct's
module needs every field visible from there. Fields are read and changed
with a dot: `user.name`. Reading, changing, or naming a private field from
outside its module is an error suggesting either `pub` on the field or a
`pub fn` on the struct that does the work instead.

An enum lists its variants. A variant is a plain name, carries values of
the types in parentheses, or carries named fields in braces, written like a
struct's:

```varyk
enum Shape {
    Circle(f64),
    Rect(f64, f64),
    Point,
}

enum Event {
    Click { x: i32, y: i32 },
    Key(string),
    Quit,
}
```

A variant's fields are as visible as its enum, as in Rust, so they take no
`pub`; two fields of one name in a variant are an error.

An enum cannot contain itself, directly or through a struct, another enum,
or an `Option` or `Result`; inside a `Vec` or a `HashMap` it can:
`enum Tree { Node(Vec<Tree>) }`.

An enum value is written with the enum's name and the variant, followed by
the variant's values in parentheses when it has any: `Shape::Point`,
`Shape::Circle(2.0)`, `Shape::Rect(1.0, 2.0)`. An enum of another module is
reached through it: `geo::Shape::Point`. The number of values must match the
variant, so `Shape::Circle()`, `Shape::Point(1.0)`, and `Shape::Circle`
without its value are errors, as is a variant the enum does not have. A
variant with named fields is written like a struct value, with every field
once, in any order: `Event::Click { x: 1, y: 2 }`; a missing field, an
unknown one, or one written twice is an error, and so are parentheses on it
or braces on a variant with values by position.

`Option`, `Result`, `Vec`, and `HashMap` need exactly their types:
`Option<i32>`, `Result<User, string>`, `Vec<Option<Shape>>`,
`HashMap<string, i32>`. A `HashMap`'s key type is an integer type, `bool`,
or `string`; its value type is any type. A `string` inside them is an owned
`String` in the generated Rust, and a `HashMap` is Rust's
`std::collections::HashMap`. Their values are written as in Rust:

```varyk
let some = Some(5);               // Option<i32>
let none: Option<i32> = None;
let ok: Result<i32, string> = Ok(1);
let failed: Result<i32, string> = Err("no");
let numbers = vec![1, 2, 3];      // Vec<i32>; every element has one type
let empty: Vec<string> = vec![];
let ages: HashMap<string, i32> = HashMap::new();
```

`Some(x)` and `vec![a, b]` know their type from what is inside. `None`,
`Vec::new()`, `HashMap::new()`, an empty `vec![]`, `Ok(x)` (its error type),
`Err(e)` (its value type), and `s.parse()` take their type from where they
go: a `let` with a written type,
a parameter, a return value, a field, or a `vec!` element after one whose
type is known. Anywhere else, write the type in a `let` first; the compiler
never works it out from later lines.

Numbers and `bool` are called Copy types: using one makes a copy, and the
original stays usable. `string`, structs, enums, `Option`, `Result`, `Vec`,
`HashMap`, and `Error` are not Copy.

**Copies and comparisons.** A struct or enum can be copied with
`x.clone()` when every field and value it holds can be: a number, a `bool`,
a `string`, an `Error`, an `Option`, `Result`, `Vec`, or `HashMap` of such types,
another struct or enum that can be copied, or a Rust type whose `.rs` file
derives `Clone` for it (see [Rust structs and methods](#rust-structs-and-methods)).
Two values of a type can be compared with `==` and `!=` by the same rule,
with `PartialEq` in place of `Clone`. A type that holds itself through a
`Vec` or a `HashMap` counts as if it could, as in Rust. `x.clone()` is a
new, owned copy of everything inside; it works the same way on an
`Error`, and on an `Option`, `Result`, `Vec`, or `HashMap` whose contents
can be copied, and
on a number or `bool` it is an error (V0100), since those are copied on use
already. When something inside is in the way, `.clone()` and `==` are
errors (V0203) that name the field, and, for a Rust type, say to derive the
trait in its `.rs` file. In the generated Rust, a struct or enum carries one
`#[derive(Clone, PartialEq)]` line listing what it allows (none when it
allows neither); nothing is ever copied unless `.clone()` is written.

An `impl` block adds functions to a struct or enum declared in the same
file. A function whose parameter list starts with `self` is a method; `self`
is the value it was called on, borrowed to read, and `mut self` borrowed to
change, like any other parameter. A function without `self` is an
associated function. A type may have several `impl` blocks, but not two
functions of one name. A method or associated function is used from
another module only when it is `pub`.

```varyk
impl Counter {
    fn new() -> Counter {
        Counter { count: 0 }
    }

    fn add(mut self, by: i32) {
        self.count = self.count + by;
    }
}
```

A method is called on a value, `counter.add(2)`, and an associated
function through its type, `Counter::new()`, or `m::Counter::new()` for a
type of module `m`. The value a method is called on is passed like any
other argument: a `mut self` method needs a value that may be changed (a
`let mut` name, a `mut` parameter, a field or element of one, or a fresh
value), and the compiler suggests where to add `mut` when it is not.

In the generated Rust, `self` is `&self` and `mut self` is `&mut self`.

### Calls on built-in types

These are all the calls `Vec`, `string`, `Option`, `Result`, `HashMap`, and
`Error` have, beside the calls of a chain (see [Chains](#chains)); any other is an
error listing the type's calls. "Reads" borrows the
value the call is made on, "changes" needs a value that may be changed, like
a `mut` parameter, and "takes" uses the value up, as `?` does (see below).
An argument is kept, like a struct field, unless the
table says it is read: a read argument is only borrowed, so a parameter or
another name for a value can be passed.

**`Vec<T>`:**

| Call | Uses the value | Result |
|---|---|---|
| `Vec::new()` | none | an empty `Vec`; its type comes from where it goes, as for `vec![]` |
| `v.push(x)` | changes | nothing; `x` is kept in `v` |
| `v.pop()` | changes | `Option<T>`: the last element, taken out, or `None` |
| `v.len()` | reads | `usize`, the number of elements |
| `v.is_empty()` | reads | `bool` |
| `v.insert(i, x)`; `i: usize` | changes | nothing; `x` is kept at `i`, and the elements from `i` on move up one |
| `v.remove(i)`; `i: usize` | changes | `T`: the element at `i`, taken out |
| `v.contains(x)`; `x` read | reads | `bool`; `T` is a number type, `bool`, or `string` |
| `v.sort()` | changes | nothing; `T` is an integer type, `bool`, or `string` |
| `v.join(sep)`; `sep: string` read | reads | new text: the elements with `sep` between them; `T` is `string` |
| `v.get(i)`; `i: usize` | reads | `Option<T>`: the element at `i`, or `None` past the end; looked into where it is made unless `T` is a number type or `bool` (see below) |
| `v.iter()` | reads; `v` must be stored | a chain of the elements (see [Chains](#chains)) |
| `v[i]` | reads, or changes when assigned | the element at `i`, which is a `usize` |

`push` and `insert` keep what they are given, like a struct field: a
literal string is copied in, a name holding its own value is given away,
and a value the function only borrows is an error. `v[i]` is a place, like
a field: it can be read, assigned (`v[0] = 5;`), have its fields read or
assigned, and have methods called on it (`tasks[0].complete()`). With `i`
past the end, `v[i]`, `insert`, and `remove` stop the program with Rust's
message. `sort` on floats, or `contains` or `sort` on structs, is an error,
and so is `join` on anything but strings.

**`string`:**

| Call | Uses the value | Result |
|---|---|---|
| `s.len()` | reads | `usize`, the length of the text in bytes |
| `s.clone()` | reads | a new copy of the text |
| `s.is_empty()` | reads | `bool` |
| `s.contains(p)`, `s.starts_with(p)`; `p: string` read | reads | `bool` |
| `s.to_uppercase()` | reads | new text |
| `s.replace(from, to)`; both `string`, read | reads | new text |
| `s.trim()` | reads; `s` must be stored, or a literal | the text of `s` without the spaces at either end: part of `s`, not a copy (see "Returning part of a parameter") |
| `s.split(sep)`; `sep: string` read | reads; `s` must be stored, or a literal | a chain of the pieces of `s` between the places `sep` appears, each part of `s`, not a copy (see [Chains](#chains)) |
| `s.push_str(t)`; `t: string` read | changes | nothing; `t` is added to the end of `s` |
| `s.parse()` | reads | `Result<T, Error>`, `T` a number type or `bool` |

`s.clone()` is the one way to copy text, and it always makes owned text.
`parse` gives an `Err` when the text is not a value of `T`, with Rust's
rules for what the text may look like, and its message names the text and
the type (`` `abc` is not a number ``, `` `yes` is not `true` or `false` ``).
`T` comes from where the result goes, as `None`'s type does, `?` included:
`let n: Result<i32, Error> = text.parse();`, or `let n: i32 = text.parse()?;`
in a function returning `Result<_, Error>`. For an `Option`, write two
statements, `let parsed: Result<i32, Error> = text.parse();` and then
`parsed.ok()`; `text.parse()?` in a function returning an `Option` is an
error (V0206), and so is one in a function whose error type is not `Error`.

**`Error`:**

| Call | Uses the value | Result |
|---|---|---|
| `Error::new(text)`; `text: string` | none | a new `Error` with the message `text`; `text` is kept, like a struct field |
| `e.message()` | reads | the message: part of `e`, not a copy (see "Returning part of a parameter") |

`{}` prints an `Error`'s message, and two errors can be compared with `==`.
There is no conversion into `Error` from another error type: a function's
own error enum becomes one with `map_err` and a function that gives its
text, `r.map_err(|e| Error::new(describe(e)))`. In the generated Rust,
`Error` is `::varyk_std::Error` and `parse` is `::varyk_std::parse`, from
the `varyk-std` crate; a single file that uses either (names `Error`, or
calls `Error::new`, `parse`, or a `json` or `env` call) gets it as a dependency at
exactly the compiler's version.

**`Option<T>`:**

| Call | Uses the value | Result |
|---|---|---|
| `o.is_some()` | reads | `bool` |
| `o.unwrap_or(d)`; `d: T` | takes | `T`: the value inside, or `d` for `None` |
| `o.ok_or(e)` | takes | `Result<T, E>`: `Ok` of the value inside, or `Err(e)` for `None`; `E` comes from where the result goes, as for `None`, and otherwise from `e` |
| `o.map(\|x\| ..)`; `x: T` | takes | `Option<R>`, `R` what the closure gives: `Some` of the closure's value for the value inside, or `None` for `None` (see [Closures](#closures)) |

**`Result<T, E>`:**

| Call | Uses the value | Result |
|---|---|---|
| `r.is_ok()`, `r.is_err()` | reads | `bool` |
| `r.ok()` | takes | `Option<T>`: the `Ok` value, or `None` for an `Err` |
| `r.unwrap_or(d)`; `d: T` | takes | `T`: the `Ok` value, or `d` for an `Err` |
| `r.map_err(\|e\| ..)`; `e: E` | takes | `Result<T, R>`, `R` what the closure gives: the `Ok` value as it is, or `Err` of the closure's value for an `Err` (see [Closures](#closures)) |

To look at the value inside an `Option` or `Result`, use `match`, `if let`,
`?`, or `unwrap_or`. There is no `unwrap` or `expect`: a value that is
absent is something to handle, not a reason to stop the program.

A call that takes its value uses it up, as `?` does: a value made right
there is used up, a name holding its own value is given away and cannot be
used again (V0305), and a stored value (a parameter, a field, an element,
or another name for one) is copied out when everything inside it, a
`Result`'s error type included, is a number or `bool`. So `o.unwrap_or(0)`
on a parameter `o: Option<i32>` is fine, while `r.unwrap_or(0)` on a
parameter `r: Result<i32, string>` is an error (V0304): `match` on it
instead.

**`HashMap<K, V>`:**

| Call | Uses the value | Result |
|---|---|---|
| `HashMap::new()` | none | an empty `HashMap`; its types come from where it goes |
| `m.insert(k, v)` | changes | `Option<V>`: the value `k` had before, or `None`; `k` and `v` are kept |
| `m.contains_key(k)`; `k` read | reads | `bool` |
| `m.len()` | reads | `usize`, the number of keys |
| `m.get(k)`; `k` read | reads | `Option<V>`: the value of `k`, or `None`; looked into where it is made unless `V` is a number type or `bool` (see below) |
| `m.keys()`, `m.values()` | reads; `m` must be stored | a chain of the keys, or of the values, in no fixed order (see [Chains](#chains)) |

A `HashMap` has no `m[k]`, and a `for` cannot go over one directly. Its
order is unspecified, as in Rust. `println!` does not work on one; `==`
does when its values can be compared (see [Types](#types)).

**Looked into where it is made.** `v.get(i)` and `m.get(k)` give an
`Option` whose value is part of the `Vec` or `HashMap`, and so does `find`
on a chain of borrowed items (see [Chains](#chains)). When that value is
a number or `bool`, the result is a plain `Option` holding a copy, usable
anywhere: `counts.get(word).unwrap_or(0)`. Otherwise it must be looked
inside right where it is made, as the value of a `match`, an `if let`, or
a `while let`:

```varyk
if let Some(line) = lines.get(1) {
    println!("second: {}", line);
}
```

`line` is another name for an element of `lines`, as after
`let line = lines[1];`: it cannot be changed, and `lines` cannot be changed
while `line` is still used (V0307). Stored in a `let`, passed, returned,
used with `?`, or given any method, `is_some()` included, such a result is
an error (V0208), and so is a pattern naming it whole. The `Vec` or
`HashMap` it is called on must be stored, not made right there (V0001).

## Literals

- Whole numbers: `0`, `42`. Negative numbers use `-`: `-4`. An underscore
  may separate digits: `1_000`.
- Fractional numbers: `2.5`. Digits are required on both sides of the dot.
- `true` and `false`.
- Strings in double quotes: `"Hello"`. The escapes are `\n` (new line),
  `\t` (tab), `\\` (backslash), `\"` (double quote), and `\0` (zero byte).
  No other escapes and no raw strings.

A number literal takes the type its place expects, such as the declared type
of a `let` or a parameter, `usize` included. Where nothing is expected, a
whole number is `i32` and a fractional one is `f64`. Number types never
convert into each other, so a count that meets a `usize` is declared as one:
`let mut i: usize = 0;`.

## Statements

```varyk
let x = 5;              // a new name; the type is worked out from the value
let y: i64 = 5;         // the same, with the type written out
let mut count = 0;      // `mut` lets the name be given a new value later
count = count + 1;      // assignment to a `let mut` name
user.name = "Bob";      // assignment to a field of a `let mut` name
scores[0] = 10;         // assignment to an element of a `let mut` Vec
return x;               // leave the function with a value
while count < 10 { }    // repeat while the condition is true
while let Some(t) = tasks.pop() { }  // repeat while the pattern fits
for i in 0..n { }       // i takes 0, 1, ..., n - 1
for item in items { }   // every element of a Vec, in order
break;                  // leave the innermost `while`, `while let`, or `for`
continue;               // go to the next round of the innermost loop
print_user(user);       // any expression followed by `;`
```

A name can be declared again with a new `let`; the new one hides the old one
from then on.

## Expressions and operators

Expressions are literals, names, module paths (`math::square`), function
calls, method calls (`counter.add(2)`), associated function calls
(`Counter::new()`), field access (`user.name`), elements (`v[i]`), struct
values (`User { .. }`, or `m::User { .. }` for a struct of module `m`), enum
values, `Option`, `Result`, and `Vec` values, `format!`, parentheses, blocks,
`if`/`else`, `match`, `?` after a `Result` or an `Option` (see
[Passing errors on](#passing-errors-on)), and `as` after a number.

A block `{ ... }` runs its statements, and if it ends with an expression and
no `;`, that expression is the block's value. A function body works the same
way, so `fn add(a: i32, b: i32) -> i32 { a + b }` returns `a + b`.

`if`/`else` is an expression, so it can produce a value:

```varyk
let label = if age >= 18 { "adult" } else { "child" };
```

Conditions must be `bool`. Chains are written
`if ... { } else if ... { } else { }`. A struct value inside an `if` or
`while` condition needs parentheses.

Operators:

- `-x` negates a number; `!b` is "not" for a `bool`.
- `+ - * / %` work on numbers only, and both sides must have the same type.
  `+` does not join strings: `format!("{}{}", a, b)` does.
- `< <= > >=` compare numbers of the same type.
- `==` and `!=` compare two values of one type: numbers, `bool`, strings,
  and a struct, an enum, an `Option`, a `Result`, a `Vec`, or a `HashMap`
  whose contents can be compared (see [Types](#types)); anything else is
  an error naming the field in the way (V0203).
- `&&` is "and", `||` is "or".
- `x as T` converts a number to another number type: any integer type,
  `usize`, `f32`, or `f64`, with Rust's rules (a float is truncated toward
  zero and saturates, an integer wraps to a narrower width: `300 as u8` is
  `44`, `-1 as u8` is `255`). Both sides must be number types (V0200): not a
  `bool`, a string, or a struct. It is how a `usize` becomes an `i32`:
  `let count = v.len() as i32;`. A number written on its own before `as`
  is an `i32` (or an `f64` with a decimal point).

Precedence, from tightest to loosest: `-` and `!` first; then `as`; then `*`, `/`, and
`%`; then `+` and `-`; then the comparisons; then `&&`; then `||`. Operators
of the same level group from left to right, so `10 - 3 - 2` is `5`.
Because `as` binds tighter than `*`, `a as i32 * 2` is `(a as i32) * 2`.
Comparisons cannot be chained: `a < b < c` is an error. Use parentheses when
in doubt.

## Closures

A closure is a small piece of code handed to a call that runs it:
`|x| expression`, or `|x| { block }` when it needs statements. It exists
only as the argument of a call that takes one: `map` on an `Option`,
`map_err` on a `Result`, and `map`, `filter`, `any`, `all`, and `find` on a
chain (see [Chains](#chains)):

```varyk
let doubled = Some(4).map(|n| n * 2);
let checked = parse_age(text).map_err(|e| format!("{}: {}", field, e));
```

A closure has exactly one parameter, the value the call hands it (V0201
otherwise), with no type written: `n` is the `i32` inside the `Option`, `e`
the error of the `Result`. It has no return type either; the call's result
holds whatever the closure gives, so `Some(4).map(|n| n * 2)` is an
`Option<i32>`. A closure anywhere else, in a `let`, as an argument to a
Varyk function, or returned, is an error (V0001): a closure is never a
value of its own. A type written on the parameter, `|x: i32|`, and `move`
before the closure are errors too (V0001).

The body is a value of its own, not a piece of the function around it:
`return`, `break`, `continue`, and `?` inside it are errors (V0001), since
they would leave the closure rather than the function. A loop inside the
body is the closure's own, and can use `break` and `continue`. A `None`,
`Vec::new()`, `Ok`, or `Err` the closure gives takes its type from where
the call's result goes, as in
`let found: Option<Option<i32>> = o.map(|n| None);` (V0207 with nothing to
take it from).

Inside the body the names of the function around it can be read, and
nothing more: assigning to one, passing it to a `mut` parameter, or calling
a changing method on it is an error (V0301, V0303), and so is keeping it
anywhere that owns its value (V0304), such as `push`, a struct field, or
the closure's own `let mut` given it; copy text with `.clone()`. A `let`
of it inside the closure is another name for it, and it is still there
after the closure. The parameter is read-only too, like a name a `match`
pattern makes: to change it, first write `let mut n = n;`.

The closure of `map` and `map_err` must give something new, because the
`Option` or `Result` it makes owns what it holds: a number, a new string,
a call's result, the parameter itself, or a string literal, which becomes
new text (`o.map(|n| if n > 5 { "big" } else { "small" })` is an
`Option<string>`). Giving a name from outside, or part of the parameter,
is an error (V0304; `.clone()` for text), and giving parts of two
different names is V0308.

In the generated Rust a closure is written as it is, and a name from
outside is borrowed by it without being changed: an owned `String` stays a
`String` and is passed on as `&name`, as anywhere else.

## Chains

A chain goes over the elements of a stored value one by one, in one
expression: it starts at a source, passes through any number of `map`s
and `filter`s, and ends at a call that gives a value.

```varyk
let total = numbers.iter().sum();
let evens = numbers.iter().filter(|n| n % 2 == 0).count();
let names: Vec<string> = users.iter().map(|u| u.name.clone()).collect();
```

The sources are `v.iter()` on a `Vec`, `s.split(sep)` on a string, and
`m.keys()` and `m.values()` on a `HashMap`, each on a stored value (or, for
`split`, a string literal): on a value made right there it is an error
(V0001), so store it with `let` first. The calls of a chain, `I` its item
type:

| Call | Result |
|---|---|
| `c.map(\|x\| ..)`; `x: I` | a chain of what the closure gives |
| `c.filter(\|x\| ..)`; `x: I`, gives a `bool` | a chain of the items for which the closure gives `true` |
| `c.collect()` | `Vec<I>`, the items in order; they must be owned or copies (see below) |
| `c.count()` | `usize`, the number of items |
| `c.sum()`; `I` a number type | `I`, the items added up |
| `c.any(\|x\| ..)`, `c.all(\|x\| ..)`; give a `bool` | `bool`: whether the closure gives `true` for any item, or for every one |
| `c.find(\|x\| ..)`; gives a `bool` | `Option<I>`: the first item for which the closure gives `true`, or `None`; looked into where it is made when the items are borrowed (see below) |

A chain must be finished where it is written: stored in a `let`, passed,
returned, used as a statement of its own, or put inside `Some` or a `vec!`,
an unfinished chain is an error (V0208), because it has no type to write.
`collect` needs no type written, and `sum` on items that are not numbers is
an error (V0200). A chain can also end as the head of a `for` (see
[Loops](#loops)).

**Items.** Every item is borrowed, a copy, or owned:

- **borrowed**: another name for an element of the stored value, read-only,
  like a name a `match` pattern makes: the elements of `v.iter()`, the keys
  or values of `m.keys()` and `m.values()`, unless they are numbers or
  `bool`s, and the pieces of `s.split(sep)`, parts of `s`;
- **copies**: numbers and `bool`s, copied as they are read;
- **owned**: new values a `map` made.

`filter` keeps the kind. `map` gives copies when its closure gives a number
or `bool`, owned items when it gives something new (a call's result, a
`.clone()`, a `format!`, a string literal), and borrowed items when it gives
part of its item (`users.iter().map(|u| u.name)`), or part of a name from
outside it (`|x| other.name`), which the items are then parts of. Giving
part of an owned item is an error (V0304): it ends with the closure.
Giving something new in one place and a part in another is V0304, and
parts of two different things V0308.

The closure of `map` gets the item itself. The closures of `filter`,
`any`, `all`, and `find` only look at it, since it goes on down the chain or
becomes the result: keeping it anywhere that owns its value is an error
(V0304); `map` is where a closure makes something new from an item. All of
them read names from outside as any closure does.

`collect` makes a `Vec` that owns its items, so a chain of borrowed items
must copy them first, `words.iter().map(|w| w.clone()).collect()`
(V0304 otherwise). `find` on copies or owned items gives a plain `Option`;
on borrowed items the `Option` holds part of the stored value and is looked
into where it is made, as `get`'s is: its value is another name for that
part, and the stored value cannot change while it is used (V0307).

```varyk
if let Some(word) = words.iter().find(|w| w.len() > 3) {
    println!("{}", word);
}
```

In the generated Rust a chain is written as it is, with `.copied()` after a
source of numbers or `bool`s, `collect::<Vec<_>>()`, and `sum::<T>()`.

## Matching

`match` looks at an enum, an `Option`, a `Result`, a number, a `bool`, or a
string and runs the first arm whose pattern fits. Like `if`, it is an
expression:

```varyk
fn area(shape: Shape) -> f64 {
    match shape {
        Shape::Circle(r) => 3.14 * r * r,
        Shape::Rect(w, h) => w * h,
        Shape::Point => 0.0,
    }
}

fn grade(score: i32) -> string {
    match score {
        90..=100 => "A",
        80..=89 => "B",
        _ => "lower",
    }
}
```

Each arm is `pattern => expression,`; the comma may be left out after a
block. An arm that starts with a block ends at its `}`, as in Rust, so
`{ 1 } + 2` there is written `({ 1 }) + 2`. Every arm must have the same type, or no arm has a value (a
`println!`, a call that returns nothing, or a block without a final
expression). An arm that always `return`s fits anywhere; a `return`,
`break`, `continue`, or assignment as an arm's body is written as a block,
`Some(x) => { return x; }` or `Some(x) => { t = t + x; }`. `None`,
`Vec::new()`, `Ok`, and `Err` in an arm take the type the `match` is
expected to have, or else the type of an earlier arm.

A pattern is one of:

- `_`, which fits anything;
- a name, which fits anything and names it;
- a variant: `Shape::Point`, `Shape::Circle(r)`, `Shape::Rect(w, h)`,
  `geo::Shape::Point` for an enum of module `geo`, `Some(x)`, `None`,
  `Ok(x)`, or `Err(e)`, with a pattern for each of the variant's values;
- a variant with named fields, naming every field once, each as `name`,
  which names it, or `name: pattern`, with `name: _` for one not needed:
  `Event::Click { x: 0, y }`. There is no `..`, so leaving a field out is
  an error;
- an integer, with an optional `-`, a string, `true`, or `false`, which
  fits that one value;
- a range of two integers, `1..=5`, which fits both ends and every number
  between them; the first end must not be above the second.

Patterns nest: `Some(Shape::Circle(r))`, `Ok(Some(x))`,
`Event::Click { x: 0, y }`. A name may appear only once in a pattern. A
literal or range end must have the value's type and fit it (`300` against a
`u8` is an error). A number with a fractional part cannot be matched with a
pattern (use `if`), and a string literal can only be the whole pattern of a
`match` on a string, never inside another pattern: for `Some("yes")`, name
the value and compare it with `==` in the arm. A struct cannot be taken
apart by a pattern (name it and look at its fields with `if`). Not in
Varyk: guards (`pattern if condition`), `a | b`, `..`, and `@`.

Every value must fit some arm: for an enum, `Option`, or `Result`, every
variant at every depth; for a `bool`, both values; and for a number or a
string, a last arm `_ => ...` or a name, because Varyk counts them as
having more values than any list of arms can name: `0..=255` on a `u8`
still needs one, which is never counted as unreachable. The error names a
value no arm fits, such as `Some(Shape::Point)` or
`Event::Click { x: _, y: _ }`. An arm that can never run, because the
arms above it already fit everything it fits, is an error too: `4` after
`1..=5`, a variant repeated after an identical arm, or anything after `_`
or a name.

`if let` runs its block when a pattern fits and its `else`, which may be
another `if` or `if let`, when it does not; like `if`, it is an expression,
and the `else` may be left out when the block has no value. `while let`
repeats its block while the pattern fits, looking at the value anew each
round; `break` and `continue` work in it as in `while`:

```varyk
if let Some(user) = users.get(0) {
    println!("{}", user.name);
} else {
    println!("nobody");
}

while let Some(task) = queue.pop() {
    run(task);
}
```

The value an `if let` or `while let` looks at follows every rule of a
`match`'s, below, and so do the names its pattern makes, which exist only
in its block.

The names a pattern makes exist only in their arm, like a `let` inside a
block: one that has the same name as an outer name hides it in the arm, and
the outer name is back, unchanged, after the `match`.

**Matching borrows what is stored.** The value matched on is either a name,
a field, or an element (something stored), or a value made right there (a
call result or a new value). An `if`, a block, or a field or element of a
value made right there (`make().status`) is an error: store it with `let`
first.

- Matching something stored never gives it away. A name the pattern makes
  for a number or `bool` is a copy. A name for anything else is another
  name for part of the stored value, with the rules of a `let` made from a
  field: while it is still used, the stored value cannot be changed or
  given away (V0307), and it cannot be stored or returned except as part of a parameter the function returns (see "Returning part of a parameter") (use
  `.clone()` for text). An arm that uses no such name may change the stored value.
- Matching a value made right there owns it: the names hold their own
  values and can be given away, pushed, or returned. The exceptions are a
  value holding, anywhere inside it, an enum that runs code when it is
  thrown away (see "Rust enums"), which is only looked at, as if stored;
  and a string, below.
- Matching a string, stored or made right there, with or without string
  literal arms, only looks at it: a name the pattern makes is borrowed
  text, which can be read but not stored or returned except as part of a parameter the function returns (see "Returning part of a parameter"), and for a string
  made right there, not kept past the `match` either. Copy it with
  `.clone()` to keep it: `other => other.clone()`.

None of the names a pattern makes can be changed. To change a copy, first
make a changeable one: `let mut n = n;`; to change the stored value, change
it by its own name.

So an `Option<Item>` kept in a `let` cannot give its `Item` away: `match`
only looks inside it. Match on the call that made it instead:
`match tasks.pop() { Some(task) => done.push(task), None => {} }`.
When the stored value was made by a `match`, `if`, or block, move that
into a function that returns it and match on a call of the function.

A `match` used as a value follows the rule of `if`: when every arm gives
something stored, the result is another name for it; when every arm gives a
new value, the result owns it. Mixing the two is an error for text, and
wherever the value is only read in place, such as in a call or a
`println!`; store it with `let` first.

In the generated Rust, a stored value is matched by reference (`match
&shape`), a string as a `&str` (`match name.as_str()`), and a copied name
is copied at the start of its arm with `let n = *n;`.

## Loops

`while` repeats while its condition is true. `for` goes over a range of
integers, the elements of a `Vec`, or the items of a chain (see
[Chains](#chains)):

```varyk
for i in 0..n { }        // i takes 0, 1, ..., n - 1
for i in 1..=n { }       // i takes 1, 2, ..., n
for task in tasks { }    // every element of tasks, in order
for word in text.split(" ") { }            // every piece of text
for n in numbers.iter().filter(|n| n > limit) { }
```

A range `a..b` counts from `a` up to, but not including, `b`; `a..=b`
counts up to and including `b`. Both ends are
integers of one type, and a written number takes the other end's type, so
`0..tasks.len()` counts in `usize`. Ranges exist only in the head of a
`for`. Nothing else can be looped over: not a string,
not an `Option`, not a range of anything but integers (V0200), and not a
`HashMap`, whose rounds would need a key and a value together (V0001). `break` and
`continue` work in `for` as in `while`. The loop variable exists only in
the loop body, and is a new name each round.

**Looping borrows what is stored**, as matching does. The `Vec` looped over
is either stored (a name, a field, or an element) or made right there (a
call result or a `vec!`). An `if`, a block, or a field or element of a
value made right there is an error: store it with `let` first.

- Looping over something stored never gives it away, and the stored `Vec`
  cannot be changed or given away anywhere inside the loop, whether or not
  the body uses the loop variable (V0307): `for x in v { v.push(1); }` is an
  error, unless the function returns right there: a change followed by
  `return`, or `return v` itself, is fine. After the loop, `v` can change
  again. To change elements as you
  go, loop over the positions instead: `for i in 0..v.len() { v[i] = 0; }`.
- The loop variable is a copy when the elements are numbers or `bool`;
  otherwise it is another name for one element, with the rules of a `let`
  made from an element: it cannot be stored or returned except as part of a parameter the function returns (see "Returning part of a parameter") (use
  `.clone()` for text).
- Looping over a value made right there owns it: the variable holds each
  element in turn and can be given away, pushed, or returned.

- Looping over a chain binds one item each round, with the rules of its
  kind: a borrowed item is another name for part of what the chain goes
  over, a copy is a copy, an owned item is owned. The body cannot change or
  give away anything the head reads (V0307): what the chain goes over, the
  argument of `split`, and every name a closure of the head reads, numbers
  and `bool`s included, so `for n in numbers.iter().filter(|n| n < limit)
  { limit = limit + 1; }` is an error. To change one of them, `collect` the
  chain first and loop over the `Vec`. After the loop they can change again,
  even while a name that took the loop variable's value is still used.

The loop variable can never be changed. To change a copy, first make a
changeable one: `let mut n = n;`; to change the stored `Vec`, change it by
its own name, after the loop.

In the generated Rust, a stored `Vec` is looped over by reference (`for x
in &v`), and a copied element is copied at the start of the body with
`let x = *x;`; a chain is written as it is.

## Passing errors on

`r?` takes the value out of a `Result`: when `r` is `Ok(v)`, `r?` is `v`;
when it is `Err(e)`, the function returns `Err(e)` at once. It works only in
a function that returns a `Result`, on a `Result` with the same error type:

```varyk
fn load(id: i32) -> Result<User, string> {
    let name = name_of(id)?;    // name_of returns Result<string, string>
    Ok(User { name: name, age: 36 })
}
```

`o?` does the same for an `Option`: when `o` is `Some(v)`, `o?` is `v`;
when it is `None`, the function returns `None` at once. It works only in a
function that returns an `Option`, on an `Option`.

Anything else is V0206: `?` in a function that does not return a `Result`
or an `Option` (`main` never does, so use `?` in a helper and `match` on its
result there), on an `Option` in a function that returns a `Result` or a
`Result` in a function that returns an `Option` (the message names both
types), on a value that is neither, or on a `Result` whose error type is
different; an error is never converted into another type.

What a `?` is expected to produce flows into its operand, so
`let n: i32 = Ok(x)?;` and `let n: i32 = Some(x)?;` type without more
annotation, and `Ok(x)?` alone takes the function's error type. `Err(e)?;`
as a statement has no value type to take and is V0207: write
`return Err(e);`. In the generated Rust, the type of a bare `Ok`, `Err`, or
`None` under `?` is written out (`Ok::<i32, String>(x)?`).

`?` can be used anywhere a value can: in a `let`, in an argument, in a
`match` arm, in a loop, or as a statement of its own (`check(text)?;`).
It takes what it is given, like a return does: a `Result` made right there
is used up, a name holding one is given away (using it again is V0305), and
a `Result` the function only borrows, such as a parameter, a field, or an
element, cannot be used with `?` (V0304). In the generated Rust, `r?` is
written as it is.

## Logging

The four `log` calls write a line to stderr. Like `json` and `env`, `log`
is written with the module's name; there is no `use log;`.

| Call | Uses the value | Result |
|---|---|---|
| `log::debug(format, args..)`; `args` read | none | nothing: a debug line |
| `log::info(format, args..)`; `args` read | none | nothing: an info line |
| `log::warn(format, args..)`; `args` read | none | nothing: a warning line |
| `log::error(format, args..)`; `args` read | none | nothing: an error line |

```varyk
// main.vr
fn main() {
    let port = 8080;
    log::info("listening on port {}", port);
    log::warn("queue is {} percent full", 90);
}
```

The format string and its arguments follow `println!` exactly: the text
is a string literal written in quotes (anything else is V0202), the
number of `{}` must match the number of arguments (V0202), and the
arguments are numbers, `bool`, strings, or an `Error` (V0203 otherwise).

`LOG` sets the level, `debug`, `info`, `warn`, `error`, or `off`; it is
`info` when unset, and a line below the level is not written. `LOG` and
`LOG_FORMAT` are read as `env::parse` reads a variable: the process
environment first, then `.env`. A text line, without colour, is the time,
the level, and the message: `2026-09-30T12:00:00.000Z INFO listening on port
8080`. With `LOG_FORMAT=json` each line is one JSON object with the keys
`time`, `level`, and `message`; any other value gives text.

A program that makes a `log` call starts logging first thing in its
`main`; a program without one writes nothing to stderr. Setup never stops
the program: a bad `LOG` value or a `.env` that cannot be read gives one
warning line, and logging goes on at `info` in text. In the generated Rust
a call is `::varyk_std::tracing::info!(..)`, and `main` begins with
`::varyk_std::start();`. A library's `log` calls go wherever the program
using it sends them.

## Tests

A top-level function marked `#[test]` is a test, in any module, `pub` or
not. `varyk test` builds the program with its tests and runs them;
`varyk build` and `varyk run` leave them out. Two calls check results,
and only inside a test:

| Call | Uses the value | Result |
|---|---|---|
| `assert(cond)`; `cond` a `bool` | none | nothing; the test fails when `cond` is false |
| `assert_eq(a, b)`; `a` and `b` read | none | nothing; the test fails when `a != b` |

```varyk
// main.vr
fn total(a: i32, b: i32) -> i32 {
    a + b
}

fn main() {
    println!("{}", total(2, 3));
}

#[test]
fn adds_two_numbers() {
    assert_eq(total(2, 3), 5);
    assert(total(0, 0) == 0);
}
```

A test takes no parameters, returns nothing, cannot be called from the
program, and cannot be the entry file's `main` (V0114). `assert` takes a
`bool` (V0200 otherwise). `assert_eq` takes two values of one type (V0200
otherwise) that `==` can compare (V0203 otherwise), reads them as `==`
does, and works out each one once. Outside a test both are V0114: a check
that stops the program has no place in service code, which returns an
`Err` instead.

A failing check stops its test and names the Varyk file and line, the
file from the package's root, or for a single file its name alone, wherever
`varyk` runs: `assertion failed at src/store.vr:12`. `assert_eq` adds both values when they print with `{}`
(numbers, `bool`, strings, and `Error`): `assertion failed at
src/store.vr:12: left is 4, right is 5`.

`varyk test` shows the Rust test runner's report, which names a test in a
module by its path (`store::adds_two_numbers`), and exits 0 when every
test passes. In the generated Rust a test is a `#[test]` function, `assert`
is `::std::assert!(cond, "assertion failed at ..")`, and `assert_eq` is
`::std::assert!` on `==` of its two values.

## Printing

`println!` prints a line. The first argument is a string literal, and each
`{}` in it is replaced by the next argument:

```varyk
println!("{} is {} years old", name, age);
```

The number of `{}` must match the number of arguments. Write `{{` and `}}`
to print `{` and `}`. Nothing may go inside the braces. The arguments must be
numbers, `bool`, strings, or an `Error`, which prints its message; a
struct, an enum, an `Option`, a `Result`, a `Vec`, or a `HashMap` cannot be
printed whole, even one that can be copied and compared.

`format!` follows the same rules and, instead of printing, makes new text:
`let line = format!("{} is {}", name, age);`. It is the way to join strings.

## Functions and parameters

```varyk
fn add(a: i32, b: i32) -> i32 {
    a + b
}
```

A function without `->` returns nothing. A function returns the value of its
body or of a `return` statement.

**Functions borrow by default.** A parameter written `user: User` lets the
function read the value it was given, but not change it. The caller keeps the
value and can use it again afterwards. You never write anything special at
the call: `print_user(user)` twice in a row is fine.

**`mut` means the function may change the caller's value.** A parameter
written `mut user: User` may be changed inside the function, and the caller
sees the change. The value passed in must itself be changeable: a `let mut`
name, a `mut` parameter, a field or element of one of those, or a fresh
value such as `User { name: "Alice" }` or a call result. The compiler says
so if it is not, and suggests where to add `mut`.

```varyk
fn rename(mut user: User) {
    user.name = "Bob";
}
```

Numbers and `bool` are simply copied into a parameter without `mut`, which
behaves the same as borrowing. A `mut i32` parameter changes the caller's
number like any other `mut` parameter.

**Giving a value away.** `let b = a;` gives the value of `a` to `b`, if it is
a string or a struct. After that, `a` cannot be used, and the compiler points
at the line where it was given away. Numbers and `bool` are copied instead.
The same happens when a name is stored in a struct field, put inside an enum
value, `Some`, `Ok`, or `Err`, or in a `vec!`, passed to `push`, returned,
or passed to a Rust function that takes a `String`.

A name made with `let` from a parameter, or from any field or element, is
another name for that same value, not a copy; so is a `let mut` text name
after one of these is assigned to it, and so is a name a `match` pattern
makes for part of a stored value, or the variable of a `for` over a stored
`Vec`. Changing it changes the original.
While the other name is still used later, the original cannot be changed or
given away; and when the other name was made with `let mut` and can change
the original, the original cannot be used at all until the other name is no
longer needed (V0307). Changing includes calling a changing method on the
original or on a part of it: after `let first = tasks[1];`,
`tasks[0].complete()` changes `tasks`. A loop body counts as coming after
itself.

**What a function only borrows, it cannot keep.** A string or struct
parameter belongs to the caller. A field that holds a string or a struct
belongs to the struct around it, even when that struct is your own. The same
goes for a `let` name made from one of these, and for an element of a `Vec`,
which belongs to the `Vec`. Such a value cannot be stored into a struct
field or an element, put inside an enum value, `Some`, `Ok`, `Err`, or a
`vec!`, or passed to `push`; the error says what to do instead. For text,
the fix is a copy: `user.name.clone()`.

**Returning part of a parameter.** A function may return part of one of its
parameters without copying it: every value it returns (its last value, and
every `return`, each branch of an `if` or `match` counting on its own) is
part of the same parameter, `self` included, and the function does not
change that parameter.

```varyk
impl User {
    fn display_name(self) -> string {
        if self.nickname.is_empty() { self.name } else { self.nickname }
    }
}

fn first(users: Vec<User>) -> User {
    users[0]
}
```

Nothing is written for it: Varyk works it out. The result of such a call is
another name for part of the value passed in that position (the value a
method is called on, for `display_name`), as a field is: `let n =
user.display_name();` cannot be changed, cannot be kept anywhere that owns
it (copy it with `.clone()`), and `user` cannot be changed or given away
while `n` is still used (V0307). It can be looked at by `match`, `if let`,
`while let`, and `for`, and have calls made on it. The value passed in that
position must be stored, or be a string literal: `make_user().display_name()`
is an error (V0001), since the new `User` would be gone at the end of the
line; store it with `let` first. `s.trim()` works the same way.

These are errors, each fixed by returning a copy (`.clone()` for text)
instead: returning part of a parameter in one place and something new in
another, a string literal included, or beside a `?`, which returns a new
`None` or `Err` early (V0304); parts of two different
parameters (V0308); part of a `let` of the function, or of a number or
`bool` parameter or `for` variable, which the function gets as its own
copy, since either ends when it returns (V0304); part of a `mut` parameter (V0304), since the caller could
not even read what it passed while the result is used; and part of a
parameter in a function that calls itself, directly or through other
functions (V0304). A number or `bool` is always copied, so returning one is
never part of anything. Varyk decides a head of a `match`, `if let`, or
`while let` by how it is written, before it knows which calls return parts,
so a field of such a call there (`match user.info().kind`) is an error
(V0001); store the call's result with `let` first.

For Rust readers: `user: User` becomes `user: &User`, `mut user: User`
becomes `user: &mut User`, and the call sites get `&` and `&mut`. A
function returning part of a parameter returns `&str` or `&User`, with one
lifetime written on that parameter and the return when Rust's elision would
not pick it. In Rust,
`mut name: T` means an owned parameter that can be rebound; Varyk uses `mut`
for the mutable borrow instead, and no Varyk-declared parameter takes
ownership of a string or struct. Copy types are passed by value.

## Async functions and tasks

`async` before `fn` makes an async function: one that can wait, on a timer
here and on the network in later milestones, without holding up the rest of
the program. It goes on a top-level function or on a method, after `pub`
when there is one: `pub async fn load(self) -> User`. Parameters, `self`,
`mut`, the return type, and the body follow the rules of any function.

```varyk
// main.vr
struct User {
    name: string,
}

impl User {
    async fn greet(self) -> string {
        time::sleep(10).await;
        format!("hello, {}", self.name)
    }
}

async fn twice(n: i64) -> i64 {
    time::sleep(10).await;
    n * 2
}

async fn main() {
    let u = User { name: "ann" };
    println!("{}", u.greet().await);
    println!("{}", twice(21).await);
}

#[test]
async fn doubles() {
    assert_eq(twice(2).await, 4);
}
```

A call to an async function is followed by `.await`, which runs it right
there and gives what it returns: `twice(21).await` is an `i64`. Its
arguments are lent as any call's are, so `u` above can be used again.
`.await` can be used inside `if`, `match`, `for`, `while`, and `println!`
like any call, and `f(x).await?` passes an `Err` on.

- Only an async function can call an async function or use `.await`
  (V0211, whose fix adds `async`), and `.await` cannot be written inside a
  closure, which is not async (V0211).
- `.await` goes after a call to an async function, `Task::all` or
  `Task::all_settled`, or a name holding a task (V0212).
- Async functions may not call each other in a cycle, directly or through
  others, and an async function may not call itself (V0214).
- An async function may not return part of a parameter (V0311): everything
  it returns is new, so return a copy (`.clone()` for text).

`async fn main()` starts the program on a runtime that spreads async work
over every core of the machine; a program that logs starts logging first
thing inside it. Nothing may call an async `main` (V0106). `#[test] async
fn` is a test, run on a runtime of its own; otherwise the rules of
[Tests](#tests) apply. Both keep the shape of any `main` and any test.

`time` is a standard module, like `json`: its call is written with the
module's name, and there is no `use time;`.

| Call | Uses the value | Result |
|---|---|---|
| `time::sleep(ms)`; `ms: u64` | none | async, nothing: waits `ms` milliseconds without holding up other work |

### Two ways to call

A call to an async function, a method, `time::sleep`, or an async function
or method of a `.rs` module included, is one of two kinds, decided by what follows it:

| Written | Kind | Type | Meaning |
|---|---|---|---|
| `f(x).await` | awaited | `T` | runs `f` here and gives its result; nothing is started |
| `f(x)` | started | `Task<T>` | starts `f` now, running alongside this function, and gives a task for its result |

```varyk
// main.vr
async fn fetch(id: i64) -> i64 {
    time::sleep(10).await;
    id * 2
}

async fn send_email(to: string) {
    // sending takes a while, and nothing needs to wait for it
    time::sleep(50).await;
}

async fn main() {
    let a = fetch(1);
    let b = fetch(2);
    send_email("ann@example.com").detach();
    println!("{}", a.await + b.await);
}
```

`a` and `b` run at the same time, so the program waits about 10
milliseconds for both, not 20, and prints `6`. Nothing waits for
`send_email`, so the program may end before it does.

An awaited call lends its arguments as any call does. A started call gives
its arguments, the value a method is called on included, to the task,
which keeps them while it runs and lends them to the function: a number
is copied, a name holding an owned value is given away (using it later is
V0305), and a new value, such as a call's result, a literal, or a
`.clone()`, is handed over. The task may outlive the function that started
it, so a value that function only borrows (a parameter, a field or element
of one, or another name for one) cannot be given to it (V0304): give it a
copy with `.clone()`. A started call cannot pass to a `mut` parameter or
call a `mut self` method (V0309): the task would change only its own copy.

A started call may stand in four places only, so that no task is thrown
away by accident:

- as the whole value of a `let` with a name: `let a = fetch(1);` (not
  `let _ = ..`, and not a branch of an `if` or `match` there);
- as what `.detach()` is called on: `send_email(to).detach();`;
- as an element of `vec!`: `vec![fetch(1), fetch(2)]` starts two tasks;
- as the whole value of the closure of a chain's last `map`, followed at
  once by `collect`: `ids.iter().map(|id| fetch(id)).collect()` starts one
  task per id.

The last two make a `Vec` of tasks, which is kept in a `let` or given to
`Task::all` or `Task::all_settled` to wait for every task in it (see
[Tasks](#tasks)). The `map` closure's items are lent to it, so give the
task a copy of anything but a number: `names.iter().map(|n|
send_email(n.clone())).collect()` (V0304 otherwise).

Anywhere else, a statement `fetch(1);` and an argument `show(fetch(1))`
among them, it is an error (V0213), since the task would be thrown away
and stopped at once; the fix is `.await`, or `.detach()`.

### Tasks

| Call | Uses the value | Result |
|---|---|---|
| `t.await` | takes `t` | `T`: waits for the task and gives its result |
| `t.detach()` | takes `t` | nothing: the task runs on with nobody waiting for it |
| `Task::all(tasks).await` | takes `tasks`, a `Vec` of tasks giving `T` | `Vec<T>`: waits for every task |
| `Task::all(tasks).await` | takes `tasks`, a `Vec` of tasks giving `Result<U, E>` | `Result<Vec<U>, E>`: every `Ok` value, or the first `Err` to arrive |
| `Task::all_settled(tasks).await` | takes `tasks`, a `Vec` of tasks giving `Result<U, E>` | `Vec<Result<U, E>>`: waits for every task and keeps each outcome |

- **A task that is thrown away is stopped.** When a name holding a task
  goes out of scope before it is awaited or detached, the task's work
  stops at its next `.await`. A `?` or `return` that leaves a function
  early stops the tasks it started and did not await yet.
- **A detached task** runs until it finishes or the program (or the test)
  ends, whichever comes first; nobody sees its result. A panic in it is
  printed and does not stop the program.
- **A panic in a task that is awaited** continues in the function that
  awaits it, as if the call had been made there.
- **A task stays where it is made.** A name holding a task can only be
  awaited or detached, once (using it again is V0305), and not inside a
  closure, which only borrows it (V0304). A `Vec` of tasks, made by `vec!`
  or `collect`, can only be kept in a `let` and given, once, to `Task::all`
  or `Task::all_settled`, directly or from that `let` (using it again is
  V0305). Anything else, indexing, `push`, a `for` over it, passing,
  returning, or giving it to another name, is an error (V0215); so is
  `.await` on a `Vec` of tasks, and writing `Task` as a type: a task's
  type is always worked out from its call.
- **A task is always used.** A name holding a task, or a `Vec` of tasks,
  that nothing awaits, detaches, or gives to `Task::all` or
  `Task::all_settled` is an error (V0213), so a forgotten `.await` is
  caught.

`Task::all` and `Task::all_settled` wait for every task in a `Vec` and give
the results in the order of the `Vec`, whatever order the tasks finish in;
an empty `Vec` gives an empty one (or `Ok` of one). Both are always
followed by `.await` (V0212 otherwise).

- `Task::all` on tasks giving a `Result` stops at the first `Err` to
  arrive: it gives that `Err` at once and stops the tasks still running,
  so `Task::all(tasks).await?` passes it on. On other tasks it gives every
  value.
- `Task::all_settled` lets every task run to its end and keeps each
  `Ok` and `Err`. It needs tasks giving a `Result` (V0200, whose note
  names `Task::all`).

```varyk
// main.vr
async fn price(id: i64) -> Result<i64, string> {
    time::sleep(10).await;
    if id > 2 {
        return Err(format!("no price for {}", id));
    }
    Ok(id * 100)
}

async fn total(ids: Vec<i64>) -> Result<i64, string> {
    let prices = Task::all(ids.iter().map(|id| price(id)).collect()).await?;
    Ok(prices.iter().sum())
}

async fn main() {
    match total(vec![1, 2]).await {
        Ok(n) => println!("total {}", n),
        Err(e) => println!("{}", e),
    }
    let outcomes = Task::all_settled(vec![price(1), price(3)]).await;
    for outcome in outcomes {
        match outcome {
            Ok(p) => println!("ok {}", p),
            Err(e) => println!("failed: {}", e),
        }
    }
}
```

prints `total 300`, `ok 100`, and `failed: no price for 3`. An empty
`vec![]` of tasks has no type to take (V0207), and `Task` cannot be
written, so a list that may be empty is a collected `map`, as in `total`.

A program with an async `main`, an async test, a started call, a
`Task::all` or `Task::all_settled` call, or a `time::sleep` call uses
`varyk-std`, which holds the runtime. In the
generated Rust an async function is an `async fn` and `.await` is written
as it is; an async `main` is `fn main() { ::varyk_std::run(async { .. }) }`,
an async test the same under `#[test]`, and `time::sleep(ms)` is
`::varyk_std::time::sleep(ms)`. A started call `f(u, 3)` evaluates its
arguments in order and starts the task with them:
`match (u, 3,) { (varyk_0, varyk_1,) => ::varyk_std::Task::start(async move { f(&varyk_0, varyk_1).await }) }`,
each passed as its parameter takes it; a method is called by its path,
`User::load(&varyk_0)`, and a call with no arguments matches on `()`. A
task is `::varyk_std::Task<T>`, and `t.detach()` is written as it is.
`Task::all(ts).await` is `::varyk_std::Task::all(ts).await`, or
`::varyk_std::Task::try_all(ts).await` for tasks giving a `Result`, and
`Task::all_settled(ts).await` is `::varyk_std::Task::all(ts).await`.

### Shared

A started call needs its own copy of what it is given. To give one struct
to many tasks without a copy for each, share it:

| Call | Uses the value | Result |
|---|---|---|
| `Shared::new(value)` | takes `value`, a struct | `Shared<T>`: puts the struct where many can read it |
| `s.clone()` | reads `s` | `Shared<T>`: another handle to the same struct; the struct is not copied |

```varyk
// main.vr
struct Config {
    factor: i64,
    name: string,
}

impl Config {
    fn describe(self) -> string {
        format!("{} x{}", self.name, self.factor)
    }
}

async fn scale(config: Shared<Config>, n: i64) -> i64 {
    time::sleep(10).await;
    config.factor * n
}

async fn main() {
    let config = Shared::new(Config { factor: 3, name: "triple" });
    let tasks = vec![scale(config.clone(), 1), scale(config.clone(), 2)];
    let results = Task::all(tasks).await;
    println!("{} {}", config.describe(), results[0] + results[1]);
}
```

prints `triple x3 9`. Each task gets a handle of its own from
`config.clone()`, and all of them read the one `Config`.

- **What it holds.** `T` is a struct, one of the program's or one from a
  `.rs` module. `Shared::new` of anything else, a number, a `Vec`, an
  `Option`, or an enum, is an error (V0216), whether or not a type is
  written; to share such a value, put it in a field of a struct.
- **Reading through it.** The struct's fields and methods are reached
  straight through the handle: `config.factor`, `config.name.len()`,
  `config.describe()`, `match config.kind { .. }`, `for x in
  config.items { .. }`.
- **Read-only.** Everything reached through a `Shared` can only be read,
  as through a parameter without `mut`: assigning to it, passing it to a
  `mut` parameter, calling a `mut self` method or a changing call such as
  `push` on it is an error (V0310). Giving one of its fields to something
  that keeps it, such as `push` or a struct literal, needs a `.clone()`
  of the field (V0304), as for a parameter's.
- **Where it is written.** `Shared<T>` is written only as the type of a
  parameter or a `let`; in a field, a return type, or inside another type
  (`Vec<Shared<T>>`) it is an error (V0216). A `Vec` or `Option` of
  handles made with `vec!` or `Some` is fine.
- **Not a `T`.** A `Shared<Config>` cannot be passed where a `Config` is
  expected (V0200): pass the fields read through it instead. It cannot be
  printed or compared (V0203), and cannot go through `json` or `env`
  (V0210).
- **Given away.** `Shared::new` takes its value, and a started call takes
  a `Shared` held by a name as it takes any value (using the name later is
  V0305), so give each task `s.clone()` and keep `s`.

`Shared` alone does not make a program use `varyk-std`: in the generated
Rust, `Shared<T>` is std's `::std::sync::Arc<T>`, `Shared::new(v)` is
`::std::sync::Arc::new(v)`, `s.clone()` is written as it is and copies a
pointer, and fields and methods are reached through Rust's auto-deref.

## Attributes

An attribute is written `#[name]` or `#[name(value)]` on the line before
what it marks, or at the start of the same line. Several may stack. There
are four:

| Attribute | Goes before | Meaning |
|---|---|---|
| `#[rename("key")]` | a struct field, or a variant that carries no data | the name used in JSON and the environment instead of the Varyk name |
| `#[default(value)]` | a struct field | the value used when the key or variable is missing |
| `#[skip]` | a struct field | never written or read by `json` or `env` |
| `#[test]` | a top-level function | a test, run by `varyk test` |

```varyk
// main.vr
struct Config {
    #[rename("PORT")]
    #[default(8080)]
    port: u16,
    #[skip]
    cache: Option<string>,
}

#[test]
fn adds() {}

fn main() {}
```

Any other name, such as `#[derive(Clone)]` or `#[serde(..)]`, is an error
listing the four (`.clone()`, `==`, and JSON need no derive). So is an
attribute anywhere else (on a struct, an enum, an `impl` block, a method,
a `mod` or `use` line, a variant that carries data, or a field of a
variant), the same attribute twice, a missing value (`#[rename]`), or a
value where none goes (`#[skip(1)]`); all V0112. A `#` anywhere an
attribute cannot start is a syntax error, and so is `#!`.

`#[rename]` takes a non-empty string in quotes. `#[default]` takes one
literal that fits the field: a whole number in the range of an integer
field (`-1` fits `i32`, `300` does not fit `u8`), a number with a
fractional part for an `f32` or `f64` field (`1.0`, not `1`, as anywhere
else a number meets a float), a string in quotes for a `string` field, or
`true` or `false` for a `bool` field. `#[default]` cannot go on an `Option`
field, which is already `None` when missing, nor on a field of any other
type. These are checked on every struct, used or not (V0209).

`#[test]` marks a test (see [Tests](#tests)). `#[rename]`, `#[default]`,
and `#[skip]` act on the types a `json` or `env` call reaches (see
[JSON](#json) and [Configuration](#configuration)); on any other type they
change nothing.

## JSON

The `json` module reads and writes JSON. Its two calls are written with
the module's name, `json::parse(..)`; there is no `use json;`.

| Call | Uses the value | Result |
|---|---|---|
| `json::parse(text)`; `text: string` read | none | `Result<T, Error>`: a `T` read from the JSON text, or an `Err` saying what is wrong |
| `json::stringify(value)`; `value: T` read | none | new text: `value` as compact JSON |

```varyk
// main.vr
enum Role {
    Admin,
    #[rename("member")]
    Member,
}

struct User {
    id: u32,
    #[rename("userName")]
    user_name: string,
    role: Role,
    nickname: Option<string>,
    #[default(18)]
    age: u8,
    #[skip]
    password_hash: Option<string>,
}

fn load(body: string) -> Result<User, Error> {
    let u: User = json::parse(body)?;
    Ok(u)
}

fn main() {
    match load("{\"id\": 7, \"userName\": \"ann\", \"role\": \"member\"}") {
        Ok(u) => println!("{}", json::stringify(u)),
        Err(e) => println!("error: {}", e),
    }
}
```

This prints `{"id":7,"userName":"ann","role":"member","nickname":null,"age":18}`.

`T` for `json::parse` comes from where the result goes, as for `s.parse()`:
a `let` with a written type, an argument, a return value, a field, and
through `?`. The head of a `match` and the value a method is called on do
not give it one, so a result looked at with `match` is put in a `let`
first: `let r: Result<User, Error> = json::parse(body);`. With no type to
take it is V0207, which shows `let u: User = json::parse(..)?;`.

How values map:

- a struct is an object; its keys are the field names, or their
  `#[rename]`, written in declaration order; keys it does not have are
  ignored when read;
- a key that is missing when read is `None` for an `Option` field, the
  `#[default]` value if the field has one, and otherwise an error naming
  the key;
- `None` is written as `null`, and `null` reads as `None`;
- an enum whose variants carry no data is a string, the variant's name or
  its `#[rename]`;
- a `Vec` is an array, and a `HashMap<string, V>` an object (in no
  particular key order);
- numbers, `bool`, and `string` are themselves; a number out of range for
  its type is an error when read (`300` for a `u8`), never a crash.

A `#[skip]` field is never written and never read. A struct that
`json::parse` reaches must give each of its skipped fields a value: a
`#[default]`, or an `Option` type, which is `None` (V0209). Once renamed,
the keys of the fields that are not skipped must all be different, and so
must the keys of an enum's variants (V0209). These are checked only on the
types a `json` call reaches.

`json::stringify` cannot fail, so it gives a `string`, not a `Result`.
Only these types can go through JSON: numbers, `bool`, `string`; `Option`
and `Vec` of such a type; `HashMap<string, V>` of one; a struct whose
fields, apart from skipped ones, are such types; and an enum whose
variants carry no data. Anything else is an error at the call that names
the part in the way (V0210): an enum with a variant that carries data, a
`HashMap` whose key is not `string`, `Error`, or a Rust type from a `.rs`
module. For data that varies by kind, use a struct with a field of a
plain enum marked `#[rename("type")]` and an `Option` field for each
kind's data. The check follows the fields all the way down, and a type
that holds itself through a `Vec` is fine.

`json::stringify` and `json::parse` only read their argument, so the
value can be used after the call. In the generated Rust they are
`::varyk_std::json::stringify(&value)` and
`::varyk_std::json::parse::<User>(&text)`, and a type that a call reaches
derives serde's `Serialize`, `Deserialize`, or both, through `varyk-std`,
as the calls need; no other type gets them.

## Configuration

`env::parse()` fills a struct from the environment. It is written with the
module's name; there is no `use env;`.

| Call | Uses the value | Result |
|---|---|---|
| `env::parse()` | none | `Result<T, Error>`: a `T` read from the environment, or an `Err` naming the variable that is wrong |

```varyk
// main.vr
enum Mode {
    Dev,
    #[rename("live")]
    Live,
}

struct Config {
    port: u16,
    #[rename("db_url")]
    database_url: string,
    mode: Mode,
    token: Option<string>,
    #[default(30)]
    timeout_secs: u32,
}

fn load() -> Result<Config, Error> {
    let c: Config = env::parse()?;
    Ok(c)
}

fn main() {
    match load() {
        Ok(c) => println!("{} {}", c.port, c.timeout_secs),
        Err(e) => println!("error: {}", e),
    }
}
```

`T` comes from where the result goes, in the same places as for
`json::parse`; with none it is V0207, which shows
`let c: Config = env::parse()?;`, and a result looked at with `match` is
put in a typed `let` first.

`T` must be a struct whose fields, apart from skipped ones, are numbers,
`bool`, `string`, enums whose variants carry no data, or an `Option` of
one of those. A nested struct, a `Vec`, or a `HashMap` field, or a `T` that
is not a struct, is an error at the call naming the part in the way
(V0210); mark a field the program fills itself `#[skip]` and make it an
`Option`.

Each field reads the variable named by its key, the field's name or its
`#[rename]`, in upper case: a field `port` reads `PORT`, and the field
`database_url` above, renamed `db_url`, reads `DB_URL` (not `DATABASE_URL`). Two fields whose variables would be
the same are V0209. For each field, in order:

1. a variable set in the process environment wins;
2. otherwise, the value in `.env`, if that file has the variable;
3. otherwise, the field's `#[default]`, or `None` for an `Option`, or an
   `Err` such as `` `PORT` is not set ``.

A value is read as its field's type: a number from its text, `true` or
`false` for a `bool`, the variant's name or `#[rename]` for an enum, and
a `string` as written. One that does not read is an `Err` naming the
variable and the value (`` `PORT` is not a number: `abc` ``), never a
crash.

`.env` is read from the current directory, once, the first time it is
needed. It is a list of `KEY=value` lines; blank lines and lines starting
with `#` are skipped, and a value may be wrapped in single or double
quotes. A missing file is fine; a malformed line is an `Err` naming the
line. `.env` is never written into the process environment. Variable
expansion (`${OTHER}`) and several files (`.env.local`) are not supported.

In the generated Rust it is `::varyk_std::env::parse::<Config>()`, and the
struct derives serde's `Deserialize` through `varyk-std`.

## Strings

There is one string type, `string`. Behind it, the compiler picks either a
borrowed `&str` or an owned `String` for each name, and `--emit-rust` shows
which.

The one allocation rule: a string literal that is placed into a struct
field, an enum value, `Some`, `Ok`, `Err`, a `vec!`, or a `Vec` element,
passed to `push`, returned from a function, put into a name that must own
its text (such as a name that is later stored in a struct field), passed to
a parameter marked `mut string`, passed to a Rust function that takes a
`String`, or written as a branch of an `if` or `match` whose other branches
make new text (`if c { make() } else { "none" }`) is copied once, at the
line where the literal is written. `format!` and `s.clone()` make new text,
which a name holding it owns, and so do `to_uppercase`, `replace`, `join`,
and `remove` on a `Vec<string>`; a name that `push_str` changes owns its
text too. A name holding `trim()`, a piece `split` gives, or the text a function
returns part of a parameter as, borrows it as a `&str`; so does a chain
item a `map` gives as part of something. `split` itself copies nothing:
each piece is part of the text it is called on. `clone` is the one copy you write
yourself,
and in the generated Rust it is `.clone()`, or `.to_string()` when `s` is a
`&str`. Passing a name to a Varyk function never copies it. Nothing else
copies a string's text behind your back. A string that is already owned
moves instead, with no copy.

## Not in milestone 5b1

These do not exist yet; where one can be written, it is an error that names
what is not supported. They are left out because no program has needed
them so far (the [roadmap](roadmap.md) lists what is scheduled): on a
`Vec`, `first`, `last`, `clear`, `extend`, `truncate`, `dedup`, `reverse`,
`swap`, and `sort_by`; on a chain, `enumerate`, `zip`, `rev`, `take`,
`skip`, `fold`, `min`, `max`, and `position`; on a string, `ends_with`,
`to_lowercase`, `chars`, `bytes`, `lines`, `find`, `trim_start`,
`trim_end`, and `split_once`, and `char` as a type; on `Option` and
`Result`, `is_none`, `map` on a `Result`, `and_then`, `unwrap_or_else`,
`ok_or_else`, `unwrap_or_default`, and `as_ref`; on a `HashMap`, `remove`,
`is_empty`, and `clear`; `HashSet`, `BTreeMap`, and `VecDeque`; a `HashMap`
in a `.rs` signature (a facade returns a `Vec` or a struct); `iter()`,
`keys()`, `values()`, `split()`, and `trim()` on a value made right there;
`Box`; struct patterns on plain structs; tuples and tuple patterns, so no
`for (k, v) in map`; struct field shorthand; a `main` that returns a
`Result`; a function returning part of two parameters, or of a `mut`
parameter; a borrowed return from a Rust signature whose lifetimes Rust's
elision would not settle; and `Debug` with `{:?}`.

Also not yet, from the Rust side and the package side: modules declared
inside a `.rs` file; importing Rust tuple and unit structs; a `.rs`
signature naming a type declared in Varyk; importing a published Varyk
library straight into Varyk code (reach it through a `.rs` module, like any
crate); `pub(crate)` and `pub(super)`; `use` with braces or globs;
re-exports (`pub use`) in either direction; settings taken from a Cargo
workspace; custom target paths; reading `[features]`; sharing cargo's
`target/` between `varyk build` and `cargo build`; `varyk init` into an
existing project; and a `varyk` command for `cargo doc`, which works
unchanged through cargo once `init` has run.

Also not yet, from the batteries: TOML; pretty-printed JSON; JSON for
enums that carry data (write a `type` field); `flatten`, aliases, and
custom formats in JSON; structured log fields (`log::info("x", id = 1)`),
log targets, and spans; `.env` variable expansion and several `.env`
files; a kind or a cause on `Error`, and automatic conversion into `Error`
from other error types at `?`; attributes on structs, methods, and
variants with data; `#[default]` on an `Option`, enum, or struct field;
`varyk_std::Error` in a `.rs` signature; and HTTP and the database, which
are milestone 5b2.

Also not yet, from async code: `race` and `any`; timeouts; channels;
changing a value shared between tasks (a `Mutex`); a task anywhere but where
it is made, `push` of a task among them; `Shared` of anything but a struct,
or in a field or a return type; async closures, and `.await` in a closure;
async recursion; an async function returning part of a parameter; cancelling
a task by hand; a runtime the program configures (thread count,
current-thread); and inferred async, where the compiler works out which
functions wait.

These are left out by design: closures as values (function types, and a
closure in a `let`, a parameter, a return, or a field), `move`, a type on a
closure's parameter, and a closure that changes a name from outside it;
`for_each` (write a `for`); iterators as values of their own (a chain is
finished where it is written); an `Option` holding part of a stored value
as a value of its own (look inside `get` and `find` where they are made);
indexing a `HashMap`; `loop`; guards, `|`, `..`, and `@` in patterns, and
`let else`; ranges anywhere but the head of a `for`; `+` on strings (use
`format!`); `Self`; `Copy` structs; traits, generics, and attributes
other than the four under [Attributes](#attributes);
derives beyond `Clone` and `PartialEq`, so `println!` stays an error on
structs, enums, `Option`, `Result`, `Vec`, and `HashMap`; a method that
takes `self` by value, or any other way to write "give this away"; `impl`
blocks for built-in types; a `use` of an enum variant; naming a crate from
Varyk code (write a `.rs` facade instead: a Rust file of the package that
wraps what the program needs from the crate in plain functions, as
"Calling Rust" shows); and everything planned for later milestones, such
as HTTP, databases, `varyk fmt`, and a language server.

`unwrap` and `expect` are never added, in this or any later milestone: a
call that stops the program when a value is absent defeats the purpose of a
language for services, and `match`, `if let`, `?`, and `unwrap_or` cover
every use.

Names are ASCII only for now (letters, digits, and `_`); string text can be
any Unicode.

## Calling Rust

A module can be a Rust file. `mod greet;` next to `greet.rs`:

```rust
// greet.rs
pub fn hello(name: &str) -> String {
    format!("Hello from Rust, {}!", name)
}
```

```varyk
// main.vr
mod greet;

fn main() {
    let name = "Varyk";
    println!("{}", greet::hello(name));
}
```

The `.rs` file is copied into the build as it is. It may use Rust's standard
library and, in a package, the crates its `[dependencies]` names; it may not
declare modules of its own or include other files. This is how a package
uses a crate: Varyk code never names one, so a `.rs` module in the package,
a *facade*, wraps what the program needs in plain functions, structs, and
enums, and Varyk imports those (`examples/packages/matcher` wraps
`regex-lite` this way). `varyk check` reads its `pub fn` signatures, its
`pub struct`s and their methods, and its `pub enum`s (not their methods
yet: call those from a plain `pub fn` instead), and stops there; the Rust
inside it is checked by rustc when you build. If rustc rejects it, the error
or warning is shown exactly as rustc worded it, at your `.rs` file and line
— it is your Rust, in your terms. This is different from a problem in the
Rust Varyk itself generates: if that is ever rejected, which should not
happen apart from the known limits under "Rust enums" below and the
thread rule of V0901 (also below), the failure is reported as a Varyk diagnostic, V0900, at the Varyk
line that produced it, carrying rustc's message and asking you to report it
as a bug (`--emit-rust` shows the generated code the report points at).
When rustc also rejected one of your `.rs` files, a V0900 may only follow
from that error, so it says so instead and comes after it. Its
top-level `pub fn` items can be called from Varyk when every parameter and
the return type is one of these:

| Rust type | In Varyk |
|---|---|
| `bool`, `i8` to `i64`, `u8` to `u64`, `usize`, `f32`, `f64` | the same type, passed by value |
| `&str` | a borrowed `string` |
| `&mut String` | a `mut string` |
| `String` parameter | an owned `string`: a literal is copied, an owned string moves |
| `String` return | `string` |
| `&str` or `&S` return, where lifetime elision names the parameter it borrows from (below) | a borrowed return of that argument: `string`, or the struct or enum |
| `S`, a struct or enum imported from a `.rs` file of the package (below) | that struct or enum, given away (moved) |
| `Vec<T>`, `Option<T>`, `Result<T, E>` where `T` and `E` are in this table | the same Varyk type, given away (moved) |
| `&T` or `&mut T` where `T` is one of the value types above but `String` | borrowed, or `mut` |
| `()` return, or none | nothing |

Any other signature cannot be called: `&String` (take `&str` instead),
generics, lifetimes, trait objects, `HashMap` and other `std` types,
references in the return type other than the ones just above (return an
owned value such as `String`),
`()` inside another type (`Result<(), String>`; use `bool` or a struct
instead), and unknown types. Calling such a function is an error that shows its Rust
signature and what to change. `unsafe fn`, `const fn`, trait
methods, names a `pub use` brings in, and functions, methods, structs, and
enums marked `pub(crate)`, `pub(super)`, `pub(self)`, or `pub(in ..)` are
not imported: Varyk imports only plain `pub`. A function, method, struct,
or enum behind `#[cfg(..)]` or `#[cfg_attr(..)]`, or a function or method
with a parameter or `self` behind one, may not exist in the build, so it is
not imported either, and neither is a `#[test]` function. Calling or naming any of these is an error with a note
saying why; for a `pub(..)` function the note says to make it plain `pub`. A module that uses a glob import
(`use ...::*`) cannot expose functions with these built-in parameter or
return types, because the glob could redefine any of their names; replace
the glob with the names the file needs. The same goes for a top-level macro
call in the file that could define names: a call of a `macro_rules!` of the
same file (at any depth, inside an inline module too) whose text, or the
text the call gives it, includes `struct`, `enum`, `union`, `type`, `use`,
`trait`, `fn`, `mod`, `impl`, or `Drop`, or calls another macro; and a call
of any macro not defined in the file (`crate::make!()`, a macro a `use`
brings in, one of another file or crate, or any after `#[macro_use] extern
crate`), except `thread_local!`. Such a call could define any name, so the
error names the macro and its line; move the macro and its uses to another
`.rs` file. A call of a macro of the file's own where none of that text
has those words or calls a macro, a call of `thread_local!` whose text has
none of those words, and a `macro_rules!` that is never called as an item
change nothing.

A `pub async fn` and an `async` method are imported under the same rules
and called as a Varyk async function is: awaited with `.await`, or started
into a task (see [Two ways to call](#two-ways-to-call)), and only from an
async function. One that returns a reference cannot be called (V0108):
return an owned value instead, as an async Varyk function does.

```rust
// fetch.rs
pub async fn price(id: i64) -> i64 {
    id * 10
}
```

```varyk
// main.vr
mod fetch;

async fn main() {
    let t = fetch::price(2);
    println!("{}", fetch::price(1).await + t.await);
}
```

It prints `30`.

A started call may run on another thread, so everything its task is given,
and everything it holds while it waits, must be able to go there. Every
type Varyk declares can; a type from Rust code may not (in Rust terms, it
is not `Send`, or not `Sync`: an `Rc`, a `Cell`, or a `RefCell` inside it).
Only rustc can tell, so `varyk check` accepts such a program and `varyk
build` reports V0901 at the started call, naming the Rust type: use `Arc`
in place of `Rc` and `Mutex` or an atomic in place of `Cell` or `RefCell`
in the Rust code, or await the call instead of starting it.

A Rust type is found two ways: a bare name is an item of the same `.rs`
file, and a full path, `crate::other::Thing`, is an item of another `.rs`
file of the package. A name brought in by a `use` line in the `.rs` file is
not followed (write the full path), and a `.rs` file cannot name a type
declared in Varyk yet.

### Rust structs and methods

A `pub struct` with named fields and no type or lifetime parameters in a
`.rs` file is a Varyk struct, `matcher::Matcher`:

```rust
// matcher.rs
pub struct Matcher {
    words: Vec<String>,
    pub hits: u32,
}

impl Matcher {
    pub fn new(pattern: &str) -> Matcher {
        let words = pattern.split('|').map(|word| word.to_string()).collect();
        Matcher { words, hits: 0 }
    }

    pub fn is_match(&self, s: &str) -> bool {
        self.words.iter().any(|word| word == s)
    }

    pub fn bump(&mut self) {
        self.hits += 1;
    }
}
```

```varyk
// main.vr
mod matcher;

fn main() {
    let mut m = matcher::Matcher::new("red|green");
    if m.is_match("red") {
        m.bump();
    }
    println!("{}", m.hits);
}
```

Its fields follow the struct field rules, with the Rust `pub`:

- a `pub` field whose type is in the table above is a field like any other;
- a field without `pub` (or with `pub(crate)`) cannot be seen from Varyk;
- a `pub` field whose type is not in the table is there but cannot be
  used: reading or assigning it is an error naming its Rust type;
- a struct literal, `matcher::Matcher { .. }`, needs every field visible
  and usable; otherwise make one with a function of the Rust file, such as
  `new`.

The `pub fn` items of `impl Matcher` blocks in the same file (any number of
them) are its methods and associated functions, and `Self` means the
struct. `&self` is Varyk's `self`, and `&mut self` is `mut self`: calling
one needs a `let mut`. Parameters and returns follow the table. A `&self`
method that returns `&str` or `&S`, for an imported struct or enum `S`
(`&Self` too), returns part of `self`, whatever its other parameters, and
the result is an alias of the receiver, as a Varyk function's borrowed return
is: it cannot be kept in a struct or an element. A `pub fn` that returns `&str` or `&S` and has exactly
one reference parameter, a `&T` and not a `&mut T`, returns part of that
argument the same way. These are the shapes Rust's lifetime elision resolves
to one parameter, so a lifetime written anywhere in the signature, such as
`fn pick<'a>(&self, other: &'a str) -> &'a str`, is not imported. Any other
borrowed return (`&mut S`, a `&mut self` method, `Option<&T>`, `&[T]`,
`&String`, a function with two reference parameters) cannot be called.
A method that takes `self` by value or has type
parameters cannot be called, and methods of trait implementations are not
imported. Of a Rust struct's or enum's `#[derive(..)]` lists, Varyk reads
`Clone` and `PartialEq`, written as bare names, and nothing else: they let
`.clone()` and `==` work on the type and on the Varyk types that hold it
(see [Types](#types)). A hand-written `impl Clone` or `impl PartialEq` is
not seen; `.clone()` or `==` on such a type is an error saying to derive
the trait instead. Other derives are ignored: a Rust struct that derives
`Copy` is still given away when passed by value.

A tuple struct, a unit struct, a struct with type or lifetime parameters,
a `#[repr(packed)]` struct (whose fields cannot be borrowed), or a struct
with no fixed size (its last field a slice, `str`, or `dyn` type) is not
imported; naming one is an error that says why.

### Rust enums

A `pub enum` with no type or lifetime parameters, whose variants are all
unit or tuple variants with types from the table above, is a Varyk enum:
nameable, constructible, and matchable exactly like one declared in Varyk,
including exhaustiveness.

```rust
// kind.rs
pub enum Kind {
    Word(String),
    Number(i32),
}

pub fn classify(text: &str) -> Kind {
    match text.parse::<i32>() {
        Ok(n) => Kind::Number(n),
        Err(_) => Kind::Word(text.to_string()),
    }
}
```

```varyk
// main.vr
mod kind;

fn describe(k: kind::Kind) -> string {
    match k {
        kind::Kind::Word(text) => text.clone(),
        kind::Kind::Number(_) => "a number",
    }
}

fn main() {
    println!("{}", describe(kind::classify("42")));
}
```

A variant with named fields, a variant holding a type not in the
table, or a variant behind `#[cfg(..)]` or `#[cfg_attr(..)]`, or with a field
behind one (which may not exist in the build), makes the whole enum opaque: still a real type, so it can be held
in a `let`, a field, a `Vec`, an `Option`, or a signature, and passed
around and returned, but none of its variants can be named. Constructing
or matching one is an error that says why the enum is opaque. A generic
or lifetime-parameterized enum is not imported at all; naming it is an
error that says why, like a tuple or unit struct. Derives are ignored.

When an enum, Rust or Varyk, has an `impl Drop` in one of the package's
`.rs` files, Rust cannot move a payload out of it, so a `match`, `if let`,
or `while let` on a call that returns one, or returns an `Option`, a
`Result`, or an enum holding one at any depth, only looks inside it, as a
`match` on a stored value does: its bindings belong to the enum and cannot be kept (V0304) or
changed; copy a `string` one with `.clone()` to keep or change it. Varyk looks for the `impl Drop` anywhere in
a `.rs` file, inside modules and functions too, and when it cannot tell
which enum one is for (a type renamed with `as` or `type`, `Drop` itself
renamed, re-exported with `pub use`, or brought in by `use std::ops::*`, or a
macro, defined or called anywhere in the file, even as an argument of
`println!`, whose text includes the word `Drop`, or any other macro call
that could write one: as an item, a call that could define names by the
rule above; inside a function or a `const` block, or as an argument of
another call, a call of a macro of the file's own by the same rule, or of
any macro not defined in the file other than `println!`, `print!`,
`eprintln!`, `eprint!`, `format!`, `vec!`, `assert!`, `assert_eq!`,
`assert_ne!`, `debug_assert!`, `debug_assert_eq!`, `debug_assert_ne!`,
`panic!`, `write!`, `writeln!`, `dbg!`, `matches!`, `todo!`,
`unimplemented!`, `unreachable!`, `concat!`, `stringify!`, `env!`,
`line!`, `file!`, `column!`, `thread_local!`, `cfg!`, `format_args!`,
`include_str!`, `include_bytes!`, `option_env!`, `compile_error!`, and
`module_path!`, which count only when a call they are
given counts; a path `std::println!` or `core::println!` is one of these
only when nothing in the file, an item or a `use`, is named `std` or
`core`, while `::std::println!` always is) it treats every enum this way; an `impl Drop` written by a
dependency's derive or attribute macro is not seen. Then the build stops with a V0900 carrying
rustc's E0509 at the `match`; copy the binding with `.clone()` in its arm,
or only read it there. The same is the one known limit for type names: a
dependency's derive or attribute macro can define a type, such as a
`String` of its own, that a signature of the file then names, and Varyk
does not see it, so the build stops with rustc's error instead of `check`.
Keep such macros out of facade files, or write the facade's signatures with
primitive types only.

A Rust type named by a `.rs` item must be visible wherever that item
is used: a Rust function or method whose signature names a type behind a
module without `pub` can be called only where that module can be seen
(V0108 elsewhere), a `pub` field of such a type can be read or assigned
only there (V0108 elsewhere), and a variant holding one makes its enum
opaque. The message
names the module and says which `pub mod` fixes it.

## The command line

```text
varyk check [file.vr]                            check for errors; never runs cargo
varyk build [file.vr] [--release] [--emit-rust]  generate and build; print the executable's path
varyk run [file.vr] [--release] [-- args...]     build, then run with the given arguments
varyk test [file.vr]                             build the tests and run them
varyk emit [file.vr] --out-dir DIR               check, then write the generated tree to DIR;
                                                  never runs cargo
varyk init [dir] [--lib]                         write a package that plain cargo build compiles
varyk add [cargo add args]                       run cargo add in the package
varyk publish [--assemble-only] [-- cargo args]  check, assemble a plain Rust crate, and run
                                                  cargo publish there
```

Without a file, each command works on the package found by looking for
`Cargo.toml` in the current directory and then upward; with no package there
it stops with "no Varyk package here; name a `.vr` file (as in `varyk run
main.vr`) or run inside a package; `varyk init` makes one" (`varyk publish`,
which takes no file, says to run it inside a package),
and when the `Cargo.toml` it finds has neither `src/main.vr` nor
`src/lib.vr`, it stops with "no Varyk package here", naming that
`Cargo.toml` and what is missing. `varyk run` on a library
stops with "this package is a library; it has nothing to run", and
`varyk build` on a library builds it and prints nothing.

- `--release` builds with optimizations. As in Rust, integer arithmetic
  that overflows stops the program in a normal build but wraps around
  silently in a `--release` build.
- `--emit-rust` prints every generated file (`Cargo.toml` and the Rust
  files), each after a line naming it, and then builds.
- `--message-format=json` works with every command and prints errors, and
  the warnings rustc gives about your `.rs` modules, as JSON on standard
  output, one object per line, instead of the text form; under `varyk run`
  and `varyk test`, whose standard output is the program's or the test
  runner's own, they go to standard error instead. The
  error, each of its labels, and its fix-it each name the file they point
  into. Lines count from 1; `column` fields count bytes from the start of the
  line, also from 1. A Rust compiler error in one of your `.rs` modules is
  an object with `level`, `file` (your file), `line`, `column`, `message`,
  `rustc_code` (`null` when rustc gives none), and `notes`, rustc's
  `help:` and `note:` lines, each a string starting with its level (`help:
  use ...`); any other compiler
  message is an object with `level` and `message`. Errors of the command
  itself (no package here, a library to run, a file that cannot be read,
  `cargo` that cannot be started) stay one line of text on standard error,
  and so does cargo failing before it compiles anything, followed by
  cargo's own words.

`run` exits with the program's exit code, and `test` with the test
runner's (see [Tests](#tests)); a Rust error while building the tests is
reported as under `build`. For a single file, the generated
Rust project lives in the build directory, `target/varyk/` under the current
directory (so in your source tree if you run `varyk` from the `.vr` file's
directory). For a package it is `target/varyk/<name>/` under the package's
directory, with cargo's build output in `target/varyk/cache/` beside it, and
the program is named after the package. An error or warning rustc reports
in a `.rs` module you wrote is shown at your file and line, unchanged; a
rustc error in the code Varyk itself generated, which should not happen,
is reported as V0900 at the Varyk line responsible (see "Calling Rust").

### `varyk init` and plain `cargo build`

`varyk init [dir]` writes a new package in `dir` (the current directory
if you leave it out), named after that directory: `Cargo.toml`,
`.gitignore` (`/target` and `.env`, so a local secrets file is never
committed), `build.rs`, `src/main.rs`, and `src/main.vr` (a hello-world
program). The `Cargo.toml` lists `varyk-std = "X.Y.Z"` under
`[dependencies]`, the compiler's own version, since a program that uses
`Error` or another `varyk-std` feature needs it. `varyk init --lib` writes
`src/lib.rs` and `src/lib.vr` (one `pub fn`) instead, and prints one line,
"created the package `<name>` in `<dir>`; run `varyk run` or `cargo run`" (`build` for a library, and "`cd <dir>`, then"
first when you gave a directory other than `.`, in single quotes if it has
a space or another character a shell treats specially). The name is the directory's name made
a valid crate name (`my app` becomes `my_app`). It refuses to run, and
writes nothing, if the other kind's root file exists there (`src/main.vr`
for `--lib`, `src/lib.vr` otherwise), since a package has only one, or else
if any of these five files already exists there, listing them; when the
directory is already a Varyk package, it says so instead. To add
Varyk to an existing Rust project, run `varyk init` in an empty directory,
move its `build.rs` (merging by hand if you already have one) and
`src/main.vr` into your project, and replace your `src/main.rs` with the
stub after moving its code into a `.rs` module: the stub brings in the
generated program, which has its own `fn main`. The package needs
`edition = "2024"`. A Rust project with nested modules cannot adopt Varyk
yet, since a `.rs` module cannot declare modules of its own.

The package `init` writes builds two ways: `varyk build`/`varyk run`, as
above, and plain `cargo build`, with `varyk` on the `PATH` (or the `VARYK`
environment variable naming its path, e.g. `VARYK=/path/to/varyk cargo
build`). The stub `src/main.rs` is one line,
`::std::include!(::std::concat!(::std::env!("OUT_DIR"),
"/varyk/src/main.rs"));`, naming the standard macros by full path so that
no macro of the package's own can take their place (a stub an older
`init` wrote, without the `::std::`, still works), with no
attribute of its own, so the `#[allow(warnings, arithmetic_overflow,
unconditional_panic)]` Varyk writes on every item it generates is the only
lint setting in force either way,
besides `#[allow(non_snake_case)]` on the `mod` line of a `.vr` module
whose name is not in snake case, which Rust applies to that module and the
modules inside it.
`build.rs` runs `varyk emit --out-dir $OUT_DIR/varyk` to produce that
tree before `src/main.rs` is compiled, and declares
`cargo:rerun-if-changed=src` and `cargo:rerun-if-env-changed=VARYK`, so
editing a `.vr` file and running `cargo build` again picks up the
change. If the emit step fails on a Varyk error, `build.rs` exits
non-zero and cargo shows its stderr, which is Varyk's normal
diagnostic rendering followed by "run `varyk build` to see this without
cargo's wrapping"; if `varyk` cannot be found at all, `build.rs` fails
with one line, "varyk was not found (tried `varyk`); install it with
`cargo install varyk` or set VARYK to its path", naming `$VARYK` instead when it is
set. Rustc errors under plain `cargo build`
show `OUT_DIR` paths and are not mapped back to Varyk source; run
`varyk build` for that.

`varyk emit [file.vr] --out-dir DIR` checks the target (a package or a
single file, exactly as `check` locates it) and writes its generated
`src/` tree under `DIR` (so `DIR/src/main.rs`, not `DIR/main.rs`); it
never writes a `Cargo.toml` and never runs cargo. `DIR/src/` is
cleared of anything Varyk did not generate there, so `emit` only writes
into a directory it owns: a new or empty one (it then leaves a
`.varyk-generated` marker file in `DIR`), one it wrote before, or one
under `target/varyk/` or cargo's `OUT_DIR`, judged after following
symbolic links further up its path. It refuses, writing nothing, any
other `DIR` that is not empty; a `DIR` whose `src/` holds any file of
the program being emitted (so `--out-dir .` inside a package is an
error); a `DIR` or `DIR/src` that is itself a symbolic link; and a
`DIR` written with `..` in it. It is the command `build.rs` calls, and
exists as its own command mainly for that use.

### `varyk add` and upgrading

`varyk add [cargo add args]` runs `cargo add` with exactly the arguments
you give, in the package found upward from the current directory (so a
relative `--path` is relative to the package), and passes cargo's output and
exit code through; Varyk interprets none of the arguments. Outside a
package it says "no Varyk package here". `cargo add` needs the
`src/main.rs` (or `src/lib.rs`) stub that `varyk init` writes; in a package
without one, add the line to `Cargo.toml` by hand.

A program that uses `varyk-std` (it names `Error`, calls one of its
features, such as `json`, `log`, `time::sleep`, or `Task::all`, starts a call, or has an
async `main` or an async test; `Shared` alone, which is std's `Arc`, does
not count) needs `varyk-std` in `[dependencies]`, as `"X.Y"` or `"X.Y.Z"`
(a leading `^` is fine) or a table with such a `version` and no `path`,
`git`, `optional`, or `package`, where `X.Y` is the compiler's version
(`~`, `=`, `>=`, `*` and lists are refused); and if `Cargo.lock` locks
`varyk-std`, a version no older than the compiler's. `varyk check` says
which of these fails (V0404) and the line to write. To upgrade, run
`cargo install varyk`, then whatever `varyk check` asks for: change the
line, or run `cargo update -p varyk-std`. A single file has no
`Cargo.toml`; its generated crate depends on the compiler's exact version.

### `varyk publish`

`varyk publish [-- cargo args]` always works on the package found
upward from the current directory; there is no single-file form. It
runs the full check, assembles a plain Rust crate at
`target/varyk/package/<name>/` under the package, and runs `cargo
publish` there, forwarding every argument after `--` untouched
(`--dry-run`, `--allow-dirty`, `--token`, and the rest are cargo's; Varyk
interprets none of them) along with cargo's exit code and its output.
`varyk publish --assemble-only` stops after the assembly and prints the
crate's directory, running no cargo and publishing nothing.

The assembled crate holds the generated tree of `varyk build` (so
`src/main.rs`/`src/lib.rs` is the real generated root, not the `init`
stub, and every module sits at its place), every `.vr` source alongside
the file it produced, for readers, the manifest with `[package] build =
false` and the same `[dependencies]` path rewrite `varyk build` uses, and
the files the manifest's `readme` and `license-file` name, plus any
`README*`/`LICENSE*` at the package root. It has no `build.rs`, so it
needs no `varyk`: a consumer adds it to `[dependencies]` like any crate
and builds it with plain `cargo build`, and `cargo install` of a
published Varyk binary works the same way. A Varyk package consuming a
published Varyk library reaches it through a facade like any crate;
direct import is an open question.

## Error codes

Every error has a code. A code is never reused for a different meaning.

| Code | Meaning |
|---|---|
| V0001 | a construct Varyk does not support yet; the message names it. Also an `if`, a block, or a field or element of a value made right there as the value a `match`, `if let`, or `while let` looks at or a `for` goes over, and a pattern taking a struct apart; a `for` over a `HashMap` (use `keys()` or `values()`); a `get` looked into on a value made right there (store it with `let` first), and so the value `trim` is called on, or the argument a function returns part of, made right there; a field of a call as such a head; a closure anywhere but as the argument of a call that takes one (the message names those calls), a type on a closure's parameter or `move`, and `return`, `break`, `continue`, or `?` inside a closure; `iter()`, `split()`, `keys()`, or `values()` on a value made right there (store it with `let` first) |
| V0002 | unexpected token or malformed syntax |
| V0003 | a bad escape in a string, or a string with no closing `"` |
| V0010 | `&x` or `&mut x` written at a call; Varyk works out references itself |
| V0011 | `&T` or `&mut T` written in a parameter type or a return type; write `name: T` or `mut name: T`, and `-> T` (`-> string` for `-> &str`) |
| V0012 | lifetime syntax such as `<'a>` or `&'a T`; lifetimes are worked out by the compiler |
| V0100 | unknown name, or a variant, method, or associated function the type does not have; for `Vec`, `string`, `Option`, `Result`, `HashMap`, and a chain the message lists their calls; for an imported struct, a note says when the `.rs` file has the method but Varyk does not import it (a trait method, `unsafe`, `const`, behind `#[cfg]`, or `pub(crate)` or another `pub(..)`), and likewise for a function or `pub use` name of a `.rs` module and for any method of an imported enum, which Varyk does not import yet; a path into an inline `mod` of a `.rs` file, whose items Varyk does not read, says so; also naming a variant of an opaque imported enum, saying why it is opaque; `.clone()` on a number or `bool`, which is copied on use |
| V0101 | unknown type, or `Option`, `Result`, `Vec`, or `HashMap` with the wrong number of types, a `HashMap` key type that is not an integer type, `bool`, or `string`, or a Rust struct or enum Varyk does not import (a tuple or unit struct, one with type or lifetime parameters, a `#[repr(packed)]` struct or one with no fixed size, one behind `#[cfg]`, or one marked `pub(crate)` or another `pub(..)`), or a type a `pub use` of the `.rs` file brings in, saying why |
| V0102 | unknown field, of a struct or of a variant with named fields, in a value or a pattern |
| V0103 | a name defined more than once (a method included, a field of a variant, a field named twice in a value or a pattern, or a name twice in one pattern), or a reserved or built-in type name used as a name, or a binding named after a unit variant of its own enum (`Point` where `Shape::Point` is meant) |
| V0104 | a module file that is missing, present as both `.vr` and `.rs` or as both `shop.vr` and `shop/mod.vr`, unreadable, a `.rs` file that cannot be parsed as Rust, or named `main` or `lib` (or `bin` in the entry file), in any capitalization; a `.rs` file that uses a crate not in `[dependencies]` (or only in `[dev-dependencies]`, or any crate in a single file) in a `use` or `extern crate` item (a crate named only in a path, `other::f()`, is rustc's to report, at build), declares a module of its own, or uses `include!`, shown at that line of the `.rs` file |
| V0105 | an item, method, or associated function used from outside its module without `pub`; a path through a module declared without `pub`; a `pub` item or field naming a type some of its users cannot see; a private struct field read, assigned, or named in a literal from outside its module; a literal of a Rust struct with a field Varyk cannot see or use; a Rust function marked `pub(crate)` (or another `pub(...)`) rather than plain `pub` |
| V0106 | a missing or malformed `fn main()`, `main` defined in a library's `src/lib.vr`, or a call to an async `main` |
| V0107 | `String` or `str` written where `string` is meant |
| V0108 | a Rust function or method whose signature Varyk cannot call, including one naming a type its callers cannot see; the message shows the signature and what to change. Also a `pub` field of a Rust struct whose Rust type Varyk cannot use (or cannot see), read or assigned, a Rust type reached through a `use` line in the `.rs` file rather than its full path, and a type in a `.rs` file with a glob `use` or a macro that could define names |
| V0109 | a struct or enum that contains itself, directly or through other structs, enums, `Option`, or `Result`; a `Vec` or `HashMap` breaks the cycle |
| V0110 | a `use` naming a crate this compiler recognizes by name (`std`, `core`, `alloc`, and in a package every crate in `[dependencies]`); call a crate from a `.rs` module in the package instead |
| V0111 | a path Varyk cannot follow: `super` in the entry file, a `use` ending at an enum variant or at a type's method or associated function, a `use` whose leading name, or whose only name (`use shop;`), is a module declared elsewhere in the package (write it from `crate::` or `super::`), or a `use` whose leading name another `use` made |
| V0112 | an attribute Varyk does not have (the message lists the four; `derive` gets a note that `.clone()`, `==`, and JSON need none), one in a place it cannot go (the note says where it goes), the same attribute twice on one item, or a value missing (`#[rename]`, `#[default]`) or not expected (`#[skip(1)]`, `#[test(1)]`) |
| V0113 | the name `Error`, the standard error type, or `Task` or `Shared`, the standard types of async code, given to a struct, an enum, a module, or a `use`, or to a `pub` struct or enum of a `.rs` module; `json`, `env`, `log`, or `time`, the standard modules, given to a module, a struct, an enum, or a `use`, or a `use` of one (`use json;`, `use json::parse;`); `assert` or `assert_eq` given to a function or a `use`; a function, method, struct, enum, module, or `use` name starting with `varyk_`, kept for what Varyk adds to the Rust it writes |
| V0114 | a `#[test]` function with parameters or a return type, a call to or `use` of one, or `main` of the entry file marked `#[test]`; `assert` or `assert_eq` outside a `#[test]` function |
| V0200 | type mismatch, including `+` on strings (use `format!`), `as` on something that is not a number, another number type meeting a `usize`, indexing something that is not a `Vec`, `match` arms of different types, a `for` over something that is not a `Vec` or a range, and a range whose ends are not integers of one type; `sort` on floats or structs, `contains` on a `Vec` of structs, `join` on a `Vec` of anything but strings, and `parse` into anything but a number or `bool`; `sum` on a chain of items that are not numbers, and a closure of `filter`, `any`, `all`, or `find` that does not give a `bool`; `Task::all` or `Task::all_settled` given anything but a `Vec` of tasks, and `Task::all_settled` on tasks that do not give a `Result`; a `Shared` given where the struct it holds is expected |
| V0201 | wrong number of arguments, or of values in an enum value; a variant value with named fields that leaves one out, or with the wrong kind of brackets; a closure with more or fewer than one parameter |
| V0202 | `println!`, `format!`, or a `log` call with the wrong number of `{}`, or something other than `{}` in braces; a `log` call whose text is not a string literal written in quotes |
| V0203 | `{}` used on anything but a number, `bool`, string, or `Error`; `==`, `!=`, or `.clone()` on a type that cannot be compared or copied, naming the field in the way and, for a Rust type, saying to derive the trait in its `.rs` file; `==` or `!=` on a `Shared`, or on a type holding one |
| V0204 | a `match` that does not handle every value: a variant at any depth, a `bool` value, or, on a number or a string, the catch-all it always needs; the message names a value shape it misses |
| V0205 | a pattern that does not fit the value: a variant of another type, the wrong number of positions in a variant, a variant pattern leaving out a named field, a literal or range of another type or not fitting it, a range whose ends are reversed, a float literal, or a string literal inside another pattern; an arm that can never run, such as one after `_` or a name; or a `match`, `if let`, or `while let` on something that is not an enum, `Option`, `Result`, number, `bool`, or string |
| V0206 | `?` in a function that does not return a `Result` or an `Option`, on a `Result` in a function returning an `Option` or the reverse, or on a value that is not a `Result` with the function's error type |
| V0207 | a `None`, `Vec::new()`, `HashMap::new()`, empty `vec![]`, `Ok`, `Err`, `parse()`, `json::parse(..)`, or `env::parse()` whose type cannot be worked out where it is written, `Err(e)?;`, `text.parse().ok()`, and a closure giving one with nothing to take its type from included; write the type in a `let` |
| V0208 | a value that must be used where it is made: an `Option` from `get`, or from `find` on a chain of borrowed items, holding part of a stored value, stored in a `let`, passed, returned, used with `?`, given any method, or named whole by a pattern (look inside it with `match` or `if let`); an unfinished chain anywhere but as the value the next call of the chain is made on or the head of a `for` (finish the chain there) |
| V0209 | a `#[rename]` value that is not a string in quotes, or is empty; a `#[default]` value that does not fit its field's type (`"x"` on an `i32`, `300` on a `u8`, `1` on an `f64`); `#[default]` on an `Option` field or on a field that is not a number, `string`, or `bool`; on a type a `json` call reaches, a skipped field with no `#[default]` that is not an `Option` when the type is read, or two fields that are not skipped, or two variants, with the same key once renamed, and, on a type `env::parse` reaches, two fields whose upper-cased keys are the same variable |
| V0210 | a type that cannot go through JSON at a `json::parse` or `json::stringify` call, or be read from the environment at an `env::parse` call (a `T` that is not a struct, or a field that is a struct, `Vec`, or `HashMap`): an enum with a variant that carries data, a `HashMap` whose key is not `string`, `Error`, a Rust type from a `.rs` module, a `Shared`, or a `Result`, anywhere inside it apart from skipped fields; the message names the part in the way |
| V0211 | a call to an async function, or `.await`, in a function that is not `async`; `.await` inside a closure |
| V0212 | `.await` after something that is not a call to an async function, `Task::all`, `Task::all_settled`, or a name holding a task; `Task::all` or `Task::all_settled` without `.await` |
| V0213 | a started call anywhere but a `let` with a name, the value `.detach()` is called on, a `vec!` element, or the value of a collected `map`'s closure, `let _ =` included; a name holding a task, or a `Vec` of tasks, that nothing awaits, detaches, or gives to `Task::all` or `Task::all_settled`: its task would be thrown away |
| V0214 | async functions that call each other in a cycle, or an async function that calls itself, whether the calls are awaited or started; the message names the cycle |
| V0215 | a name holding a task used other than by `.await` or `.detach()`, or a `Vec` of tasks used other than by `Task::all` or `Task::all_settled` (indexed, given `push`, looped over, passed, returned, given to another name, or followed by `.await`); `Task` written as a type |
| V0216 | `Shared` of anything but a struct, at `Shared::new` or written, or `Shared` written anywhere but a parameter's or a `let`'s type (a field, a return type, or inside another type) |
| V0300 | changing a parameter that was declared without `mut`, by assigning to it or calling `push` or `pop` on it |
| V0301 | changing a `let` name that was declared without `mut`, or a name a `match` pattern or a `for` made, by assigning to it or calling `push` or `pop` on it; also changing, inside a closure, a name from outside it or the closure's parameter |
| V0302 | a `let` name without `mut` passed to a `mut` parameter or used to call a `mut self` method |
| V0303 | a parameter without `mut`, or a name a `match` pattern or a `for` made, passed to a `mut` parameter or used to call a `mut self` method; also, inside a closure, a name from outside it or the closure's parameter so passed |
| V0304 | a value the function only borrows, stored in a struct, an element, an enum value, `Some`, `Ok`, `Err`, or a `vec!`, passed to `push`, used with `?`, or returned; a stored `Option` or `Result` with more than numbers and `bool`s inside, used up by `unwrap_or`, `ok_or`, or `ok`; returns that mix part of a parameter with something new, or that are part of a `let` of the function, of a number or `bool` parameter or `for` variable, of a `mut` parameter, or of a parameter of a function that calls itself; also a binding of a `match` on an enum that runs code when it is thrown away (an `impl Drop`), kept or given away; a name from outside a closure kept inside it, or given by a closure of `map` or `map_err`, as is part of its parameter; `collect` on a chain of borrowed items (copy them with `.map(\|w\| w.clone())`); the item a closure of `filter`, `any`, `all`, or `find` looks at, kept or given away; a chain's `map` closure giving part of an owned item, or something new beside a part; a value the function only borrows given to a started call, whose task keeps it, or a task detached inside a closure; for text the fix is `.clone()` |
| V0305 | a value used after it was given away, to a started call's task among others, or a task awaited or detached twice, or a `Vec` of tasks given to `Task::all` or `Task::all_settled` twice |
| V0306 | a later argument changes or gives away a value that an earlier argument of the same call still borrows, or uses the value a method is called on while the method may change it, as in `v.push(v.len())`, or an index changes the `Vec` it indexes, as in `v[g(v)]` with `g` taking `mut v` |
| V0307 | a value changed or given away while another name for part of it is still used later (a name bound inside a looked-into `get` or `find`, or the result of a call returning part of it, included), or inside a `for` that goes over it or whose head reads it (the argument of `split`, or a name a closure of a chain in the head reads) |
| V0308 | a function returning part of one parameter in one place and part of another in another, or a closure giving parts of two names (a chain's `map` closure giving part of its item and part of a name from outside included); return a copy in one of them (in both, for a chain's `map`), or make two functions |
| V0309 | a started call passing a value to a `mut` parameter, or calling a `mut self` method: the task would change only its own copy |
| V0310 | changing something reached through a `Shared`: assigning to it, passing it to a `mut` parameter, or calling a `mut self` method or a changing call such as `push` on it |
| V0311 | an async function that returns part of a parameter; return a copy instead |
| V0400 | a package whose `Cargo.toml` does not say `edition = "2024"` |
| V0401 | something in `Cargo.toml` Varyk does not support yet: a key outside the fixed set `check` reads, a `src/bin/`, `examples/`, `tests/`, or `benches/` directory or the root file of the other kind (`src/lib.rs` beside `src/main.vr`), which cargo would build as further targets, a setting taken from a Cargo workspace (`workspace = true`), dependencies for only some platforms (`[target.'cfg(..)'.dependencies]`), `links`, which needs a build script the published crate does not carry, a custom `build` script, `[lints]`, or `[patch]` or `[replace]`, in the package or in the root manifest of an enclosing workspace |
| V0402 | a Cargo target table (`[lib]`, `[[bin]]`, `[[example]]`, `[[test]]`, `[[bench]]`), not supported yet: a package is one program from `src/main.vr` or one library from `src/lib.vr`, found by cargo's defaults |
| V0403 | a `Cargo.toml` that cannot be used: it cannot be read, is not valid TOML, has a top-level key cargo reads as a table (`workspace`, `dependencies`, `features`, ...) that is not one, has no `[package]` `name`, names with `workspace` a directory that has no workspace manifest or sits under a `Cargo.toml` whose `workspace` is not a table, or its `name` is not letters, digits, `-`, and `_` starting with a letter or `_`, or a `[package]` key has a value of the wrong shape (`license = 1`), or its `rust-version` is not `MAJOR.MINOR[.PATCH]`, or is `cache` or `package`, or a program (not a library) is called `deps`, `examples`, `build`, or `incremental` in any case, the names of Cargo's own build folders, or its `version` is present but not text of the form `MAJOR.MINOR.PATCH` that cargo accepts; or its package has both `src/main.vr` and `src/lib.vr` (from the command line, a `Cargo.toml` found but whose package has neither is reported before any check runs: "no Varyk package here" and why, in one line) |
| V0404 | the `varyk-std` dependency of a program that uses it is missing, comes from a `path` or `git`, is `optional` or renamed, has a requirement that is not `X.Y` or `X.Y.Z` (optionally `^`) on the compiler's version, or `Cargo.lock` locks an older `varyk-std`; the note gives the line to write, or `cargo update -p varyk-std` |
| V0900 | rustc rejected the Rust code Varyk generated, which should not happen, except for the known limits listed under "Calling Rust"; the message carries rustc's own message and code and asks you to report it, or, when rustc also rejected a `.rs` module you wrote, says it may follow from that error. An error or warning in a `.rs` module you wrote is not this code: it is shown at your file, unchanged |
| V0901 | at `varyk build`: a started call whose task holds a value from Rust code that cannot be sent to, or shared with, another thread (an `Rc`, `Cell`, or `RefCell` inside it); the message names the Rust type |
