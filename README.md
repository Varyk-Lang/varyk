# Varyk

Varyk is an experimental programming language with Rust-like safety and
Go-like simplicity, created by [Vlad Mickevic](https://github.com/vlamic).

Varyk exists to make Rust available to everyone.

Rust is one of the safest and fastest languages there is, and one of the
hardest to learn. Its guarantees belong in every program, but its complexity
keeps most people out. Varyk keeps what makes Rust strong: memory safety
without a garbage collector, native speed, and the Rust ecosystem. It removes
the complexity that stands between people and those benefits, whether they
come from another language, are writing their first program, or are an AI
agent writing code.

Varyk compiles to Rust and runs on the Rust ecosystem, the way TypeScript
compiles to JavaScript. The Rust compiler checks everything Varyk generates,
so the guarantees are Rust's own. Varyk code lives in `.vr` files, Rust code
can live in `.rs` files next to them, and the two build together.

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

It prints `Alice`, then `Bob`. This is the Rust that Varyk generates for it
(`src/main.rs` from `varyk build --emit-rust examples/borrowing.vr`):

```rust
#![allow(
    dead_code,
    unused_variables,
    unused_mut,
    arithmetic_overflow,
    unconditional_panic
)]

struct User {
    name: String,
}

fn rename(user: &mut User) {
    user.name = "Bob".to_string();
}

fn print_user(user: &User) {
    println!("{}", user.name);
}

fn main() {
    let mut user = User {
        name: "Alice".to_string(),
    };
    print_user(&user);
    rename(&mut user);
    print_user(&user);
}
```

The generated Rust is checked by the Rust compiler like any other Rust code.
Varyk keeps Rust's safety model and adds no garbage collector.

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

The compiler has three commands:

```text
varyk check <file.vr>    check the program for errors; never runs cargo
varyk build <file.vr>    generate Rust, build it, and print the path of the executable
varyk run <file.vr>      build, then run the program
```

`build` and `run` accept `--release`. `build` also accepts `--emit-rust`,
which prints the generated Rust. Every command accepts
`--message-format=json`, which prints errors as JSON, one object per line.
When running from this repository, put `cargo run -p varyk --` in front, as
in the example above.

## Status

Varyk is experimental and pre-0.1. Any syntax, error code, or command-line
flag may change before a 0.1 release, which is not scheduled. This is
milestone 1: a small compiler that proves the approach. It builds and runs
the six programs in `examples/`, and it reports every error it knows about
with a code, a plain-word message, and, where it can, a suggested fix. Enums,
loops other than `while`, packages, and much more are not there yet; see
[docs/language.md](docs/language.md) for exactly what works.

## Documents

- [varyk.com](https://varyk.com): the project site.

- [docs/language.md](docs/language.md): the language reference, one page.
- [docs/design.md](docs/design.md): the principles and the decisions behind them.
- [docs/open-questions.md](docs/open-questions.md): questions not yet answered.
- [docs/roadmap.md](docs/roadmap.md): what comes next.
- [docs/specs/2026-09-23-varyk-design.md](docs/specs/2026-09-23-varyk-design.md): the full design.

## License

Licensed under either of [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE),
at your option.

Unless you explicitly state otherwise, any contribution intentionally
submitted for inclusion in the work by you, as defined in the Apache-2.0
license, shall be dual licensed as above, without any additional terms or
conditions.

## Trademark

The code is open source; the name is not. "Varyk" is a trademark of
Vladislav Mickevic, and the licenses above do not grant rights to it. You may
use the name to refer to the project, but a modified version or a fork needs
a different name. See [TRADEMARKS.md](TRADEMARKS.md).
