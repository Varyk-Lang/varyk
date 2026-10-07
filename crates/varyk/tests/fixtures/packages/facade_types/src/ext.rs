// The facade forms of `Time`, `Uuid`, and `Bytes` (milestone 5c spec
// 2.5): `Time` and `Uuid` by value, alone and in `Option`, `Vec`, and
// `Result`; `Bytes` lent and owned; the three as `pub` fields and `Bytes`
// as the field of an enum's variant.

pub fn later(t: varyk_std::Time, seconds: i64) -> Result<varyk_std::Time, varyk_std::Error> {
    t.add_seconds(seconds)
}

pub fn read_time(text: &str) -> Option<varyk_std::Time> {
    varyk_std::Time::from_iso(text).ok()
}

pub fn or_now(t: Option<varyk_std::Time>) -> varyk_std::Time {
    match t {
        Some(t) => t,
        None => varyk_std::Time::now(),
    }
}

pub fn latest(times: Vec<varyk_std::Time>) -> Option<varyk_std::Time> {
    times.into_iter().max()
}

pub fn twice(t: varyk_std::Time) -> Vec<varyk_std::Time> {
    vec![t, t]
}

pub fn same(id: varyk_std::Uuid) -> varyk_std::Uuid {
    id
}

pub fn ids(count: usize) -> Vec<varyk_std::Uuid> {
    (0..count).map(|_| varyk_std::Uuid::v4()).collect()
}

pub fn first_id(ids: Vec<varyk_std::Uuid>) -> Option<varyk_std::Uuid> {
    ids.first().copied()
}

pub fn pick(id: Option<varyk_std::Uuid>, other: varyk_std::Uuid) -> varyk_std::Uuid {
    match id {
        Some(id) => id,
        None => other,
    }
}

pub fn read_id(text: &str) -> Result<varyk_std::Uuid, varyk_std::Error> {
    text.parse()
}

pub fn size(b: &varyk_std::Bytes) -> usize {
    b.len()
}

pub fn keep(b: varyk_std::Bytes) -> varyk_std::Bytes {
    b
}

pub struct Upload {
    pub id: varyk_std::Uuid,
    pub at: varyk_std::Time,
    pub data: varyk_std::Bytes,
}

impl Upload {
    pub fn new(data: varyk_std::Bytes) -> Upload {
        Upload {
            id: varyk_std::Uuid::new(),
            at: varyk_std::Time::now(),
            data,
        }
    }
}

// `varyk-http`'s WebSocket message has this shape.
pub enum Message {
    Text(String),
    Binary(varyk_std::Bytes),
}

pub fn next(binary: bool) -> Message {
    if binary {
        Message::Binary(varyk_std::Bytes::from_text("abc"))
    } else {
        Message::Text("hello".to_string())
    }
}
