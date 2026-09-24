//! Expression emission with the type-bridging rule (spec 6.3).
//!
//! Every expression has a generated Rust type ([`Have`]) and every context
//! a required one ([`Need`]); [`FnEmitter::bridge`] inserts `&`, `&mut`,
//! `*`, `.as_str()`, or `.to_string()` to get from one to the other,
//! relying on Rust's reborrowing and deref coercion where a reference
//! already has the right shape.

use varyk_syntax::{BinaryOp, UnaryOp};

use super::rust::{item_path, struct_path};
use crate::hir::{
    HirBlock, HirExpr, HirExprKind, HirFunction, HirProgram, HirStmt, LocalId, StringRepr,
    declared_inside, field_root, is_block_like, leaves,
};
use crate::resolve::{Callee, ModuleId};
use crate::types::{FloatKind, IntKind, ParamMode, Ty};

/// What a local is in the generated Rust.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Repr {
    /// The value itself: a Copy value, an owned struct, or a `String`.
    Owned,
    /// A reference: `&T` / `&mut T`, including `&String` / `&mut String`.
    Ref { mutable: bool },
    /// A `&str`.
    Str,
}

/// The generated Rust type of an expression, as far as bridging cares.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Have {
    /// A temporary value: a literal other than a string, a call result, a
    /// struct literal, or an operator result.
    Temp,
    /// An owned place: an owned local or a field path.
    Place,
    /// A reference-typed local.
    Ref { mutable: bool },
    /// A `&str`: a string literal or a `&str` local.
    Str,
}

/// The type a context requires.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Need {
    /// The owned value: a Copy value, a struct, or a `String`.
    Value,
    /// A shared reference (`&str` for a string). `binding` is set for a
    /// `let` initializer, where Rust has no expected type to reborrow to.
    Shared { binding: bool },
    /// A mutable reference.
    Mut { binding: bool },
    /// A `&str`.
    Str,
    /// Whatever form the value already has: a `println!` argument, or an
    /// expression statement's value, which is dropped.
    AsIs,
}

/// Emits one function's statements and expressions.
pub(super) struct FnEmitter<'a> {
    pub(super) program: &'a HirProgram,
    /// The module being emitted.
    pub(super) module: ModuleId,
    /// Every Varyk function's module, indexed by `FnId`.
    pub(super) fn_modules: &'a [ModuleId],
    pub(super) function: &'a HirFunction,
    /// Indexed by `LocalId`.
    pub(super) reprs: Vec<Repr>,
}

impl<'a> FnEmitter<'a> {
    pub(super) fn new(
        program: &'a HirProgram,
        module: ModuleId,
        fn_modules: &'a [ModuleId],
        function: &'a HirFunction,
    ) -> Self {
        let mut emitter = FnEmitter {
            program,
            module,
            fn_modules,
            function,
            reprs: vec![Repr::Owned; function.locals.len()],
        };
        for param in &function.params {
            emitter.reprs[param.local.0 as usize] = match (param.mode, param.ty) {
                (ParamMode::Owned, _) => Repr::Owned,
                (ParamMode::SharedBorrow, Ty::String) => Repr::Str,
                (ParamMode::SharedBorrow, _) => Repr::Ref { mutable: false },
                (ParamMode::MutableBorrow, _) => Repr::Ref { mutable: true },
            };
        }
        emitter.scan_block(&function.body);
        emitter
    }

    // --- Local representations ----------------------------------------------

    fn scan_block(&mut self, block: &HirBlock) {
        for stmt in &block.stmts {
            match stmt {
                HirStmt::Let { local, value, .. } => {
                    self.scan_expr(value);
                    self.reprs[local.0 as usize] = self.let_repr(*local, value);
                }
                HirStmt::Assign { target, value, .. } => {
                    self.scan_expr(target);
                    self.scan_expr(value);
                }
                HirStmt::Expr { expr, .. } => self.scan_expr(expr),
                HirStmt::Return { value, .. } => {
                    if let Some(value) = value {
                        self.scan_expr(value);
                    }
                }
                HirStmt::While { cond, body, .. } => {
                    self.scan_expr(cond);
                    self.scan_block(body);
                }
                HirStmt::Break { .. } | HirStmt::Continue { .. } => {}
            }
        }
        if let Some(tail) = &block.tail {
            self.scan_expr(tail);
        }
    }

