# Varyk milestone 4: closures, iterators, and patterns

Date: 2026-09-29.

**Status.** Design, not yet implemented. Extends the milestone-1 spec,
`2026-09-23-varyk-design.md` (cited as "M1 §n"), the milestone-2 spec,
`2026-09-25-milestone-2-design.md` ("M2 §n"), and the milestone-3 spec,
`2026-09-26-milestone-3-design.md` ("M3 §n"). Their principles, ownership
rules, and compiler architecture stay in force. Varyk is experimental and
pre-1.0: anything here may change.

## 1. Summary and scope

Milestones 1 to 3 made Varyk a language that can express a small program
and ship it as a Cargo package. Milestone 4 makes everyday service code
short: closures on iterator chains, `if let` and `while let`, patterns that
look more than one level deep and match numbers and strings, the useful
methods of `Option`, `Result`, `Vec`, and `string`, `HashMap`, `as` between
number types, `.clone()` and `==` on structs and enums, and functions that
return part of what they borrow without a copy. Every addition keeps the
rules of the first three milestones: calls never write `&`, mutation is
declared with `mut`, and the compiler inserts no allocation except a literal
placed into an owned slot and the copies the writer asks for with
`.clone()`.

Two decisions shape the milestone. First, **closures exist only as
arguments of the built-in calls that take one.** Varyk has no generics, so
no Varyk function can take a closure, and a closure is never a value: it
cannot be stored, passed to a Varyk function, or returned. That removes
function types, capture modes, and the `Fn` traits from the surface, and
leaves closures where they pay off, on iterator chains and on `Option` and
`Result`. Second, **a borrowed return is inferred from the body.** A
function whose every return is part of one parameter returns a borrow of
that parameter, with the lifetime written in the generated Rust and the
call result treated as another name for the argument, by the alias rule of
M2 §3.1. This closes the "returns stay owned" rule of M1 §4.4 and M2 §3.3
and answers two open questions (section 9).

The roadmap's former milestone 4 also held `varyk fmt` and five interop
items. This spec takes the language only, as milestone 2 did: the formatter
and three interop items move to milestone 6, one interop item that the
service batteries will need moves to milestone 5, and one, derives on
imported Rust types, is done here for `Clone` and `PartialEq` with its
`Debug` half following the formatter to milestone 6 (section 8).

Milestone 4 is an MVP like the three before it. The test for every item was
"does one of the examples in section 4 need it"; what failed is listed in
section 2.13 and scheduled or left open. Only what this document lists is
supported; anything else is rejected with a diagnostic that names the
construct.

## 2. Language surface additions

### 2.1 Lexical elements

`as` leaves the reserved list of M1 §4.1 and becomes a keyword (section
2.9). New tokens: `|` on its own (closures), `..=` (ranges), and the
two-keyword forms `if let` and `while let`. `HashMap` joins the reserved
names of M2 §2.1 (`Some`, `None`, `Ok`, `Err`, `Option`, `Result`, `Vec`,
`String`), because the generated Rust names it. Everything else is
unchanged.

### 2.2 Closures

```varyk
let evens = numbers.iter().filter(|n| n % 2 == 0).count();
let names: Vec<string> = users.iter().map(|u| u.name.clone()).collect();
let doubled = Some(4).map(|n| n * 2);
```

A closure is `|x| expression` or `|x| { block }`, with exactly one
parameter, since every call that takes a closure hands it one value (V0201
for none or several). The parameter carries no type: it takes the type the
call gives it (the item of the chain, the payload of the `Option` or
`Result`), so `|n: i32|` is V0001. There is no return type annotation and
no `move`. A closure may appear only as the argument of a call in the
tables of section 2.7 whose parameter is written as a closure; anywhere
else, in a `let`, an argument to a Varyk function, a return, a field, it is
V0001 with a message naming the calls that accept one.

Inside a closure, the names of the enclosing function are visible and can
be read, and nothing more: assigning to one, passing it to a `mut`
parameter, or calling a changing method on it is V0301 or V0303 worded for
a closure, "inside a closure a name from outside can only be read". A
closure's parameter is a local of the closure, read-only like a pattern
binding (M2 §3.1). A closure body is an expression, not a piece of the
function around it: `return`, `break`, `continue`, and `?` inside one are
V0001, because in Rust they would leave the closure, not the function, or
not compile at all. What a closure parameter refers to, and what a
closure's body may return, is in section 3.2.

### 2.3 Iterator chains

```varyk
let total = numbers.iter().sum();
let short: Vec<string> = words.iter().filter(|w| w.len() < 4).map(|w| w.clone()).collect();
for word in text.split(" ") { }
```

A **chain** starts at a source, passes through zero or more adapters, and
ends at a terminal or as the head of a `for`. The rows are in section 2.7:
the sources are `v.iter()` on a `Vec`, `s.split(sep)` on a string, and
`m.keys()` and `m.values()` on a `HashMap`; the adapters are `map` and
`filter`; the terminals are `collect`, `count`, `sum`, `any`, `all`, and
`find`. Every call in a chain is written in one expression. The type of an
unfinished chain has no Varyk spelling, so a chain stored in a `let`, passed
as an argument, returned, or used as anything but the receiver of the next
call or a `for` head is V0208, "finish the chain here". A chain is lazy in
the generated Rust as in Rust; a Varyk writer never sees that, since a
chain cannot be observed before it ends.

`collect()` produces a `Vec` of the items. Its element type is the item
type, so no annotation is needed. `count()` is a `usize`; `sum()` is the
item type, which must be a number (V0200); `any` and `all` take a closure
returning `bool`; `find` takes the same and gives an `Option` of the item,
which section 2.8 says how to open. What an item is, borrowed, copied, or
owned, and which of them `collect` accepts, is in section 3.3.

`s.split(sep)` gives each piece of `s` between occurrences of `sep`, as
Rust does, `sep` a string. There is no `chars()`: Varyk has no character
type (section 2.13).

### 2.4 `if let` and `while let`

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

`if let pattern = value { } else { }` runs the first block when the pattern
fits and the `else` block, which may be an `if` or another `if let`,
otherwise; the `else` may be left out, as with `if`. It is an expression
like `if`, with the same typing of its blocks (V0200).
`while let pattern = value { }` repeats while the pattern fits, evaluating
`value` anew each round. The pattern is any pattern of section 2.5, and
the value follows every rule of a `match` head: it must be a place or a
temporary (M2 §2.3), and what the bindings refer to is decided by M2 §3.2,
or by section 2.8 for a looked-into result and section 3.1 for a
borrowed-return result. `break` and `continue` work in `while let` as in
`while`.

### 2.5 Patterns

Patterns grow in four ways.

**Variants with named fields.**

```varyk
enum Event {
    Click { x: i32, y: i32 },
    Key(string),
    Quit,
}

let e = Event::Click { x: 1, y: 2 };
match e {
    Event::Click { x: 0, y: 0 } => println!("origin"),
    Event::Click { x, y } => println!("{}, {}", x, y),
    Event::Key(k) => println!("{}", k),
    Event::Quit => println!("quit"),
}
```

A variant may carry named fields, written and matched like a struct. The
fields of a variant are as visible as the enum, as in Rust; there is no `pub`
on them, and two fields of one name in a variant, or a field named twice in
a value or a pattern, are V0103 as in a struct literal. A value names
every field (V0201 for a missing one, V0102 for an unknown one). A pattern
names every field, each as `name`, which binds it, or `name: pattern` (`name:
_` to ignore it); there is no `..`, so a variant pattern that leaves a field
out is V0205. The same rule already holds for struct literals, and struct
patterns on plain structs are not added (match on the fields with `if`).

**Nested patterns.** Any position that took a name or `_` in M2 §2.3 now
takes any pattern: `Some(Shape::Circle(r))`, `Ok(Some(x))`,
`Event::Click { x: 0, y }`. A name still appears once per pattern (V0103).

