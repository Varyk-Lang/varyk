# consumer

A plain Rust binary that depends on the `units` Varyk library the way any
crate would, with no `varyk` on the path. Its dependency is the crate
`varyk publish` assembles at `../target/varyk/package/units`, so that
assembly must exist before `cargo build` here works. Make it, without
publishing anything, from `examples/packages/units`:

```sh
varyk publish --assemble-only
```

Then, here:

```sh
cargo run    # prints "7 meters"
```
