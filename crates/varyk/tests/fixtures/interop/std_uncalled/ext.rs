// Never called: the program still needs `varyk-std` to compile it.
#[allow(dead_code)]
pub fn load(id: i64) -> Result<i64, varyk_std::Error> {
    Ok(id)
}

#[allow(dead_code)]
pub fn count(values: Vec<varyk_std::Value>) -> usize {
    values.len()
}

pub fn twice(n: i64) -> i64 {
    n * 2
}