**Literal and range patterns.** An integer literal (with an optional `-`),
a string literal, `true`, or `false` matches that one value. A range
`a..=b` of two integer literals, each with an optional `-`, matches every
integer between them, both ends included; both ends must have the value's
type and `a` must not exceed `b` (V0205). A literal or range end that does
not fit the value's type (`300` against a `u8`) is V0205, as the same
literal in a `let` is V0200, since rustc refuses it even inside generated
code. Floats cannot be matched
with a literal (V0205, "use `if`"). A string literal matches only as the
whole pattern of a `match`, `if let`, or `while let` on a string; inside
another pattern, `Some("yes")` or `Event::Key("a")`, it is V0205 ("name it
and compare with `==` in the arm"), because Rust compares a `String` with a
literal only after `.as_str()`, which the backend can write for the head
and not for a nested position. With these, `match` accepts a number, a
`bool`, or a string as its value, which M2 §2.3 made V0205:

```varyk
fn grade(score: i32) -> string {
    match score {
        90..=100 => "A",
        80..=89 => "B",
        _ => "lower",
    }
}
```

**Exhaustiveness and reachability.** Varyk still checks both, before
cargo runs. Every value must fit some arm: for enums, `Option`, and
`Result` every variant at every depth, for `bool` both values, and for a
number or a string a catch-all arm, because Varyk treats every number type
and `string` as having infinitely many values: ranges never add up to a
whole type, so `0..=255` on a `u8` still needs the catch-all, and that
catch-all is never reported as unreachable (over-strict where Rust is
finer, never unsound). V0204 names a value shape no arm fits, such as
`Some(Shape::Point)` or `Event::Click { x: _, y: _ }`. An arm that no value
can reach, because the arms above it already fit everything it fits, is
V0205; this generalises M2's "catch-all not last" rule and reports what
rustc would only warn about, since the generated items allow warnings. The
M2 §2.3 exemption for a variant arm repeated after an identical one goes
with it: that arm can never run either.
Not in milestone 4, as in milestone 2: guards, `|`, `..`, `@`.

### 2.6 `?` on `Option`

`expr?` in a function returning `Option<T>` takes an `Option<U>` and yields
the `U`, returning `None` at once on `None`. `?` on an `Option` in a
function returning `Result`, or on a `Result` in a function returning
`Option`, is V0206 with a message naming both, and there is still no
conversion of any kind.

An expected type now flows through `?`: in a function returning `Option<_>`,
`expr` in `expr?` is expected to be `Option<U>`, and in a function returning
`Result<_, E>`, `Result<U, E>`, where `U` is the type expected of `expr?` when
there is one and otherwise a hole the operand fills itself (`Ok(x)` takes `U`
from `x`, `Some(x)` likewise) and `E` is always the function's error type.
This makes `let n: i32 = text.parse()?;` type without a further annotation in
a function returning `Option`, and closes the milestone-2 follow-up where
`Ok(x)?` was V0207. `Err(e)?;` as a statement still has nothing to take its
value type from and stays V0207; write `return Err(e);`.

### 2.7 The standard table

The tables below, with M2 §2.6's rows and the `.clone()` and `==` of section
2.10, are the whole standard-library surface of milestone 4. Every row is used
by an example in section 4; what is not is in section 2.13. "Reads" means the
receiver is a shared borrow, "changes" a mutable borrow, with the diagnostics
of M1 §4.2. "Takes" means the receiver is used up like the operand of `?`
(section 3.4). A parameter written as a type is an owned slot unless the row
says it is read, and a read `string` argument is passed exactly as to a Varyk
`string` parameter, as a `&str`, since `&mut String` is not a pattern Rust's
string methods accept; a parameter written `|x| ..` is a closure. `T` is the
element or payload type, `K` and `V` a map's key and value types, `R` whatever
a closure returns.

**Why a table, and not the Rust methods as they are.** Nothing below
reimplements a Rust function: the backend emits `v.iter().filter(..).count()`
as written, and each row is a signature declared in Varyk's vocabulary, the
way TypeScript's `lib.d.ts` declares the JavaScript runtime without
implementing it. The declaration is what Varyk cannot do without. To write a
call as Rust it must know the result type, or nothing after the call can be
typed; whether the receiver is read, changed, or used up, or it cannot
write `&`, `&mut`, or neither; whether the result borrows from the receiver,
or `let first = v.get(0)` could outlive `v`; and whether the call allocates,
or `.cloned()` and `.to_owned()` would slip in as hidden copies. An unknown
method passed through to rustc loses all four at once, and its failure would
arrive as a V0900 in generated code rather than a plain-word diagnostic.
Reading the signatures from Rust's standard library instead, as the importer
reads a facade, fails on exactly what the importer refuses: `filter` takes
`P: FnMut(&Self::Item) -> bool`, `get` returns `Option<&T>`, `split` takes
`P: Pattern`, `HashMap::get` takes `&Q where K: Borrow<Q>`, and resolving
those is a Rust type checker with trait selection, the generics-and-traits
question this design has declined so far (section 9). So the cost of a
method is one row, one soundness template, and one line in the reference,
and the pages of this spec go to the mechanisms that make whole families of
rows declarable at all: closure typing, item kinds, looked-into results,
borrowed returns, and taking methods. Any method the table lacks is one
plain function away in a `.rs` facade, checked by rustc in the user's own
terms (M3 §1); that is the pass-through Varyk already has.

**`Vec<T>`** (with M2's `new`, `push`, `pop`, `len`, `v[i]`):

| Call | Receiver | Result |
|---|---|---|
| `v.is_empty()` | reads | `bool` |
| `v.insert(i, x)`; `i: usize`, `x: T` | changes | nothing; stops the program when `i` is past the end, as `v[i]` does |
| `v.remove(i)`; `i: usize` | changes | `T`, taken out; stops the program when `i` is past the end |
| `v.contains(x)`; `x: T` read, `T` a number, `bool`, or `string` | reads | `bool` |
| `v.sort()`; `T` an integer, `bool`, or `string` | changes | nothing |
| `v.join(sep)`; `T` is `string`, `sep: string` read | reads | `string` |
| `v.iter()` | reads; `v` must be a place | a chain of `T` items (section 3.3) |
| `v.get(i)`; `i: usize` | reads; `v` must be a place when the result is looked into | `Option<T>`: looked into here, or a plain `Option` of a copy when `T` is a number or `bool` (section 2.8) |

**`string`** (with M2's `len` and `clone`):

| Call | Receiver | Result |
|---|---|---|
| `s.is_empty()` | reads | `bool` |
| `s.contains(p)`, `s.starts_with(p)`; `p: string` read | reads | `bool` |
| `s.to_uppercase()` | reads | `string`, new |
| `s.replace(from, to)`; both `string`, read | reads | `string`, new |
| `s.trim()` | reads; `s` must be a place or a literal | `string`, part of `s` (section 3.1) |
| `s.split(sep)`; `sep: string` read | reads; `s` must be a place or a literal | a chain of `string` items, parts of `s` |
| `s.push_str(t)`; `t: string` read | changes | nothing |
| `s.parse()` | reads | `Option<T>`, `T` a number type or `bool` taken from where the result goes |

`parse` gives `None` when the text is not a value of `T`, with Rust's rules
for what the text may look like. Rust's `parse` returns a `Result` whose
error type Varyk cannot name and whose message says only that the text did
not parse, so an `Option` carries the same information; `ok_or` turns it
into the `Result` a function wants. `T` is taken from the expected type
like `None` is (M2 §2.6), through `?` by section 2.6, and from nowhere
else: the checker never types a receiver from the call made on it, so
`text.parse().ok_or(e)?` is V0207, and a function returning `Result` writes
two statements, `let parsed: Option<i32> = text.parse();` and then
`parsed.ok_or(e)?`, as `readings.vr` does. `push_str` on a local makes that
local own its text, as passing it to a `mut string` parameter does (M1
§4.3).

**`Option<T>`:**

| Call | Receiver | Result |
|---|---|---|
| `o.is_some()` | reads | `bool` |
| `o.unwrap_or(d)`; `d: T` | takes | `T` |
| `o.map(\|x\| ..)`; `x: T` owned | takes | `Option<R>` |
| `o.ok_or(e)`; `e: E` | takes | `Result<T, E>` |

**`Result<T, E>`:**

| Call | Receiver | Result |
|---|---|---|
| `r.is_ok()`, `r.is_err()` | reads | `bool` |
| `r.ok()` | takes | `Option<T>` |
| `r.unwrap_or(d)`; `d: T` | takes | `T` |
| `r.map_err(\|e\| ..)`; `e: E` owned | takes | `Result<T, R>` |

`unwrap` and `expect` are not in Varyk, now or later: a call that stops
the program because a value is absent defeats the purpose of a language for
services, where the absence is a request to handle. `match`, `if let`, `?`,
and `unwrap_or` cover every use, and a value that truly cannot be absent is
opened with a `match` whose other arm says what the writer believed. The
closures of `Option::map` and `map_err` must return something new (section
3.2).

**`HashMap<K, V>`**, new in this milestone, `K` an integer type, `bool`, or
`string`, `V` any type; written `HashMap<string, i32>` in a type; never
Copy; printed with `{}` never; compared with `==` when `V` can be (section
2.10):

| Call | Receiver | Result |
|---|---|---|
| `HashMap::new()` | none | `HashMap<K, V>`, typed from where it goes like `Vec::new()` |
| `m.insert(k, v)`; `k: K`, `v: V` | changes | `Option<V>`, the value `k` had before |
| `m.get(k)`; `k: K` read | reads; `m` must be a place when the result is looked into | `Option<V>`: looked into here, or a plain `Option` of a copy when `V` is a number or `bool` (section 2.8) |
| `m.contains_key(k)`; `k: K` read | reads | `bool` |
| `m.len()` | reads | `usize` |
| `m.keys()`, `m.values()` | reads; `m` must be a place | a chain of `K` or `V` items |

A `HashMap` has no `m[k]` (Rust's cannot be assigned through; `insert` is
the one way to write, `get` the one way to read), and a `for` cannot run
over it directly, because a pair needs a tuple pattern (V0001, "use
`keys()` or `values()`"). A `HashMap` payload breaks a self-containing
cycle the way a `Vec` payload does (M2 §2.2), so V0109 accepts
`struct Node { children: HashMap<string, Node> }`; Rust lays both out on
the heap. Its iteration order is unspecified, as in Rust;
the example in section 4 sorts the keys.

**Chains** (section 2.3), `I` the item type:

| Call | Result |
|---|---|
| `c.map(\|x\| ..)`; `x: I` | a chain of `R` items |
| `c.filter(\|x\| ..)`; `x: I`, returns `bool` | a chain of `I` items |
| `c.collect()` | `Vec<I>`; items must be owned or copies (section 3.3) |
| `c.count()` | `usize` |
| `c.sum()`; `I` a number type | `I` |
| `c.any(\|x\| ..)`, `c.all(\|x\| ..)`; returns `bool` | `bool` |
| `c.find(\|x\| ..)`; returns `bool` | `Option<I>`: looked into here when the items are borrowed, a plain `Option` when they are copies or owned |
| `for x in c { }` | `x` is one item per round |

### 2.8 Values looked into where they are made

`v.get(i)`, `m.get(k)`, and `find` on borrowed items give an `Option` whose
`Some` holds part of the value the call was made on. When that part is a
number or `bool`, the result is an ordinary `Option<T>` holding a copy, as
reading a number from a place always copies (`.copied()` in the generated
Rust), and `counts.get(word).unwrap_or(0)` is the everyday spelling.
Otherwise Rust spells the type `Option<&T>`, and Varyk has no spelling for
an `Option` of a borrowed value, so such a result must be opened right
where it is made: as the head of a `match`, an `if let`, or a `while let`.
Stored in a `let`, passed, returned, used with `?`, or given any method,
`is_some()` included, it is V0208, "look inside it here with `match` or
`if let`". The names its patterns bind are aliases of what the payload is
part of, exactly as if that had been matched (M2 §3.2): read-only, tied to
it by the rule of M2 §3.1, with the receiver's root for `get` and the item's
root for `find` (section 3.3). The receiver of a looked-into `get` must
therefore be a place (V0001, "store it with `let` first"), so the alias has
a root; a `find` gets its root from its chain's source, and when the result
is a plain `Option` of a copy, any receiver will do.

```varyk
if let Some(line) = lines.get(1) {
    println!("second: {}", line);
}
```

`line` is another name for an element of `lines`, and `lines` cannot be
changed while `line` is still used, as after `let line = lines[1];`.

### 2.9 `as`

`expr as T` converts between number types: any integer type, `usize`, and
`f32` and `f64`, with Rust's rules (truncation toward zero and saturation
from a float, wrapping between integer widths). Both sides must be number
types (V0200); `as` binds tighter than `*`, as in Rust, so
`v.len() as i32 * 2` is `(v.len() as i32) * 2`. It is the answer to the
`usize` friction M2 §2.7 accepted: `let count = v.len() as i32;`. `as` in
`use path as name;` is unchanged.

### 2.10 `.clone()` and `==` on structs and enums

Every Varyk struct and enum that can be cloned derives `Clone` in the
generated Rust, and every one that can be compared derives `PartialEq`, in
one `#[derive(..)]` attribute on the type.
A type **can be cloned** when every field or payload is a number, a `bool`,
a `string`, an `Option`, `Result`, `Vec`, or `HashMap` of types that can be
cloned, a Varyk struct or enum that can be cloned, or a Rust struct or enum
imported from a `.rs` module that has `#[derive(Clone)]`. It **can be
compared** by the same rule with `PartialEq`. A type that contains itself
through a `Vec` or a `HashMap` is judged as if the recursion held, as
Rust's derives do.

`x.clone()` on a value of a type that can be cloned is a deep copy, owned,
spelled where it costs, as `s.clone()` has been since milestone 2; on a
`Vec`, `Option`, `Result`, or `HashMap` whose contents can be cloned it
works the same way. On a number or `bool` it is V0100 ("numbers are copied
on use; drop `.clone()`"). `==` and `!=` work on any two values of one type
that can be compared, `Option`, `Result`, `Vec`, and `HashMap` of such
types included. On a type that cannot, either is V0203, which names the
field that prevents it and, for a Rust field, says to derive the trait in
the `.rs` file. `{}` still prints numbers, `bool`, and strings only.

The derives are compile-time only, cost nothing at run time, and appear in
the generated Rust for the reader. Whether serde's traits should follow the
same automatic rule is the open question of M1 §9, unchanged; this section
is an input to it (section 9).

### 2.11 `..=` in `for`

`for i in a..=b { }` counts from `a` to `b`, both included, with the rules
of `a..b` from M2 §2.4. `..=` and `..` exist in `for` heads and, `..=`
only, in patterns.

### 2.12 Rust interop

The importer of M1 §4.5 and M3 §4 changes in two ways.

- **Borrowed returns.** A `&self` method whose return type is `&str` or
  `&S`, for an imported struct or enum `S`, imports with a borrowed return
  rooted at `self`, whatever its other parameters; a `pub fn` with that
  return type and exactly one reference parameter, which is `&T` not
  `&mut T`, imports rooted at that parameter. These are the cases Rust's
  lifetime elision resolves to that parameter, so the root is certain, and
  they match what section 3.1 infers for Varyk functions. The signature
  must use elision: a written lifetime anywhere in it keeps the item opaque,
  as the importer already does for every reference with a lifetime, so
  `fn pick<'a>(&self, other: &'a str) -> &'a str` is never rooted at
  `self`. Any other borrowed return, `&mut S`, a `&mut self` receiver,
  `Option<&T>`, `&[T]`, `&String`, or a free function with two reference
  parameters, stays V0108.
- **Derives.** `#[derive(..)]` lists on imported structs and enums are
  read for `Clone` and `PartialEq`, and nothing else; a manual `impl Clone`
  is not seen (over-strict: `.clone()` on it is V0203 saying to derive).

Closures in Rust signatures (`impl Fn`, `fn(..)`, `Box<dyn Fn>`) stay
opaque and V0108.

### 2.13 Not in milestone 4

Cut because no example needs them, and listed here so the reference can
say so: on `Vec`, `first`, `last`, `clear`, `extend`, `truncate`, `dedup`,
`reverse`, `swap`, `sort_by`; on chains, `enumerate`, `zip`, `rev`, `take`,
`skip`, `fold`, `min`, `max`, `position`; on strings, `ends_with`,
`to_lowercase`, `chars`, `bytes`, `lines`, `find`, `trim_start`,
`trim_end`, `split_once`, and `char` as a type; on `Option` and `Result`,
`is_none`, `Result::map`, `and_then`, `unwrap_or_else`, `ok_or_else`,
`unwrap_or_default`, `as_ref`; on `HashMap`, `remove`, `is_empty`, `clear`;
`HashSet`, `BTreeMap`, `VecDeque`; `HashMap` in a `.rs` signature (a
facade returns a `Vec` or a struct); `iter()`, `keys()`, `values()`,
`split()`, and `trim()` on a value made right there; `Box`; struct patterns
on plain structs;
tuples and tuple patterns, so `for (k, v) in map`; struct field shorthand;
`main` returning `Result`; a borrowed return rooted at two parameters or at
a `mut` parameter (section 3.1); a borrowed return from a Rust signature
elision would not resolve; `Debug` and `{:?}`.

Not in this milestone by design: closures as values (function types, a
closure in a `let`, a parameter, a return, or a field), `move`, typed
closure parameters, capture by mutable borrow, `for_each` (write a `for`);
`unwrap` and `expect`; an `Option` of a borrowed value as a first-class
value; indexing a `HashMap`; guards, `|`, `..`, `@`, `let else`, `loop`;
`Copy` structs; `impl` blocks for built-in types; traits, generics,
attributes; and everything of milestones 5 and 6: the batteries, `async`,
`varyk fmt`, the language server, the interop items of section 8.

## 3. Ownership rules

### 3.1 Borrowed returns

M1 §4.4 and M2 §3.3 made every return an owned slot, so `fn name_of(user:
User) -> string { user.name }` was V0304 and the writer copied with
`.clone()`. Milestone 4 infers the other case.

**The rule.** A function's returns are the tail expression of its body and
the operand of every `return`. Each is one of: **new**, an owned value, a
literal, a struct or enum value, a call result that is owned, a Copy value;
or **part of a parameter**, a borrowed place (M1 §4.2) whose root (M2 §3.1)
is a parameter, `self` included. A tail or `return` operand that is an
`if`, `if let`, `match`, or block contributes each of its leaf values as a
return of its own, as M2 §3.2 reads an `if` in place. The **roots** of a
return are the roots of its place (M2 §3.1 finds them, following `let`
aliases; an `if` value that is part of `a` in one branch and of `b` in the
other has both). When every return is new, the function returns an owned
value, as before. When at least one return is part of a parameter and the
roots of all returns together are exactly one parameter, the function has
a **borrowed return** rooted at that parameter. Anything else is an error:

- a return that is part of a parameter beside a return that is new, a
  literal included, is V0304 as before, with the `.clone()` fix-it for
  text and a note that the two returns disagree (a literal is an owned
  slot's value, as in M2 §3.2, so `if done { "x" } else { self.title }`
  still asks for `self.title.clone()`); a `?` in the body counts as a
  return that is new, since it returns `None` or `Err` early;
- returns whose roots together are two different parameters is V0308,
  "this function returns part of `a` in one place and part of `b` in
  another"; the fix is a copy or two functions;
- a return that is part of a local (`let u = mk(); u.name`) is V0304 with
  the message that `u` ends when the function returns, and so is a return
  of a rootless borrowed place from a call (`first_word("a b")`, or a `let`
  holding one); a `let` holding a literal itself is made owned at the
  literal, as M1 §4.3 has always done;
- a return that is part of a `mut` parameter, `self` of a `mut self` method
  included, is V0304, "make the parameter read-only or copy the value":
  Rust would keep the caller's mutable borrow alive as long as the result
  is used, so the caller could not even read the argument meanwhile, and
  no getter needs it.

A parameter of a Copy type never roots anything (M1 §4.2), so a function
returning an `i32` field is new, as today.

**At the call.** The result of a call to a function with a borrowed return
is a borrowed place whose root is the root of the argument passed in the
rooted position (the receiver, for a method). It follows every rule such a
place already follows: a `let` from it is an alias, read-only; it cannot
flow into an owned slot (V0304, `.clone()` for text); while it or any alias
of it may still be used, the argument's root may not be changed or given
away (V0307). The result is a place for every purpose, never a temporary:
it can be the receiver of `trim`, `split`, `get`, or `iter`, the rooted
argument of another such call, and the head of a `match`, `if let`,
`while let`, or `for`, whose bindings are then aliases with the same root,
as for any place (M2 §3.2). The argument in the rooted position must itself
be a place, a name, a field, an element, or such a result; a value made
right there, a call whose result is owned, a struct or enum value, an `if`,
or a block, is V0001, "store it with `let` first", because Rust would drop
the temporary while the result still points into it. A string literal is
the one exception: it lives for the whole program, so `first_word("a b")`
and `"a b".split(" ")` are fine. The result is then a borrowed place like
any other, so storing or returning it is V0304 with the `.clone()` fix-it
(the literal's own conversion rule does not reach through a call), only
with no root, so nothing can conflict with it under V0307. `trim()` and
`split()` in section 2.7 are borrowed returns rooted at their receiver and
follow this paragraph; `get()` has the same root and follows section 2.8.

**Across functions.** Classification is a pass of its own, before the bodies
are analysed: build the call graph of the program, every function included
whether or not `main` reaches it, take its strongly connected components in
reverse topological order, and classify each function of a component from its
returns, treating a call to a function of the same component as new and a call
to an already classified function by its result. So a function returning
`name_of(user)` is itself part of `user`. A function in a recursive group (one
that calls itself, or a component of more than one function) never has a
borrowed return: a return of it that is part of a parameter is V0304 with the
`.clone()` fix-it, since a call inside the group could not be classified
before the group itself, and treating it as new would let the caller store a
reference as if it were owned. Over-strict, never unsound, and no example is
recursive that way. A function whose returns are in error is reported once, at
its body, and counts as new for its callers. Finding roots needs the alias
roots of M2 §3.1, so this pass computes them for each body and the body
analysis reuses them.

**Generated Rust.** A borrowed return is `-> &str`, `-> &User`. When the
generated signature has exactly one reference parameter (a `mut i32`
parameter is a `&mut i32` there and counts), or the function is a method
rooted at `self`, Rust's elision names the right parameter and nothing is
written.
Otherwise the backend writes one lifetime, on the rooted parameter and the
return: `fn first<'a>(users: &'a Vec<User>, sep: &str) -> &'a User`. At the
call, the type-driven rule of M1 §6.3 supplies `&`, `.as_str()`, and `*`.

```varyk
impl User {
    fn display_name(self) -> string {
        if self.nickname.is_empty() { self.name } else { self.nickname }
    }
}
```

```rust
impl User {
    fn display_name(&self) -> &str {
        if self.nickname.is_empty() { &self.name } else { &self.nickname }
    }
}
```

This answers the two open questions of M1 §9 on return ownership and on
lifetimes across function boundaries (section 9). M2 §3.3's reason for
deferring, "a performance feature, not a capability", holds: `.clone()`
still expresses every program; what changes is that a getter no longer
copies.

### 3.2 Closures

A closure body is analysed as a block of the enclosing function with two
kinds of extra local.

**Parameters.** A closure parameter is what the call hands it: for a
chain, one item, and section 3.3 says whether that is borrowed, a copy, or
owned, and which calls only let the closure look at it; for `Option::map`
and `map_err`, the payload of a receiver that was taken, so an owned local
(a copy, when the payload is a number or `bool`). A
parameter is read-only, like a pattern binding (M2 §3.1): assigning to it,
or passing it to a `mut` parameter, is V0301 or V0303, worded as M2 §3.1
words it for an alias ("change the original") or for a copy or owned value
(the `let mut x = x;` fix-it).

**Captures.** Every name of the enclosing function used in the body is
captured by shared borrow. Inside the body it keeps exactly the kind and
representation it has outside, an owned local stays owned and a number stays a
copy, because a Rust closure borrows a captured name without changing its
type, so the backend writes `keep(line, &levels)` for an owned `levels` and `n
< limit` for a Copy `limit` as it would outside; what changes is that it may
be read, and neither changed (V0301, V0303, worded for a closure, section 2.2)
nor given away. Flowing a captured owned local into an owned slot in the body
is V0304 ("`name` belongs to the function around this closure"); `.clone()` is
the fix for text. With shared captures only, a closure never moves or changes
anything outside itself, so the chain it belongs to is one expression that
only reads its surroundings, and the alias rules of M2 §3.1 hold across it
without new flow analysis. This is why `v.iter().map(|x| v.push(1))` is
rejected as a change to a captured name rather than needing a borrow conflict
rule.

**What a closure returns.** The body of a `map` closure on a chain is
classified like a function body (section 3.1): its result is **new** (owned
or Copy), or **part of** its parameter when the parameter is a borrowed
item (`|u| u.name` over `users.iter()`), or **part of a captured name**
(`|x| other.name`); mixing new with a part is V0304, two different roots
is V0308. When the item is owned or a copy, the parameter is a local of the
closure, and returning part of it is V0304 as returning a field of a local
is (section 3.1). A body classified new is an owned slot, so a string
literal in it converts, and `map(|n| if n > 5 { "big" } else { "small" })`
makes owned strings. The classification decides the items the `map` produces
(section 3.3). The closures of `Option::map` and `Result::map_err` must
return something new: their parameter is an owned local, so returning part
of it is V0304 as returning a field of a local is, and returning part of a
captured name would make an `Option` or `Result` of a borrowed value, which
section 2.8 gives no home; V0304 says to copy it. The closures of `filter`,
`any`, `all`, and `find` return a `bool`.

### 3.3 Items of a chain

Every item of a chain is one of three kinds, decided at the source and
changed only by `map`:

- **borrowed**: another name for one element of a stored value, read-only,
  with the root of that value: the elements of `v.iter()` when the elements
  are not Copy; the pieces of `s.split(sep)`; the
  keys or values of `m.keys()`, `m.values()` when they are not Copy; and
  the result of a `map` whose closure returns part of its parameter, rooted
  at the parameter's root, which is the item's root at that point in the
  chain, or part of a captured name, rooted at that name's root;
- **copies**: the elements of a stored `Vec` or map when they are numbers
  or `bool`, copied as they are read, as a `for` variable or a pattern
  binding is (M2 §3.2), and the result of a `map` whose closure returns a
  Copy value;
- **owned**: the result of a `map` whose closure returns a new non-Copy
  value. A source never yields owned items: `iter()`, `split()`, `keys()`,
  and `values()` need a place as their receiver, or for `split()` a string
  literal (section 3.1), and anything else is V0001, "store it with `let`
  first", so no chain consumes a `Vec` or a map, and `for x in v` on a
  temporary stays the way to take elements out (M2 §3.2).

`filter` keeps the kind. The closure parameter of `map` is the item itself, of
the kind the chain has at that point. The closure parameter of `filter`,
`any`, `all`, and `find` only looks at the item, which continues down the
chain or becomes the result: it is a read-only borrowed place when the item is
borrowed, with the item's root, or when the item is owned and the call is
`filter` or `find`, which Rust hands a reference to a value that has no root,
so nothing conflicts with it under V0307; a copy when the item is a number or
`bool`; and for `any` and `all` over owned items, which Rust hands over by
value, a read-only owned local of the closure that is never given away.
Flowing any of them into an owned slot is V0304, and `map` is where a closure
may make something new from an item. A borrowed item is an alias rooted at its
root (defined below), so while the chain runs, that root cannot be changed or
given away; since a chain is one expression and its closures only read
(section 3.2), this can only be violated inside a `for` over the chain, where
the `for` rule of M2 §3.1 already applies: `for w in text.split(" ") {
text.push_str(w); }` is V0307. The same holds for every other name the head
reads: the argument of the source (`sep` in `text.split(sep)`) and every name
a closure in the head captures, numbers and `bool` included, because Rust's
iterator keeps the closure and its borrows alive for the whole loop and a
closure borrows even a Copy value. So the body may not change or give away any
of them: with `seen` a `Vec<string>`, `for w in words.iter().filter(|w|
seen.contains(w)) { seen.push(w.clone()); }` is V0307 at the `push`, as is
`limit = limit + 1` inside `for n in numbers.iter().filter(|n| n < limit) {
}`, and the fix is to `collect` first. Outside a `for`, a chain is one
expression and its closures die at the terminal, so nothing more is needed.

**Terminals.** `collect` needs owned items or copies: a chain of borrowed
items is V0304, "these are parts of `words`; copy them with
`.map(|w| w.clone())`", because a `Vec` of borrowed values has no Varyk
type. `count`, `sum`, `any`, and `all` accept any kind. `find` on borrowed
items is looked into where it is made (section 2.8), its bindings aliases
with the item's root; on copies or owned items it is a plain `Option`. A
`for` over a chain binds one item per round with the rules of the item's
kind: a borrowed item is an alias with the item's root, a copy is a copy,
an owned item is owned. The **root of a borrowed item** is the root of
whatever it is part of: the source's receiver for `iter`, `split`, `keys`,
and `values`, and for a `map` result the root its closure's classification
gave (section 3.2), which for `|x| other.name` is `other`, not the source.
So after `match v.iter().map(|x| other.name).find(|n| n.len() > 0)`, the
arm's `n` is another name for part of `other`, and `other` cannot change
while `n` is used (V0307), which is what rustc requires too.

**Generated Rust.** Over Copy elements the backend writes
`.iter().copied()`, `.keys().copied()`, `.values().copied()`, so the
closures downstream take plain values, as
`let x = *x;` does for a `for` variable; a closure parameter whose Rust
type is one reference deeper than the place Varyk gives it (`filter` and
`find` receive `&Item`) is dereferenced once at entry. `collect`
is `collect::<Vec<_>>()` and `sum` is `sum::<T>()` with the item type, so
rustc needs no annotation from the writer.

### 3.4 Methods that take their receiver

`unwrap_or`, `map`, `ok_or`, `ok`, and `map_err` use up their receiver, as
`?` does (M2 §3.4): a temporary is used up; an owned local is given away
and using it again is V0305; a stored `Option` or `Result` (a parameter, a
field, an element, an alias) may be taken only when every type inside it,
a `Result`'s error type included, is a number or `bool`, so that the whole
value is Copy in Rust and is copied out; otherwise it is V0304, "match on
it instead", since taking it would need a copy of its contents.
`r.unwrap_or(0)` on a stored `Result<i32, string>` is therefore V0304,
while `o.unwrap_or(0)` on a stored `Option<i32>` is fine. `remove` on a
`Vec` takes the element out, so its result is owned; `insert`'s `Option<V>`
result is owned too. `get` reads and is section 2.8.

### 3.5 Strings

The representation inference of M1 §4.3 and M2 §3.5 gains these sources: a
local holding `to_uppercase`, `replace`, `join`, `remove` from a
`Vec<string>`, `unwrap_or` of an `Option<string>`, or a `clone` of anything
is owned; a local that is ever the receiver of `push_str` is owned, as one
passed to a `mut string` parameter is; a local bound to `trim()`, to a
piece of `split`, or to a borrowed `string` return of a Varyk or Rust
function is a borrowed `&str`, whatever the root's representation, and so
is a name bound by a `match`, `if let`, or `while let` on a string, whose
head is always written as `&str` (section 5); a local
bound to an item of a chain over a `Vec<string>` place, or to a borrowed
struct return's field, is a borrowed place with the representation of what
it refers to, as a `let` from an element is.

## 4. Milestone-4 examples

All must build and run with the shown output and are the integration tests, as
in M1 §5. Each begins with the `// expected output:` comment, and
`crates/varyk/tests/examples.rs` asserts the same text with one `run_*` test
per example; `run_interop`, `run_todo`, and the matcher package test get their
new lines. Six new single-file programs, one updated, and two small edits to
existing examples.

`examples/iterators.vr`, output `55`, `3`, `true`, `2 4 6`, `apple, banana,
cherry`, `3`:

```varyk
fn main() {
    let numbers = vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10];
    println!("{}", numbers.iter().sum());
    println!("{}", numbers.iter().filter(|n| n % 3 == 0).count());
    println!("{}", numbers.iter().any(|n| n > 9));
    let doubled: Vec<string> = numbers.iter().filter(|n| n < 4).map(|n| format!("{}", n * 2)).collect();
    println!("{}", doubled.join(" "));
    let names = vec!["cherry", "apple", "banana"];
    let mut sorted: Vec<string> = names.iter().map(|n| n.clone()).collect();
    sorted.sort();
    println!("{}", sorted.join(", "));
    let text = "one two three";
    println!("{}", text.split(" ").count());
}
```

Exercises: two of the three sources, both adapters, four terminals,
copies of numbers, borrowed strings copied with `clone` before `collect`,
`sort`, `join`, and a `split` chain ending in `count`.

`examples/words.vr`, output `and 1`, `cat 2`, `dog 1`, `ran 1`, `saw 1`,
`the 3`, `6 distinct words`:

```varyk
fn main() {
    let text = "the cat saw the dog and the cat ran";
    let mut counts: HashMap<string, i32> = HashMap::new();
    for word in text.split(" ") {
        let n = counts.get(word).unwrap_or(0) + 1;
        counts.insert(word.clone(), n);
    }
    let mut words: Vec<string> = counts.keys().map(|w| w.clone()).collect();
    words.sort();
    for word in words {
        if let Some(n) = counts.get(word) {
            println!("{} {}", word, n);
        }
    }
    println!("{} distinct words", counts.len());
}
```

Exercises: `HashMap`, `get` on a number payload as a plain `Option` with
`unwrap_or` and `if let`, a borrowed `split` item as a key after `clone`,
`keys()` through `map` and `collect`, and `sort`.

`examples/patterns.vr`, output `key a`, `click at the origin`, `click at
3, 4`, `quit`, `A B lower`, `true false`, `first click at x = 0`, `3`,
`2`, `1`, `quit`:

```varyk
enum Event {
    Click { x: i32, y: i32 },
    Key(string),
    Quit,
}

fn describe(event: Event) -> string {
    match event {
        Event::Click { x: 0, y: 0 } => "click at the origin",
        Event::Click { x, y } => format!("click at {}, {}", x, y),
        Event::Key(key) => format!("key {}", key),
        Event::Quit => "quit",
    }
}

fn grade(score: i32) -> string {
    match score {
        90..=100 => "A",
        80..=89 => "B",
        _ => "lower",
    }
}

fn is_yes(answer: string) -> bool {
    match answer {
        "y" => true,
        "yes" => true,
        _ => false,
    }
}

fn first_click(events: Vec<Event>) -> Option<i32> {
    for event in events {
        if let Event::Click { x, y: _ } = event {
            return Some(x);
        }
    }
    None
}

fn main() {
    let events = vec![Event::Key("a"), Event::Click { x: 0, y: 0 }, Event::Click { x: 3, y: 4 }, Event::Quit];
    for event in events {
        println!("{}", describe(event));
    }
    println!("{} {} {}", grade(95), grade(85), grade(12));
    println!("{} {}", is_yes("yes"), is_yes("no"));
    match first_click(events) {
        Some(x) => println!("first click at x = {}", x),
        None => println!("no clicks"),
    }
    let mut stack = vec![1, 2, 3];
    while let Some(top) = stack.pop() {
        println!("{}", top);
    }
    let last = Some(Event::Quit);
    if let Some(Event::Quit) = last {
        println!("quit");
    }
}
```

`describe` mixes literals and `format!`, so its return is new, as in
milestone 2. Exercises: named-field variants, literal fields in a pattern,
range and string patterns with a catch-all, `if let` on a place with a
copied binding, `while let` on a temporary, a nested pattern.

`examples/getters.vr`, output `Alice`, `Bob`, `[hello]`, `Alice`, `[]`, in
full with its generated Rust:

```varyk
struct User {
    name: string,
    nickname: string,
}

impl User {
    fn new(name: string, nickname: string) -> User {
        User { name: name.clone(), nickname: nickname.clone() }
    }

    fn display_name(self) -> string {
        if self.nickname.is_empty() { self.name } else { self.nickname }
    }
}

fn trimmed(text: string) -> string {
    text.trim()
}

fn first(users: Vec<User>) -> User {
    users[0]
}

fn name_unless(user: User, hidden: string) -> string {
    if user.name == hidden { user.nickname } else { user.name }
}

fn main() {
    let user = User::new("Alice", "");
    let friend = User::new("Robert", "Bob");
    println!("{}", user.display_name());
    println!("{}", friend.display_name());
    let padded = "  hello  ";
    println!("[{}]", trimmed(padded));
    let users = vec![user, friend];
    let leader = first(users);
    println!("{}", leader.name);
    println!("[{}]", name_unless(leader, "Alice"));
}
```

```rust
#[derive(Clone, PartialEq)]
struct User {
    name: String,
    nickname: String,
}

impl User {
    fn new(name: &str, nickname: &str) -> User {
        User { name: name.to_string(), nickname: nickname.to_string() }
    }

    fn display_name(&self) -> &str {
        if self.nickname.is_empty() {
            &self.name
        } else {
            &self.nickname
        }
    }
}

fn trimmed(text: &str) -> &str {
    text.trim()
}

fn first(users: &Vec<User>) -> &User {
    &users[0usize]
}

fn name_unless<'a>(user: &'a User, hidden: &str) -> &'a str {
    if user.name == hidden {
        &user.nickname
    } else {
        &user.name
    }
}

fn main() {
    let user = User::new("Alice", "");
    let friend = User::new("Robert", "Bob");
    println!("{}", User::display_name(&user));
    println!("{}", User::display_name(&friend));
    let padded = "  hello  ";
    println!("[{}]", trimmed(padded));
    let users: Vec<User> = vec![user, friend];
    let leader = first(&users);
    println!("{}", leader.name);
    println!("[{}]", name_unless(leader, "Alice"));
}
```

The lint attributes of M3 §2.3 are left out of the listing, and `println!`
and `vec!` are shown without the `::std::` the backend writes before them.
A method call is written by its path, `User::display_name(&user)`, and an
index literal and a `vec!` `let` carry their types. `new` still
copies, because it stores; the four getters do not. `name_unless` has two
reference parameters, so the backend writes the lifetime; `leader` is an alias
of `users`, so its result is too, and the last line prints Alice's empty
nickname between brackets. Exercises: a borrowed return rooted at `self`, at a
string parameter, at a `Vec` parameter, and at one of two reference
parameters; a `let` alias from a call passed on to another getter; the derives
of section 2.10.

`examples/readings.vr`, output `25 -> 77`, `not a number: abc`, `1`, `10`,
`7`, `true`, `true`, `0`:

```varyk
enum Unit {
    Celsius,
    Fahrenheit,
}

struct Reading {
    value: f64,
    unit: Unit,
}

fn parse_reading(text: string) -> Result<Reading, string> {
    let parsed: Option<f64> = text.trim().parse();
    let value = parsed.ok_or(format!("not a number: {}", text.trim()))?;
    Ok(Reading { value: value, unit: Unit::Celsius })
}

fn to_fahrenheit(reading: Reading) -> Reading {
    if reading.unit == Unit::Fahrenheit {
        reading.clone()
    } else {
        Reading { value: reading.value * 9.0 / 5.0 + 32.0, unit: Unit::Fahrenheit }
    }
}

fn main() {
    let inputs = vec![" 25 ", "abc"];
    for input in inputs {
        match parse_reading(input) {
            Ok(reading) => {
                let converted = to_fahrenheit(reading);
                println!("{} -> {}", reading.value, converted.value);
            }
            Err(message) => println!("{}", message),
        }
    }
    let lengths = vec![3, 10];
    println!("{}", lengths.len() as i32 - 1);
    println!("{}", lengths.get(1).unwrap_or(0));
    let missing: Option<i32> = None;
    println!("{}", missing.unwrap_or(7));
    println!("{}", Some(4).map(|n| n * 2).is_some());
    let failed: Result<i32, string> = Err("bad");
    println!("{}", failed.is_err());
    println!("{}", failed.unwrap_or(0));
}
```

Exercises: `parse` with a written type, `ok_or` with `?`, `==` on an enum,
`.clone()` on a struct, `as`, `get` on a `Vec` of numbers as a plain
`Option`, `unwrap_or`, `map` with a closure on a
temporary, `is_some`, `is_err`, and a taking method giving an owned local
away.

`examples/text.vr`, output `ERROR_disk_full, WARN_low_memory`, `2 of 3
lines kept`, `true`, `true`, `false`, `second: DEBUG tick`, `ERROR DISK
FULL`, `found WARN low memory`, `true`, `600`, `true`, `42`, `bad code:
x!`, `8080`, `hello world`:

```varyk
fn keep(line: string, levels: Vec<string>) -> bool {
    levels.iter().any(|level| line.starts_with(level))
}

fn first_error(lines: Vec<string>) -> Option<usize> {
    for i in 0..lines.len() {
        if lines[i].starts_with("ERROR") {
            return Some(i);
        }
    }
    None
}

fn report(lines: Vec<string>) -> Option<string> {
    let i = first_error(lines)?;
    Some(lines[i].to_uppercase())
}

fn parse_code(text: string) -> Result<i32, string> {
    let parsed: Option<i32> = text.parse();
    parsed.ok_or(format!("bad code: {}", text))
}

fn next_port(text: string) -> Option<i32> {
    let port: i32 = text.trim().parse()?;
    Some(port + 1)
}

fn main() {
    let mut lines: Vec<string> = Vec::new();
    lines.push("ERROR disk full");
    lines.push("INFO started");
    lines.push("WARN low memory");
    lines.insert(1, "DEBUG tick");
    lines.remove(2);
    let levels = vec!["ERROR", "WARN"];
    let kept: Vec<string> = lines.iter().filter(|line| keep(line, levels)).map(|line| line.replace(" ", "_")).collect();
    println!("{}", kept.join(", "));
    println!("{} of {} lines kept", kept.len(), lines.len());
    println!("{}", lines.iter().map(|line| line.trim()).all(|line| line.contains(" ")));
    println!("{}", lines.contains("DEBUG tick"));
    println!("{}", lines.is_empty());
    if let Some(line) = lines.get(1) {
        println!("second: {}", line);
    }
    match report(lines) {
        Some(text) => println!("{}", text),
        None => println!("no errors"),
    }
    match lines.iter().find(|line| line.starts_with("WARN")) {
        Some(line) => println!("found {}", line),
        None => println!("no warnings"),
    }
    let mut codes: HashMap<string, i32> = HashMap::new();
    for i in 1..=3 {
        codes.insert(format!("e{}", i), i * 100);
    }
    println!("{}", codes.contains_key("e2"));
    println!("{}", codes.values().sum());
    println!("{}", parse_code("404").is_ok());
    let maybe = parse_code("42").ok();
    if let Some(code) = maybe {
        println!("{}", code);
    } else {
        println!("no code");
    }
    match parse_code("x").map_err(|e| format!("{}!", e)) {
        Ok(code) => println!("{}", code),
        Err(message) => println!("{}", message),
    }
    println!("{}", next_port(" 8079 ").unwrap_or(0));
    let mut greeting = "hello";
    greeting.push_str(" world");
    println!("{}", greeting);
}
```

`keep` captures its parameter `line` in a closure; `main` captures `levels`
in a `filter` and calls a Varyk function from it. Exercises: closure
captures, a `map` returning part of its borrowed parameter, `all`, `find`
and `get` looked into with `match` and `if let`,
`?` on an `Option` twice, once with the expected type flowing into `parse`
on a `trim` result, `..=` in a `for`, `if let` with `else`, `insert`,
`remove`, `contains` on a `Vec` and a string, `starts_with`, `replace`,
`to_uppercase`, `push_str`, `contains_key`, `values` through `sum`, `ok`,
`is_ok`, and `map_err`.

`examples/todo/` (M2 §4) is updated: `count_done` becomes
`tasks.iter().filter(|t| t.is_done()).count()`, and `Task` gains
`pub fn title(self) -> string { self.title }`, a borrowed getter that
`label` calls in its `format!`. Output unchanged.

`examples/interop/greet.rs` gains `pub fn first_word(s: &str) -> &str`
(the text up to the first space), and `main.vr` prints
`greet::first_word("Hello from Rust")`, adding the line `Hello` to its
output, showing the imported borrowed return.
`examples/packages/matcher/src/text.rs` puts `#[derive(Clone, PartialEq)]`
on `Kind`, and `main.vr` prints
`text::classify("apple") == text::Kind::Word`, adding the line `true`.

## 5. Compiler changes

The pipeline of M1 §6.2 is unchanged. Per stage:

**`varyk-syntax`.** Tokens `|`, `..=`; `as` becomes a keyword. Parser:
closures in argument position, `if let` and `while let`, `expr as Type` as
a postfix with Rust's precedence, named fields in enum declarations,
variant values, and patterns, a recursive pattern grammar with literal and
range patterns, `..=` in `for` heads. AST: `ExprKind::Closure { params,
body }`, `ExprKind::IfLet`, `StmtKind::WhileLet`, `ExprKind::Cast`,
`Pattern` becomes a tree (`Wild`, `Bind`, `Variant(path, Vec<Pattern>)`,
`Struct(path, Vec<(name, Pattern)>)`, `Literal`, `Range`), `Variant` gains
a named-fields form. The parser keeps reporting Rust habits with a fix-it
where one is obvious: `|x: i32|` (V0001, drop the type), `move |x|`
(V0001), `..` in a struct pattern (V0001, name the fields), a guard
(V0001), and `-> &str` or `-> &T` (V0011 widened to return types, "write
`-> string`; Varyk works out the borrow"), which the milestone-1
follow-ups list as getting a generic V0002 today. The messages that point
at milestone 4 today (`..=` in the parser, named-field variants in the
parser and checker) go away with the features.

**Resolver.** `HashMap` is a reserved built-in name. `builtins.rs` gains
owners `Option`, `Result`, `HashMap`, and `Chain`, and new shapes: `K`,
`V`, `E`, a closure `Fn(T) -> R` (on a `Chain` owner `T` is the item), and a
result kind per row
(`Value`, `Borrowed`, `LookInside`, `Chain`), all as data. Closure
parameters and `if let`/`while let` bindings enter scopes like pattern
bindings. Variant field names are resolved and checked for duplicates.

**Types and checker.** `Ty::HashMap(Box<Ty>, Box<Ty>)`, and an internal
`Ty::Chain(Box<Ty>)` for an unfinished chain that no annotation can name
(V0208 where a nameable type is required). Closures are typed at their
call: parameter types from the row, body checked in a nested scope, result
type feeding the row's `R`. Expected types flow through `?` (section 2.6)
and into `parse`, `HashMap::new()`, and closure bodies. `as` checks both
sides are numbers. Exhaustiveness and reachability become one usefulness
check over a constructor set per type: variants, `true`/`false`, and for
every number type and `string` the literals and ranges written, ranges
split at every written end so the constructors are disjoint, plus one
"anything else" constructor no pattern but a catch-all covers, so a
catch-all is always required and never useless there; it reports V0204
with a witness pattern and V0205 for a useless arm. The
"can be cloned" and "can be compared" judgements of section 2.10 are
computed once per type as a fixed point and stored on `HirStruct` and
`HirEnum`.

**HIR.** `Closure { param, captures, body }`, `IfLet`, `WhileLet`, `Cast`,
the pattern tree, named variant fields, `derives` on types,
`HirFunction.ret_root: Option<LocalId>` for a borrowed return, and
`rooted: Option<usize>` on `Call` and `MethodCall`, the argument whose roots
a borrowed result has, set for borrowed-return calls and for the table's
borrowed and looked-into rows.

**Borrow analysis.** A pre-pass classifies every function's return (section
3.1) over the call graph before bodies are analysed, one strongly connected
component at a time. In bodies: call results of borrowed-return functions and
of `Borrowed` rows are borrowed places rooted at the argument; look-inside
results are checked to sit in an allowed position and their bindings are
rooted at the receiver for `get` and at the item's root for `find` (section
3.3); chains carry an item kind (section 3.3) that closure bodies read for
their parameter and `map` closures write for their result; closure bodies are
analysed as nested blocks whose captures keep their outer kind and
representation and are read-only and never given away inside the closure,
decided by position against the closure's span so the same local is
unaffected outside it; the `any`/`all` parameter over owned items is never
given away either (an owned pattern binding of M2 §3.2 is read-only but may
be given away); taking methods move an owned receiver and
reject a stored one with non-Copy contents; the alias rule of M2 §3.1 is
unchanged and covers all of it.

**Backend.**

- One `#[derive(..)]` attribute per type listing `Clone` and `PartialEq`
  as section 2.10 allows, in that order.
- Borrowed return types with elision or one written lifetime (section
  3.1).
- `if let` and `while let`; `(expr as T)` always in parentheses, because
  rustc reads `x as i32 < n` as the start of generic arguments, and an
  integer literal operand written with its type suffix (`-1i32 as u8`),
  because rustc types an unsuffixed literal from the cast target.
- A temporary matched in place, like a stored value, when an enum with a
  destructor appears anywhere inside its type, since Rust cannot move a
  payload out of such an enum at any depth.
- Closures, with a `let x = *x;` at entry where the parameter's Rust type
  is one reference deeper than its Varyk kind, keyed by the item's kind of
  section 3.3 and not by the M2 `for` rule: after `.copied()` no deref for
  a `map`, `any`, or `all` parameter and one for a `filter` or `find`
  parameter, which receive `&Item` even then; none in `Some(n)` from `find`
  on copies or from `get` on a `Vec<i32>`, which the backend follows with
  `.copied()`; none in `Some(line)` from `get` on a `Vec<string>` either,
  whose head is written by value because it already holds a reference, so
  `line` is the `&String` an alias is. A
  captured name and an `any`/`all` parameter over owned items keep their
  outer or owned representation and get `&` where a Varyk parameter needs
  it, as any owned local does.
- `.copied()` after a source of Copy items and after `get` on a Copy
  payload, `.collect::<Vec<_>>()`, `.sum::<T>()` (section 3.3).
- `::std::collections::HashMap` written in full, so no `use` line can be
  shadowed; `.parse::<T>().ok()`.
- `Ok::<T, E>(x)?`, `Err::<T, E>(e)?`, and `None::<T>?` with the full type
  when an `Ok`, `Err`, or `None` typed from its expectation is the direct
  operand of `?`, because rustc cannot recover the other type parameter
  through `?`.
- A `match`, `if let`, `while let`, or `for` head that is already a
  reference in the generated Rust, a parameter, an alias, or a
  borrowed-return result, gets no `&`, as M2's backend already does for a
  `&Vec` parameter.
- A `match`, `if let`, or `while let` head of type `string`, with or
  without string literal arms, written as an expression of type exactly
  `&str`, the place itself when
  it already is one and `.as_str()` on a `String` or `&String` place, never
  the `&place` of a plain M2 head, because a literal pattern does not see
  through `&&str` or `&String`; its name bindings are `&str` (section 3.5).
- Both operands of `==` and `!=` brought to one reference depth for every
  comparable type, as strings already are, so `u == param` with an owned
  `u` and a borrowed `param` compiles; `contains` on a `Vec<string>`
  written as `match (&v, x) { (haystack, needle) =>
  haystack.iter().any(|e| e == needle) }` under the same rule, because
  `Vec<String>::contains` wants a `&String`; both sides are evaluated
  before the compiler's own names come into scope, so a user name `e`,
  `haystack`, or `needle` in either side is never shadowed.
- The source map of M3 §5 covers every new node.

**Interop.** Section 2.12: borrowed returns where elision names the root,
and derive lists.

**Driver and CLI.** Unchanged.

## 6. Diagnostics

New codes, following M1 §6.6; existing codes keep their meaning, some
widened as noted. Every message is written in plain words, names what to
change, and gives the Rust term in a note.

| Code | Meaning |
|---|---|
| V0001 (widened) | also a closure anywhere but as the argument of a call that takes one, naming those calls; a typed closure parameter or `move`; `return`, `break`, `continue`, or `?` inside a closure; a `for` over a `HashMap` ("use `keys()` or `values()`"); a value made right there where a place is needed: the receiver of `trim`, `split`, `iter`, `keys`, `values`, or a looked-into `get`, or the rooted argument of a borrowed-return call, a string literal excepted ("store it with `let` first") |
| V0100 (widened) | also `.clone()` on a number or `bool`; lists the table's names for `Option`, `Result`, `HashMap`, and a chain |
| V0101 (widened) | also `HashMap` with the wrong number of types, or a key type that is not an integer, `bool`, or `string` |
| V0102 (widened) | also an unknown field of a named-field variant, in a value or a pattern |
| V0103 (widened) | also `HashMap` declared as a name; a duplicate field in a variant |
| V0011 (widened) | also `&T` or `&mut T` in a return type; write `T`, Varyk works out the borrow |
| V0108 (reworded) | a borrowed return Varyk cannot import no longer promises milestone 4 |
| V0109 (narrowed) | a `HashMap` payload breaks a cycle as a `Vec` payload does |
| V0200 (widened) | also `as` on a non-number; `sum` on items that are not numbers; `sort` on floats or structs; `contains` on a `Vec` of structs; `join` on a `Vec` of anything but strings; `parse` into anything but a number or `bool`; a `for` range `a..=b` whose ends differ in type, as M2 has for `a..b` |
| V0201 (widened) | also a closure with more or fewer than one parameter; a variant value missing a named field |
| V0203 (narrowed and widened) | `==`, `!=`, or `.clone()` on a type that cannot be compared or copied, naming the field that prevents it; `{}` on anything but a number, `bool`, or string |
| V0204 (widened) | also a `match` on a number or string without a catch-all arm, a `bool` missing a value, and a nested shape no arm fits, shown as a pattern |
| V0205 (widened) | also an arm no value can reach; a float literal pattern; a range pattern with its ends reversed, not of the value's type, or not fitting it; a literal pattern not of the value's type or not fitting it; a string literal inside another pattern; a named-field variant pattern that leaves a field out |
| V0206 (widened) | also `?` on an `Option` in a function returning `Result`, and the reverse |
| V0207 (widened) | also `parse()` and `HashMap::new()` with nothing to take a type from, and a closure body that is `None`, `Vec::new()`, `Ok`, or `Err` with nothing expected |
| V0208 | a value that must be used where it is made: an unfinished chain, or an `Option` holding part of a stored value (`get`, `find`), stored, passed, returned, used with `?`, or given any method |
| V0301, V0303 (widened) | also changing a name captured by a closure, or a closure's parameter |
| V0304 (widened) | also `collect` of borrowed items; a captured name, or a `filter`, `any`, `all`, or `find` parameter, flowing into an owned slot inside a closure, an owned one included; a captured name returned from an `Option::map` or `map_err` closure; a taking method on a stored `Option` or `Result` with non-Copy contents ("match on it instead"); returns that mix a part of a parameter with a new value, a return that is part of a local, a return that is part of a `mut` parameter, and a return that is part of a parameter in a recursive group |
| V0307 (widened) | also a value changed or given away while the result of a borrowed-return call on it, or a binding of a looked-into result, is still used; and a name a chain in a `for` head reads (the source's argument or a closure's capture) changed or given away inside that loop |
| V0308 | a function or closure returning part of more than one parameter (or of a parameter and a captured name) |

## 7. Testing and definition of done

The posture of M1 §7, M2 §7, and M3 §10 holds: unit tests per pass, `insta`
snapshots for generated Rust and rendered diagnostics, integration tests
that build and run every example, CI with `fmt`, `clippy -D warnings`, and
`test`, on stable and 1.85.

- The soundness sweep of `crates/varyk/tests/soundness.rs` gets templates
  for every construct in section 2 and every rule in section 3, in the
  must-build and must-reject sets: each item kind through each terminal
  and a `for`; a closure capturing each value kind and returning each
  kind; borrowed returns rooted at `self`, a parameter, a `Vec` parameter,
  and through another call; taking methods on each receiver kind, a stored
  `Option<i32>` included; a `match` on a `bool` and a negative literal
  pattern; `else if let`; a closure with a block body; `for` over an
  `iter().filter()` chain; `find` on copies and on owned items;
  `contains` and `sort` on integers; a `HashMap` with a struct value looked
  into and `keys()` over integer keys; an imported `&self` method returning
  `&str` and `.clone()` on an imported type deriving `Clone`; a struct
  containing itself through a `HashMap`, cloned and compared;
  look-inside results in each allowed and each forbidden position; nested
  and literal patterns on places and temporaries; `if let` and `while let`
  heads of each kind; derives on a type with a Rust field with and without
  the derive.
- Exhaustiveness and reachability have unit tests against a list of cases
  with rustc's verdict recorded beside each, so the two never disagree in
  the unsound direction.
- The must-reject set holds the programs this spec's reviews found rustc
  rejecting: a borrowed return rooted at a `mut` parameter, a `for` body
  changing a name its head's closure captures or its source's argument, a
  nested string literal pattern, `break` in a closure, `unwrap_or` on a
  stored `Result<i32, string>`, and a recursive function returning part of
  a parameter. The must-build set holds the two the
  backend rewrites: a cast before `<`, and a `&String` head with string
  literal arms.
- Each new or widened diagnostic has a fixture under
  `crates/varyk/tests/fixtures/errors/`, a registration in `errors.rs`, a
  test asserting code and span, and a rendered snapshot.
- A new test asserts `docs/language.md` mentions every keyword and
  reserved word of the lexer, every built-in type name, every call name in
  `builtins.rs`, and every diagnostic code in `codes.rs`. This is the
  reference-coverage test M1 §7 promised for milestone 4, in its
  minimum form.
- `docs/language.md` is updated in the same change as each construct, and
  "Not in milestone 3" is replaced by section 2.13.
- `docs/open-questions.md` marks the two return questions answered
  (section 9), and `docs/design.md` gains the decisions of section 10.

Milestone 4 is done when the examples of section 4 build and run with their
expected output, every construct in section 2 either works or produces a
named diagnostic, the tests above pass, and `docs/language.md`,
`docs/design.md`, `docs/open-questions.md`, and `docs/roadmap.md` reflect
this spec.

## 8. Roadmap changes

Milestone 4 in `docs/roadmap.md` is retitled "closures, iterators, and
patterns" and rewritten to this spec's items. Moved out:

- **To milestone 5, batteries for services:** `.rs` signatures naming
  Varyk-declared types. The `varyk-std` facades will take and return Varyk
  structs, so the batteries spec owns it.
- **To milestone 6, tooling and beyond:** `varyk fmt`; nested modules in
  `.rs` files; import of Rust tuple and unit structs; direct import of a
  published Varyk library from Varyk; `Debug` on imported structs, which
  waits for a `{:?}` placeholder.
- **Unscheduled:** `Box`, until a program needs a recursive type that
  `Vec` or `HashMap` cannot hold; the rest of section 2.13.

Milestones 5 and 6 are otherwise unchanged.

## 9. Open questions

Answered from M1 §9, marked so in `docs/open-questions.md`:

- "How should owned versus borrowed return values be expressed and
  inferred?" Inferred from the body, never written: every return new, or
  every return part of one parameter (section 3.1).
- "How should lifetime inference work across function boundaries?" The
  call result is another name for the argument in the rooted position, and
  the alias rule of M2 §3.1 does the rest; one written lifetime in the
  generated Rust where elision would not pick that parameter.

Added:

- Should closures ever be values, with function types in the surface? This
  spec says no until the generics and traits question is answered, since a
  Varyk function could not take one.
- Should serde derivation follow section 2.10, automatic wherever the
  fields allow, or be declared? Section 2.10 is an input to the
  milestone-5 question, not its answer.
- Should `parse` return a `Result` once milestone 5 has a shared error
  type, so `?` works on it directly?
- Should an `Option` of a borrowed value ever be a first-class value, so
  `get` and `find` could be stored and passed? Section 2.8 keeps it to the
  head of a `match`, `if let`, or `while let`, and copies a number or `bool`
  payload out instead.
- Does Varyk need a character type? `chars()` waits on it.
- Should the standard table move out of the compiler into a declaration
  file in Varyk's own signature vocabulary, so rows are added without a
  compiler change and a facade author can declare shapes the importer
  cannot infer? Section 2.7 says why the table exists; where it lives is a
  later choice.

Unchanged: whether `match` on an owned local should move it. `while let
Some(x) = v.pop()` and `remove` are the milestone-4 ways to take an
element out.

## 10. Decisions

Appended to the decisions log of M1 §10.

| Decision | Choice | Why |
|---|---|---|
| Milestone 4 scope | language only; `varyk fmt` and the interop leftovers to milestones 5 and 6 | two independent areas, as in milestone 2; the formatter needs comment-preserving syntax work of its own |
| Closures | only as arguments of built-in calls; never a value; shared captures only; one untyped parameter | no generics means no Varyk function can take one; shared captures keep chains free of new borrow-flow rules; the item type is always known |
| Chains | one expression from source to terminal; unfinished chains have no type; a source needs a stored receiver or a string literal | the iterator types have no Varyk spelling; nothing is lost, since a chain cannot be observed before it ends |
| Items | borrowed, copies, or owned, decided at the source and by `map`; `collect` needs owned or copies | mirrors `for` and `match` on places and temporaries; a `Vec` of borrowed values has no Varyk type |
| Look-inside results | `get` and `find` on borrowed items open only in a `match`/`if let`/`while let` head; on a number or `bool` payload they are a plain `Option` of a copy | an `Option<&T>` has no Varyk spelling; the bindings are aliases exactly as in a `match` on what the payload is part of, so nothing new is needed; numbers copy everywhere else |
| Borrowed returns | inferred: every return part of one read-only parameter; one lifetime written when elision would not name it; the call result is an alias of the argument | closes M1 §4.4 without new syntax; getters stop copying; two roots, a `mut` root, and mixing stay errors, over-strict and sound |
| Number items | a chain over stored numbers or `bool` copies them at the source | the same copy `for` and `match` already make; downstream closures take plain values |
| `parse` | returns `Option<T>`, `T` from the expected type | Rust's error type has no Varyk name and no more information; `ok_or` gives the `Result` |
| `unwrap`, `expect` | never added, a principle rather than a cut | a call that stops the program on an absent value defeats the purpose of a language for services; `match`, `if let`, `?`, `unwrap_or` cover every use |
| Derives | `Clone` and `PartialEq` automatic where every field allows, `Debug` not | zero run-time cost, `.clone()` stays the one visible copy, `==` on enums is everyday code; `{:?}` does not exist |
| `HashMap` | in the table; `get` and `insert`, no indexing, no direct `for` | Rust's map cannot be assigned through an index; pairs need tuples |
| `as` | number types only | the `usize` friction of M2 §2.7 was real; anything else has a method |
| Reachability | a useless arm is an error | the check is free once exhaustiveness is exact; rustc's warning is silenced in generated code and an unreachable arm is a bug |
| Chars | no `char` type, no `chars()` | services split and trim strings; a character type is its own design |
| Standard calls | a table of declared signatures, never pass-through of unknown Rust methods and never signatures read from `std` | Varyk must know each call's type, receiver mode, borrow, and allocation to write the Rust and keep plain-word errors; `std` signatures need generics and traits to read; a facade is the pass-through |
