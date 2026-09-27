pub enum K {
    A,
    B,
}

impl K {
    pub fn count(&self) -> i32 {
        1
    }
}

pub fn make() -> K {
    K::A
}
