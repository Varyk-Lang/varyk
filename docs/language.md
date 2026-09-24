# The Varyk language

This page describes everything Varyk accepts today, in milestone 1 of an
experimental, pre-0.1 language (see [roadmap.md](roadmap.md) for what comes
next). Anything not described here is rejected with
an error that names what is not supported. For the reasons behind the design,
see [design.md](design.md).

Varyk code is compiled to Rust. You do not need to know Rust to read this
page. Notes marked "For Rust readers" say what the generated Rust looks like
and can be skipped.

## Files and modules

A Varyk program is a file ending in `.vr`. This is the entry file, and it
must define a function called `main` that takes nothing and returns nothing.
The program starts there.

```varyk
fn main() {
    println!("Hello, world!");
}
```

A program can be split into modules. `mod math;` in the entry file loads the
module `math` from `math.vr` or `math.rs` in the same directory. A `.rs` file
is Rust code; see "Calling Rust" below. Having both files is an error, and a
module cannot be called `main` or `lib`.

Inside a module, only items marked `pub` can be used from outside it. Use
them through the module name:

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

A top-level item is a function (`fn`), a struct (`struct`), or a module
declaration (`mod name;`), each optionally preceded by `pub`. All fields of a
`pub struct` are public. Only the entry file may declare modules and only the
entry file may define `main`.

A `pub struct` from a module cannot yet be named from the file that imports
it, neither as a type nor in a struct literal (`m::S` is not supported in
milestone 1); you can only receive one from a function.

## Comments, keywords, and reserved words

A comment starts with `//` and runs to the end of the line.

The keywords are `fn`, `pub`, `let`, `mut`, `struct`, `mod`, `if`, `else`,
`while`, `break`, `continue`, `return`, `true`, and `false`.

Every other Rust keyword is reserved: `as`, `async`, `await`, `const`,
`crate`, `dyn`, `enum`, `extern`, `for`, `impl`, `in`, `loop`, `match`,
`move`, `ref`, `self`, `Self`, `static`, `super`, `trait`, `type`, `unsafe`,
`use`, `where`, `abstract`, `become`, `box`, `do`, `final`, `gen`, `macro`,
`override`, `priv`, `try`, `typeof`, `unsized`, `virtual`, and `yield`. Using
one is an error that says the construct is not supported yet. None of them
can be used as a name.

## Types

| Type | What it holds |
|---|---|
| `bool` | `true` or `false` |
| `i8`, `i16`, `i32`, `i64` | whole numbers that can be negative, in 8, 16, 32, or 64 bits |
| `u8`, `u16`, `u32`, `u64` | whole numbers that cannot be negative |
| `f32`, `f64` | numbers with a fractional part |
| `string` | text |
| a struct you declare | a group of named fields |

A struct declares its fields and their types:

```varyk
struct User {
    name: string,
    age: i32,
}
```

A struct value is written with its name and every field:
`User { name: "Alice", age: 30 }`. Fields are read and changed with a dot:
`user.name`.

Numbers and `bool` are called Copy types: using one makes a copy, and the
original stays usable. `string` and structs are not Copy.

## Literals

- Whole numbers: `0`, `42`. Negative numbers use `-`: `-4`. An underscore
  may separate digits: `1_000`.
- Fractional numbers: `2.5`. Digits are required on both sides of the dot.
- `true` and `false`.
- Strings in double quotes: `"Hello"`. The escapes are `\n` (new line),
  `\t` (tab), `\\` (backslash), `\"` (double quote), and `\0` (zero byte).
  No other escapes and no raw strings.

A number literal takes the type its place expects, such as the declared type
of a `let` or a parameter. Where nothing is expected, a whole number is `i32`
and a fractional one is `f64`.

## Statements

```varyk
let x = 5;              // a new name; the type is worked out from the value
let y: i64 = 5;         // the same, with the type written out
let mut count = 0;      // `mut` lets the name be given a new value later
count = count + 1;      // assignment to a `let mut` name
user.name = "Bob";      // assignment to a field of a `let mut` name
return x;               // leave the function with a value
while count < 10 { }    // repeat while the condition is true
break;                  // leave the innermost `while`
continue;               // go to the next round of the innermost `while`
print_user(user);       // any expression followed by `;`
```

