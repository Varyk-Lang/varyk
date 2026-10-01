pub struct A {
    pub n: i32,
}

impl A {
    pub fn new() -> A {
        A { n: 1 }
    }

    pub const fn c() -> i32 {
        1
    }

    #[cfg(test)]
    pub fn t(&self) {}

    pub fn g(&self, #[cfg(test)] y: i32) {}
}

impl Clone for A {
    fn clone(&self) -> A {
        A { n: self.n }
    }
}
