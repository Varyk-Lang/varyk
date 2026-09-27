pub enum Kind {
    Word(i32),
    Number(i32),
    Unit,
}

pub fn classify(n: i32) -> Kind {
    if n == 0 {
        Kind::Unit
    } else if n < 0 {
        Kind::Word(n)
    } else {
        Kind::Number(n)
    }
}
