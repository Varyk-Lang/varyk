pub struct Db {
    pub calls: i64,
}

impl Db {
    pub fn open() -> Db {
        Db { calls: 0 }
    }

    pub fn one(&self, query: &'static str) -> i64 {
        query.len() as i64 + self.calls
    }
}
