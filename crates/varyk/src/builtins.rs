//! The built-in calls (M2 spec 2.6, M4 spec 2.7, M5a spec 2.3): the whole
//! standard-library surface Varyk offers on `Vec`, `string`, `Option`,
//! `Result`, `HashMap`, chains, `Error`, `json`, and `env`, as data. Nothing is read from Rust's standard library; the
//! checker types these calls from [`TABLE`], borrow analysis reads each
//! entry's parameter modes, and the backend emits each by its name.

use crate::types::{IntKind, ParamMode, Ty};

/// The built-in type a call belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Owner {
    Vec,
    String,
    Option,
    Result,
    HashMap,
    /// An unfinished chain (M4 spec 2.3), whose rows use `T` for the item.
    Chain,
    /// `Error` (M5a spec 2.3), made by `varyk-std`.
    Error,
    /// The `json` module (M5a spec 2.4), whose calls are written
    /// `json::name(..)`.
    Json,
    /// The `env` module (M5a spec 2.5), whose call is `env::parse()`.
    Env,
    /// The `log` module (M5a spec 2.6): `log::info(format, args..)`.
    Log,
    /// The `time` module (milestone 5b1 spec 2.7): `time::sleep(ms)`.
    Time,
    /// The checks of a `#[test]` function (M5a spec 2.7), `assert` and
    /// `assert_eq`, called by their bare names.
    Test,
    /// `Task<T>` (milestone 5b1 spec 2.4), a started call's task.
    Task,
    /// `Shared<T>` (milestone 5b1 spec 2.6): std's `Arc`, not a
    /// `varyk-std` call.
    Shared,
}

impl Owner {
    /// The type's name as written in Varyk.
    pub fn name(self) -> &'static str {
        match self {
            Owner::Vec => "Vec",
            Owner::String => "string",
            Owner::Option => "Option",
            Owner::Result => "Result",
            Owner::HashMap => "HashMap",
            Owner::Chain => "chain",
            Owner::Error => "Error",
            Owner::Json => "json",
            Owner::Env => "env",
            Owner::Log => "log",
            Owner::Time => "time",
            Owner::Test => "test",
            Owner::Task => "Task",
            Owner::Shared => "Shared",
        }
    }

    /// The owner of the calls on a value of type `ty`, with the types its
    /// shapes stand for; `None` for a type without table rows.
    pub fn of(ty: &Ty) -> Option<(Owner, Subst)> {
        let subst = Subst::default();
        Some(match ty {
            Ty::Vec(t) => (
                Owner::Vec,
                Subst {
                    t: (**t).clone(),
                    ..subst
                },
            ),
            Ty::String => (
                Owner::String,
                Subst {
                    t: Ty::String,
                    ..subst
                },
            ),
            Ty::Option(t) => (
                Owner::Option,
                Subst {
                    t: (**t).clone(),
                    ..subst
                },
            ),
            Ty::Result(t, e) => (
                Owner::Result,
                Subst {
                    t: (**t).clone(),
                    e: (**e).clone(),
                    ..subst
                },
            ),
            // A map is looked up by its key, so `T` is `K` there.
            Ty::HashMap(k, v) => (
                Owner::HashMap,
                Subst {
                    t: (**k).clone(),
                    k: (**k).clone(),
                    v: (**v).clone(),
                    ..subst
                },
            ),
            Ty::Chain(item) => (
                Owner::Chain,
                Subst {
                    t: (**item).clone(),
                    ..subst
                },
            ),
            Ty::Error => (Owner::Error, subst),
            Ty::Task(t) => (
                Owner::Task,
                Subst {
                    t: (**t).clone(),
                    ..subst
                },
            ),
            Ty::Shared(t) => (
                Owner::Shared,
                Subst {
                    t: (**t).clone(),
                    ..subst
                },
            ),
            _ => return None,
        })
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
    /// Used up, like the operand of `?` (M4 spec 3.4): a temporary or an
    /// owned local is moved, and a stored value is copied out only when
    /// it is Copy in Rust.
    Takes,
}

/// A parameter or result type, in the letters of M4 spec 2.7: `T` the
/// element or payload type (a map's key, which it is looked up by), `E` a
/// `Result`'s error type, `K` and `V` a map's key and value types, and the
/// expected type for a result taken from where it goes. A `Read` shape is
/// a parameter lent read-only rather than an owned slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    T,
    VecOfT,
    OptionOfT,
    Usize,
    /// `u64` (`time::sleep`'s milliseconds).
    U64,
    String,
    Unit,
    Bool,
    K,
    V,
    E,
    OptionOfV,
    ResultOfTAndE,
    HashMapOfKV,
    /// `Result<X, Error>`, `X` the type the expected `Result` holds
    /// (`parse`).
    ResultOfExpected,
    /// `Error`.
    Error,
    /// A `string` lent read-only, passed as a `&str`.
    ReadString,
    /// A `T` lent read-only.
    ReadT,
    /// `R`, what the call's closure returns.
    R,
    /// `Option<R>`.
    OptionOfR,
    /// `Result<T, R>`.
    ResultOfTAndR,
    /// A closure, `|x| ..` (M4 spec 2.2): its type is what it returns.
    Closure(ClosureShape),
    /// A chain of `T` items (M4 spec 2.3).
    ChainOfT,
    /// A chain of `K` items.
    ChainOfK,
    /// A chain of `V` items.
    ChainOfV,
    /// A chain of `string` items.
    ChainOfString,
    /// A chain of `R` items.
    ChainOfR,
    /// `Vec<Task<T>>`, the tasks `Task::all` waits for (milestone 5b1
    /// spec 2.5).
    TasksOfT,
    /// `Shared<T>` (milestone 5b1 spec 2.6).
    SharedOfT,
}

