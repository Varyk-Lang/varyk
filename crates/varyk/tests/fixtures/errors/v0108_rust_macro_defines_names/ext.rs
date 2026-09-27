macro_rules! point {
    ($name:ident) => {
        pub struct $name {
            pub x: i32,
        }
    };
}

point!(Spot);

pub fn double(n: i32) -> i32 {
    n * 2
}
