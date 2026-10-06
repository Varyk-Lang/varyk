# Varyk milestone 5b4: `varyk-http` and the golden path

Date: 2026-10-05.

**Status.** Design, not yet implemented. Extends the milestone-1 to
milestone-5b3 specs, `2026-09-23-varyk-design.md` ("M1 §n"),
`2026-09-25-milestone-2-design.md` ("M2 §n"),
`2026-09-26-milestone-3-design.md` ("M3 §n"),
`2026-09-29-milestone-4-design.md` ("M4 §n"),
`2026-09-30-milestone-5a-design.md` ("M5a §n"),
`2026-10-01-milestone-5b1-design.md` ("M5b1 §n"),
`2026-10-02-milestone-5b2-design.md` ("M5b2 §n"), and
`2026-10-03-milestone-5b3-design.md` ("M5b3 §n"). Their principles,
ownership rules, and compiler architecture stay in force. Varyk is
experimental and pre-1.0: anything here may change.

The package this milestone exists for, `varyk-http`, lives in its own
repository, `Varyk-Lang/varyk-http`, and gets its own spec there once
this one is approved, as `varyk-sql` did (M5b3). This spec is the
compiler side: how a Varyk function becomes a route handler, what the
compiler checks and generates for it, the two facade shapes the package
needs, the status on `Error`, and the command line. Section 6 fixes the
part of the package the compiler depends on.

## 1. Summary and scope

Milestone 5's bar is one golden path: a users API on a database is
`varyk init`, `varyk add http sql`, one file, and `varyk run` away, within
fifteen minutes of `cargo install varyk`, and `varyk build --release`
leaves one native executable. It is met when 5b4 and milestone 5c
(time, ids, and bytes, section 10) are done. After 5b4 the file is this, the site's hero
figure made real:

```varyk
struct User {
    id: i64,
    name: string,
}

struct NewUser {
    name: string,
}

struct State {
    db: sql::Pool,
}

async fn get_user(id: i64, state: Shared<State>) -> Result<Option<User>, Error> {
    state.db.first("select id, name from users where id = ?", id).await
}

async fn create_user(user: NewUser, state: Shared<State>) -> Result<http::Response, Error> {
    if user.name.is_empty() {
        return Err(http::bad_request("a user needs a name"));
    }
    let created: User = state.db.one("insert into users (name) values (?) returning id, name", user.name).await?;
    let mut r = http::Response::json(created);
    r.set_status(201);
    Ok(r)
}

async fn open_db() -> Result<sql::Pool, Error> {
    let db = sql::connect("sqlite://users.db?mode=rwc").await?;
    db.run("create table if not exists users (id integer primary key, name text not null)").await?;
    Ok(db)
}

async fn main() {
    let port: u16 = 3000;
    let db = match open_db().await {
        Ok(db) => db,
        Err(e) => {
            log::error("cannot open the database: {}", e);
            return;
        }
    };
    let mut app = http::App::new(Shared::new(State { db: db }));
    app.get("/users/{id}", get_user);
    app.post("/users", create_user);
    if let Err(e) = app.serve(port).await {
        log::error("cannot serve: {}", e);
    }
}
```

Nothing in it is new syntax. What is new is that `get_user` may be
written where an argument goes, and that the compiler checks the route
`"/users/{id}"` against `get_user`'s signature before cargo runs.

Varyk has no function values, so some part of a route table must be
known to the compiler. 5b4 adds:

1. the route table: when the build holds the package whose crate name
   is `varyk-http`, its `App` type's route and hook calls are intrinsics
   the compiler checks and compiles (section 2.1);
2. the binding rules: a handler's parameters bind to the path by name,
   to the query string by name, to the JSON body by type, to the state by
   type, and to a struct of the package by type (section 2.2), and its
   return value is the response (section 2.3);
3. `before`, `before_on`, and `after` hooks with fixed signatures
   (section 2.4), and `app.request` for tests (section 2.5);
4. an optional status on `Error` (section 2.6);
5. two facade shapes: a type parameter `&T: Serialize` in a parameter, and
   a bare `varyk_std::Error` return (section 2.7);
6. `varyk add http sql`: several official packages in one call, by
   shorthand or full name (section 2.9).

Everything else about HTTP, the server and client surface, settings and
their defaults, WebSockets, server-sent events, multipart, metrics, and
the `users` demo, is `varyk-http`'s, within the contract of section 6.
The facade rule stands: a crate is reached through a `.rs` module in the
same package (M3 §1), and the program's crate never names axum.

### 1.1 What was decided and why

- **axum with tower-http underneath, reqwest with rustls for the
  client.** Three criteria were set: lightweight and performant, the needs
  of a real API possible (tracing, CORS, body limits, timeouts,
  compression, auth), and a Varyk program as simple as Go or JavaScript.
  Writing a server by hand was ruled out as a project of its own. Of the
  frameworks with a middleware shelf, axum sits in the top tier of
  TechEmpower round 23 (within 10 to 15 percent of actix-web on JSON and
  plaintext, level with it on the database test, in the tier of Go's
  fastest frameworks, an order of magnitude above Node and Python), runs
  on the same multi-threaded tokio runtime `varyk-std` starts, and is
  maintained by the tokio organisation, which Varyk already bets on. The
  actix family was rejected for running handlers on its own per-worker
  runtimes beside ours; salvo and poem for resting on one maintainer
  each. axum's criticised surface, the `Handler` trait, extractor
  ordering, and its compile errors, never reaches a Varyk user: the
  compiler generates every adapter with concrete types, and a generation
  bug is a V0900 against the compiler, not a trait error against the
  program.
- **The compiler knows one package by name.** Every design that keeps
  the program simple and the route checked has the compiler knowing how a
  path binds to a signature. A general "parameter that takes a function"
  facade shape would need the same knowledge plus a mechanism nothing
  else uses; closures or function types in the language are open
  questions far larger than this milestone; attributes on handlers would
  scatter the route table that 5b2 chose to keep in one place. So
  `varyk-http` is special the way `varyk-std` is, said outright, and a
  package nobody designed with the compiler can still offer a plain
  dispatch function through the facade shapes every package has.
- **One binding rule for the package's own types.** A handler parameter
  whose type is a struct of `varyk-http` binds through the package, not
  through the compiler, so `Request` today and `WebSocket`, `Sse`, and
  `Multipart` later need no compiler change. That is what lets those
  items sit on `varyk-http`'s own checklist inside this milestone rather
  than in a follow-up milestone.
