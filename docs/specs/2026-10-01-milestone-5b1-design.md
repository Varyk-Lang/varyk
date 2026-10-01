# Varyk milestone 5b1: async functions and tasks

Date: 2026-10-01.

**Status.** Design, not yet implemented. Extends the milestone-1 to
milestone-5a specs, `2026-09-23-varyk-design.md` ("M1 §n"),
`2026-09-25-milestone-2-design.md` ("M2 §n"),
`2026-09-26-milestone-3-design.md` ("M3 §n"),
`2026-09-29-milestone-4-design.md` ("M4 §n"), and
`2026-09-30-milestone-5a-design.md` ("M5a §n"). Their principles,
ownership rules, and compiler architecture stay in force. Varyk is
experimental and pre-1.0: anything here may change.

## 1. Summary and scope

Milestone 5b1 is the concurrency slice of milestone 5 (M5a §10). It adds:

- `async fn` and `.await`, on a multi-threaded tokio runtime built into
  `varyk-std`;
- **started calls**: a call to an async function without `.await` starts
  it at once and gives a `Task<T>`, as calling an async function does in
  JavaScript;
- `Task::all` and `Task::all_settled`, JavaScript's `Promise.all` and
  `Promise.allSettled`;
- `Shared<T>`, a read-only value many tasks can hold at once;
- `time::sleep`, the one new battery, so programs and tests have something
  to wait on;
- import of `pub async fn` from `.rs` modules.

It answers the two open questions milestone 5 was gated on that 5a moved
here (M1 §9):

- **The runtime is multi-threaded.** A current-thread runtime would not
  keep `Send` out of the picture: `tokio::spawn`, and the HTTP server and
  database crates 5b2 builds on, need `Send` futures under either flavour.
  Every type Varyk declares is already `Send` and `Sync`, so the bound
  costs a Varyk writer nothing; only a type from a `.rs` module can fail it
  (V0901).
- **Ownership transfer needs no syntax.** A started call is the one place
  a value is handed over: its arguments are owned slots (section 3). Every
  Varyk-declared parameter still borrows.

There is no `spawn` built-in and no `go` keyword. A writer who wants work
to run concurrently calls the function and keeps the task; a writer who
wants the result now writes `.await`. Calls are explicit about waiting, as
in JavaScript and Rust; inferring which functions wait, as Go's surface
does, is recorded as an open question (section 11).

Milestone 5b1 is an MVP like the milestones before it. The test for every
item was "does one of the examples in section 6, or 5b2's HTTP handlers,
need it"; what failed is listed in section 2.9. Only what this document
lists is supported; anything else is rejected with a diagnostic that names
the construct.

## 2. Language surface additions

A program using all of it:

```
struct Config {
    factor: i64,
}

async fn score(id: i64, cfg: Shared<Config>) -> i64 {
    time::sleep(10).await;
    id * cfg.factor
}

async fn lookup(id: i64) -> Result<string, Error> {
    if id < 0 {
        return Err(Error::new("ids are not negative"));
    }
    Ok(format!("user {}", id))
}

async fn main() {
    let cfg = Shared::new(Config { factor: 3 });

    let a = score(1, cfg.clone());
    let b = lookup(2);
    println!("{}", a.await);

    let ids: Vec<i64> = vec![1, 2, 3];
    let tasks = ids.iter().map(|id| score(id, cfg.clone())).collect();
    let scores = Task::all(tasks).await;
    println!("{}", scores.len());

    let checks = vec![lookup(4), lookup(-1)];
    let outcomes = Task::all_settled(checks).await;
    for outcome in outcomes.iter() {
        match outcome {
            Ok(name) => println!("{}", name),
            Err(e) => println!("failed: {}", e.message()),
        }
    }

    match b.await {
        Ok(name) => println!("{}", name),
        Err(e) => println!("failed: {}", e.message()),
    }
}

#[test]
async fn scores_scale() {
    let cfg = Shared::new(Config { factor: 2 });
    assert_eq(score(5, cfg).await, 10);
}
```

### 2.1 Lexical elements

`async` and `await` leave the reserved words (M1 §4.1) and become keywords.
`Task`, `Shared`, and `time` become reserved names (section 2.8).

