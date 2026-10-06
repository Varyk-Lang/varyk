//! The Rust backend (spec 6.4): crate, module, item, and statement
//! emission. Expressions are in `rust_expr`.
//!
//! The output is deterministic and readable without rustfmt: four-space
//! indentation, one statement per line, a blank line between items, and a
//! trailing newline. Every generated `fn`, `struct`, `enum`, and `impl`
//! carries [`ALLOW_ITEM`] (spec 2.3) and every `use` carries
//! [`ALLOW_USE`]; no `mod` line carries either, so a copied `.rs` module,
//! wherever it sits in the tree, warns and fails as Rust normally does.

use std::cell::RefCell;

use super::rust_expr::{Adapted, Adapter, FnEmitter, Need, Repr};
use super::writer::{Mark, Writer};
use super::{Backend, CrateInfo, GeneratedCrate};
use crate::hir::{
    Binding, HirBlock, HirDefault, HirEnum, HirExprKind, HirForHead, HirFunction, HirHook,
    HirModule, HirModuleKind, HirParam, HirProgram, HirRoute, HirStmt, HirStruct, HookKind,
    LocalId, ReturnShape, is_place_or_rooted, path_text,
};
use crate::resolve::{FieldDef, ModuleId, StructId, UserType, VariantFieldsDef};
use crate::types::{Derives, ParamMode, Serde, Ty};
use varyk_syntax::Span;

/// The item-level attribute every generated `fn`, `struct`, `enum`, and
/// `impl` carries (spec 2.3): the `warnings` group covers every
/// warn-level lint, and the two deny-by-default lints are named because
/// a group cannot include them. The program still panics at run time on
/// overflow or division by zero (M1 §6.4).
const ALLOW_ITEM: &str = "#[allow(warnings, arithmetic_overflow, unconditional_panic)]";

/// The attribute every generated `use` carries instead of [`ALLOW_ITEM`]:
/// the one lint a plain `use` raises, and one clippy's `useless_attribute`
/// accepts there.
const ALLOW_USE: &str = "#[allow(unused_imports)]";

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
/// user wrote one and `pub` for a `pub use` (milestone 5b3 spec 4), each
/// carrying [`ALLOW_USE`], not [`ALLOW_ITEM`]: clippy's
/// `useless_attribute` (deny by default) rejects any other lint allow on
/// a `use`. Every use site is written with its full path, so the alias
/// never affects the generated Rust.
fn write_use_lines(module: &HirModule, writer: &mut Writer) {
    for use_ in &module.uses {
        writer.line(0, ALLOW_USE, None);
        let vis = if use_.is_pub { "pub " } else { "" };
        let line = match &use_.alias {
            Some(alias) => format!("{vis}use {} as {alias};", use_.path),
            None => format!("{vis}use {};", use_.path),
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

    let adapters = RefCell::new(Vec::new());

    for (index, (_, item)) in items.into_iter().enumerate() {
        if index > 0 {
            writer.line(0, "", None);
        }
        match item {
            Item::Struct(s) => emit_struct(program, s, writer),
            Item::Enum(e) => emit_enum(program, e, writer),
            Item::Fn(f) => {
                writer.line(0, ALLOW_ITEM, None);
                FnEmitter::new(program, f, &adapters).function(writer, 0);
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
                    FnEmitter::new(program, f, &adapters).function(writer, 1);
                }
                writer.line(0, "}", None);
            }
        }
    }
    write_adapters(program, module, &adapters.into_inner(), writer);
}

/// Writes the adapters the module's functions asked for (milestone 5b4
/// spec 4), after its items, each `varyk_route_N` with `N` its place in
/// `adapters`, and every line of it carrying the span of the call that
/// asked for it, marked [`Mark::Route`], so a rustc error inside is
/// reported at that call.
fn write_adapters(
    program: &HirProgram,
    module: ModuleId,
    adapters: &[Adapter],
    writer: &mut Writer,
) {
    for (number, adapter) in adapters.iter().enumerate() {
        writer.line(0, "", None);
        let mut lines = AdapterLines {
            writer: &mut *writer,
            span: adapter.of.span(),
        };
        lines.line(0, ALLOW_ITEM);
        match &adapter.of {
            Adapted::Route(route) => {
                route_adapter(program, module, &adapter.package, number, route, &mut lines)
            }
            Adapted::Hook(hook) => {
                hook_adapter(program, module, &adapter.package, number, hook, &mut lines)
            }
        }
    }
}

/// The lines of one adapter, each carrying `span` and [`Mark::Route`].
pub(super) struct AdapterLines<'w> {
    writer: &'w mut Writer,
    span: Span,
}

