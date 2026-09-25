# Varyk milestone 2: enums, matching, methods, and collections

Date: 2026-09-25.

**Status.** Design, not yet implemented. Extends the milestone-1 spec,
`2026-09-23-varyk-design.md`, whose principles, ownership rules, string rules,
and compiler architecture stay in force; section numbers written "M1 §n"
refer to it. Varyk is experimental and pre-0.1: anything here may change.

## 1. Summary and scope

Milestone 1 proved borrow-by-default on six programs. Milestone 2 makes the
language able to express a real small program: enums and `match`, `for`,
methods, `Option`, `Result`, `Vec`, `?`, `format!`, and a deliberate string
copy. Every addition keeps the milestone-1 rules: calls never write `&`,
mutation is declared with `mut`, the compiler inserts no allocation except a
literal placed into an owned slot and the copy the writer asks for with
`.clone()`.

The roadmap's former milestone 2 covered four independent areas. This spec
takes the language core only. Packages, Cargo dependencies, `use`, nested
modules, field-level `pub`, Rust struct import, the source map, `varyk init`,
and publishing become milestone 3; closures, iterators, `if let`, `HashMap`,
the rest of the standard-library surface, `varyk fmt`, and the
reference-coverage test become milestone 4 (section 8).

Two follow-ups from `docs/plans/2026-09-23-milestone-1-followups.md` are in
scope because the new features depend on them: the representation refactor
(borrow analysis records each local's final Rust representation and the
backend reads it, so the new kinds of local are decided in one place) and
the alias-binding soundness fix, which section 3.1 generalises.

Returning part of a borrowed value, the "lifetime inference for borrowed
returns" the milestone-1 spec promised for milestone 2, is deferred to
milestone 4 (section 3.3). Milestone 2 keeps milestone 1's rule and gives
the writer `.clone()`: a copy the source shows is the MVP answer, and it
removes lifetime emission, return classification, and two diagnostics from
this milestone.

Milestone 2 is an MVP like milestone 1. The test for every item below was
"does one of the six examples in section 4 need it"; what failed that test
is listed in section 2.12 and scheduled later. Only what this document lists
is supported; anything else is rejected with a diagnostic that names the
construct.

## 2. Language surface additions

### 2.1 Lexical elements

New keywords: `enum`, `impl`, `match`, `for`, `in`, `self`. They leave the
reserved list of M1 §4.1. New tokens: `=>`, `?`, `..`, `[`, `]`, and `<`
and `>` in type positions. `format!` and `vec!` are intrinsics
recognised like `println!`. Everything else in M1 §4.1 is unchanged.

`Some`, `None`, `Ok`, `Err`, `Option`, `Result`, `Vec`, and `String` are
reserved names: a struct, enum, function, module, local, or field cannot be
called any of them (V0103), because the generated Rust would shadow Rust's
own names. `String` already gets V0107 when written as a type, and `Self`
stays a reserved keyword (V0001).

New type: `usize`, the integer type of lengths and indexes (section 2.7).

### 2.2 Enums

```varyk
enum Shape {
    Circle(f64),
    Rect(f64, f64),
    Point,
}
```

An enum has one or more variants. A variant is a unit (`Point`) or a tuple
of types (`Circle(f64)`, `Rect(f64, f64)`), the two forms `Option` and
`Result` use. Variants with named fields are milestone 4. Values are written
with the enum's name: `Shape::Point`, `Shape::Circle(2.0)`, or through a
module, `geo::Shape::Point`. A variant the enum does not have is V0100.

`pub enum` follows `pub struct`. Variants are as visible as the enum itself.
An enum that contains itself, directly or through structs, other enums, or
an `Option` or `Result` payload, is rejected like a self-containing struct
(V0109), because there is no `Box` yet and Rust lays those out inline; a
`Vec` payload breaks the cycle, so `enum Tree { Node(Vec<Tree>) }` is
fine.

Enums are never Copy, cannot be compared with `==` or `!=`, and cannot be
printed whole (V0203 extended). A variant's payload, the `x` in
`Shape::Circle(x)`, is an owned slot like a struct literal's field (section
3.4), and its count must match the variant: `Shape::Circle()`,
`Shape::Point(1.0)`, or a bare `Shape::Circle` used as a value is V0201.
An enum's parts are read with `match`.

### 2.3 `match` and patterns

`match` is an expression, like `if`:

```varyk
fn area(shape: Shape) -> f64 {
    match shape {
        Shape::Circle(radius) => 3.14 * radius * radius,
        Shape::Rect(w, h) => w * h,
        Shape::Point => 0.0,
    }
}
```