A name can be declared again with a new `let`; the new one hides the old one
from then on.

## Expressions and operators

Expressions are literals, names, module paths (`math::square`), function
calls, field access (`user.name`), struct values, parentheses, blocks, and
`if`/`else`.

A block `{ ... }` runs its statements, and if it ends with an expression and
no `;`, that expression is the block's value. A function body works the same
way, so `fn add(a: i32, b: i32) -> i32 { a + b }` returns `a + b`.

`if`/`else` is an expression, so it can produce a value:

```varyk
let label = if age >= 18 { "adult" } else { "child" };
```

Conditions must be `bool`. Chains are written `if ... { } else if ... { } else { }`.
A struct value inside an `if` or `while` condition needs parentheses.

Operators:

- `-x` negates a number; `!b` is "not" for a `bool`.
- `+ - * / %` work on numbers only, and both sides must have the same type.
  `+` does not join strings yet.
- `< <= > >=` compare numbers of the same type.
- `==` and `!=` compare numbers, `bool`, and strings. Structs cannot be
  compared.
- `&&` is "and", `||` is "or".

Precedence, from tightest to loosest: `-` and `!` first; then `*`, `/`, and
`%`; then `+` and `-`; then the comparisons; then `&&`; then `||`. Operators
of the same level group from left to right, so `10 - 3 - 2` is `5`.
Comparisons cannot be chained: `a < b < c` is an error. Use parentheses when
in doubt.

## Printing

`println!` prints a line. The first argument is a string literal, and each
`{}` in it is replaced by the next argument:

```varyk
println!("{} is {} years old", name, age);
```

The number of `{}` must match the number of arguments. Write `{{` and `}}`
to print `{` and `}`. Nothing may go inside the braces. The arguments must be
numbers, `bool`, or strings; a struct cannot be printed whole, so print its
fields.

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
name, a `mut` parameter, a field of one of those, or a fresh value such as
`User { name: "Alice" }` or a call result. The compiler says so if it is not,
and suggests where to add `mut`.

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
The same happens when a name is stored in a struct field, returned, or
passed to a Rust function that takes a `String`.

A name made with `let` from a parameter, or from any field, is another name
for that same value, not a copy. Changing it changes the
original. In milestone 1, Rust's checker may reject a function that uses the
original and this other name together.

**What a function only borrows, it cannot keep.** A string or struct
parameter belongs to the caller. A field that holds a string or a struct
belongs to the struct around it, even when that struct is your own. The same
goes for a `let` name made from one of these. Such a value cannot be stored
into a struct field or returned from the function. For example,
`fn name(user: User) -> string { user.name }` is an error in milestone 1.

For Rust readers: `user: User` becomes `user: &User`, `mut user: User`
becomes `user: &mut User`, and the call sites get `&` and `&mut`. In Rust,
`mut name: T` means an owned parameter that can be rebound; Varyk uses `mut`
for the mutable borrow instead, and no Varyk-declared parameter takes
ownership of a string or struct. Copy types are passed by value.

## Strings

There is one string type, `string`. Behind it, the compiler picks either a
borrowed `&str` or an owned `String` for each name, and `--emit-rust` shows
which.

The one allocation rule: a string literal that is placed into a struct field,
returned from a function, put into a name that must own its text (such as
a name that is later stored in a struct field), passed to a parameter marked
`mut string`, or passed to a Rust function that takes a `String` is copied
once, at the line where the literal is written. Passing a name to a Varyk
function never copies it. Nothing else copies a string's text behind your
back. A string that is already owned moves instead, with no copy.

## Not in milestone 1

These do not exist yet, and using them is an error: enums, `match`, `for` and
`loop`, closures, traits, `impl` blocks and methods, generics, `Option`,
`Result`, `?`, lists such as `Vec`, attributes and derives, `use`, Cargo
dependencies, modules inside modules, per-field `pub`, joining strings with
`+`, an explicit way to copy a string, printing a struct, `async`, threads,
`unsafe`, raw pointers, foreign-function interfaces, macros other than
`println!`, a package manifest, a formatter, and a language server. A
function cannot return something it only borrows, and a struct cannot store
it.

