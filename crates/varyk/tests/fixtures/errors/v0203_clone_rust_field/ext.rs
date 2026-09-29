pub struct Handle {
    pub id: i32,
}

pub fn open() -> Handle {
    Handle { id: 1 }
}