Each arm is `pattern => expression,`; the comma may be left out after an arm
whose body is a block. Every arm must have the same type (V0200, as for
`if`/`else`). An arm whose body is a `println!`, a call that returns nothing,
or a block without a tail has no value, and then every arm must have none; a
block that always `return`s takes the expected type, as M1's checker already
does for `if`. A bare `return` as an arm body is a Rust habit that gets V0001
with a fix-it wrapping it in a block. The value matched on must be an enum,
an `Option`, or a `Result`; matching on a number, a `bool`, or a string is
milestone 4 (use `if`). Patterns are one level deep:

- `_`, which matches anything;
- a name, which matches anything and binds it;
- a variant, `Shape::Point`, `Shape::Circle(p)`, `Shape::Rect(p, q)`,
  `Some(p)`, `None`, `Ok(p)`, `Err(p)`, or through a module
  `geo::Shape::Point`, where each `p` is a name or `_`.

A variant pattern must have exactly the variant's number of positions
(V0205), and a name may appear once per pattern (V0103). Not in milestone 2: nested patterns such as `Some(Shape::Point)`
(match the inner value in the arm), literal patterns, guards
(`pattern if cond`), alternatives (`a | b`), rest patterns (`..`), range
patterns, `@` bindings, `if let`, and `while let`.

**Exhaustiveness** is checked by Varyk, not left to rustc: every variant
must have an arm, or the last arm must be a catch-all (`_` or a name). A
missing variant is V0204, which names it in plain words. A catch-all
anywhere but last is V0205, since the arms after it could never match.
With one-level patterns and that ordering rule, Varyk and rustc agree on
what is covered. A variant arm repeated after an identical one is rustc's
unreachable-pattern warning and stays silent, like all rustc warnings in
this milestone.

The value matched on, and the `Vec` a `for` runs over, must be a place or
a temporary as section 3.2 defines them; an `if`, a block, or a field or
element of a temporary (`make().status`) in that position is V0001, and
the fix is to bind the value with `let` first. How `match`
borrows what it looks at is in section 3.2.

### 2.4 `for` and ranges

```varyk
for i in 0..n { }        // i takes 0, 1, ..., n - 1
for item in items { }    // every element of a Vec, in order
```

A range's ends are integers of the same type, and a literal end takes the
other end's type, as the operands of `+` do, so `0..v.len()` is a range of
`usize`. The loop variable has that type and is a plain copy. Ranges exist
only in the head of a `for`; `..=` is milestone 4. Over a `Vec`, the loop
variable is one element per round; section 3.2 says whether it is borrowed
or owned. `break` and `continue` work in `for` as in `while`.

Nothing else is iterable in milestone 2: not strings, not ranges of
anything but integers, not `Option`; a `for` over anything else is V0200,
naming what was expected, as is `s[0]` on anything but a `Vec`.

### 2.5 `impl` blocks and methods

```varyk
struct Counter {
    count: i32,
}

impl Counter {
    fn new() -> Counter {
        Counter { count: 0 }
    }

    fn add(mut self, by: i32) {
        self.count = self.count + by;
    }

    fn value(self) -> i32 {
        self.count
    }
}
```

An `impl` block names a struct or enum declared in the same file and holds
functions. A function whose first parameter is `self` is a method, called
`value.name(args)`; `self` has no type annotation and is a shared borrow,
`mut self` a mutable borrow, exactly the parameter rule of M1 §4.2. There is
no owned `self`, because there is no ownership-transfer syntax. A function
without `self` is an associated function, called `Type::name(args)`, or
`m::Type::name(args)` through a module. A method or associated function
that does not exist is V0100, naming the type. A method's `pub` works like a
function's `pub`: a `pub` method of a module's type is callable on a value
of that type anywhere, a non-`pub` one only inside the module (V0105). A
type may have several `impl` blocks; two methods of one name are a
duplicate (V0103).

The receiver of a method call is an argument like any other: a `mut self`
method needs a mutable place (M1 §4.2), with the same diagnostics and fix-its
as passing to a `mut` parameter. Methods on enums work the same way. Not in
milestone 2: `Self`, trait implementations, methods on built-in types, and
`impl` blocks in a different file from the type.

### 2.6 `Option`, `Result`, and `Vec`

The three generic types from Rust's standard library can be used without
Varyk having a way to declare generics. In type positions they are written
as in Rust, nested freely: `Option<i32>`, `Result<User, string>`,
`Vec<Option<Shape>>`. A `string` inside them, like a `string` field or a
variant payload, is an owned `String` in the generated Rust. They are never
Copy, never compared with `==`, and never printed whole.