impl AdapterLines<'_> {
    pub(super) fn line(&mut self, indent: usize, text: &str) {
        self.writer
            .marked_line(indent, text, Some(self.span), Some(Mark::Route));
    }
}

/// A route's adapter (milestone 5b4 spec 4): it takes the request and
/// binds each of the handler's parameters from it as the route's
/// [`Binding`] says (the package's own types last), returning the
/// package's response at once when one cannot be bound. It then calls the
/// handler, lending each parameter as its mode says, and makes the
/// response from what the handler returns.
fn route_adapter(
    program: &HirProgram,
    module: ModuleId,
    package: &str,
    number: usize,
    route: &HirRoute,
    lines: &mut AdapterLines,
) {
    lines.line(
        0,
        &format!(
            "async fn varyk_route_{number}(varyk_req: {package}::Request) -> {package}::Response {{"
        ),
    );
    let handler = program.function(route.handler);
    // The path, query, body, and state first, then the package's own
    // types, so a 400 comes before a live handler's early response; the
    // call below keeps the handler's order.
    let mut lent = vec![String::new(); route.params.len()];
    for package_types in [false, true] {
        for (index, (binding, param)) in route.params.iter().zip(&handler.params).enumerate() {
            if matches!(binding, Binding::Package(_)) == package_types {
                lent[index] = bind(program, module, binding, param, index, "r", lines);
            }
        }
    }
    let call = format!(
        "{}({}).await",
        item_path(program, handler.module, module, &handler.name),
        lent.join(", ")
    );
    match &route.ret {
        ReturnShape::Nothing => {
            lines.line(1, &format!("{call};"));
            lines.line(1, &format!("{package}::respond_empty()"));
        }
        ReturnShape::Result(ok) => {
            lines.line(1, &format!("match {call} {{"));
            lines.line(2, &format!("Ok(v) => {},", response(package, ok, "v")));
            lines.line(2, &format!("Err(e) => {package}::respond_error(e),"));
            lines.line(1, "}");
        }
        shape => lines.line(1, &response(package, shape, &call)),
    }
    lines.line(0, "}");
}