/// A closure parameter of a row: the type it hands its closure and what
/// the closure returns. Separate from [`Shape`], which cannot hold itself
/// in a `const` table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClosureShape {
    pub param: ClosureParamShape,
    pub result: ClosureResult,
}

/// The value a row hands its closure: `T`, the payload or element (on a
/// chain, the item), or `E`, a `Result`'s error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClosureParamShape {
    T,
    E,
}

impl ClosureParamShape {
    pub fn ty(self, subst: &Subst) -> Ty {
        match self {
            ClosureParamShape::T => subst.t.clone(),
            ClosureParamShape::E => subst.e.clone(),
        }
    }
}

/// What a row's closure must return: a `bool`, or anything, which is `R`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClosureResult {
    Bool,
    Any,
}

/// The types a row's shapes stand for. A part the call has no use for is
/// `()`; the checker fills in every part a row's shapes name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Subst {
    pub t: Ty,
    pub e: Ty,
    pub k: Ty,
    pub v: Ty,
    /// What a closure returns (M4 spec 2.2).
    pub r: Ty,
    pub expected: Ty,
}

impl Default for Subst {
    fn default() -> Self {
        Subst {
            t: Ty::Unit,
            e: Ty::Unit,
            k: Ty::Unit,
            v: Ty::Unit,
            r: Ty::Unit,
            expected: Ty::Unit,
        }
    }
}

impl Shape {
    /// The type this shape stands for under `subst`.
    pub fn ty(self, subst: &Subst) -> Ty {
        let boxed = |ty: &Ty| Box::new(ty.clone());
        match self {
            Shape::T | Shape::ReadT => subst.t.clone(),
            Shape::VecOfT => Ty::Vec(boxed(&subst.t)),
            Shape::OptionOfT => Ty::Option(boxed(&subst.t)),
            Shape::Usize => Ty::Int(IntKind::Usize),
            Shape::U64 => Ty::Int(IntKind::U64),
            Shape::String | Shape::ReadString => Ty::String,
            Shape::Unit => Ty::Unit,
            Shape::Bool => Ty::Bool,
            Shape::K => subst.k.clone(),
            Shape::V => subst.v.clone(),
            Shape::E => subst.e.clone(),
            Shape::OptionOfV => Ty::Option(boxed(&subst.v)),
            Shape::ResultOfTAndE => Ty::Result(boxed(&subst.t), boxed(&subst.e)),
            Shape::HashMapOfKV => Ty::HashMap(boxed(&subst.k), boxed(&subst.v)),
            Shape::ResultOfExpected => Ty::Result(boxed(&subst.expected), Box::new(Ty::Error)),
            Shape::Error => Ty::Error,
            Shape::R => subst.r.clone(),
            Shape::OptionOfR => Ty::Option(boxed(&subst.r)),
            Shape::ResultOfTAndR => Ty::Result(boxed(&subst.t), boxed(&subst.r)),
            Shape::Closure(closure) => match closure.result {
                ClosureResult::Bool => Ty::Bool,
                ClosureResult::Any => subst.r.clone(),
            },
            Shape::ChainOfT => Ty::Chain(boxed(&subst.t)),
            Shape::ChainOfK => Ty::Chain(boxed(&subst.k)),
            Shape::ChainOfV => Ty::Chain(boxed(&subst.v)),
            Shape::ChainOfString => Ty::Chain(Box::new(Ty::String)),
            Shape::ChainOfR => Ty::Chain(boxed(&subst.r)),
            Shape::TasksOfT => Ty::Vec(Box::new(Ty::Task(boxed(&subst.t)))),
            Shape::SharedOfT => Ty::Shared(boxed(&subst.t)),
        }
    }

    /// Whether the parameter is lent read-only rather than an owned slot.
    pub fn is_read(self) -> bool {
        matches!(self, Shape::ReadString | Shape::ReadT)
    }

    /// Whether the parameter is a closure.
    pub fn is_closure(self) -> bool {
        matches!(self, Shape::Closure(_))
    }
}

/// What a row's result is. Every row of milestone 2 and every row whose
/// result is a plain value is `Value`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResultKind {
    /// A value of its own, owned or Copy, borrowing nothing.
    Value,
    /// Part of the receiver, as a borrowed return is part of its root (M4
    /// spec 3.1): the call is a place rooted where the receiver is.
    Borrowed,
    /// An `Option` holding part of the receiver (M4 spec 2.8): a plain
    /// `Option` of a copy when the payload is Copy, and otherwise one that
    /// must be looked inside right where it is made. On a chain (`find`),
    /// looked into only when the items are borrowed (M4 spec 3.3).
    LookInside,
    /// A chain (M4 spec 2.3): a source on its receiver, or an adapter.
    Chain,
}

/// What a row asks of the element type `T` of the `Vec` it is called on
/// (M4 spec 2.7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ElementRule {
    Any,
    /// A number type.
    Number,
    /// An integer type, `bool`, or `string`: what `sort` orders.
    Ordered,
    /// A number type, `bool`, or `string`: what `contains` compares.
    Comparable,
    /// `string` only: what `join` joins.
    StringOnly,
}

impl ElementRule {
    pub fn accepts(self, ty: &Ty) -> bool {
        match self {
            ElementRule::Any => true,
            ElementRule::Number => matches!(ty, Ty::Int(_) | Ty::Float(_)),
            ElementRule::Ordered => matches!(ty, Ty::Int(_) | Ty::Bool | Ty::String),
            ElementRule::Comparable => {
                matches!(ty, Ty::Int(_) | Ty::Float(_) | Ty::Bool | Ty::String)
            }
            ElementRule::StringOnly => *ty == Ty::String,
        }
    }

    /// The `Vec`s the rule accepts, in words.
    pub fn wanted(self) -> &'static str {
        match self {
            ElementRule::Any => "a `Vec`",
            ElementRule::Number => "a `Vec` of a number type",
            ElementRule::Ordered => "a `Vec` of an integer type, `bool`, or `string`",
            ElementRule::Comparable => "a `Vec` of a number type, `bool`, or `string`",
            ElementRule::StringOnly => "a `Vec<string>`",
        }
    }
}

