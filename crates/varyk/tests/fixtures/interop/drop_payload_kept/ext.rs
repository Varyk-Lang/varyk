pub struct Acc {
    pub n: i32,
}

pub enum Guard {
    Held(String),
    Box(Acc),
    Empty,
}

impl Drop for Guard {
    fn drop(&mut self) {}
}

pub fn make_guard() -> Guard {
    Guard::Held(String::from("hi"))
}
