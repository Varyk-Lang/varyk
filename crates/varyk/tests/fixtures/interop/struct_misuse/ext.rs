pub struct Matcher {
    pub label: String,
    words: Vec<String>,
}

impl Matcher {
    pub fn new(pattern: &str) -> Matcher {
        Matcher { label: pattern.to_string(), words: Vec::new() }
    }
    pub fn is_match(&self, s: &str) -> bool {
        self.words.iter().any(|w| w == s)
    }
    pub fn into_inner(self) -> String {
        self.label
    }
    pub fn name(&self) -> &String {
        &self.label
    }
}
