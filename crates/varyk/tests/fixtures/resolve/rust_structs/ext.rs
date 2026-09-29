use std::collections::HashMap;
use crate::other::Thing;

pub struct Matcher {
    pub label: String,
    words: Vec<String>,
    pub counts: HashMap<String, i32>,
    pub(crate) hits: u32,
}

impl Matcher {
    pub fn new(pattern: &str) -> Self {
        Matcher { label: pattern.to_string(), words: Vec::new(), counts: HashMap::new(), hits: 0 }
    }
    pub fn is_match(&self, s: &str) -> bool {
        self.words.iter().any(|w| w == s)
    }
    pub fn bump(&mut self) {
        self.hits += 1;
    }
    pub fn into_inner(self) -> String {
        self.label
    }
    pub fn name(&self) -> &String {
        &self.label
    }
    pub fn pick<T>(&self, value: T) -> T {
        value
    }
}

pub struct Holder {
    pub full: crate::other::Thing,
    pub short: Thing,
    pub local: crate::Local,
    pub list: Vec<Matcher>,
    pub wrapped: Wrap<i32>,
}

pub struct Wrap<T> {
    pub value: T,
}

pub struct Meters(pub f64);

pub fn pair(m: &Matcher, n: &mut Matcher, t: crate::other::Thing) -> Option<Matcher> {
    None
}
