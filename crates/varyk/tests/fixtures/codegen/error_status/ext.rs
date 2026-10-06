pub fn bad(text: &str) -> varyk_std::Error {
    varyk_std::Error::with_status(400, text.to_string())
}
