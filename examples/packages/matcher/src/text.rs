// A facade: Varyk code never names a crate, so this file wraps
// `regex_lite` in a struct, methods, and an enum Varyk can import.

pub struct Matcher {
    re: regex_lite::Regex,
}

impl Matcher {
    pub fn new(pattern: &str) -> Matcher {
        let re = regex_lite::Regex::new(pattern).expect("the pattern should be a valid regex");
        Matcher { re }
    }

    pub fn is_match(&self, s: &str) -> bool {
        self.re.is_match(s)
    }

    pub fn count(&self, s: &str) -> usize {
        self.re.find_iter(s).count()
    }
}

pub enum Kind {
    Word,
    Number(i32),
}

pub fn classify(s: &str) -> Kind {
    // Deliberately unused: rustc's warning about it shows through
    // `varyk build` at this file and line, as Rust warnings do.
    let trimmed = s.trim();
    match s.parse::<i32>() {
        Ok(n) => Kind::Number(n),
        Err(_) => Kind::Word,
    }
}