/// One row of the tables of M2 spec 2.6 and M4 spec 2.7.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Builtin {
    pub owner: Owner,
    pub name: &'static str,
    pub receiver: Receiver,
    /// An owned slot each, unless a `Read` shape (spec 3.4).
    pub params: &'static [Shape],
    pub result: Shape,
    pub result_kind: ResultKind,
    /// What the row asks of a `Vec`'s element type.
    pub element: ElementRule,
    /// An async call (milestone 5b1 spec 2.7), awaited like a call to an
    /// async function.
    pub is_async: bool,
}

/// A row whose result is a plain value and which asks nothing of the
/// element type.
const fn row(
    owner: Owner,
    name: &'static str,
    receiver: Receiver,
    params: &'static [Shape],
    result: Shape,
) -> Builtin {
    Builtin {
        owner,
        name,
        receiver,
        params,
        result,
        result_kind: ResultKind::Value,
        element: ElementRule::Any,
        is_async: false,
    }
}

/// `row`, with an element rule.
const fn element_row(
    name: &'static str,
    receiver: Receiver,
    params: &'static [Shape],
    result: Shape,
    element: ElementRule,
) -> Builtin {
    Builtin {
        element,
        ..row(Owner::Vec, name, receiver, params, result)
    }
}

/// `row`, whose result is part of its receiver (M4 spec 3.1).
const fn borrowed_row(owner: Owner, name: &'static str, result: Shape) -> Builtin {
    Builtin {
        result_kind: ResultKind::Borrowed,
        ..row(owner, name, Receiver::Reads, &[], result)
    }
}

/// A closure parameter handing its closure `param` and taking back
/// anything.
const fn closure(param: ClosureParamShape) -> Shape {
    Shape::Closure(ClosureShape {
        param,
        result: ClosureResult::Any,
    })
}

/// `row`, whose result is looked into (M4 spec 2.8).
const fn look_inside_row(owner: Owner, params: &'static [Shape], result: Shape) -> Builtin {
    Builtin {
        result_kind: ResultKind::LookInside,
        ..row(owner, "get", Receiver::Reads, params, result)
    }
}

/// A source of a chain (M4 spec 2.3): reads its receiver, which must be
/// a place (or, for `split`, a string literal).
const fn source_row(
    owner: Owner,
    name: &'static str,
    params: &'static [Shape],
    result: Shape,
) -> Builtin {
    Builtin {
        result_kind: ResultKind::Chain,
        ..row(owner, name, Receiver::Reads, params, result)
    }
}

/// A row of a chain, which takes the chain it is called on.
const fn chain_row(
    name: &'static str,
    params: &'static [Shape],
    result: Shape,
    result_kind: ResultKind,
) -> Builtin {
    Builtin {
        result_kind,
        ..row(Owner::Chain, name, Receiver::Takes, params, result)
    }
}

/// A closure parameter handed the item and returning a `bool`.
const TEST: Shape = Shape::Closure(ClosureShape {
    param: ClosureParamShape::T,
    result: ClosureResult::Bool,
});

use Receiver::{Changes, Reads, Takes};

