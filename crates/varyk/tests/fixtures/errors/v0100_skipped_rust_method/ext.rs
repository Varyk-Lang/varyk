pub struct A {
    pub n: i32,
}

impl A {
    pub fn new() -> A {
        A { n: 1 }
    }
}

pub trait Labelled {
    fn label(&self) -> String;
}

impl Labelled for A {
    fn label(&self) -> String {
        String::from("a")
    }
}