/// A hook's adapter (milestone 5b4 spec 4). A `before` adapter takes the
/// request, binds the state if the hook reads it, lends the hook the
/// request, and gives `None` to let the request through or `Some` with the
/// response that stops it, made by the package from the hook's result. An
/// `after` adapter takes the request and the response, which it rebinds
/// `mut` and lends the hook as the hook's parameter says, and gives the
/// response on.
fn hook_adapter(
    program: &HirProgram,
    module: ModuleId,
    package: &str,
    number: usize,
    hook: &HirHook,
    lines: &mut AdapterLines,
) {
    let (head, stop) = match hook.kind {
        HookKind::Before => (
            format!(
                "async fn varyk_route_{number}(varyk_req: {package}::Request) -> \
                 Option<{package}::Response> {{"
            ),
            "Some(r)",
        ),
        HookKind::After => (
            format!(
                "async fn varyk_route_{number}(varyk_req: {package}::Request, varyk_res: \
                 {package}::Response) -> {package}::Response {{"
            ),
            // The package's response on, as a route does (spec 4); the
            // state of an accepted hook always binds, so this is never
            // reached and `varyk_res` is not lost.
            "r",
        ),
    };
    lines.line(0, &head);
    let function = program.function(hook.hook);
    // The request, then for an `after` hook the response; the hook's
    // request is never `mut` (spec 2.4).
    let mut lent = vec!["&varyk_req".to_string()];
    if hook.kind == HookKind::After {
        lines.line(1, "let mut varyk_res = varyk_res;");
        let res = match function.params.get(1).map(|param| param.mode) {
            Some(ParamMode::MutableBorrow) => "&mut varyk_res",
            _ => "&varyk_res",
        };
        lent.push(res.to_string());
    }
    if hook.state {
        let index = lent.len();
        if let Some(param) = function.params.get(index) {
            lent.push(bind(
                program,
                module,
                &Binding::State,
                param,
                index,
                stop,
                lines,
            ));
        }
    }
    let call = format!(
        "{}({}).await",
        item_path(program, function.module, module, &function.name),
        lent.join(", ")
    );
    match hook.kind {
        HookKind::Before => lines.line(1, &format!("{package}::respond_before({call})")),
        HookKind::After => {
            lines.line(1, &format!("{call};"));
            lines.line(1, "varyk_res");
        }
    }
    lines.line(0, "}");
}

/// Binds the handler's parameter `param`, the `index`th, from
/// `varyk_req` as `binding` says, into `varyk_<index>`, returning `stop`
/// with the package's response `r` when it cannot be (`r` for a route,
/// `Some(r)` for a `before` hook); gives how the handler is lent it: a
/// number or `bool` by value, `&mut` for a `mut` parameter, and `&`
/// otherwise, as a started call lends its arguments (milestone 5b4 spec
/// 2.2, 4).
pub(super) fn bind(
    program: &HirProgram,
    module: ModuleId,
    binding: &Binding,
    param: &HirParam,
    index: usize,
    stop: &str,
    lines: &mut AdapterLines,
) -> String {
    let ty = |ty: &Ty| rust_type(program, ty, module);
    let value = match binding {
        Binding::Path(name) => format!("varyk_req.param::<{}>(\"{name}\")", ty(&param.ty)),
        Binding::Query(name) => format!("varyk_req.query::<{}>(\"{name}\")", ty(&param.ty)),
        Binding::Body => format!(
            "varyk_req.json::<{}>(\"{}\").await",
            ty(&param.ty),
            param.name
        ),
        Binding::State => {
            let state = match &param.ty {
                Ty::Shared(state) => state.as_ref(),
                other => other,
            };
            format!("varyk_req.state::<{}>()", ty(state))
        }
        Binding::Package(_) => format!("varyk_req.bind::<{}>().await", ty(&param.ty)),
    };
    let mutability = if param.mode == ParamMode::MutableBorrow {
        "mut "
    } else {
        ""
    };
    lines.line(
        1,
        &format!("let {mutability}varyk_{index} = match {value} {{"),
    );
    lines.line(2, "Ok(v) => v,");
    lines.line(2, &format!("Err(r) => return {stop},"));
    lines.line(1, "};");
    match param.mode {
        ParamMode::Owned => format!("varyk_{index}"),
        ParamMode::SharedBorrow => format!("&varyk_{index}"),
        ParamMode::MutableBorrow => format!("&mut varyk_{index}"),
    }
}

/// The response for `value`, what a handler returned, by `shape`
/// (milestone 5b4 spec 2.3, 4), each a function of the package's
/// contract but an `http::Response`, which is sent as built.
fn response(package: &str, shape: &ReturnShape, value: &str) -> String {
    match shape {
        ReturnShape::Nothing => format!("{{ {value}; {package}::respond_empty() }}"),
        ReturnShape::Json => format!("{package}::respond_json(&{value})"),
        ReturnShape::Option => format!("{package}::respond_option({value})"),
        ReturnShape::Response => value.to_string(),
        ReturnShape::Result(ok) => format!(
            "match {value} {{ Ok(v) => {}, Err(e) => {package}::respond_error(e) }}",
            response(package, ok, "v")
        ),
    }
}

