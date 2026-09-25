//! The typed HIR (spec section 6.3): the type checker's output and the
//! input of borrow analysis (`borrow`) and the backend (`backend`).
//!
//! The HIR records decisions, not syntax: every expression carries its
//! [`Ty`], every call its resolved [`Callee`], every parameter its
//! [`ParamMode`], and every binding a [`LocalId`]. Every node keeps the
//! [`Span`] of the syntax it came from, because later passes point
//! diagnostics and fix-its at it. Integer and float literals keep their
//! source text and are never value-parsed: rustc enforces their range.

use varyk_syntax::{BinaryOp, Span, UnaryOp};

use crate::builtins::BuiltinId;
use crate::resolve::{Callee, EnumId, FnId, ImportedSig, ModuleId, StructId, UserType};
use crate::types::{ParamMode, Ty};

/// Index into [`HirFunction::locals`]. Parameters come first: parameter
/// `i` is always `LocalId(i)`, so a local is a parameter exactly when its
/// index is below `params.len()`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LocalId(pub u32);

/// The whole checked program.
#[derive(Debug, Clone, PartialEq)]
pub struct HirProgram {
    /// Indexed by `ModuleId`; the entry module is `ModuleId(0)`.
    pub modules: Vec<HirModule>,
    /// Every Varyk function of every module, indexed by `FnId`.
    pub functions: Vec<HirFunction>,
    /// Indexed by `StructId`.
    pub structs: Vec<HirStruct>,
    /// Indexed by `EnumId`.
    pub enums: Vec<HirEnum>,
    /// Indexed by `ImportedFnId`: every imported Rust function with its
    /// mapped modes, types, and original signature text.
    pub imported: Vec<ImportedSig>,
    pub entry: ModuleId,
}

impl HirProgram {
    /// Every Varyk function of every module, in `FnId` order.
    pub fn functions(&self) -> impl Iterator<Item = &HirFunction> {
        self.functions.iter()
    }

