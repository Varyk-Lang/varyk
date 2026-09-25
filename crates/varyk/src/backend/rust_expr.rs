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
    HirArm, HirExpr, HirExprKind, HirFunction, HirPattern, HirProgram, LocalId, MethodRef,
    StringRepr, VariantRef, declared_inside, field_root, is_block_like, is_place, leaves,
};
use crate::resolve::{Callee, ModuleId, UserType};
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
    pub(super) function: &'a HirFunction,
}

impl<'a> FnEmitter<'a> {
    pub(super) fn new(program: &'a HirProgram, function: &'a HirFunction) -> Self {
        FnEmitter {
            program,
            module: function.module,
            function,
        }
    }

    // --- Local representations ----------------------------------------------

    /// The representation of `local`, read from what borrow analysis
    /// recorded: a `string` local's [`StringRepr`], else its place info. A
    /// `mut` parameter is a `&mut T`, a Copy value otherwise owned, and a
    /// borrowed place a reference.
    pub(super) fn repr(&self, local: LocalId) -> Repr {
        let index = local.0 as usize;
        let info = &self.function.locals[index];
        match info.repr {
            Some(StringRepr::Str) => Repr::Str,
            Some(StringRepr::Owned) => Repr::Owned,
            Some(StringRepr::RefOwned) => Repr::Ref { mutable: false },
            Some(StringRepr::MutOwned) => Repr::Ref { mutable: true },
            None if index < self.function.params.len() && info.mutable => {
                Repr::Ref { mutable: true }
            }
            None if !info.ty.is_copy() && info.place.borrowed => Repr::Ref {
                mutable: info.place.mutable,
            },
            None => Repr::Owned,
        }
    }

