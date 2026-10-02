//! Which types can be cloned and which can be compared (M4 spec 2.10):
//! every Varyk struct and enum derives `Clone` and `PartialEq` where each
//! of its fields and payloads allows, and a Rust type imported from a
//! `.rs` module has what its `#[derive(..)]` lists (M4 spec 2.12).
//!
//! The judgement is an optimistic fixed point: every Varyk type starts
//! out allowed and loses a trait once one of its parts does not have it,
//! until nothing changes, so a type that contains itself through a `Vec`
//! or a `HashMap` is judged as if the recursion held, as Rust's derives
//! are. A type loses a trait only because of a part that had already lost
//! it the round before, so following the recorded parts always ends at a
//! Rust type.
//!
//! JSON (M5a spec 2.9) is judged apart: whether a type can go through
//! `json` at all ([`convertible`], a walk that stops at a type it has
//! already entered), which structs and enums a call reaches in which
//! direction ([`Reached`]), and the attribute checks only a reached type
//! is held to ([`reached_checks`]).

use std::collections::HashMap;

use varyk_syntax::Span;

use crate::diagnostics::{Diagnostic, codes};
use crate::resolve::{EnumDef, StructDef, VariantFieldsDef};
use crate::types::Ty;

/// The derives of one struct or enum: whether it can be cloned (`Clone`)
/// and compared with `==` (`PartialEq`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct Derives {
    pub clone: bool,
    pub eq: bool,
}

/// What keeps a type from being cloned or compared, in words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Blocker {
    /// The part of the value's own struct or enum that is in the way
    /// ("its field `handle`"); `None` when the value's type is not a
    /// struct or enum of its own but holds one (`Vec<Handle>`).
    pub field: Option<String>,
    /// Why, from that part down to the Rust type in the way, ending with
    /// what to change in the `.rs` file.
    pub reason: String,
}

/// The two derivable traits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Trait {
    Clone,
    PartialEq,
}

impl Trait {
    fn has(self, derives: Derives) -> bool {
        match self {
            Trait::Clone => derives.clone,
            Trait::PartialEq => derives.eq,
        }
    }

    /// The Rust name of the trait.
    fn name(self) -> &'static str {
        match self {
            Trait::Clone => "Clone",
            Trait::PartialEq => "PartialEq",
        }
    }

    /// "cannot be {verb}".
    fn verb(self) -> &'static str {
        match self {
            Trait::Clone => "copied",
            Trait::PartialEq => "compared",
        }
    }
}

/// A struct or enum, by its index in its table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Named {
    Struct(usize),
    Enum(usize),
}

/// The judgement for one trait: whether each struct and enum has it, and,
/// for a Varyk one without it, the index (in [`parts`] order) of the part
/// that took it away.
#[derive(Debug, Clone, Default)]
struct Solved {
    structs: Vec<(bool, Option<usize>)>,
    enums: Vec<(bool, Option<usize>)>,
}

impl Solved {
    fn get(&self, named: Named) -> (bool, Option<usize>) {
        match named {
            Named::Struct(id) => self.structs[id],
            Named::Enum(id) => self.enums[id],
        }
    }
}

/// Every struct's and enum's derives, and why a type cannot be cloned or
/// compared.
#[derive(Debug, Clone)]
pub struct Judged<'a> {
    structs: &'a [StructDef],
    enums: &'a [EnumDef],
    clone: Solved,
    eq: Solved,
}

/// Judges every struct and enum of the program. An imported one has what
/// its `#[derive(..)]` lists (`StructDef::derives`, `EnumDef::derives`);
/// a Varyk one what its fields or payloads allow.
pub fn compute<'a>(structs: &'a [StructDef], enums: &'a [EnumDef]) -> Judged<'a> {
    Judged {
        structs,
        enums,
        clone: solve(structs, enums, Trait::Clone),
        eq: solve(structs, enums, Trait::PartialEq),
    }
}

fn solve(structs: &[StructDef], enums: &[EnumDef], tr: Trait) -> Solved {
    let start = |derives: Option<Derives>| (derives.is_none_or(|d| tr.has(d)), None);
    let mut solved = Solved {
        structs: structs.iter().map(|d| start(d.derives)).collect(),
        enums: enums.iter().map(|d| start(d.derives)).collect(),
    };
    loop {
        let before = solved.clone();
        let mut changed = false;
        let named = (0..structs.len())
            .map(Named::Struct)
            .chain((0..enums.len()).map(Named::Enum));
        for named in named {
            if !before.get(named).0 || imported(structs, enums, named) {
                continue;
            }
            let blocking = parts(structs, enums, named)
                .iter()
                .position(|(_, ty)| !holds(&before, ty));
            if let Some(index) = blocking {
                let slot = match named {
                    Named::Struct(id) => &mut solved.structs[id],
                    Named::Enum(id) => &mut solved.enums[id],
                };
                *slot = (false, Some(index));
                changed = true;
            }
        }
        if !changed {
            return solved;
        }
    }
}

fn imported(structs: &[StructDef], enums: &[EnumDef], named: Named) -> bool {
    match named {
        Named::Struct(id) => structs[id].derives.is_some(),
        Named::Enum(id) => enums[id].derives.is_some(),
    }
}

/// Every field or payload of a Varyk struct or enum, in declaration
/// order, with its words for a message ("its field `name`").
fn parts<'a>(
    structs: &'a [StructDef],
    enums: &'a [EnumDef],
    named: Named,
) -> Vec<(String, &'a Ty)> {
    match named {
        Named::Struct(id) => structs[id]
            .fields
            .iter()
            .map(|field| (format!("its field `{}`", field.name), &field.ty))
            .collect(),
        Named::Enum(id) => {
            let mut parts = Vec::new();
            for variant in &enums[id].variants {
                let name = &variant.name;
                match &variant.fields {
                    VariantFieldsDef::Tuple(types) => {
                        for (index, ty) in types.iter().enumerate() {
                            let words = if types.len() == 1 {
                                format!("the value of its variant `{name}`")
                            } else {
                                format!("value {} of its variant `{name}`", index + 1)
                            };
                            parts.push((words, ty));
                        }
                    }
                    VariantFieldsDef::Named(fields) => {
                        for (field, ty) in fields {
                            parts
                                .push((format!("the field `{field}` of its variant `{name}`"), ty));
                        }
                    }
                }
            }
            parts
        }
    }
}

