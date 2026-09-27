# The Varyk language

This page describes everything Varyk accepts today, in milestone 3 of an
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
`self`, `crate`, `super`, `use`, `true`, and `false`. `self`, `crate`, and
`super` also start a path: `self::`, `crate::`, and `super::`.

Every other Rust keyword is reserved: `as`, `async`, `await`, `const`,
`dyn`, `extern`, `loop`, `move`, `ref`, `Self`, `static`, `trait`, `type`,
`unsafe`, `where`, `abstract`, `become`, `box`, `do`, `final`, `gen`,
`macro`, `override`, `priv`, `try`, `typeof`, `unsized`, `virtual`, and
`yield`. Using one is an error that says the construct is not supported
yet, except `as` in `use path as name;`. None of them can be used as a
name.

`Some`, `None`, `Ok`, `Err`, `Option`, `Result`, `Vec`, and `String` are
reserved names: no function, method, struct, enum, variant, field, module,
parameter, or `let` name can be any of them, because the generated Rust
would then hide Rust's own. A struct, enum, or module also cannot take a
built-in type's name (`i32`, `string`, `str`, and so on); a function, field,
or local can (`let string = "x";` is fine).

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

An enum lists its variants. A variant is a plain name or carries values of
the types in parentheses:

```varyk
enum Shape {
    Circle(f64),
    Rect(f64, f64),
    Point,
}
```

An enum cannot contain itself, directly or through a struct, another enum,
or an `Option` or `Result`; inside a `Vec` it can:
`enum Tree { Node(Vec<Tree>) }`.

An enum value is written with the enum's name and the variant, followed by
the variant's values in parentheses when it has any: `Shape::Point`,
`Shape::Circle(2.0)`, `Shape::Rect(1.0, 2.0)`. An enum of another module is
reached through it: `geo::Shape::Point`. The number of values must match the
variant, so `Shape::Circle()`, `Shape::Point(1.0)`, and `Shape::Circle`
without its value are errors, as is a variant the enum does not have.

`Option`, `Result`, and `Vec` need exactly their types: `Option<i32>`,
`Result<User, string>`, `Vec<Option<Shape>>`. A `string` inside them is an
owned `String` in the generated Rust. Their values are written as in Rust:

```varyk
let some = Some(5);               // Option<i32>
let none: Option<i32> = None;
let ok: Result<i32, string> = Ok(1);
let failed: Result<i32, string> = Err("no");
let numbers = vec![1, 2, 3];      // Vec<i32>; every element has one type
let empty: Vec<string> = vec![];
```

`Some(x)` and `vec![a, b]` know their type from what is inside. `None`,
`Vec::new()`, an empty `vec![]`, `Ok(x)` (its error type), and `Err(e)` (its
value type) take their type from where they go: a `let` with a written type,
a parameter, a return value, a field, or a `vec!` element after one whose
type is known. Anywhere else, write the type in a `let` first; the compiler
never works it out from later lines.

Numbers and `bool` are called Copy types: using one makes a copy, and the
original stays usable. `string`, structs, enums, `Option`, `Result`, and
`Vec` are not Copy.

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

### Calls on `Vec` and `string`

These are all the calls `Vec` and `string` have. "Reads" borrows the value
the call is made on, "changes" needs a value that may be changed, like a
`mut` parameter.

| Call | Uses the value | Result |
|---|---|---|
| `Vec::new()` | none | an empty `Vec`; its type comes from where it goes, as for `vec![]` |
| `v.push(x)` | changes | nothing; `x` is kept in `v` |
| `v.pop()` | changes | `Option<T>`: the last element, taken out, or `None` |
| `v.len()` | reads | `usize`, the number of elements |
| `v[i]` | reads, or changes when assigned | the element at `i`, which is a `usize` |
| `s.len()` | reads | `usize`, the length of the text in bytes |
| `s.clone()` | reads | a new copy of the text |

`push` keeps what it is given, like a struct field: a literal string is
copied in, a name holding its own value is given away, and a value the
function only borrows is an error. `v[i]` is a place, like a field: it can
be read, assigned (`v[0] = 5;`), have its fields read or assigned, and have
methods called on it (`tasks[0].complete()`). With `i` past the end, the
program stops with Rust's message. `s.clone()` is the one way to copy text,
and it always makes owned text.

