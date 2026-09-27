#[repr(packed)]
pub struct S {
    pub a: u8,
    pub x: i32,
}

pub fn make() -> S {
    S { a: 1, x: 2 }
}
