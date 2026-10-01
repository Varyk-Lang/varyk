//! Expression emission with the type-bridging rule (spec 6.3).
//!
//! Every expression has a generated Rust type ([`Have`]) and every context
//! a required one ([`Need`]); [`FnEmitter::bridge`] inserts `&`, `&mut`,
//! `*`, `.as_str()`, or `.to_string()` to get from one to the other,
//! relying on Rust's reborrowing and deref coercion where a reference
//! already has the right shape.

use std::cell::Cell;

use varyk_syntax::{BinaryOp, UnaryOp};

use super::rust::{item_path, rust_type, struct_path};
use crate::builtins::{Owner, Receiver, ResultKind, Shape};
use crate::hir::{
    AssertKind, HirArm, HirBlock, HirExpr, HirExprKind, HirFunction, HirLiteral, HirPattern,
    HirProgram, LocalId, LocalKind, MethodRef, StringRepr, VariantRef, declared_inside, field_root,
    is_block_like, is_looked_into, leaves, matched_in_place, rooted_argument,
};
use crate::resolve::{Callee, ModuleId, UserType, VariantFieldsDef};
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
    /// A reference-typed local, or a call with `rooted`.
    Ref { mutable: bool },
    /// A `&str`: a string literal, a `&str` local, or a `string` call with
    /// `rooted`.
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
    /// How many casts enclose the expression being emitted: under one,
    /// every integer literal carries its type suffix, since rustc types an
    /// unsuffixed literal from the cast target, through blocks and `-`
    /// (`{ 300 } as u8`, `- -1 as u8`).
    in_cast: Cell<u32>,
}

impl<'a> FnEmitter<'a> {
    pub(super) fn new(program: &'a HirProgram, function: &'a HirFunction) -> Self {
        FnEmitter {
            program,
            module: function.module,
            function,
            in_cast: Cell::new(0),
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
            // Part of its argument, already a reference (M4 spec 3.1, 5):
            // a `&str` for text. (A looked-into result is an `Option` of a
            // reference, written as it is in a head.)
            _ if rooted_argument(expr).is_some() && !is_looked_into(expr) => match expr.ty {
                Ty::String => Have::Str,
                _ => Have::Ref { mutable: false },
            },
            _ => Have::Temp,
        }
    }

