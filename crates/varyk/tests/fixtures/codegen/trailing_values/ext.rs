pub fn run(query: &'static str, values: Vec<varyk_std::Value>) -> usize {
    query.len() + values.len()
}

pub struct Db {
    pub name: String,
}

impl Db {
    pub fn new() -> Db {
        Db {
            name: String::new(),
        }
    }

    pub fn exec(&self, query: &'static str, values: Vec<varyk_std::Value>) -> usize {
        self.name.len() + query.len() + values.len()
    }
}
