//! Registering what `.rs` modules import (M3 spec 4.1, 4.2, 4.3): free
//! functions, structs and enums in the same tables as Varyk-declared
//! ones, and structs' methods and associated functions as type members.

use std::collections::HashSet;

use varyk_syntax::Span;
use varyk_syntax::{Item, UseDecl};

use crate::interop::{
    Expander, FieldVis, ImportedEnum, ImportedModule, ImportedStruct, MacroRisk, StructKind,
    tidy_signature,
};
use crate::types::Ty;

use super::signatures::Mapper;
use super::{
    Callee, DropCause, EnumDef, EnumId, FieldDef, ImportedFnId, ImportedSig, Module, ModuleId,
    ModuleKind, StructDef, StructId, Symbols, Unusable, UserType, VariantDef, VariantFieldsDef,
};

/// Marks the enums an `impl Drop` in any `.rs` file is for (see
/// `interop::DropScan`). A block's type is matched by its last path
/// segment: when exactly one enum, imported or declared in Varyk, has
/// that name, it is the one; when none does and the name is a struct,
/// the block is not about an enum. Anything else (a name some `type` or
/// `use ... as` could stand for, several enums of that name, a name no
/// type has, a type or macro the scan cannot read) marks every enum,
/// which only makes a `match` on a temporary look inside it instead of
/// taking it apart: never wrong, only stricter.
fn mark_drops(symbols: &mut Symbols, modules: &[Module], imports: &[(ModuleId, ImportedModule)]) {
    let mut aliases: HashSet<&str> = HashSet::new();
    let mut structs: HashSet<&str> = symbols.structs.iter().map(|s| s.name.as_str()).collect();
    for (_, imported) in imports {
        aliases.extend(imported.drops.aliases.iter().map(String::as_str));
        structs.extend(imported.drops.structs.iter().map(String::as_str));
    }
    for module in modules {
        if let ModuleKind::Varyk(program) = &module.kind {
            for item in &program.items {
                if let Item::Use(UseDecl {
                    alias: Some(alias), ..
                }) = item
                {
                    aliases.insert(alias.name.as_str());
                }
            }
        }
    }
    // Why every enum is marked: the first file that makes it so.
    let mut all: Option<String> = None;
    let mut marked = Vec::new();
    for (_, imported) in imports {
        let file = &imported.file;
        if let Some(why) = &imported.drops.unknown {
            all.get_or_insert_with(|| format!("`{file}` {why}"));
        }
        for name in &imported.drops.targets {
            let matching: Vec<usize> = (0..symbols.enums.len())
                .filter(|&id| symbols.enums[id].name == *name)
                .collect();
            if aliases.contains(name.as_str()) {
                all.get_or_insert_with(|| {
                    format!(
                        "`{file}` implements `Drop` for `{name}`, a name that could stand for \
                         any type"
                    )
                });
            } else if let [id] = matching[..] {
                marked.push(id);
            } else if !(matching.is_empty() && structs.contains(name.as_str())) {
                all.get_or_insert_with(|| {
                    format!("`{file}` implements `Drop` for a type Varyk cannot resolve")
                });
            }
        }
    }
    for (id, def) in symbols.enums.iter_mut().enumerate() {
        if marked.contains(&id) {
            def.drops = Some(DropCause::Impl);
        } else if let Some(why) = &all {
            def.drops = Some(DropCause::Unsure(why.clone()));
        }
    }
}