Names are ASCII only for now (letters, digits, and `_`); string text can be any Unicode.

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

The `.rs` file is copied into the build as it is. It may use only Rust's
standard library and may not declare modules of its own or include other
files. `varyk check` reads its `pub fn` signatures and stops there; the Rust
inside it is checked by rustc when you build. Its top-level `pub fn` items
can be called from Varyk when every parameter and the return type is one of
these:

| Rust type | In Varyk |
|---|---|
| `bool`, `i8` to `i64`, `u8` to `u64`, `f32`, `f64` | the same type, passed by value |
| `&T` or `&mut T` where `T` is one of the above | borrowed, or `mut` |
| `&str` | a borrowed `string` |
| `&mut String` | a `mut string` |
| `String` parameter | an owned `string`: a literal is copied, an owned string moves |
| `String` return | `string` |
| `()` return, or none | nothing |

Any other signature, including `&String`, generics, lifetimes, trait
objects, references in the return type, or unknown types, cannot be called.
Calling such a function is an error that shows its Rust signature. Methods,
`unsafe fn`, `async fn`, `const fn`, and `pub(crate)` functions are not
imported. A module that uses a glob import (`use ...::*`) cannot expose
functions with these built-in parameter or return types, because the glob
could redefine any of their names.

## The command line

```text
varyk check <file.vr>                            check for errors; never runs cargo
varyk build <file.vr> [--release] [--emit-rust]  generate and build; print the executable's path
varyk run <file.vr> [--release] [-- args...]     build, then run with the given arguments
```

- `--release` builds with optimizations.
- `--emit-rust` prints every generated file (`Cargo.toml` and the Rust
  files), each after a line naming it, and then builds.
- `--message-format=json` works with every command and prints errors as JSON
  on standard output, one object per line, instead of the text form. The
  error, each of its labels, and its fix-it each name the file they point
  into. Lines count from 1; `column` fields count bytes from the start of the
  line, also from 1.

`run` exits with the program's exit code. The generated Rust project lives
in the build directory, `target/varyk/` under the current directory (so in
your source tree if you run `varyk` from the `.vr` file's directory). If the Rust compiler rejects the generated code, which should be rare,
its output is shown after a line from Varyk that says so.

## Error codes

Every error has a code. A code is never reused for a different meaning.

| Code | Meaning |
|---|---|
| V0001 | a construct Varyk does not support yet; the message names it |
| V0002 | unexpected token or malformed syntax |
| V0003 | a bad escape in a string, or a string with no closing `"` |
| V0010 | `&x` or `&mut x` written at a call; Varyk works out references itself |
| V0011 | `&T` or `&mut T` written in a parameter type; write `name: T` or `mut name: T` |
| V0012 | lifetime syntax such as `<'a>` or `&'a T`; lifetimes are worked out by the compiler |
| V0100 | unknown name |
| V0101 | unknown type |
| V0102 | unknown field |
| V0103 | a name defined more than once, or a built-in type name used for a struct or module |
| V0104 | a module file that is missing, present as both `.vr` and `.rs`, unreadable, a `.rs` file that cannot be parsed as Rust, or named `main` or `lib` |
| V0105 | an item used from outside its module without `pub` |
| V0106 | a missing or malformed `fn main()` |
| V0107 | `String` or `str` written where `string` is meant |
| V0108 | a Rust function whose signature Varyk cannot call; the message shows the signature |
| V0109 | a struct that contains itself, directly or through other structs' fields |
| V0200 | type mismatch |
| V0201 | wrong number of arguments |
| V0202 | `println!` with the wrong number of `{}`, or something other than `{}` in braces |
| V0203 | `{}`, `==`, or `!=` used on a struct |
| V0300 | changing a parameter that was declared without `mut` |
| V0301 | changing a `let` name that was declared without `mut` |
| V0302 | a `let` name without `mut` passed to a `mut` parameter |
| V0303 | a parameter without `mut` passed to a `mut` parameter |
| V0304 | a value the function only borrows, stored in a struct or returned |
| V0305 | a value used after it was given away |
| V0306 | a later argument changes or gives away a value that an earlier argument of the same call still borrows |
