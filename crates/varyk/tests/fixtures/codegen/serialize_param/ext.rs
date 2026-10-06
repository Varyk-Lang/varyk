pub struct Client {
    pub base: String,
}

impl Client {
    pub fn open() -> Client {
        Client {
            base: String::new(),
        }
    }

    pub fn post<T: varyk_std::serde::Serialize + ?Sized>(&self, url: &str, body: &T) -> String {
        format!("{}{} {}", self.base, url, varyk_std::json::stringify(body))
    }
}

pub fn json<T: varyk_std::serde::Serialize + ?Sized>(value: &T) -> String {
    varyk_std::json::stringify(value)
}