### 2.2 Async functions

`async` may come before `fn` on a top-level function and on a method in an
`impl`, after `pub` when there is one: `pub async fn load(self) -> User`.
Parameters, `self`, `mut`, return types, and the body follow the rules for
any function. An async function's body may use `.await` and start calls;
an ordinary function's body may do neither (V0211).

- `async fn main()` is the program's entry: the generated `main` runs it
  on the runtime (section 5). Its shape is the shape of any `main` (no
  parameters, no return type), and nothing may call it (V0106): the
  generated `main` is an ordinary Rust function.
- `#[test] async fn` is a test, run on a runtime of its own; the M5a §2.7
  rules otherwise apply.
- An async function may not return part of a parameter (M4 §3.1; V0311):
  every value it returns is new.
- Async functions may not call each other in a cycle, directly or through
  others, whether the calls are awaited or started (V0214).

### 2.3 Two ways to call

A call to an async function, a method or a `.rs` import included, is one
of two kinds, decided by what follows it:

| Written | Kind | Type | Meaning |
|---|---|---|---|
| `f(x).await` | awaited | `T` | runs `f` here and gives its result; nothing is started |
| `f(x)` | started | `Task<T>` | starts `f` now, running alongside this function, and gives a task for its result |

An awaited call passes its arguments as any call does: borrowed by
default. A started call gives its arguments, the receiver of a method
included, to the task (section 3).

A started call may appear in four places only, so that no task is
dropped by accident:

- as the whole value of a `let` with a name, not `_`, and not a branch
  of an `if` or `match` there: `let a = fetch(1);`;
- as the receiver of `.detach()`: `send_email(user).detach();`;
- as an element of `vec!`: `vec![fetch(1), fetch(2)]`;
- as the whole value of the closure of a chain's last `map`, immediately
  followed by `collect`: `ids.iter().map(|id| fetch(id)).collect()` starts
  one task per id.

Anywhere else, a statement `f(x);` among them, it is an error (V0213): the
task would be thrown away and cancelled at once (section 2.4). Its help
offers `.await` and `.detach()`.

`.await` applies to an awaited call, to `Task::all` and
`Task::all_settled`, and to a local holding a task. On a `Vec` of tasks,
held by a local or written in place, it is V0215 (section 2.4), whose
help names `Task::all`; on anything else, V0212. `.await` inside a closure is an error (V0211):
the closure is not async.

### 2.4 `Task<T>`

| Call | Types | Meaning |
|---|---|---|
| `t.await` | takes `t`; gives `T` | waits for the task and gives its result |
| `t.detach()` | takes `t`; gives nothing | lets the task run on with nobody waiting for it |
| `Task::all(tasks).await` | takes `tasks: Vec<Task<T>>` | see section 2.5 |
| `Task::all_settled(tasks).await` | takes `tasks: Vec<Task<Result<U, E>>>`; gives `Vec<Result<U, E>>` | see section 2.5 |

- **A task that is dropped is cancelled.** When a task goes out of scope
  without being awaited, detached, or given to `Task::all` or
  `Task::all_settled`, the work it stands for stops at its next `.await`. A `?` that returns early from a
  function cancels the tasks it had started and not yet awaited.
- **A detached task** runs until it finishes or its runtime stops, when
  `main` or the test returns, whichever is first; nothing reports its
  result. A panic in it is printed to stderr by Rust's panic hook and
  does not stop the program.
- **A panic in a task** that is awaited, directly or through `Task::all`
  or `Task::all_settled`, continues in the function that awaits it, as if
  the call had been made there.
- **A task is used only where it is made.** A started call may be
  detached at once; a local holding a task may only be awaited or
  detached. A `Vec` of tasks, made by `vec!` or
  `collect`, may only be held by a local or given to `Task::all` or
  `Task::all_settled`, directly or from that local. Indexing, `push`, a
  `for` over it, passing it to a function, returning it, or any other use
  is an error (V0215), and so is writing `Task` as a type anywhere: its type is
  always worked out from the call. A task stays in the function that
  started it.