/// Whether a value of type `ty` has the trait, by `solved`.
fn holds(solved: &Solved, ty: &Ty) -> bool {
    blocking(solved, ty).is_none()
}

/// The first struct or enum inside `ty`, itself included, that lacks the
/// trait by `solved`.
fn blocking(solved: &Solved, ty: &Ty) -> Option<Named> {
    match ty {
        Ty::Struct(id) => {
            (!solved.structs[id.0 as usize].0).then_some(Named::Struct(id.0 as usize))
        }
        Ty::Enum(id) => (!solved.enums[id.0 as usize].0).then_some(Named::Enum(id.0 as usize)),
        Ty::Option(inner) | Ty::Vec(inner) => blocking(solved, inner),
        Ty::Result(a, b) | Ty::HashMap(a, b) => blocking(solved, a).or_else(|| blocking(solved, b)),
        // Numbers, `bool`, and `string` have both; a chain and `()` are
        // never a field and are not judged here.
        _ => None,
    }
}

impl Judged<'_> {
    /// The derives of struct `id`.
    pub fn of_struct(&self, id: usize) -> Derives {
        Derives {
            clone: self.clone.structs[id].0,
            eq: self.eq.structs[id].0,
        }
    }

    /// The derives of enum `id`.
    pub fn of_enum(&self, id: usize) -> Derives {
        Derives {
            clone: self.clone.enums[id].0,
            eq: self.eq.enums[id].0,
        }
    }

    /// Whether a value of type `ty` can be cloned; a number or `bool`
    /// can, though `.clone()` on one is refused for another reason.
    pub fn can_clone(&self, ty: &Ty) -> Result<(), Blocker> {
        self.check(ty, Trait::Clone)
    }

    /// Whether two values of type `ty` can be compared with `==`.
    pub fn can_compare(&self, ty: &Ty) -> Result<(), Blocker> {
        self.check(ty, Trait::PartialEq)
    }

    fn check(&self, ty: &Ty, tr: Trait) -> Result<(), Blocker> {
        let solved = match tr {
            Trait::Clone => &self.clone,
            Trait::PartialEq => &self.eq,
        };
        let Some(first) = blocking(solved, ty) else {
            return Ok(());
        };
        // The value's own struct or enum is named by the headline, so the
        // part in its way is the field; anything else starts the reason.
        let own = matches!(ty, Ty::Struct(_) | Ty::Enum(_));
        let mut field = None;
        let mut sentences = Vec::new();
        let mut at = first;
        loop {
            let (name, is_imported) = match at {
                Named::Struct(id) => (&self.structs[id].name, self.structs[id].derives.is_some()),
                Named::Enum(id) => (&self.enums[id].name, self.enums[id].derives.is_some()),
            };
            if is_imported {
                let tr = tr.name();
                sentences.push(format!(
                    "`{name}` is a Rust type whose `.rs` file does not derive `{tr}` for it; \
                     write `#[derive({tr})]` above it there, or add `{tr}` to its \
                     `#[derive(..)]` list (Varyk reads only `#[derive(..)]`, not a \
                     hand-written `impl`)"
                ));
                break;
            }
            let (_, Some(index)) = solved.get(at) else {
                // Unreachable by construction: a Varyk type lacks the
                // trait only through a recorded part.
                break;
            };
            let parts = parts(self.structs, self.enums, at);
            let Some((words, part_ty)) = parts.get(index) else {
                break;
            };
            if own && at == first && field.is_none() {
                field = Some(words.clone());
            } else {
                sentences.push(format!(
                    "`{name}` cannot be {} because of {words}",
                    tr.verb()
                ));
            }
            let Some(next) = blocking(solved, part_ty) else {
                break;
            };
            at = next;
        }
        Err(Blocker {
            field,
            reason: sentences.join("; "),
        })
    }
}

/// The name of `ty` as written in Varyk (`Vec<User>`).
pub fn ty_name(structs: &[StructDef], enums: &[EnumDef], ty: &Ty) -> String {
    let name = |ty: &Ty| ty_name(structs, enums, ty);
    match ty {
        Ty::Bool => "bool".to_string(),
        Ty::Int(kind) => kind.name().to_string(),
        Ty::Float(kind) => kind.name().to_string(),
        Ty::String => "string".to_string(),
        Ty::Struct(id) => structs[id.0 as usize].name.clone(),
        Ty::Enum(id) => enums[id.0 as usize].name.clone(),
        Ty::Option(inner) => format!("Option<{}>", name(inner)),
        Ty::Result(ok, err) => format!("Result<{}, {}>", name(ok), name(err)),
        Ty::Vec(inner) => format!("Vec<{}>", name(inner)),
        Ty::HashMap(key, value) => format!("HashMap<{}, {}>", name(key), name(value)),
        Ty::Chain(item) => format!("chain of {}", name(item)),
        Ty::Task(t) => format!("Task<{}>", name(t)),
        Ty::Shared(t) => format!("Shared<{}>", name(t)),
        Ty::Error => "Error".to_string(),
        Ty::Unit => "()".to_string(),
    }
}

/// Which ways a `json` or `env` call converts a struct or enum (M5a spec
/// 2.4): written (`Serialize`), read (`Deserialize`), or both. Set only on
/// a type a call reaches, which is then convertible.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct Serde {
    pub serialize: bool,
    pub deserialize: bool,
}