Values are constructed with `Some(x)`, `None`, `Ok(x)`, `Err(e)`,
`Vec::new()`, and `vec![a, b, c]`, whose elements must all have one type.
`Some(x)` and `vec![a, b]` know their type from their contents. `None`,
`Vec::new()`, an empty `vec![]`, `Ok(x)` (its error type), and `Err(e)` (its
value type) take the type the surrounding position expects: a `let` with a
written type, a parameter, a return, a field, a `match` arm when the `match`
itself has an expected type or an earlier arm fixed one, a `vec!` element
after a typed one. Where nothing is expected, V0207 says to write the type,
with `let v: Vec<i32> = Vec::new();` as the shape. This is the
integer-literal rule of M1 §4.1 applied to these values; the checker still
never infers backwards from later uses.

The calls below are the whole standard-library surface of milestone 2.
"Reads" means the receiver is a shared borrow and "changes" a mutable
borrow, with the diagnostics of M1 §4.2. Every result is a fresh owned value,
except `v[i]`, which is a place.

| Call | Receiver | Result |
|---|---|---|
| `Vec::new()` | none | `Vec<T>` |
| `v.push(x)`; `x` is an owned slot of type `T` | changes | nothing |
| `v.pop()` | changes | `Option<T>` |
| `v.len()` | reads | `usize` |
| `v[i]`; `i: usize` | a place, borrowed or mutable as `v` is | `T`, part of `v` |
| `s.len()` on `string` | reads | `usize` |
| `s.clone()` | reads | `string`; the one deliberate copy |

`Option` and `Result` have no methods: `match` is the one way to look inside
one, and it also replaces `is_some` and `unwrap`. `v[i]` is a place like a
field: it can be read, assigned to, have a field read or assigned, receive a
method call, and be matched on, under the place rules of M1 §4.2 and
section 3. With `i` out of range the program stops with Rust's own message,
as in Rust. Everything else on these types, and every other string method,
is milestone 4: each either needs closures, an `Option` of a borrowed value,
or generics, or is not needed to prove this milestone.

### 2.7 `usize`

`usize` is an integer type like `u64`, the type of a length, an index, and a
count of elements; it is what `len` and `v[i]` use because Rust does. An
integer literal becomes `usize` where a `usize` is expected, as with every
other integer type. There are no casts (`as` stays reserved): a loop that
indexes a `Vec` declares its counter as `usize`, and V0200 says so when an
`i32` meets a `usize`.

### 2.8 `?`

`expr?` is allowed in a function whose return type is `Result<T, E>` when
`expr` is a `Result<U, E>` with the same `E`. It yields the `U`; on `Err`
the function returns that error at once. Any other use is V0206, which
names the function's return type and the value's. There is no automatic
error conversion, and `?` on `Option` is milestone 4. `main` still returns
nothing, so `?` is used in helper functions.

### 2.9 `format!` and `vec!`

`format!("{} is {}", a, b)` follows every `println!` rule of M1 §4.1 and
returns a new owned `string`. It is the only way to join strings in
milestone 2; `+` on strings stays an error (V0200) whose fix-it points at
`format!`. `vec![a, b]` builds a `Vec` from its elements, each of which is an
owned slot: a literal string converts, an owned local moves, a borrowed
place is rejected. Both are compiler intrinsics like `println!`, not a macro
system.

### 2.10 Paths to module types

Types declared in a module can now be named from the file that imports it:
`m::User` in a type position or a struct literal, `m::Shape::Point` for a
variant, `m::Counter::new()` for an associated function. This closes the
milestone-1 gap where a module's struct could only be received from a
function. The generated Rust keeps the `crate::` prefix of M1 §4.5.
Visibility is unchanged from milestone 1: all fields of a `pub struct` are
public; field-level `pub` is milestone 3.

### 2.11 Rust interop

The import of `.rs` signatures (M1 §4.5) changes in one way: `usize` joins
the primitive rows of its table, passed by value or borrowed like `u64`. A
Rust function whose signature mentions `Vec`, `Option`, `Result`, or any
struct or enum stays opaque, and calling it is V0108 as before; importing
Rust types is milestone 3.

### 2.12 Not in milestone 2

Cut from this milestone because no example needs them, and scheduled in
section 8: enum variants with named fields; nested and literal patterns;
`match` on numbers, `bool`, and strings; `..=`; `?` on `Option`; every
method of `Option` and `Result`; `is_empty`, `remove`, `clear` on `Vec`;
every string method but `len` and `clone`; field-level `pub`.