/// Registers every `.rs` module's imports: first every struct's and
/// enum's name (or, for one Varyk does not import, why), so any
/// signature, field, or variant payload in any `.rs` module can name any
/// of them (spec 4.4); then free functions, struct fields and methods,
/// and enum variants, mapped.
pub(super) fn register(
    symbols: &mut Symbols,
    modules: &[Module],
    imports: Vec<(ModuleId, ImportedModule)>,
) {
    let mut structs: Vec<(ModuleId, StructId, ImportedStruct)> = Vec::new();
    let mut enums: Vec<(ModuleId, EnumId, ImportedEnum)> = Vec::new();
    for (module, imported) in &imports {
        let file = modules[module.0 as usize].file;
        let scope = &mut symbols.scopes[module.0 as usize];
        scope
            .restricted
            .extend(imported.restricted_fns.iter().cloned());
        scope.file = imported.file.clone();
        scope.expander = imported.expander.as_ref().map(|expander| {
            let Expander { shown, line, risk } = expander;
            let (what, fix) = match risk {
                MacroRisk::Writes => ("", "move the macro and its uses"),
                MacroRisk::Relays => (
                    " calls a macro defined elsewhere, which",
                    "move the macro and its uses",
                ),
                MacroRisk::Elsewhere => (" is a macro defined elsewhere, which", "move this call"),
            };
            format!(
                "`{shown}` at `{}:{line}`{what} could define names Varyk cannot see, so Varyk \
                 cannot tell which types this file's signatures and fields name; {fix} to \
                 another `.rs` file, and write out by hand any function you call from Varyk",
                imported.file
            )
        });
        for (kind, name, vis) in &imported.restricted_types {
            let why = format!(
                "it is `{vis}` in the Rust file; Varyk imports only plain `pub`, so write `pub \
                 {kind} {name}` in the Rust file"
            );
            scope.skipped.insert(name.clone(), (kind, why));
        }
        for (kind, name) in &imported.cfg_types {
            let why = "it is behind `#[cfg]`, so it may not exist in the build".to_string();
            scope.skipped.insert(name.clone(), (kind, why));
        }
        for (name, kind, why) in &imported.skipped_fns {
            scope.skipped_fns.insert(name.clone(), (kind, why));
        }
        scope
            .inline_mods
            .extend(imported.inline_mods.iter().cloned());
        for s in &imported.structs {
            let scope = &mut symbols.scopes[module.0 as usize];
            if let Some(why) = why_skipped_struct(s) {
                scope
                    .skipped
                    .insert(s.name.clone(), ("struct", why.to_string()));
                continue;
            }
            if scope.types.contains_key(&s.name) {
                continue;
            }
            let id = StructId(symbols.structs.len() as u32);
            scope.types.insert(s.name.clone(), UserType::Struct(id));
            symbols.structs.push(StructDef {
                name: s.name.clone(),
                module: *module,
                is_pub: true,
                imported: true,
                derives: Some(s.derives),
                fields: Vec::new(),
                span: Span::new(file, s.span.start as u32, s.span.end as u32),
            });
            structs.push((*module, id, s.clone()));
        }
        for e in &imported.enums {
            let scope = &mut symbols.scopes[module.0 as usize];
            if let Some(why) = why_skipped_enum(e) {
                scope
                    .skipped
                    .insert(e.name.clone(), ("enum", why.to_string()));
                continue;
            }
            if scope.types.contains_key(&e.name) {
                continue;
            }
            let id = EnumId(symbols.enums.len() as u32);
            scope.types.insert(e.name.clone(), UserType::Enum(id));
            symbols.enums.push(EnumDef {
                name: e.name.clone(),
                module: *module,
                is_pub: true,
                imported: true,
                derives: Some(e.derives),
                drops: None,
                opaque: e.opaque.clone(),
                variants: Vec::new(),
                span: Span::new(file, e.span.start as u32, e.span.end as u32),
            });
            enums.push((*module, id, e.clone()));
        }
    }

    mark_drops(symbols, modules, &imports);

    for (module, imported) in imports {
        for f in imported.fns {
            let id = ImportedFnId(symbols.imported.len() as u32);
            let mut sig = Mapper::new(symbols, module).sig(None, f);
            hide_if_unreachable(symbols, &mut sig);
            symbols.scopes[module.0 as usize]
                .fns
                .entry(sig.name.clone())
                .or_insert(Callee::Imported(id));
            symbols.imported.push(sig);
        }
    }

    for (module, id, s) in structs {
        let file = modules[module.0 as usize].file;
        let mapper = Mapper::new(symbols, module);
        let fields = s
            .fields
            .iter()
            .map(|field| {
                let hidden = |ty: &Ty| match field.vis {
                    FieldVis::Pub => symbols.hidden_type(ty, symbols.reach(module, true)),
                    _ => None,
                };
                let (ty, unusable) = match mapper.field(&field.ty) {
                    Ok(ty) => match hidden(&ty) {
                        None => (ty, None),
                        Some((note, within)) => (
                            ty,
                            Some(Unusable {
                                rust_ty: field.ty.text(),
                                note: Some(note),
                                within: Some(within),
                            }),
                        ),
                    },
                    Err(unusable) => (Ty::Unit, Some(unusable)),
                };
                FieldDef {
                    name: field.name.clone(),
                    ty,
                    is_pub: field.vis == FieldVis::Pub,
                    unusable,
                    span: Span::new(file, field.span.start as u32, field.span.end as u32),
                }
            })
            .collect();
        let sigs: Vec<_> = s
            .methods
            .into_iter()
            .map(|method| {
                let mut sig = mapper.sig(Some(id), method);
                hide_if_unreachable(symbols, &mut sig);
                sig
            })
            .collect();
        symbols.structs[id.0 as usize].fields = fields;
        for sig in sigs {
            let callee = Callee::Imported(ImportedFnId(symbols.imported.len() as u32));
            symbols
                .members
                .entry((UserType::Struct(id), sig.name.clone()))
                .or_insert(callee);
            symbols.imported.push(sig);
        }
        for (name, vis) in s.restricted_methods {
            symbols
                .restricted_members
                .entry((UserType::Struct(id), name))
                .or_insert(vis);
        }
        for (name, why) in s.skipped_methods {
            symbols
                .skipped_members
                .entry((UserType::Struct(id), name))
                .or_insert(why);
        }
    }

    for (module, id, e) in enums {
        let mapper = Mapper::new(symbols, module);
        let mut opaque = symbols.enums[id.0 as usize].opaque.clone();
        let variants = e
            .variants
            .into_iter()
            .map(|v| {
                let payload = v
                    .payload
                    .iter()
                    .map(|ty| match mapper.field(ty) {
                        Ok(ty) => {
                            let reach = symbols.reach(module, true);
                            if let Some((hidden, _)) = symbols.hidden_type(&ty, reach) {
                                opaque.get_or_insert_with(|| {
                                    format!(
                                        "has a variant, `{}`, holding a type Varyk cannot use \
                                         ({hidden})",
                                        v.name
                                    )
                                });
                            }
                            ty
                        }
                        Err(unusable) => {
                            opaque
                                .get_or_insert_with(|| unusable_payload_reason(&v.name, &unusable));
                            Ty::Unit
                        }
                    })
                    .collect();
                VariantDef {
                    name: v.name,
                    fields: VariantFieldsDef::Tuple(payload),
                }
            })
            .collect();
        symbols.enums[id.0 as usize].variants = variants;
        symbols.enums[id.0 as usize].opaque = opaque;
        for name in e.methods {
            symbols
                .skipped_members
                .entry((UserType::Enum(id), name))
                .or_insert(crate::interop::ENUM_METHOD);
        }
    }
}

