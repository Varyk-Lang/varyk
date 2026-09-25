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

/// A named type: `string`, `i32`, `User`, and so on are all just names,
/// resolved later by the compiler's resolver. A type may carry an optional
/// module segment (`m::User`, spec 2.10) and generic arguments (`Vec<i32>`,
/// `Result<User, string>`, nested freely, spec 2.6); the parser only records
/// them, and the resolver checks that the module, the name, and the argument
/// count exist.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeExpr {
    pub module: Option<Ident>,
    pub name: Ident,
    pub args: Vec<TypeExpr>,
    pub span: Span,
}

impl TypeExpr {
    /// Renders the whole type back to source text: the module segment (if
    /// any), the name, and any generic arguments, recursively. Used
    /// wherever a diagnostic or fix-it needs to show the type as written,
    /// rather than just its base name.
    pub fn display_name(&self) -> String {
        let mut out = String::new();
        if let Some(module) = &self.module {
            out.push_str(&module.name);
            out.push_str("::");
        }
        out.push_str(&self.name.name);
        if !self.args.is_empty() {
            out.push('<');
            for (i, arg) in self.args.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                out.push_str(&arg.display_name());
            }
            out.push('>');
        }
        out
    }
}

/// A whole parsed source file: the top-level items it declares, in order.
#[derive(Debug, Clone, PartialEq)]
pub struct Program {
    pub items: Vec<Item>,
}

/// A top-level declaration (spec 4.1, 2.2, 2.5): a function, a struct, an
/// enum, an `impl` block, or a module declaration. `mod name;` only
/// declares a submodule; resolving it to the submodule's own file and items
/// is the compiler's `resolve` module's job.
#[derive(Debug, Clone, PartialEq)]
pub enum Item {
    Function(Function),
    Struct(StructDecl),
    Enum(EnumDecl),
    Impl(ImplBlock),
    Mod(ModDecl),
}

/// Whether a function is a plain function, or a method with a `self`
/// receiver (spec 2.5): `self` is a shared borrow, `mut self` a mutable
/// borrow. There is no owned `self` in milestone 2.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelfMode {
    None,
    Shared,
    Mutable,
}