Not in this milestone by design: returning part of a borrowed value
(section 3.3); closures and function types; iterators and
iterator adapters; `if let`, `while let`, `loop`; match guards, `|`, `..`,
range and `@` patterns; ranges outside `for`; `+` on strings; `as`; tuples;
`Self`; traits and trait implementations; derives of any kind, so `==` and
`println!` on structs, enums, `Option`, `Result`, and `Vec` stay errors;
`clone` on anything but a string; `HashMap`; `Box`; `main` returning
`Result`; owned `self` or any ownership-transfer syntax; methods on built-in
types; and everything of milestones 3 and later: Cargo dependencies, `use`,
nested modules, Rust struct import, the source map, `varyk fmt`, the
language server, async.

## 3. Ownership rules

### 3.1 Aliases

Milestone 1 already had aliases and one deferred bug about them: a `let`
made from a parameter or a field is another name for the same value, and
using the original and the alias together got past `varyk check` and was
rejected by rustc. Milestone 2 makes the rule explicit and applies it to
every new way of naming part of a value.

An **alias** of a place is:

- a `let` initialised from, or a `let mut` later assigned, a parameter, a
  field, or a `Vec` element (M1's borrowed places, unchanged, including
  M1's exemption: a Copy value read from any of these is a copy, not an
  alias);
- a name bound to a non-Copy value by a `match` pattern on a place
  (section 3.2);
- the loop variable of a `for` over a `Vec` of a non-Copy type that is a
  place (section 3.2).

Its **root** is the local the place lives under, a parameter or a `let`,
found by following field and index paths down and other aliases through:
the root of `t` in `let t = self.title` is `self`, and the root of `first`
in `let first = tasks[1]` is `tasks`. An alias is a borrowed place: a
reference in the generated Rust, unable to flow into an owned slot except by
`.clone()` on a string.

**Which aliases can change things.** A `let` alias keeps M1 §4.2's rule:
written `let mut` from a mutable place, it is a **mutable alias** (a `&mut`
in the generated Rust) and may be assigned to or passed to a `mut`
parameter; otherwise it is read-only. A pattern binding or a `for` variable
is always read-only, whether it is an alias, a Copy value, or an owned value
from a temporary (section 3.2). Assigning to one, or passing it to a `mut`
parameter, is V0301 or V0303 worded for what it is: for an alias the fix is
to change the original place, since there is no `mut` to add; for a Copy or
owned binding the fix-it is `let mut n = n;` first.

**The rule.** While an alias may still be used, its root may not be changed
or given away. While a mutable alias may still be used, its root may not be
used at all. "Changed" means that the root, or any part of it, is assigned
to, passed to a `mut` parameter, or used as the receiver of a changing
method: `tasks[0].complete()` changes `tasks`. Reading the root of a
read-only alias, or any part of it, is fine.

"May still be used" is decided the way the move checker of M1 decides
whether a moved name is used later: any use after the change in the
function's flow, with a loop body counting as following itself. A `for`
variable counts as used at the end of every round of its loop, whether or
not the body mentions it: Varyk treats the `Vec` as borrowed for the whole
loop, so a `for` over a place never lets the body change that place. Rust's
checker is finer after a `break`; Varyk's rule is over-strict there, not
unsound.

The diagnostic (V0307) points at the change and at the later use of the
alias, and says in plain words that `n` is another name for part of `user`,
so `user` cannot be changed until `n` is no longer needed; the note gives
the Rust term. This one rule replaces the alias cases milestone 1 deferred
and covers what rustc would report as E0502 and E0505 for these programs.
It is over-strict where Rust's borrow checker is finer, which is
acceptable: it never accepts what rustc rejects.

### 3.2 Matching and iterating borrow

`match` on a **place** (a local, a parameter, a field, an element) never
moves it. The generated Rust matches on a shared reference to the place,
reborrowing a `mut` parameter rather than handing it over (section 5).
Every name a pattern binds to a non-Copy value is an alias of the place,
read-only, tied to it by the rule of section 3.1. A name bound to a Copy
value is a copy, as a Copy field is, and is not an alias: the backend copies
it at the start of the arm with `let n = *n;`, so the root is free again as
soon as the pattern has matched.

`match` on a **temporary** (a call result, a struct or enum literal) owns
what it looks at: bound names are owned locals, moved out of the temporary,
and the temporary is gone.

