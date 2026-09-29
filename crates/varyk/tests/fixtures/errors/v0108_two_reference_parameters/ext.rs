pub fn longer(a: &str, b: &str) -> &str {
    if a.len() > b.len() { a } else { b }
}