    /// What a `let` initializer of (or a plain assignment to) `local` must
    /// produce.
    pub(super) fn binding_need(&self, local: LocalId) -> Need {
        match self.repr(local) {
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
            HirExprKind::Local(id) => match self.repr(*id) {
                Repr::Owned => Have::Place,
                Repr::Ref { mutable } => Have::Ref { mutable },
                Repr::Str => Have::Str,
            },
            HirExprKind::Field { .. } | HirExprKind::Index { .. } => Have::Place,
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
            HirExprKind::Match { scrutinee, arms } => {
                self.match_expr(scrutinee, arms, self.branch_need(expr, need), indent)
            }
            // A field or element lent mutably reaches it mutably through an
            // `if` or block base too.
            HirExprKind::Field { .. } | HirExprKind::Index { .. }
                if matches!(need, Need::Mut { .. }) =>
            {
                let text = self.place(expr, true, indent);
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
    /// reference (or a field of one). Mirrors borrow analysis's `Leaf`: a
    /// non-`Copy` element of a new `Vec` is new here, and borrow analysis
    /// lets it through only where a `let` keeps that `Vec` alive.
    fn is_new(&self, leaf: &HirExpr, whole: &HirExpr, mutable: bool) -> bool {
        let base = field_root(leaf);
        match &base.kind {
            HirExprKind::String(_) => mutable,
            HirExprKind::Local(id) => {
                declared_inside(&self.function.locals[id.0 as usize], whole.span)
                    && self.repr(*id) == Repr::Owned
            }
            _ if is_block_like(base) => false,
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
                    ref ty if ty.is_compound() && leaves(expr).iter().all(new) => Need::Value,
                    ref ty if ty.is_compound() => Need::Shared { binding: true },
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
                format!("{path}({})", self.args(args, &modes, indent))
            }
            HirExprKind::MethodCall {
                receiver,
                method,
                args,
            } => self.method_call(receiver, *method, args, indent),
            HirExprKind::Field { .. } | HirExprKind::Index { .. } => {
                self.place(expr, false, indent)
            }
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
                // String operands (of `==` and `!=`) are normalized to
                // `&str` only as needed: a `String` and a `&str` compare
                // in any mix, a `&String` or a block-like one does not.
                let need = |operand: &HirExpr| {
                    if operand.ty != Ty::String {
                        Need::Value
                    } else if is_block_like(operand)
                        || matches!(self.have(operand), Have::Ref { .. })
                    {
                        Need::Str
                    } else {
                        Need::AsIs
                    }
                };
                format!(
                    "{} {} {}",
                    self.operand(lhs, Some((*op, false)), need(lhs), indent),
                    op.as_str(),
                    self.operand(rhs, Some((*op, true)), need(rhs), indent)
                )
            }
            HirExprKind::Println { format, args } => {
                self.format_call("println", format, args, indent)
            }
            HirExprKind::Format { format, args } => {
                self.format_call("format", format, args, indent)
            }
            HirExprKind::EnumLit { variant, args } => {
                let path = self.variant_path(*variant);
                if args.is_empty() {
                    return path;
                }
                let args: Vec<String> = args
                    .iter()
                    .map(|arg| self.expr(arg, Need::Value, indent))
                    .collect();
                format!("{path}({})", args.join(", "))
            }
            // The operand is an owned slot: moved or a temporary (spec 3.4).
            HirExprKind::Try(operand) => {
                let text = self.expr(operand, Need::Value, indent);
                format!("{}?", postfix(operand, text))
            }
            HirExprKind::VecLit(elements) => {
                let elements: Vec<String> = elements
                    .iter()
                    .map(|element| self.expr(element, Need::Value, indent))
                    .collect();
                format!("vec![{}]", elements.join(", "))
            }
            HirExprKind::Block(_) | HirExprKind::If { .. } | HirExprKind::Match { .. } => {
                unreachable!("`expr` emits blocks, `if`s, and `match`es itself")
            }
        }
    }

    /// `match head { arms }` (spec 5): a place is matched through a shared
    /// reference (`&v` for an owned place, `v` for a `&T`, `&*v` for a
    /// `&mut T`), a temporary as it is. A Copy value bound through that
    /// reference is copied at the start of its arm with `let n = *n;`,
    /// which makes an expression arm a block. Each arm's value is emitted
    /// as `need` says.
    fn match_expr(&self, head: &HirExpr, arms: &[HirArm], need: Need, indent: usize) -> String {
        let by_reference = is_place(head);
        let head_need = if by_reference {
            Need::Shared { binding: true }
        } else {
            Need::Value
        };
        let inner = indent + 1;
        let pad = "    ".repeat(inner);
        let mut out = format!("match {} {{\n", self.expr(head, head_need, indent));
        for arm in arms {
            let copies: Vec<String> = arm
                .pattern
                .bindings()
                .into_iter()
                .filter(|(local, _)| {
                    by_reference && self.function.locals[local.0 as usize].ty.is_copy()
                })
                .map(|(local, _)| {
                    let name = self.local_name(local);
                    format!("let {name} = *{name};")
                })
                .collect();
            let body = match &arm.body.kind {
                HirExprKind::Block(block) => self.block_after(block, &copies, need, inner),
                _ if copies.is_empty() => format!("{},", self.expr(&arm.body, need, inner)),
                _ => {
                    let body_pad = "    ".repeat(inner + 1);
                    let mut body = String::from("{\n");
                    for copy in &copies {
                        body.push_str(&format!("{body_pad}{copy}\n"));
                    }
                    let value = self.expr(&arm.body, need, inner + 1);
                    body.push_str(&format!("{body_pad}{value}\n{pad}}}"));
                    body
                }
            };
            out.push_str(&format!("{pad}{} => {body}\n", self.pattern(&arm.pattern)));
        }
        out.push_str(&"    ".repeat(indent));
        out.push('}');
        out
    }

    /// A pattern: variants spelled as in a value, by their path from this
    /// module.
    fn pattern(&self, pattern: &HirPattern) -> String {
        match pattern {
            HirPattern::Wildcard => "_".to_string(),
            HirPattern::Binding(local) => self.local_name(*local).to_string(),
            HirPattern::Variant { variant, bindings } => {
                let path = self.variant_path(*variant);
                if bindings.is_empty() {
                    return path;
                }
                let names: Vec<&str> = bindings
                    .iter()
                    .map(|binding| binding.map_or("_", |local| self.local_name(local)))
                    .collect();
                format!("{path}({})", names.join(", "))
            }
        }
    }

    /// A variant's path from this module: `Shape::Point`,
    /// `crate::geo::Shape::Point`, or `Some`.
    fn variant_path(&self, variant: VariantRef) -> String {
        match variant {
            VariantRef::User(id, index) => {
                let e = &self.program.enums[id.0 as usize];
                let owner = item_path(self.program, e.module, self.module, &e.name);
                format!("{owner}::{}", e.variants[index].0)
            }
            VariantRef::Some => "Some".to_string(),
            VariantRef::None => "None".to_string(),
            VariantRef::Ok => "Ok".to_string(),
            VariantRef::Err => "Err".to_string(),
        }
    }

    /// Call arguments, each as its parameter's mode requires.
    fn args(&self, args: &[HirExpr], modes: &[ParamMode], indent: usize) -> String {
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
        args.join(", ")
    }

    /// `receiver.name(args)`. Rust's method call borrows the receiver as
    /// the method's `self` needs, so a place receiver is written as it is
    /// (never moved); a string receiver that is a `&str` has `clone`
    /// spelled `.to_string()`, since cloning a `&str` copies only the
    /// reference (spec 3.5).
    fn method_call(
        &self,
        receiver: &HirExpr,
        method: MethodRef,
        args: &[HirExpr],
        indent: usize,
    ) -> String {
        let (name, modes): (&str, Vec<ParamMode>) = match method {
            MethodRef::Varyk(id) => {
                let function = self.program.function(id);
                let modes = function.params.iter().map(|p| p.mode).collect();
                (&function.name, modes)
            }
            MethodRef::Builtin(id) => (id.get().name, id.modes()),
        };
        let args = self.args(args, &modes[1..], indent);
        if receiver.ty != Ty::String {
            let mutable = modes[0] == ParamMode::MutableBorrow;
            let receiver = self.field_base(receiver, mutable, indent);
            return format!("{receiver}.{name}({args})");
        }
        let (receiver, is_str) = if is_block_like(receiver) {
            (
                format!("({})", self.expr(receiver, Need::Str, indent)),
                true,
            )
        } else if self.have(receiver) == Have::Str {
            (self.raw(receiver, indent), true)
        } else {
            (self.field_base(receiver, false, indent), false)
        };
        if name == "clone" && is_str {
            format!("{receiver}.to_string()")
        } else {
            format!("{receiver}.{name}({args})")
        }
    }

    /// `println!` or `format!` (`name`): every argument as it is, since
    /// the format machinery borrows it.
    fn format_call(&self, name: &str, format: &str, args: &[HirExpr], indent: usize) -> String {
        let mut out = format!("{name}!(\"{format}\"");
        for arg in args {
            out.push_str(", ");
            out.push_str(&self.expr(arg, Need::AsIs, indent));
        }
        out.push(')');
        out
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
            _ if is_block_like(expr) => !text.starts_with('('),
            _ => false,
        };
        if wrap { format!("({text})") } else { text }
    }

