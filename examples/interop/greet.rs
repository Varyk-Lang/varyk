pub fn hello(name: &str) -> String {
    format!("Hello from Rust, {}!", name)
}

pub fn first_word(s: &str) -> &str {
    match s.find(' ') {
        Some(i) => &s[..i],
        None => s,
    }
}