/// The path to the root of the package module `module` belongs to, from
/// this crate: `::http` for `::http::server`, as the program's key for
/// the package names it (M5b2 spec 5), and `crate` for a module of this
/// package, whose root names every item of the contract (milestone 5b4
/// spec 7.2).
fn package_root(program: &HirProgram, module: ModuleId) -> String {
    let path = program.module_path(module);
    match path
        .strip_prefix("::")
        .and_then(|rest| rest.split("::").next())
    {
        Some(key) => format!("::{key}"),
        None => "crate".to_string(),
    }
}

fn emit_struct(program: &HirProgram, s: &HirStruct, writer: &mut Writer) {
    writer.line(0, ALLOW_ITEM, None);
    write_derives(s.derives, writer);
    write_serde_derives(s.serde, s.span, writer);
    let vis = if s.is_pub { "pub " } else { "" };
    if s.fields.is_empty() {
        writer.line(0, &format!("{vis}struct {} {{}}", s.name), Some(s.span));
        return;
    }
    writer.line(0, &format!("{vis}struct {} {{", s.name), Some(s.span));
    for field in &s.fields {
        if let Some(line) = field_serde(s, field) {
            writer.line(1, &line, Some(field.span));
        }
        let vis = if field.is_pub { "pub " } else { "" };
        let ty = rust_type(program, &field.ty, s.module);
        writer.line(1, &format!("{vis}{}: {ty},", field.name), Some(s.span));
    }
    writer.line(0, "}", None);
    if !s.serde.deserialize {
        return;
    }
    // The functions behind the `#[default]`s of a read struct, in an
    // `impl` block beside it (M5a spec 7.5): serde calls each for a
    // missing key. As associated functions they cannot collide across
    // types, and no method of the program may start with `varyk_`.
    let defaults: Vec<(&FieldDef, &HirDefault)> = s
        .fields
        .iter()
        .filter_map(|field| field.attrs.default.as_ref().map(|d| (field, d)))
        .collect();
    if defaults.is_empty() {
        return;
    }
    writer.line(0, "", None);
    writer.line(0, ALLOW_ITEM, None);
    writer.line(0, &format!("impl {} {{", s.name), None);
    for (index, (field, default)) in defaults.into_iter().enumerate() {
        let value = match default {
            HirDefault::Int(value) => value.to_string(),
            HirDefault::Float(value) => format!("{value:?}"),
            // A literal placed into an owned slot: the one allocation the
            // compiler inserts (M1 §4.3). The text is as written between
            // the quotes, whose escapes are Rust's too.
            HirDefault::Str(text) => format!("\"{text}\".to_string()"),
            HirDefault::Bool(value) => value.to_string(),
        };
        let ty = rust_type(program, &field.ty, s.module);
        if index > 0 {
            writer.line(0, "", None);
        }
        writer.line(
            1,
            &format!("fn {}() -> {ty} {{", default_fn(field)),
            Some(field.span),
        );
        writer.line(2, &value, Some(field.span));
        writer.line(1, "}", None);
    }
    writer.line(0, "}", None);
}

/// The name of the associated function behind a `#[default]` of field
/// `field`: `varyk_default_<field>`, a prefix no Varyk method may take
/// (M5a spec 2.10).
fn default_fn(field: &FieldDef) -> String {
    format!("varyk_default_{}", field.name)
}

/// The `#[serde(..)]` line of a field of a struct a `json` call reaches:
/// `rename`, `skip`, and, when the struct is read, `default`; `None` when
/// it needs none, or no call reaches the struct (M5a spec 2.2).
fn field_serde(s: &HirStruct, field: &FieldDef) -> Option<String> {
    if !s.serde.any() {
        return None;
    }
    let mut parts = Vec::new();
    if field.attrs.skip {
        parts.push("skip".to_string());
    } else if let Some(key) = &field.attrs.rename {
        parts.push(format!("rename = \"{key}\""));
    }
    if s.serde.deserialize && field.attrs.default.is_some() {
        parts.push(format!("default = \"{}::{}\"", s.name, default_fn(field)));
    }
    if parts.is_empty() {
        return None;
    }
    Some(format!("#[serde({})]", parts.join(", ")))
}

