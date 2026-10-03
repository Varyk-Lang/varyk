pub struct Store {
    pub name: String,
}

impl Store {
    pub fn open() -> Store {
        Store {
            name: String::new(),
        }
    }

    pub async fn one<T: varyk_std::serde::de::DeserializeOwned>(
        &self,
        text: &'static str,
    ) -> Result<T, varyk_std::Error> {
        varyk_std::json::parse(text)
    }

    pub async fn first<T: serde::de::DeserializeOwned>(
        &self,
        text: &'static str,
    ) -> Result<Option<T>, varyk_std::Error> {
        varyk_std::json::parse(text)
    }

    pub fn all<T: varyk_std::serde::de::DeserializeOwned>(
        &self,
        text: &'static str,
    ) -> Result<Vec<T>, varyk_std::Error> {
        varyk_std::json::parse(text)
    }

    pub fn read<T: varyk_std::serde::de::DeserializeOwned>(
        text: &'static str,
    ) -> Result<T, varyk_std::Error> {
        varyk_std::json::parse(text)
    }
}

pub fn read<T: varyk_std::serde::de::DeserializeOwned>(
    text: &'static str,
) -> Result<T, varyk_std::Error> {
    varyk_std::json::parse(text)
}

pub async fn fetch<T: varyk_std::serde::de::DeserializeOwned>(
    text: &'static str,
) -> Result<T, varyk_std::Error> {
    varyk_std::json::parse(text)
}
