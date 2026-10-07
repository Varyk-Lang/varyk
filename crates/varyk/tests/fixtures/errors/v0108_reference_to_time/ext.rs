pub fn year_of(t: &varyk_std::Time) -> i64 {
    t.to_unix() / 31_556_952 + 1970
}
