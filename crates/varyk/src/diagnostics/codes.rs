//! The full diagnostic code registry.
//!
//! The syntax codes (`V0001`-`V0003`, `V0010`-`V0012`) are defined in
//! `varyk-syntax` and re-exported here so callers only need one path. The
//! rest — name resolution, typing, and borrow codes — are defined here. A
//! code, once assigned, is never reused for a different meaning (spec
//! section 3).

pub use varyk_syntax::{V0001, V0002, V0003, V0010, V0011, V0012};

/// Unknown name.
pub const V0100: &str = "V0100";
/// Unknown type.
pub const V0101: &str = "V0101";
/// Unknown field.
pub const V0102: &str = "V0102";
/// Duplicate definition.
pub const V0103: &str = "V0103";
/// Module file missing, ambiguous, unparseable, or named `main`.
pub const V0104: &str = "V0104";
/// Item not visible, needs `pub`.
pub const V0105: &str = "V0105";
/// Missing or malformed `main`.
pub const V0106: &str = "V0106";
/// `String` or `str` spelled where `string` is meant.
pub const V0107: &str = "V0107";
/// Unsupported Rust signature.
pub const V0108: &str = "V0108";
/// A struct that contains itself, directly or through other structs.
pub const V0109: &str = "V0109";
/// Type mismatch.
pub const V0200: &str = "V0200";
/// Wrong argument count.
pub const V0201: &str = "V0201";
/// `println!` placeholder count mismatch or unsupported placeholder.
pub const V0202: &str = "V0202";
/// `{}` or `==` applied to a struct.
pub const V0203: &str = "V0203";
/// Mutation through a non-`mut` parameter.
pub const V0300: &str = "V0300";
/// Assignment to an immutable `let` binding.
pub const V0301: &str = "V0301";
/// Immutable binding passed to a `mut` parameter.
pub const V0302: &str = "V0302";
/// Non-`mut` parameter passed to a `mut` parameter.
pub const V0303: &str = "V0303";
/// Borrowed place flowing into an owned slot.
pub const V0304: &str = "V0304";
/// Use after move.
pub const V0305: &str = "V0305";
/// The same value passed by reference twice to one call, once to a `mut`
/// parameter.
pub const V0306: &str = "V0306";
