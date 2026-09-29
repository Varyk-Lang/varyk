#[derive(Clone)]
pub struct Handle {
    pub id: i32,
}

pub struct Socket {
    pub port: i32,
}

pub fn open() -> Handle {
    Handle { id: 1 }
}

pub fn listen() -> Socket {
    Socket { port: 80 }
}