impl Serde {
    /// What either of two sets of calls reaches.
    pub fn or(self, other: Serde) -> Serde {
        Serde {
            serialize: self.serialize || other.serialize,
            deserialize: self.deserialize || other.deserialize,
        }
    }

    /// Whether any call reaches the type.
    pub fn any(self) -> bool {
        self.serialize || self.deserialize
    }

    fn has(self, direction: Direction) -> bool {
        match direction {
            Direction::Serialize => self.serialize,
            Direction::Deserialize => self.deserialize,
        }
    }

    fn set(&mut self, direction: Direction) {
        match direction {
            Direction::Serialize => self.serialize = true,
            Direction::Deserialize => self.deserialize = true,
        }
    }
}

/// The way a call converts: `json::stringify` writes, `json::parse` and
/// `env::parse` read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Serialize,
    Deserialize,
}

/// The part of a type that keeps it from going through `json` or `env`
/// (M5a spec 2.9).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Part {
    /// An enum with a variant that carries data: the enum and the first
    /// such variant.
    DataVariant {
        enum_name: String,
        variant: String,
    },
    /// A `HashMap` whose key is not `string`, named.
    MapKey(String),
    Error,
    /// A struct or enum imported from a `.rs` module, named.
    Rust(String),
    /// A struct or enum declared in another Varyk package (M5b2 spec
    /// 2.5): its name and the package's.
    Package {
        name: String,
        package: String,
    },
    /// Any other type (`Result`, `()`), named.
    Other(String),
    /// A `Shared` (milestone 5b1 spec 2.6), named: a handle, not data.
    Shared(String),
    /// `env` only: the type read is not a struct, named.
    NotStruct(String),
    /// `env` only: a field that is not a single value (a struct, `Vec`, or
    /// `HashMap`, possibly in an `Option`), named.
    NotFlat(String),
}

/// What a call converts through, for the words of its diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Medium {
    Json,
    Env,
}

impl Medium {
    /// How the medium is named in a sentence.
    pub fn name(self) -> &'static str {
        match self {
            Medium::Json => "JSON",
            Medium::Env => "the environment",
        }
    }
}

/// Why a type cannot go through `json` or `env`: the part in the way and
/// where it sits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotConvertible {
    pub part: Part,
    /// From the call's type down to the part, one step per field:
    /// "`Order`'s field `payment`". Empty when the part is the type
    /// itself or sits in its `Option`, `Vec`, or `HashMap`.
    pub path: Vec<String>,
    /// The declaration of the last field of `path`, which holds the part.
    pub at: Option<Span>,
}

impl NotConvertible {
    /// The label at the call: the part in the way, in plain words.
    pub fn label(&self, medium: Medium) -> String {
        match &self.part {
            Part::DataVariant { variant, .. } => format!("`{variant}` carries data"),
            Part::MapKey(map) => format!("`{map}` has keys that are not `string`"),
            Part::Error => "`Error` holds a message, not data".to_string(),
            Part::Rust(name) => format!("`{name}` is a Rust type from a `.rs` module"),
            Part::Package { name, package } => {
                format!("`{name}` is declared in the package `{package}`")
            }
            Part::Other(name) => {
                format!("`{name}` is not a type that {} can hold", medium.name())
            }
            Part::Shared(name) => format!("`{name}` is a handle to a value, not data"),
            Part::NotStruct(name) => format!("`{name}` is not a struct"),
            Part::NotFlat(name) => format!("`{name}` is more than one value"),
        }
    }

    /// Where the part sits, then what to write instead.
    pub fn notes(&self, medium: Medium) -> Vec<String> {
        let through = medium.name();
        let mut notes = Vec::new();
        if self.path.len() > 1 {
            notes.push(format!("it is inside {}", self.path.join(", then ")));
        }
        notes.push(match &self.part {
            Part::DataVariant { enum_name, .. } => match medium {
                Medium::Json => format!(
                    "only an enum whose variants carry no data can go through JSON; for data, \
                     use a struct with a field of a plain enum marked `#[rename(\"type\")]` (a \
                     `{enum_name}Kind` with a variant per kind) and an `Option` field for each \
                     kind's data"
                ),
                Medium::Env => format!(
                    "only an enum whose variants carry no data can be read from {through}; \
                     `{enum_name}` needs a plain enum of its kinds instead"
                ),
            },
            Part::MapKey(_) => {
                "the keys of a JSON object are text, so only a `HashMap<string, _>` can go \
                 through JSON"
                    .to_string()
            }
            Part::Error => {
                "keep the error's text instead, in a `string` field set from `e.message()`"
                    .to_string()
            }
            Part::Rust(_) => format!(
                "only Varyk's own types go through {through}; copy what is needed into a Varyk \
                 struct"
            ),
            Part::Package { name, package } => {
                let example = match medium {
                    Medium::Json => format!(
                        "`pub fn {}_json({}: {name}) -> string`",
                        snake(name),
                        snake(name)
                    ),
                    Medium::Env => format!("`pub fn {}() -> Result<{name}, Error>`", snake(name)),
                };
                let instead = if self.path.is_empty() {
                    format!("do it there instead, with a `pub fn` such as {example}")
                } else {
                    format!(
                        "do it there instead, or give this field a type of this package in \
                         place of `{name}`"
                    )
                };
                format!(
                    "a package is built without knowing who uses it, so only `{package}` can \
                     convert its own types through {through}; {instead}"
                )
            }
            Part::Other(_) => match medium {
                Medium::Json => "JSON holds numbers, `bool`, `string`, `Option`, `Vec`, \
                     `HashMap<string, _>`, structs, and enums whose variants carry no data"
                    .to_string(),
                Medium::Env => ENV_FIELDS.to_string(),
            },
            Part::Shared(_) => format!(
                "a `Shared` cannot go through {through}; read the fields through it into a \
                 value of a type that can"
            ),
            Part::NotStruct(_) => {
                "`env::parse` fills a struct, one field from each variable; read into a \
                 struct and use its fields"
                    .to_string()
            }
            Part::NotFlat(_) => format!(
                "a variable holds one value, so {ENV_FIELDS}; a field the program fills itself \
                 can be made an `Option` and marked `#[skip]`"
            ),
        });
        notes
    }
}

