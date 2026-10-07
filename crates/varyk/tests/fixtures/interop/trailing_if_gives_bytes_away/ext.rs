pub fn run(query: &'static str, values: Vec<varyk_std::Value>) -> usize {
    query.len() + values.len()
}
