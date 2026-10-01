pub struct Counter {
    pub n: i64,
}

impl Counter {
    pub async fn total(&self, more: i64) -> i64 {
        self.n + more
    }
}

pub async fn fetch(id: i64, name: String) -> String {
    format!("{name}{id}")
}