    /// Visits the blocks nested in `expr`, which may declare locals.
    fn scan_expr(&mut self, expr: &HirExpr) {
        match &expr.kind {
            HirExprKind::Block(block) => self.scan_block(block),
            HirExprKind::If { cond, then, else_ } => {
                self.scan_expr(cond);
                self.scan_block(then);
                if let Some(else_) = else_ {
                    self.scan_block(else_);
                }
            }
            HirExprKind::Call { args, .. } | HirExprKind::Println { args, .. } => {
                for arg in args {
                    self.scan_expr(arg);
                }
            }
            HirExprKind::Field { base, .. } => self.scan_expr(base),
            HirExprKind::StructLit { fields, .. } => {
                for (_, value) in fields {
                    self.scan_expr(value);
                }
            }
            HirExprKind::Unary { operand, .. } => self.scan_expr(operand),
            HirExprKind::Binary { lhs, rhs, .. } => {
                self.scan_expr(lhs);
                self.scan_expr(rhs);
            }
            HirExprKind::Int(_)
            | HirExprKind::Float(_)
            | HirExprKind::Bool(_)
            | HirExprKind::String(_)
            | HirExprKind::Local(_) => {}
        }
    }

    /// The representation of `let local = value`.
    fn let_repr(&self, local: LocalId, value: &HirExpr) -> Repr {
        let info = &self.function.locals[local.0 as usize];
        if info.ty.is_copy() {
            return Repr::Owned;
        }
        if !info.place.borrowed {
            return match info.repr {
                Some(StringRepr::Borrowed) => Repr::Str,
                _ => Repr::Owned,
            };
        }
        // A borrowed place is a reference, except that a `string` one whose
        // pointee is a `&str`, or whose branches include a `&str`, is a
        // `&str` itself, so every branch agrees on one type. A mutable one
        // stays a `&mut String`, so changing it changes what it refers to:
        // a literal branch then converts (borrow analysis makes every `let`
        // it may refer to a `String`).
        let str_leaf = leaves(value)
            .into_iter()
            .any(|leaf| self.have(leaf) == Have::Str);
        if info.ty == Ty::String
            && !info.place.mutable
            && (info.repr == Some(StringRepr::Borrowed) || str_leaf)
        {
            Repr::Str
        } else {
            Repr::Ref {
                mutable: info.place.mutable,
            }
        }
    }

    /// What a `let` initializer of (or a plain assignment to) `local` must
    /// produce.
    pub(super) fn binding_need(&self, local: LocalId) -> Need {
        match self.reprs[local.0 as usize] {
            Repr::Owned => Need::Value,
            Repr::Str => Need::Str,
            Repr::Ref { mutable: false } => Need::Shared { binding: true },
            Repr::Ref { mutable: true } => Need::Mut { binding: true },
        }
    }

    pub(super) fn local_name(&self, local: LocalId) -> &str {
        &self.function.locals[local.0 as usize].name
    }

    // --- Expressions --------------------------------------------------------

    fn have(&self, expr: &HirExpr) -> Have {
        match &expr.kind {
            HirExprKind::String(_) => Have::Str,
            HirExprKind::Local(id) => match self.reprs[id.0 as usize] {
                Repr::Owned => Have::Place,
                Repr::Ref { mutable } => Have::Ref { mutable },
                Repr::Str => Have::Str,
            },
            HirExprKind::Field { .. } => Have::Place,
            _ => Have::Temp,
        }
    }

    /// `expr` as Rust code of the type `need` requires, at `indent` levels
    /// (for the inner lines of a multi-line `if` or block).
    pub(super) fn expr(&self, expr: &HirExpr, need: Need, indent: usize) -> String {
        if self.borrows_new_value(expr, need) {
            // Borrowing inside each branch would borrow a temporary that
            // dies with the branch; borrow the whole value instead.
            let text = self.expr(expr, Need::Value, indent);
            return self.bridge(expr, text, need);
        }
        match &expr.kind {
            HirExprKind::If { cond, then, else_ } => {
                let need = self.branch_need(expr, need);
                let mut out = format!(
                    "if {} {}",
                    self.expr(cond, Need::Value, indent),
                    self.block(then, need, indent)
                );
                if let Some(else_) = else_ {
                    out.push_str(" else ");
                    match else_.tail.as_deref() {
                        Some(
                            tail @ HirExpr {
                                kind: HirExprKind::If { .. },
                                ..
                            },
                        ) if else_.stmts.is_empty() => {
                            out.push_str(&self.expr(tail, need, indent));
                        }
                        _ => out.push_str(&self.block(else_, need, indent)),
                    }
                }
                out
            }
            HirExprKind::Block(block) => self.block(block, self.branch_need(expr, need), indent),
            // A field lent mutably reaches it mutably through an `if` or
            // block base too.
            HirExprKind::Field { .. } if matches!(need, Need::Mut { .. }) => {
                let text = self.field(expr, true, indent);
                self.bridge(expr, text, need)
            }
            _ => {
                let text = self.raw(expr, indent);
                self.bridge(expr, text, need)
            }
        }
    }