- **A task is always used.** A local holding a task, or a `Vec` of tasks,
  that nothing in the function awaits, detaches, or gives to `Task::all`
  or `Task::all_settled` is an error (V0213), so a forgotten `.await` is
  caught; so is binding one to `_` (`let _ = fetch(1);`). A `?` or `return` before that use still cancels the task, on
  purpose.
- `Task::all` and `Task::all_settled` are always followed by `.await`
  (V0212 otherwise).

### 2.5 `Task::all` and `Task::all_settled`

Both wait for every task in a `Vec` and give the results in the order of
the `Vec`, whatever order the tasks finish in.

- `Task::all(tasks).await`, for tasks whose `T` is not a `Result`, gives
  `Vec<T>`.
- `Task::all(tasks).await`, for tasks whose `T` is `Result<U, E>`, gives
  `Result<Vec<U>, E>`: the `Ok` values in order, or the first `Err` to
  arrive. On that `Err` it returns at once and cancels the tasks still
  running. `Task::all(tasks).await?` works as `?` does on any `Result`.
- `Task::all_settled(tasks).await` needs tasks whose `T` is a `Result`
  (V0200 otherwise, whose help names `Task::all`) and gives
  `Vec<Result<U, E>>`: every task runs to its end, and each outcome is
  kept.

An empty `Vec` gives an empty `Vec` (or `Ok` of one).

### 2.6 `Shared<T>`

| Call | Types | Meaning |
|---|---|---|
| `Shared::new(value)` | `value: T`, taken; gives `Shared<T>` | puts `value` where many can read it |
| `s.clone()` | reads `s`; gives `Shared<T>`, new | another handle to the same value; the value is not copied |

`T` is a struct, a Varyk one or one imported from a `.rs` module,
checked at every `Shared::new` whether or not a type is written (V0216).
Its
fields and methods are reached straight through the handle: `s.factor`,
`s.items.len()`, `s.describe()`. Everything reached through a `Shared` is
read-only, as a parameter without `mut` is: assigning to it, passing it
to a `mut` parameter, or calling a `mut self` method or a changing table
call on it is an error (V0310). A type that reaches a `Shared` cannot go
through `json` or `env` (V0210).

`Shared<T>` is written only as the type of a parameter or a `let`; any
other `T`, or `Shared` written in a field, a return type, or inside
another written type, is an error (V0216). Where `Shared` may appear is
checked on written types only: an inferred `Vec` or `Option` of `Shared`,
from `vec!` or `Some`, is fine. That covers both of its uses: one value given to many tasks
without a copy per task (start each with `s.clone()`), and 5b2's
application state, handed to every handler. A `Shared` cannot be compared
or printed (V0203), and is not a `T`: passing one where a `T` is
expected is V0200 (read the fields through it instead).

### 2.7 `time::sleep`

| Call | Types | Meaning |
|---|---|---|
| `time::sleep(ms)` | `ms: u64`; async, gives nothing | waits `ms` milliseconds without holding up other tasks |

It is an async call like any other: awaited (`time::sleep(10).await`) or
started.

### 2.8 Reserved names and imports

`Task` and `Shared` are reserved as `Error` is, and `time` as `json`,
`env`, and `log` are (M5a §2.10), with the same cases, `use time;` and
`use time::sleep;` among them. Each is V0113.

Every example, fixture, unit or integration test, snapshot, comment, and
live doc in this repository that names a struct `Task` renames it
(`Item`): `examples/todo`, nine fixtures, the `resolve`, `types`,
`borrow`, and `codegen` tests, and a line each of `docs/language.md` and
`docs/open-questions.md`, among others. The dated records under
`docs/specs/` and `docs/plans/` are left as written. A program with its own `Task` type must
rename it. This is the breaking change of the milestone, accepted so that
`Task::all` reads as it does in other languages (section 12).

`pub async fn` and `async` methods in `.rs` modules are imported under
the M3 signature rules, and called in either way of section 2.3. A Rust
async function that returns a reference is unsupported (V0108), as an
async Varyk function returning part of a parameter is.

### 2.9 Not in milestone 5b1

