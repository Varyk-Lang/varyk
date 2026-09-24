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

use crate::resolve::{Callee, FnId, ImportedSig, ModuleId, StructId};
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
    /// Indexed by `StructId`.
    pub structs: Vec<HirStruct>,
    /// Indexed by `ImportedFnId`: every imported Rust function with its
    /// mapped modes, types, and original signature text.
    pub imported: Vec<ImportedSig>,
    pub entry: ModuleId,
}

impl HirProgram {
    /// Every Varyk function of every module, in module order.
    pub fn functions(&self) -> impl Iterator<Item = &HirFunction> {
        self.modules.iter().flat_map(|module| match &module.kind {
            HirModuleKind::Varyk { functions } => functions.as_slice(),
            HirModuleKind::Rust { .. } => &[],
        })
    }

    /// The Varyk function with id `id`.
    pub fn function(&self, id: FnId) -> &HirFunction {
        self.functions()
            .find(|function| function.id == id)
            .expect("every FnId names a lowered function")
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
    /// A `.vr` module's functions, in source order.
    Varyk { functions: Vec<HirFunction> },
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
pub struct HirFunction {
    pub id: FnId,
    pub name: String,
    pub is_pub: bool,
    pub params: Vec<HirParam>,
    /// [`Ty::Unit`] when no return type is written.
    pub ret: Ty,
    pub body: HirBlock,
    /// Every parameter and `let` binding, indexed by [`LocalId`].
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

/// A parameter or `let` binding. A later `let` of the same name shadows
/// with a new `LocalId`.
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
    /// `borrow::analyze` fills it in for every `string` local:
    ///
    /// - a parameter: `Borrowed` (`&str`), or `Owned` when `mut` (it is a
    ///   `&mut String`);
    /// - a `let` that is not a borrowed place: `Owned` (`String`) when
    ///   inference says it needs to be owned, else `Borrowed` (`&str`);
    /// - a `let` that is a borrowed place (a reference in the generated
    ///   Rust): the representation of what it refers to, `Owned` for a
    ///   field or a `mut` parameter (`&String` / `&mut String`) and
    ///   `Borrowed` for a non-`mut` parameter (`&str`).
    pub repr: Option<StringRepr>,
}

/// How a `string` local is represented in the generated Rust.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StringRepr {
    /// `&str`.
    Borrowed,
    /// `String`, or a reference to one for a borrowed place.
    Owned,
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
    /// `target = value;`. `target` is a `Local` or a `Field` chain on one.
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
    Break {
        span: Span,
    },
    Continue {
        span: Span,
    },
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
}

/// The expressions whose value `expr` evaluates to: `expr` itself, or,
/// through blocks and `if` branches, their tails. A block without a tail
/// contributes nothing.
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
            _ => out.push(expr),
        }
    }
    let mut out = Vec::new();
    collect(expr, &mut out);
    out
}

/// The innermost base of a field chain (`a` in `a.b.c`); `expr` itself
/// for anything but a field access.
pub(crate) fn field_root(expr: &HirExpr) -> &HirExpr {
    match &expr.kind {
        HirExprKind::Field { base, .. } => field_root(base),
        _ => expr,
    }
}

/// Whether `expr` is an `if` or a block.
pub(crate) fn is_block_like(expr: &HirExpr) -> bool {
    matches!(expr.kind, HirExprKind::If { .. } | HirExprKind::Block(_))
}

/// Whether `local` is declared inside `within` (an expression or a
/// block), so that it is gone once `within` has been evaluated.
pub(crate) fn declared_inside(local: &LocalInfo, within: Span) -> bool {
    within.start <= local.span.start && local.span.end <= within.end
}