fn emit_enum(program: &HirProgram, e: &HirEnum, writer: &mut Writer) {
    writer.line(0, ALLOW_ITEM, None);
    write_derives(e.derives, writer);
    write_serde_derives(e.serde, e.span, writer);
    let vis = if e.is_pub { "pub " } else { "" };
    writer.line(0, &format!("{vis}enum {} {{", e.name), Some(e.span));
    for variant in &e.variants {
        let name = &variant.name;
        if let (true, Some(key)) = (e.serde.any(), &variant.rename) {
            writer.line(1, &format!("#[serde(rename = \"{key}\")]"), Some(e.span));
        }
        let line = match &variant.fields {
            VariantFieldsDef::Tuple(types) if types.is_empty() => format!("{name},"),
            VariantFieldsDef::Tuple(types) => {
                let types: Vec<String> = types
                    .iter()
                    .map(|ty| rust_type(program, ty, e.module))
                    .collect();
                format!("{name}({}),", types.join(", "))
            }
            VariantFieldsDef::Named(fields) => {
                let fields: Vec<String> = fields
                    .iter()
                    .map(|(field, ty)| format!("{field}: {}", rust_type(program, ty, e.module)))
                    .collect();
                format!("{name} {{ {} }},", fields.join(", "))
            }
        };
        writer.line(1, &line, Some(e.span));
    }
    writer.line(0, "}", None);
}

/// The one `#[derive(..)]` line of a struct or enum (M4 spec 2.10, 5):
/// `Clone` and `PartialEq` as it allows, in that order; none when it
/// allows neither.
fn write_derives(derives: Derives, writer: &mut Writer) {
    let names: Vec<&str> = [(derives.clone, "Clone"), (derives.eq, "PartialEq")]
        .into_iter()
        .filter_map(|(has, name)| has.then_some(name))
        .collect();
    if !names.is_empty() {
        writer.line(0, &format!("#[derive({})]", names.join(", ")), None);
    }
}

/// The serde derives of a struct or enum a `json` call reaches (M5a spec
/// 7.5), through `varyk-std`'s serde by its absolute path, so the program
/// needs no serde of its own; nothing for a type no call reaches.
fn write_serde_derives(serde: Serde, span: Span, writer: &mut Writer) {
    let names: Vec<&str> = [
        (serde.serialize, "::varyk_std::serde::Serialize"),
        (serde.deserialize, "::varyk_std::serde::Deserialize"),
    ]
    .into_iter()
    .filter_map(|(has, name)| has.then_some(name))
    .collect();
    if names.is_empty() {
        return;
    }
    writer.line(0, &format!("#[derive({})]", names.join(", ")), None);
    writer.line(0, "#[serde(crate = \"::varyk_std::serde\")]", Some(span));
}

