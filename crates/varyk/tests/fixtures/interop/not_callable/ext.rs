pub fn by_ref_string(s: &String) -> usize {
    s.len()
}

pub fn generic<T>(value: T) -> T {
    value
}

pub fn ref_return(s: &str) -> &String {
    Box::leak(Box::new(s.to_string()))
}