```varyk
match tasks[0].status {            // a place: nothing moves
    Status::Done => ...
}
match parse(text) {                // a temporary: `value` is owned
    Ok(value) => ...
    Err(message) => ...
}
```

`for item in v` follows the same split. Over a place, `item` is an alias of
one element, or a copy when the element type is Copy, and `v` may not be
changed inside the loop (section 3.1). Over a temporary, `item` owns each
element in turn and the `Vec` is consumed.

A `match` used as a value follows M1's rule for an `if` read in place: its
value is the set of its arms' values, so `let n = match opt { Some(x) => x,
None => fallback }` makes `n` an alias when every arm is a place, an owned
local when every arm is new, and V0304 when they mix.

A `match` arm may assign to the root of its scrutinee only when no alias
from the pattern is used after that assignment, by section 3.1. So in a
`mut self` method, `Status::Done => { self.status = Status::Open; }` inside
`match self.status` is fine (a unit pattern binds nothing), and rustc
accepts the generated code.

### 3.3 Returns stay owned

Milestone 1's rule holds: a return value is an owned slot, so a literal
converts, an owned value moves, and a borrowed place, such as `user.name`
in `fn name_of(user: User) -> string { user.name }`, cannot be returned. What
changes is the diagnostic: V0304 no longer promises that milestone 2 will
allow it, and for a string it offers the fix-it `.clone()`.

A borrowed return is a performance feature, not a capability: `.clone()`
expresses every program the examples need, and the copy is visible where it
happens, which principle 5 asks for. Inferring borrowed returns from the
body, with explicit lifetimes in the generated Rust and the call result as
an alias of the argument, is milestone 4. The open questions of M1 §9 on
return ownership and cross-function lifetimes stay open.

### 3.4 Owned slots and moves

The owned slots of milestone 2 are `v.push(x)`, the elements of `vec![..]`,
the value assigned to `v[i]`, a `Some`, `Ok`, or `Err` argument, the payload
of any variant value, the operand of `?`, and a return value. They follow
M1 §4.3 unchanged: a literal string converts, an owned local moves, and a
borrowed place is rejected with V0304, whose message now says what to do
instead of promising milestone 2. M1's rule that a `let` holding a literal
which later flows into an owned slot is made owned at the `let` applies to
all of them.

There are still no partial moves: `let n = user.name` on an owned `user`
makes `n` an alias, as in milestone 1, never a move of the field. This has
a known consequence: an owned local of type `Option<Task>` can never give up
its `Task`, since `match` on it borrows and `Task` has no `clone`. The way
out in milestone 2 is to `match` on the call that produced it, which is a
temporary; whether `match` on an owned local should instead move it, as in
Rust, is recorded as an open question in M1 §9.

### 3.5 Strings

`s.clone()` is the one deliberate copy and always produces an owned string,
whatever `s`'s representation; the backend spells it `.to_string()` when the
receiver is a `&str`. `format!` produces an owned string. The representation
inference of M1 §4.3 gains these sources and nothing else: a local holding a
`clone`, a `format!`, or a `pop` of a `Vec<string>` is owned; a local bound
by a `match` pattern on a place, or a `for` variable over a place, is a
borrowed place with the representation of what it refers to, as a `let`
from a field is.

## 4. Milestone-2 examples

All six must compile and run with the shown output, and are the integration
tests, as in M1 §5. Each begins with the `// expected output:` comment the
test harness reads.

`examples/enums.vr`, output `3.14`, `6`, `0`:

```varyk
enum Shape {
    Circle(f64),
    Rect(f64, f64),
    Point,
}

fn area(shape: Shape) -> f64 {
    match shape {
        Shape::Circle(radius) => 3.14 * radius * radius,
        Shape::Rect(w, h) => w * h,
        Shape::Point => 0.0,
    }
}

fn main() {
    let shapes = vec![Shape::Circle(1.0), Shape::Rect(2.0, 3.0), Shape::Point];
    for shape in shapes {
        println!("{}", area(shape));
    }
}
```

`examples/methods.vr`, output `0`, `3`: the `Counter` of section 2.5, with
`main` creating one with `Counter::new()`, printing `value()`, calling
`add(3)`, and printing `value()` again.

`examples/collections.vr`, output `3`, `10`, `20`, `30`, `60`, `2`:

```varyk
fn sum(values: Vec<i32>) -> i32 {
    let mut total = 0;
    for value in values {
        total = total + value;
    }
    total
}

fn main() {
    let mut values: Vec<i32> = Vec::new();
    values.push(10);
    values.push(20);
    values.push(30);
    println!("{}", values.len());
    for i in 0..values.len() {
        println!("{}", values[i]);
    }
    println!("{}", sum(values));
    values.pop();
    println!("{}", values.len());
}
```