    /// The Varyk function with id `id`.
    pub fn function(&self, id: FnId) -> &HirFunction {
        &self.functions[id.0 as usize]
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct HirModule {
    pub id: ModuleId,
    /// The `mod` name, or `resolve::ENTRY_MODULE_NAME` for the entry.
    pub name: String,
    pub kind: HirModuleKind,
}

#[derive(Debug, Clone, PartialEq)]
pub enum HirModuleKind {
    /// A `.vr` module; its functions are in [`HirProgram::functions`].
    Varyk,
    /// A `.rs` module's source text, copied verbatim into the generated crate.
    Rust { source: String },
}

#[derive(Debug, Clone, PartialEq)]
pub struct HirStruct {
    pub name: String,
    pub module: ModuleId,
    pub is_pub: bool,
    /// In declaration order; a field index is a position in this list.
    pub fields: Vec<(String, Ty)>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HirEnum {
    pub name: String,
    pub module: ModuleId,
    pub is_pub: bool,
    /// Each variant's name and payload types, in declaration order.
    pub variants: Vec<(String, Vec<Ty>)>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HirFunction {
    pub id: FnId,
    /// The module declaring the function.
    pub module: ModuleId,
    /// The type whose `impl` block declares the function; `None` for a
    /// free function.
    pub owner: Option<UserType>,
    pub name: String,
    pub is_pub: bool,
    /// A method's receiver comes first, as `LocalId(0)` named `self`,
    /// with the `impl` type and the receiver's mode.
    pub params: Vec<HirParam>,
    /// [`Ty::Unit`] when no return type is written.
    pub ret: Ty,
    pub body: HirBlock,
    /// Every parameter, `let`, pattern binding, and `for` variable, indexed
    /// by [`LocalId`].
    pub locals: Vec<LocalInfo>,
    /// The whole declaration, starting at `fn` (or `pub`).
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HirParam {
    pub local: LocalId,
    pub name: String,
    pub ty: Ty,
    pub mode: ParamMode,
    /// The whole parameter, `mut` included when written.
    pub span: Span,
}

/// A parameter, `let` binding, `match` pattern binding, or `for`
/// variable. A later `let` of the same name shadows with a new `LocalId`.
#[derive(Debug, Clone, PartialEq)]
pub struct LocalInfo {
    pub name: String,
    pub ty: Ty,
    /// `mut` was written: a `mut` parameter or a `let mut`.
    pub mutable: bool,
    /// The span of the name, where a `mut ` fix-it inserts for a `let` or
    /// a parameter.
    pub span: Span,
    /// Whether the local is a borrowed place and a mutable place (spec
    /// 4.2). The type checker leaves the default; `borrow::analyze` fills
    /// it in. A field path's info derives from its root local plus the
    /// field's type: see `borrow::place_info`.
    pub place: PlaceInfo,
    /// The generated-Rust representation of a `string` local (spec 4.3);
    /// `None` for every other type. The type checker leaves `None`;
    /// `borrow::analyze` fills it in for every `string` local, and the
    /// backend reads it rather than deriving its own:
    ///
    /// - a parameter: `Str`, or `MutOwned` when `mut`;
    /// - a `let` that is not a borrowed place: `Owned` when inference says
    ///   it needs to be owned, else `Str`;
    /// - a `let` that is a borrowed place (a reference in the generated
    ///   Rust): `MutOwned` when it is a mutable place; else `Str` when it
    ///   refers to a non-`mut` parameter or its initializer may evaluate to
    ///   a `&str` (a literal or a `Str` local), so every branch agrees on
    ///   one type; else `RefOwned`;
    /// - a `match` pattern binding or a `for` variable: `RefOwned` on a
    ///   place (the `String` inside what is matched on or looped over),
    ///   `Owned` on a temporary.
    pub repr: Option<StringRepr>,
}

/// How a `string` local is represented in the generated Rust.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StringRepr {
    /// `&str`.
    Str,
    /// `String`.
    Owned,
    /// `&String`.
    RefOwned,
    /// `&mut String`.
    MutOwned,
}

/// A place's ownership facts (spec 4.2), computed by borrow analysis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PlaceInfo {
    /// A borrowed place: a non-Copy parameter, a non-Copy field, or a
    /// `let` initialized from one. It is a reference in the generated Rust
    /// and cannot flow into an owned slot.
    pub borrowed: bool,
    /// A mutable place: may be assigned to and passed to a `mut`
    /// parameter.
    pub mutable: bool,
    /// What a borrowed place derives from, for diagnostics; `None` exactly
    /// when not `borrowed`.
    pub origin: Option<Origin>,
}

/// What a borrowed place derives from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// A non-Copy parameter (its `LocalId`, always below `params.len()`).
    Param(LocalId),
    /// A non-Copy field of this struct.
    Struct(StructId),
    /// A non-Copy field of this local, which owns its value (a `let` that
    /// is not a borrowed place): the root of an alias made from it (spec
    /// 3.1).
    Local(LocalId),
}

#[derive(Debug, Clone, PartialEq)]
pub struct HirBlock {
    pub stmts: Vec<HirStmt>,
    pub tail: Option<Box<HirExpr>>,
    /// The tail's type, or [`Ty::Unit`] without one (or the expected type,
    /// for a block that always returns before its end).
    pub ty: Ty,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum HirStmt {
    /// `let [mut] name[: T] = value;`; the binding is `locals[local]`.
    Let {
        local: LocalId,
        value: HirExpr,
        span: Span,
    },
    /// `target = value;`. `target` is a `Local`, or a chain of `Field`s and
    /// `Index`es on one.
    Assign {
        target: HirExpr,
        value: HirExpr,
        span: Span,
    },
    Expr {
        expr: HirExpr,
        span: Span,
    },
    Return {
        value: Option<HirExpr>,
        span: Span,
    },
    While {
        cond: HirExpr,
        body: HirBlock,
        span: Span,
    },
    /// `for local in head { body }` (spec 2.4): `local` is bound afresh
    /// for each round and lives only in `body`.
    For {
        local: LocalId,
        head: HirForHead,
        body: HirBlock,
        span: Span,
    },
    Break {
        span: Span,
    },
    Continue {
        span: Span,
    },
}

/// What a `for` goes over (spec 2.4, 3.2).
#[derive(Debug, Clone, PartialEq)]
pub enum HirForHead {
    /// `start..end`: two integers of one type, `end` excluded.
    Range { start: HirExpr, end: HirExpr },
    /// A `Vec`: a place, which the loop only looks at, for the whole loop;
    /// or a temporary, which the loop owns and uses up.
    Vec(HirExpr),
}

#[derive(Debug, Clone, PartialEq)]
pub struct HirExpr {
    pub kind: HirExprKind,
    pub ty: Ty,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum HirExprKind {
    /// Raw digits, as written.
    Int(String),
    /// Raw `digits.digits`, as written.
    Float(String),
    Bool(bool),
    /// Raw text between the quotes, escapes not yet resolved.
    String(String),
    /// A parameter or `let` binding.
    Local(LocalId),
    Call {
        callee: Callee,
        args: Vec<HirExpr>,
    },
    Field {
        base: Box<HirExpr>,
        name: String,
    },
    /// Field values in source order (which is evaluation order), each with
    /// its index into `HirStruct::fields`; every field appears once.
    StructLit {
        id: StructId,
        fields: Vec<(usize, HirExpr)>,
    },
    Unary {
        op: UnaryOp,
        operand: Box<HirExpr>,
    },
    Binary {
        op: BinaryOp,
        lhs: Box<HirExpr>,
        rhs: Box<HirExpr>,
    },
    Block(HirBlock),
    If {
        cond: Box<HirExpr>,
        then: HirBlock,
        else_: Option<HirBlock>,
    },
    /// `println!`: the raw format text (placeholders already checked) and
    /// its arguments.
    Println {
        format: String,
        args: Vec<HirExpr>,
    },
    /// `format!`, checked like `println!`: a new `string`.
    Format {
        format: String,
        args: Vec<HirExpr>,
    },
    /// A variant value (`Shape::Circle(r)`, `Shape::Point`) or a built-in
    /// constructor (`Some(x)`, `None`, `Ok(x)`, `Err(e)`), with its
    /// payload in order; every payload is an owned slot (spec 3.4).
    EnumLit {
        variant: VariantRef,
        args: Vec<HirExpr>,
    },
    /// `vec![..]`: its elements, each an owned slot (spec 2.9).
    VecLit(Vec<HirExpr>),
    /// `receiver.method(args)` (spec 2.5, 2.6): the receiver is an argument
    /// passed the way the method's `self` (or the table's receiver mode)
    /// says. A Varyk associated function or `Vec::new()` is a `Call`.
    MethodCall {
        receiver: Box<HirExpr>,
        method: MethodRef,
        args: Vec<HirExpr>,
    },
    /// `base[index]`: an element of the `Vec` `base`, a place derived from
    /// `base` like a field (spec 2.6).
    Index {
        base: Box<HirExpr>,
        index: Box<HirExpr>,
    },
    /// `match scrutinee { arms }` (spec 2.3). The scrutinee is a place (a
    /// local, or a field or element of one), which the `match` only looks
    /// at, or a temporary, which it owns (spec 3.2).
    Match {
        scrutinee: Box<HirExpr>,
        arms: Vec<HirArm>,
    },
    /// `operand?` (spec 2.8): `operand` is a `Result` with the function's
    /// error type, an owned slot (spec 3.4); the value is its `Ok` payload,
    /// and on `Err` the function returns that error at once.
    Try(Box<HirExpr>),
}

/// One arm of a `match`: its pattern's bindings live only in its body.
#[derive(Debug, Clone, PartialEq)]
pub struct HirArm {
    pub pattern: HirPattern,
    pub body: HirExpr,
    pub span: Span,
}

/// A pattern, one level deep (spec 2.3), typed against the scrutinee.
#[derive(Debug, Clone, PartialEq)]
pub enum HirPattern {
    /// `_`: matches anything, binds nothing.
    Wildcard,
    /// A name: matches anything and binds the whole value.
    Binding(LocalId),
    /// A variant, with each position's binding, or `None` for `_`.
    Variant {
        variant: VariantRef,
        bindings: Vec<Option<LocalId>>,
    },
}

impl HirPattern {
    /// The locals the pattern binds, each with whether it names the whole
    /// value (a catch-all name) rather than a part of it.
    pub fn bindings(&self) -> Vec<(LocalId, bool)> {
        match self {
            HirPattern::Wildcard => Vec::new(),
            HirPattern::Binding(local) => vec![(*local, true)],
            HirPattern::Variant { bindings, .. } => bindings
                .iter()
                .flatten()
                .map(|local| (*local, false))
                .collect(),
        }
    }
}

/// The method a [`HirExprKind::MethodCall`] calls: a Varyk method, whose
/// first parameter is its receiver, or a row of the built-in table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MethodRef {
    Varyk(FnId),
    Builtin(BuiltinId),
}

/// A variant, named by a value or (from task 8) a pattern: a user enum's
/// variant by index into `HirEnum::variants`, or one of the variants of
/// `Option` and `Result`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VariantRef {
    User(EnumId, usize),
    Some,
    None,
    Ok,
    Err,
}

/// The expressions whose value `expr` evaluates to: `expr` itself, or,
/// through blocks, `if` branches, and `match` arms, their tails. A block
/// without a tail contributes nothing.
pub(crate) fn leaves(expr: &HirExpr) -> Vec<&HirExpr> {
    fn block_leaves<'a>(block: &'a HirBlock, out: &mut Vec<&'a HirExpr>) {
        if let Some(tail) = &block.tail {
            collect(tail, out);
        }
    }
    fn collect<'a>(expr: &'a HirExpr, out: &mut Vec<&'a HirExpr>) {
        match &expr.kind {
            HirExprKind::Block(block) => block_leaves(block, out),
            HirExprKind::If { then, else_, .. } => {
                block_leaves(then, out);
                if let Some(else_) = else_ {
                    block_leaves(else_, out);
                }
            }
            HirExprKind::Match { arms, .. } => {
                for arm in arms {
                    collect(&arm.body, out);
                }
            }
            _ => out.push(expr),
        }
    }
    let mut out = Vec::new();
    collect(expr, &mut out);
    out
}

