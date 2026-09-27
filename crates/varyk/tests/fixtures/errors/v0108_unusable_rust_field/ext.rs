use std::collections::HashMap;

pub struct Counts {
    pub total: i32,
    pub by_word: HashMap<String, i32>,
}

impl Counts {
    pub fn new() -> Counts {
        Counts { total: 0, by_word: HashMap::new() }
    }

    pub fn distinct(&self) -> i32 {
        self.by_word.len() as i32
    }
}
