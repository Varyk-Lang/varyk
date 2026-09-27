pub enum Shape {
    Point,
    #[cfg(feature = "round")]
    Circle(f64),
}