/// Makes `sig` callable only from inside the module a type it names is
/// seen from (V0108 when called from elsewhere), when that is fewer
/// modules than it can be called from (M3 spec 4.2): the generated Rust
/// would name that type where it is private (rustc E0603).
/// A method's reach is its struct's, which is that of its own module, as
/// for a free function, since every imported item is `pub`.
fn hide_if_unreachable(symbols: &Symbols, sig: &mut ImportedSig) {
    if !sig.callable {
        return;
    }
    let reach = symbols.reach(sig.module, true);
    let hidden = sig
        .params
        .iter()
        .map(|(ty, _)| ty)
        .chain(std::iter::once(&sig.ret))
        .find_map(|ty| symbols.hidden_type(ty, reach));
    if let Some((note, within)) = hidden {
        sig.note = Some(note);
        sig.within = Some(within);
    }
}

/// Why a `.rs` struct is not imported (spec 4.1), as the end of a sentence
/// starting "it is not imported because"; `None` when it is imported.
fn why_skipped_struct(s: &ImportedStruct) -> Option<&'static str> {
    if s.generic {
        return Some(
            "it has type or lifetime parameters; Varyk imports only structs without them, so \
             wrap it in a struct that has none",
        );
    }
    if let Some(why) = s.unfit {
        return Some(why);
    }
    match s.kind {
        StructKind::Named => None,
        StructKind::Tuple => {
            Some("it is a tuple struct, which Varyk does not import yet; give its fields names")
        }
        StructKind::Unit => {
            Some("it is a unit struct, which Varyk does not import yet; give it a named field")
        }
    }
}

/// Why a `.rs` enum is not imported at all (spec 4.3), as the end of a
/// sentence starting "it is not imported because"; `None` when it is
/// imported (whether plainly or as an opaque type — an opaque enum is
/// still imported, just with its variants blocked, so it is not skipped
/// here).
fn why_skipped_enum(e: &ImportedEnum) -> Option<&'static str> {
    e.generic.then_some(
        "it has type or lifetime parameters; Varyk imports only enums without them, so wrap it \
         in an enum that has none",
    )
}

/// The note of a V0101 naming a skipped `.rs` struct or enum `name`
/// (spec 4.1, 4.3), or of an unmappable Rust type naming it (spec 4.4).
/// `kind` is `"struct"` or `"enum"`.
pub(super) fn skipped_note(kind: &str, name: &str, why: &str) -> String {
    format!("the Rust {kind} `{name}` is not imported because {why}")
}

/// Why a variant's payload makes its enum opaque, once every module's
/// imports are known (M3 spec 4.3, 4.4): the type, tidied, and what to
/// change when `unusable.note` says more than the type itself.
fn unusable_payload_reason(variant: &str, unusable: &Unusable) -> String {
    match &unusable.note {
        Some(note) => {
            format!("has a variant, `{variant}`, holding a type Varyk cannot use ({note})")
        }
        None => format!(
            "has a variant, `{variant}`, holding `{}`, which Varyk cannot use",
            tidy_signature(&unusable.rust_ty)
        ),
    }
}
