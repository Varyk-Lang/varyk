pub fn keep(text: String, values: Vec<varyk_std::Value>) -> usize {
    text.len() + values.len()
}

pub fn grow(list: &mut Vec<i64>, values: Vec<varyk_std::Value>) {
    list.push(values.len() as i64);
}
