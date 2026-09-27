//! The Rust backend (spec 6.4): crate, module, item, and statement
//! emission. Expressions are in `rust_expr`.
//!
//! The output is deterministic and readable without rustfmt: four-space
//! indentation, one statement per line, a blank line between items, and a
//! trailing newline. Every generated `fn`, `struct`, `enum`, `impl`, and
//! `use` carries [`ALLOW_ITEM`] (spec 2.3); no `mod` line does, so a
//! copied `.rs` module, wherever it sits in the tree, warns and fails as
//! Rust normally does.

use super::rust_expr::{FnEmitter, Need, Repr};
use super::writer::Writer;
use super::{Backend, CrateInfo, GeneratedCrate};
use crate::hir::{
    HirBlock, HirEnum, HirExprKind, HirForHead, HirFunction, HirModule, HirModuleKind, HirProgram,
    HirStmt, HirStruct, is_place,
};
use crate::resolve::{ModuleId, StructId, UserType};
use crate::types::{ParamMode, Ty};
use varyk_syntax::Span;

/// The item-level attribute every generated `fn`, `struct`, `enum`,
/// `impl`, and `use` carries (spec 2.3): the `warnings`
/// group covers every warn-level lint, and the two deny-by-default lints
/// are named because a group cannot include them. The program still
/// panics at run time on overflow or division by zero (M1 §6.4).
const ALLOW_ITEM: &str = "#[allow(warnings, arithmetic_overflow, unconditional_panic)]";

/// Generates a Cargo project whose `src/` mirrors the program's modules.
pub struct RustBackend;

impl Backend for RustBackend {
    fn generate(&self, program: &HirProgram, info: &CrateInfo) -> GeneratedCrate {
        let cargo_toml = info.manifest.to_string();

        // Each `.vr` module becomes its own file at its tree path, holding
        // the `mod` lines of the modules it declares; a `.rs` module is
        // copied to its tree path (spec 2.3, 3.1).
        let mut files = Vec::new();
        let mut copied = Vec::new();
        for module in &program.modules {
            match &module.kind {
                HirModuleKind::Varyk => {
                    let mut writer = Writer::new(format!("src/{}", module.path));
                    let has_mods = write_mod_lines(program, module.id, &mut writer);
                    let has_uses = !module.uses.is_empty();
                    let has_items = program.structs.iter().any(|s| s.module == module.id)
                        || program.enums.iter().any(|e| e.module == module.id)
                        || program.functions().any(|f| f.module == module.id);
                    if has_mods && (has_uses || has_items) {
                        writer.line(0, "", None);
                    }
                    write_use_lines(module, &mut writer);
                    if has_uses && has_items {
                        writer.line(0, "", None);
                    }
                    write_module_items(program, module.id, &mut writer);
                    files.push(writer.finish());
                }
                HirModuleKind::Rust { path, .. } => {
                    copied.push((format!("src/{}", module.path), path.clone()));
                }
            }
        }

        GeneratedCrate {
            package_name: info.name.clone(),
            cargo_toml,
            files,
            copied,
        }
    }
}

/// Writes the `mod` line of every module `module` declares, `pub mod` for
/// a `pub` one; the caller adds any blank line that follows. No `mod`
/// line carries [`ALLOW_ITEM`]: a lint attribute there would reach every
/// `.rs` module nested below, silencing its warnings and its deny-level
/// lints, and every generated item carries its own. The one lint about a
/// `mod` line itself, a `.vr` module name not in snake case, is allowed
/// on that line, which Rust applies to the module and those inside it.
/// Returns whether it wrote anything.
fn write_mod_lines(program: &HirProgram, module: ModuleId, writer: &mut Writer) -> bool {
    let mut any = false;
    for child in program.modules.iter().filter(|m| m.parent == Some(module)) {
        if matches!(child.kind, HirModuleKind::Varyk) && !is_snake_case(&child.name) {
            writer.line(0, "#[allow(non_snake_case)]", None);
        }
        let vis = if child.is_pub { "pub " } else { "" };
        writer.line(0, &format!("{vis}mod {};", child.name), None);
        any = true;
    }
    any
}