/// The tables, without `v[i]`, which is a place rather than a call.
pub const TABLE: &[Builtin] = &[
    row(Owner::Vec, "new", Receiver::None, &[], Shape::VecOfT),
    row(Owner::Vec, "push", Changes, &[Shape::T], Shape::Unit),
    row(Owner::Vec, "pop", Changes, &[], Shape::OptionOfT),
    row(Owner::Vec, "len", Reads, &[], Shape::Usize),
    row(Owner::String, "len", Reads, &[], Shape::Usize),
    // Also `.clone()` on any other type that can be cloned (M4 spec
    // 2.10), whose value is then of the receiver's type.
    row(Owner::String, "clone", Reads, &[], Shape::String),
    row(Owner::Vec, "is_empty", Reads, &[], Shape::Bool),
    row(
        Owner::Vec,
        "insert",
        Changes,
        &[Shape::Usize, Shape::T],
        Shape::Unit,
    ),
    row(Owner::Vec, "remove", Changes, &[Shape::Usize], Shape::T),
    element_row(
        "contains",
        Reads,
        &[Shape::ReadT],
        Shape::Bool,
        ElementRule::Comparable,
    ),
    element_row("sort", Changes, &[], Shape::Unit, ElementRule::Ordered),
    element_row(
        "join",
        Reads,
        &[Shape::ReadString],
        Shape::String,
        ElementRule::StringOnly,
    ),
    row(Owner::String, "is_empty", Reads, &[], Shape::Bool),
    row(
        Owner::String,
        "contains",
        Reads,
        &[Shape::ReadString],
        Shape::Bool,
    ),
    row(
        Owner::String,
        "starts_with",
        Reads,
        &[Shape::ReadString],
        Shape::Bool,
    ),
    row(Owner::String, "to_uppercase", Reads, &[], Shape::String),
    row(
        Owner::String,
        "replace",
        Reads,
        &[Shape::ReadString, Shape::ReadString],
        Shape::String,
    ),
    borrowed_row(Owner::String, "trim", Shape::String),
    row(
        Owner::String,
        "push_str",
        Changes,
        &[Shape::ReadString],
        Shape::Unit,
    ),
    row(Owner::String, "parse", Reads, &[], Shape::ResultOfExpected),
    row(Owner::Option, "is_some", Reads, &[], Shape::Bool),
    row(Owner::Result, "is_ok", Reads, &[], Shape::Bool),
    row(Owner::Result, "is_err", Reads, &[], Shape::Bool),
    row(Owner::Option, "unwrap_or", Takes, &[Shape::T], Shape::T),
    row(
        Owner::Option,
        "ok_or",
        Takes,
        &[Shape::E],
        Shape::ResultOfTAndE,
    ),
    row(Owner::Result, "ok", Takes, &[], Shape::OptionOfT),
    row(Owner::Result, "unwrap_or", Takes, &[Shape::T], Shape::T),
    row(
        Owner::Option,
        "map",
        Takes,
        &[closure(ClosureParamShape::T)],
        Shape::OptionOfR,
    ),
    row(
        Owner::Result,
        "map_err",
        Takes,
        &[closure(ClosureParamShape::E)],
        Shape::ResultOfTAndR,
    ),
    look_inside_row(Owner::Vec, &[Shape::Usize], Shape::OptionOfT),
    row(
        Owner::HashMap,
        "new",
        Receiver::None,
        &[],
        Shape::HashMapOfKV,
    ),
    row(
        Owner::HashMap,
        "insert",
        Changes,
        &[Shape::K, Shape::V],
        Shape::OptionOfV,
    ),
    row(
        Owner::HashMap,
        "contains_key",
        Reads,
        &[Shape::ReadT],
        Shape::Bool,
    ),
    row(Owner::HashMap, "len", Reads, &[], Shape::Usize),
    look_inside_row(Owner::HashMap, &[Shape::ReadT], Shape::OptionOfV),
    source_row(Owner::Vec, "iter", &[], Shape::ChainOfT),
    source_row(
        Owner::String,
        "split",
        &[Shape::ReadString],
        Shape::ChainOfString,
    ),
    source_row(Owner::HashMap, "keys", &[], Shape::ChainOfK),
    source_row(Owner::HashMap, "values", &[], Shape::ChainOfV),
    chain_row(
        "map",
        &[closure(ClosureParamShape::T)],
        Shape::ChainOfR,
        ResultKind::Chain,
    ),
    chain_row("filter", &[TEST], Shape::ChainOfT, ResultKind::Chain),
    chain_row("collect", &[], Shape::VecOfT, ResultKind::Value),
    chain_row("count", &[], Shape::Usize, ResultKind::Value),
    Builtin {
        element: ElementRule::Number,
        ..chain_row("sum", &[], Shape::T, ResultKind::Value)
    },
    chain_row("any", &[TEST], Shape::Bool, ResultKind::Value),
    chain_row("all", &[TEST], Shape::Bool, ResultKind::Value),
    chain_row("find", &[TEST], Shape::OptionOfT, ResultKind::LookInside),
    // `Error::new` takes its text as an owned `string` (M5a spec 3).
    row(
        Owner::Error,
        "new",
        Receiver::None,
        &[Shape::String],
        Shape::Error,
    ),
    borrowed_row(Owner::Error, "message", Shape::String),
    // `json::parse` reads its text and makes the type expected of it;
    // `json::stringify` reads its value (M5a spec 2.4, 3).
    row(
        Owner::Json,
        "parse",
        Receiver::None,
        &[Shape::ReadString],
        Shape::ResultOfExpected,
    ),
    row(
        Owner::Json,
        "stringify",
        Receiver::None,
        &[Shape::ReadT],
        Shape::String,
    ),
    // `env::parse` makes the struct expected of it (M5a spec 2.5).
    row(
        Owner::Env,
        "parse",
        Receiver::None,
        &[],
        Shape::ResultOfExpected,
    ),
    // The four `log` calls take a format string and arguments, checked
    // like `println!`'s; the rows only name them (M5a spec 2.6).
    row(Owner::Log, "debug", Receiver::None, &[], Shape::Unit),
    row(Owner::Log, "info", Receiver::None, &[], Shape::Unit),
    row(Owner::Log, "warn", Receiver::None, &[], Shape::Unit),
    row(Owner::Log, "error", Receiver::None, &[], Shape::Unit),
    // `assert(cond)` and `assert_eq(a, b)`, only inside a `#[test]`
    // function; the checker types them itself (M5a spec 2.7).
    row(
        Owner::Test,
        "assert",
        Receiver::None,
        &[Shape::Bool],
        Shape::Unit,
    ),
    row(
        Owner::Test,
        "assert_eq",
        Receiver::None,
        &[Shape::T, Shape::T],
        Shape::Unit,
    ),
    // `time::sleep(ms)` waits without holding up other tasks (milestone
    // 5b1 spec 2.7).
    Builtin {
        is_async: true,
        ..row(
            Owner::Time,
            "sleep",
            Receiver::None,
            &[Shape::U64],
            Shape::Unit,
        )
    },
    // `t.detach()` takes the task and lets it run on (milestone 5b1 spec
    // 2.4).
    row(Owner::Task, "detach", Takes, &[], Shape::Unit),
    // `Task::all(tasks).await` and `Task::all_settled(tasks).await` take
    // the tasks and wait for every one (milestone 5b1 spec 2.5). The
    // checker types their results itself: `all` on tasks giving a
    // `Result` gives a `Result` of the values.
    task_all_row("all"),
    task_all_row("all_settled"),
    // `Shared::new(value)` takes the struct it shares, and `s.clone()`
    // gives another handle to it (milestone 5b1 spec 2.6); the checker
    // asks that `T` be a struct.
    row(
        Owner::Shared,
        "new",
        Receiver::None,
        &[Shape::T],
        Shape::SharedOfT,
    ),
    row(Owner::Shared, "clone", Reads, &[], Shape::SharedOfT),
];

/// `Task::all` or `Task::all_settled`: async, taking a `Vec` of tasks.
const fn task_all_row(name: &'static str) -> Builtin {
    Builtin {
        is_async: true,
        ..row(
            Owner::Task,
            name,
            Receiver::None,
            &[Shape::TasksOfT],
            Shape::VecOfT,
        )
    }
}

