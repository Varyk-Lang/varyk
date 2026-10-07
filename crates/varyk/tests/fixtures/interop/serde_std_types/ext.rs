// A type parameter bounded by serde reaches `Time`, `Uuid`, and `Bytes`
// with no change (milestone 5c spec 2.5).

pub fn decode<T: varyk_std::serde::de::DeserializeOwned>(
    text: &str,
) -> Result<T, varyk_std::Error> {
    varyk_std::json::parse(text)
}

pub fn encode<T: varyk_std::serde::Serialize + ?Sized>(value: &T) -> String {
    varyk_std::json::stringify(value)
}
