pub struct Acc {
    pub n: i32,
}

pub enum Guard {
    Held(String),
    Box(Acc),
    Data(varyk_std::Bytes),
    Empty,
}

impl Drop for Guard {
    fn drop(&mut self) {}
}

pub fn make_guard() -> Guard {
    Guard::Held(String::from("hi"))
}