/// The Rust spelling of a value of type `ty`, from `from`. A `string` is
/// an owned `String` wherever a type is spelled: a field, a payload, a
/// return, and inside `Option`, `Result`, `Vec`, and `HashMap`.
pub(super) fn rust_type(program: &HirProgram, ty: &Ty, from: ModuleId) -> String {
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
        // In full, so that no `use` line of a `.rs` module can shadow it.
        Ty::HashMap(key, value) => format!(
            "::std::collections::HashMap<{}, {}>",
            rust_type(program, key, from),
            rust_type(program, value, from)
        ),
        // Never spelled for a program `check` accepts (V0208).
        Ty::Chain(item) => format!(
            "impl ::std::iter::Iterator<Item = {}>",
            rust_type(program, item, from)
        ),
        // In full, so that no module of the program can shadow it (M5a
        // spec 7.5).
        Ty::Error => "::varyk_std::Error".to_string(),
        // Never spelled for a program `check` accepts (V0215).
        Ty::Task(t) => format!("::varyk_std::Task<{}>", rust_type(program, t, from)),
        // Std's own pointer, in full (milestone 5b1 spec 5).
        Ty::Shared(t) => format!("::std::sync::Arc<{}>", rust_type(program, t, from)),
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
/// with `span` and `mark`.
fn push_lines(
    writer: &mut Writer,
    indent: usize,
    text: &str,
    span: Option<Span>,
    mark: Option<Mark>,
) {
    let mut lines = text.split('\n');
    if let Some(first) = lines.next() {
        writer.marked_line(indent, first, span, mark);
    }
    for line in lines {
        writer.marked_line(0, line, span, mark);
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
        | HirStmt::WhileLet { span, .. }
        | HirStmt::For { span, .. }
        | HirStmt::Break { span }
        | HirStmt::Continue { span } => *span,
        HirStmt::Route(route) => route.span,
        HirStmt::Hook(hook) => hook.span,
    }
}

impl FnEmitter<'_> {
    /// Writes the function's declaration, starting at `indent` levels (one
    /// inside an `impl` block): the signature and opening brace as one
    /// line carrying the function's span, its body's statements each
    /// carrying their own span, then the closing brace (structural). A
    /// method's receiver is `&self` or `&mut self`.
    ///
    /// A borrowed return (M4 spec 3.1) is `-> &str` or `-> &T`; when the
    /// signature has more than one reference parameter and the root is
    /// not `self`, elision would not name the root, so one lifetime is
    /// written on the rooted parameter and the return.
    pub(super) fn function(&self, writer: &mut Writer, indent: usize) {
        let f: &HirFunction = self.function;
        let vis = if f.is_pub { "pub " } else { "" };
        let is_self = |p: &HirParam| f.owner.is_some() && p.name == "self";
        let references = f
            .params
            .iter()
            .filter(|p| p.mode != ParamMode::Owned)
            .count();
        let lifetime = f.ret_root.filter(|root| {
            references > 1 && f.params.get(root.0 as usize).is_some_and(|p| !is_self(p))
        });
        let params: Vec<String> = f
            .params
            .iter()
            .map(|p| match p.mode {
                _ if !is_self(p) => {
                    let ty = param_type(self.program, &p.ty, p.mode, self.module);
                    let ty = match lifetime {
                        Some(root) if root == p.local => ty.replacen('&', "&'a ", 1),
                        _ => ty,
                    };
                    format!("{}: {ty}", p.name)
                }
                ParamMode::MutableBorrow => "&mut self".to_string(),
                _ => "&self".to_string(),
            })
            .collect();
        let named = if lifetime.is_some() { "'a " } else { "" };
        let ret = match (&f.ret, f.ret_root) {
            (Ty::Unit, _) => String::new(),
            (Ty::String, Some(_)) => format!(" -> &{named}str"),
            (ty, Some(_)) => format!(" -> &{named}{}", rust_type(self.program, ty, self.module)),
            (ty, None) => format!(" -> {}", rust_type(self.program, ty, self.module)),
        };
        let generics = if lifetime.is_some() { "<'a>" } else { "" };
        // A test is run by `cargo test`'s harness (M5a spec 7.5).
        if f.is_test {
            writer.line(indent, "#[test]", None);
        }
        let is_main = f.owner.is_none() && f.module == self.program.entry && f.name == "main";
        // An async `main` or test is an ordinary Rust function that runs
        // its body on `varyk-std`'s runtime (milestone 5b1 spec 5).
        let runs = f.is_async && (is_main || f.is_test);
        let asyncness = if f.is_async && !runs { "async " } else { "" };
        writer.line(
            indent,
            &format!(
                "{vis}{asyncness}fn {}{generics}({}){ret} {{",
                f.name,
                params.join(", ")
            ),
            Some(f.span),
        );
        let inner = if runs { indent + 1 } else { indent };
        if runs {
            writer.line(inner, "::varyk_std::run(async {", Some(f.span));
        }
        // A program that logs starts logging first thing (M5a spec 7.5),
        // in `main` and inside an async test's runtime, so a message
        // logged under `varyk test` reaches stderr (milestone 5b4 spec
        // 7.5); `start` tolerates being called by many tests.
        if self.program.logs && (is_main || runs) {
            writer.line(inner + 1, "::varyk_std::start();", Some(f.span));
        }
        if runs {
            self.block_lines(writer, &f.body, self.return_need(), inner);
            writer.line(inner, "})", None);
            writer.line(indent, "}", None);
        } else {
            self.block_after(writer, &f.body, self.return_need(), indent);
        }
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
        self.block_lines(writer, block, need, indent);
        writer.line(indent, "}", None);
    }

    /// Writes `block`'s statements and tail into `writer` at `indent + 1`,
    /// as [`FnEmitter::block_after`] does, without the closing `}`.
    fn block_lines(&self, writer: &mut Writer, block: &HirBlock, need: Need, indent: usize) {
        let inner = indent + 1;
        for stmt in &block.stmts {
            self.stmt(writer, stmt, inner);
        }
        if let Some(tail) = &block.tail {
            self.mark.set(None);
            let text = self.expr(tail, need, inner);
            push_lines(writer, inner, &text, Some(tail.span), self.mark.take());
        }
    }

    /// Writes one statement's physical lines into `writer` at `indent`,
    /// every line tagged with the statement's own span (even when its
    /// text is multi-line, e.g. a `while` or `for` body: expression
    /// emission below a statement stays string-based, so a nested block's
    /// lines take the statement's span rather than their own).
    fn stmt(&self, writer: &mut Writer, stmt: &HirStmt, indent: usize) {
        self.mark.set(None);
        let text = self.stmt_text(stmt, indent);
        push_lines(
            writer,
            indent,
            &text,
            Some(stmt_span(stmt)),
            self.mark.take(),
        );
    }

    /// The path to the `varyk-http` package whose `App` the local `app`
    /// holds, from this crate: `::http`.
    fn package_of(&self, app: LocalId) -> Option<String> {
        match self.function.locals[app.0 as usize].ty {
            Ty::Struct(id) => Some(package_root(
                self.program,
                self.program.structs[id.0 as usize].module,
            )),
            _ => None,
        }
    }

    /// The number of the module's adapter for `adapted`, added to the
    /// module's list the first time it is asked for: the `N` of
    /// `varyk_route_N` (milestone 5b4 spec 4).
    pub(super) fn adapter(&self, package: &str, adapted: Adapted) -> usize {
        let mut adapters = self.adapters.borrow_mut();
        let span = adapted.span();
        if let Some(number) = adapters.iter().position(|a| a.of.span() == span) {
            return number;
        }
        adapters.push(Adapter {
            package: package.to_string(),
            of: adapted,
        });
        adapters.len() - 1
    }

    /// One statement's Rust text, without its leading indentation or
    /// trailing newline (may itself be multi-line, e.g. a `while` or
    /// `for` body).
    fn stmt_text(&self, stmt: &HirStmt, indent: usize) -> String {
        match stmt {
            HirStmt::Let { local, value, .. } => {
                let info = &self.function.locals[local.0 as usize];
                let mutability = if info.mutable { "mut " } else { "" };
                // A value holding `Option`, `Result`, `Vec`, or `HashMap`
                // may have taken part of its type from the `let` (`None`,
                // `vec![]`, `HashMap::new()`, spec 2.6), so the Rust keeps
                // the type written.
                let ty = match info.ty {
                    Ty::Option(_) | Ty::Result(..) | Ty::Vec(_) | Ty::HashMap(..)
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
                HirExprKind::If { .. }
                | HirExprKind::IfLet { .. }
                | HirExprKind::Block(_)
                | HirExprKind::Match { .. }
                    if expr.ty == Ty::Unit =>
                {
                    self.expr(expr, Need::AsIs, indent)
                }
                _ => format!("{};", self.expr(expr, Need::AsIs, indent)),
            },
            HirStmt::Return { value: None, .. } => "return;".to_string(),
            HirStmt::Return {
                value: Some(value), ..
            } => format!("return {};", self.expr(value, self.return_need(), indent)),
            HirStmt::While { cond, body, .. } => format!(
                "while {} {}",
                self.expr(cond, Need::Value, indent),
                self.block(body, Need::AsIs, indent)
            ),
            HirStmt::WhileLet {
                pattern,
                value,
                body,
                head_is_str,
                ..
            } => {
                let (head, copies) = self.let_head(value, pattern, *head_is_str, indent);
                format!(
                    "while let {} = {head} {}",
                    self.pattern(pattern),
                    self.block_body(body, &copies, Need::AsIs, indent)
                )
            }
            HirStmt::For {
                local, head, body, ..
            } => {
                let name = self.local_name(*local);
                let (head, copies) = match head {
                    HirForHead::Range {
                        start,
                        end,
                        inclusive,
                    } => {
                        let start = self.expr(start, Need::Value, indent);
                        let mut end = self.expr(end, Need::Value, indent);
                        // Rust would take an end starting with a block for
                        // the loop body.
                        if end.starts_with('{') {
                            end = format!("({end})");
                        }
                        let dots = if *inclusive { "..=" } else { ".." };
                        (format!("{start}{dots}{end}"), Vec::new())
                    }
                    // A place is looped over through a shared reference
                    // (spec 5), and a Copy element reached through it is
                    // copied first; a temporary is owned by the loop.
                    HirForHead::Vec(vec) if is_place_or_rooted(vec) => {
                        let head = self.expr(vec, Need::Shared { binding: true }, indent);
                        let copies = if self.function.locals[local.0 as usize].ty.is_copy() {
                            vec![format!("let {name} = *{name};")]
                        } else {
                            Vec::new()
                        };
                        (head, copies)
                    }
                    // A chain is written as it is: its items are what the
                    // variable is (M4 spec 3.3).
                    HirForHead::Vec(head) | HirForHead::Chain(head) => {
                        (self.expr(head, Need::Value, indent), Vec::new())
                    }
                };
                format!(
                    "for {name} in {head} {}",
                    self.block_body(body, &copies, Need::AsIs, indent)
                )
            }
            HirStmt::Break { .. } => "break;".to_string(),
            HirStmt::Continue { .. } => "continue;".to_string(),
            // The route's adapter is written after the module's functions;
            // here the app is given it (milestone 5b4 spec 4).
            HirStmt::Route(route) => {
                self.mark.set(Some(Mark::Route));
                let Some(package) = self.package_of(route.app) else {
                    return "compile_error!(\"a route on something that is not an app\");"
                        .to_string();
                };
                let number = self.adapter(&package, Adapted::Route(route.clone()));
                format!(
                    "{}.{}(\"{}\", {package}::route(varyk_route_{number}));",
                    self.local_name(route.app),
                    route.method.call(),
                    path_text(&route.path)
                )
            }
            // A hook's adapter likewise, from the same count.
            HirStmt::Hook(hook) => {
                self.mark.set(Some(Mark::Route));
                let Some(package) = self.package_of(hook.app) else {
                    return "compile_error!(\"a hook on something that is not an app\");"
                        .to_string();
                };
                let number = self.adapter(&package, Adapted::Hook(hook.clone()));
                let wrapped = match hook.kind {
                    HookKind::Before => format!("{package}::before_hook(varyk_route_{number})"),
                    HookKind::After => format!("{package}::after_hook(varyk_route_{number})"),
                };
                let prefix = match &hook.prefix {
                    Some(prefix) => format!("\"{}\", ", path_text(prefix)),
                    None => String::new(),
                };
                format!(
                    "{}.{}({prefix}{wrapped});",
                    self.local_name(hook.app),
                    hook.call()
                )
            }
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