`race` and `any`; timeouts; channels; changing a value shared between
tasks (`Mutex`); a task anywhere but where it is made (section 2.4),
`push` of a task among them; `Shared` of anything but a struct, or in a
field or return type; async closures and `.await` in closures; async recursion; async functions
returning part of a parameter; cancelling a task by hand; a runtime
configured by the program (thread count, current-thread); inferred async
(section 11); HTTP and the database (5b2).

## 3. Ownership rules

One new rule, built from the existing owned slot (M1 §4.2):

- **The arguments of a started call are owned slots**, the receiver of a
  method included. A number or `bool` is copied; an owned local is moved
  in, and using it later is V0305; a new value (a call's result, a
  literal, `.clone()`) is moved in. A borrowed place, a parameter, a field
  or element of one, or an alias, is V0304, whose note says the task may
  outlive this function and offers `.clone()` or `Shared`. String literals
  follow the literal rule, the one allocation the compiler inserts.
- **A started call may not pass to a `mut` parameter**, a `mut self`
  receiver included (V0309): the task changes its own copy, and the change
  could never be seen.
- `t.await` and `t.detach()` take the local `t`; `Task::all` and
  `Task::all_settled` take the local, `vec!`, or `collect` they are
  given. Using it
  afterwards is V0305. Section 2.4 keeps tasks out of every other place;
  a task local detached inside a closure, which only borrows it, is
  V0304 (`.await` there is already V0211).
- `Shared::new` takes its argument. Places reached through a `Shared` are
  read-only (section 2.6).

Inside the task the function still borrows: the generated task owns its
copies of the arguments and lends them to the function (section 5), so the
same function can be awaited or started. Names held across an `.await`
need no rule: they are borrows inside the async function's own future.

The compiler inserts no allocation beyond the literal rule. Each started
call allocates its task inside tokio, as `push` allocates inside `Vec`:
the writer asked for it by starting the call. `Shared::new` allocates
once, written by the writer; `s.clone()` copies a pointer.

## 4. The `varyk-std` additions

- `run(future)`: builds the multi-threaded runtime (one worker per core,
  `enable_all`, so the I/O and timer drivers are there for any `.rs`
  dependency that needs them) and blocks on the future. If the runtime cannot be built, it
  prints the reason to stderr and exits with code 1.
- `Task<T>`: a wrapper on tokio's `JoinHandle<T>` that aborts the task when
  dropped, with `start(future)` (`tokio::spawn`) and `detach(self)`, and
  that is itself a `Future` of `T`. Awaiting resumes a panic with
  `std::panic::resume_unwind`. A task cancelled under its waiter, which
  happens only while the runtime shuts down and the waiter is being
  dropped too, leaves the waiter pending rather than make up a value.
- `Task::all` on futures-util's `join_all`, and `Task::try_all` on its
  `FuturesUnordered`, each task tagged with its index: it returns at the
  first `Err` to finish, whatever the list's length, and puts the `Ok`
  values back in order. (`try_join_all` is not used: above 30 futures it
  waits in list order.) Dropping the rest cancels them. `try_all` is
  `Task::all` for `Result` tasks, chosen by the backend; `Task::all_settled` is the plain `all` on `Result` tasks and
  needs no function of its own.
- `time::sleep(ms: u64)` on `tokio::time::sleep`.

`Shared<T>` needs nothing here: it is `std::sync::Arc<T>`.

A program **uses `varyk-std`** (M5a §1) also when it has an async `main`
or an async test, a started call, `Task::all` or `Task::all_settled`, or
`time::sleep`, since the generated Rust then calls `::varyk_std`; the
single-file dependency and the V0404 checks (M5a §5) follow. `Shared`
alone does not.

New dependencies: tokio with the `rt-multi-thread` and `time` features
(no macros: the runtime is built with tokio's builder, so a package never
depends on tokio itself), and `futures-util` without default features
(`alloc` only) for `join_all` and `FuturesUnordered`. The M5a §4 rules hold: MSRV, no `unwrap` or `expect`.

## 5. Generated Rust

