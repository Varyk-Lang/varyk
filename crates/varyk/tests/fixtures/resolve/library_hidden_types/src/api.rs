pub struct Shelf {
    pub square: crate::parts::Square,
}

impl Shelf {
    pub fn top(&self) -> crate::parts::Square {
        crate::parts::Square { side: self.square.side }
    }
}