/// The innermost base of a chain of fields and elements (`a` in `a.b[i].c`,
/// spec 3.1); `expr` itself for anything but a field access or an index.
pub(crate) fn field_root(expr: &HirExpr) -> &HirExpr {
    match &expr.kind {
        HirExprKind::Field { base, .. } | HirExprKind::Index { base, .. } => field_root(base),
        _ => expr,
    }
}

/// Whether `expr` is an `if`, a block, or a `match`: a value made of the
/// values of its branches (see [`leaves`]).
pub(crate) fn is_block_like(expr: &HirExpr) -> bool {
    matches!(
        expr.kind,
        HirExprKind::If { .. } | HirExprKind::Block(_) | HirExprKind::Match { .. }
    )
}

/// A local, or a field or element of a place.
pub(crate) fn is_place(expr: &HirExpr) -> bool {
    match &expr.kind {
        HirExprKind::Local(_) => true,
        HirExprKind::Field { base, .. } | HirExprKind::Index { base, .. } => is_place(base),
        _ => false,
    }
}

/// Whether `local` is declared inside `within` (an expression or a
/// block), so that it is gone once `within` has been evaluated.
pub(crate) fn declared_inside(local: &LocalInfo, within: Span) -> bool {
    within.start <= local.span.start && local.span.end <= within.end
}