| Varyk | Rust |
|---|---|
| `async fn main() { .. }` | `fn main() { ::varyk_std::run(async { .. }) }` |
| `#[test] async fn t() { .. }` | `#[test] fn t() { ::varyk_std::run(async { .. }) }` |
| `async fn f(u: User) -> i64` | `async fn f(u: &User) -> i64` |
| `f(u).await` | `f(&u).await` |
| `f(u, 3)`, started | `match (u, 3,) { (varyk_0, varyk_1,) => ::varyk_std::Task::start(async move { f(&varyk_0, varyk_1).await }) }` |
| `u.load()`, started | `match (u,) { (varyk_0,) => ::varyk_std::Task::start(async move { User::load(&varyk_0).await }) }` |
| `Task::all(ts).await` | `::varyk_std::Task::all(ts).await`, or `try_all` for `Result` tasks |
| `Task::all_settled(ts).await` | `::varyk_std::Task::all(ts).await` |
| `t.detach()` | `t.detach()` |
| `time::sleep(ms).await` | `::varyk_std::time::sleep(ms).await` |
| `Shared<T>`, `Shared::new(v)` | `::std::sync::Arc<T>`, `::std::sync::Arc::new(v)` |

`::varyk_std::start()` for logging (M5a §7.5) goes inside the `run` block
of an async `main`, first; an async test gets what a test gets today. The `match` around a started call evaluates every argument in order
before any name is bound, exactly as a direct call would, the shape the
`log` calls already use. The `varyk_` names (M5a §2.10) cannot shadow a
function of the program, and a local of the program that happens to use
one is evaluated before the arm binds its own. A call with no arguments
matches on `()`. Each `varyk_N` is passed as the parameter takes it, by value for a number or an owned Rust
parameter and by reference otherwise.

`Send` and `Sync` are never written. Every type Varyk declares, `Error` and
`Shared` of one included, has both. A `.rs` module can still bring in a
value that does not: as an argument of a started call, held across an
`.await` anywhere in the started function, or inside the future of an
imported async function; or, since the task lends its arguments and a
`Shared` holds its struct across threads, a value that is not `Sync`
(a `Cell` or `RefCell` field). rustc then rejects the `Task::start`
(`run` does not need `Send`). The driver recognises rustc's "cannot be
sent between threads safely" and "cannot be shared between threads
safely" errors at a started call, whatever the cause, and
reports V0901 at the Varyk line, naming the Rust type, instead of V0900,
since it is the `.rs` module's choice and not a compiler bug.

## 6. Milestone-5b1 examples

New single files, each with its expected output in `tests/examples.rs`:

- `examples/tasks.vr`: two different async functions started and then
  awaited one after the other, with `time::sleep` so they overlap;
  `Task::all` over ten started calls; an async method; an `#[test] async
  fn`, which `tests/examples.rs` runs with `varyk test`, beside its
  `varyk run` case.
- `examples/fanout.vr`: `Task::all` over `Result` tasks, one failing,
  with `?` in a helper and the error printed in `main`; `Task::all_settled`
  over the same calls, every outcome printed.
- `examples/shared.vr`: one `Shared<Config>` given to 10,000 started tasks
  through `s.clone()`, their results summed; fast enough for the example
  test.

Updated: `examples/todo` renames its `Task` struct (section 2.8).

## 7. Compiler changes

Line numbers are approximate.

### 7.1 Syntax (`varyk-syntax`)

- `lexer.rs` (~49): `async` and `await` leave `RESERVED_KEYWORDS` and
  become keywords.
- `parser/item.rs`: `async` before `fn`, after an optional `pub`, at the top
  level and in an `impl`; the AST function gains `is_async`.
- The expression parser: `.await` as a postfix form beside field access;
  the AST gains an `Await` expression.

### 7.2 Resolve

- `Task`, `Shared`, and `time::` resolve to the standard table; the
  reserved names of section 2.8, V0113.
- The `main` and `#[test]` shape checks (V0106, V0114) accept `async`.
- `interop/items.rs` (~375): `async` no longer skips a function or method;
  an async Rust function returning a reference is V0108.

### 7.3 Types

- `Ty` gains `Task(T)` and `Shared(T)`; `Task` used or written where it
  may not be, V0215; `Shared` of anything but a struct, at `Shared::new` or
  written, or written where it may not be, V0216.
- Each call to an async function is marked awaited or started (section
  2.3) and typed `T` or `Task<T>`; `.await` on a `Task<T>` gives `T`.
