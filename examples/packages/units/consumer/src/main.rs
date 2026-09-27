// Plain Rust, built with plain cargo: no `varyk` is involved. Varyk
// parameters borrow by default, so `add` takes `&Meters`.
use units::length::{Meters, add};

fn main() {
    let a = Meters { value: 3 };
    let b = Meters { value: 4 };
    println!("{} meters", add(&a, &b).value);
}
