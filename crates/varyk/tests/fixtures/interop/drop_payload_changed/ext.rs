pub struct Acc {
    pub n: i32,
}

impl Acc {
    pub fn add(&mut self, k: i32) {
        self.n += k;
    }
}

pub enum Guard {
    Held(Acc),
    Empty,
}

impl Drop for Guard {
    fn drop(&mut self) {}
}

pub fn make_guard() -> Guard {
    Guard::Held(Acc { n: 1 })
}
