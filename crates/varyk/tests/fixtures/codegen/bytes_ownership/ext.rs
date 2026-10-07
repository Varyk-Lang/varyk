pub fn size(b: &varyk_std::Bytes) -> usize {
    b.len()
}

pub fn keep(b: varyk_std::Bytes) -> varyk_std::Bytes {
    b
}

pub enum Message {
    Text(String),
    Binary(varyk_std::Bytes),
}
