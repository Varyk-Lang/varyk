//! The Rust backend (spec 6.4): crate, module, item, and statement
//! emission. Expressions are in `rust_expr`.
//!
//! The output is deterministic and readable without rustfmt: four-space
//! indentation, one statement per line, a blank line between items, and a
//! trailing newline.

use super::rust_expr::{FnEmitter, Need, Repr};
use super::{Backend, GeneratedCrate};
use crate::hir::{
    HirBlock, HirExprKind, HirFunction, HirModuleKind, HirProgram, HirStmt, HirStruct,
};
use crate::resolve::{ModuleId, StructId};
use crate::types::{ParamMode, Ty};

/// The crate-wide attribute every generated `main.rs` starts with.
const ALLOW: &str =
    "#![allow(dead_code, unused_variables, unused_mut, arithmetic_overflow, unconditional_panic)]";

/// Generates a Cargo project whose `src/` mirrors the program's modules.
pub struct RustBackend;

impl Backend for RustBackend {
    fn generate(&self, program: &HirProgram, package_name: &str) -> GeneratedCrate {
        let cargo_toml = format!(
            "[package]\nname = \"{package_name}\"\nversion = \"0.0.0\"\nedition = \"2024\"\n\n[workspace]\n\n[dependencies]\n"
        );

        let mut fn_modules = Vec::new();
        for module in &program.modules {
            if let HirModuleKind::Varyk { functions } = &module.kind {
                for function in functions {
                    let index = function.id.0 as usize;
                    if fn_modules.len() <= index {
                        fn_modules.resize(index + 1, module.id);
                    }
                    fn_modules[index] = module.id;
                }
            }
        }

        let mut main = format!("{ALLOW}\n\n");
        let mut files = Vec::new();
        let others: Vec<_> = program
            .modules
            .iter()
            .filter(|module| module.id != program.entry)
            .collect();
        for module in &others {
            main.push_str(&format!("mod {};\n", module.name));
        }
        if !others.is_empty() {
            main.push('\n');
        }
        main.push_str(&module_items(program, program.entry, &fn_modules));
        files.push(("src/main.rs".to_string(), main));

        for module in others {
            let content = match &module.kind {
                HirModuleKind::Varyk { .. } => module_items(program, module.id, &fn_modules),
                HirModuleKind::Rust { source } => source.clone(),
            };
            files.push((format!("src/{}.rs", module.name), content));
        }

        GeneratedCrate {
            package_name: package_name.to_string(),
            cargo_toml,
            files,
        }
    }
}

/// A Varyk module's structs and functions in source order, separated by
/// blank lines.
fn module_items(program: &HirProgram, module: ModuleId, fn_modules: &[ModuleId]) -> String {
    // The HIR keeps structs and functions apart; their spans (in the same
    // file) restore the source order.
    let mut items: Vec<(u32, String)> = program
        .structs
        .iter()
        .filter(|s| s.module == module)
        .map(|s| (s.span.start, emit_struct(program, s)))
        .collect();
    if let HirModuleKind::Varyk { functions } = &program.modules[module.0 as usize].kind {
        for function in functions {
            let emitter = FnEmitter::new(program, module, fn_modules, function);
            items.push((function.span.start, emitter.function()));
        }
    }
    items.sort_by_key(|(start, _)| *start);
    let items: Vec<String> = items.into_iter().map(|(_, item)| item).collect();
    items.join("\n")
}

fn emit_struct(program: &HirProgram, s: &HirStruct) -> String {
    let vis = if s.is_pub { "pub " } else { "" };
    if s.fields.is_empty() {
        return format!("{vis}struct {} {{}}\n", s.name);
    }
    let mut out = format!("{vis}struct {} {{\n", s.name);
    for (name, ty) in &s.fields {
        let ty = rust_type(program, *ty, s.module);
        out.push_str(&format!("    {vis}{name}: {ty},\n"));
    }
    out.push_str("}\n");
    out
}

