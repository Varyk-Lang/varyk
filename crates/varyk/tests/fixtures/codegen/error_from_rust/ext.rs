pub fn load(id: i64) -> Result<i64, varyk_std::Error> {
    if id < 0 {
        return Err(varyk_std::Error::new(format!("no item {id}")));
    }
    Ok(id * 2)
}
