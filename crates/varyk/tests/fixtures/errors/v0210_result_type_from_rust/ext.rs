pub struct Handle {
    pub id: i64,
}

pub fn read<T: varyk_std::serde::de::DeserializeOwned>(
    text: &'static str,
) -> Result<T, varyk_std::Error> {
    varyk_std::json::parse(text)
}
