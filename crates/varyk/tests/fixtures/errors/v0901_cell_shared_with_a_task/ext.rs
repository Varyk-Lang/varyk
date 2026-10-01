pub struct Counter {
    pub start: i64,
    hits: std::cell::Cell<i64>,
}

impl Counter {
    pub fn new(start: i64) -> Counter {
        Counter {
            start,
            hits: std::cell::Cell::new(0),
        }
    }

    pub fn hit(&self) -> i64 {
        self.hits.set(self.hits.get() + 1);
        self.start + self.hits.get()
    }
}
