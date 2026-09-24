pub fn take(s: String) -> i32 {
    s.len() as i32
}

pub fn push(s: &mut String) {
    s.push('!');
}

pub fn make() -> String {
    String::from("made")
}