`examples/errors.vr`, output `84`, `not a number: abc`, `none`:

```varyk
fn parse_answer(text: string) -> Result<i32, string> {
    if text == "42" {
        Ok(42)
    } else {
        Err(format!("not a number: {}", text))
    }
}

fn doubled(text: string) -> Result<i32, string> {
    let value = parse_answer(text)?;
    Ok(value * 2)
}

fn describe(result: Result<i32, string>) -> string {
    match result {
        Ok(value) => format!("{}", value),
        Err(message) => message.clone(),
    }
}

fn first_even(values: Vec<i32>) -> Option<i32> {
    for value in values {
        if value % 2 == 0 {
            return Some(value);
        }
    }
    None
}

fn main() {
    println!("{}", describe(doubled("42")));
    println!("{}", describe(doubled("abc")));
    match first_even(vec![1, 3, 5]) {
        Some(value) => println!("{}", value),
        None => println!("none"),
    }
}
```

`describe` needs the `.clone()`: `message` is part of `result`, which the
function only borrows, so returning it is V0304 with the `.clone()` fix-it.

`examples/strings.vr`, output `Alice`, `Alice`, `Hello, Alice!`, `5`,
`true`, in full with its generated Rust:

```varyk
struct User {
    name: string,
}

fn name_of(user: User) -> string {
    user.name.clone()
}

fn main() {
    let user = User { name: "Alice" };
    let name = name_of(user);
    println!("{}", name);
    let copy = name.clone();
    println!("{}", copy);
    println!("{}", format!("Hello, {}!", name));
    println!("{}", name.len());
    println!("{}", name == "Alice");
}
```

```rust
struct User {
    name: String,
}

fn name_of(user: &User) -> String {
    user.name.clone()
}

fn main() {
    let user = User { name: "Alice".to_string() };
    let name = name_of(&user);
    println!("{}", name);
    let copy = name.clone();
    println!("{}", copy);
    println!("{}", format!("Hello, {}!", name));
    println!("{}", name.len());
    println!("{}", name == "Alice");
}
```

`examples/todo/main.vr` with `examples/todo/task.vr`, the combined program,
output `[ ] Buy milk`, `[ ] Write spec`, `[x] Buy milk`, `[ ] Write spec`,
`1 of 2 done`:

```varyk
// task.vr
pub enum Status {
    Open,
    Done,
}

pub struct Task {
    title: string,
    status: Status,
}

impl Task {
    pub fn new(title: string) -> Task {
        Task { title: title.clone(), status: Status::Open }
    }

    pub fn complete(mut self) {
        self.status = Status::Done;
    }

    pub fn is_done(self) -> bool {
        match self.status {
            Status::Done => true,
            Status::Open => false,
        }
    }

    pub fn label(self) -> string {
        let mark = if self.is_done() { "x" } else { " " };
        format!("[{}] {}", mark, self.title)
    }
}
```

```varyk
// main.vr
mod task;

fn count_done(tasks: Vec<task::Task>) -> usize {
    let mut done: usize = 0;
    for t in tasks {
        if t.is_done() {
            done = done + 1;
        }
    }
    done
}

fn print_all(tasks: Vec<task::Task>) {
    for t in tasks {
        println!("{}", t.label());
    }
}

fn main() {
    let mut tasks: Vec<task::Task> = Vec::new();
    tasks.push(task::Task::new("Buy milk"));
    tasks.push(task::Task::new("Write spec"));
    print_all(tasks);
    tasks[0].complete();
    print_all(tasks);
    println!("{} of {} done", count_done(tasks), tasks.len());
}
```

`Task::new` shows the cost model: `title` is borrowed, storing it needs a
copy, and the writer spells the copy. `tasks[0].complete()` is a changing
method on an element of a `let mut` local, a mutable place.

## 5. Compiler changes

The pipeline of M1 §6.2 is unchanged. Per stage:

**`varyk-syntax`.** New tokens and keywords (section 2.1). The type parser
reads generic arguments. AST additions: `EnumDecl` with unit and tuple variants,
`ImplBlock` holding functions whose first parameter may be `self` or
`mut self`, `Stmt::For` with a range or an expression head, `ExprKind::Match`
with arms and one-level patterns, `MethodCall`, `Index`, `Try`, `EnumLit`, and
the two intrinsics. The parser keeps reporting Rust habits (`&self`,
`&mut self`, `Self`, `impl Trait for`) as V0011 or V0001 with a fix-it where
one is obvious.

