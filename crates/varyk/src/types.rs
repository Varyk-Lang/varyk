//! Resolved types and parameter modes (spec sections 4.1, 4.2, 6.3).

use crate::resolve::StructId;

/// A resolved milestone-1 type: the primitives, `string`, a user-declared
/// struct, or the unit type of a function with no return value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Ty {
    Bool,
    Int(IntKind),
    Float(FloatKind),
    /// `string`: never `Copy` in Varyk, whatever its generated Rust form.
    String,
    Struct(StructId),
    Unit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IntKind {
    I8,
    I16,
    I32,
    I64,
    U8,
    U16,
    U32,
    U64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FloatKind {
    F32,
    F64,
}

impl IntKind {
    /// The type's name, the same in Varyk and Rust.
    pub fn name(self) -> &'static str {
        match self {
            IntKind::I8 => "i8",
            IntKind::I16 => "i16",
            IntKind::I32 => "i32",
            IntKind::I64 => "i64",
            IntKind::U8 => "u8",
            IntKind::U16 => "u16",
            IntKind::U32 => "u32",
            IntKind::U64 => "u64",
        }
    }
}

impl FloatKind {
    /// The type's name, the same in Varyk and Rust.
    pub fn name(self) -> &'static str {
        match self {
            FloatKind::F32 => "f32",
            FloatKind::F64 => "f64",
        }
    }
}

impl Ty {
    /// `bool`, the integers, and the floats are `Copy` (spec 4.2); `string`
    /// and structs are not.
    pub fn is_copy(self) -> bool {
        matches!(self, Ty::Bool | Ty::Int(_) | Ty::Float(_))
    }

    /// Maps a primitive type name (`bool`, `i32`, `f64`, `string`, ...) to
    /// its type; `None` for any other name.
    pub fn from_primitive_name(name: &str) -> Option<Ty> {
        Some(match name {
            "bool" => Ty::Bool,
            "i8" => Ty::Int(IntKind::I8),
            "i16" => Ty::Int(IntKind::I16),
            "i32" => Ty::Int(IntKind::I32),
            "i64" => Ty::Int(IntKind::I64),
            "u8" => Ty::Int(IntKind::U8),
            "u16" => Ty::Int(IntKind::U16),
            "u32" => Ty::Int(IntKind::U32),
            "u64" => Ty::Int(IntKind::U64),
            "f32" => Ty::Float(FloatKind::F32),
            "f64" => Ty::Float(FloatKind::F64),
            "string" => Ty::String,
            _ => return None,
        })
    }
}

/// How a parameter is passed (spec 6.3). Every function, Varyk-declared or
/// imported, has one per parameter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ParamMode {
    SharedBorrow,
    MutableBorrow,
    Owned,
}

mod check;

pub use check::typecheck;
