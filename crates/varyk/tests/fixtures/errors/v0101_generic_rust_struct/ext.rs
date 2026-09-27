pub struct Wrap<T> {
    pub value: T,
}

pub fn unwrap_i32(w: &Wrap<i32>) -> i32 {
    w.value
}
