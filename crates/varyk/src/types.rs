//! Resolved types and parameter modes (spec sections 4.1, 4.2, 6.3).

use crate::resolve::{EnumId, StructId};

/// A resolved type: the primitives, `string`, a user-declared struct or
/// enum, one of the four standard generic types (spec 2.6, M4 spec 2.7),
/// or the unit type of a function with no return value.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Ty {
    Bool,
    Int(IntKind),
    Float(FloatKind),
    /// `string`: never `Copy` in Varyk, whatever its generated Rust form.
    String,
    Struct(StructId),
    Enum(EnumId),
    Option(Box<Ty>),
    Result(Box<Ty>, Box<Ty>),
    Vec(Box<Ty>),
    /// `HashMap<K, V>`, `K` an integer type, `bool`, or `string`.
    HashMap(Box<Ty>, Box<Ty>),
    /// An unfinished chain of items of this type (M4 spec 2.3): internal,
    /// with no Varyk spelling, so it may only be the receiver of the next
    /// call of the chain (V0208 anywhere else).
    Chain(Box<Ty>),
    /// `Error` (M5a spec 2.3): a message, made by `Error::new` and by every
    /// failing `varyk-std` call. Neither Copy nor text: the ownership
    /// rules treat it like a struct.
    Error,
    /// `Task<T>` (milestone 5b1 spec 2.4): what a started call gives,
    /// awaited or detached where it is made. Never written: its type is
    /// always worked out from the call (V0215).
    Task(Box<Ty>),
    /// `Shared<T>` (milestone 5b1 spec 2.6): a handle many can read a
    /// struct through, `::std::sync::Arc<T>` in Rust. `T` is always a
    /// struct (V0216), and everything reached through it is read-only
    /// (V0310).
    Shared(Box<Ty>),
    Unit,
}

/// Every type name Varyk knows without a declaration: the primitives,
/// `string`, the four standard generic types, `Error`, and `Shared`. The
/// reference test checks `docs/language.md` mentions each.
pub const BUILTIN_TYPE_NAMES: &[&str] = &[
    "bool", "i8", "i16", "i32", "i64", "u8", "u16", "u32", "u64", "usize", "f32", "f64", "string",
    "Option", "Result", "Vec", "HashMap", "Error", "Shared",
];

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
    Usize,
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
            IntKind::Usize => "usize",
        }
    }

    /// The smallest and largest value of the type.
    pub fn range(self) -> (i128, i128) {
        match self {
            IntKind::I8 => (i8::MIN.into(), i8::MAX.into()),
            IntKind::I16 => (i16::MIN.into(), i16::MAX.into()),
            IntKind::I32 => (i32::MIN.into(), i32::MAX.into()),
            IntKind::I64 => (i64::MIN.into(), i64::MAX.into()),
            IntKind::U8 => (0, u8::MAX.into()),
            IntKind::U16 => (0, u16::MAX.into()),
            IntKind::U32 => (0, u32::MAX.into()),
            // Taken as 64 bits wide, as on every 64-bit target.
            IntKind::U64 | IntKind::Usize => (0, u64::MAX.into()),
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
    /// `bool`, the integers, and the floats are `Copy` (spec 4.2); `string`,
    /// structs, enums, `Option`, `Result`, `Vec`, and `HashMap` are not.
    pub fn is_copy(&self) -> bool {
        matches!(self, Ty::Bool | Ty::Int(_) | Ty::Float(_))
    }

    /// Whether the generated Rust type is `Copy`: a number or `bool`, or an
    /// `Option` or `Result` of such types at every depth, a `Result`'s
    /// error type included (M4 spec 3.4). A stored value of such a type can
    /// be taken by a method that uses up its receiver, since Rust copies
    /// it out.
    pub fn copy_in_rust(&self) -> bool {
        match self {
            Ty::Option(inner) => inner.copy_in_rust(),
            Ty::Result(ok, err) => ok.copy_in_rust() && err.copy_in_rust(),
            ty => ty.is_copy(),
        }
    }

    /// A struct, an enum, `Option`, `Result`, `Vec`, `HashMap`, `Error`, a
    /// task, or a `Shared`: a value
    /// that is neither Copy nor text, and that the ownership rules treat
    /// like a struct.
    pub fn is_compound(&self) -> bool {
        matches!(
            self,
            Ty::Struct(_)
                | Ty::Enum(_)
                | Ty::Option(_)
                | Ty::Result(..)
                | Ty::Vec(_)
                | Ty::HashMap(..)
                | Ty::Error
                | Ty::Task(_)
                | Ty::Shared(_)
        )
    }

    /// Whether `self` is or holds `Error`: a program naming it uses
    /// `varyk-std` (M5a spec 1).
    pub fn has_error(&self) -> bool {
        match self {
            Ty::Error => true,
            Ty::Option(inner) | Ty::Vec(inner) | Ty::Chain(inner) | Ty::Task(inner) => {
                inner.has_error()
            }
            Ty::Result(a, b) | Ty::HashMap(a, b) => a.has_error() || b.has_error(),
            _ => false,
        }
    }

    /// Whether `self` is or holds a `Shared` (milestone 5b1 spec 2.6),
    /// which cannot be compared or printed.
    pub fn has_shared(&self) -> bool {
        match self {
            Ty::Shared(_) => true,
            Ty::Option(inner) | Ty::Vec(inner) => inner.has_shared(),
            Ty::Result(a, b) | Ty::HashMap(a, b) => a.has_shared() || b.has_shared(),
            _ => false,
        }
    }

    /// The struct a field access or a method call on `self` reaches: a
    /// struct's own, or the one a `Shared` holds (milestone 5b1 spec 2.6).
    pub fn reached(&self) -> &Ty {
        match self {
            Ty::Shared(inner) => inner,
            ty => ty,
        }
    }

    /// Whether `self` is or holds an unfinished chain.
    pub fn has_chain(&self) -> bool {
        match self {
            Ty::Chain(_) => true,
            Ty::Option(inner) | Ty::Vec(inner) => inner.has_chain(),
            Ty::Result(a, b) | Ty::HashMap(a, b) => a.has_chain() || b.has_chain(),
            _ => false,
        }
    }

    /// Whether `self` is a task or a `Vec` of tasks (milestone 5b1 spec
    /// 2.4), which may only be used where it is made.
    pub fn is_tasks(&self) -> bool {
        match self {
            Ty::Task(_) => true,
            Ty::Vec(inner) => matches!(**inner, Ty::Task(_)),
            _ => false,
        }
    }

    /// Whether `self` may be a `HashMap` key: an integer type, `bool`, or
    /// `string` (M4 spec 2.7).
    pub fn is_map_key(&self) -> bool {
        matches!(self, Ty::Int(_) | Ty::Bool | Ty::String)
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
            "usize" => Ty::Int(IntKind::Usize),
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
pub(crate) mod derives;

pub use check::typecheck;
pub use derives::{Derives, Serde};
