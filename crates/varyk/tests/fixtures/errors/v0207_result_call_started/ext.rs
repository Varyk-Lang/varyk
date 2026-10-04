pub async fn fetch<T: varyk_std::serde::de::DeserializeOwned>(
    text: &'static str,
) -> Result<T, varyk_std::Error> {
    varyk_std::json::parse(text)
}