/// What the fields of a struct read from the environment can be.
const ENV_FIELDS: &str = "the fields of a struct read from the environment can be numbers, \
     `bool`, `string`, enums whose variants carry no data, or an `Option` of one of these";

/// Whether a value of type `ty` can be read by `env::parse` (M5a spec
/// 2.5): a Varyk struct whose fields, except skipped ones, are numbers,
/// `bool`, `string`, unit-only enums, or `Option`s of those. Otherwise the
/// first part in the way.
pub fn env_readable(
    structs: &[StructDef],
    enums: &[EnumDef],
    ty: &Ty,
) -> Result<(), NotConvertible> {
    let whole = |part| NotConvertible {
        part,
        path: Vec::new(),
        at: None,
    };
    if let Ty::Shared(_) = ty {
        return Err(whole(Part::Shared(ty_name(structs, enums, ty))));
    }
    let Ty::Struct(id) = ty else {
        return Err(whole(Part::NotStruct(ty_name(structs, enums, ty))));
    };
    let def = &structs[id.0 as usize];
    if let Some(item) = &def.package {
        return Err(whole(package_part(&def.name, &item.name)));
    }
    if def.imported {
        return Err(whole(Part::Rust(def.name.clone())));
    }
    for field in def.fields.iter().filter(|field| !field.attrs.skip) {
        let value = match &field.ty {
            Ty::Option(inner) => inner,
            other => other,
        };
        let part = match value {
            Ty::Bool | Ty::Int(_) | Ty::Float(_) | Ty::String => continue,
            Ty::Enum(_) => match convertible(structs, enums, value) {
                Ok(()) => continue,
                Err(blocked) => blocked.part,
            },
            Ty::Struct(_) | Ty::Vec(_) | Ty::HashMap(..) => {
                Part::NotFlat(ty_name(structs, enums, value))
            }
            Ty::Error => Part::Error,
            other => Part::Other(ty_name(structs, enums, other)),
        };
        return Err(NotConvertible {
            part,
            path: vec![format!("`{}`'s field `{}`", def.name, field.name)],
            at: Some(field.span),
        });
    }
    Ok(())
}

/// The part in the way for `name`, a struct or enum declared in the
/// Varyk package `package` (M5b2 spec 2.5).
fn package_part(name: &str, package: &str) -> Part {
    Part::Package {
        name: name.to_string(),
        package: package.to_string(),
    }
}

/// `name`, a type's name, in snake case: `stop` for `Stop`, `bus_stop`
/// for `BusStop`.
fn snake(name: &str) -> String {
    let mut out = String::new();
    for (index, c) in name.chars().enumerate() {
        if c.is_ascii_uppercase() && index > 0 {
            out.push('_');
        }
        out.push(c.to_ascii_lowercase());
    }
    out
}

/// Whether a value of type `ty` can go through `json` (M5a spec 2.9): a
/// number, `bool`, or `string`; an `Option` or `Vec` of a convertible
/// type; a `HashMap<string, _>` of one; a Varyk struct whose fields,
/// except skipped ones, are convertible; or a Varyk enum whose variants
/// carry no data. Otherwise the first part in the way.
pub fn convertible(
    structs: &[StructDef],
    enums: &[EnumDef],
    ty: &Ty,
) -> Result<(), NotConvertible> {
    let mut entered = Vec::new();
    let mut path = Vec::new();
    walk(structs, enums, ty, &mut entered, &mut path).map_err(|part| NotConvertible {
        part,
        at: path.last().map(|(_, span)| *span),
        path: path.into_iter().map(|(words, _)| words).collect(),
    })
}

/// [`convertible`] from `ty`, not entering a struct or enum twice: a type
/// holding itself is judged as if the recursion held, since the first
/// visit sees every part. `path` ends at the field holding the part in
/// the way, with its declaration.
fn walk(
    structs: &[StructDef],
    enums: &[EnumDef],
    ty: &Ty,
    entered: &mut Vec<Named>,
    path: &mut Vec<(String, Span)>,
) -> Result<(), Part> {
    match ty {
        Ty::Bool | Ty::Int(_) | Ty::Float(_) | Ty::String => Ok(()),
        Ty::Option(inner) | Ty::Vec(inner) => walk(structs, enums, inner, entered, path),
        Ty::HashMap(key, value) => {
            if **key != Ty::String {
                return Err(Part::MapKey(ty_name(structs, enums, ty)));
            }
            walk(structs, enums, value, entered, path)
        }
        Ty::Struct(id) => {
            let named = Named::Struct(id.0 as usize);
            if entered.contains(&named) {
                return Ok(());
            }
            entered.push(named);
            let def = &structs[id.0 as usize];
            if let Some(item) = &def.package {
                return Err(package_part(&def.name, &item.name));
            }
            if def.imported {
                return Err(Part::Rust(def.name.clone()));
            }
            for field in def.fields.iter().filter(|field| !field.attrs.skip) {
                let words = format!("`{}`'s field `{}`", def.name, field.name);
                path.push((words, field.span));
                walk(structs, enums, &field.ty, entered, path)?;
                path.pop();
            }
            Ok(())
        }
        Ty::Enum(id) => {
            let def = &enums[id.0 as usize];
            if let Some(item) = &def.package {
                return Err(package_part(&def.name, &item.name));
            }
            if def.imported {
                return Err(Part::Rust(def.name.clone()));
            }
            // A variant with braces, even empty ones (`A {}`), is a struct
            // variant to serde, so it counts as carrying data.
            let data = def
                .variants
                .iter()
                .find(|v| matches!(v.fields, VariantFieldsDef::Named(_)) || !v.types().is_empty());
            match data {
                Some(variant) => Err(Part::DataVariant {
                    enum_name: def.name.clone(),
                    variant: variant.name.clone(),
                }),
                None => Ok(()),
            }
        }
        Ty::Error => Err(Part::Error),
        // A `Shared` is a handle, not a value to write or read
        // (milestone 5b1 spec 2.6).
        Ty::Shared(_) => Err(Part::Shared(ty_name(structs, enums, ty))),
        Ty::Result(..) | Ty::Chain(_) | Ty::Task(_) | Ty::Unit => {
            Err(Part::Other(ty_name(structs, enums, ty)))
        }
    }
}