impl Builtin {
    /// Whether the call is a `varyk-std` call (M5a spec 1): a row of
    /// `Error`, `json`, `env`, `log`, `time`, or `Task` (milestone 5b1 spec
    /// 4), or `parse`, whose `Error` `varyk-std` makes.
    pub fn uses_std(&self) -> bool {
        matches!(
            self.owner,
            Owner::Error | Owner::Json | Owner::Env | Owner::Log | Owner::Time | Owner::Task
        ) || (self.owner == Owner::String && self.name == "parse")
    }
}

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
    /// a method), then a shared borrow per read parameter and per closure,
    /// which only borrows what it captures, and an owned slot per other
    /// one.
    pub fn modes(self) -> Vec<ParamMode> {
        let entry = self.get();
        let receiver = match entry.receiver {
            Receiver::None => None,
            Receiver::Reads => Some(ParamMode::SharedBorrow),
            Receiver::Changes => Some(ParamMode::MutableBorrow),
            Receiver::Takes => Some(ParamMode::Owned),
        };
        receiver
            .into_iter()
            .chain(entry.params.iter().map(|shape| {
                if shape.is_read() || shape.is_closure() {
                    ParamMode::SharedBorrow
                } else {
                    ParamMode::Owned
                }
            }))
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

/// The rows with a closure parameter, in table order: the only calls a
/// closure may be written in (M4 spec 2.2).
pub fn closure_rows() -> impl Iterator<Item = &'static Builtin> {
    TABLE
        .iter()
        .filter(|entry| entry.params.iter().any(|shape| shape.is_closure()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::FloatKind;

    fn row(owner: Owner, name: &str, method: bool) -> &'static Builtin {
        match lookup(owner, name, method) {
            Some(id) => id.get(),
            None => panic!("no row {name} on {owner:?}"),
        }
    }

    #[test]
    fn the_table_holds_the_calls_of_spec_2_6() {
        assert_eq!(
            names(Owner::Vec, true),
            [
                "push", "pop", "len", "is_empty", "insert", "remove", "contains", "sort", "join",
                "get", "iter"
            ]
        );
        assert_eq!(names(Owner::Vec, false), ["new"]);
        let push = lookup(Owner::Vec, "push", true).expect("push");
        assert_eq!(push.modes(), [ParamMode::MutableBorrow, ParamMode::Owned]);
        let new = lookup(Owner::Vec, "new", false).expect("Vec::new");
        assert_eq!(new.path(), "Vec::new");
        assert!(new.modes().is_empty());
        let pop = lookup(Owner::Vec, "pop", true).expect("pop");
        let strings = Subst {
            t: Ty::String,
            ..Subst::default()
        };
        assert_eq!(
            pop.get().result.ty(&strings),
            Ty::Option(Box::new(Ty::String))
        );
        assert_eq!(lookup(Owner::String, "push", true), None);
        assert_eq!(lookup(Owner::Vec, "new", true), None);
    }

    /// A row as the spec's table writes it: owner, name, receiver,
    /// parameters, result, and element rule.
    type Row = (
        Owner,
        &'static str,
        Receiver,
        &'static [Shape],
        Shape,
        ElementRule,
    );

    #[test]
    fn the_value_rows_of_m4_spec_2_7() {
        use ElementRule::{Any, Comparable, Ordered, StringOnly};
        use Receiver::{Changes, None as NoReceiver, Reads};
        use Shape::*;
        let rows: &[Row] = &[
            (Owner::Vec, "is_empty", Reads, &[], Bool, Any),
            (Owner::Vec, "insert", Changes, &[Usize, T], Unit, Any),
            (Owner::Vec, "remove", Changes, &[Usize], T, Any),
            (Owner::Vec, "contains", Reads, &[ReadT], Bool, Comparable),
            (Owner::Vec, "sort", Changes, &[], Unit, Ordered),
            (Owner::Vec, "join", Reads, &[ReadString], String, StringOnly),
            (Owner::String, "is_empty", Reads, &[], Bool, Any),
            (Owner::String, "contains", Reads, &[ReadString], Bool, Any),
            (
                Owner::String,
                "starts_with",
                Reads,
                &[ReadString],
                Bool,
                Any,
            ),
            (Owner::String, "to_uppercase", Reads, &[], String, Any),
            (
                Owner::String,
                "replace",
                Reads,
                &[ReadString, ReadString],
                String,
                Any,
            ),
            (Owner::String, "push_str", Changes, &[ReadString], Unit, Any),
            (Owner::String, "parse", Reads, &[], ResultOfExpected, Any),
            (Owner::Option, "is_some", Reads, &[], Bool, Any),
            (Owner::Result, "is_ok", Reads, &[], Bool, Any),
            (Owner::Result, "is_err", Reads, &[], Bool, Any),
            (Owner::HashMap, "new", NoReceiver, &[], HashMapOfKV, Any),
            (Owner::HashMap, "insert", Changes, &[K, V], OptionOfV, Any),
            (Owner::HashMap, "contains_key", Reads, &[ReadT], Bool, Any),
            (Owner::HashMap, "len", Reads, &[], Usize, Any),
        ];
        for &(owner, name, receiver, params, result, element) in rows {
            let entry = row(owner, name, receiver != NoReceiver);
            assert_eq!(entry.receiver, receiver, "{name}");
            assert_eq!(entry.params, params, "{name}");
            assert_eq!(entry.result, result, "{name}");
            assert_eq!(entry.result_kind, ResultKind::Value, "{name}");
            assert_eq!(entry.element, element, "{name}");
        }
        assert_eq!(
            names(Owner::String, true),
            [
                "len",
                "clone",
                "is_empty",
                "contains",
                "starts_with",
                "to_uppercase",
                "replace",
                "trim",
                "push_str",
                "parse",
                "split"
            ]
        );
        assert_eq!(
            names(Owner::HashMap, true),
            ["insert", "contains_key", "len", "get", "keys", "values"]
        );
        assert_eq!(names(Owner::HashMap, false), ["new"]);
        assert_eq!(Owner::HashMap.name(), "HashMap");
    }

    #[test]
    fn the_taking_borrowed_and_looked_into_rows_of_m4_spec_2_7() {
        use Receiver::{Reads, Takes};
        use ResultKind::{Borrowed, LookInside, Value};
        use Shape::*;
        /// As [`Row`], with the result kind in place of the element rule.
        type KindRow = (
            Owner,
            &'static str,
            Receiver,
            &'static [Shape],
            Shape,
            ResultKind,
        );
        let rows: &[KindRow] = &[
            (Owner::Option, "unwrap_or", Takes, &[T], T, Value),
            (Owner::Option, "ok_or", Takes, &[E], ResultOfTAndE, Value),
            (Owner::Result, "ok", Takes, &[], OptionOfT, Value),
            (Owner::Result, "unwrap_or", Takes, &[T], T, Value),
            (
                Owner::Option,
                "map",
                Takes,
                &[Closure(ClosureShape {
                    param: ClosureParamShape::T,
                    result: ClosureResult::Any,
                })],
                OptionOfR,
                Value,
            ),
            (
                Owner::Result,
                "map_err",
                Takes,
                &[Closure(ClosureShape {
                    param: ClosureParamShape::E,
                    result: ClosureResult::Any,
                })],
                ResultOfTAndR,
                Value,
            ),
            (Owner::Vec, "get", Reads, &[Usize], OptionOfT, LookInside),
            (Owner::String, "trim", Reads, &[], String, Borrowed),
            (
                Owner::HashMap,
                "get",
                Reads,
                &[ReadT],
                OptionOfV,
                LookInside,
            ),
        ];
        for &(owner, name, receiver, params, result, kind) in rows {
            let id = lookup(owner, name, true).unwrap_or_else(|| panic!("{name}"));
            let entry = id.get();
            assert_eq!(entry.receiver, receiver, "{name}");
            assert_eq!(entry.params, params, "{name}");
            assert_eq!(entry.result, result, "{name}");
            assert_eq!(entry.result_kind, kind, "{name}");
            assert_eq!(entry.element, ElementRule::Any, "{name}");
        }
        // A taking receiver is used up like an owned argument.
        let modes = lookup(Owner::Option, "unwrap_or", true).map(BuiltinId::modes);
        assert_eq!(modes, Some(vec![ParamMode::Owned, ParamMode::Owned]));
        let modes = lookup(Owner::HashMap, "get", true).map(BuiltinId::modes);
        assert_eq!(
            modes,
            Some(vec![ParamMode::SharedBorrow, ParamMode::SharedBorrow])
        );
        assert_eq!(
            names(Owner::Option, true),
            ["is_some", "unwrap_or", "ok_or", "map"]
        );
        assert_eq!(
            names(Owner::Result, true),
            ["is_ok", "is_err", "ok", "unwrap_or", "map_err"]
        );
        // A closure only borrows what it captures.
        let modes = lookup(Owner::Option, "map", true).map(BuiltinId::modes);
        assert_eq!(modes, Some(vec![ParamMode::Owned, ParamMode::SharedBorrow]));
        let closures: Vec<(Owner, &str)> = closure_rows()
            .map(|entry| (entry.owner, entry.name))
            .collect();
        assert_eq!(
            closures[..2],
            [(Owner::Option, "map"), (Owner::Result, "map_err")]
        );
        // What a stored receiver may be taken as: Copy in Rust, at every
        // depth, a `Result`'s error type included.
        let (i32_, boxed) = (Ty::Int(IntKind::I32), |ty: Ty| Box::new(ty));
        assert!(i32_.copy_in_rust());
        assert!(Ty::Option(boxed(Ty::Option(boxed(Ty::Bool)))).copy_in_rust());
        assert!(Ty::Result(boxed(i32_.clone()), boxed(Ty::Bool)).copy_in_rust());
        assert!(!Ty::Result(boxed(i32_.clone()), boxed(Ty::String)).copy_in_rust());
        assert!(!Ty::Option(boxed(Ty::String)).copy_in_rust());
        assert!(!Ty::Vec(boxed(i32_)).copy_in_rust());
    }

    #[test]
    fn the_chain_rows_of_m4_spec_2_7() {
        use Receiver::{Reads, Takes};
        use ResultKind::{Chain, LookInside, Value};
        use Shape::*;
        /// As [`Row`], with the result kind in place of the element rule.
        type KindRow = (
            Owner,
            &'static str,
            Receiver,
            &'static [Shape],
            Shape,
            ResultKind,
        );
        const ANY: Shape = Closure(ClosureShape {
            param: ClosureParamShape::T,
            result: ClosureResult::Any,
        });
        const FILTER: &[Shape] = &[Closure(ClosureShape {
            param: ClosureParamShape::T,
            result: ClosureResult::Bool,
        })];
        let rows: &[KindRow] = &[
            (Owner::Vec, "iter", Reads, &[], ChainOfT, Chain),
            (
                Owner::String,
                "split",
                Reads,
                &[ReadString],
                ChainOfString,
                Chain,
            ),
            (Owner::HashMap, "keys", Reads, &[], ChainOfK, Chain),
            (Owner::HashMap, "values", Reads, &[], ChainOfV, Chain),
            (Owner::Chain, "map", Takes, &[ANY], ChainOfR, Chain),
            (Owner::Chain, "filter", Takes, FILTER, ChainOfT, Chain),
            (Owner::Chain, "collect", Takes, &[], VecOfT, Value),
            (Owner::Chain, "count", Takes, &[], Usize, Value),
            (Owner::Chain, "sum", Takes, &[], T, Value),
            (Owner::Chain, "any", Takes, FILTER, Bool, Value),
            (Owner::Chain, "all", Takes, FILTER, Bool, Value),
            (Owner::Chain, "find", Takes, FILTER, OptionOfT, LookInside),
        ];
        for &(owner, name, receiver, params, result, kind) in rows {
            let id = lookup(owner, name, true).unwrap_or_else(|| panic!("{name}"));
            let entry = id.get();
            assert_eq!(entry.receiver, receiver, "{name}");
            assert_eq!(entry.params, params, "{name}");
            assert_eq!(entry.result, result, "{name}");
            assert_eq!(entry.result_kind, kind, "{name}");
            let element = if name == "sum" {
                ElementRule::Number
            } else {
                ElementRule::Any
            };
            assert_eq!(entry.element, element, "{name}");
        }
        assert_eq!(
            names(Owner::Chain, true),
            [
                "map", "filter", "collect", "count", "sum", "any", "all", "find"
            ]
        );
        assert_eq!(Owner::Chain.name(), "chain");
        // `T` is the item, so `T`, `VecOfT`, and `OptionOfT` serve a chain.
        let item = Ty::Chain(Box::new(Ty::String));
        let (owner, subst) = Owner::of(&item).expect("a chain has rows");
        assert_eq!(owner, Owner::Chain);
        assert_eq!(subst.t, Ty::String);
        assert_eq!(VecOfT.ty(&subst), Ty::Vec(Box::new(Ty::String)));
        let subst = Subst {
            t: Ty::Bool,
            k: Ty::String,
            v: Ty::Int(IntKind::I32),
            r: Ty::Int(IntKind::U8),
            ..Subst::default()
        };
        let chain = |ty: Ty| Ty::Chain(Box::new(ty));
        assert_eq!(ChainOfT.ty(&subst), chain(Ty::Bool));
        assert_eq!(ChainOfK.ty(&subst), chain(Ty::String));
        assert_eq!(ChainOfV.ty(&subst), chain(Ty::Int(IntKind::I32)));
        assert_eq!(ChainOfString.ty(&subst), chain(Ty::String));
        assert_eq!(ChainOfR.ty(&subst), chain(Ty::Int(IntKind::U8)));
        // A chain consumes its receiver in Rust, so its rows take it.
        let modes = lookup(Owner::Chain, "map", true).map(BuiltinId::modes);
        assert_eq!(modes, Some(vec![ParamMode::Owned, ParamMode::SharedBorrow]));
        let closures: Vec<(Owner, &str)> = closure_rows()
            .map(|entry| (entry.owner, entry.name))
            .collect();
        assert_eq!(
            closures,
            [
                (Owner::Option, "map"),
                (Owner::Result, "map_err"),
                (Owner::Chain, "map"),
                (Owner::Chain, "filter"),
                (Owner::Chain, "any"),
                (Owner::Chain, "all"),
                (Owner::Chain, "find"),
            ]
        );
    }

    #[test]
    fn read_parameters_are_shared_borrows_and_the_rest_owned() {
        let modes = |owner, name| {
            lookup(owner, name, true)
                .unwrap_or_else(|| panic!("{name}"))
                .modes()
        };
        let (read, changes, owned) = (
            ParamMode::SharedBorrow,
            ParamMode::MutableBorrow,
            ParamMode::Owned,
        );
        assert_eq!(modes(Owner::String, "replace"), [read, read, read]);
        assert_eq!(modes(Owner::String, "push_str"), [changes, read]);
        assert_eq!(modes(Owner::Vec, "contains"), [read, read]);
        assert_eq!(modes(Owner::Vec, "join"), [read, read]);
        assert_eq!(modes(Owner::Vec, "insert"), [changes, owned, owned]);
        assert_eq!(modes(Owner::Vec, "remove"), [changes, owned]);
        assert_eq!(modes(Owner::HashMap, "contains_key"), [read, read]);
        assert_eq!(modes(Owner::HashMap, "insert"), [changes, owned, owned]);
    }

    #[test]
    fn shapes_substitute_every_type_they_stand_for() {
        let subst = Subst {
            t: Ty::String,
            e: Ty::Bool,
            k: Ty::String,
            v: Ty::Int(IntKind::I32),
            r: Ty::Float(FloatKind::F32),
            expected: Ty::Int(IntKind::U8),
        };
        let boxed = |ty: Ty| Box::new(ty);
        let f32_ = Ty::Float(FloatKind::F32);
        assert_eq!(Shape::R.ty(&subst), f32_);
        assert_eq!(Shape::OptionOfR.ty(&subst), Ty::Option(boxed(f32_.clone())));
        assert_eq!(
            Shape::ResultOfTAndR.ty(&subst),
            Ty::Result(boxed(Ty::String), boxed(f32_.clone()))
        );
        let any = ClosureShape {
            param: ClosureParamShape::E,
            result: ClosureResult::Any,
        };
        assert_eq!(Shape::Closure(any).ty(&subst), f32_);
        assert_eq!(any.param.ty(&subst), Ty::Bool);
        let test = ClosureShape {
            param: ClosureParamShape::T,
            result: ClosureResult::Bool,
        };
        assert_eq!(Shape::Closure(test).ty(&subst), Ty::Bool);
        assert_eq!(test.param.ty(&subst), Ty::String);
        assert_eq!(Shape::K.ty(&subst), Ty::String);
        assert_eq!(Shape::V.ty(&subst), Ty::Int(IntKind::I32));
        assert_eq!(Shape::E.ty(&subst), Ty::Bool);
        assert_eq!(Shape::Bool.ty(&subst), Ty::Bool);
        assert_eq!(
            Shape::OptionOfV.ty(&subst),
            Ty::Option(boxed(Ty::Int(IntKind::I32)))
        );
        assert_eq!(
            Shape::ResultOfTAndE.ty(&subst),
            Ty::Result(boxed(Ty::String), boxed(Ty::Bool))
        );
        assert_eq!(
            Shape::ResultOfExpected.ty(&subst),
            Ty::Result(boxed(Ty::Int(IntKind::U8)), boxed(Ty::Error))
        );
        assert_eq!(Shape::Error.ty(&subst), Ty::Error);
        assert_eq!(Shape::ReadString.ty(&subst), Ty::String);
        assert_eq!(Shape::ReadT.ty(&subst), Ty::String);
        assert_eq!(
            Shape::HashMapOfKV.ty(&subst),
            Ty::HashMap(boxed(Ty::String), boxed(Ty::Int(IntKind::I32)))
        );
    }

    #[test]
    fn the_error_rows_of_m5a_spec_2_3() {
        let new = lookup(Owner::Error, "new", false).expect("Error::new");
        assert_eq!(new.path(), "Error::new");
        assert_eq!(new.get().params, [Shape::String]);
        assert_eq!(new.get().result, Shape::Error);
        assert_eq!(new.modes(), [ParamMode::Owned]);
        let message = row(Owner::Error, "message", true);
        assert_eq!(message.result, Shape::String);
        assert_eq!(message.result_kind, ResultKind::Borrowed);
        assert_eq!(names(Owner::Error, true), ["message"]);
        assert_eq!(names(Owner::Error, false), ["new"]);
        let (owner, _) = Owner::of(&Ty::Error).expect("`Error` has rows");
        assert_eq!(owner, Owner::Error);
        assert!(new.get().uses_std());
        assert!(message.uses_std());
        assert!(row(Owner::String, "parse", true).uses_std());
        assert!(!row(Owner::String, "trim", true).uses_std());
    }

    #[test]
    fn the_json_rows_of_m5a_spec_2_4() {
        assert_eq!(names(Owner::Json, false), ["parse", "stringify"]);
        assert!(names(Owner::Json, true).is_empty());
        let parse = lookup(Owner::Json, "parse", false).expect("json::parse");
        assert_eq!(parse.path(), "json::parse");
        assert_eq!(parse.get().params, [Shape::ReadString]);
        assert_eq!(parse.get().result, Shape::ResultOfExpected);
        assert_eq!(parse.modes(), [ParamMode::SharedBorrow]);
        let stringify = lookup(Owner::Json, "stringify", false).expect("json::stringify");
        assert_eq!(stringify.get().params, [Shape::ReadT]);
        assert_eq!(stringify.get().result, Shape::String);
        assert_eq!(stringify.modes(), [ParamMode::SharedBorrow]);
        assert!(parse.get().uses_std() && stringify.get().uses_std());
        assert_eq!(Owner::Json.name(), "json");
    }

    #[test]
    fn the_env_row_of_m5a_spec_2_5() {
        assert_eq!(names(Owner::Env, false), ["parse"]);
        let parse = lookup(Owner::Env, "parse", false).expect("env::parse");
        assert_eq!(parse.path(), "env::parse");
        assert!(parse.get().params.is_empty());
        assert_eq!(parse.get().result, Shape::ResultOfExpected);
        assert!(parse.get().uses_std());
        assert_eq!(Owner::Env.name(), "env");
    }

    #[test]
    fn the_shared_rows_of_milestone_5b1_spec_2_6() {
        assert_eq!(names(Owner::Shared, false), ["new"]);
        assert_eq!(names(Owner::Shared, true), ["clone"]);
        let new = lookup(Owner::Shared, "new", false).expect("Shared::new");
        assert_eq!(new.path(), "Shared::new");
        assert_eq!(new.modes(), [ParamMode::Owned]);
        let clone = lookup(Owner::Shared, "clone", true).expect("clone on Shared");
        assert_eq!(clone.modes(), [ParamMode::SharedBorrow]);
        let subst = Subst {
            t: Ty::Bool,
            ..Subst::default()
        };
        for id in [new, clone] {
            assert_eq!(id.get().result.ty(&subst), Ty::Shared(Box::new(Ty::Bool)));
            // `Shared` is std's `Arc`.
            assert!(!id.get().uses_std() && !id.get().is_async);
        }
    }

    #[test]
    fn the_task_rows_of_milestone_5b1_spec_2_5() {
        assert_eq!(names(Owner::Task, false), ["all", "all_settled"]);
        assert_eq!(names(Owner::Task, true), ["detach"]);
        for name in ["all", "all_settled"] {
            let id = lookup(Owner::Task, name, false).expect("a Task row");
            assert_eq!(id.path(), format!("Task::{name}"));
            assert!(id.get().is_async && id.get().uses_std());
            assert_eq!(id.modes(), [ParamMode::Owned]);
            let subst = Subst {
                t: Ty::Bool,
                ..Subst::default()
            };
            assert_eq!(
                id.get().params[0].ty(&subst),
                Ty::Vec(Box::new(Ty::Task(Box::new(Ty::Bool))))
            );
        }
    }

    #[test]
    fn element_rules_accept_what_spec_2_7_says() {
        let (i32_, f64_) = (Ty::Int(IntKind::I32), Ty::Float(FloatKind::F64));
        let s = Ty::Struct(crate::resolve::StructId(0));
        assert!(ElementRule::Comparable.accepts(&f64_));
        assert!(ElementRule::Comparable.accepts(&Ty::String));
        assert!(!ElementRule::Comparable.accepts(&s));
        assert!(ElementRule::Ordered.accepts(&i32_));
        assert!(ElementRule::Ordered.accepts(&Ty::Bool));
        assert!(!ElementRule::Ordered.accepts(&f64_));
        assert!(ElementRule::StringOnly.accepts(&Ty::String));
        assert!(!ElementRule::StringOnly.accepts(&i32_));
        assert!(ElementRule::Number.accepts(&f64_));
        assert!(!ElementRule::Number.accepts(&Ty::Bool));
        assert!(ElementRule::Any.accepts(&s));
    }
}
