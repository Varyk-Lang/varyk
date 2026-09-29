pub fn first_word(s: &str) -> &str {
    s.split(' ').next().unwrap_or("")
}

pub struct Note {
    body: String,
}

impl Note {
    pub fn new(body: &str) -> Note {
        Note {
            body: body.to_string(),
        }
    }

    pub fn text(&self) -> &str {
        &self.body
    }
}
