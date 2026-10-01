pub struct Matcher {
    words: Vec<String>,
    pub hits: u32,
}

impl Matcher {
    pub fn new(pattern: &str) -> Matcher {
        let words = pattern.split('|').map(|word| word.to_string()).collect();
        Matcher { words, hits: 0 }
    }

    pub fn is_match(&self, s: &str) -> bool {
        self.words.iter().any(|word| word == s)
    }
}