/// Whether rustc's `non_snake_case` lint accepts `name`: no uppercase
/// letter and no `__` once leading and trailing `_` are trimmed.
fn is_snake_case(name: &str) -> bool {
    let name = name.trim_matches('_');
    !name.contains("__") && !name.chars().any(|c| c.is_ascii_uppercase())
}

/// Writes every `use` line of `module` (spec 3.3) as the canonical
/// `crate::`-rooted path to what it names, with `as alias` kept when the
/// user wrote one, each carrying [`ALLOW_ITEM`] like every other
/// generated item. Every use site is written with its full path, so the
/// alias never affects the generated Rust.
fn write_use_lines(module: &HirModule, writer: &mut Writer) {
    for use_ in &module.uses {
        writer.line(0, ALLOW_ITEM, None);
        let line = match &use_.alias {
            Some(alias) => format!("use {} as {alias};", use_.path),
            None => format!("use {};", use_.path),
        };
        writer.line(0, &line, Some(use_.span));
    }
}

/// One item of a module: a Varyk-declared struct, enum, or free function,
/// or the merged `impl` block of a type's methods (spec 6.4: multiple
/// `impl` blocks of the same type become one).
enum Item<'p> {
    Struct(&'p HirStruct),
    Enum(&'p HirEnum),
    Fn(&'p HirFunction),
    Impl(UserType, Vec<&'p HirFunction>),
}

/// Writes a Varyk module's structs, enums, `impl` blocks, and functions in
/// source order, separated by blank lines, each item preceded by
/// [`ALLOW_ITEM`].
fn write_module_items(program: &HirProgram, module: ModuleId, writer: &mut Writer) {
    // The HIR keeps structs, enums, and functions apart; their spans (in
    // the same file) restore the source order.
    let mut items: Vec<(u32, Item)> = program
        .structs
        .iter()
        .filter(|s| s.module == module && !s.imported)
        .map(|s| (s.span.start, Item::Struct(s)))
        .collect();
    items.extend(
        program
            .enums
            .iter()
            .filter(|e| e.module == module && !e.imported)
            .map(|e| (e.span.start, Item::Enum(e))),
    );
    let mut impls: Vec<(UserType, u32, Vec<&HirFunction>)> = Vec::new();
    for function in program.functions().filter(|f| f.module == module) {
        let start = function.span.start;
        match function.owner {
            None => items.push((start, Item::Fn(function))),
            Some(owner) => match impls.iter_mut().find(|(o, ..)| *o == owner) {
                Some((_, _, fns)) => fns.push(function),
                None => impls.push((owner, start, vec![function])),
            },
        }
    }
    for (owner, start, fns) in impls {
        items.push((start, Item::Impl(owner, fns)));
    }
    items.sort_by_key(|(start, _)| *start);

    for (index, (_, item)) in items.into_iter().enumerate() {
        if index > 0 {
            writer.line(0, "", None);
        }
        match item {
            Item::Struct(s) => emit_struct(program, s, writer),
            Item::Enum(e) => emit_enum(program, e, writer),
            Item::Fn(f) => {
                writer.line(0, ALLOW_ITEM, None);
                FnEmitter::new(program, f).function(writer, 0);
            }
            Item::Impl(owner, fns) => {
                let name = match owner {
                    UserType::Struct(id) => &program.structs[id.0 as usize].name,
                    UserType::Enum(id) => &program.enums[id.0 as usize].name,
                };
                writer.line(0, ALLOW_ITEM, None);
                writer.line(0, &format!("impl {name} {{"), None);
                for (index, f) in fns.into_iter().enumerate() {
                    if index > 0 {
                        writer.line(0, "", None);
                    }
                    FnEmitter::new(program, f).function(writer, 1);
                }
                writer.line(0, "}", None);
            }
        }
    }
}

fn emit_struct(program: &HirProgram, s: &HirStruct, writer: &mut Writer) {
    writer.line(0, ALLOW_ITEM, None);
    let vis = if s.is_pub { "pub " } else { "" };
    if s.fields.is_empty() {
        writer.line(0, &format!("{vis}struct {} {{}}", s.name), Some(s.span));
        return;
    }
    writer.line(0, &format!("{vis}struct {} {{", s.name), Some(s.span));
    for field in &s.fields {
        let vis = if field.is_pub { "pub " } else { "" };
        let ty = rust_type(program, &field.ty, s.module);
        writer.line(1, &format!("{vis}{}: {ty},", field.name), Some(s.span));
    }
    writer.line(0, "}", None);
}

fn emit_enum(program: &HirProgram, e: &HirEnum, writer: &mut Writer) {
    writer.line(0, ALLOW_ITEM, None);
    let vis = if e.is_pub { "pub " } else { "" };
    writer.line(0, &format!("{vis}enum {} {{", e.name), Some(e.span));
    for (name, payload) in &e.variants {
        if payload.is_empty() {
            writer.line(1, &format!("{name},"), Some(e.span));
        } else {
            let types: Vec<String> = payload
                .iter()
                .map(|ty| rust_type(program, ty, e.module))
                .collect();
            writer.line(1, &format!("{name}({}),", types.join(", ")), Some(e.span));
        }
    }
    writer.line(0, "}", None);
}

/// The Rust spelling of a value of type `ty`, from `from`. A `string` is
/// an owned `String` wherever a type is spelled: a field, a payload, a
/// return, and inside `Option`, `Result`, and `Vec`.
fn rust_type(program: &HirProgram, ty: &Ty, from: ModuleId) -> String {
    match ty {
        Ty::Bool => "bool".to_string(),
        Ty::Int(kind) => kind.name().to_string(),
        Ty::Float(kind) => kind.name().to_string(),
        Ty::String => "String".to_string(),
        Ty::Struct(id) => struct_path(program, *id, from),
        Ty::Enum(id) => {
            let e = &program.enums[id.0 as usize];
            item_path(program, e.module, from, &e.name)
        }
        Ty::Option(inner) => format!("Option<{}>", rust_type(program, inner, from)),
        Ty::Result(ok, err) => format!(
            "Result<{}, {}>",
            rust_type(program, ok, from),
            rust_type(program, err, from)
        ),
        Ty::Vec(inner) => format!("Vec<{}>", rust_type(program, inner, from)),
        Ty::Unit => "()".to_string(),
    }
}

/// The Rust type of a parameter of type `ty` passed by `mode`.
fn param_type(program: &HirProgram, ty: &Ty, mode: ParamMode, from: ModuleId) -> String {
    match (mode, ty) {
        (ParamMode::SharedBorrow, Ty::String) => "&str".to_string(),
        (ParamMode::SharedBorrow, _) => format!("&{}", rust_type(program, ty, from)),
        (ParamMode::MutableBorrow, _) => format!("&mut {}", rust_type(program, ty, from)),
        (ParamMode::Owned, _) => rust_type(program, ty, from),
    }
}

/// The path to struct `id` from module `from`.
pub(super) fn struct_path(program: &HirProgram, id: StructId, from: ModuleId) -> String {
    let s = &program.structs[id.0 as usize];
    item_path(program, s.module, from, &s.name)
}

/// The path to item `name` of module `module` from module `from`: bare in
/// the same module, else its full path from the crate root,
/// `crate::shop::cart::Cart`.
pub(super) fn item_path(
    program: &HirProgram,
    module: ModuleId,
    from: ModuleId,
    name: &str,
) -> String {
    if module == from {
        name.to_string()
    } else {
        format!("{}::{name}", program.module_path(module))
    }
}

fn indentation(indent: usize) -> String {
    "    ".repeat(indent)
}

/// Writes `text` (possibly multi-line, e.g. a `while` or `for` body) as
/// physical lines: the first at `indent` levels, the rest as they are
/// (already carrying their own absolute indentation), every line tagged
/// with `span`.
fn push_lines(writer: &mut Writer, indent: usize, text: &str, span: Option<Span>) {
    let mut lines = text.split('\n');
    if let Some(first) = lines.next() {
        writer.line(indent, first, span);
    }
    for line in lines {
        writer.line(0, line, span);
    }
}

/// `stmt`'s own span, for the source map.
fn stmt_span(stmt: &HirStmt) -> Span {
    match stmt {
        HirStmt::Let { span, .. }
        | HirStmt::Assign { span, .. }
        | HirStmt::Expr { span, .. }
        | HirStmt::Return { span, .. }
        | HirStmt::While { span, .. }
        | HirStmt::For { span, .. }
        | HirStmt::Break { span }
        | HirStmt::Continue { span } => *span,
    }
}

impl FnEmitter<'_> {
    /// Writes the function's declaration, starting at `indent` levels (one
    /// inside an `impl` block): the signature and opening brace as one
    /// line carrying the function's span, its body's statements each
    /// carrying their own span, then the closing brace (structural). A
    /// method's receiver is `&self` or `&mut self`.
    pub(super) fn function(&self, writer: &mut Writer, indent: usize) {
        let f: &HirFunction = self.function;
        let vis = if f.is_pub { "pub " } else { "" };
        let params: Vec<String> = f
            .params
            .iter()
            .map(|p| match p.mode {
                _ if f.owner.is_none() || p.name != "self" => {
                    let ty = param_type(self.program, &p.ty, p.mode, self.module);
                    format!("{}: {ty}", p.name)
                }
                ParamMode::MutableBorrow => "&mut self".to_string(),
                _ => "&self".to_string(),
            })
            .collect();
        let ret = match &f.ret {
            Ty::Unit => String::new(),
            ty => format!(" -> {}", rust_type(self.program, ty, self.module)),
        };
        let need = if f.ret == Ty::Unit {
            Need::AsIs
        } else {
            Need::Value
        };
        writer.line(
            indent,
            &format!("{vis}fn {}({}){ret} {{", f.name, params.join(", ")),
            Some(f.span),
        );
        self.block_after(writer, &f.body, need, indent);
    }

    /// Writes `block`'s statements and tail into `writer`, each physical
    /// line tagged with its own HIR node's span, then the closing `}` at
    /// `indent` (structural). The opening `{` is the caller's job, merged
    /// onto its own header line (the function signature, here).
    pub(super) fn block_after(
        &self,
        writer: &mut Writer,
        block: &HirBlock,
        need: Need,
        indent: usize,
    ) {
        let inner = indent + 1;
        for stmt in &block.stmts {
            self.stmt(writer, stmt, inner);
        }
        if let Some(tail) = &block.tail {
            let text = self.expr(tail, need, inner);
            push_lines(writer, inner, &text, Some(tail.span));
        }
        writer.line(indent, "}", None);
    }

    /// Writes one statement's physical lines into `writer` at `indent`,
    /// every line tagged with the statement's own span (even when its
    /// text is multi-line, e.g. a `while` or `for` body: expression
    /// emission below a statement stays string-based, so a nested block's
    /// lines take the statement's span rather than their own).
    fn stmt(&self, writer: &mut Writer, stmt: &HirStmt, indent: usize) {
        let text = self.stmt_text(stmt, indent);
        push_lines(writer, indent, &text, Some(stmt_span(stmt)));
    }

    /// One statement's Rust text, without its leading indentation or
    /// trailing newline (may itself be multi-line, e.g. a `while` or
    /// `for` body).
    fn stmt_text(&self, stmt: &HirStmt, indent: usize) -> String {
        match stmt {
            HirStmt::Let { local, value, .. } => {
                let info = &self.function.locals[local.0 as usize];
                let mutability = if info.mutable { "mut " } else { "" };
                // A value holding `Option`, `Result`, or `Vec` may have
                // taken part of its type from the `let` (`None`, `vec![]`,
                // spec 2.6), so the Rust keeps the type written.
                let ty = match info.ty {
                    Ty::Option(_) | Ty::Result(..) | Ty::Vec(_)
                        if self.repr(*local) == Repr::Owned =>
                    {
                        format!(": {}", rust_type(self.program, &info.ty, self.module))
                    }
                    _ => String::new(),
                };
                let value = self.expr(value, self.binding_need(*local), indent);
                format!("let {mutability}{}{ty} = {value};", info.name)
            }
            HirStmt::Assign { target, value, .. } => {
                let (target, need) = match &target.kind {
                    HirExprKind::Local(id) => {
                        let name = self.local_name(*id);
                        match self.repr(*id) {
                            // Assignment through a reference writes the
                            // referent.
                            Repr::Ref { .. } => (format!("*{name}"), Need::Value),
                            Repr::Owned | Repr::Str => (name.to_string(), self.binding_need(*id)),
                        }
                    }
                    HirExprKind::Field { .. } | HirExprKind::Index { .. } => {
                        (self.place(target, false, indent), Need::Value)
                    }
                    _ => unreachable!("an assignment target is a local or a field or index chain"),
                };
                format!("{target} = {};", self.expr(value, need, indent))
            }
            HirStmt::Expr { expr, .. } => match &expr.kind {
                // A bare place statement is a use in Varyk, not a move.
                HirExprKind::Local(_) | HirExprKind::Field { .. } | HirExprKind::Index { .. } => {
                    format!(
                        "let _ = {};",
                        self.expr(expr, Need::Shared { binding: true }, indent)
                    )
                }
                HirExprKind::If { .. } | HirExprKind::Block(_) | HirExprKind::Match { .. }
                    if expr.ty == Ty::Unit =>
                {
                    self.expr(expr, Need::AsIs, indent)
                }
                _ => format!("{};", self.expr(expr, Need::AsIs, indent)),
            },
            HirStmt::Return { value: None, .. } => "return;".to_string(),
            HirStmt::Return {
                value: Some(value), ..
            } => format!("return {};", self.expr(value, Need::Value, indent)),
            HirStmt::While { cond, body, .. } => format!(
                "while {} {}",
                self.expr(cond, Need::Value, indent),
                self.block(body, Need::AsIs, indent)
            ),
            HirStmt::For {
                local, head, body, ..
            } => {
                let name = self.local_name(*local);
                let (head, copies) = match head {
                    HirForHead::Range { start, end } => {
                        let start = self.expr(start, Need::Value, indent);
                        let mut end = self.expr(end, Need::Value, indent);
                        // Rust would take an end starting with a block for
                        // the loop body.
                        if end.starts_with('{') {
                            end = format!("({end})");
                        }
                        (format!("{start}..{end}"), Vec::new())
                    }
                    // A place is looped over through a shared reference
                    // (spec 5), and a Copy element reached through it is
                    // copied first; a temporary is owned by the loop.
                    HirForHead::Vec(vec) if is_place(vec) => {
                        let head = self.expr(vec, Need::Shared { binding: true }, indent);
                        let copies = if self.function.locals[local.0 as usize].ty.is_copy() {
                            vec![format!("let {name} = *{name};")]
                        } else {
                            Vec::new()
                        };
                        (head, copies)
                    }
                    HirForHead::Vec(vec) => (self.expr(vec, Need::Value, indent), Vec::new()),
                };
                format!(
                    "for {name} in {head} {}",
                    self.block_body(body, &copies, Need::AsIs, indent)
                )
            }
            HirStmt::Break { .. } => "break;".to_string(),
            HirStmt::Continue { .. } => "continue;".to_string(),
        }
    }

    /// `{`, the block's statements and tail one level deeper than `indent`,
    /// then `}` at `indent`. The tail is emitted as `need` requires. Used
    /// where a block is embedded as a string, inside a larger expression
    /// or statement (an `if`, a `while` or `for` body, a `match` arm):
    /// expression emission stays string-based below a statement boundary,
    /// so the caller's own statement span covers every line this
    /// produces.
    pub(super) fn block(&self, block: &HirBlock, need: Need, indent: usize) -> String {
        self.block_body(block, &[], need, indent)
    }

    /// [`FnEmitter::block`] with the lines `first` (statements, such as a
    /// `match` arm's copies) before the block's own statements.
    pub(super) fn block_body(
        &self,
        block: &HirBlock,
        first: &[String],
        need: Need,
        indent: usize,
    ) -> String {
        let inner = indent + 1;
        let pad = indentation(inner);
        let mut out = String::from("{\n");
        for line in first {
            out.push_str(&pad);
            out.push_str(line);
            out.push('\n');
        }
        for stmt in &block.stmts {
            out.push_str(&pad);
            out.push_str(&self.stmt_text(stmt, inner));
            out.push('\n');
        }
        if let Some(tail) = &block.tail {
            out.push_str(&pad);
            out.push_str(&self.expr(tail, need, inner));
            out.push('\n');
        }
        out.push_str(&indentation(indent));
        out.push('}');
        out
    }
}