**Resolver.** Enums and variants get `EnumId`s; `impl` blocks attach their
functions to the type, and a method is found by type then name. `Option`,
`Result`, `Vec`, and their constructors are built-in names resolved before
user names, which is why they are reserved. A new module `builtins.rs` holds
the table of section 2.6 as data: name, receiver mode, parameter types,
result type. Nothing is read from Rust's standard library.

**Types.** `Ty` stops being `Copy` and gains `Enum(EnumId)`,
`Option(Box<Ty>)`, `Result(Box<Ty>, Box<Ty>)`, `Vec(Box<Ty>)`, and
`Int(Usize)`. Expected-type inference extends to `None`, `Vec::new()`,
`Ok`, and `Err` (section 2.6). The checker types patterns against the
scrutinee, checks exhaustiveness (section 2.3), types `?` against the
function's return type, and types method calls from the table or the `impl`
block. No interning yet; the checker clones types.

**HIR.** New nodes: `Match` with typed patterns, `For` with a range or
`Vec` head, `MethodCall` carrying a resolved `MethodRef` (a Varyk function
or a table entry), `Index`, `Try`, `EnumLit`, `Format`, `VecLit`; values and
patterns share one `VariantRef` naming a user variant or `Some`, `None`,
`Ok`, `Err`.
`HirEnum` beside `HirStruct`. `HirFunction` gains `module`.
`Origin::Local(LocalId)` for aliases rooted at a local. The functions of the
program live in one `Vec` indexed by `FnId`, per the follow-ups doc.

**Borrow analysis.** The representation refactor: every local's final
`StringRepr` and `PlaceInfo` are recorded here and the backend only reads
them. Alias tracking (section 3.1) reuses the move checker's flow. Pattern
bindings and `for` variables get their `PlaceInfo` from the scrutinee or
head.

**Backend.** Emits `enum` declarations, `impl` blocks with `&self` and
`&mut self`, `match &place { .. }` and `match temporary { .. }`, `for x in
&v` and `for x in v`, `for i in a..b`, `format!` and `vec!`, `?`,
`.to_string()` for `clone` on a `&str`, and `let n = *n;` at the start of
an arm or loop body for each Copy binding made through a reference,
wrapping an expression arm in a block to hold it. The head of a `match` or
`for` on a place is always a shared borrow, by the type-driven rule of M1
§6.3: `&v` for an owned local, field, or element, `v` as is when it is
already a `&T` (a non-`mut` parameter or a read-only alias), and `&*v`
when it is a `&mut T` (a `mut` parameter or a mutable alias), because
iterating or matching a `&mut` directly would move it. It reads
representations from the HIR instead of re-deriving them.

**Driver and CLI.** Unchanged.

## 6. Diagnostics

New codes, following M1 §6.6; existing codes keep their meaning, some
widened as noted. Every new message is written in plain words, names what to
change, and gives the Rust term in a note.

| Code | Meaning |
|---|---|
| V0100 (widened) | also an unknown variant, method, or associated function, naming the type; lists the table's names for a built-in type |
| V0101 (widened) | also `Option`, `Result`, or `Vec` with the wrong number of type arguments, showing the expected shape |
| V0103 (widened) | also a reserved name (`Some`, `None`, `Ok`, `Err`, `Option`, `Result`, `Vec`, `String`) declared as a name, a duplicate method, and a name bound twice in one pattern |
| V0105 (widened) | also a non-`pub` method or associated function used from another module |
| V0109 (widened) | also an enum that contains itself, through `Option` or `Result` payloads included |
| V0200 (widened) | also `+` on strings, with a fix-it to `format!`; an `i32` where a `usize` is needed, saying to declare the count as `usize`; `match` arms of different types; a `for` over, or an index on, something that is not a `Vec` |
| V0201 (widened) | also a variant value with the wrong number of payloads |
| V0203 (widened) | also `==`, `!=`, or `{}` on an enum, `Option`, `Result`, or `Vec` |
| V0204 | a `match` that does not handle every variant, naming one it misses |
| V0205 | a pattern that does not fit the value: wrong variant, wrong number of positions, a catch-all that is not the last arm, or matching on something that is not an enum, `Option`, or `Result` |
| V0206 | `?` in a function that does not return `Result`, or on a value that is not a `Result` with the function's error type |
| V0207 | a value whose type cannot be worked out here (`None`, `Vec::new()`, an empty `vec![]`, `Ok`, `Err`); says to write the type |
| V0301, V0303 (widened) | also assigning to, or passing to a `mut` parameter, any pattern or `for` binding; an alias says to change the original, a Copy or owned binding gets the `let mut n = n;` fix-it |
| V0304 (reworded) | the "milestone 2 will allow this" note goes away; each case says what to do, and a string gets the `.clone()` fix-it |
| V0307 | a place changed or given away while another name for part of it is still used later |

