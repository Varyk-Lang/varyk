//! The typed HIR (spec section 6.3): the type checker's output and the
//! input of borrow analysis (`borrow`) and the backend (`backend`).
//!
//! The HIR records decisions, not syntax: every expression carries its
//! [`Ty`], every call its resolved [`Callee`], every parameter its
//! [`ParamMode`], and every binding a [`LocalId`]. Every node keeps the
//! [`Span`] of the syntax it came from, because later passes point
//! diagnostics and fix-its at it. Integer and float literals keep their
//! source text and are never value-parsed: rustc enforces their range.

use std::path::PathBuf;

use varyk_syntax::{BinaryOp, Span, UnaryOp};

use crate::builtins::{BuiltinId, ResultKind};
use crate::resolve::{
    Callee, DropCause, EnumId, FieldDef, FnId, ImportedFnId, ImportedSig, ModuleId, StructId,
    UserType, VariantDef,
};
pub use crate::resolve::{FieldAttrs, HirDefault};
use crate::types::{Derives, ParamMode, Serde, Ty};

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
    /// Whether the program uses `varyk-std` (M5a spec 1): it makes a
    /// `varyk-std` call or names `Error`, so its crate depends on it.
    pub uses_std: bool,
    /// Whether the program makes a `log` call, so its `main` starts
    /// logging (M5a spec 2.6, 7.5).
    pub logs: bool,
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

    /// The Rust path of module `id` from the crate root: `crate` for the
    /// entry module, `crate::shop::cart` for a nested one.
    pub fn module_path(&self, id: ModuleId) -> String {
        let mut names = Vec::new();
        let mut current = &self.modules[id.0 as usize];
        while let Some(parent) = current.parent {
            names.push(current.name.as_str());
            current = &self.modules[parent.0 as usize];
        }
        names.push("crate");
        names.reverse();
        names.join("::")
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct HirModule {
    pub id: ModuleId,
    /// The `mod` name, or `resolve::ENTRY_MODULE_NAME` for the entry.
    pub name: String,
    /// The module that declares this one; `None` for the entry.
    pub parent: Option<ModuleId>,
    /// Declared `pub mod`.
    pub is_pub: bool,
    /// The generated file's path under `src/`, mirroring the module tree
    /// (spec 2.3): `main.rs`, `shop.rs`, `shop/mod.rs`, `shop/cart.rs`.
    pub path: String,
    /// This module's own `use` declarations (spec 3.3), in source order,
    /// each already rewritten to the canonical `crate::`-rooted path of
    /// what it names, so the backend emits them verbatim.
    pub uses: Vec<HirUse>,
    pub kind: HirModuleKind,
}

/// `use path;` or `use path as alias;` (spec 3.3), ready for emission:
/// `path` is the canonical `crate::`-rooted path to whatever the `use`
/// names, the same form the backend writes for the item elsewhere, so the
/// generated line resolves regardless of how the user wrote it.
#[derive(Debug, Clone, PartialEq)]
pub struct HirUse {
    pub path: String,
    /// `Some` only when the user wrote `as`; the local name is otherwise
    /// already `path`'s last segment.
    pub alias: Option<String>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum HirModuleKind {
    /// A `.vr` module; its functions are in [`HirProgram::functions`].
    Varyk,
    /// A `.rs` module's source text, copied verbatim into the generated
    /// crate, and the user's file it was read from (spec 2.3: the backend
    /// copies this path into [`crate::backend::GeneratedCrate::copied`]
    /// rather than reading `source` again).
    Rust { source: String, path: PathBuf },
}

#[derive(Debug, Clone, PartialEq)]
pub struct HirStruct {
    pub name: String,
    pub module: ModuleId,
    pub is_pub: bool,
    /// Imported from a `.rs` module (M3 spec 4.1): the backend emits no
    /// definition for it.
    pub imported: bool,
    /// In declaration order; a field index is a position in this list.
    pub fields: Vec<FieldDef>,
    /// Which traits it derives (M4 spec 2.10), or, imported, lists.
    pub derives: Derives,
    /// The ways a `json` call converts it (M5a spec 2.4): what it derives
    /// of serde's `Serialize` and `Deserialize`.
    pub serde: Serde,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HirEnum {
    pub name: String,
    pub module: ModuleId,
    pub is_pub: bool,
    /// Imported from a `.rs` module (M3 spec 4.3): the backend emits no
    /// definition for it.
    pub imported: bool,
    /// Each variant, in declaration order.
    pub variants: Vec<VariantDef>,
    /// Has, or may have, an `impl Drop` in a `.rs` module (see
    /// `EnumDef::drops`).
    pub drops: Option<DropCause>,
    /// Which traits it derives (M4 spec 2.10), or, imported, lists.
    pub derives: Derives,
    /// The ways a `json` call converts it (M5a spec 2.4).
    pub serde: Serde,
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
    /// Marked `#[test]` (M5a spec 2.7).
    pub is_test: bool,
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
    /// The parameter (`self` included) a borrowed return is part of (M4
    /// spec 3.1): the backend writes `-> &str` or `-> &T`. The type checker
    /// leaves `None`; `borrow::analyze` fills it in. For a function whose
    /// returns borrow analysis rejects (V0304, V0308) it names the
    /// parameter a rejected return is part of, so the Rust written for the
    /// rejected program is the borrowed return rustc refuses; callers see
    /// such a function as returning something new.
    pub ret_root: Option<LocalId>,
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
    /// - a pattern binding or a `for` variable: `RefOwned` on a place (the
    ///   `String` inside what is matched on or looped over), `Owned` on a
    ///   temporary; `Str` for one a pattern on a string binds, whatever the
    ///   head (M4 spec 3.5).
    pub repr: Option<StringRepr>,
    /// What bound it: a plain local, or a closure's parameter.
    pub kind: LocalKind,
}

/// What bound a local.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalKind {
    /// A parameter, `let`, pattern binding, or `for` variable.
    Plain,
    /// The parameter of a closure (M4 spec 2.2, 3.2). `deref` says the
    /// generated Rust hands it one reference deeper than its Varyk kind,
    /// so the closure starts with `let x = *x;`: written by borrow
    /// analysis, as `repr` is, and read by the backend.
    ClosureParam { deref: bool },
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
    /// A name of the function around a closure, used inside it (M4 spec
    /// 3.2): the closure only borrows it.
    Captured(LocalId),
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
    /// `while let pattern = value { body }` (M4 spec 2.4): `value` is
    /// evaluated anew each round, as `match`'s head, and the pattern's
    /// bindings live only in `body`. `head_is_str` as on
    /// [`HirExprKind::Match`].
    WhileLet {
        pattern: HirPattern,
        value: HirExpr,
        body: HirBlock,
        head_is_str: bool,
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
    /// `start..end` (`end` excluded) or `start..=end` (`inclusive`): two
    /// integers of one type.
    Range {
        start: Box<HirExpr>,
        end: Box<HirExpr>,
        inclusive: bool,
    },
    /// A `Vec`: a place, which the loop only looks at, for the whole loop;
    /// or a temporary, which the loop owns and uses up.
    Vec(HirExpr),
    /// A chain, one item per round (M4 spec 2.3, 3.3); the body may not
    /// change or give away anything the head reads.
    Chain(HirExpr),
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
    /// `rooted` as on [`HirExprKind::MethodCall`], set by borrow analysis
    /// for a call of a function with a borrowed return (M4 spec 3.1).
    Call {
        callee: Callee,
        args: Vec<HirExpr>,
        rooted: Option<usize>,
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
    /// `log::debug`, `info`, `warn`, or `error` (M5a spec 2.6), checked
    /// like `println!`: `level` is the call's name.
    Log {
        level: &'static str,
        format: String,
        args: Vec<HirExpr>,
    },
    /// `assert(cond)` or `assert_eq(a, b)` in a test (M5a spec 2.7), of
    /// type `()`. For `assert_eq`, `cond` is the checked `a == b`.
    /// `location` is the call's Varyk file and line (`src/store.vr:12`),
    /// filled here since the backend sees only spans.
    Assert {
        cond: Box<HirExpr>,
        location: String,
        kind: AssertKind,
    },
    /// `format!`, checked like `println!`: a new `string`.
    Format {
        format: String,
        args: Vec<HirExpr>,
    },
    /// A variant value (`Shape::Circle(r)`, `Shape::Point`) or a built-in
    /// constructor (`Some(x)`, `None`, `Ok(x)`, `Err(e)`), with its
    /// payload in order; every payload is an owned slot (spec 3.4). A
    /// variant with named fields (`Event::Click { x: 1, y: 2 }`, M4 spec
    /// 2.5) has its values in source order, which is evaluation order, and
    /// `fields` gives each one's position in the declaration; `fields` is
    /// `None` for a variant with values by position.
    ///
    /// `write_full_type` is set for `Ok`, `Err`, and `None` that are the
    /// direct operand of `?`: the backend writes their type arguments
    /// (`Ok::<T, E>(x)?`), since rustc cannot infer them there.
    EnumLit {
        variant: VariantRef,
        args: Vec<HirExpr>,
        fields: Option<Vec<usize>>,
        write_full_type: bool,
    },
    /// `vec![..]`: its elements, each an owned slot (spec 2.9).
    VecLit(Vec<HirExpr>),
    /// `receiver.method(args)` (spec 2.5, 2.6): the receiver is an argument
    /// passed the way the method's `self` (or the table's receiver mode)
    /// says. A Varyk associated function or `Vec::new()` is a `Call`.
    ///
    /// `rooted` is the argument position (0 for the receiver) whose roots
    /// the result borrows (M4 spec 2.8, 3.1, 5): the call is then a place
    /// of that argument, as a field is of its base. The checker sets it on
    /// a looked-into row whose payload is not Copy and on a `Borrowed` row;
    /// borrow analysis sets it on a call of a method with a borrowed
    /// return.
    ///
    /// `looked_into` is set by borrow analysis on a `find` whose chain's
    /// items are borrowed (M4 spec 2.8, 3.3): its result holds part of
    /// what the chain's source reads, and must be looked inside where it
    /// is made, as a looked-into row with `rooted` must; the checker
    /// leaves it unset.
    MethodCall {
        receiver: Box<HirExpr>,
        method: MethodRef,
        args: Vec<HirExpr>,
        rooted: Option<usize>,
        looked_into: bool,
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
    ///
    /// `head_is_str` is set for every head of type `string` (M4 spec 3.5,
    /// 5): the backend writes it as exactly a `&str`, and every name its
    /// patterns bind to a `string` is a `&str` borrowed from it.
    Match {
        scrutinee: Box<HirExpr>,
        arms: Vec<HirArm>,
        head_is_str: bool,
    },
    /// `if let pattern = value { then } else { else_ }` (M4 spec 2.4): an
    /// expression typed like `if`, its `value` a head following the rules
    /// of `match`'s, and the pattern's bindings living only in `then`.
    IfLet {
        pattern: Box<HirPattern>,
        value: Box<HirExpr>,
        then: HirBlock,
        else_: Option<HirBlock>,
        head_is_str: bool,
    },
    /// `operand?` (spec 2.8, M4 spec 2.6): `operand` is a `Result` with the
    /// function's error type or an `Option` in a function returning
    /// `Option` (`kind`), an owned slot (spec 3.4); the value is its `Ok` or
    /// `Some` payload, and otherwise the function returns at once.
    Try {
        operand: Box<HirExpr>,
        kind: TryKind,
    },
    /// `expr as T` (M4 spec 2.9): a conversion between number types; the
    /// target is the expression's type.
    Cast {
        expr: Box<HirExpr>,
        ty: Ty,
    },
    /// `|param| body` (M4 spec 2.2), only ever the argument of a table row
    /// whose parameter is a closure; its type is what the body gives.
    /// `captures` are the locals of the function around it that the body
    /// uses, in order of first use: a local declared outside the closure's
    /// span, which the closure borrows (M4 spec 3.2). A body written as an
    /// expression is a block with that expression as its tail.
    ///
    /// `returns_part` is set by borrow analysis on the closure of a chain's
    /// `map` whose value is part of its parameter or of a captured name
    /// (M4 spec 3.2, 3.3): the backend writes that value as a reference,
    /// as it writes a borrowed return. The checker leaves it unset.
    Closure {
        param: LocalId,
        captures: Vec<LocalId>,
        body: HirBlock,
        returns_part: bool,
    },
}

/// Which check an [`HirExprKind::Assert`] is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssertKind {
    /// `assert(cond)`.
    Plain,
    /// `assert_eq(a, b)`; `show` when the values print with `{}`, so a
    /// failure shows them.
    Eq { show: bool },
}

/// Which type a `?` works on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TryKind {
    Option,
    Result,
}

/// One arm of a `match`: its pattern's bindings live only in its body.
#[derive(Debug, Clone, PartialEq)]
pub struct HirArm {
    pub pattern: HirPattern,
    pub body: HirExpr,
    pub span: Span,
}

/// A pattern (spec 2.3, M4 spec 2.5), typed against the value it looks
/// at and nested to any depth.
#[derive(Debug, Clone, PartialEq)]
pub enum HirPattern {
    /// `_`: matches anything, binds nothing.
    Wildcard,
    /// A name: matches anything and binds it.
    Binding(LocalId),
    /// A variant with values by position, or none: each position's own
    /// pattern.
    Variant {
        variant: VariantRef,
        fields: Vec<HirPattern>,
    },
    /// A variant with named fields: every field once, in declaration
    /// order, with its pattern.
    Struct {
        variant: VariantRef,
        fields: Vec<(String, HirPattern)>,
    },
    /// An integer, a string (only as the whole pattern), or a `bool`.
    Literal(HirLiteral),
    /// `lo..=hi`, both ends included, `lo <= hi`.
    Range { lo: i128, hi: i128 },
}

/// The value of a literal pattern.
#[derive(Debug, Clone, PartialEq)]
pub enum HirLiteral {
    Int(i128),
    /// Raw text between the quotes, escapes not yet resolved.
    Str(String),
    Bool(bool),
}

impl HirPattern {
    /// The locals the pattern binds at any depth, each with whether it
    /// names the whole value (a catch-all name) rather than a part of it.
    pub fn bindings(&self) -> Vec<(LocalId, bool)> {
        fn collect(pattern: &HirPattern, whole: bool, out: &mut Vec<(LocalId, bool)>) {
            match pattern {
                HirPattern::Wildcard | HirPattern::Literal(_) | HirPattern::Range { .. } => {}
                HirPattern::Binding(local) => out.push((*local, whole)),
                HirPattern::Variant { fields, .. } => {
                    for field in fields {
                        collect(field, false, out);
                    }
                }
                HirPattern::Struct { fields, .. } => {
                    for (_, field) in fields {
                        collect(field, false, out);
                    }
                }
            }
        }
        let mut out = Vec::new();
        collect(self, true, &mut out);
        out
    }
}

/// The method a [`HirExprKind::MethodCall`] calls: a Varyk method, whose
/// first parameter is its receiver, a method of an imported Rust struct,
/// whose receiver mode is [`ImportedSig::modes`]'s first (M3 spec 4.2),
/// or a row of the built-in table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MethodRef {
    Varyk(FnId),
    Imported(ImportedFnId),
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
/// through blocks, `if` and `if let` branches, and `match` arms, their
/// tails. A block
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
            HirExprKind::IfLet { then, else_, .. } => {
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
/// spec 3.1), through a call with `rooted` to the argument it borrows
/// from (`v` in `v.get(0)`, M4 spec 2.8); `expr` itself for anything else.
pub(crate) fn field_root(expr: &HirExpr) -> &HirExpr {
    match &expr.kind {
        HirExprKind::Field { base, .. } | HirExprKind::Index { base, .. } => field_root(base),
        _ => match rooted_argument(expr) {
            Some(argument) => field_root(argument),
            None => expr,
        },
    }
}

/// The argument a call with `rooted` borrows its result from: its
/// receiver, or one of its arguments; `None` for anything else.
pub(crate) fn rooted_argument(expr: &HirExpr) -> Option<&HirExpr> {
    match &expr.kind {
        HirExprKind::MethodCall {
            rooted: Some(0),
            receiver,
            ..
        } => Some(receiver),
        HirExprKind::MethodCall {
            rooted: Some(index),
            args,
            ..
        } => args.get(index - 1),
        HirExprKind::Call {
            rooted: Some(index),
            args,
            ..
        } => args.get(*index),
        _ => None,
    }
}

/// Whether `expr` is a call of a looked-into row whose result holds part
/// of its receiver (M4 spec 2.8): an `Option` Varyk has no spelling for,
/// which must be looked inside right where it is made.
pub(crate) fn is_looked_into(expr: &HirExpr) -> bool {
    match &expr.kind {
        HirExprKind::MethodCall {
            looked_into: true, ..
        } => true,
        HirExprKind::MethodCall {
            method: MethodRef::Builtin(id),
            rooted: Some(_),
            ..
        } => id.get().result_kind == ResultKind::LookInside,
        _ => false,
    }
}

/// Whether `expr` is an `if`, an `if let`, a block, or a `match`: a
/// value made of the values of its branches (see [`leaves`]).
pub(crate) fn is_block_like(expr: &HirExpr) -> bool {
    matches!(
        expr.kind,
        HirExprKind::If { .. }
            | HirExprKind::IfLet { .. }
            | HirExprKind::Block(_)
            | HirExprKind::Match { .. }
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

/// [`is_place`], or a call with `rooted` whose argument is one: the
/// result is part of that argument (M4 spec 2.8); or a looked-into `find`,
/// part of what its chain's source reads (M4 spec 3.3). Borrow analysis
/// and the backend decide place against temporary with this; the checker
/// keeps [`is_place`].
pub(crate) fn is_place_or_rooted(expr: &HirExpr) -> bool {
    match &expr.kind {
        HirExprKind::Local(_)
        | HirExprKind::MethodCall {
            looked_into: true, ..
        } => true,
        HirExprKind::Field { base, .. } | HirExprKind::Index { base, .. } => {
            is_place_or_rooted(base)
        }
        _ => rooted_argument(expr).is_some_and(is_place_or_rooted),
    }
}

/// Whether a `match`, `if let`, or `while let` on `head` only looks
/// inside it, its bindings aliases of its parts, rather than owning it:
/// `head` is a place (or a call rooted at one), or a temporary with an
/// enum that has a destructor anywhere inside its type (at the top, or
/// inside an `Option`, a `Result`, or an enum's payloads), whose payload
/// Rust cannot move out at any depth (E0509).
pub(crate) fn matched_in_place(enums: &[HirEnum], head: &HirExpr) -> bool {
    is_place_or_rooted(head) || dropping_enum(enums, &head.ty).is_some()
}

/// The first enum with a destructor in `ty`, looking through `Option`,
/// `Result`, and enum payloads (M4 spec 5).
pub(crate) fn dropping_enum(enums: &[HirEnum], ty: &Ty) -> Option<EnumId> {
    fn find(enums: &[HirEnum], ty: &Ty, seen: &mut Vec<EnumId>) -> Option<EnumId> {
        match ty {
            Ty::Option(inner) => find(enums, inner, seen),
            Ty::Result(ok, err) => find(enums, ok, seen).or_else(|| find(enums, err, seen)),
            // An enum holding itself is looked into once.
            Ty::Enum(id) if !seen.contains(id) => {
                seen.push(*id);
                let def = &enums[id.0 as usize];
                if def.drops.is_some() {
                    return Some(*id);
                }
                def.variants
                    .iter()
                    .flat_map(VariantDef::types)
                    .find_map(|ty| find(enums, ty, seen))
            }
            _ => None,
        }
    }
    find(enums, ty, &mut Vec::new())
}

/// Whether `local` is declared inside `within` (an expression or a
/// block), so that it is gone once `within` has been evaluated.
pub(crate) fn declared_inside(local: &LocalInfo, within: Span) -> bool {
    within.start <= local.span.start && local.span.end <= within.end
}
