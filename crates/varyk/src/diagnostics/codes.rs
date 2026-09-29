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
/// Unknown type, or a Rust struct Varyk did not import (generic, tuple,
/// or unit).
pub const V0101: &str = "V0101";
/// Unknown field.
pub const V0102: &str = "V0102";
/// Duplicate definition.
pub const V0103: &str = "V0103";
/// Module file missing, ambiguous, unparseable, or named `main`.
pub const V0104: &str = "V0104";
/// Item or field not visible, needs `pub`; or a struct literal with
/// fields that cannot be set here.
pub const V0105: &str = "V0105";
/// Missing or malformed `main`, or `main` in a library's `src/lib.vr`.
pub const V0106: &str = "V0106";
/// `String` or `str` spelled where `string` is meant.
pub const V0107: &str = "V0107";
/// Unsupported Rust signature, or a field whose Rust type Varyk cannot
/// use.
pub const V0108: &str = "V0108";
/// A struct that contains itself, directly or through other structs.
pub const V0109: &str = "V0109";
/// `use` of a crate this compiler recognizes by name (`std`, `core`,
/// `alloc`, and a package's `[dependencies]`);
/// Varyk code reaches crates through a `.rs` module in the package. A
/// leading name it does not recognize as a crate is V0100 instead (with a
/// note), so a typo of a local module name is not misreported as one.
pub const V0110: &str = "V0110";
/// A path Varyk cannot follow: a variant, `super` in the crate root, or a
/// `use` leading name that is a module declared elsewhere in the crate,
/// reachable only through `crate::`.
pub const V0111: &str = "V0111";
/// Type mismatch.
pub const V0200: &str = "V0200";
/// Wrong argument count.
pub const V0201: &str = "V0201";
/// `println!` placeholder count mismatch or unsupported placeholder.
pub const V0202: &str = "V0202";
/// `{}` or `==` applied to a struct.
pub const V0203: &str = "V0203";
/// A `match` that does not handle every variant, naming one it misses.
pub const V0204: &str = "V0204";
/// A pattern that does not fit the value matched on: a variant of another
/// type, the wrong number of positions, a catch-all that is not the last
/// arm, or a value that is not an enum, `Option`, or `Result`.
pub const V0205: &str = "V0205";
/// `?` in a function that does not return `Result`, or on a value that is
/// not a `Result` with the function's error type.
pub const V0206: &str = "V0206";
/// A value whose type cannot be worked out where it is written (`None`,
/// an empty `vec![]`, `Ok`, `Err`); the type must be written.
pub const V0207: &str = "V0207";
/// A value that must be used where it is made: an `Option` holding part of
/// a stored value (`get`) stored, passed, returned, used with `?`, or
/// given any method (M4 spec 2.8).
pub const V0208: &str = "V0208";
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
/// A place changed or given away while another name for part of it is
/// still used later.
pub const V0307: &str = "V0307";
/// A function returning part of more than one parameter (M4 spec 3.1).
pub const V0308: &str = "V0308";
/// The package's edition is not 2024.
pub const V0400: &str = "V0400";
/// `workspace = true` in the manifest, or a target-specific dependency
/// table; not supported yet.
pub const V0401: &str = "V0401";
/// A target table (`[lib]`, `[[bin]]`, ...) with a `path`: a package's
/// roots are fixed at `src/main.vr` and `src/lib.vr`.
pub const V0402: &str = "V0402";
/// This `Cargo.toml` cannot be used: unreadable, not valid TOML, no
/// `[package]` `name`, an invalid or reserved `name`, or both or neither
/// of `src/main.vr` and `src/lib.vr`.
pub const V0403: &str = "V0403";
/// The Rust rustc compiled from a generated file was rejected, at the
/// Varyk line that produced it; carries rustc's message and code and asks
/// for a bug report, since Varyk's own checks should have caught this
/// first (spec 5). An error or warning in a copied `.rs` module is the
/// user's own Rust and is not this code: it passes through at their file.
pub const V0900: &str = "V0900";

/// Every code, the syntax codes first, in order: what the reference test
/// checks `docs/language.md` against.
pub const ALL: &[&str] = &[
    V0001, V0002, V0003, V0010, V0011, V0012, V0100, V0101, V0102, V0103, V0104, V0105, V0106,
    V0107, V0108, V0109, V0110, V0111, V0200, V0201, V0202, V0203, V0204, V0205, V0206, V0207,
    V0208, V0300, V0301, V0302, V0303, V0304, V0305, V0306, V0307, V0308, V0400, V0401, V0402,
    V0403, V0900,
];

#[cfg(test)]
mod tests {
    use super::ALL;

    /// `ALL` names every `pub const` of this file and every code of its
    /// `pub use` line, and nothing else.
    #[test]
    fn all_lists_every_code_of_this_file() {
        let text = include_str!("codes.rs");
        let mut expected: Vec<&str> = Vec::new();
        for line in text.lines() {
            if let Some(rest) = line.strip_prefix("pub use varyk_syntax::{") {
                let names = rest.trim_end_matches("};");
                expected.extend(names.split(", "));
            } else if let Some((name, _)) = line
                .strip_prefix("pub const ")
                .and_then(|rest| rest.split_once(':'))
                .filter(|(name, _)| *name != "ALL")
            {
                expected.push(name);
            }
        }
        assert_eq!(ALL, expected.as_slice());
    }
}