## 7. Testing and definition of done

The posture of M1 §7 holds: unit tests per pass, `insta` snapshots for
generated Rust and rendered diagnostics, integration tests that build and
run every example, CI with `fmt`, `clippy -D warnings`, and `test`.

- The soundness sweep in `crates/varyk/tests/soundness.rs` gets templates
  for every construct in section 2 and every rule in section 3, in the
  must-build and must-reject sets. Any "passes check, fails in rustc" bug
  found during the milestone extends the templates, not just the one case.
- The alias cases milestone 1 deferred, listed in the follow-ups doc, join
  the must-reject set.
- Each new diagnostic has a test asserting code and span and a rendered
  snapshot.
- `docs/language.md` is updated in the same change as each construct, and
  the milestone-1 "Not in milestone 1" list is replaced by section 2.12.
- `docs/open-questions.md` and section 9 of the milestone-1 spec, already
  updated for this spec, are kept in step with any change to it.

Milestone 2 is done when the six examples build and run with their expected
output, every construct in section 2 either works or produces a named
diagnostic, the tests above pass, and `docs/language.md`, `docs/design.md`,
and `docs/roadmap.md` reflect this spec.

## 8. Roadmap changes

`docs/roadmap.md` is renumbered. Milestone 2 becomes this spec. The former
milestone-2 items and this spec's cuts move as follows:

- **Milestone 3: packages and interop.** `Cargo.toml` as the manifest,
  Cargo dependencies and `use`, nested modules and `mod.vr`, field-level
  `pub`, Rust struct import, the source map and per-file rustc warnings,
  `varyk init` with a `build.rs`, publishing to crates.io, and the site.
- **Milestone 4: closures, iterators, and tooling.** Closures and function
  types, iterator adapters, borrowed return values inferred from the body,
  `if let` and `while let`, enum variants with
  named fields, nested and literal patterns, `match` on numbers and
  strings, `..=`, `?` on `Option`, the `Option` and `Result` methods, the
  rest of the `Vec` and `string` methods, `HashMap`, `Box`, `as`, `clone` on
  structs and enums, `varyk fmt`, and the test that `docs/language.md`
  covers the parser.
- **Milestone 5: batteries for services** (the former milestone 3), and
  **milestone 6: tooling and beyond** (the former milestone 4), unchanged.

## 9. Decisions

Appended to the decisions log of the milestone-1 spec (M1 §10).

| Decision | Choice | Why |
|---|---|---|
| Milestone 2 scope | language core only; packages, closures, and tooling later | four independent areas do not fit one MVP; a real program needs enums and collections before it needs dependencies |
| MVP cuts | borrowed returns, struct variants, nested and literal patterns, `Option`/`Result` methods, most `Vec` and `string` methods, `..=`, `?` on `Option`, field `pub` deferred | no example needs them; `match` is the one way to look inside an `Option` or `Result`; one-level patterns make exhaustiveness exact |
| String copy | `s.clone()`, strings only | Rust's own spelling; the cost is visible at the call; no implicit clone anywhere |
| Closures | deferred to milestone 4 | without generics no Varyk function can take one; they pay off only with iterator adapters |
| String joining | `format!` only; `+` rejected with a fix-it | one spelling, allocation visible at the call; Rust's `+` consumes its left side |
| Borrowed returns | deferred to milestone 4; `.clone()` is the milestone 2 answer | a performance feature, not a capability; removes lifetime emission and return classification from the MVP |
| Matching a place | never moves; non-Copy bindings are aliases, Copy ones are copied at arm entry; a temporary is owned | borrow by default, applied to `match` and `for` |
| Exhaustiveness | checked by Varyk: every variant or a catch-all | plain-word diagnostics before cargo runs; exact, since patterns are one level deep |
| Standard types | `Option`, `Result`, `Vec` with a hand-written table of six methods and indexing | no generics in the surface; the table is small, explicit, and sound by construction |
| Lengths and indexes | `usize` added, no casts | matches Rust and rustc's types; `as` is lossy and can wait |
| Type holes | `None`, `Vec::new()`, an empty `vec![]`, `Ok`, `Err` take the expected type or ask for an annotation | the integer-literal rule, no backward inference |
