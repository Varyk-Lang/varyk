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

/// Where a path starts (spec 3.1, 3.3): one of the three keyword prefixes,
/// or nothing (a plain module name declared in this file, or no module
/// segment at all). The parser records whichever prefix is written;
/// resolving `Crate` and `Super` to an actual module, and rejecting
/// `Super` in the crate root (V0111), is the compiler's resolver's job.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathStart {
    Crate,
    SelfMod,
    Super,
    None,
}

/// A `::`-separated path (spec 3.1, 3.3): an optional keyword prefix
/// (`crate`, `self`, `super`) followed by name segments. What the segments
/// (and any name that follows them, for a type or expression path) resolve
/// to -- a chain of modules, a type, a function, a variant -- is decided by
/// where the path appears and, ultimately, the compiler's resolver, never
/// the parser.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Path {
    pub leading: PathStart,
    pub segments: Vec<Ident>,
    pub span: Span,
}

/// A named type: `string`, `i32`, `User`, and so on are all just names,
/// resolved later by the compiler's resolver. A type may carry a module
/// path (`m::User`, `shop::cart::Cart`, spec 2.10, 3.1) and generic
/// arguments (`Vec<i32>`, `Result<User, string>`, nested freely, spec 2.6);
/// the parser only records them, and the resolver checks that the path,
/// the name, and the argument count exist.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeExpr {
    pub path: Option<Path>,
    pub name: Ident,
    pub args: Vec<TypeExpr>,
    pub span: Span,
}