    /// Whether `expr` is a block-like expression, read by reference, that
    /// must be borrowed as a whole: a Copy one lent to a shared parameter,
    /// which is simply copied, or one whose branches are new values (see
    /// [`FnEmitter::is_new`]), apart from string literals. Borrowing inside
    /// each branch would borrow a value that dies with the branch. Borrow
    /// analysis rejects one that mixes new values with places.
    fn borrows_new_value(&self, expr: &HirExpr, need: Need) -> bool {
        if !is_block_like(expr)
            || !matches!(
                need,
                Need::Shared { binding: false } | Need::Mut { binding: false } | Need::Str
            )
        {
            return false;
        }
        if expr.ty.is_copy() && matches!(need, Need::Shared { .. }) {
            return true;
        }
        let mutable = matches!(need, Need::Mut { .. });
        let leaves = leaves(expr);
        leaves.iter().any(|leaf| self.is_new(leaf, expr, mutable))
            && leaves.iter().all(|leaf| {
                self.is_new(leaf, expr, mutable) || matches!(leaf.kind, HirExprKind::String(_))
            })
    }

    /// Whether `leaf`, a value the block-like `whole` may evaluate to, is
    /// made while `whole` runs and is gone after it: a temporary (a call
    /// result, a struct literal, an operator result, a literal other than
    /// a string one unless it is borrowed mutably), a field of one, or a
    /// local declared inside `whole` that holds its value rather than a
    /// reference (or a field of one). Mirrors borrow analysis's `Leaf`.
    fn is_new(&self, leaf: &HirExpr, whole: &HirExpr, mutable: bool) -> bool {
        let base = field_root(leaf);
        match &base.kind {
            HirExprKind::String(_) => mutable,
            HirExprKind::Local(id) => {
                declared_inside(&self.function.locals[id.0 as usize], whole.span)
                    && self.reprs[id.0 as usize] == Repr::Owned
            }
            HirExprKind::If { .. } | HirExprKind::Block(_) => false,
            _ => true,
        }
    }

    /// The need each branch tail of a block-like `expr` gets, so that all
    /// branches have one Rust type.
    fn branch_need(&self, expr: &HirExpr, need: Need) -> Need {
        match need {
            Need::AsIs => {
                let new = |leaf: &&HirExpr| self.is_new(leaf, expr, false);
                match expr.ty {
                    // Text that is new in some branch (borrow analysis
                    // allows literals alongside) is a `String` in all;
                    // otherwise each branch borrows its text.
                    Ty::String if !leaves(expr).iter().any(new) => Need::Str,
                    // A discarded struct whose branches are all new values
                    // has no place to borrow: borrowing each branch would
                    // reference a value that dies with it (E0716). Emit
                    // the branches as plain values instead; otherwise every
                    // branch is a place, borrowed so that it is not moved.
                    Ty::Struct(_) if leaves(expr).iter().all(new) => Need::Value,
                    Ty::Struct(_) => Need::Shared { binding: true },
                    _ => Need::Value,
                }
            }
            _ => need,
        }
    }