/// The Rust spelling of a value of type `ty`, from `from`.
fn rust_type(program: &HirProgram, ty: Ty, from: ModuleId) -> String {
    match ty {
        Ty::Bool => "bool".to_string(),
        Ty::Int(kind) => kind.name().to_string(),
        Ty::Float(kind) => kind.name().to_string(),
        Ty::String => "String".to_string(),
        Ty::Struct(id) => struct_path(program, id, from),
        Ty::Unit => "()".to_string(),
    }
}

/// The Rust type of a parameter of type `ty` passed by `mode`.
fn param_type(program: &HirProgram, ty: Ty, mode: ParamMode, from: ModuleId) -> String {
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
/// the same module, else with a `crate::` prefix.
pub(super) fn item_path(
    program: &HirProgram,
    module: ModuleId,
    from: ModuleId,
    name: &str,
) -> String {
    if module == from {
        name.to_string()
    } else if module == program.entry {
        format!("crate::{name}")
    } else {
        format!("crate::{}::{name}", program.modules[module.0 as usize].name)
    }
}

fn indentation(indent: usize) -> String {
    "    ".repeat(indent)
}

impl FnEmitter<'_> {
    fn function(&self) -> String {
        let f: &HirFunction = self.function;
        let vis = if f.is_pub { "pub " } else { "" };
        let params: Vec<String> = f
            .params
            .iter()
            .map(|p| {
                let ty = param_type(self.program, p.ty, p.mode, self.module);
                format!("{}: {ty}", p.name)
            })
            .collect();
        let ret = match f.ret {
            Ty::Unit => String::new(),
            ty => format!(" -> {}", rust_type(self.program, ty, self.module)),
        };
        let need = if f.ret == Ty::Unit {
            Need::AsIs
        } else {
            Need::Value
        };
        format!(
            "{vis}fn {}({}){ret} {}\n",
            f.name,
            params.join(", "),
            self.block(&f.body, need, 0)
        )
    }

    /// `{`, the block's statements and tail one level deeper than `indent`,
    /// then `}` at `indent`. The tail is emitted as `need` requires.
    pub(super) fn block(&self, block: &HirBlock, need: Need, indent: usize) -> String {
        let inner = indent + 1;
        let pad = indentation(inner);
        let mut out = String::from("{\n");
        for stmt in &block.stmts {
            out.push_str(&pad);
            out.push_str(&self.stmt(stmt, inner));
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

    /// One statement, without its leading indentation or trailing newline.
    fn stmt(&self, stmt: &HirStmt, indent: usize) -> String {
        match stmt {
            HirStmt::Let { local, value, .. } => {
                let info = &self.function.locals[local.0 as usize];
                let mutability = if info.mutable { "mut " } else { "" };
                let value = self.expr(value, self.binding_need(*local), indent);
                format!("let {mutability}{} = {value};", info.name)
            }
            HirStmt::Assign { target, value, .. } => {
                let (target, need) = match &target.kind {
                    HirExprKind::Local(id) => {
                        let name = self.local_name(*id);
                        match self.reprs[id.0 as usize] {
                            // Assignment through a reference writes the
                            // referent.
                            Repr::Ref { .. } => (format!("*{name}"), Need::Value),
                            Repr::Owned | Repr::Str => (name.to_string(), self.binding_need(*id)),
                        }
                    }
                    HirExprKind::Field { base, name, .. } => (
                        format!("{}.{name}", self.field_base(base, false, indent)),
                        Need::Value,
                    ),
                    _ => unreachable!("an assignment target is a local or a field chain"),
                };
                format!("{target} = {};", self.expr(value, need, indent))
            }
            HirStmt::Expr { expr, .. } => match &expr.kind {
                // A bare place statement is a use in Varyk, not a move.
                HirExprKind::Local(_) | HirExprKind::Field { .. } => {
                    format!(
                        "let _ = {};",
                        self.expr(expr, Need::Shared { binding: true }, indent)
                    )
                }
                HirExprKind::If { .. } | HirExprKind::Block(_) if expr.ty == Ty::Unit => {
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
            HirStmt::Break { .. } => "break;".to_string(),
            HirStmt::Continue { .. } => "continue;".to_string(),
        }
    }
}
