pub struct Namer {
    label: String,
}

impl Namer {
    pub fn name<T: varyk_std::serde::Serialize + ?Sized>(&self, value: &T) -> &str {
        let _ = value;
        &self.label
    }
}
