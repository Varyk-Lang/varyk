pub struct Store {
    pub name: String,
}

impl Store {
    pub fn open() -> Store {
        Store {
            name: String::new(),
        }
    }

    pub async fn all<T: varyk_std::serde::de::DeserializeOwned>(
        &self,
        text: &'static str,
    ) -> Result<Vec<T>, varyk_std::Error> {
        varyk_std::json::parse(text)
    }
}