/// The structs and enums `json` and `env` calls reach, and in which
/// directions (M5a spec 2.4): what gets `Serialize` and `Deserialize`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Reached {
    /// Indexed by `StructId`.
    pub structs: Vec<Serde>,
    /// Indexed by `EnumId`.
    pub enums: Vec<Serde>,
    /// Indexed by `StructId`: the first call that reads the struct.
    read_at: Vec<Option<Span>>,
}

impl Reached {
    /// Nothing reached yet, for `structs` structs and `enums` enums.
    pub fn new(structs: usize, enums: usize) -> Self {
        Reached {
            structs: vec![Serde::default(); structs],
            enums: vec![Serde::default(); enums],
            read_at: vec![None; structs],
        }
    }

    /// Marks every struct and enum a value of the convertible type `ty`
    /// holds, `ty` itself included, as converted in `direction` by the call
    /// at `span`. Skipped fields are not followed.
    pub fn add(&mut self, structs: &[StructDef], ty: &Ty, direction: Direction, span: Span) {
        match ty {
            Ty::Option(inner) | Ty::Vec(inner) => self.add(structs, inner, direction, span),
            Ty::HashMap(_, value) => self.add(structs, value, direction, span),
            Ty::Struct(id) => {
                let id = id.0 as usize;
                if self.structs[id].has(direction) {
                    return;
                }
                self.structs[id].set(direction);
                if direction == Direction::Deserialize {
                    self.read_at[id] = Some(span);
                }
                for field in structs[id].fields.iter().filter(|field| !field.attrs.skip) {
                    self.add(structs, &field.ty, direction, span);
                }
            }
            Ty::Enum(id) => self.enums[id.0 as usize].set(direction),
            _ => {}
        }
    }
}

/// The text of a string literal as written between its quotes, with its
/// escapes (`\n`, `\t`, `\\`, `\"`, `\0`) read: a key as JSON sees it.
pub fn unescaped(text: &str) -> String {
    let mut out = String::new();
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('0') => out.push('\0'),
            Some(other) => out.push(other),
            None => out.push('\\'),
        }
    }
    out
}

