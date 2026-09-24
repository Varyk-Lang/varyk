pub fn mix(a: i64, b: &i32, c: &mut String, d: String) -> u8 {
    c.push_str(&d);
    (a + *b as i64) as u8
}
