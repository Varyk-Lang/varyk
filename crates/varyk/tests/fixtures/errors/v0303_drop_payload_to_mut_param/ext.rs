pub enum Guard {
    Held(String),
    Empty,
}

impl Drop for Guard {
    fn drop(&mut self) {}
}

pub fn make_guard() -> Guard {
    Guard::Held(String::from("hi"))
}
