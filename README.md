# Varyk

Build backend services simply. Ship Rust binaries.

Varyk is a small language for APIs, workers, and microservices, created by
[Vlad Mickevic](https://github.com/vlamic). It removes Rust's ownership
ceremony and keeps Rust's safety, speed, ecosystem, and deployment model:
you write Go-like application code, the compiler turns it into readable
Rust, rustc checks it, and you ship one native binary. Rust's safety, Go's
simplicity.

- No garbage collector, and no runtime beyond Rust's own.
- No lifetime annotations, no `&` or `&mut` to choose at a call site, and
  one string type.
- One native binary to deploy, with no language runtime to install.
- Cargo and crates.io underneath: a Varyk package is a Cargo package.
- Readable generated Rust, yours to keep.
- Drop into Rust whenever you need it, in a `.rs` file beside your Varyk,
  in the same build.

```sh
cargo install varyk
```

JSON, configuration from the environment, logging, and `varyk test` are in
milestone 5a, and `async` functions and tasks in 5b1; HTTP and databases
are milestone 5b2 on the [roadmap](docs/roadmap.md); what works today is under
[Status](#status).

## Use Varyk until you need Rust

A Varyk package is a Cargo package: a `Cargo.toml` and a `src/main.vr` or
`src/lib.vr`. `varyk init` writes one that both `varyk run` and plain
`cargo build` compile, and `varyk publish` ships it to crates.io as a plain
Rust crate that needs no Varyk to use. There is nothing to bootstrap: every
crate on crates.io is available from the first day, and every Varyk package
joins them.

| Layer | File | |
|---|---|---|
| Your service | `src/main.vr` | Varyk. It never names a crate, and it never writes a reference. |
| Your facade | `src/text.rs` | Rust. It wraps what the program needs in plain functions, structs, and enums, which Varyk imports like a module of its own. |
| Any crate | crates.io | Added with `cargo add`. Need Stripe, AWS, or Kafka? Use the Rust crate. |

`examples/packages/matcher` wraps `regex-lite` this way:

```rust
// src/text.rs
pub struct Matcher {
    re: regex_lite::Regex,
}

impl Matcher {
    pub fn new(pattern: &str) -> Matcher { /* ... */ }
    pub fn is_match(&self, s: &str) -> bool { /* ... */ }
    pub fn count(&self, s: &str) -> usize { /* ... */ }
}
```

```varyk
// src/main.vr
mod text;

fn main() {
    let digits = text::Matcher::new("[0-9]+");
    println!("{}", digits.count("route 66 or 101"));
}
```

Need something Varyk doesn't provide? Use the Rust crate. Need Rust itself,
for the one hot loop or the borrow-heavy library? Write it in the `.rs`
file and call it from Varyk. Varyk is for the application: kernels,
database engines, custom allocators, and borrow-heavy libraries stay in
Rust, in the same build.

## How it works

Varyk compiles to Rust and runs on the Rust ecosystem, the way TypeScript
compiles to JavaScript, though Varyk is not a superset of Rust. Most of
Rust's surface is the spelling of decisions the compiler can make on its
own: which `&` to write, which string type, which lifetime. Varyk keeps
Rust's ownership model inside the compiler and takes the spelling out of
the language. The Rust compiler checks everything Varyk generates, so the
guarantees are Rust's own.

`examples/borrowing.vr`:

```varyk
struct User {
    name: string,
}

fn rename(mut user: User) {
    user.name = "Bob";
}

fn print_user(user: User) {
    println!("{}", user.name);
}

fn main() {
    let mut user = User {
        name: "Alice",
    };

    print_user(user);
    rename(user);
    print_user(user);
}
```

It prints `Alice`, then `Bob`. This is the generated `src/main.rs` for it,
as the compiler writes it; `varyk build --emit-rust examples/borrowing.vr`
prints the same file, additionally passed through rustfmt when rustfmt is
installed:

<!-- emit-rust: examples/borrowing.vr -->
```rust
#[allow(warnings, arithmetic_overflow, unconditional_panic)]
#[derive(Clone, PartialEq)]
struct User {
    name: String,
}

#[allow(warnings, arithmetic_overflow, unconditional_panic)]
fn rename(user: &mut User) {
    user.name = "Bob".to_string();
}

#[allow(warnings, arithmetic_overflow, unconditional_panic)]
fn print_user(user: &User) {
    ::std::println!("{}", user.name);
}

#[allow(warnings, arithmetic_overflow, unconditional_panic)]
fn main() {
    let mut user = User { name: "Alice".to_string() };
    print_user(&user);
    rename(&mut user);
    print_user(&user);
}
```

`varyk build` writes that Cargo project under `target/varyk/` and hands it
to cargo; rustc and the full borrow checker check it like any other Rust
code, and Varyk never uses `unsafe` to get around them. The project is
readable, and it is yours.

## Who it is for

Varyk exists so that ordinary backend services can be written simply and
shipped as safe Rust. First, developers building services in Go,
TypeScript, or Python, who want Go's simplicity and one-binary deployment
without the garbage collector, with rustc catching the bugs those languages
compile. Then developers coming from Rust, who want the same safety model
with less ceremony for application code. And teams that write code with
coding agents: agents already write good code, and the slow part is
reviewing it. A small, concrete language with one way to do each thing
keeps agent output reviewable, rustc checks its memory safety and data
races, and diagnostics with codes and fix-its come in machine-readable
form, so the agent fixes its own mistakes before a person looks.

## Try it

You need a stable Rust toolchain installed through
[rustup](https://rustup.rs). Varyk calls `cargo` to build your program.

Install the compiler from crates.io:

```sh
cargo install varyk
varyk run examples/hello.vr
```

Or build it from this repository and run the first example:

```sh
cargo build -p varyk
cargo run -p varyk -- run examples/hello.vr
```

The main commands:

```text
varyk check [file.vr]    check the program for errors; never runs cargo
varyk build [file.vr]    generate and build the Rust; prints the executable path
varyk run [file.vr]      build, then run the program, forwarding its exit code
varyk init [dir]         write a new package
varyk publish            publish the package to crates.io as a plain Rust crate
```

Without a file, a command works on the package that contains the current
directory (it searches upward for `Cargo.toml`).
`build` and `run` accept `--release`. `build` also accepts `--emit-rust`,
which prints the generated Rust. Every command accepts
`--message-format=json`, which prints errors as JSON, one object per line.
When running from this repository, put `cargo run -p varyk --` in front, as
in the example above.

## Status

Varyk is experimental and pre-1.0: anything may change, including any
syntax, error code, or command-line flag, and a breaking change bumps the
minor version. This is milestone 5b1: async functions and tasks, on top of milestone 5a's data, configuration, logging, and tests and milestone 4's closures, iterators, and patterns.
Structs, enums, and `match`, `for` loops, methods, `Option`, `Result`,
`Vec`, `?`, and `format!` work, and so do packages with dependencies,
modules at any depth, `use`, private fields, and Rust structs and enums
imported from `.rs` files; the compiler builds and runs every program in
`examples/`, and it reports every error it knows about with a code, a
plain-word message, and, where it can, a suggested fix. HTTP,
databases, and much more are not there yet; see
[docs/language.md](docs/language.md) for exactly what works.

Milestone 5b1 adds `async fn` and `.await`; a call without `.await` starts
a task, and `Task::all`, `Task::all_settled`, `.detach()`, `Shared<T>`, and
`time::sleep` wait for, share with, and time tasks, on a built-in
multi-threaded runtime. `Task`, `Shared`, and `time` are now reserved names,
a breaking change. See `examples/tasks.vr`, `fanout.vr`, and `shared.vr`.

Milestone 5a adds a built-in `Error`, so `parse` returns a `Result`;
attributes `#[rename]`, `#[default]`, `#[skip]`, and `#[test]`; `json::parse`
and `json::stringify`; `env::parse`, which fills a struct from the
environment and `.env`; `log::info` and its three siblings; and
`varyk test` and `varyk add`. The `varyk-std` crate behind them is a
dependency of every package. `parse` returning a `Result` is a breaking
change. The `users` package in `examples/packages/` shows them together.

Milestone 4 adds closures, as the argument of a call like `filter` or
`map`; chains such as `names.iter().filter(|n| n.len() > 3).count()`;
`HashMap`; and getters that return part of what they are given without
copying it. Nothing is written for that: Varyk sees that every return of
`display_name` is part of `self`, and the generated Rust returns a
reference into it, where milestone 3 needed a `.clone()`:

```varyk
fn display_name(self) -> string {
    if self.nickname.is_empty() { self.name } else { self.nickname }
}
```

```rust
fn display_name(&self) -> &str {
    if self.nickname.is_empty() {
        &self.name
    } else {
        &self.nickname
    }
}
```

## Documents

- [varyk.com](https://varyk.com): the project site.
- [docs/language.md](docs/language.md): the language reference, one page.
- [docs/design.md](docs/design.md): the principles and the decisions behind them.
- [docs/open-questions.md](docs/open-questions.md): questions not yet answered.
- [docs/roadmap.md](docs/roadmap.md): what comes next.
- [docs/specs/2026-09-23-varyk-design.md](docs/specs/2026-09-23-varyk-design.md): the full design.
- [docs/specs/2026-09-25-milestone-2-design.md](docs/specs/2026-09-25-milestone-2-design.md): milestone 2's additions.
- [docs/specs/2026-09-26-milestone-3-design.md](docs/specs/2026-09-26-milestone-3-design.md): milestone 3's additions.
- [docs/specs/2026-09-29-milestone-4-design.md](docs/specs/2026-09-29-milestone-4-design.md): milestone 4's additions.

## License

Licensed under either of [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE),
at your option.

Unless you explicitly state otherwise, any contribution intentionally
submitted for inclusion in the work by you, as defined in the Apache-2.0
license, shall be dual licensed as above, without any additional terms or
conditions.

## Trademark

The code is open source; the name and the logo are not. "Varyk" and the
Varyk logo (the two-gear mark) are trademarks of Vladislav Mickevic, and the
licenses above do not grant rights to them. You may use the name to refer to
the project, but a modified version or a fork needs a different name and its
own mark. See [TRADEMARKS.md](TRADEMARKS.md).
