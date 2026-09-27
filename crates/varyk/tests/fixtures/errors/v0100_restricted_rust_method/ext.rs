pub struct Tally {
    pub n: i32,
}

impl Tally {
    pub fn new() -> Tally {
        Tally { n: 0 }
    }

    pub(crate) fn bump(&mut self) {
        self.n += 1;
    }
}