    /// What the function's tail and `return` operands must produce: its
    /// owned value, or a shared reference for a borrowed return (M4 spec
    /// 3.1), which Rust's coercion makes a `&str` from a `&String`.
    pub(super) fn return_need(&self) -> Need {
        match (&self.function.ret, self.function.ret_root) {
            (Ty::Unit, _) => Need::AsIs,
            (_, Some(_)) => Need::Shared { binding: false },
            (_, None) => Need::Value,
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
                self.else_branch(&mut out, else_.as_ref(), need, indent);
                out
            }
            HirExprKind::IfLet {
                pattern,
                value,
                then,
                else_,
                head_is_str,
            } => {
                let need = self.branch_need(expr, need);
                let (head, copies) = self.let_head(value, pattern, *head_is_str, indent);
                let mut out = format!(
                    "if let {} = {head} {}",
                    self.pattern(pattern),
                    self.block_body(then, &copies, need, indent)
                );
                self.else_branch(&mut out, else_.as_ref(), need, indent);
                out
            }
            HirExprKind::Block(block) => self.block(block, self.branch_need(expr, need), indent),
            HirExprKind::Match {
                scrutinee,
                arms,
                head_is_str,
            } => self.match_expr(
                scrutinee,
                arms,
                *head_is_str,
                self.branch_need(expr, need),
                indent,
            ),
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
        // Part of its argument, and already a reference (M4 spec 2.8).
        if rooted_argument(leaf).is_some() {
            return false;
        }
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
                // A stored `Option<i32>` taken by `unwrap_or`, say, is
                // copied out as a number is (M4 spec 3.4).
                Have::Ref { .. } if expr.ty.copy_in_rust() => format!("*{text}"),
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
                // Only a program `check` rejects lends a shared reference
                // mutably; spelled so, rustc names the same fault (E0596).
                Have::Ref { mutable: false } => format!("&mut *{text}"),
                Have::Ref { mutable: true } => text,
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
                Ty::Int(IntKind::I32) if self.in_cast.get() == 0 => text.clone(),
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
            HirExprKind::Call {
                callee,
                args,
                started: true,
                ..
            } => {
                let (path, modes) = self.callee(*callee);
                let args: Vec<&HirExpr> = args.iter().collect();
                self.started_call(&path, &args, &modes, indent)
            }
            HirExprKind::MethodCall {
                receiver,
                method,
                args,
                started: true,
                ..
            } => {
                let callee = match method {
                    MethodRef::Varyk(id) => Callee::Varyk(*id),
                    MethodRef::Imported(id) => Callee::Imported(*id),
                    MethodRef::Builtin(id) => Callee::Builtin(*id),
                };
                let (path, modes) = self.callee(callee);
                let args: Vec<&HirExpr> = std::iter::once(&**receiver).chain(args).collect();
                self.started_call(&path, &args, &modes, indent)
            }
            HirExprKind::Call { callee, args, .. } => {
                let (mut path, modes) = self.callee(*callee);
                // `json::parse` and `env::parse` name the type they read (M5a spec 7.5).
                if let (Callee::Builtin(id), Ty::Result(read, _)) = (callee, &expr.ty) {
                    if matches!(id.get().owner, Owner::Json | Owner::Env) {
                        let read = rust_type(self.program, read, self.module);
                        path = format!("{path}::<{read}>");
                    }
                }
                // `Task::all` over tasks giving a `Result` returns at the
                // first `Err`, `varyk-std`'s `try_all`; `Task::all_settled`
                // is the plain `all` (milestone 5b1 spec 5).
                if let Callee::Builtin(id) = callee {
                    if id.get().owner == Owner::Task {
                        let name = match expr.ty {
                            Ty::Result(..) => "try_all",
                            _ => "all",
                        };
                        path = format!("::varyk_std::Task::{name}");
                    }
                }
                format!("{path}({})", self.args(args, &modes, indent))
            }
            HirExprKind::MethodCall {
                receiver,
                method,
                args,
                ..
            } => self.method_call(receiver, *method, args, &expr.ty, indent),
            HirExprKind::Field { .. } | HirExprKind::Index { .. } => {
                self.place(expr, false, indent)
            }
            HirExprKind::StructLit { id, fields } => {
                let path = struct_path(self.program, *id, self.module);
                if fields.is_empty() {
                    return format!("{path} {{}}");
                }
                let defs = &self.program.structs[id.0 as usize].fields;
                let fields: Vec<String> = fields
                    .iter()
                    .map(|(index, value)| {
                        format!(
                            "{}: {}",
                            defs[*index].name,
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
                let (lhs, rhs) = self.binary_operands(*op, lhs, rhs, indent);
                format!("{lhs} {} {rhs}", op.as_str())
            }
            HirExprKind::Assert {
                cond,
                location,
                kind,
            } => self.assert_call(cond, location, *kind, indent),
            HirExprKind::Println { format, args } => {
                self.format_call("println", format, args, indent)
            }
            HirExprKind::Log {
                level,
                format,
                args,
            } => self.log_call(level, format, args, indent),
            HirExprKind::Format { format, args } => {
                self.format_call("format", format, args, indent)
            }
            HirExprKind::EnumLit {
                variant,
                args,
                fields: Some(fields),
                ..
            } => {
                let path = self.variant_path(*variant);
                let names = self.field_names(*variant);
                let values: Vec<String> = args
                    .iter()
                    .zip(fields)
                    .map(|(arg, field)| {
                        // The type checker gives each position a field of
                        // this variant, so a name is always found.
                        let name = names.get(*field).map_or("", String::as_str);
                        format!("{name}: {}", self.expr(arg, Need::Value, indent))
                    })
                    .collect();
                format!("{path} {{ {} }}", values.join(", "))
            }
            HirExprKind::EnumLit {
                variant,
                args,
                fields: None,
                write_full_type,
            } => {
                let mut path = self.variant_path(*variant);
                if *write_full_type {
                    path.push_str(&self.full_type_arguments(*variant, &expr.ty));
                }
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
            HirExprKind::Try { operand, .. } => {
                let text = self.expr(operand, Need::Value, indent);
                format!("{}?", postfix(operand, text))
            }
            // The operand is a call, which `.await` follows as it is.
            HirExprKind::Await(operand) => format!("{}.await", self.raw(operand, indent)),
            // Written with its own parentheses, so no context regroups it.
            HirExprKind::Cast { expr: operand, ty } => {
                self.in_cast.set(self.in_cast.get() + 1);
                let text = self.operand(operand, None, Need::Value, indent);
                self.in_cast.set(self.in_cast.get() - 1);
                format!("({text} as {})", rust_type(self.program, ty, self.module))
            }
            HirExprKind::VecLit(elements) => {
                let elements: Vec<String> = elements
                    .iter()
                    .map(|element| self.expr(element, Need::Value, indent))
                    .collect();
                format!("::std::vec![{}]", elements.join(", "))
            }
            HirExprKind::Closure {
                param,
                body,
                returns_part,
                ..
            } => self.closure(*param, body, *returns_part, indent),
            HirExprKind::Block(_)
            | HirExprKind::If { .. }
            | HirExprKind::IfLet { .. }
            | HirExprKind::Match { .. } => {
                unreachable!("`expr` emits blocks, `if`s, `if let`s, and `match`es itself")
            }
        }
    }

    /// `|param| body` (M4 spec 5): a body written as an expression as it
    /// is, a block as a block, starting with `let x = *x;` when borrow
    /// analysis says the parameter comes one reference deeper than its
    /// Varyk kind. The value the body gives is its own, or, for a chain's
    /// `map` returning part of something (`returns_part`), a reference to
    /// that part, as a borrowed return's (M4 spec 3.1, 3.3).
    fn closure(
        &self,
        param: LocalId,
        body: &HirBlock,
        returns_part: bool,
        indent: usize,
    ) -> String {
        let name = self.local_name(param);
        let deref = matches!(
            self.function.locals[param.0 as usize].kind,
            LocalKind::ClosureParam { deref: true }
        );
        let copies = if deref {
            vec![format!("let {name} = *{name};")]
        } else {
            Vec::new()
        };
        let need = match (returns_part, &body.ty) {
            (false, _) => Need::Value,
            (true, Ty::String) => Need::Str,
            (true, _) => Need::Shared { binding: false },
        };
        let text = match body.tail.as_deref() {
            // A block of just its tail, with the tail's span: an expression.
            Some(tail) if tail.span == body.span && copies.is_empty() => {
                self.expr(tail, need, indent)
            }
            _ => self.block_body(body, &copies, need, indent),
        };
        format!("|{name}| {text}")
    }

    /// The turbofish of a built-in constructor that is the operand of `?`:
    /// `::<T, E>` for `Ok` and `Err`, `::<T>` for `None`, from `ty`, the
    /// type of the whole constructor.
    fn full_type_arguments(&self, variant: VariantRef, ty: &Ty) -> String {
        let spell = |ty: &Ty| rust_type(self.program, ty, self.module);
        match (variant, ty) {
            (VariantRef::Ok | VariantRef::Err, Ty::Result(ok, err)) => {
                format!("::<{}, {}>", spell(ok), spell(err))
            }
            (VariantRef::None, Ty::Option(inner)) => format!("::<{}>", spell(inner)),
            _ => String::new(),
        }
    }

    /// `else` and its block after an `if` or `if let` in `out`: `else if`
    /// or `else if let` for a block that is only one of those.
    fn else_branch(&self, out: &mut String, else_: Option<&HirBlock>, need: Need, indent: usize) {
        let Some(else_) = else_ else {
            return;
        };
        out.push_str(" else ");
        match else_.tail.as_deref() {
            Some(
                tail @ HirExpr {
                    kind: HirExprKind::If { .. } | HirExprKind::IfLet { .. },
                    ..
                },
            ) if else_.stmts.is_empty() => {
                out.push_str(&self.expr(tail, need, indent));
            }
            _ => out.push_str(&self.block(else_, need, indent)),
        }
    }

    /// The head of an `if let` or `while let` as [`FnEmitter::head`]
    /// writes it, and the statements [`FnEmitter::copies`] gives
    /// `pattern`.
    pub(super) fn let_head(
        &self,
        head: &HirExpr,
        pattern: &HirPattern,
        head_is_str: bool,
        indent: usize,
    ) -> (String, Vec<String>) {
        (
            self.head(head, head_is_str, indent),
            self.copies(head, pattern, head_is_str),
        )
    }

    /// The head of a `match`, `if let`, or `while let` (spec 5, M4 spec
    /// 5): a place is looked at through a shared reference (`&v` for an
    /// owned place, `v` for a `&T`, `&*v` for a `&mut T`), and so is a
    /// temporary with an enum that has a destructor inside it; any other
    /// temporary as it is; a looked-into result as it is, since it already
    /// holds a reference, and a borrowed-return result as the reference it
    /// is (never copied out, even when Copy in Rust, so a binding is a
    /// reference the arm copies); a string (`head_is_str`) as exactly a `&str`,
    /// because a string literal pattern sees through nothing else.
    fn head(&self, head: &HirExpr, head_is_str: bool, indent: usize) -> String {
        let need = if head_is_str {
            Need::Str
        } else if is_looked_into(head) {
            // Already holds a reference into its receiver (M4 spec 5).
            Need::Value
        } else if matched_in_place(&self.program.enums, head) {
            Need::Shared { binding: true }
        } else {
            Need::Value
        };
        self.expr(head, need, indent)
    }

    /// `let n = *n;` for each Copy value `pattern` binds through a
    /// reference, at any depth: the one [`FnEmitter::head`] makes of a
    /// place, or the one a looked-into result holds.
    fn copies(&self, head: &HirExpr, pattern: &HirPattern, head_is_str: bool) -> Vec<String> {
        if head_is_str || !matched_in_place(&self.program.enums, head) {
            return Vec::new();
        }
        pattern
            .bindings()
            .into_iter()
            .filter(|(local, _)| self.function.locals[local.0 as usize].ty.is_copy())
            .map(|(local, _)| {
                let name = self.local_name(local);
                format!("let {name} = *{name};")
            })
            .collect()
    }

    /// `match head { arms }` (spec 5), its head as [`FnEmitter::head`]
    /// writes it. A Copy value bound through a reference is copied at the
    /// start of its arm with `let n = *n;`, which makes an expression arm
    /// a block. Each arm's value is emitted as `need` says.
    fn match_expr(
        &self,
        head: &HirExpr,
        arms: &[HirArm],
        head_is_str: bool,
        need: Need,
        indent: usize,
    ) -> String {
        let inner = indent + 1;
        let pad = "    ".repeat(inner);
        let mut out = format!("match {} {{\n", self.head(head, head_is_str, indent));
        for arm in arms {
            let copies = self.copies(head, &arm.pattern, head_is_str);
            let body = match &arm.body.kind {
                HirExprKind::Block(block) => self.block_body(block, &copies, need, inner),
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
    /// module, and literals as their values.
    pub(super) fn pattern(&self, pattern: &HirPattern) -> String {
        match pattern {
            HirPattern::Wildcard => "_".to_string(),
            HirPattern::Binding(local) => self.local_name(*local).to_string(),
            HirPattern::Variant { variant, fields } => {
                let path = self.variant_path(*variant);
                if fields.is_empty() {
                    return path;
                }
                let fields: Vec<String> = fields.iter().map(|field| self.pattern(field)).collect();
                format!("{path}({})", fields.join(", "))
            }
            HirPattern::Struct { variant, fields } => {
                let fields: Vec<String> = fields
                    .iter()
                    .map(|(name, field)| format!("{name}: {}", self.pattern(field)))
                    .collect();
                format!(
                    "{} {{ {} }}",
                    self.variant_path(*variant),
                    fields.join(", ")
                )
            }
            HirPattern::Literal(HirLiteral::Int(value)) => value.to_string(),
            HirPattern::Literal(HirLiteral::Bool(value)) => value.to_string(),
            HirPattern::Literal(HirLiteral::Str(text)) => format!("\"{text}\""),
            HirPattern::Range { lo, hi } => format!("{lo}..={hi}"),
        }
    }

    /// The field names of `variant`, a variant with named fields, in
    /// declaration order.
    fn field_names(&self, variant: VariantRef) -> Vec<String> {
        let VariantRef::User(id, index) = variant else {
            return Vec::new();
        };
        match self.program.enums[id.0 as usize]
            .variants
            .get(index)
            .map(|variant| &variant.fields)
        {
            Some(VariantFieldsDef::Named(fields)) => {
                fields.iter().map(|(name, _)| name.clone()).collect()
            }
            _ => Vec::new(),
        }
    }

    /// A variant's path from this module: `Shape::Point`,
    /// `crate::geo::Shape::Point`, or `Some`.
    fn variant_path(&self, variant: VariantRef) -> String {
        match variant {
            VariantRef::User(id, index) => {
                let e = &self.program.enums[id.0 as usize];
                let owner = item_path(self.program, e.module, self.module, &e.name);
                format!("{owner}::{}", e.variants[index].name)
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

    /// A method call of type `ty`. A Varyk or imported method is called by
    /// its path, `Type::name(receiver, args)`, the receiver lent as its
    /// `self` needs: in the `receiver.name(args)` form Rust's method lookup
    /// can pick a trait method of the same name over the inherent one Varyk
    /// checked (a by-value `Into::into`, or a `&self` `Clone::clone` over
    /// a `&mut self` method).
    ///
    /// A builtin is `receiver.name(args)`: Rust's method call borrows the
    /// receiver as the method's `self` needs, so a place receiver is
    /// written as it is (never moved); a string receiver that is a `&str`
    /// has `clone` spelled `.to_string()`, since cloning a `&str` copies
    /// only the reference (spec 3.5). A read `string` argument is a
    /// `&str`, a read `T` a `&T`, and every other argument its value (M4
    /// spec 2.7); `parse` is `::varyk_std::parse::<T>(&s)`, and `contains`
    /// on a `Vec<string>` compares each element with the argument at one
    /// reference depth, since `Vec<String>::contains` wants a `&String`.
    /// `get` on a Copy payload is followed by `.copied()`, and an `if` or
    /// block receiver of a taking row is taken by value.
    fn method_call(
        &self,
        receiver: &HirExpr,
        method: MethodRef,
        args: &[HirExpr],
        ty: &Ty,
        indent: usize,
    ) -> String {
        let id = match method {
            MethodRef::Varyk(id) => {
                return self.path_call(Callee::Varyk(id), receiver, args, indent);
            }
            MethodRef::Imported(id) => {
                return self.path_call(Callee::Imported(id), receiver, args, indent);
            }
            MethodRef::Builtin(id) => id,
        };
        let entry = id.get();
        let name = entry.name;
        let mutable = entry.receiver == Receiver::Changes;
        let element = Owner::of(&receiver.ty).map_or(Ty::Unit, |(_, subst)| subst.t);
        let args: Vec<String> = args
            .iter()
            .zip(entry.params)
            .map(|(arg, shape)| {
                let need = match shape {
                    Shape::ReadString => Need::Str,
                    Shape::ReadT if element == Ty::String => Need::Str,
                    Shape::ReadT => Need::Shared { binding: false },
                    _ => Need::Value,
                };
                self.expr(arg, need, indent)
            })
            .collect();
        let args = args.join(", ");
        if entry.owner == Owner::Vec && name == "contains" && element == Ty::String {
            // Both sides are evaluated, the receiver first, before the
            // names below are in scope, so a user name in either is never
            // shadowed. In parentheses, since a `match` starting a
            // statement ends there (`match .. {} == false;` does not parse).
            let haystack = self.expr(receiver, Need::Shared { binding: false }, indent);
            return format!(
                "(match ({haystack}, {args}) {{ (haystack, needle) => \
                 haystack.iter().any(|e| e == needle) }})"
            );
        }
        if entry.receiver == Receiver::Takes && is_block_like(receiver) {
            // Used up: an `if` or block is taken by value, each branch
            // moved or copied out (M4 spec 3.4).
            let receiver = self.expr(receiver, Need::Value, indent);
            return format!("({receiver}).{name}({args})");
        }
        if receiver.ty != Ty::String {
            let receiver = self.field_base(receiver, mutable, indent);
            // A Copy payload is copied out (M4 spec 2.8), and so are the
            // Copy items of a source (M4 spec 3.3); a `find` on copies
            // already holds one.
            let copied = match ty {
                Ty::Option(payload)
                    if entry.result_kind == ResultKind::LookInside
                        && entry.owner != Owner::Chain
                        && payload.is_copy() =>
                {
                    ".copied()"
                }
                Ty::Chain(item) if entry.owner != Owner::Chain && item.is_copy() => ".copied()",
                _ => "",
            };
            // The types rustc cannot take from anywhere else (M4 spec 3.3).
            let turbofish = match (entry.owner, name) {
                (Owner::Chain, "collect") => "::<Vec<_>>".to_string(),
                (Owner::Chain, "sum") => {
                    format!("::<{}>", rust_type(self.program, ty, self.module))
                }
                _ => String::new(),
            };
            return format!("{receiver}.{name}{turbofish}({args}){copied}");
        }
        // A string changed in place (`push_str`) must be a `String`: an
        // `if` or block is reached mutably as a field base is, each branch
        // reborrowed, and a literal is lent as to a `mut string` parameter.
        let (receiver, is_str) = if mutable && is_block_like(receiver) {
            (self.field_base(receiver, true, indent), false)
        } else if mutable && self.have(receiver) == Have::Str {
            let lent = self.expr(receiver, Need::Mut { binding: false }, indent);
            (format!("({lent})"), false)
        } else if is_block_like(receiver) {
            (
                format!("({})", self.expr(receiver, Need::Str, indent)),
                true,
            )
        } else if self.have(receiver) == Have::Str {
            (self.raw(receiver, indent), true)
        } else {
            (self.field_base(receiver, mutable, indent), false)
        };
        match (name, ty) {
            // Every `.clone()` is recorded as the `string` `clone` row; only
            // a `string` receiver can be a `&str` (`is_str`) and reach this
            // arm.
            ("clone", _) if is_str => format!("{receiver}.to_string()"),
            // `varyk-std`'s `parse`, whose error says what the text is not
            // (M5a spec 2.8); it reads a `&str`.
            ("parse", Ty::Result(read, _)) => {
                let text = if is_str {
                    receiver
                } else {
                    format!("&{receiver}")
                };
                format!(
                    "::varyk_std::parse::<{}>({text})",
                    rust_type(self.program, read, self.module)
                )
            }
            _ => format!("{receiver}.{name}({args})"),
        }
    }

    /// `Type::name(receiver, args)` for the method `callee`.
    fn path_call(
        &self,
        callee: Callee,
        receiver: &HirExpr,
        args: &[HirExpr],
        indent: usize,
    ) -> String {
        let (path, modes) = self.callee(callee);
        let receiver = self.args(std::slice::from_ref(receiver), &modes[..1], indent);
        if args.is_empty() {
            return format!("{path}({receiver})");
        }
        format!(
            "{path}({receiver}, {})",
            self.args(args, &modes[1..], indent)
        )
    }

    /// The operands of `lhs op rhs` as written around `op`: those of
    /// `==` and `!=` brought to a form Rust compares (M4 spec 5).
    fn binary_operands(
        &self,
        op: BinaryOp,
        lhs: &HirExpr,
        rhs: &HirExpr,
        indent: usize,
    ) -> (String, String) {
        // The operands of `==` and `!=` on a struct, an enum, or a
        // container are brought to one reference depth (M4 spec 5): both
        // borrowed when either already is a reference or is block-like
        // (read by reference, so no branch is moved), else both as they
        // are, which Rust compares in place without moving.
        let by_ref = |operand: &HirExpr| {
            is_block_like(operand) || matches!(self.have(operand), Have::Ref { .. })
        };
        let shared = lhs.ty.is_compound() && (by_ref(lhs) || by_ref(rhs));
        // String operands (of `==` and `!=`) are normalized to `&str`
        // only as needed: a `String` and a `&str` compare in any mix, a
        // `&String` or a block-like one does not.
        let need = |operand: &HirExpr| {
            if shared {
                Need::Shared { binding: false }
            } else if operand.ty != Ty::String {
                Need::Value
            } else if is_block_like(operand) || matches!(self.have(operand), Have::Ref { .. }) {
                Need::Str
            } else {
                Need::AsIs
            }
        };
        (
            self.operand(lhs, Some((op, false)), need(lhs), indent),
            self.operand(rhs, Some((op, true)), need(rhs), indent),
        )
    }

    /// `assert(cond)` or `assert_eq(a, b)` (M5a spec 7.5) as
    /// `::std::assert!`, by its full path like every std macro, with a
    /// message naming the Varyk `location`. `assert_eq` evaluates each
    /// operand once, before anything else, as Rust's own `assert_eq!`
    /// does: it matches on references to both and compares what they
    /// point to, written as `==` writes it; the message shows both values
    /// when they print with `{}` (Varyk types have no `Debug`).
    fn assert_call(
        &self,
        cond: &HirExpr,
        location: &str,
        kind: AssertKind,
        indent: usize,
    ) -> String {
        let message = format_literal(&format!("assertion failed at {location}"));
        let (AssertKind::Eq { show }, HirExprKind::Binary { op, lhs, rhs }) = (kind, &cond.kind)
        else {
            let cond = self.expr(cond, Need::Value, indent);
            return format!("::std::assert!({cond}, \"{message}\")");
        };
        let (lhs, rhs) = self.binary_operands(*op, lhs, rhs, indent);
        let shown = if show {
            ": left is {}, right is {}\", varyk_left, varyk_right"
        } else {
            "\""
        };
        format!(
            "match (&({lhs}), &({rhs})) {{ (varyk_left, varyk_right) => \
             ::std::assert!(*varyk_left == *varyk_right, \"{message}{shown}) }}"
        )
    }

    /// `println!` or `format!` (`name`): every argument as it is, since
    /// the format machinery borrows it. Every std macro is written by its
    /// full path, `::std::println!`: a bare name could be a
    /// `#[macro_export]` macro of a `.rs` module of the package.
    fn format_call(&self, name: &str, format: &str, args: &[HirExpr], indent: usize) -> String {
        let mut out = format!("{}!(\"{format}\"", macro_path(name));
        for arg in args {
            out.push_str(", ");
            out.push_str(&self.expr(arg, Need::AsIs, indent));
        }
        out.push(')');
        out
    }

    /// `log::<level>(..)` (M5a spec 2.6) as tracing's macro. tracing reads
    /// its arguments only when the level is on, while a Varyk call reads
    /// them as `println!` does, whatever `LOG` says: each argument is
    /// evaluated first, once, by matching on references to all of them,
    /// and the macro formats what they point to.
    fn log_call(&self, level: &str, format: &str, args: &[HirExpr], indent: usize) -> String {
        let name = format!("tracing::{level}");
        if args.is_empty() {
            return self.format_call(&name, format, args, indent);
        }
        let values: Vec<String> = args
            .iter()
            .map(|arg| format!("&({})", self.expr(arg, Need::AsIs, indent)))
            .collect();
        let names: Vec<String> = (0..args.len()).map(|i| format!("varyk_{i}")).collect();
        format!(
            "match ({},) {{ ({},) => {}!(\"{format}\", {}) }}",
            values.join(", "),
            names.join(", "),
            macro_path(&name),
            names.join(", ")
        )
    }

    /// A started call of `path` (milestone 5b1 spec 5): every argument is
    /// evaluated in order into `varyk_N` by a `match`, as a direct call
    /// would evaluate it, before anything is bound; the task then owns
    /// them and passes each as its parameter takes it, by value to an
    /// owned one and by reference otherwise.
    fn started_call(
        &self,
        path: &str,
        args: &[&HirExpr],
        modes: &[ParamMode],
        indent: usize,
    ) -> String {
        let values: Vec<String> = args
            .iter()
            .map(|arg| format!("{},", self.expr(arg, Need::Value, indent)))
            .collect();
        let names: Vec<String> = (0..args.len()).map(|i| format!("varyk_{i},")).collect();
        let passed: Vec<String> = modes
            .iter()
            .enumerate()
            .map(|(i, mode)| match mode {
                ParamMode::Owned => format!("varyk_{i}"),
                _ => format!("&varyk_{i}"),
            })
            .collect();
        format!(
            "match ({}) {{ ({}) => ::varyk_std::Task::start(async move {{ {path}({}).await }}) }}",
            values.join(" "),
            names.join(" "),
            passed.join(", ")
        )
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
            | HirExprKind::Try { .. }
            | HirExprKind::Await(_) => self.raw(base, indent),
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
                // An associated function is reached through its struct.
                let path = match sig.owner {
                    None => item_path(self.program, sig.module, self.module, &sig.name),
                    Some(owner) => {
                        let owner = struct_path(self.program, owner, self.module);
                        format!("{owner}::{}", sig.name)
                    }
                };
                (path, sig.modes())
            }
            // In full, so that no `use` line of a `.rs` module can shadow it.
            Callee::Builtin(id) if id.get().owner == Owner::HashMap => {
                (format!("::std::collections::{}", id.path()), id.modes())
            }
            // Absolute, so that no module of the program named `varyk_std`
            // can shadow it (M5a spec 7.5).
            Callee::Builtin(id)
                if matches!(
                    id.get().owner,
                    Owner::Error
                        | Owner::Json
                        | Owner::Env
                        | Owner::Log
                        | Owner::Time
                        | Owner::Task
                ) =>
            {
                (format!("::varyk_std::{}", id.path()), id.modes())
            }
            // Std's own pointer, in full (milestone 5b1 spec 5).
            Callee::Builtin(id) if id.get().owner == Owner::Shared => {
                ("::std::sync::Arc::new".to_string(), id.modes())
            }
            Callee::Builtin(id) => (id.path(), id.modes()),
        }
    }
}

/// The full path of the macro `name`: `tracing::info` lives in
/// `varyk-std`, the others in `std`.
fn macro_path(name: &str) -> String {
    match name.strip_prefix("tracing::") {
        Some(level) => format!("::varyk_std::tracing::{level}"),
        None => format!("::std::{name}"),
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

/// `text` as the inside of a Rust string literal used as a format string:
/// escaped as Rust writes it, with `{` and `}` doubled.
fn format_literal(text: &str) -> String {
    let escaped = format!("{:?}", text.replace('{', "{{").replace('}', "}}"));
    escaped[1..escaped.len() - 1].to_string()
}
