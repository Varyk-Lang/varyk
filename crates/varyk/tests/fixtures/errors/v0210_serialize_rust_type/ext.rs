pub struct Handle {
    pub id: i64,
}

pub fn open() -> Handle {
    Handle { id: 1 }
}

pub fn json<T: varyk_std::serde::Serialize + ?Sized>(value: &T) -> String {
    varyk_std::json::stringify(value)
}
