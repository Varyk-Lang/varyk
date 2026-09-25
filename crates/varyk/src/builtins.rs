//! The built-in calls of milestone 2 (spec 2.6): the whole standard-library
//! surface Varyk offers on `Vec` and `string`, as data. Nothing is read from
//! Rust's standard library; the checker types these calls from [`TABLE`],
//! borrow analysis reads each entry's receiver mode, and the backend emits
//! each by its name.

use crate::types::{IntKind, ParamMode, Ty};

/// The built-in type a call belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Owner {
    Vec,
    String,
}

impl Owner {
    /// The type's name as written in Varyk.
    pub fn name(self) -> &'static str {
        match self {
            Owner::Vec => "Vec",
            Owner::String => "string",
        }
    }
}

/// How a call uses its receiver.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Receiver {
    /// No receiver: an associated function, called `Vec::name(..)`.
    None,
    /// A shared borrow.
    Reads,
    /// A mutable borrow.
    Changes,
}

/// A parameter or result type; `T` stands for the element type of the
/// `Vec` the call is made on (or expected, for `Vec::new()`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    T,
    VecOfT,
    OptionOfT,
    Usize,
    String,
    Unit,
}

impl Shape {
    /// The type this shape stands for, with `element` as `T`.
    pub fn ty(self, element: &Ty) -> Ty {
        match self {
            Shape::T => element.clone(),
            Shape::VecOfT => Ty::Vec(Box::new(element.clone())),
            Shape::OptionOfT => Ty::Option(Box::new(element.clone())),
            Shape::Usize => Ty::Int(IntKind::Usize),
            Shape::String => Ty::String,
            Shape::Unit => Ty::Unit,
        }
    }
}

/// One row of the table of spec 2.6.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Builtin {
    pub owner: Owner,
    pub name: &'static str,
    pub receiver: Receiver,
    /// Every parameter is an owned slot (spec 3.4).
    pub params: &'static [Shape],
    pub result: Shape,
}

/// The table of spec 2.6, without `v[i]`, which is a place rather than a
/// call.
pub const TABLE: &[Builtin] = &[
    Builtin {
        owner: Owner::Vec,
        name: "new",
        receiver: Receiver::None,
        params: &[],
        result: Shape::VecOfT,
    },
    Builtin {
        owner: Owner::Vec,
        name: "push",
        receiver: Receiver::Changes,
        params: &[Shape::T],
        result: Shape::Unit,
    },
    Builtin {
        owner: Owner::Vec,
        name: "pop",
        receiver: Receiver::Changes,
        params: &[],
        result: Shape::OptionOfT,
    },
    Builtin {
        owner: Owner::Vec,
        name: "len",
        receiver: Receiver::Reads,
        params: &[],
        result: Shape::Usize,
    },
    Builtin {
        owner: Owner::String,
        name: "len",
        receiver: Receiver::Reads,
        params: &[],
        result: Shape::Usize,
    },
    Builtin {
        owner: Owner::String,
        name: "clone",
        receiver: Receiver::Reads,
        params: &[],
        result: Shape::String,
    },
];

/// A row of [`TABLE`], by index.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BuiltinId(pub u8);

impl BuiltinId {
    pub fn get(self) -> &'static Builtin {
        &TABLE[self.0 as usize]
    }

    /// How the call is written: `Vec::new` for an associated function,
    /// else the method's name.
    pub fn path(self) -> String {
        let entry = self.get();
        match entry.receiver {
            Receiver::None => format!("{}::{}", entry.owner.name(), entry.name),
            _ => entry.name.to_string(),
        }
    }

    /// The parameter modes borrow analysis sees: the receiver's first (for
    /// a method), then an owned slot per parameter.
    pub fn modes(self) -> Vec<ParamMode> {
        let entry = self.get();
        let receiver = match entry.receiver {
            Receiver::None => None,
            Receiver::Reads => Some(ParamMode::SharedBorrow),
            Receiver::Changes => Some(ParamMode::MutableBorrow),
        };
        receiver
            .into_iter()
            .chain(entry.params.iter().map(|_| ParamMode::Owned))
            .collect()
    }
}

/// The call `name` on `owner`: a method when `method`, else an associated
/// function.
pub fn lookup(owner: Owner, name: &str, method: bool) -> Option<BuiltinId> {
    TABLE
        .iter()
        .position(|entry| {
            entry.owner == owner
                && entry.name == name
                && (entry.receiver != Receiver::None) == method
        })
        .map(|index| BuiltinId(index as u8))
}

/// The names of `owner`'s methods (when `method`) or associated functions,
/// in table order, for a diagnostic that lists them.
pub fn names(owner: Owner, method: bool) -> Vec<&'static str> {
    TABLE
        .iter()
        .filter(|entry| entry.owner == owner && (entry.receiver != Receiver::None) == method)
        .map(|entry| entry.name)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_holds_the_calls_of_spec_2_6() {
        assert_eq!(names(Owner::Vec, true), ["push", "pop", "len"]);
        assert_eq!(names(Owner::Vec, false), ["new"]);
        assert_eq!(names(Owner::String, true), ["len", "clone"]);
        let push = lookup(Owner::Vec, "push", true).expect("push");
        assert_eq!(push.modes(), [ParamMode::MutableBorrow, ParamMode::Owned]);
        let new = lookup(Owner::Vec, "new", false).expect("Vec::new");
        assert_eq!(new.path(), "Vec::new");
        assert!(new.modes().is_empty());
        let pop = lookup(Owner::Vec, "pop", true).expect("pop");
        assert_eq!(
            pop.get().result.ty(&Ty::String),
            Ty::Option(Box::new(Ty::String))
        );
        assert_eq!(lookup(Owner::String, "push", true), None);
        assert_eq!(lookup(Owner::Vec, "new", true), None);
    }
}
