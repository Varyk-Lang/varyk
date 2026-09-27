pub struct Matcher {
    pub label: String,
    words: Vec<String>,
}

impl Matcher {
    pub fn new(label: &str) -> Matcher {
        Matcher { label: label.to_string(), words: Vec::new() }
    }
}
