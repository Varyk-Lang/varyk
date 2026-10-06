pub struct Client {
    pub base: String,
}

impl Client {
    pub fn open() -> Client {
        Client {
            base: String::new(),
        }
    }

    pub fn post<T: serde::Serialize + ?Sized>(&self, url: &str, body: &T) -> usize {
        url.len() + varyk_std::json::stringify(body).len()
    }
}

pub fn json<T: varyk_std::serde::Serialize + ?Sized>(value: &T) -> String {
    varyk_std::json::stringify(value)
}