- **Auth is a function call, not a slot.** `before` accepts or rejects. A
  handler that needs the authenticated user calls a Varyk function of the
  program, `let user = auth(req, state)?;`, which is explicit and
  Go-like; a typed slot that `before` fills was considered and dropped as
  mechanism.
- **Custom headers and status are a return type**, `http::Response`,
  beside the plain return shapes, so the common handler returns a value
  and the one that needs a `location` header or a 201 builds a response.
- **What a production API needs that the language lacks** was listed
  while designing this: date and time, UUIDs, and bytes. They are a slice
  of their own before milestone 5's bar is called met (section 10), so
  that the users API does not ship `created_at` as a string.

## 2. Language surface

### 2.1 The route table

When the package graph of a build (M5b2 §4) holds a package whose crate
name is `varyk-http`, under whatever key the program gave it (`http`
after `varyk add http`), the compiler marks the struct `App` of that
package's root as the route table. When the package being checked is
itself the one named `varyk-http`, its own root `App` is marked the same
way, so the package's own tests can add routes and send requests
through `app.request`. These calls on it are intrinsics,
checked and compiled by the compiler, and shadow any method of the same
name the package's facade declares:

| Call | Meaning |
|---|---|
| `http::App::new(state)` | `state` is a `Shared<T>`; `T` is the app's state type |
| `app.get(path, f)`, `app.post(path, f)`, `app.put(path, f)`, `app.patch(path, f)`, `app.delete(path, f)` | a route: `f` names an async function of the program (section 2.2) |
| `app.before(f)`, `app.before_on(prefix, f)` | `f` runs before every request a route matches, or before those whose route's path starts with `prefix` (section 2.4) |
| `app.after(f)` | `f` runs on every response the router makes (section 2.4) |

`path` and `prefix` are string literals (M5b3 §2.3); `App::new` takes
its argument as `Shared::new` does (M5b1 §3). The route and hook calls
change the app, as a `mut self` method does, so the app's local is `let
mut` (V0302 otherwise). Every other method of
`App` is an ordinary method of an imported struct (M3 §4), two of them
fixed by the contract of section 6: `app.serve(port).await`, which
serves until SIGTERM or ctrl-c, with `port` a `u16`, and gives
`Result<bool, Error>`, an `Err` when the port cannot be bound; and
`app.request(req).await`, one request in process with no port (section
2.5), giving `http::Response`. The settings, CORS, and metrics are
further such methods, and the package documents them.

`App` is otherwise an ordinary struct imported from the package's
facade: a program names it as `http::App`, holds one, passes it, and
returns it (`fn build_app(state: Shared<State>) -> http::App`, which
tests use). One rule keeps the state type, taken from `App::new`, known
at every route call without a type parameter on `App` in the surface: a
route or hook call is made on a local bound to `App::new(..)` in the
same function, not inside a loop (which would register the route once
per turn, and the router refuses a route twice), and the local is never
assigned again (which would change the state type the routes were
checked against). Anything else is V0221.

### 2.2 Routes and binding

A route's path is `/` followed by segments separated by `/`, each a
literal of ASCII letters, digits, `-`, `_`, `.`, and `~`, or `{name}`
with `name` an identifier, every name distinct, no empty segment, and no
trailing `/` except the root `"/"`. Anything else is V0222. A path is
matched whole; `/users/{id}` does not match `/users/1/posts`. Among the
routes of one app, two with the same method and path are V0222 at the
second. Any other conflict the router finds is never a panic: an `Err` from
`serve`, and a 500 with the conflict logged from `request` (section
6.1).

`f` is the path of an `async fn` declared in the current package, in
any module, visible from the route call by the ordinary rule (V0105;
`admin::list_users` is `pub`), not `main` (V0106), not a `#[test]`
function (V0114), not a method, not an imported Rust function, not a
function of another package, not a closure, and not a local. Each of
its parameters
binds by one rule, decided by its name and type, in this order:

1. a parameter whose name is a `{name}` of the path is a **path
   parameter**; its type is an integer type, `bool`, or `string`;
2. a parameter of type `Shared<T>` is the **state**; `T` is the type the
   app's `App::new` was given; at most one;
3. a parameter whose type is a struct named at the `varyk-http`
   package's root (section 6.1) other than `App`, `Response`, and
   `Client` (`http::Request` in 5b4; `WebSocket`, `Sse`, `Multipart`, or
   another when the package adds it) is **bound by the package**; at
   most one of each type;
4. on `post`, `put`, and `patch`, one remaining parameter of a data type,
   a struct or enum, or a `Vec` or `HashMap` of a convertible type (M5a
   §2.9), is the **body**, read from JSON; it joins the serde reach
   analysis as a read type (M5a §2.4) with the attribute checks of
   V0209, and a Rust type or a type of another package, at the top or
   inside, is V0210, as for `json::parse`;
5. any other parameter whose type is an integer type, `bool`, `string`,
   or `Option` of one is a **query parameter**, bound by name from the
   query string; a plain type is required and its absence is a 400; an
   `Option` is `None` when absent.

A parameter that fits none of these (an `http::Response` among them), a
`{name}` with no parameter, a second body, a body on `get` or `delete`, a `Shared` of another type, a
second `Shared`, or a path parameter of another type is V0219, one
diagnostic per problem, at the route call, naming the parameter or the
segment. An `f` that is not an async function of the program, or whose
return type is outside section 2.3, is V0220.

A path parameter whose segment does not parse as its type (`abc` for an
`i64`), a query parameter likewise, a required query parameter that is
missing, and a body that is not JSON of the type, are each a 400 whose
body names the parameter and the problem, built by the package; the
handler is not called. A path no route matches is a 404, and a matched
path with another method a 405, both the package's.

The handler is called as any Varyk function is (M1 §4.2): its parameters
borrow by default. The adapter owns what it parsed, the path and query
values, the body, the state handle, and a package value made for this
request, and lends each one as the parameter's mode says, as a started
call lends its `varyk_N` (M5b1 §5): a number or `bool` by value, by
mutable reference from a `let mut` for a `mut` parameter, and by
reference otherwise. Everything the adapter lends lives until the handler
returns, so borrow analysis has nothing new to learn.

### 2.3 Responses

A handler's return type is one of:

| Returns | Response |
|---|---|
| nothing | 204 with no body |
| `T`, a type `json::stringify` writes (M5a §2.9), other than an `Option` at the top | 200, JSON of `T` |
| `Option<T>` | 200 with JSON, or 404 for `None` with the package's not-found body |
| `http::Response` | as built (section 6.2) |
| `Result<X, Error>` with `X` one of the above but nothing | `X`'s response, or the error's (section 2.6) |