`Option` and `Result` have no calls; the rest of Rust's calls on these types
are not available yet.

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
for i in 0..n { }       // i takes 0, 1, ..., n - 1
for item in items { }   // every element of a Vec, in order
break;                  // leave the innermost `while` or `for`
continue;               // go to the next round of the innermost `while` or `for`
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
`if`/`else`, `match`, and `?` after a `Result` (see
[Passing errors on](#passing-errors-on)).

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
- `==` and `!=` compare numbers, `bool`, and strings. Structs, enums,
  `Option`, `Result`, and `Vec` cannot be compared.
- `&&` is "and", `||` is "or".

Precedence, from tightest to loosest: `-` and `!` first; then `*`, `/`, and
`%`; then `+` and `-`; then the comparisons; then `&&`; then `||`. Operators
of the same level group from left to right, so `10 - 3 - 2` is `5`.
Comparisons cannot be chained: `a < b < c` is an error. Use parentheses when
in doubt.

## Matching

`match` looks at an enum, an `Option`, or a `Result` and runs the arm whose
pattern fits. Like `if`, it is an expression:

```varyk
fn area(shape: Shape) -> f64 {
    match shape {
        Shape::Circle(r) => 3.14 * r * r,
        Shape::Rect(w, h) => w * h,
        Shape::Point => 0.0,
    }
}
```

Each arm is `pattern => expression,`; the comma may be left out after a
block. Every arm must have the same type, or no arm has a value (a
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
  `Ok(x)`, or `Err(e)`, with one name or `_` for each of the variant's
  values. A name may appear only once in a pattern.

Patterns are one level deep: `Some(Shape::Point)` is an error (match on the
inner value inside the arm), and so are literal patterns, guards
(`pattern if condition`), `a | b`, `..`, and `if let`. To look at a number,
a `bool`, or a string, use `if`.

Every variant must have an arm, or the last arm must be `_` or a name; the
error names a variant that is missing. `_` or a name anywhere but last is an
error, since the arms after it could never run.

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
  given away (V0307), and it cannot be stored or returned (use `.clone()`
  for text). An arm that uses no such name may change the stored value.
- Matching a value made right there owns it: the names hold their own
  values and can be given away, pushed, or returned.

None of the names a pattern makes can be changed. To change a copy, first
make a changeable one: `let mut n = n;`; to change the stored value, change
it by its own name.

So an `Option<Task>` kept in a `let` cannot give its `Task` away: `match`
only looks inside it. Match on the call that made it instead:
`match tasks.pop() { Some(task) => done.push(task), None => {} }`.

A `match` used as a value follows the rule of `if`: when every arm gives
something stored, the result is another name for it; when every arm gives a
new value, the result owns it. Mixing the two is an error for text, and
wherever the value is only read in place, such as in a call or a
`println!`; store it with `let` first.

In the generated Rust, a stored value is matched by reference (`match
&shape`), and a copied name is copied at the start of its arm with
`let n = *n;`.

## Loops

`while` repeats while its condition is true. `for` goes over a range of
integers or the elements of a `Vec`:

```varyk
for i in 0..n { }        // i takes 0, 1, ..., n - 1
for task in tasks { }    // every element of tasks, in order
```

A range `a..b` counts from `a` up to, but not including, `b`. Both ends are
integers of one type, and a written number takes the other end's type, so
`0..tasks.len()` counts in `usize`. Ranges exist only in the head of a
`for`; `..=` is an error. Nothing else can be looped over: not a string,
not an `Option`, not a range of anything but integers (V0200). `break` and
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
  made from an element: it cannot be stored or returned (use `.clone()` for
  text).
- Looping over a value made right there owns it: the variable holds each
  element in turn and can be given away, pushed, or returned.

The loop variable can never be changed. To change a copy, first make a
changeable one: `let mut n = n;`; to change the stored `Vec`, change it by
its own name, after the loop.

In the generated Rust, a stored `Vec` is looped over by reference (`for x
in &v`), and a copied element is copied at the start of the body with
`let x = *x;`.

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

Anything else is V0206: `?` in a function that does not return a `Result`
(`main` never does, so use `?` in a helper and `match` on its result there),
on a value that is not a `Result`, or on a `Result` whose error type is
different; an error is never converted into another type. `?` on an
`Option` comes in a later milestone; use `match`.

`?` can be used anywhere a value can: in a `let`, in an argument, in a
`match` arm, in a loop, or as a statement of its own (`check(text)?;`).
It takes what it is given, like a return does: a `Result` made right there
is used up, a name holding one is given away (using it again is V0305), and
a `Result` the function only borrows, such as a parameter, a field, or an
element, cannot be used with `?` (V0304). In the generated Rust, `r?` is
written as it is.

## Printing

`println!` prints a line. The first argument is a string literal, and each
`{}` in it is replaced by the next argument:

```varyk
println!("{} is {} years old", name, age);
```

The number of `{}` must match the number of arguments. Write `{{` and `}}`
to print `{` and `}`. Nothing may go inside the braces. The arguments must be
numbers, `bool`, or strings; a struct, an enum, an `Option`, a `Result`, or a
`Vec` cannot be printed whole.

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
`vec!`, passed to `push`, or returned from the function; the error says what
to do instead. For example, `fn name(user: User) -> string { user.name }` is
an error; for text, the fix is a copy, `user.name.clone()`.

For Rust readers: `user: User` becomes `user: &User`, `mut user: User`
becomes `user: &mut User`, and the call sites get `&` and `&mut`. In Rust,
`mut name: T` means an owned parameter that can be rebound; Varyk uses `mut`
for the mutable borrow instead, and no Varyk-declared parameter takes
ownership of a string or struct. Copy types are passed by value.

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
which a name holding it owns; `clone` is the one copy you write yourself,
and in the generated Rust it is `.clone()`, or `.to_string()` when `s` is a
`&str`. Passing a name to a Varyk function never copies it. Nothing else
copies a string's text behind your back. A string that is already owned
moves instead, with no copy.

## Not in milestone 3

These do not exist yet; where one can be written, it is an error that names
what is not supported. They are left out of this milestone (the
[roadmap](roadmap.md) lists what is scheduled): modules declared inside a
`.rs` file; importing Rust tuple and unit structs; using what an imported
Rust struct derives (`Clone`, `PartialEq`, `Debug`); a `.rs` signature naming a type declared in
Varyk; importing a published Varyk library straight into Varyk code (reach
it through a `.rs` module, like any crate); `pub(crate)` and `pub(super)`;
`use` with braces or globs; re-exports
(`pub use`) in either direction; settings taken from a Cargo workspace;
custom target paths; reading `[features]`; sharing cargo's `target/`
between `varyk build` and `cargo build`; `varyk init` into an existing
project; and `varyk` commands for `cargo test`, `cargo doc`, and
`cargo add`, which work unchanged through cargo once `init` has run. Still
missing from milestone 2: enum variants with named fields; patterns inside
patterns and literal patterns; `match` on numbers, `bool`, and strings;
`..=`; `?` on an `Option`; any call on `Option` or `Result`; `is_empty`,
`remove`, and `clear` on a `Vec`; and any call on a string but `len` and
`clone`.

These are left out by design: returning part of a value the function only
borrows; closures and function types; iterators; `if let`, `while let`, and
`loop`; guards, `|`, `..`, ranges, and `@` in patterns; ranges anywhere but
the head of a `for`; `+` on strings (use `format!`); `as` outside `use`;
tuples; `Self`; traits; derives of any kind, so `==` and `println!` stay
errors on structs, enums, `Option`, `Result`, and `Vec`; `clone` on
anything but a string; `HashMap`; `Box`; a `main` that returns a `Result`;
a method that takes `self` by value, or any other way to write "give this
away"; `impl` blocks for built-in types; a `use` of an enum variant;
naming a crate from Varyk code (write a `.rs` facade instead: a Rust file
of the package that wraps what the program needs from the crate in plain
functions, as "Calling Rust" shows); and everything planned for later milestones, such
as `varyk fmt`, a language server, and `async`.

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
happen apart from the known limits under "Rust enums" below, the failure is reported as a Varyk diagnostic, V0900, at the Varyk
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
| `S`, a struct or enum imported from a `.rs` file of the package (below) | that struct or enum, given away (moved) |
| `Vec<T>`, `Option<T>`, `Result<T, E>` where `T` and `E` are in this table | the same Varyk type, given away (moved) |
| `&T` or `&mut T` where `T` is one of the value types above but `String` | borrowed, or `mut` |
| `()` return, or none | nothing |

Any other signature cannot be called: `&String` (take `&str` instead),
generics, lifetimes, trait objects, `HashMap` and other `std` types,
references in the return type (return an owned value such as `String`),
`()` inside another type (`Result<(), String>`; use `bool` or a struct
instead), and unknown types. Calling such a function is an error that shows its Rust
signature and what to change. `unsafe fn`, `async fn`, `const fn`, trait
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
one needs a `let mut`. Parameters and returns follow the table. A method
that takes `self` by value, returns a borrow such as `&str`, or has type
parameters cannot be called, and methods of trait implementations are not
imported. Derives are ignored: a Rust struct that derives `Copy` is still
given away when passed by value.

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
`.rs` files, Rust cannot move a payload out of it, so a `match` on a call
that returns one only looks inside it, as a `match` on a stored value
does: its bindings belong to the enum and cannot be kept (V0304) or
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
varyk emit [file.vr] --out-dir DIR               check, then write the generated tree to DIR;
                                                  never runs cargo
varyk init [dir] [--lib]                         write a package that plain cargo build compiles
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
  output, one object per line, instead of the text form; under `varyk run`,
  whose standard output is the program's own, they go to standard error
  instead. The
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

`run` exits with the program's exit code. For a single file, the generated
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
`.gitignore`, `build.rs`, `src/main.rs`, and `src/main.vr` (a hello-world
program). `varyk init --lib` writes `src/lib.rs` and `src/lib.vr` (one
`pub fn`) instead, and prints one line, "created the package `<name>` in `<dir>`; run
`varyk run` or `cargo run`" (`build` for a library, and "`cd <dir>`, then"
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
| V0001 | a construct Varyk does not support yet; the message names it. Also an `if`, a block, or a field or element of a value made right there as the value a `match` looks at or a `for` goes over |
| V0002 | unexpected token or malformed syntax |
| V0003 | a bad escape in a string, or a string with no closing `"` |
| V0010 | `&x` or `&mut x` written at a call; Varyk works out references itself |
| V0011 | `&T` or `&mut T` written in a parameter type; write `name: T` or `mut name: T` |
| V0012 | lifetime syntax such as `<'a>` or `&'a T`; lifetimes are worked out by the compiler |
| V0100 | unknown name, or a variant, method, or associated function the type does not have; for `Vec` and `string` the message lists their calls; for an imported struct, a note says when the `.rs` file has the method but Varyk does not import it (a trait method, `unsafe`, `const`, `async`, behind `#[cfg]`, or `pub(crate)` or another `pub(..)`), and likewise for a function or `pub use` name of a `.rs` module and for any method of an imported enum, which Varyk does not import yet; a path into an inline `mod` of a `.rs` file, whose items Varyk does not read, says so; also naming a variant of an opaque imported enum, saying why it is opaque |
| V0101 | unknown type, or `Option`, `Result`, or `Vec` with the wrong number of types, or a Rust struct or enum Varyk does not import (a tuple or unit struct, one with type or lifetime parameters, a `#[repr(packed)]` struct or one with no fixed size, one behind `#[cfg]`, or one marked `pub(crate)` or another `pub(..)`), or a type a `pub use` of the `.rs` file brings in, saying why |
| V0102 | unknown field |
| V0103 | a name defined more than once (a method included, or a name twice in one pattern), or a reserved or built-in type name used as a name, or a binding named after a unit variant of its own enum (`Point` where `Shape::Point` is meant) |
| V0104 | a module file that is missing, present as both `.vr` and `.rs` or as both `shop.vr` and `shop/mod.vr`, unreadable, a `.rs` file that cannot be parsed as Rust, or named `main` or `lib` (or `bin` in the entry file), in any capitalization; a `.rs` file that uses a crate not in `[dependencies]` (or only in `[dev-dependencies]`, or any crate in a single file) in a `use` or `extern crate` item (a crate named only in a path, `other::f()`, is rustc's to report, at build), declares a module of its own, or uses `include!`, shown at that line of the `.rs` file |
| V0105 | an item, method, or associated function used from outside its module without `pub`; a path through a module declared without `pub`; a `pub` item or field naming a type some of its users cannot see; a private struct field read, assigned, or named in a literal from outside its module; a literal of a Rust struct with a field Varyk cannot see or use; a Rust function marked `pub(crate)` (or another `pub(...)`) rather than plain `pub` |
| V0106 | a missing or malformed `fn main()`, or `main` defined in a library's `src/lib.vr` |
| V0107 | `String` or `str` written where `string` is meant |
| V0108 | a Rust function or method whose signature Varyk cannot call, including one naming a type its callers cannot see; the message shows the signature and what to change. Also a `pub` field of a Rust struct whose Rust type Varyk cannot use (or cannot see), read or assigned, a Rust type reached through a `use` line in the `.rs` file rather than its full path, and a type in a `.rs` file with a glob `use` or a macro that could define names |
| V0109 | a struct or enum that contains itself, directly or through other structs, enums, `Option`, or `Result` |
| V0110 | a `use` naming a crate this compiler recognizes by name (`std`, `core`, `alloc`, and in a package every crate in `[dependencies]`); call a crate from a `.rs` module in the package instead |
| V0111 | a path Varyk cannot follow: `super` in the entry file, a `use` ending at an enum variant or at a type's method or associated function, a `use` whose leading name, or whose only name (`use shop;`), is a module declared elsewhere in the package (write it from `crate::` or `super::`), or a `use` whose leading name another `use` made |
| V0200 | type mismatch, including `+` on strings (use `format!`), another number type meeting a `usize`, indexing something that is not a `Vec`, `match` arms of different types, a `for` over something that is not a `Vec` or a range, and a range whose ends are not integers of one type |
| V0201 | wrong number of arguments, or of values in an enum value |
| V0202 | `println!` or `format!` with the wrong number of `{}`, or something other than `{}` in braces |
| V0203 | `{}`, `==`, or `!=` used on a struct, an enum, `Option`, `Result`, or `Vec` |
| V0204 | a `match` that does not handle every variant; the message names one it misses |
| V0205 | a pattern that does not fit the value: a variant of another type, the wrong number of names in a variant, `_` or a name that is not the last arm, or a `match` on something that is not an enum, `Option`, or `Result` |
| V0206 | `?` in a function that does not return a `Result`, or on a value that is not a `Result` with the function's error type |
| V0207 | a `None`, `Vec::new()`, empty `vec![]`, `Ok`, or `Err` whose type cannot be worked out where it is written; write the type in a `let` |
| V0300 | changing a parameter that was declared without `mut`, by assigning to it or calling `push` or `pop` on it |
| V0301 | changing a `let` name that was declared without `mut`, or a name a `match` pattern or a `for` made, by assigning to it or calling `push` or `pop` on it |
| V0302 | a `let` name without `mut` passed to a `mut` parameter or used to call a `mut self` method |
| V0303 | a parameter without `mut`, or a name a `match` pattern or a `for` made, passed to a `mut` parameter or used to call a `mut self` method |
| V0304 | a value the function only borrows, stored in a struct, an element, an enum value, `Some`, `Ok`, `Err`, or a `vec!`, passed to `push`, used with `?`, or returned; also a binding of a `match` on an enum that runs code when it is thrown away (an `impl Drop`), kept or given away; for text the fix is `.clone()` |
| V0305 | a value used after it was given away |
| V0306 | a later argument changes or gives away a value that an earlier argument of the same call still borrows, or uses the value a method is called on while the method may change it, as in `v.push(v.len())`, or an index changes the `Vec` it indexes, as in `v[g(v)]` with `g` taking `mut v` |
| V0307 | a value changed or given away while another name for part of it is still used later, or inside a `for` that goes over it |
| V0400 | a package whose `Cargo.toml` does not say `edition = "2024"` |
| V0401 | something in `Cargo.toml` Varyk does not support yet: a key outside the fixed set `check` reads, a `src/bin/`, `examples/`, `tests/`, or `benches/` directory or the root file of the other kind (`src/lib.rs` beside `src/main.vr`), which cargo would build as further targets, a setting taken from a Cargo workspace (`workspace = true`), dependencies for only some platforms (`[target.'cfg(..)'.dependencies]`), `links`, which needs a build script the published crate does not carry, a custom `build` script, `[lints]`, or `[patch]` or `[replace]`, in the package or in the root manifest of an enclosing workspace |
| V0402 | a Cargo target table (`[lib]`, `[[bin]]`, `[[example]]`, `[[test]]`, `[[bench]]`), not supported yet: a package is one program from `src/main.vr` or one library from `src/lib.vr`, found by cargo's defaults |
| V0403 | a `Cargo.toml` that cannot be used: it cannot be read, is not valid TOML, has a top-level key cargo reads as a table (`workspace`, `dependencies`, `features`, ...) that is not one, has no `[package]` `name`, names with `workspace` a directory that has no workspace manifest or sits under a `Cargo.toml` whose `workspace` is not a table, or its `name` is not letters, digits, `-`, and `_` starting with a letter or `_`, or a `[package]` key has a value of the wrong shape (`license = 1`), or its `rust-version` is not `MAJOR.MINOR[.PATCH]`, or is `cache` or `package`, or a program (not a library) is called `deps`, `examples`, `build`, or `incremental` in any case, the names of Cargo's own build folders, or its `version` is present but not text of the form `MAJOR.MINOR.PATCH` that cargo accepts; or its package has both `src/main.vr` and `src/lib.vr` (from the command line, a `Cargo.toml` found but whose package has neither is reported before any check runs: "no Varyk package here" and why, in one line) |
| V0900 | rustc rejected the Rust code Varyk generated, which should not happen, except for the known limits listed under "Calling Rust"; the message carries rustc's own message and code and asks you to report it, or, when rustc also rejected a `.rs` module you wrote, says it may follow from that error. An error or warning in a `.rs` module you wrote is not this code: it is shown at your file, unchanged |