/// `fn name(params) -> ReturnType { body }`, optionally `pub`. Inside an
/// `impl` block the parameter list may start with a `self` receiver
/// (`self_mode`), which is never a [`Param`] (spec 2.5).
#[derive(Debug, Clone, PartialEq)]
pub struct Function {
    pub name: Ident,
    pub is_pub: bool,
    pub self_mode: SelfMode,
    /// The `self` keyword of the receiver, where a `mut ` fix-it inserts;
    /// `None` exactly when `self_mode` is [`SelfMode::None`].
    pub self_span: Option<Span>,
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

/// `enum Name { variants... }` (spec 2.2), optionally `pub`. An enum has one
/// or more variants; an empty `{ }` is `V0002`, since the parser found a
/// complete-looking declaration missing the one thing it must have.
#[derive(Debug, Clone, PartialEq)]
pub struct EnumDecl {
    pub name: Ident,
    pub is_pub: bool,
    pub variants: Vec<EnumVariant>,
    pub span: Span,
}

/// A unit variant (`Point`) or a tuple variant (`Circle(f64)`); `fields` is
/// empty for a unit variant. Named-field variants (`Circle { x: f64 }`) are
/// milestone 4 and never produce one of these: they are `V0001`.
#[derive(Debug, Clone, PartialEq)]
pub struct EnumVariant {
    pub name: Ident,
    pub fields: Vec<TypeExpr>,
    pub span: Span,
}

/// `impl Name { fns... }` (spec 2.5): a block of functions and methods for
/// a struct or enum declared in the same file. Whether `name` actually
/// names a struct or enum is the compiler's `resolve` module's job.
#[derive(Debug, Clone, PartialEq)]
pub struct ImplBlock {
    pub type_name: Ident,
    pub functions: Vec<Function>,
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
    /// `for var in head { body }` (spec 2.4). `head` is either a half-open
    /// range or a `Vec` iterated element by element; the loop variable's
    /// type and whether the `Vec` case borrows or owns each element is the
    /// type checker's job (task 8).
    For {
        var: Ident,
        head: ForHead,
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

/// The head of a `for` loop (spec 2.4): a half-open integer range
/// (`a..b`), parsed only here since ranges exist nowhere else in Varyk, or
/// an expression iterated element by element as a `Vec`. `..=` never
/// produces a [`ForHead::Range`]: it is `V0001`, milestone 4.
#[derive(Debug, Clone, PartialEq)]
pub enum ForHead {
    Range { start: Box<Expr>, end: Box<Expr> },
    Expr(Box<Expr>),
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

    /// `name`, `module::name`, or `module::Type::name` (spec 2.5, 2.10): a
    /// plain name, a module-qualified one, or a module-and-type-qualified
    /// one (an associated function or enum variant reached through a
    /// module). A two-segment path fills `module` only; a three-segment
    /// one fills both `module` and `type_`. A path longer than three
    /// segments is rejected with `V0001` (still built from its first two
    /// segments and its last, so later passes have something to work
    /// with). Resolving `type_` is milestone 2 tasks 6 and 7's job; until
    /// then the compiler rejects it with `V0001` rather than silently
    /// mis-resolving it.
    Path {
        module: Option<Ident>,
        type_: Option<Ident>,
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

    /// `callee(args...)`. `callee` must be a `Path`, naming a plain
    /// function or an associated function (spec 2.5); `x.f(args)` is
    /// method-call syntax and parses as [`ExprKind::MethodCall`] instead,
    /// never as a `Call` with a `Field` callee. Any other callee shape
    /// (calling the result of a grouped expression, an index, and so on)
    /// is rejected: `V0002` from the parser, and defensively `V0001` from
    /// the type checker if such a node ever reaches it.
    Call {
        callee: Box<Expr>,
        args: Vec<Expr>,
    },

    Field {
        base: Box<Expr>,
        name: Ident,
    },

    /// `receiver.method(args...)` (spec 2.5): a method call, `value.name(args)`.
    MethodCall {
        receiver: Box<Expr>,
        method: Ident,
        args: Vec<Expr>,
    },

    /// `base[index]` (spec 2.6): indexing, `v[i]`. Whether `base` is
    /// actually a `Vec`, and whether `index` is a `usize`, is the type
    /// checker's job.
    Index {
        base: Box<Expr>,
        index: Box<Expr>,
    },

    /// `operand?` (spec 2.8): propagates an `Err` out of the enclosing
    /// function. Whether `operand` is actually a `Result` compatible with
    /// the function's return type is the type checker's job.
    Try {
        operand: Box<Expr>,
    },

    /// `match scrutinee { arms... }` (spec 2.3), an expression like `if`.
    /// It may also stand as a statement without a trailing `;`, the same
    /// as `if` and a bare block, distinguished the same way by
    /// `Stmt::Expr`'s `has_semi`.
    Match {
        scrutinee: Box<Expr>,
        arms: Vec<MatchArm>,
    },

    /// `vec![a, b, c]` (spec 2.9), a compiler intrinsic like `println!`,
    /// not a macro system: its elements, each an owned slot.
    VecLit(Vec<Expr>),

    /// `Name { field: expr, ... }`, or `module::Name { field: expr, ... }`
    /// (spec 2.10). Never parsed in condition position.
    StructLit {
        module: Option<Ident>,
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

    /// `println!(format, args...)` or `format!(format, args...)` (spec
    /// 2.9): compiler intrinsics sharing one shape, told apart by `name`.
    /// `format` keeps the format string's raw text and span; placeholder
    /// checking is the type checker's job.
    Intrinsic {
        name: Ident,
        format: (String, Span),
        args: Vec<Expr>,
    },
}

/// One arm of a `match` (spec 2.3): `pattern => body`, with an optional
/// trailing comma the parser consumes but does not record.
#[derive(Debug, Clone, PartialEq)]
pub struct MatchArm {
    pub pattern: Pattern,
    pub body: Expr,
    pub span: Span,
}

/// A `match` pattern, one level deep (spec 2.3). Not in milestone 2, each
/// `V0001`: nested patterns, literal patterns, guards (`pattern if cond`),
/// alternatives (`a | b`), rest patterns (`..`), range patterns, and `@`
/// bindings.
#[derive(Debug, Clone, PartialEq)]
pub enum Pattern {
    /// `_`: matches anything, binds nothing.
    Wildcard(Span),
    /// A bare name: matches anything and binds it. The parser cannot tell
    /// this apart from a zero-argument variant written without its type
    /// (`None`): the type checker (task 8) decides against the matched
    /// value's type.
    Name(Ident),
    /// A variant, optionally reached through a type and a module:
    /// `Point`, `Some(p)`, `Shape::Circle(p, q)`, `geo::Shape::Point`.
    Variant(VariantPattern),
}

impl Pattern {
    pub fn span(&self) -> Span {
        match self {
            Pattern::Wildcard(span) => *span,
            Pattern::Name(ident) => ident.span,
            Pattern::Variant(variant) => variant.span,
        }
    }
}

/// A variant pattern (spec 2.3): `name`, or `type_::name`, or
/// `module::type_::name`, each with an optional parenthesized list of
/// sub-patterns. A single remaining segment always fills `type_`, never
/// `module`: unlike [`ExprKind::Path`], a pattern never calls anything, so
/// there is no bare-module case to weigh it against. A pattern path longer
/// than three segments is `V0001`.
#[derive(Debug, Clone, PartialEq)]
pub struct VariantPattern {
    pub module: Option<Ident>,
    pub type_: Option<Ident>,
    pub name: Ident,
    pub subpatterns: Vec<SubPattern>,
    pub span: Span,
}

/// A sub-pattern inside a variant pattern's parentheses (spec 2.3): a name
/// or `_`. Nothing else is one level deep.
#[derive(Debug, Clone, PartialEq)]
pub enum SubPattern {
    Wildcard(Span),
    Name(Ident),
}
