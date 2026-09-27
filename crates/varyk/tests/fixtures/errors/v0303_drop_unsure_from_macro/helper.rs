macro_rules! quiet {
    ($t:ty) => {
        impl Drop for $t {
            fn drop(&mut self) {}
        }
    };
}
