//! The names under which `Time` and `Uuid` ask a serializer that is not
//! human-readable to keep their type. A facade for a database whose
//! serializer is binary matches these names in `serialize_newtype_struct`:
//! `TIME` wraps the `i64` Unix microseconds, `UUID` wraps the 16 raw bytes
//! (`serialize_bytes`). `Bytes` needs no name: it calls `serialize_bytes`.
//! A human-readable serializer, JSON included, gets the written string, as
//! it always has.
//!
//! The forms are one-way: a `Deserialize` impl reads the string or the raw
//! bytes as before and never looks for these names. These strings are a
//! contract with those facades and never change.

/// The newtype struct name of a `Time`, around its `i64` Unix microseconds.
pub const TIME: &str = "$__varyk_std_Time";

/// The newtype struct name of a `Uuid`, around its 16 raw bytes.
pub const UUID: &str = "$__varyk_std_Uuid";
