# Varyk

Varyk is an experimental programming language for backend services, with
Rust-like safety and Go-like simplicity, that compiles to Rust, created by
[Vlad Mickevic](https://github.com/vlamic).

It is a small language for APIs, workers, and command-line tools: you write
Go-like application code and ship a native binary with no garbage collector
and the Rust ecosystem behind it. The service batteries (HTTP, JSON,
databases, `async`) are milestone 5 on the [roadmap](docs/roadmap.md).

Varyk exists to make Rust available to everyone.

Rust is one of the safest and fastest languages there are, and one of the
hardest to learn. Its guarantees belong in every program, but its complexity
keeps most people out, whether they come from another language or are
writing their first program. Varyk keeps what makes Rust strong: memory
safety without a garbage collector, native speed, and the Rust ecosystem. It
removes the complexity that stands between developers and those benefits:
first developers building services in Go, TypeScript, or Python, then
developers coming from Rust, and AI agents writing code.

Varyk compiles to Rust and runs on the Rust ecosystem, the way TypeScript
compiles to JavaScript, though Varyk is not a superset of Rust. The Rust
compiler checks everything Varyk generates, so the guarantees are Rust's
own. Varyk code lives in `.vr` files, Rust code can live in `.rs` files next
to them, and the two build together.

Varyk is for the application. Kernels, database engines, custom allocators,
and borrow-heavy libraries stay in Rust, in a `.rs` file beside your Varyk,
in the same build.

## An example

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

The generated Rust is checked by the Rust compiler like any other Rust code.
Varyk keeps Rust's safety model and adds no garbage collector.

## Packages and crates

A Varyk package is a Cargo package: a `Cargo.toml` and a `src/main.vr` or
`src/lib.vr`. `varyk init` writes one that both `varyk run` and plain
`cargo build` compile, and `varyk publish` ships it to crates.io as a plain
Rust crate that needs no Varyk to use.

`cargo add` works unchanged. Varyk code reaches a crate through a facade, a
`.rs` file in the package that wraps it: the facade wraps what the program
needs in plain functions, structs, and enums, which Varyk imports like its
own, and Varyk code never names a crate itself. `examples/packages/matcher`
wraps `regex-lite` this way:

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
minor version. This is milestone 3: packages and interop. Structs, enums,
and `match`, `for` loops, methods, `Option`, `Result`, `Vec`, `?`, and
`format!` work, and so do packages with dependencies, modules at any depth,
`use`, private fields, and Rust structs and enums imported from `.rs` files;
the compiler builds and runs every program in `examples/`, and it reports
every error it knows about with a code, a plain-word message, and, where it
can, a suggested fix. Closures, iterators, and much more are not there yet;
see [docs/language.md](docs/language.md) for exactly what works.

## Documents

- [varyk.com](https://varyk.com): the project site.
- [docs/language.md](docs/language.md): the language reference, one page.
- [docs/design.md](docs/design.md): the principles and the decisions behind them.
- [docs/open-questions.md](docs/open-questions.md): questions not yet answered.
- [docs/roadmap.md](docs/roadmap.md): what comes next.
- [docs/specs/2026-09-23-varyk-design.md](docs/specs/2026-09-23-varyk-design.md): the full design.
- [docs/specs/2026-09-25-milestone-2-design.md](docs/specs/2026-09-25-milestone-2-design.md): milestone 2's additions.
- [docs/specs/2026-09-26-milestone-3-design.md](docs/specs/2026-09-26-milestone-3-design.md): milestone 3's additions.

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