- `builtins.rs`: rows for `Task::all`, `Task::all_settled`, `detach`,
  `Shared::new`, `clone` on `Shared`, and `time::sleep` (an async row).
- The context checks, V0211 to V0213 (the unused-task case of V0213
  included), and the async call cycle, V0214, a
  walk over the calls between async functions like the one behind M4's
  recursion check for borrowed returns.
- V0311 for an async function whose returns are part of a parameter.
- `Builtin::uses_std` and the program's use of `varyk-std` cover the
  cases of section 4.

### 7.4 Borrow analysis

- The arguments of a started call are owned slots (section 3), V0304 with
  the started-call note, V0305 after a move into a task; V0309.
- `.await`, `detach`, and the `Task` rows take their operand (section 3).
- Places through a `Shared` are read-only, V0310.

### 7.5 Backend

As section 5: `run` around an async `main` and async tests, `async` on
functions, the `match` around each started call, `try_all` for `Result`
tasks, and `Arc` for `Shared`.

### 7.6 Driver

- `driver/messages.rs`: rustc's "cannot be sent between threads safely"
  and "cannot be shared between threads safely" errors at a started call
  become V0901 (section 5).
- `crates/varyk-std/Cargo.toml`: tokio and `futures-util`.

## 8. Diagnostics

New codes, each with a fixture, a registration in `tests/errors.rs`, and a
row in `docs/language.md`:

| Code | Meaning |
|---|---|
| V0211 | `.await`, or a call to an async function, in an ordinary function; `.await` in a closure |
| V0212 | `.await` on something that is not a call to an async function, `Task::all`, `Task::all_settled`, a local holding a task, or a `Vec` of tasks (that is V0215); `Task::all` or `Task::all_settled` without `.await` |
| V0213 | a started call anywhere but a named `let`, the receiver of `.detach()`, a `vec!` element, or a collected `map` closure's value, or a task never awaited, detached, or given to `Task::all` or `Task::all_settled`, or bound to `_`: its task would be thrown away |
| V0214 | async functions that call each other in a cycle, naming it |
| V0215 | a task, or a `Vec` of tasks, used other than as section 2.4 allows, or `Task` written as a type |
| V0216 | `Shared` of anything but a struct, at `Shared::new` or written, or `Shared` written anywhere but a parameter's or a `let`'s type |
| V0309 | a started call passing a value to a `mut` parameter or `mut self` receiver |
| V0310 | changing something reached through a `Shared` |
| V0311 | an async function that returns part of a parameter |
| V0901 | a started call whose work holds a value from a `.rs` module that cannot be sent to, or shared with, another thread |

Reused: V0106 for a call to an async `main`; V0108 for an async Rust
function returning a reference; V0113 for the new reserved names; V0200
for `Task::all_settled` on tasks that are not `Result`s, or a `Shared`
given where a `T` is expected; V0203 for printing or comparing a
`Shared`; V0210 for a `Shared` in a `json` or `env` call; V0304 for a
borrowed place given to a started call, or a task detached inside a
closure; V0305 for a value used after a task took it.

Headlines in plain words, for example V0213: "this starts `fetch` and
then throws its task away, which stops it", with help "wait for it with
`.await`, or let it run on its own with `.detach()`"; and V0211:
"`fetch` waits for something, so only an `async fn` can call it", with the
fix-it adding `async` to the function.

## 9. Testing and definition of done

- Unit tests for the new table rows, the awaited/started marking, the
  context checks, the cycle check, and the started-call slots.
- `insta` snapshots of the generated Rust: an async `main` with logging,
  an async test, an async method, an awaited and a started call with each
  kind of argument, both forms of `Task::all`, and `Shared`.
- Soundness templates (M2 §7) for every new construct, among them an
  imported `.rs` async function awaited and started, a started method,
  started calls in a collected `map` over borrowed and copied items,
  `Shared` fields and methods, and `.await` inside `for` (over a chain
  with a closure among them), `while`, `match`, and `if let` in a started
  function.
- `varyk-std` unit tests: dropping a `Task` aborts it; `detach` does not;
  `Task::all` keeps order; `try_all` returns at the first `Err` to finish
  and aborts the rest, with more than 30 tasks and the failing one last
  in the list; a panic resumes in the waiter.
