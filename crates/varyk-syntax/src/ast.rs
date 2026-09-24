//! The AST produced by the parser (spec section 6.3). Every node carries a
//! [`Span`], except [`Program`] and [`Item`], whose span is whichever
//! variant/item they hold.

use crate::span::Span;

/// A name plus the span it was written at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ident {
    pub name: String,
    pub span: Span,
}

/// A named type: `string`, `i32`, `User`, and so on are all just names in
/// milestone 1, resolved later by the compiler's type checker. Milestone 1
/// has no other type syntax (no references or generics).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeExpr {
    pub name: Ident,
    pub span: Span,
}

/// A whole parsed source file: the top-level items it declares, in order.
#[derive(Debug, Clone, PartialEq)]
pub struct Program {
    pub items: Vec<Item>,
}

/// A top-level declaration (spec 4.1): a function, a struct, or a module
/// declaration. `mod name;` only declares a submodule; resolving it to the
/// submodule's own file and items is the compiler's `resolve` module's job.
#[derive(Debug, Clone, PartialEq)]
pub enum Item {
    Function(Function),
    Struct(StructDecl),
    Mod(ModDecl),
}

/// `fn name(params) -> ReturnType { body }`, optionally `pub`.
#[derive(Debug, Clone, PartialEq)]
pub struct Function {
    pub name: Ident,
    pub is_pub: bool,
    pub params: Vec<Param>,
    pub return_type: Option<TypeExpr>,
    pub body: Block,
    pub span: Span,
}

/// One parameter: `name: T` is a shared borrow, `mut name: T` is a mutable
/// borrow (spec 4.2).
#[derive(Debug, Clone, PartialEq)]
pub struct Param {
    pub name: Ident,
    pub mutable: bool,
    pub ty: TypeExpr,
    pub span: Span,
}

/// `struct Name { fields... }`, optionally `pub` (which makes every field
/// public; field-level `pub` is milestone 2, spec 4.1).
#[derive(Debug, Clone, PartialEq)]
pub struct StructDecl {
    pub name: Ident,
    pub is_pub: bool,
    pub fields: Vec<FieldDecl>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FieldDecl {
    pub name: Ident,
    pub ty: TypeExpr,
    pub span: Span,
}

/// `mod name;` (spec 4.5): declares a submodule resolved to `name.vr` or
/// `name.rs` in the same directory. Resolving and merging the submodule's
/// items is the compiler's `resolve` module's job; this node only records
/// the declaration.
#[derive(Debug, Clone, PartialEq)]
pub struct ModDecl {
    pub name: Ident,
    pub is_pub: bool,
    pub span: Span,
}

/// A statement: `let`, assignment, `return`, `while`, `break`, `continue`,
/// or an expression used for its side effects (or as a block's tail,
/// distinguished by `has_semi` on [`Stmt::Expr`] so the type checker can tell a
/// mid-block statement from a block's value).
#[derive(Debug, Clone, PartialEq)]
pub enum Stmt {
    Let {
        name: Ident,
        mutable: bool,
        ty: Option<TypeExpr>,
        value: Expr,
        span: Span,
    },
    /// Assignment to a place expression: a path (a variable) or a field
    /// access. Anything else as the target is rejected (`V0002`) before
    /// this node is ever built.
    Assign {
        target: Expr,
        value: Expr,
        span: Span,
    },
    /// An expression statement. `has_semi` is false only for a block-like
    /// expression (`if`, a bare block) written without a trailing `;` in
    /// the middle of a block, not at its end.
    Expr {
        expr: Expr,
        span: Span,
        has_semi: bool,
    },
    Return {
        value: Option<Expr>,
        span: Span,
    },
    While {
        cond: Expr,
        body: Block,
        span: Span,
    },
    Break {
        span: Span,
    },
    Continue {
        span: Span,
    },
}

/// `{ stmts...; tail }`. The tail expression, if present, is the block's
/// value.
#[derive(Debug, Clone, PartialEq)]
pub struct Block {
    pub stmts: Vec<Stmt>,
    pub tail: Option<Box<Expr>>,
    pub span: Span,
}

/// Unary operators (spec 4.1). Unary binds tighter than any binary
/// operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    Neg,
    Not,
}

/// Binary operators (spec 4.1), in Rust's precedence order from loosest to
/// tightest: `||`, `&&`, the comparisons (non-associative), `+ -`, then
/// `* / %`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryOp {
    Or,
    And,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    Add,
    Sub,
    Mul,
    Div,
    Rem,
}

impl BinaryOp {
    /// The operator as written, the same in Varyk and Rust.
    pub fn as_str(self) -> &'static str {
        match self {
            BinaryOp::Or => "||",
            BinaryOp::And => "&&",
            BinaryOp::Eq => "==",
            BinaryOp::Ne => "!=",
            BinaryOp::Lt => "<",
            BinaryOp::Le => "<=",
            BinaryOp::Gt => ">",
            BinaryOp::Ge => ">=",
            BinaryOp::Add => "+",
            BinaryOp::Sub => "-",
            BinaryOp::Mul => "*",
            BinaryOp::Div => "/",
            BinaryOp::Rem => "%",
        }
    }
}

/// An expression node: its kind plus the span it spans in the source.
#[derive(Debug, Clone, PartialEq)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ExprKind {
    /// Raw digits, as lexed: the type checker decides the type from context.
    Integer(String),
    /// Raw `digits.digits`, as lexed.
    Float(String),
    Bool(bool),
    /// Raw text between the quotes, as written: escapes are copied unchanged
    /// into the generated Rust, which resolves them.
    String(String),

    /// `name` or `module::name`. A path longer than two segments is
    /// rejected with `V0001` (milestone 1 has no nested module paths); the
    /// parser still returns a `Path` built from the first and last
    /// segments so that later passes have something to work with.
    Path {
        module: Option<Ident>,
        name: Ident,
    },

    Unary {
        op: UnaryOp,
        operand: Box<Expr>,
    },

    Binary {
        op: BinaryOp,
        lhs: Box<Expr>,
        rhs: Box<Expr>,
    },

    /// `callee(args...)`. `callee` must be a `Path`; a `Field` callee is
    /// method-call syntax, which milestone 1 does not have, and is
    /// rejected with `V0001` (the `Call` node is still produced so parsing
    /// can continue).
    Call {
        callee: Box<Expr>,
        args: Vec<Expr>,
    },

    Field {
        base: Box<Expr>,
        name: Ident,
    },

    /// `Name { field: expr, ... }`. Never parsed in condition position.
    StructLit {
        name: Ident,
        fields: Vec<(Ident, Expr)>,
    },

    Block(Block),

    /// `if cond { ... }` or `if cond { ... } else { ... }`. `cond` is
    /// parsed in condition position, so no struct literal is parsed
    /// directly in it. An `else if` is represented as `else_` holding a
    /// synthetic block whose only content is the nested `If` as its tail
    /// expression.
    If {
        cond: Box<Expr>,
        then: Block,
        else_: Option<Block>,
    },

    /// `println!(format, args...)`, the one compiler intrinsic in
    /// milestone 1. `format` keeps the format string's raw text and span;
    /// placeholder checking is the type checker's job.
    Intrinsic {
        name: Ident,
        format: (String, Span),
        args: Vec<Expr>,
    },
}
