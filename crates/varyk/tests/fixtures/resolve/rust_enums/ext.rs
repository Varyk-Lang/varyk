use std::collections::HashMap;
use crate::other::Thing;

pub enum Kind {
    Word,
    Number(i32),
}

pub enum Shape {
    Circle { radius: f64 },
    Point,
}

pub enum Counted {
    Tally(HashMap<String, i32>),
}

pub enum ViaUse {
    Ref(Thing),
}

pub enum Holds {
    Full(crate::other::Thing),
}

pub enum Wrap<T> {
    Value(T),
}

pub struct Container {
    pub kind: Kind,
}

pub fn make_kind(n: i32) -> Kind {
    Kind::Number(n)
}
