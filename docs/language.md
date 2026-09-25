# The Varyk language

This page describes everything Varyk accepts today, in milestone 2 of an
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

A top-level item is a function (`fn`), a struct (`struct`), an enum
(`enum`), an `impl` block, or a module declaration (`mod name;`), each
optionally preceded by `pub` (except `impl`). All fields of a `pub struct`
are public, and all variants of a `pub enum`. Only the entry file may
declare modules and only the entry file may define `main`.

A `pub struct` or `pub enum` of a module is named from the file that imports
it through the module name, as a type and in a struct literal:

```varyk
// main.vr
mod m;

fn make() -> m::User {
    m::User { name: "Bob" }
}
```

## Comments, keywords, and reserved words

A comment starts with `//` and runs to the end of the line.

The keywords are `fn`, `pub`, `let`, `mut`, `struct`, `mod`, `if`, `else`,
`while`, `break`, `continue`, `return`, `enum`, `impl`, `match`, `for`, `in`,
`self`, `true`, and `false`.

Every other Rust keyword is reserved: `as`, `async`, `await`, `const`,
`crate`, `dyn`, `extern`, `loop`, `move`, `ref`, `Self`, `static`, `super`,
`trait`, `type`, `unsafe`, `use`, `where`, `abstract`, `become`, `box`, `do`,
`final`, `gen`, `macro`, `override`, `priv`, `try`, `typeof`, `unsized`,
`virtual`, and `yield`. Using one is an error that says the construct is not
supported yet. None of them can be used as a name.

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

Conditions must be `bool`. Chains are written `if ... { } else if ... { } else { }`.
A struct value inside an `if` or `while` condition needs parentheses.

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

## Not in milestone 2

These do not exist yet, and using them is an error that names what is not
supported. They are planned for a later milestone (see
[roadmap.md](roadmap.md)): enum variants with named fields; patterns inside
patterns and literal patterns; `match` on numbers, `bool`, and strings;
`..=`; `?` on an `Option`; any call on `Option` or `Result`; `is_empty`,
`remove`, and `clear` on a `Vec`; any call on a string but `len` and
`clone`; and per-field `pub`.

These are left out of milestone 2 by design: returning part of a value the
function only borrows; closures and function types; iterators; `if let`,
`while let`, and `loop`; guards, `|`, `..`, ranges, and `@` in patterns;
ranges anywhere but the head of a `for`; `+` on strings (use `format!`);
`as`; tuples; `Self`; traits; derives of any kind, so `==` and `println!`
stay errors on structs, enums, `Option`, `Result`, and `Vec`; `clone` on
anything but a string; `HashMap`; `Box`; a `main` that returns a `Result`;
a method that takes `self` by value, or any other way to write "give this
away"; `impl` blocks for built-in types; and everything planned for later
milestones: Cargo dependencies, `use`, modules inside modules, using Rust
structs, a source map, `varyk fmt`, a language server, and `async`.

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
| `bool`, `i8` to `i64`, `u8` to `u64`, `usize`, `f32`, `f64` | the same type, passed by value |
| `&T` or `&mut T` where `T` is one of the above | borrowed, or `mut` |
| `&str` | a borrowed `string` |
| `&mut String` | a `mut string` |
| `String` parameter | an owned `string`: a literal is copied, an owned string moves |
| `String` return | `string` |
| `()` return, or none | nothing |

Any other signature, including `&String`, `Vec`, `Option`, `Result`,
structs and enums, generics, lifetimes, trait objects, references in the return type, or unknown types, cannot be called.
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
| V0001 | a construct Varyk does not support yet; the message names it. Also an `if`, a block, or a field or element of a value made right there as the value a `match` looks at or a `for` goes over |
| V0002 | unexpected token or malformed syntax |
| V0003 | a bad escape in a string, or a string with no closing `"` |
| V0010 | `&x` or `&mut x` written at a call; Varyk works out references itself |
| V0011 | `&T` or `&mut T` written in a parameter type; write `name: T` or `mut name: T` |
| V0012 | lifetime syntax such as `<'a>` or `&'a T`; lifetimes are worked out by the compiler |
| V0100 | unknown name, or a variant, method, or associated function the type does not have; for `Vec` and `string` the message lists their calls |
| V0101 | unknown type, or `Option`, `Result`, or `Vec` with the wrong number of types |
| V0102 | unknown field |
| V0103 | a name defined more than once (a method included, or a name twice in one pattern), or a reserved or built-in type name used as a name, or a binding named after a unit variant of its own enum (`Point` where `Shape::Point` is meant) |
| V0104 | a module file that is missing, present as both `.vr` and `.rs`, unreadable, a `.rs` file that cannot be parsed as Rust, or named `main` or `lib` |
| V0105 | an item, method, or associated function used from outside its module without `pub` |
| V0106 | a missing or malformed `fn main()` |
| V0107 | `String` or `str` written where `string` is meant |
| V0108 | a Rust function whose signature Varyk cannot call; the message shows the signature |
| V0109 | a struct or enum that contains itself, directly or through other structs, enums, `Option`, or `Result` |
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
| V0304 | a value the function only borrows, stored in a struct, an element, an enum value, `Some`, `Ok`, `Err`, or a `vec!`, passed to `push`, used with `?`, or returned; for text the fix is `.clone()` |
| V0305 | a value used after it was given away |
| V0306 | a later argument changes or gives away a value that an earlier argument of the same call still borrows, or uses the value a method is called on while the method may change it, as in `v.push(v.len())`, or an index changes the `Vec` it indexes, as in `v[g(v)]` with `g` taking `mut v` |
| V0307 | a value changed or given away while another name for part of it is still used later, or inside a `for` that goes over it |
