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
/// An attribute Varyk does not have, in a place it cannot go, written
/// twice, or with a value missing or not expected (M5a spec 2.2).
pub const V0112: &str = "V0112";
/// A name the standard library needs (M5a spec 2.10): a struct, enum, or
/// module named `Error`, or a `.rs` module's `pub` struct or enum named so;
/// a module named `json`, `env`, or `log`, or a `use` of one; a function
/// named `assert` or `assert_eq`; any item, method, module, or `use` name
/// starting with `varyk_`.
pub const V0113: &str = "V0113";
/// A `#[test]` function with parameters or a return type, called, or the
/// entry `main` (M5a spec 2.7).
pub const V0114: &str = "V0114";
/// Type mismatch.
pub const V0200: &str = "V0200";
/// Wrong argument count.
pub const V0201: &str = "V0201";
/// `println!` placeholder count mismatch or unsupported placeholder.
pub const V0202: &str = "V0202";
/// `{}` applied to a struct, enum, or container, or `==` or `.clone()`
/// on a type that cannot have it; `Error` prints its message.
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
/// A `#[rename]` or `#[default]` value that does not fit, or `#[default]`
/// on a field whose type cannot have one; on a type a `json` or `env`
/// call reaches, a skipped field that is read with no default, or two
/// keys that are the same (M5a spec 2.2).
pub const V0209: &str = "V0209";
/// A type that cannot go through `json` or `env` at a call, naming the
/// part in the way (M5a spec 2.9).
pub const V0210: &str = "V0210";
/// A call to an async function, or `.await`, in an ordinary function;
/// `.await` in a closure (milestone 5b1 spec 2.2, 2.3).
pub const V0211: &str = "V0211";
/// `.await` on something that is not a call to an async function
/// (milestone 5b1 spec 2.3).
pub const V0212: &str = "V0212";
/// A started call whose task would be thrown away, or a task, or a `Vec`
/// of tasks, that nothing awaits or detaches (milestone 5b1 spec 2.3,
/// 2.4).
pub const V0213: &str = "V0213";
/// Async functions that call each other in a cycle, naming it (milestone
/// 5b1 spec 2.2).
pub const V0214: &str = "V0214";
/// A task, or a `Vec` of tasks, used other than where it is made allows,
/// or `Task` written as a type (milestone 5b1 spec 2.4).
pub const V0215: &str = "V0215";
/// `Shared` of anything but a struct, at `Shared::new` or written, or
/// `Shared` written anywhere but a parameter's or a `let`'s type
/// (milestone 5b1 spec 2.6).
pub const V0216: &str = "V0216";
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
/// A started call passing a value to a `mut` parameter or a `mut self`
/// receiver (milestone 5b1 spec 3).
pub const V0309: &str = "V0309";
/// Changing something reached through a `Shared` (milestone 5b1 spec
/// 2.6).
pub const V0310: &str = "V0310";
/// An async function that returns part of a parameter (milestone 5b1 spec
/// 2.2).
pub const V0311: &str = "V0311";
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
/// A program that uses `varyk-std` whose `Cargo.toml` does not depend on
/// it, depends on it from a `path` or `git` or with a version that is not
/// the compiler's minor, or whose `Cargo.lock` holds an older one.
pub const V0404: &str = "V0404";
/// The Rust rustc compiled from a generated file was rejected, at the
/// Varyk line that produced it; carries rustc's message and code and asks
/// for a bug report, since Varyk's own checks should have caught this
/// first (spec 5). An error or warning in a copied `.rs` module is the
/// user's own Rust and is not this code: it passes through at their file.
pub const V0900: &str = "V0900";
/// A started call whose task holds a value from Rust code that cannot be
/// sent to, or shared with, another thread: rustc's "cannot be sent (or
/// shared) between threads safely" at a generated `Task::start`, at the
/// Varyk line (milestone 5b1 spec 5). The `.rs` module's choice, not a
/// bug in Varyk.
pub const V0901: &str = "V0901";

/// Every code, the syntax codes first, in order: what the reference test
/// checks `docs/language.md` against.
pub const ALL: &[&str] = &[
    V0001, V0002, V0003, V0010, V0011, V0012, V0100, V0101, V0102, V0103, V0104, V0105, V0106,
    V0107, V0108, V0109, V0110, V0111, V0112, V0113, V0114, V0200, V0201, V0202, V0203, V0204,
    V0205, V0206, V0207, V0208, V0209, V0210, V0211, V0212, V0213, V0214, V0215, V0216, V0300,
    V0301, V0302, V0303, V0304, V0305, V0306, V0307, V0308, V0309, V0310, V0311, V0400, V0401,
    V0402, V0403, V0404, V0900, V0901,
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