- Interop: `pub async fn` imported (the `ignores_async_fn` test becomes
  an import test); an async function returning a reference refused.
- A V0210 fixture: `env::parse` into a `let` typed `Shared<Config>`.
- A V0404 fixture: a package with only an async `main` and no `varyk-std`
  in its `Cargo.toml`.
- The V0901 fixtures: a `.rs` struct holding an `Rc`, and one holding a
  `Cell`, each passed to a started call.
- `docs/language.md` gains an "Async functions and tasks" section
  (sections 2.2 to 2.7 and the not-in list), the new codes, and the
  widened rule for when a program uses `varyk-std` (section 4);
  `docs/design.md` the concurrency model; `docs/open-questions.md` and
  `docs/roadmap.md` (sections 10 and 11).

Done when every example in section 6 and every earlier example builds and
prints its expected output, no live doc, example, or test still names a
struct `Task` (section 2.8), `varyk test` passes on `tasks.vr` and `users`,
and the gate of `AGENTS.md` passes on stable and 1.85.

## 10. Roadmap changes

The 5b1 entry becomes:

- `async fn`, `.await`, and started calls on a built-in multi-threaded
  tokio runtime, with `Send`, `Sync`, and `Pin` kept out of the surface
- `Task<T>`: cancelled when dropped, `detach`, `Task::all`, and
  `Task::all_settled`
- `Shared<T>` for a read-only struct held by many tasks
- `time::sleep`, and `pub async fn` imported from `.rs` modules
- Examples `tasks`, `fanout`, and `shared`; `docs/language.md`,
  `docs/design.md`, `docs/open-questions.md`, and `docs/roadmap.md`
  updated

5b2 gains a note: application state shared by every handler is a
`Shared<T>`.

## 11. Open questions

Answered, marked so in `docs/open-questions.md`:

- "Multi-threaded or current-thread async runtime, and how do `Send` and
  `Sync` failures surface?" Multi-threaded; Varyk types are always `Send`
  and `Sync`, and a `.rs` type that is not is V0901 (section 1).
- "What syntax should explicit ownership transfer use for Varyk-declared
  functions?" None: the arguments of a started call are the one transfer
  (section 3).

Added:

- Should Varyk infer which functions are async, as it infers borrows, and
  await every call implicitly, with concurrency spelled `go f(x)`? That is
  Go's surface on Rust's runtime; 5b1 keeps waiting explicit.
- Should tasks race, time out, or talk through channels, and in what
  shape?
- Should tasks change a shared value, and through what (`Mutex`, an actor,
  a channel)?

## 12. Decisions

Appended to the decisions log of M1 §10.

| Decision | Choice | Why |
|---|---|---|
| Waiting | explicit `async fn` and `.await` | familiar to JavaScript and Rust writers and to models; a waiting point is visible |
| Starting | a call without `.await` starts a task at once; no `spawn` | JavaScript's eager promises; two unawaited calls really overlap |
| Runtime | multi-threaded tokio inside `varyk-std` | tokio, the HTTP server, and the database need `Send` anyway; tasks spread over cores |
| Ownership transfer | a started call's arguments are owned slots; no new syntax | the only place a value outlives its function; parameters still borrow |
| Dropped tasks | cancelled; `detach` to keep running | no work leaks past the code that started it; a forgotten `.await` is an error, not a background job |
| Waiting on many | `Task::all` (first `Err` cancels the rest) and `Task::all_settled` (every outcome) | `Promise.all` and `Promise.allSettled`; two names keep cancellation visible |
| Names | `Task` reserved; `todo`'s `Task` renamed | `Task::all` reads as in other languages; a type with associated functions follows `Vec::new()` |
| Shared values | `Shared<T>` of a struct, read-only, `Arc` underneath, in parameters and `let`s only | one value for thousands of tasks without a copy each; 5b2's application state |
| Where tasks live | made by a `let`, `.detach()`, `vec!`, or collected `map`; only awaited, detached, or given to `Task::all` or `Task::all_settled` | no task is dropped by accident or moved out of a borrowed place |
| Async recursion | refused | Rust needs a boxed future for it; nothing in the examples recurses |
