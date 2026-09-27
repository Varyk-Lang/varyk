pub struct Boxed {
    pub h: crate::shop::hidden::H,
}

pub fn make() -> crate::shop::hidden::H {
    crate::shop::hidden::H { x: 1 }
}

pub fn boxed() -> Boxed {
    Boxed { h: make() }
}
