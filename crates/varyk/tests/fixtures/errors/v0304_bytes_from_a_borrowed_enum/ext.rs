pub enum Message {
    Text(String),
    Binary(varyk_std::Bytes),
}

pub fn keep(b: varyk_std::Bytes) -> varyk_std::Bytes {
    b
}