    /// Converts `text`, the emission of `expr`, to the type `need` requires.
    fn bridge(&self, expr: &HirExpr, text: String, need: Need) -> String {
        let have = self.have(expr);
        match need {
            Need::AsIs => text,
            Need::Value => match have {
                Have::Ref { .. } if expr.ty.is_copy() => format!("*{text}"),
                Have::Str if expr.ty == Ty::String => {
                    format!("{}.to_string()", postfix(expr, text))
                }
                _ => text,
            },
            Need::Shared { binding } => match have {
                Have::Temp | Have::Place => format!("&{}", prefix(expr, text)),
                Have::Ref { mutable: true } if binding => format!("&*{text}"),
                Have::Ref { .. } | Have::Str => text,
            },
            Need::Mut { binding } => match have {
                Have::Temp | Have::Place => format!("&mut {}", prefix(expr, text)),
                Have::Ref { mutable: true } if binding => format!("&mut *{text}"),
                // A literal lent mutably is a temporary `String`. Borrow
                // analysis never lends a `&str` local mutably: it makes a
                // `let` so lent a `String`, and a binding of borrowed text
                // is not a mutable place.
                Have::Str => format!("&mut {}.to_string()", postfix(expr, text)),
                Have::Ref { .. } => text,
            },
            Need::Str => match have {
                Have::Str => text,
                _ => format!("{}.as_str()", postfix(expr, text)),
            },
        }
    }

    /// `expr` without bridging; its sub-expressions are bridged to what
    /// their own contexts require.
    fn raw(&self, expr: &HirExpr, indent: usize) -> String {
        match &expr.kind {
            HirExprKind::Int(text) => match expr.ty {
                Ty::Int(IntKind::I32) => text.clone(),
                Ty::Int(kind) => format!("{text}{}", kind.name()),
                _ => text.clone(),
            },
            HirExprKind::Float(text) => match expr.ty {
                Ty::Float(FloatKind::F32) => format!("{text}f32"),
                _ => text.clone(),
            },
            HirExprKind::Bool(value) => value.to_string(),
            HirExprKind::String(text) => format!("\"{text}\""),
            HirExprKind::Local(id) => self.local_name(*id).to_string(),
            HirExprKind::Call { callee, args } => {
                let (path, modes) = self.callee(*callee);
                let args: Vec<String> = args
                    .iter()
                    .zip(modes)
                    .map(|(arg, mode)| {
                        let need = match mode {
                            ParamMode::SharedBorrow => Need::Shared { binding: false },
                            ParamMode::MutableBorrow => Need::Mut { binding: false },
                            ParamMode::Owned => Need::Value,
                        };
                        self.expr(arg, need, indent)
                    })
                    .collect();
                format!("{path}({})", args.join(", "))
            }
            HirExprKind::Field { .. } => self.field(expr, false, indent),
            HirExprKind::StructLit { id, fields } => {
                let path = struct_path(self.program, *id, self.module);
                if fields.is_empty() {
                    return format!("{path} {{}}");
                }
                let names = &self.program.structs[id.0 as usize].fields;
                let fields: Vec<String> = fields
                    .iter()
                    .map(|(index, value)| {
                        format!(
                            "{}: {}",
                            names[*index].0,
                            self.expr(value, Need::Value, indent)
                        )
                    })
                    .collect();
                format!("{path} {{ {} }}", fields.join(", "))
            }
            HirExprKind::Unary { op, operand } => {
                let op = match op {
                    UnaryOp::Neg => "-",
                    UnaryOp::Not => "!",
                };
                format!("{op}{}", self.operand(operand, None, Need::Value, indent))
            }
            HirExprKind::Binary { op, lhs, rhs } => {
                // String operands (of `==` and the other comparisons) are
                // normalized to `&str`.
                let need = if lhs.ty == Ty::String {
                    Need::Str
                } else {
                    Need::Value
                };
                format!(
                    "{} {} {}",
                    self.operand(lhs, Some((*op, false)), need, indent),
                    op.as_str(),
                    self.operand(rhs, Some((*op, true)), need, indent)
                )
            }
            HirExprKind::Println { format, args } => {
                let mut out = format!("println!(\"{format}\"");
                for arg in args {
                    out.push_str(", ");
                    out.push_str(&self.expr(arg, Need::AsIs, indent));
                }
                out.push(')');
                out
            }
            HirExprKind::Block(_) | HirExprKind::If { .. } => {
                unreachable!("`expr` emits blocks and `if`s itself")
            }
        }
    }