/// V0209 for what only a reaching call decides (M5a spec 2.2): a skipped
/// field of a struct that is read, with no `#[default]` and not an
/// `Option`; and, on any reached type, two fields that are not skipped,
/// or two variants, with the same key. A key is the name or its
/// `#[rename]`, passed through `key` (the identity for JSON, upper case
/// for the environment); a variant's key is always as written.
pub fn reached_checks(
    structs: &[StructDef],
    enums: &[EnumDef],
    reached: &Reached,
    medium: Medium,
    key: fn(&str) -> String,
) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    for (id, def) in structs.iter().enumerate() {
        if !reached.structs[id].any() {
            continue;
        }
        if let (true, Some(read)) = (reached.structs[id].deserialize, reached.read_at[id]) {
            let unfilled = def.fields.iter().filter(|field| {
                field.attrs.skip
                    && field.attrs.default.is_none()
                    && !matches!(field.ty, Ty::Option(_))
            });
            for field in unfilled {
                diagnostics.push(
                    Diagnostic::new(
                        codes::V0209,
                        field.span,
                        format!(
                            "the field `{}` is skipped, so it has no value when `{}` is read",
                            field.name, def.name
                        ),
                    )
                    .with_label(read, format!("`{}` is read here", def.name))
                    .with_note(
                        "give it a `#[default(..)]`, or make it an `Option`, which is `None` \
                         when read",
                    ),
                );
            }
        }
        let mut seen: HashMap<String, usize> = HashMap::new();
        for (index, field) in def.fields.iter().enumerate() {
            if field.attrs.skip {
                continue;
            }
            let written = field
                .attrs
                .rename
                .as_deref()
                .map_or_else(|| field.name.clone(), unescaped);
            let field_key = key(&written);
            let Some(&first) = seen.get(&field_key) else {
                seen.insert(field_key, index);
                continue;
            };
            let first = &def.fields[first];
            let (what, note) = match medium {
                Medium::Json => (
                    "the key",
                    "a field's key is its name, or its `#[rename]`; each key of a struct must \
                     be different",
                ),
                Medium::Env => (
                    "the variable",
                    "a field reads the variable named by its key, its name or its `#[rename]`, \
                     in upper case; each field of a struct needs a different variable",
                ),
            };
            diagnostics.push(
                Diagnostic::new(
                    codes::V0209,
                    field.span,
                    format!(
                        "two fields of `{}` have {what} `{field_key}`: `{}` and `{}`",
                        def.name, first.name, field.name
                    ),
                )
                .with_label(first.span, format!("`{}` has it first", first.name))
                .with_note(note),
            );
        }
    }
    for (id, def) in enums.iter().enumerate() {
        if !reached.enums[id].any() {
            continue;
        }
        let mut seen: HashMap<String, &str> = HashMap::new();
        for variant in &def.variants {
            let variant_key = variant
                .rename
                .as_deref()
                .map_or_else(|| variant.name.clone(), unescaped);
            let Some(&first) = seen.get(&variant_key) else {
                seen.insert(variant_key, &variant.name);
                continue;
            };
            diagnostics.push(
                Diagnostic::new(
                    codes::V0209,
                    def.span,
                    format!(
                        "two variants of `{}` have the key `{variant_key}`: `{first}` and `{}`",
                        def.name, variant.name
                    ),
                )
                .with_note(
                    "a variant's key is its name, or its `#[rename]`; each key of an enum must \
                     be different",
                ),
            );
        }
    }
    diagnostics
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use varyk_syntax::{FileId, SourceFile};

    use super::*;
    use crate::resolve::{Resolved, resolve};

    fn resolved(entry: SourceFile) -> Resolved {
        let mut sources = Vec::new();
        match resolve(entry, &mut sources) {
            Ok(resolved) => resolved,
            Err(diagnostics) => panic!("expected no diagnostics, got {diagnostics:#?}"),
        }
    }

    fn resolved_str(text: &str) -> Resolved {
        resolved(SourceFile::new(FileId(0), "dummy/test.vr", text))
    }

    fn resolved_path(rel: &str) -> Resolved {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(rel);
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        resolved(SourceFile::new(FileId(0), path, text))
    }

    const BOTH: Derives = Derives {
        clone: true,
        eq: true,
    };

    /// The derives of the struct or enum named `name`.
    fn derives_of(resolved: &Resolved, name: &str) -> Derives {
        let symbols = &resolved.symbols;
        let judged = compute(&symbols.structs, &symbols.enums);
        if let Some(id) = symbols.structs.iter().position(|s| s.name == name) {
            return judged.of_struct(id);
        }
        match symbols.enums.iter().position(|e| e.name == name) {
            Some(id) => judged.of_enum(id),
            None => panic!("no type `{name}`"),
        }
    }

    #[test]
    fn a_struct_of_numbers_strings_and_containers_derives_both() {
        let resolved = resolved_str(
            "enum Unit {\n    C,\n    F,\n}\nstruct R {\n    a: i32,\n    b: f64,\n    c: bool,\n    \
             d: string,\n    e: Vec<string>,\n    f: Option<Unit>,\n    g: Result<i64, string>,\n    \
             h: HashMap<string, Vec<u8>>,\n}\nfn main() {}\n",
        );
        assert_eq!(derives_of(&resolved, "R"), BOTH);
        assert_eq!(derives_of(&resolved, "Unit"), BOTH);
    }

    #[test]
    fn f64_blocks_nothing() {
        let resolved = resolved_str("struct P {\n    x: f64,\n    y: f32,\n}\nfn main() {}\n");
        assert_eq!(derives_of(&resolved, "P"), BOTH);
    }

    #[test]
    fn a_field_of_a_rust_type_without_the_derive_blocks_clone_naming_the_field() {
        let resolved =
            resolved_path("crates/varyk/tests/fixtures/errors/v0203_clone_rust_field/main.vr");
        let symbols = &resolved.symbols;
        let judged = compute(&symbols.structs, &symbols.enums);
        let session = symbols
            .structs
            .iter()
            .position(|s| s.name == "Session")
            .unwrap_or(usize::MAX);
        assert_eq!(
            judged.of_struct(session),
            Derives {
                clone: false,
                eq: false
            }
        );
        let Err(blocker) = judged.can_clone(&Ty::Struct(crate::resolve::StructId(session as u32)))
        else {
            panic!("expected a blocker");
        };
        assert_eq!(blocker.field.as_deref(), Some("its field `handle`"));
        assert!(
            blocker.reason.contains("`Handle` is a Rust type"),
            "{blocker:?}"
        );
        assert!(blocker.reason.contains("add `Clone`"), "{blocker:?}");
        // In a `Vec`, no field of the value's own is in the way.
        let Err(blocker) = judged.can_clone(&Ty::Vec(Box::new(Ty::Struct(
            crate::resolve::StructId(session as u32),
        )))) else {
            panic!("expected a blocker");
        };
        assert_eq!(blocker.field, None);
        assert!(
            blocker
                .reason
                .starts_with("`Session` cannot be copied because of its field `handle`; `Handle`"),
            "{blocker:?}"
        );
    }

    #[test]
    fn a_rust_type_deriving_clone_only_allows_clone_only() {
        let resolved =
            resolved_path("crates/varyk/tests/fixtures/errors/v0203_compare_blocked/main.vr");
        assert_eq!(
            derives_of(&resolved, "Session"),
            Derives {
                clone: true,
                eq: false
            }
        );
        assert_eq!(
            derives_of(&resolved, "Handle"),
            Derives {
                clone: true,
                eq: false
            }
        );
    }

    #[test]
    fn recursion_through_a_vec_or_a_hash_map_derives_both() {
        let resolved = resolved_str(
            "enum Tree {\n    Leaf(i32),\n    Node(Vec<Tree>),\n}\nstruct Dir {\n    name: string,\n    \
             children: HashMap<string, Dir>,\n}\nfn main() {}\n",
        );
        assert_eq!(derives_of(&resolved, "Tree"), BOTH);
        assert_eq!(derives_of(&resolved, "Dir"), BOTH);
    }

    // --- JSON (M5a spec 2.9) ------------------------------------------------

    fn struct_ty(resolved: &Resolved, name: &str) -> Ty {
        match resolved.symbols.structs.iter().position(|s| s.name == name) {
            Some(id) => Ty::Struct(crate::resolve::StructId(id as u32)),
            None => panic!("no struct `{name}`"),
        }
    }

    fn convertible_ty(resolved: &Resolved, ty: &Ty) -> Result<(), NotConvertible> {
        let symbols = &resolved.symbols;
        convertible(&symbols.structs, &symbols.enums, ty)
    }

    fn span() -> varyk_syntax::Span {
        varyk_syntax::Span::new(FileId(0), 0, 0)
    }

    const NESTED: &str = "enum Role {\n    Admin,\n    Member,\n}\nstruct Address {\n    city: string,\n}\n\
        struct User {\n    name: string,\n    home: Option<Address>,\n    role: Role,\n    \
        tags: HashMap<string, Vec<u8>>,\n}\nstruct Unused {\n    n: i32,\n}\nfn main() {}\n";

    #[test]
    fn a_nested_struct_reached_from_a_write_is_serialize_only() {
        let resolved = resolved_str(NESTED);
        let symbols = &resolved.symbols;
        let user = struct_ty(&resolved, "User");
        assert_eq!(convertible_ty(&resolved, &user), Ok(()));
        let mut reached = Reached::new(symbols.structs.len(), symbols.enums.len());
        reached.add(&symbols.structs, &user, Direction::Serialize, span());
        let write = Serde {
            serialize: true,
            deserialize: false,
        };
        let names: Vec<(&str, Serde)> = symbols
            .structs
            .iter()
            .map(|s| s.name.as_str())
            .zip(reached.structs.iter().copied())
            .collect();
        assert_eq!(
            names,
            [
                ("Address", write),
                ("User", write),
                ("Unused", Serde::default())
            ]
        );
        assert_eq!(reached.enums, [write]);
    }

    #[test]
    fn a_read_marks_deserialize_and_both_directions_add_up() {
        let resolved = resolved_str(NESTED);
        let symbols = &resolved.symbols;
        let user = struct_ty(&resolved, "User");
        let mut reached = Reached::new(symbols.structs.len(), symbols.enums.len());
        reached.add(
            &symbols.structs,
            &Ty::Vec(Box::new(user.clone())),
            Direction::Deserialize,
            span(),
        );
        assert_eq!(
            reached.structs[1],
            Serde {
                serialize: false,
                deserialize: true
            }
        );
        reached.add(&symbols.structs, &user, Direction::Serialize, span());
        assert_eq!(
            reached.structs[0],
            Serde {
                serialize: true,
                deserialize: true
            }
        );
    }

    #[test]
    fn a_skipped_field_is_neither_reached_nor_judged() {
        let resolved = resolved_str(
            "enum Shape {\n    Dot(i32),\n}\nstruct Cache {\n    n: i32,\n}\nstruct S {\n    n: i32,\n    \
             #[skip]\n    shape: Option<Shape>,\n    #[skip]\n    cache: Option<Cache>,\n}\nfn main() {}\n",
        );
        let symbols = &resolved.symbols;
        let s = struct_ty(&resolved, "S");
        assert_eq!(convertible_ty(&resolved, &s), Ok(()));
        let mut reached = Reached::new(symbols.structs.len(), symbols.enums.len());
        reached.add(&symbols.structs, &s, Direction::Serialize, span());
        assert_eq!(reached.structs[0], Serde::default());
        assert_eq!(reached.enums[0], Serde::default());
    }

    #[test]
    fn an_enum_with_data_is_not_convertible_naming_the_variant() {
        let resolved = resolved_str(
            "enum Payment {\n    Cash,\n    Card(string),\n}\nstruct Order {\n    id: i32,\n    \
             payment: Payment,\n}\nfn main() {}\n",
        );
        let order = struct_ty(&resolved, "Order");
        let Err(blocked) = convertible_ty(&resolved, &order) else {
            panic!("expected a blocker");
        };
        assert_eq!(
            blocked.part,
            Part::DataVariant {
                enum_name: "Payment".to_string(),
                variant: "Card".to_string()
            }
        );
        assert_eq!(blocked.path, ["`Order`'s field `payment`"]);
        assert_eq!(blocked.label(Medium::Json), "`Card` carries data");
        let notes = blocked.notes(Medium::Json);
        assert!(
            notes.iter().any(|n| n.contains("#[rename(\"type\")]")),
            "{notes:#?}"
        );
        // One field deep, the label at the field says where it is.
        assert!(blocked.at.is_some());
        assert!(
            !notes.iter().any(|n| n.contains("it is inside")),
            "{notes:#?}"
        );
    }

    #[test]
    fn a_variant_with_empty_braces_carries_data() {
        let resolved = resolved_str(
            "enum Kind {\n    A {},\n    B,\n}\nstruct C {\n    kind: Kind,\n}\nfn main() {}\n",
        );
        let c = struct_ty(&resolved, "C");
        let Err(blocked) = convertible_ty(&resolved, &c) else {
            panic!("expected a blocker");
        };
        assert_eq!(
            blocked.part,
            Part::DataVariant {
                enum_name: "Kind".to_string(),
                variant: "A".to_string()
            }
        );
    }

    #[test]
    fn a_map_with_other_keys_an_error_and_a_rust_type_are_not_convertible() {
        let resolved = resolved_str("struct S {\n    n: i32,\n}\nfn main() {}\n");
        let map = Ty::HashMap(
            Box::new(Ty::Int(crate::types::IntKind::I32)),
            Box::new(Ty::String),
        );
        let Err(blocked) = convertible_ty(&resolved, &map) else {
            panic!("expected a blocker");
        };
        assert_eq!(
            blocked.part,
            Part::MapKey("HashMap<i32, string>".to_string())
        );
        assert!(blocked.path.is_empty());
        let Err(blocked) = convertible_ty(&resolved, &Ty::Option(Box::new(Ty::Error))) else {
            panic!("expected a blocker");
        };
        assert_eq!(blocked.part, Part::Error);
        let resolved =
            resolved_path("crates/varyk/tests/fixtures/errors/v0203_clone_rust_field/main.vr");
        let session = struct_ty(&resolved, "Session");
        let Err(blocked) = convertible_ty(&resolved, &session) else {
            panic!("expected a blocker");
        };
        assert_eq!(blocked.part, Part::Rust("Handle".to_string()));
        assert_eq!(blocked.path, ["`Session`'s field `handle`"]);
        let result = Ty::Result(Box::new(Ty::Bool), Box::new(Ty::String));
        let Err(blocked) = convertible_ty(&resolved, &result) else {
            panic!("expected a blocker");
        };
        assert_eq!(
            blocked.part,
            Part::Other("Result<bool, string>".to_string())
        );
    }

    #[test]
    fn a_type_holding_itself_through_a_vec_is_judged_without_looping() {
        let resolved = resolved_str(
            "struct Dir {\n    name: string,\n    children: Vec<Dir>,\n    parent: Option<Link>,\n}\n\
             struct Link {\n    to: Vec<Dir>,\n}\nfn main() {}\n",
        );
        let symbols = &resolved.symbols;
        let dir = struct_ty(&resolved, "Dir");
        assert_eq!(convertible_ty(&resolved, &dir), Ok(()));
        let mut reached = Reached::new(symbols.structs.len(), symbols.enums.len());
        reached.add(&symbols.structs, &dir, Direction::Deserialize, span());
        assert!(reached.structs.iter().all(|s| s.deserialize));
        let resolved = resolved_str(
            "enum Kind {\n    Big(i32),\n}\nstruct Tree {\n    kids: Vec<Tree>,\n    kind: Kind,\n}\nfn main() {}\n",
        );
        let tree = struct_ty(&resolved, "Tree");
        assert!(convertible_ty(&resolved, &tree).is_err());
    }

    /// The reachability-dependent V0209 checks of `text` after the calls
    /// `calls` (a struct's name and the direction).
    fn reached_errors(text: &str, calls: &[(&str, Direction)]) -> Vec<Diagnostic> {
        let resolved = resolved_str(text);
        let symbols = &resolved.symbols;
        let mut reached = Reached::new(symbols.structs.len(), symbols.enums.len());
        for (name, direction) in calls {
            let ty = struct_ty(&resolved, name);
            reached.add(&symbols.structs, &ty, *direction, span());
        }
        reached_checks(
            &symbols.structs,
            &symbols.enums,
            &reached,
            Medium::Json,
            str::to_string,
        )
    }

    const SKIPPED: &str = "struct User {\n    name: string,\n    #[skip]\n    hash: string,\n    \
        #[skip]\n    #[default(3)]\n    tries: i32,\n    #[skip]\n    note: Option<string>,\n}\nfn main() {}\n";

    #[test]
    fn a_read_skipped_field_with_no_default_is_v0209() {
        let errors = reached_errors(SKIPPED, &[("User", Direction::Deserialize)]);
        assert_eq!(errors.len(), 1, "{errors:#?}");
        let d = &errors[0];
        assert_eq!(d.code, crate::diagnostics::codes::V0209);
        assert!(d.message.contains("`hash`"), "{d:#?}");
        assert!(d.notes.iter().any(|n| n.contains("#[default")), "{d:#?}");
    }

    #[test]
    fn a_skipped_field_with_no_default_is_fine_when_only_written() {
        let errors = reached_errors(SKIPPED, &[("User", Direction::Serialize)]);
        assert!(errors.is_empty(), "{errors:#?}");
        assert!(reached_errors(SKIPPED, &[]).is_empty());
    }

    #[test]
    fn two_fields_with_the_same_key_after_rename_are_v0209() {
        let text = "struct User {\n    #[rename(\"name\")]\n    user_name: string,\n    name: string,\n    \
            #[skip]\n    #[rename(\"id\")]\n    a: Option<i32>,\n    id: i32,\n}\nfn main() {}\n";
        let errors = reached_errors(text, &[("User", Direction::Serialize)]);
        assert_eq!(errors.len(), 1, "{errors:#?}");
        let d = &errors[0];
        assert_eq!(d.code, crate::diagnostics::codes::V0209);
        assert!(d.message.contains("`name`"), "{d:#?}");
        assert_eq!(d.labels.len(), 1, "{d:#?}");
        // Unreached, the same struct is fine.
        assert!(reached_errors(text, &[]).is_empty());
    }

    #[test]
    fn a_renamed_variant_equal_to_another_is_v0209() {
        let text = "enum Role {\n    #[rename(\"Member\")]\n    Admin,\n    Member,\n}\nstruct User {\n    \
            role: Role,\n}\nfn main() {}\n";
        let errors = reached_errors(text, &[("User", Direction::Deserialize)]);
        assert_eq!(errors.len(), 1, "{errors:#?}");
        let d = &errors[0];
        assert_eq!(d.code, crate::diagnostics::codes::V0209);
        assert!(d.message.contains("`Role`"), "{d:#?}");
        assert!(d.message.contains("`Member`"), "{d:#?}");
    }

    #[test]
    fn keys_compare_after_escapes_and_through_the_key_function() {
        let text =
            "struct E {\n    #[rename(\"PORT\")]\n    a: i32,\n    port: i32,\n}\nfn main() {}\n";
        let resolved = resolved_str(text);
        let symbols = &resolved.symbols;
        let mut reached = Reached::new(symbols.structs.len(), symbols.enums.len());
        reached.add(
            &symbols.structs,
            &struct_ty(&resolved, "E"),
            Direction::Deserialize,
            span(),
        );
        let same = reached_checks(
            &symbols.structs,
            &symbols.enums,
            &reached,
            Medium::Json,
            str::to_string,
        );
        assert!(same.is_empty(), "{same:#?}");
        let upper = reached_checks(
            &symbols.structs,
            &symbols.enums,
            &reached,
            Medium::Env,
            |key| key.to_uppercase(),
        );
        assert_eq!(upper.len(), 1, "{upper:#?}");
        assert_eq!(unescaped("a\\tb\\\\\\\"\\n\\0"), "a\tb\\\"\n\0");
    }

    #[test]
    fn mutually_recursive_structs_through_vec_derive_both() {
        let resolved = resolved_str(
            "struct A {\n    bs: Vec<B>,\n}\nstruct B {\n    a: Option<A>,\n    as_: Vec<A>,\n}\nfn main() {}\n",
        );
        assert_eq!(derives_of(&resolved, "A"), BOTH);
        assert_eq!(derives_of(&resolved, "B"), BOTH);
    }
}
