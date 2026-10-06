pub struct Failures {
    last: varyk_std::Error,
}

impl Failures {
    pub fn last(&self) -> &varyk_std::Error {
        &self.last
    }
}