    /// An operator operand, parenthesized when it is a binary expression
    /// that binds looser than its `parent` (the binary operator and whether
    /// this is its right operand; `None` under a unary operator), or a
    /// block-like one (the HIR has no grouping node, and Rust reads a
    /// leading `if` or `{` as a statement).
    fn operand(
        &self,
        expr: &HirExpr,
        parent: Option<(BinaryOp, bool)>,
        need: Need,
        indent: usize,
    ) -> String {
        let text = self.expr(expr, need, indent);
        let wrap = match expr.kind {
            HirExprKind::Binary { op, .. } => parent.is_none_or(|(parent, right)| {
                let (inner, outer) = (precedence(op), precedence(parent));
                inner < outer || (inner == outer && (right || inner == COMPARISON))
            }),
            // Already wrapped when bridged as a whole: `(if ..).as_str()`.
            HirExprKind::If { .. } | HirExprKind::Block(_) => !text.starts_with('('),
            _ => false,
        };
        if wrap { format!("({text})") } else { text }
    }

    /// The field access `expr`, its base reached mutably when `mutable`.
    fn field(&self, expr: &HirExpr, mutable: bool, indent: usize) -> String {
        let HirExprKind::Field { base, name } = &expr.kind else {
            unreachable!("`field` emits field accesses")
        };
        format!("{}.{name}", self.field_base(base, mutable, indent))
    }

    /// The base of a field access or a field assignment target: a place
    /// reached through auto-deref, never moved; reached mutably when
    /// `mutable`.
    pub(super) fn field_base(&self, base: &HirExpr, mutable: bool, indent: usize) -> String {
        match &base.kind {
            HirExprKind::Field { .. } => self.field(base, mutable, indent),
            HirExprKind::Local(_) | HirExprKind::Call { .. } => self.raw(base, indent),
            // A block or `if` base is read by reference rather than by
            // value: field access auto-derefs, so this keeps a place leaf
            // (a local or field ending a branch) unmoved. Each branch
            // reborrows (`&*r`), since a base has no expected type that
            // would reborrow a `&mut` local instead of moving it. A base
            // whose branches are new values borrows the whole completed
            // value instead (see `borrows_new_value`).
            HirExprKind::Block(_) | HirExprKind::If { .. } => {
                let need = if mutable {
                    Need::Mut { binding: false }
                } else {
                    Need::Shared { binding: false }
                };
                let need = if self.borrows_new_value(base, need) {
                    need
                } else if mutable {
                    Need::Mut { binding: true }
                } else {
                    Need::Shared { binding: true }
                };
                format!("({})", self.expr(base, need, indent))
            }
            _ => format!("({})", self.expr(base, Need::Value, indent)),
        }
    }

    /// The callee's path from the current module and its parameter modes.
    fn callee(&self, callee: Callee) -> (String, Vec<ParamMode>) {
        match callee {
            Callee::Varyk(id) => {
                let function = self.program.function(id);
                let module = self.fn_modules[id.0 as usize];
                let path = item_path(self.program, module, self.module, &function.name);
                (path, function.params.iter().map(|p| p.mode).collect())
            }
            Callee::Imported(id) => {
                let sig = &self.program.imported[id.0 as usize];
                let path = item_path(self.program, sig.module, self.module, &sig.name);
                (path, sig.params.iter().map(|(_, mode)| *mode).collect())
            }
        }
    }
}

/// `text` ready for a prefix operator: parenthesized when `expr` is a
/// binary or block-like expression.
fn prefix(expr: &HirExpr, text: String) -> String {
    match expr.kind {
        HirExprKind::Binary { .. } | HirExprKind::If { .. } | HirExprKind::Block(_) => {
            format!("({text})")
        }
        _ => text,
    }
}

/// `text` ready for a method call: parenthesized when `expr` is an
/// operator or block-like expression.
fn postfix(expr: &HirExpr, text: String) -> String {
    match expr.kind {
        HirExprKind::Binary { .. }
        | HirExprKind::Unary { .. }
        | HirExprKind::If { .. }
        | HirExprKind::Block(_) => format!("({text})"),
        _ => text,
    }
}

/// Rust's comparisons do not chain: `a == b == c` does not parse.
const COMPARISON: u8 = 3;

/// How tightly `op` binds, the same in Varyk and Rust.
fn precedence(op: BinaryOp) -> u8 {
    match op {
        BinaryOp::Or => 1,
        BinaryOp::And => 2,
        BinaryOp::Eq | BinaryOp::Ne | BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge => {
            COMPARISON
        }
        BinaryOp::Add | BinaryOp::Sub => 4,
        BinaryOp::Mul | BinaryOp::Div | BinaryOp::Rem => 5,
    }
}