`T` joins the serde reach analysis as a write type, with the same checks
as a `json::stringify` argument (V0209, V0210). `Vec<User>` and
`HashMap<string, i64>` are `T`s; `string` is one too, sent as a JSON
string. Varyk has no `()`, so a handler that can fail and has nothing
to send returns `Result<http::Response, Error>` with
`http::Response::empty()`, a 204. Any other return type is V0220.

A `before` function that returns `Ok(true)` lets the request through;
`Ok(false)` is a 403 with the package's forbidden body, for a check that
has no message to give; `Err(e)` is `e`'s response.

### 2.4 Hooks

`before`, `before_on`, and `after` take a function of the current
package, chosen as a handler is (section 2.2), with a fixed signature,
its parameters by position:

| Hook | Signature |
|---|---|
| `before(f)`, `before_on(prefix, f)` | `async fn f(req: http::Request) -> Result<bool, Error>`, or with a second parameter `state: Shared<T>` |
| `after(f)` | `async fn f(req: http::Request, res: http::Response)`, returning nothing, `res` `mut` when the hook changes it, or with a third parameter `state: Shared<T>` |

The parameter names are the writer's; the parameters come in the order
of the table. An `after` hook changes the response through its `mut`
parameter (`res.set_header(..)`) rather than returning one, since an
async function cannot return part of a parameter (V0311). Hooks run in
the order they were registered; a request no route matches (a 404 or
405) runs no `before` hook; for one a route matches, every `before` and
`before_on` whose
prefix matches runs before the handler, stopping at the first
`Ok(false)` or `Err`, and every `after` runs on the response, the
handler's or a `before`'s rejection, in turn. A `prefix` is a path as in section 2.2 with no
`{name}` segments, matched on whole segments against the path pattern
of the route the request matched, as the program wrote it: `"/admin"`
covers the routes `/admin` and `/admin/users/{id}`, not
`/administrators`. A hook of the wrong
shape, a `Shared` of another type than the app's included, is V0220.
The adapter lends the hook its own incoming request, and an `after`
hook the response, each as the parameter's mode says. A hook's request
cannot be `mut` (V0220, with the note "a change to the request here
does not reach the handler"), since each hook reads the request and
none passes one on. There is no wrapping hook (one that calls the
next itself); `before` and `after` together cover auth, checks, logging,
and response headers, the package's request tracing covers timing, and
a wrapping hook is an open question (section 11).

### 2.5 Requests in process

`app.request(req).await` runs one request through the hooks and routes
without binding a port, and gives the `http::Response` the server would
have sent. `req` is an `http::Request` built by the program
(`http::Request::new("GET", "/users/1")`, then `set_header`, `set_body`;
section 6.2), the same type a handler receives. It is for `varyk test`:

```varyk
#[test]
async fn gets_a_user() {
    let app = build_app(Shared::new(test_state()));
    let r = app.request(http::Request::new("GET", "/users/1")).await;
    assert_eq(r.status(), 200);
}
```

with `build_app` the function `main` uses too:

```varyk
fn build_app(state: Shared<State>) -> http::App {
    let mut app = http::App::new(state.clone());
    app.get("/users/{id}", get_user);
    app
}
```

(`state.clone()` because a parameter is borrowed, as for any `Shared`.)
`request` reads the app (`&self` in the facade), so a test may send
several requests to one app. It is an awaited call like any async call
(M5b1 §2.3), and may be started into a task.

### 2.6 `Error` with a status

`Error` (M5a §2.3) gains an optional HTTP status:

| Call | Types | Meaning |
|---|---|---|
| `Error::new(text)` | unchanged | an error with no status |
| `Error::with_status(status, text)` | `status: u16`; `text: string`, owned | an error with that status |
| `e.message()` | unchanged | the message |
| `e.status()` | gives `Option<u16>` | the status, when one was set |

`Error` stays cloneable and comparable (the status takes part in both),
still prints its message alone with `{}`, and still cannot go through
JSON. `Error::with_status` takes its text as `Error::new` does, and
keeps any status as given rather than giving a `Result`. `varyk-http`
writes its constructors in Varyk over it, in its `src/lib.vr`:
`http::bad_request(text)` (400), `http::unauthorized(text)` (401),
`http::forbidden(text)` (403), `http::not_found(text)` (404),
`http::conflict(text)` (409), and `http::error(status, text)` for any
other, with `status` a `u16`; `text` is a `string`, read.

How the package sends an error is fixed here because it is AGENTS.md's
"safe by default" rule for the network: an `Err(e)` with a status from
400 to 599 is sent with that status and the body `{"error":
"<message>"}`; an `Err(e)` with no status, or with a status outside 400
to 599, is a 500 whose message is logged at error level with the
method and path and whose body is a fixed `{"error": "internal error"}`.

A status therefore means the message was written for the client. Only
Varyk code sets one, with `Error::with_status` or `varyk-http`'s
constructors over it; no official package's Rust calls
`Error::with_status`, and its Rust documentation says so. A `?` on a `varyk-sql` call in a handler
therefore never sends a database message to a client.

### 2.7 Two facade shapes

**A type parameter in a parameter.** A `pub fn` or method of a `.rs`
module may have one type parameter with the bound `serde::Serialize +
?Sized` or `varyk_std::serde::Serialize + ?Sized`, written inline and by
full path, appearing exactly once, as `&T` in one parameter, and nowhere
in the return:

```rust
pub fn json<T: varyk_std::serde::Serialize + ?Sized>(value: &T) -> Response
pub async fn post<T: serde::Serialize + ?Sized>(&self, url: &str, body: &T) -> Result<Response, varyk_std::Error>
```

`?Sized` is required because a string argument is lent as `&str`, as
for `json::stringify`, whose own bound is the same; the bound without it
is V0108 with a note saying to add it.

At a call, the argument is any type `json::stringify` writes (M5a §2.9):
it joins the serde reach analysis as a write type with the checks of
V0209, and a Rust or foreign type is V0210. The argument is read, not
given away, as a `json::stringify` argument is (M5a §3), so the local
stays usable after the call. A function may carry this parameter and, as
M5b3 §2.1 allows, a `DeserializeOwned` parameter filled from the result,
but not both at once: that, `T` twice, `T` by value or in a `Vec`, a
`where` clause, or another bound keeps today's rule, imported but not
callable (V0108), with the note saying which.

**A bare `varyk_std::Error` return.** A `pub fn` or method of a `.rs`
module may return `varyk_std::Error`, by that full path, as its whole
return type; Varyk sees it as `Error`, for a facade that makes an error
of its own. `varyk_std::Error` in a parameter, a field, or inside
another type still waits for a package that needs it (M5b3 §2.4).

Both shapes make the program use `varyk-std` under the rule of M5b3
§2.4: a signature of the program's own `.rs` modules naming them counts
whether or not it is called, and one reached through a dependency
package counts at the call.

### 2.8 What a route makes the program use

A program with an `App::new` call needs `varyk-std` (the generated
adapters derive serde through it, section 4), under M5a §5.1's V0404
help, and sets up logging as a program with a `log` call does (M5a
§2.6), whether or not it has one: the package's request lines and the
messages of 500s are written through `tracing` in its Rust, which sets
nothing up by itself. Its handlers' body and return types join the reach sets as
section 2.2 and 2.3 say.

### 2.9 `varyk add http sql`

`varyk add` (M5b3 §2.6) gains a second shorthand and accepts several in
one call:

| Written | Runs |
|---|---|
| `http` or `varyk-http` | `cargo add varyk-http --rename http` |
| `sql` or `varyk-sql` | `cargo add varyk-sql --rename sql` |

The rule: when the first argument is an official name, a shorthand or a
full name, and so is the second, each leading official name runs one
`cargo add`, in order, stopping at the first failure, and no other
argument is taken: anything after them is refused with "pass other
arguments with one package at a time: `varyk add sql --features
postgres`". When only the first argument is an official name, the call
is 5b3's: that name becomes its crate and `--rename`, and the rest goes
to cargo as written (`varyk add sql --features postgres`). A shorthand
with `--rename` is refused, "write the full name: `varyk add varyk-http
--rename web`", since `http` and `sql` are also names of unrelated
crates. When the arguments carry `--rename` after a full name, or the
first argument is not an official name, the whole call passes through to
cargo as today.

### 2.10 Not in milestone 5b4

A wrapping hook; route groups as values; a handler reading a header
without naming `http::Request`; a body bound by content type (XML, forms);
per-request headers on the client beyond its defaults; a route or hook
call on an app received from elsewhere; a route whose handler is a method or an imported Rust
function, or a function of another package; `serve` taking an address
(the address is a settings call, section 6.3);
TLS in the server (the proxy's job, said in the package's README);
streaming request and response bodies; OpenAPI generation; and date,
time, UUID, and bytes types (section 10).

## 3. Safety

- **A route is checked before anything runs.** Every binding problem is a
  `varyk check` diagnostic at the route call, so a renamed path
  parameter cannot silently stop binding. A query parameter's name is
  its key in the query string, so renaming one changes the API. One case is rustc's alone, as for a
  started call: a handler that holds a value of a `.rs` module that
  cannot go to another thread across an `.await`. The driver reports it
  as V0901 at the route call, and a state type that cannot be shared
  between threads as V0901 at `App::new` (section 7.5).
- **Input is parsed by the package, not the handler.** A path segment,
  query value, or body that does not fit its type is a 400 built by the
  package, with the handler never called, so a handler's parameters are
  always values of their types.
- **No internal message reaches a client.** An error without a status is
  a 500 with a fixed body and a log line (section 2.6); a panic in a
  handler is the same, contained by the package. A database message, a
  file path, or a URL in an `Error` therefore stays in the log.
- **Nothing outlives a request.** The adapter owns what it parsed and
  lends it to the handler for the call; the state is an `Arc` handle.
  The ownership rules of M1 to M5b1 hold unchanged.
- **The generated Rust is readable and names only the package.** One
  adapter per route, a few lines each, calling `::http::` items
  (section 4); axum is never written by the compiler.
- **Safe defaults are the package's** (section 6.3): limits, timeouts,
  and panic containment on without being asked; CORS off until asked.

## 4. Generated Rust

For each route the backend writes one adapter function in the module of
the function that holds the route call, with concrete types, and one
call on the app. Adapters, of routes and hooks alike, share one counter
per module, `varyk_route_0`,
`varyk_route_1`, in source order; the `varyk_` prefix is the
compiler's (V0113), so no name of the program can clash, and the same
handler on two routes gets two adapters. Inside an adapter the incoming
request is `varyk_req`, an `after` adapter's response `varyk_res`, and
the bound values `varyk_0`, `varyk_1`, in the handler's parameter
order, as a started call names its arguments (M5b1 §5), so a handler
parameter called `req` changes nothing. For
`app.get("/users/{id}", get_user)` with `get_user(id: i64, state:
Shared<State>)` returning `Result<Option<User>, Error>`:

```rust
async fn varyk_route_0(varyk_req: ::http::Request) -> ::http::Response {
    let varyk_0 = match varyk_req.param::<i64>("id") {
        Ok(v) => v,
        Err(r) => return r,
    };
    let varyk_1 = match varyk_req.state::<State>() {
        Ok(v) => v,
        Err(r) => return r,
    };
    match get_user(varyk_0, &varyk_1).await {
        Ok(v) => ::http::respond_option(v),
        Err(e) => ::http::respond_error(e),
    }
}

app.get("/users/{id}", ::http::route(varyk_route_0));
```

`::http::` stands for the path to the package from the crate that holds
the route call, through that crate's own key. The pieces, each a
function or method of the package's contract (section 6.1):

- `varyk_req.param::<P>("name")`, `.query::<Q>("name")`, `.json::<B>("name")`
  (awaited), `.state::<S>()`, and `.bind::<K>()` (awaited), each a
  `&self` method, so `varyk_req` needs no `mut` and stays usable after any of
  them, each giving `Result<value, Response>`, the `Err` being the 400
  or 500 the package built, returned at once as above; `state` gives an
  `Arc<S>`; the compiler writes the turbofish from the parameter's type,
  through `rust_type`, so rustc infers nothing;
- the bindings in two groups: the path, query, body, and state values
  first, then the values the package binds by type (`bind::<K>()`),
  so a parameter that fails with a 400 does so before `bind` can send a
  live handler's early response (a WebSocket's upgrade);
- the handler call, with the parameters in the handler's order, each
  lent as its mode says (section 2.2): `varyk_0` by value for the `i64`,
  `&varyk_1` for the state, and `&mut varyk_N` from a `let mut` for a
  `mut` parameter;
- one response call per return shape of section 2.3: `respond_empty()`,
  `respond_json(&v)`, `respond_option(v)` (which is `respond_json` or the
  404), `v` itself for an `http::Response`, and for a `Result` the
  `match` above with `respond_error(e)`;
- `::http::route(adapter)` wraps the adapter for `App::get` and the
  others. A `before` adapter takes `(varyk_req: Request)`, lends
  `&varyk_req` to the hook as its request, and gives `Option<Response>`: `None` lets the request through, `Some` stops it,
  built by `::http::respond_before(result)` from the hook's
  `Result<bool, Error>` (`Ok(false)` the package's 403, `Err` as
  `respond_error`); it is wrapped by `::http::before_hook(adapter)`. An
  `after` adapter takes `(varyk_req: Request, varyk_res: Response)`,
  binds `varyk_res` with `let mut`, calls the hook with `(&varyk_req,
  &mut varyk_res)`, and gives `varyk_res`; it is wrapped by
  `::http::after_hook(adapter)`. Both bind the state with `.state()` as
  a route does, a `before` adapter returning `Some(r)` where a route
  returns `r`;
- `::http::App::new(state)` with the `Arc` the `Shared` already is (M5b1
  §5); `serve`, `request`, and the settings as the imported methods they
  are, written as every imported method is (M3 §4.2).

A query parameter of type `Option<i64>` is `varyk_req.query::<Option<i64>>`.
The body call `varyk_req.json::<NewUser>("user").await`, named after
the handler's parameter so the 400 can name it, needs `NewUser`'s
`Deserialize` derive, written by the reach analysis as for `json::parse`;
`respond_json(&user)` needs `Serialize` likewise. Hooks and routes are
registered in source order, which fixes the order hooks run in (section
2.4), not which routes they cover.

## 5. `varyk-std`

`Error` gains a field `status: Option<u16>`, a constructor
`Error::with_status(status: u16, message: String) -> Error`, and
`status(&self) -> Option<u16>`; `new` leaves it `None`; `Clone` and
`PartialEq` include it, and `Display` stays the message alone. The Rust
documentation of `with_status` says that a status from 400 to 599 means
the message is sent to the client, and any other status is sent as a
500 (section 2.6). Nothing else changes; no new
dependency. (JSON log lines already exist, `LOG_FORMAT=json`, M5a §2.6.)

## 6. The `varyk-http` contract

What the compiler writes and checks, and so what `varyk-http` must
provide at the versions the version table in its README names. The
package's own spec owns everything else.

### 6.1 Items the compiler names

Named at the package's root module (`src/lib.vr`), declared there or
brought there by `pub use` (M5b3 §2.5), and reached as `::http::` in the
generated Rust through the program's key: `App` with `new(Arc<S>)`, `get`, `post`,
`put`, `patch`, `delete`, `before`, `before_on`, `after`, `serve`, and
`request`, the route and hook methods taking `&mut self`, and
`pub async fn serve(&self, port: u16) -> Result<bool, varyk_std::Error>`
and `pub async fn request(&self, req: Request) -> Response` with those
receivers (the importer maps only `&self` and `&mut self`); `Request` with
the `&self` methods `param`, `query`, `json`, `state`, and `bind`;
`Response`; `route`, `before_hook`, `after_hook`; `respond_empty`,
`respond_json`, `respond_option`, `respond_error`, `respond_before`.
`param` and `query` accept every type section 2.2 binds there, and
`bind` every struct named at the root but `App`, `Response`, and
`Client`; how the package
bounds them (a trait of its own) is its business, since the generated
code never names the bound. The signatures the importer cannot call (generic ones)
are imported as uncallable, as any such Rust is (V0108), and a program
that names one gets that diagnostic; the compiler calls them by
generation only.

`Request` is `Clone`, its body held in a shared buffer so a copy is
cheap: every hook adapter and the route adapter take the request by
value, and `bind::<Request>()` gives such a copy.
`App`, `Request`, and `Response` are `Send + Sync`, since every adapter
holds `&varyk_req` across an `.await` and axum needs its futures to be
`Send`; a `Request` therefore keeps its body where a `&self` method can
read it (read whole under the body limit before the adapter runs,
except a `multipart/form-data` body, which the package keeps as a stream
for `bind` to take).
Conflicting routes are an `Err` from `serve` and a 500 with the conflict
logged from `request`, never a panic. A `before_on` prefix is tested
against the matched route's path pattern, not the request's path, so a
request whose route lies under the prefix always runs the hook whatever
its spelling; a hook that could be skipped by
spelling a path differently would be an auth bypass. For the same
reason every hook applies to every route of the app, whatever their order
of registration; registration order fixes only the order hooks run in.
axum's `layer` covers only routes already added, so the package applies
the hooks when it builds the router in `serve` and `request`.

The compiler checks, when it marks the route table, that `App`,
`Request`, and `Response` are structs named at the package's root; a
missing one is V0407, "this `varyk-http` does not match this `varyk`",
with the two versions and a pointer to the version table in
`varyk-http`'s README. Anything finer
is rustc's, reported as V0900 against the generated adapter.

### 6.2 `Request` and `Response` in Varyk

Imported through the ordinary rules (M3 §4), so the package may add to
them without a compiler change:

- `http::Request::new(method, path)`; getters `method()`, `path()`,
  `header(name)` and `cookie(name)` giving `Option<string>`, `body()`
  giving `string`; setters `set_header(name, value)` and `set_body(text)`
  as `mut self` methods, for `app.request`.
- `http::Response::json(value)` (the shape of section 2.7), `text(s)`,
  `empty()` (a 204), `file(dir, name)`, which sends the file `name` from
  the folder `dir` the program names, and refuses a `name` that is
  absolute, has a `..` segment, has a segment starting with `.`, or
  resolves through a link outside `dir`, as a 404 with the reason
  logged, so no input can reach `.env`, the database, or the source;
  setters `set_status(code)`, `set_header(name,
  value)`, `set_cookie(name, value, max_age)` (HttpOnly, Secure, and
  SameSite=Lax set; `max_age` in seconds); getters `status()`,
  `header(name)`, `body()` giving the text, and `read_json()` filled from
  where the result goes (M5b3 §2.1). One type for what a handler builds, what
  `app.request` gives, and what the client receives. A header name or
  value that is not valid HTTP cannot be refused by a setter without a
  `Result` on every call, so it is a 500 at send time with the message
  logged.

Getters, setters, and constructors have different names (`status()`
and `set_status`, `text(s)` and `body()`, `json(value)` and
`read_json()`) because a Rust struct cannot have two methods of one
name.

### 6.3 Defaults and settings

On by default: request tracing through `tracing` (one line per request
with method, path, status, and time), a request body limit, a request
timeout, panic containment as a 500, and graceful shutdown on SIGTERM
and ctrl-c that finishes in-flight requests within a bound. Each of these
bounds one request; nothing limits the service as a whole by default,
since memory is the platform's to manage. By one call
on `app` before `serve`: CORS, response compression (off by default,
since compressing a secret beside text an attacker chose leaks it
through the length), `metrics(path)` for Prometheus request
counts and latency, the bind address, each limit, and a maximum of
requests in flight with 503 beyond it, for a service that wants to shed
load. The server listens
on `127.0.0.1` unless the program sets an address, so a service run on a
laptop is not open to its network; a container sets `0.0.0.0` with that
call, and the package's README shows it beside its `Dockerfile` lines. Rate limiting per
client, TLS, and security headers are the proxy's, said in the README.

### 6.4 Client

`http::Client::new()`, with `set_header(name, value)` and `set_timeout(ms)`
as `mut self` methods, and `get(url)`, `delete(url)`, `post(url, body)`,
`put(url, body)`, `patch(url, body)`, each async and giving
`Result<Response, Error>`; `body` through the `&T: Serialize` shape, sent
as JSON. A response of any status is a `Response`, so the caller reads
`status()` and may read the error body; only a failure to get a response,
a bad URL, a connection or TLS failure, the timeout, or a body over the
size limit, is an `Error`
(with no status). rustls with bundled roots, a 30 second default timeout,
redirects followed only to the same scheme, host, and port, so a default
header such as an API key never goes to a host the program did not
name (any other redirect is returned as its 3xx `Response`, whose
`location` header the caller may read), and the body read into memory under a
size limit.

### 6.5 The package's checklist

Inside milestone 5b4, each item checked at the `varyk-http` release that
ships it: the core of sections 6.1 to 6.4 and the `users` demo under
`demo/users` with its README being the fifteen-minute path; then
WebSockets (`WebSocket` bound by type, `send(text)` and `recv()`),
server-sent events (`Sse`, `send(text)`), and multipart uploads
(`Multipart`, each part's `text()` and `save_to(dir, name)`, with the
rule of `Response::file`), each a bound type of section 2.2 needing no
compiler change, text and files until a bytes type exists. A WebSocket
exists only after the 101 response is sent, and an event stream's
response must be sent before the handler sends its first event, so for
`WebSocket` and `Sse` the package's `route` runs the adapter as a task
of its own and answers first; the handler's return value is then
ignored (an `Err` logged), which the package's spec says, and a handler
taking one of them returns nothing, or `Result<http::Response, Error>`
so that `?` works on its calls.

## 7. Compiler changes

Line numbers are approximate.

### 7.1 Interop (`crates/varyk/src/interop/`)

- `items.rs` (`type_param` ~489, the one-bound check ~519): a type
  parameter bounded by `Serialize + ?Sized`, used once as `&T` in a parameter, recorded on the
  signature beside the 5b3 `DeserializeOwned` one; both at once, or any
  other shape, refused with a reason for the V0108 note.
- `signatures.rs` (`map_reference` ~365, `map_value` ~402): `&T` with the
  recorded parameter as `RustTy::SerializeParam`. A bare
  `varyk_std::Error` is already mapped to `RustTy::Error` here (~430);
  only resolve refuses it as a whole return (section 7.2).

### 7.2 Resolve

- `resolve/signatures.rs` (`param` ~109, `ret` ~120): `SerializeParam`
  becomes a parameter with a write-type hole, borrowed; `ret` stops
  refusing a bare `Error` return (~146).
- `resolve/mod.rs` and `packages.rs` (~89, ~238): a package whose crate
  name is `varyk-http` found in the graph, and its `App`, `Request`, and
  `Response` looked up at its root (V0407 when missing). A path marks the
  route table when it resolves to that `App` struct, whatever key it was
  written through; the adapter writes the path of the package holding the
  route call to it, as every path to another package is written (M5b2).
  Two versions of `varyk-http` in one build each have their own `App`.
  When the package being checked is named `varyk-http`, its own root
  items are looked up and marked the same way, its route calls writing
  the package's own crate path.

### 7.3 Types

- The handler argument of a route or hook call, once the receiver is
  known to be a marked app, resolved as a function path (not a value)
  before `path_value` (`types/check/values.rs` ~36, called from
  `check.rs` ~893) would report V0100 for the bare
  name, and recorded on the call; a path that is not a function is
  V0220.
- `types/check/values.rs` (`path_owner` ~104, `convert_call` ~517,
  `read_type` ~619): a new `Owner::App` for the marked struct, with the
  intrinsic table of section 2.1; `App::new` typed as `Shared::new` is
  (~446) and the state type recorded on the app's local; the route call
  check of section 2.2 against the handler's signature (V0219, V0222),
  the return check of section 2.3 (V0220), and hooks (section 2.4); body
  and return types added to the read and write reach sets; a
  `SerializeParam` argument checked as a `json::stringify` argument.
- `types/check.rs` (~49-96, `uses_std` ~214) and the `logs` flag
  (`values.rs` ~500): both set by an `App::new` call.
- `builtins.rs` (~631): a row for `status` on `Error`, giving
  `Option<u16>`, beside `message`; and `Error::with_status(status,
  text)` beside `Error::new`, its text an owned slot as `new`'s is,
  written as `::varyk_std::Error::with_status`.
- The rule of section 2.1 (V0221): the local each `App::new` is bound
  to, recorded per function, and route and hook calls checked against
  it.
- HIR: a route registration node (method, path segments, handler `FnId`,
  each parameter's binding kind, the return shape, the state type), a
  hook node, and an `App::new` node.

### 7.4 Borrow analysis (`crates/varyk/src/borrow.rs`)

- A route or hook node is a use of the app's local as a `mut self`
  receiver (V0302 when it is not `let mut`); the handler path is not a
  value and moves or borrows nothing; `App::new`'s argument is an owned
  slot, with V0304 and V0305 as for `Shared::new`.

### 7.5 Backend (`crates/varyk/src/backend/rust_expr.rs`, `rust.rs`)

- The adapter of section 4 per route and hook node, written into the
  module's generated file after its functions, every line of it
  carrying the span of its route or hook call in the source map, so a
  rustc error inside one is reported at that call (V0900, or V0901);
  the registration call;
  `App::new` with the `Arc` argument; `request` and `serve` as imported
  methods (`callee` ~1266, `args` ~850); `::varyk_std::start()` in
  `main` when the `logs` flag is set (`rust.rs` ~535), as today, and
  now also first inside the `run` block of an async `#[test]`, so a 500
  in a test sent through `app.request` logs its message (`start` uses
  `try_init`, so many tests may call it).
- A `SerializeParam` argument passed by reference as a `json::stringify`
  argument is, with no turbofish: rustc infers `T` from the argument
  (`str` for a string lent as `&str`), so the 5b3 `type_arg` path is not
  used for it.
- `driver/messages.rs` (`thread_safety` ~257): it already maps a
  "cannot be sent" or "cannot be shared" error at a mapped span to
  V0901; its message gains a wording for a route or hook call ("the
  handler or hook added here holds ..") and for `App::new` ("the state given
  here holds .."), where a state type that is not `Sync` fails. A route
  in a dependency Varyk package stays V0900 for now, as a started call
  there does.

### 7.6 Command line

- `cli.rs` (`add_args` ~805, `run_add` ~825): the second row, the full
  names, several runs, and the refusals of section 2.9; `add_args` gives
  a list of argument lists.

## 8. Diagnostics

New codes, each with a fixture, a registration in `tests/errors.rs`, and a
row in `docs/language.md`:

| Code | Meaning |
|---|---|
| V0219 | a route's path and its handler do not fit: a segment with no parameter, a parameter that binds to nothing, a body where none can be, two of one kind, a parameter of the wrong type |
| V0220 | a handler or hook of the wrong shape: not an async function of the current package, a return type outside the list, a hook signature outside the list |
| V0221 | a route or hook call on an app that is not a local bound to `App::new` in this function, a route or hook call in a loop, or an assignment to that local |
| V0222 | a route path or `before_on` prefix that is not valid, or a route already taken |
| V0407 | the `varyk-http` package in the build does not match this compiler |

Reused: V0105, V0106, and V0114 for a handler or hook that is not
visible from the route call, is `main`, or is a `#[test]` function, each
row in `docs/language.md` gaining "or named as a route handler or
hook", and the rows and `codes.rs` comments of V0210 (a route's body or
return type, a `Serialize` argument), V0217 (a route path or
`before_on` prefix), and V0901 (a route or hook call, `App::new`, and
the `driver/messages.rs` comment that `Task::start` is the only `Send`
bound) gaining the new case; V0108, with a new note, for a `Serialize` parameter or a bare
`varyk_std::Error` where section 2.7 does not admit it; V0209 and V0210
for body, return, and `Serialize` argument types; V0217 for a route path
or prefix that is not a literal; V0200 for an `App::new` argument that is
not a `Shared`; V0302 for a route or hook call on an app that is not
`let mut`; V0404 for a program with routes and no `varyk-std`; V0901 for
a handler that holds a value that cannot go to another thread.

## 9. Testing and definition of done

- Interop unit tests: the `Serialize` parameter accepted, and each refused
  shape with its note; the bare `Error` return.
- Type tests: each binding kind of section 2.2 accepted; each V0219 case;
  each V0220 case for handlers and for the three hooks; each V0221 and
  V0222 case; the reach sets gaining a body and a return type; `uses_std`
  set by `App::new`.
- `insta` snapshots of the generated Rust for a route with a path
  parameter, with a query parameter, with a body, with a `mut` body,
  with a package type, with each return shape, for `before`,
  `before_on`, and `after`, for one handler on two routes, for
  `App::new`, and of a `main` and an async `#[test]` with routes and no
  `log` call, each of which starts logging.
- Soundness templates (M2 §7): a `Serialize` argument still usable after
  the call; the state handle usable after a route call.
- A fixture package `crates/varyk/tests/fixtures/packages/varyk-http/`
  whose crate name is `varyk-http`: a stub facade with the items of
  section 6.1 and 6.2 over an in-memory router with no network and no
  axum (its `serve` gives `Ok(true)` at once), enough for `varyk check`,
  the snapshots, and `varyk run` of a program fixture beside it,
  `fixtures/packages/http_user/`, that depends on it by path, serves a
  fixed list of users from a `Shared` state, registers routes and hooks (one `before` after the route it must
cover, checked through `app.request`),
  sends requests through
  `app.request`, and prints the expected output; a `#[test]` in it runs
  under `varyk test`. Both run in `tests/examples.rs` from a copy made
  together, as `store` and `store_user` do (M5b3 §8), and each manifest's
  `varyk-std` line gets an `extra-files` entry in
  `release-please-config.json`.
- `varyk-std` tests: `with_status`, `status`, and `Display`, `Clone`, and
  `PartialEq` with a status.
- Type tests and a codegen snapshot for `Error::with_status` from Varyk
  code; a test that a package named `varyk-http` marks its own `App`, so
  the stub fixture's `src/tests.vr` adds a route and sends a request
  through `app.request`.
- Unit tests of `add_args`: `http sql`, `varyk-http varyk-sql`, `sql
  --features postgres` and `varyk-sql --features postgres`, the refusal
  of `http sql --features postgres` and of `http sql serde`, `sql serde`
  kept as 5b3 has it, the refusal of `http --rename web`, and
`varyk-http --rename web` passing through;
  `tests/cli.rs` runs a refusal end to end.
- The language-reference test (M4 §7) covers the new codes.
- `docs/language.md` (a new "HTTP" section before "Calling Rust"; the two
  shapes in the "Calling Rust" table; "Not in milestone 5b3" renamed and
  updated; `varyk add` with several packages), `docs/design.md`,
  `docs/open-questions.md`, `docs/roadmap.md` (section 10), `README.md`,
  and AGENTS.md (the compiler knows `varyk-http` by name) updated; the
  site's hero changed to handle `serve`'s `Result` and to use
  `set_status`.
- With the 2026-10-06 amendments: the stub fixture's constructors move
  to its `src/lib.vr` over `Error::with_status`, and its `respond_error`
  sends a status only from 400 to 599 and its `before_on` tests the
  matched route's pattern, and it runs no `before` hook for a request no
  route matches (`http_user`'s `GET /nowhere` line becomes a 404 with
  the `after` hook's header); the hero (section 1), the HTTP example in
  `docs/language.md`, and the site's hero drop their own "listening"
  log line, since `serve` logs one after it binds; `docs/language.md`'s
  HTTP table rows for `before` and `after` and its paragraph on `after`
  hooks say what section 2.1 says: a `before` hook runs for every request
  a route matches, and an `after` hook on every response the router
  makes; route and hook adapters bind the package's
  types after the other parameters (section 4); `docs/language.md` (the
  `Error` table, the status range, `before_on`, the in-flight limit, and
  the two places that call the bare `varyk_std::Error` return the
  constructors' mechanism) and `docs/roadmap.md` (the `varyk-http` list
  without a default in-flight limit) say the same.
- CI green, including the 1.85 build; no `unwrap`, `expect`, or other
  crash-on-absence call added.

Done when every item of the roadmap's 5b4 compiler list is checked;
`varyk-http`'s items are checked at its releases, and milestone 5's bar
waits for them and for the types slice of section 10.

## 10. Roadmap changes

The 5b4 entry becomes two lists under one heading.

The compiler:

- a route table: `http::App` of the `varyk-http` package known to the
  compiler; `get`, `post`, `put`, `patch`, and `delete` routes with
  `{name}` segments; handlers' parameters bound by name to the path and
  the query string, by type to the JSON body, to the state (a
  `Shared<T>`), and to the package's own types; the return value as the
  response, `None` as 404, `http::Response` for a custom status and
  headers; every route checked against its handler by `varyk check`;
- `before`, `before_on`, and `after` hooks, and `app.request` for tests;
- `Error` carrying an optional status, set by `Error::with_status` and
  the `http::` constructors written over it; an error without one is a
  500 whose message is logged and not sent;
- a `.rs` function with a type parameter `&T: Serialize` in a parameter
  (moved from 5b3), and a `.rs` function returning `varyk_std::Error`;
- `varyk add http sql`, several official packages in one call, by
  shorthand or full name;
- the fixture packages, and the docs.

`varyk-http`, in its own repository, each item at the release that
ships it:

- the server on axum and tower-http: the contract above, `Request` and
  `Response`, tracing, body limit, timeout, panic containment, and
  graceful shutdown on by default; CORS, compression, metrics, address
  (`127.0.0.1` unless set), the limits, and a maximum of requests in
  flight by one call; cookies with secure
  defaults; `Response::file`;
- the client on reqwest with rustls: `Client` with default headers and a
  timeout, `get`, `post`, `put`, `patch`, `delete`, a `Response` for any
  status;
- the `users` API on `varyk-sql` as its demo, and the fifteen-minute
  path in its README: `varyk init`, `varyk add http sql`, one file, and
  `varyk run`;
- WebSockets, server-sent events, and multipart uploads as bound types,
  text and files until a bytes type exists.

The milestone-5 introduction becomes six milestones, its bar met when
5b4 and 5c are done. Added before that bar, as its own spec after this
one:

**Milestone 5c: time, ids, and bytes.** A date-time type with `now()`,
ISO 8601 in JSON and native columns in `varyk-sql`; a UUID type; a bytes
type, which lets `varyk-http` read uploads and binary bodies. Milestone 5
is met when 5b4 and 5c are done.

The agent evaluation stays a line under 5b4, its own work in its own
repository. "`varyk-mongo` and `varyk-redis`" stay the next packages
after it. From "Unscheduled", nothing moves; JWT verification and password
hashing are noted there as facades over `jsonwebtoken` and `argon2`, to
become packages if people keep writing the same facade.

## 11. Open questions

Added to `docs/open-questions.md`:

- Should there be a wrapping hook, `app.wrap(f)` with `async fn f(req:
  http::Request, next: http::Next) -> http::Response`? `Next` would be a
  facade struct with one async method, so the cost is small; 5b4 keeps
  `before` and `after` until a need shows up that they cannot meet.
- Should routes be grouped as values (`let admin = app.group("/admin")`),
  or does `before_on` cover what groups are for?
- Should a handler be able to bind a header or a cookie by name, as a
  path parameter binds, without naming `http::Request`?
- Should a body bind by content type (XML, form data), with `xml::parse`
  and `xml::stringify` beside `json`? XML goes through `req.body()` and a
  facade until then.
- Should the client take per-request headers, which needs a request
  builder, or do default headers on `Client` cover a service's needs?
- Should the compiler write an OpenAPI document from the route table and
  the types it binds (`varyk openapi`)? It knows everything the document
  needs.
- Should routes be added to an app a function received, which needs the
  state type to travel with it (`http::App<State>` in the surface)?
- Should the compiler know any package but `varyk-std` and `varyk-http`
  by name, and should a community package be able to offer route binding?
  Function values, if they ever come, would answer the second.

Partly answered, each entry in `docs/open-questions.md` rewritten so its
answered half names 5b4 and the rest stays open: "Should the facade's
type parameter be allowed in parameter position (`&T: Serialize`), and
in other return shapes": the first yes (section 2.7), other return shapes
still open; "Should `varyk_std::Error` be accepted in parameters and
fields, and should a facade be able to carry a status or kind on it":
a status yes (section 2.6), parameters, fields, and a kind still open.

## 12. Decisions

Taken with the user on 2026-10-05:

- `varyk-http` is built on axum with tower-http, and its client on reqwest
  with rustls; no hand-written server. The reasons and the benchmark
  figures are in section 1.1. The tokio organisation maintaining axum was
  named as a reason of its own: Varyk's runtime is already a bet on
  tokio.
- The compiler knows `varyk-http` by crate name and generates one
  adapter per route; the alternatives are in section 1.1.
- A handler may receive the query string by name, the body by type, the
  state by type, and `http::Request` by type; headers are read through
  `http::Request` and set through `http::Response`, which also carries a
  custom status.
- `before` and `before_on` accept or reject, and `after` changes the
  response (approved with the surface of section 2.1); a handler that
  needs the authenticated user calls a function of the program.
- `app.request` for in-process tests is in.
- No follow-up milestone for WebSockets, server-sent events, and
  multipart: they are `varyk-http`'s own items inside 5b4, made possible
  by the one binding rule for the package's types.
- Metrics and panic containment are `varyk-http`'s, and a limit on
  requests in flight is one by name, off by default (decided 2026-10-06:
  memory is the platform's to manage);
  JWT and password hashing are facades; OpenAPI is an open question; date,
  time, UUID, and bytes are milestone 5c before milestone 5's bar.
- `varyk add varyk-http varyk-sql` works as `varyk add http sql` does.

Taken with the user on 2026-10-06, while designing `varyk-http`'s own
spec:

- `Error::with_status(status, text)` is a Varyk call, so `varyk-http`'s
  constructors are Varyk functions and as much of the package as the
  language allows is Varyk.
- The compiler marks the `App` of the package being checked when that
  package is `varyk-http`, so the package's own tests are Varyk.
- An error is sent with its status only for a status from 400 to 599;
  any other is a 500, so a 2xx or 3xx can never carry an error's text.