impl TypeExpr {
    /// Renders the whole type back to source text: the path (if any), the
    /// name, and any generic arguments, recursively. Used wherever a
    /// diagnostic or fix-it needs to show the type as written, rather than
    /// just its base name.
    pub fn display_name(&self) -> String {
        let mut out = String::new();
        if let Some(path) = &self.path {
            match path.leading {
                PathStart::Crate => out.push_str("crate::"),
                PathStart::SelfMod => out.push_str("self::"),
                PathStart::Super => out.push_str("super::"),
                PathStart::None => {}
            }
            for segment in &path.segments {
                out.push_str(&segment.name);
                out.push_str("::");
            }
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

/// An attribute, `#[name]` or `#[name(literal)]` (M5a spec 2.2), written
/// before an item, a struct field, an enum variant, a field of a variant,
/// or a method. The parser keeps every one it reads; which names exist and
/// where each may go is the compiler's resolver's to check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attribute {
    pub name: Ident,
    /// What is between the parentheses; `None` when there are none.
    pub arg: Option<AttrArg>,
    /// From `#` through `]`.
    pub span: Span,
}

/// The argument of an attribute: one literal, a number optionally after
/// `-`, with a number's raw text as lexed and a string's text between the
/// quotes, unmodified.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttrArg {
    Str(String),
    Int {
        text: String,
        negative: bool,
    },
    Float {
        text: String,
        negative: bool,
    },
    Bool(bool),
    /// Anything else between the parentheses (`#[derive(Clone)]`): kept
    /// so the resolver can name the attribute, never a valid argument.
    Other,
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
    Use(UseDecl),
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

/// `fn name(params) -> ReturnType { body }`, optionally `pub` and then
/// `async` (milestone 5b1 spec 2.2). Inside an
/// `impl` block the parameter list may start with a `self` receiver
/// (`self_mode`), which is never a [`Param`] (spec 2.5).
#[derive(Debug, Clone, PartialEq)]
pub struct Function {
    pub attrs: Vec<Attribute>,
    pub name: Ident,
    pub is_pub: bool,
    /// `async fn` (milestone 5b1 spec 2.2).
    pub is_async: bool,
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

/// `struct Name { fields... }`, optionally `pub` (spec 4.1). Whether a
/// field is visible follows the same rule as everything else (spec 3.2):
/// see [`FieldDecl::is_pub`].
#[derive(Debug, Clone, PartialEq)]
pub struct StructDecl {
    pub attrs: Vec<Attribute>,
    pub name: Ident,
    pub is_pub: bool,
    pub fields: Vec<FieldDecl>,
    pub span: Span,
}

/// A struct field: `name: T`, optionally `pub` (spec 3.4). Visibility
/// follows the same rule as everything else (section 3.2): a private
/// field is visible in the declaring module and its descendants, which
/// includes the struct's own methods.
#[derive(Debug, Clone, PartialEq)]
pub struct FieldDecl {
    pub attrs: Vec<Attribute>,
    pub name: Ident,
    pub ty: TypeExpr,
    pub is_pub: bool,
    pub span: Span,
}

/// `enum Name { variants... }` (spec 2.2), optionally `pub`. An enum has one
/// or more variants; an empty `{ }` is `V0002`, since the parser found a
/// complete-looking declaration missing the one thing it must have.
#[derive(Debug, Clone, PartialEq)]
pub struct EnumDecl {
    pub attrs: Vec<Attribute>,
    pub name: Ident,
    pub is_pub: bool,
    pub variants: Vec<EnumVariant>,
    pub span: Span,
}

/// A variant of an enum: a unit variant (`Point`), a tuple variant
/// (`Circle(f64)`), or a variant with named fields (`Click { x: i32 }`, M4
/// spec 2.5).
#[derive(Debug, Clone, PartialEq)]
pub struct EnumVariant {
    pub attrs: Vec<Attribute>,
    pub name: Ident,
    pub fields: VariantFields,
    pub span: Span,
}

/// What a variant holds: nothing, values by position (`Circle(f64)`; an
/// empty `Circle()` is a `Tuple` of no types), or named fields.
#[derive(Debug, Clone, PartialEq)]
pub enum VariantFields {
    Unit,
    Tuple(Vec<TypeExpr>),
    Named(Vec<VariantField>),
}

/// A named field of a variant: `name: T`. It has no visibility of its own:
/// a variant's fields are as visible as the enum, and Rust rejects `pub`
/// on them.
#[derive(Debug, Clone, PartialEq)]
pub struct VariantField {
    pub attrs: Vec<Attribute>,
    pub name: Ident,
    pub ty: TypeExpr,
    pub span: Span,
}

/// `impl Name { fns... }` (spec 2.5): a block of functions and methods for
/// a struct or enum declared in the same file. Whether `name` actually
/// names a struct or enum is the compiler's `resolve` module's job.
#[derive(Debug, Clone, PartialEq)]
pub struct ImplBlock {
    pub attrs: Vec<Attribute>,
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
    pub attrs: Vec<Attribute>,
    pub name: Ident,
    pub is_pub: bool,
    pub span: Span,
}

/// `use path;` or `use path as name;` (spec 3.3): introduces a local alias
/// for whatever `path` names -- a module, a struct, an enum, or a function
/// -- resolved and checked by the compiler's resolver, never the parser.
/// `pub use path;` (milestone 5b3 spec 2.5) also re-exports the item.
#[derive(Debug, Clone, PartialEq)]
pub struct UseDecl {
    pub attrs: Vec<Attribute>,
    pub is_pub: bool,
    pub path: Path,
    pub alias: Option<Ident>,
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
    /// `while let pattern = value { body }` (M4 spec 2.4): `value` is
    /// parsed in condition position, as `while`'s condition is.
    WhileLet {
        pattern: Pattern,
        value: Expr,
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

/// The head of a `for` loop (spec 2.4): an integer range, half-open
/// (`a..b`) or `inclusive` (`a..=b`, M4 spec 2.11), parsed only here since
/// an expression range exists nowhere else in Varyk, or an expression
/// iterated element by element as a `Vec`.
#[derive(Debug, Clone, PartialEq)]
pub enum ForHead {
    Range {
        start: Box<Expr>,
        end: Box<Expr>,
        inclusive: bool,
    },
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

    /// `name`, `module::name`, or `module::Type::name` (spec 2.5, 2.10,
    /// 3.1): a plain name, a module-qualified one, or a
    /// module-and-type-qualified one (an associated function or enum
    /// variant reached through a module), with paths now growing to any
    /// depth as nested modules do. `path` covers every segment before the
    /// final one (the module chain, and the type name when the path names
    /// an associated function or a variant); `name` is that final segment,
    /// the thing actually read or called. Whether `path`'s segments name a
    /// deeper module chain than milestone 3 resolves, a single module, or a
    /// type in this file (the two-way ambiguity a bare one-segment `path`
    /// carries, spec 2.10) is the compiler's resolver's job, not the
    /// parser's.
    Path {
        path: Option<Path>,
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

    /// `operand.await` (milestone 5b1 spec 2.3). Whether `operand` is
    /// something that can be waited for is the type checker's job.
    Await(Box<Expr>),

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

    /// `Name { field: expr, ... }`, or `path::Name { field: expr, ... }`
    /// with a module path of any depth and an optional `crate::`,
    /// `self::`, or `super::` prefix (spec 2.10, 3.1). Never parsed in
    /// condition position.
    StructLit {
        path: Option<Path>,
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

    /// `if let pattern = value { ... }`, with an optional `else` (M4 spec
    /// 2.4). `value` is parsed in condition position, as `if`'s condition
    /// is, and `else if` and `else if let` are a synthetic block holding
    /// the nested expression as its tail, as for [`ExprKind::If`].
    IfLet {
        pattern: Pattern,
        value: Box<Expr>,
        then: Block,
        else_: Option<Block>,
        span: Span,
    },

    /// `|params| body` (M4 spec 2.2). The parameters carry no type; how
    /// many there may be, and where a closure may appear, is the
    /// checker's to decide.
    Closure {
        params: Vec<Ident>,
        body: Box<Expr>,
        span: Span,
    },

    /// `expr as T` (M4 spec 2.9), binding tighter than `*`. `ty` is a
    /// plain name with no path and no generic arguments.
    Cast {
        expr: Box<Expr>,
        ty: TypeExpr,
        span: Span,
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

/// A pattern (spec 2.3, M4 spec 2.5), in a `match` arm, an `if let`, or a
/// `while let`, nested to any depth. Not in Varyk, each `V0001` from the
/// parser: guards (`pattern if cond`), alternatives (`a | b`), rest
/// patterns (`..`), and `@` bindings.
#[derive(Debug, Clone, PartialEq)]
pub enum Pattern {
    /// `_`: matches anything, binds nothing.
    Wildcard(Span),
    /// A bare name: matches anything and binds it. The parser cannot tell
    /// this apart from a zero-argument variant written without its type
    /// (`None`): the type checker decides against the matched value's
    /// type.
    Name(Ident),
    /// A variant, optionally reached through a type and a module, with its
    /// values by position: `Point`, `Some(p)`, `Shape::Circle(p, q)`,
    /// `geo::Shape::Point`. `path`'s last segment is always the type
    /// (unlike an expression path, a pattern never calls anything, so
    /// there is no bare-module case to weigh it against); any segments
    /// before that are the module chain. `span` ends at the closing `)`
    /// when there are parentheses, so `Point()` is told apart from
    /// `Point`.
    Variant {
        path: Option<Path>,
        name: Ident,
        fields: Vec<Pattern>,
        span: Span,
    },
    /// A variant or struct with named fields: `Event::Click { x: 0, y }`.
    /// A shorthand field `y` is `(y, Pattern::Name(y))`.
    Struct {
        path: Option<Path>,
        name: Ident,
        fields: Vec<(Ident, Pattern)>,
        span: Span,
    },
    /// A literal: an integer with an optional `-`, a string, `true`, or
    /// `false` (and a float, which the checker rejects).
    Literal(Literal, Span),
    /// `start..=end`, both ends included.
    Range {
        start: Literal,
        end: Literal,
        span: Span,
    },
}

impl Pattern {
    pub fn span(&self) -> Span {
        match self {
            Pattern::Wildcard(span) | Pattern::Literal(_, span) => *span,
            Pattern::Name(ident) => ident.span,
            Pattern::Variant { span, .. }
            | Pattern::Struct { span, .. }
            | Pattern::Range { span, .. } => *span,
        }
    }
}

/// The value of a literal pattern or of a range pattern's end, with the
/// raw text of a number as lexed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Literal {
    Int { text: String, negative: bool },
    Float { text: String, negative: bool },
    Str(String),
    Bool(bool),
}