    /// The field access or element `expr`, its base reached mutably when
    /// `mutable`.
    pub(super) fn place(&self, expr: &HirExpr, mutable: bool, indent: usize) -> String {
        match &expr.kind {
            HirExprKind::Field { base, name } => {
                format!("{}.{name}", self.field_base(base, mutable, indent))
            }
            HirExprKind::Index { base, index } => format!(
                "{}[{}]",
                self.field_base(base, mutable, indent),
                self.expr(index, Need::Value, indent)
            ),
            _ => unreachable!("`place` emits field accesses and elements"),
        }
    }

    /// The base of a field access, an element, a method call, or an
    /// assignment target of one of those: a place reached through
    /// auto-deref, never moved; reached mutably when `mutable`.
    pub(super) fn field_base(&self, base: &HirExpr, mutable: bool, indent: usize) -> String {
        match &base.kind {
            HirExprKind::Field { .. } | HirExprKind::Index { .. } => {
                self.place(base, mutable, indent)
            }
            HirExprKind::Local(_)
            | HirExprKind::Call { .. }
            | HirExprKind::MethodCall { .. }
            | HirExprKind::Try(_) => self.raw(base, indent),
            // A block or `if` base is read by reference rather than by
            // value: field access auto-derefs, so this keeps a place leaf
            // (a local or field ending a branch) unmoved. Each branch
            // reborrows (`&*r`), since a base has no expected type that
            // would reborrow a `&mut` local instead of moving it. A base
            // whose branches are new values borrows the whole completed
            // value instead (see `borrows_new_value`).
            _ if is_block_like(base) => {
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
                // An associated function is reached through its type.
                let path = match function.owner {
                    None => item_path(self.program, function.module, self.module, &function.name),
                    Some(owner) => {
                        let type_name = match owner {
                            UserType::Struct(id) => &self.program.structs[id.0 as usize].name,
                            UserType::Enum(id) => &self.program.enums[id.0 as usize].name,
                        };
                        let owner =
                            item_path(self.program, function.module, self.module, type_name);
                        format!("{owner}::{}", function.name)
                    }
                };
                (path, function.params.iter().map(|p| p.mode).collect())
            }
            Callee::Imported(id) => {
                let sig = &self.program.imported[id.0 as usize];
                let path = item_path(self.program, sig.module, self.module, &sig.name);
                (path, sig.params.iter().map(|(_, mode)| *mode).collect())
            }
            Callee::Builtin(id) => (id.path(), id.modes()),
        }
    }
}

/// `text` ready for a prefix operator: parenthesized when `expr` is a
/// binary or block-like expression.
fn prefix(expr: &HirExpr, text: String) -> String {
    match expr.kind {
        HirExprKind::Binary { .. } => format!("({text})"),
        _ if is_block_like(expr) => format!("({text})"),
        _ => text,
    }
}

/// `text` ready for a method call: parenthesized when `expr` is an
/// operator or block-like expression.
fn postfix(expr: &HirExpr, text: String) -> String {
    match expr.kind {
        HirExprKind::Binary { .. } | HirExprKind::Unary { .. } => format!("({text})"),
        _ if is_block_like(expr) => format!("({text})"),
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
