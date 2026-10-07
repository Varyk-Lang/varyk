//! Mapping `syn` types to [`RustTy`] (M1 spec 4.5, extended by M3 spec
//! 4.4).

use std::collections::HashSet;

use quote::ToTokens;
use syn::{GenericArgument, PathArguments, Type, TypeReference};

use super::ident_name;

/// A Rust type, mapped to the subset the spec's import table assigns a
/// Varyk type and mode.
///
/// `Ref`/`RefMut` are built over a type that could be a value (a
/// primitive, [`RustTy::Named`], `Vec`, `Option`, or `Result`) — a
/// reference to `str`/`String` has its own variant, and one to anything
/// else is [`Opaque`].
///
/// [`Opaque`]: RustTy::Opaque
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RustTy {
    Bool,
    I8,
    I16,
    I32,
    I64,
    U8,
    U16,
    U32,
    U64,
    Usize,
    F32,
    F64,
    /// `&str`.
    Str,
    /// Owned `String`.
    String,
    /// `&mut String`.
    RefMutString,
    /// `&'static str` as a parameter: text written in the program
    /// (milestone 5b3 spec 2.3). Anywhere else it is [`RustTy::Opaque`].
    Literal,
    /// `Vec<varyk_std::Value>` as the last parameter: any number of
    /// values after the others (milestone 5b3 spec 2.2). Anywhere else it
    /// is [`RustTy::Opaque`].
    Values,
    /// `&T`.
    Ref(Box<RustTy>),
    /// `&mut T`.
    RefMut(Box<RustTy>),
    /// `()`, or a missing return type.
    Unit,
    /// A type item named by path (spec 4.4); the resolver decides whether
    /// it is an imported struct.
    Named(RustPath),
    /// `Vec<T>`.
    Vec(Box<RustTy>),
    /// `Option<T>`.
    Option(Box<RustTy>),
    /// `Result<T, E>`.
    Result(Box<RustTy>, Box<RustTy>),
    /// `varyk_std::Error`, written by that full path (milestone 5b3 spec
    /// 2.4); the resolver accepts it only as the error of a `Result`.
    Error,
    /// `varyk_std::Time`, written by that full path (milestone 5c spec
    /// 2.5).
    Time,
    /// `varyk_std::Uuid`, written by that full path (milestone 5c spec
    /// 2.5).
    Uuid,
    /// `varyk_std::Bytes`, written by that full path (milestone 5c spec
    /// 2.5).
    Bytes,
    /// The function's one type parameter, filled at the call from where
    /// the result goes (milestone 5b3 spec 2.1); only in a return of
    /// `Result<T, varyk_std::Error>`, `Result<Option<T>, ..>`, or
    /// `Result<Vec<T>, ..>`.
    Param,
    /// `&T` as a parameter, `T` the function's one type parameter bounded
    /// by `Serialize + ?Sized`, filled from the argument (milestone 5b4
    /// spec 2.7); only where the importer accepted that shape.
    SerializeParam,
    /// Anything else: generics, explicit lifetimes, trait objects, `impl
    /// Trait`, tuples, arrays, references to anything unmapped, `std`
    /// paths, and references in return position. Holds the original type
    /// text via [`quote::ToTokens`].
    Opaque(String),
}

impl RustTy {
    /// The type as Rust spells it, for diagnostics.
    pub fn text(&self) -> String {
        let simple = match self {
            RustTy::Bool => "bool",
            RustTy::I8 => "i8",
            RustTy::I16 => "i16",
            RustTy::I32 => "i32",
            RustTy::I64 => "i64",
            RustTy::U8 => "u8",
            RustTy::U16 => "u16",
            RustTy::U32 => "u32",
            RustTy::U64 => "u64",
            RustTy::Usize => "usize",
            RustTy::F32 => "f32",
            RustTy::F64 => "f64",
            RustTy::Str => "&str",
            RustTy::String => "String",
            RustTy::RefMutString => "&mut String",
            RustTy::Literal => "&'static str",
            RustTy::Values => "Vec<varyk_std::Value>",
            RustTy::Unit => "()",
            RustTy::Error => "varyk_std::Error",
            RustTy::Time => "varyk_std::Time",
            RustTy::Uuid => "varyk_std::Uuid",
            RustTy::Bytes => "varyk_std::Bytes",
            RustTy::Param => "T",
            RustTy::SerializeParam => "&T",
            RustTy::Ref(inner) => return format!("&{}", inner.text()),
            RustTy::RefMut(inner) => return format!("&mut {}", inner.text()),
            RustTy::Named(
                RustPath::Local(name)
                | RustPath::Used(name)
                | RustPath::Glob(name)
                | RustPath::Macro(name),
            ) => {
                return name.clone();
            }
            RustTy::Named(RustPath::Crate(segments)) => {
                return format!("crate::{}", segments.join("::"));
            }
            RustTy::Vec(inner) => return format!("Vec<{}>", inner.text()),
            RustTy::Option(inner) => return format!("Option<{}>", inner.text()),
            RustTy::Result(ok, err) => return format!("Result<{}, {}>", ok.text(), err.text()),
            RustTy::Opaque(text) => return text.clone(),
        };
        simple.to_string()
    }
}

/// How a signature or field names a type item (spec 4.4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RustPath {
    /// A bare name of an item of this file (or `Self` in an `impl` block,
    /// already replaced by the struct's name).
    Local(String),
    /// `crate::a::b::T`: the segments after `crate`, the item's name last.
    Crate(Vec<String>),
    /// A bare name a `use` line of this file brings in; not followed, so
    /// never mapped ("write the full path").
    Used(String),
    /// A bare name no item or named `use` of this file declares, in a file
    /// with a glob `use`, which could bring in its own item of that name;
    /// never mapped ("replace the glob").
    Glob(String),
    /// A bare name no item or named `use` of this file declares, or one a
    /// primitive or `std` type has, in a file with an item macro that
    /// could define type names (see [`super::collect_names`]); never
    /// mapped ("move the macro").
    Macro(String),
}

/// An item of `varyk-std` a `.rs` signature may name (milestone 5b3
/// spec 2.2-2.4, milestone 5c spec 2.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum StdItem {
    /// `varyk_std::Error`.
    Error,
    /// `varyk_std::Value`.
    Value,
    /// `varyk_std::Time`.
    Time,
    /// `varyk_std::Uuid`.
    Uuid,
    /// `varyk_std::Bytes`.
    Bytes,
    /// `serde::de::DeserializeOwned` or
    /// `varyk_std::serde::de::DeserializeOwned`, the bound of a type
    /// parameter.
    DeserializeOwned,
    /// `serde::Serialize` or `varyk_std::serde::Serialize`, the bound,
    /// beside `?Sized`, of a type parameter filled from an argument
    /// (milestone 5b4 spec 2.7).
    Serialize,
}

/// The [`StdItem`] `path` names, by its full path (a leading `::` is
/// allowed), with no generic arguments; `None` for any other path,
/// including a bare name a `use` brings in.
pub(super) fn std_item(path: &syn::Path) -> Option<StdItem> {
    let segments: Option<Vec<String>> = path
        .segments
        .iter()
        .map(|segment| {
            matches!(segment.arguments, PathArguments::None).then(|| ident_name(&segment.ident))
        })
        .collect();
    let segments = segments?;
    let segments: Vec<&str> = segments.iter().map(String::as_str).collect();
    match segments[..] {
        ["varyk_std", "Error"] => Some(StdItem::Error),
        ["varyk_std", "Value"] => Some(StdItem::Value),
        ["varyk_std", "Time"] => Some(StdItem::Time),
        ["varyk_std", "Uuid"] => Some(StdItem::Uuid),
        ["varyk_std", "Bytes"] => Some(StdItem::Bytes),
        ["serde", "de", "DeserializeOwned"] | ["varyk_std", "serde", "de", "DeserializeOwned"] => {
            Some(StdItem::DeserializeOwned)
        }
        ["serde", "Serialize"] | ["varyk_std", "serde", "Serialize"] => Some(StdItem::Serialize),
        _ => None,
    }
}

/// What bare names mean in the file being imported; see
/// [`super::collect_names`].
#[derive(Debug, Clone, Default)]
pub(super) struct Names {
    pub types: HashSet<String>,
    pub used: HashSet<String>,
    pub glob: bool,
    /// Some top-level item macro could define type names.
    pub expands: bool,
    pub shadowed: HashSet<String>,
    /// Inside an `impl S` block, `S`: what `Self` means.
    pub self_ty: Option<String>,
    /// The signature's accepted type parameter, which is
    /// [`RustTy::Param`] (milestone 5b3 spec 2.1), and `&T` of it
    /// [`RustTy::SerializeParam`] (milestone 5b4 spec 2.7).
    pub param: Option<String>,
}

impl Names {
    /// These names, with `Self` meaning `owner`.
    pub(super) fn in_impl(&self, owner: &str) -> Names {
        Names {
            self_ty: Some(owner.to_string()),
            ..self.clone()
        }
    }
}

/// The identifier of a type written as a single bare path segment with no
/// generic arguments (`i32`, `String`) — `None` for anything qualified,
/// multi-segment, or with `<...>` arguments (`Vec<i32>`, `std::fmt::Debug`,
/// `<T as Trait>::Assoc`).
fn plain_path_ident(ty: &Type) -> Option<String> {
    let Type::Path(type_path) = ty else {
        return None;
    };
    if type_path.qself.is_some() || type_path.path.leading_colon.is_some() {
        return None;
    }
    let [segment] = type_path.path.segments.iter().collect::<Vec<_>>()[..] else {
        return None;
    };
    if !matches!(segment.arguments, PathArguments::None) {
        return None;
    }
    Some(ident_name(&segment.ident))
}

/// [`plain_path_ident`], but `None` when that name is shadowed by one of
/// the file's own names — such a name never resolves to a primitive or
/// `std` type this module maps by spelling.
fn bare_ident(ty: &Type, names: &Names) -> Option<String> {
    let name = plain_path_ident(ty)?;
    if names.shadowed.contains(&name) {
        None
    } else {
        Some(name)
    }
}

/// Maps a bare (non-reference) primitive type name to its [`RustTy`].
fn primitive_ty(ty: &Type, names: &Names) -> Option<RustTy> {
    Some(match bare_ident(ty, names)?.as_str() {
        "bool" => RustTy::Bool,
        "i8" => RustTy::I8,
        "i16" => RustTy::I16,
        "i32" => RustTy::I32,
        "i64" => RustTy::I64,
        "u8" => RustTy::U8,
        "u16" => RustTy::U16,
        "u32" => RustTy::U32,
        "u64" => RustTy::U64,
        "usize" => RustTy::Usize,
        "f32" => RustTy::F32,
        "f64" => RustTy::F64,
        _ => return None,
    })
}

fn is_named(ty: &Type, name: &str, names: &Names) -> bool {
    bare_ident(ty, names).as_deref() == Some(name)
}

fn type_to_text(ty: &Type) -> String {
    ty.to_token_stream().to_string()
}

/// A bare name that is not a primitive or `String` (spec 4.4): `Self`, an
/// item of this file, or a name a `use` (or a glob) brings in; anything
/// else (a generic parameter, a prelude type) is `None`.
fn named(name: &str, names: &Names) -> Option<RustTy> {
    if name == "Self" {
        return names
            .self_ty
            .as_ref()
            .map(|owner| RustTy::Named(RustPath::Local(owner.clone())));
    }
    if names.types.contains(name) {
        return Some(RustTy::Named(RustPath::Local(name.to_string())));
    }
    if names.used.contains(name) {
        return Some(RustTy::Named(RustPath::Used(name.to_string())));
    }
    if names.glob {
        return Some(RustTy::Named(RustPath::Glob(name.to_string())));
    }
    if names.expands {
        return Some(RustTy::Named(RustPath::Macro(name.to_string())));
    }
    None
}

/// `crate::a::b::T` with no generic arguments anywhere: the segments
/// after `crate`.
fn crate_path(ty: &Type) -> Option<Vec<String>> {
    let Type::Path(type_path) = ty else {
        return None;
    };
    if type_path.qself.is_some() || type_path.path.leading_colon.is_some() {
        return None;
    }
    let mut segments = type_path.path.segments.iter();
    let first = segments.next()?;
    if ident_name(&first.ident) != "crate" || !matches!(first.arguments, PathArguments::None) {
        return None;
    }
    let rest: Option<Vec<String>> = segments
        .map(|segment| {
            matches!(segment.arguments, PathArguments::None).then(|| ident_name(&segment.ident))
        })
        .collect();
    rest.filter(|rest| !rest.is_empty())
}

/// `Vec<T>`, `Option<T>`, or `Result<T, E>` (unshadowed), each argument
/// mapped as a value; `None` for any other type.
fn generic_std(ty: &Type, names: &Names) -> Option<RustTy> {
    let Type::Path(type_path) = ty else {
        return None;
    };
    if type_path.qself.is_some() || type_path.path.leading_colon.is_some() {
        return None;
    }
    let [segment] = type_path.path.segments.iter().collect::<Vec<_>>()[..] else {
        return None;
    };
    let name = ident_name(&segment.ident);
    if names.shadowed.contains(&name) {
        // Shadowed only because a macro could define it: say so.
        let own = names.types.contains(&name) || names.used.contains(&name) || names.glob;
        return (names.expands && !own).then_some(RustTy::Named(RustPath::Macro(name)));
    }
    let PathArguments::AngleBracketed(arguments) = &segment.arguments else {
        return None;
    };
    let args: Option<Vec<RustTy>> = arguments
        .args
        .iter()
        .map(|arg| match arg {
            GenericArgument::Type(ty) => Some(map_value(ty, names)),
            _ => None,
        })
        .collect();
    let mut args = args?;
    // An argument Varyk cannot use makes the whole type opaque, with its
    // text, like any other unmapped type.
    if args.iter().any(|arg| matches!(arg, RustTy::Opaque(_))) {
        return Some(RustTy::Opaque(type_to_text(ty)));
    }
    Some(match (name.as_str(), args.len()) {
        ("Vec", 1) => RustTy::Vec(Box::new(args.remove(0))),
        ("Option", 1) => RustTy::Option(Box::new(args.remove(0))),
        ("Result", 2) => {
            let ok = args.remove(0);
            RustTy::Result(Box::new(ok), Box::new(args.remove(0)))
        }
        _ => return None,
    })
}

/// Maps a reference type (`&T` / `&mut T`) appearing in parameter position.
/// `&'static str` is [`RustTy::Literal`] (milestone 5b3 spec 2.3); any
/// other reference carrying an explicit lifetime is [`RustTy::Opaque`],
/// per spec 4.5's "explicit lifetimes ... imported as opaque".
fn map_reference(reference: &TypeReference, whole: &Type, names: &Names) -> RustTy {
    let inner = reference.elem.as_ref();
    if let Some(lifetime) = &reference.lifetime {
        let literal = lifetime.ident == "static"
            && reference.mutability.is_none()
            && is_named(inner, "str", names);
        return if literal {
            RustTy::Literal
        } else {
            RustTy::Opaque(type_to_text(whole))
        };
    }
    // `&str` is the table's shared-borrow-of-string case; `&mut str` isn't
    // in the table at all (mutable string slices are not part of the
    // spec's mapping), so it falls through to the generic Opaque case
    // below rather than being conflated with `&str`.
    if is_named(inner, "str", names) && reference.mutability.is_none() {
        return RustTy::Str;
    }
    // `&mut String` maps; a shared `&String` is Opaque (not callable),
    // and the type checker suggests `&str`.
    if is_named(inner, "String", names) {
        return match reference.mutability {
            Some(_) => RustTy::RefMutString,
            None => RustTy::Opaque(type_to_text(whole)),
        };
    }
    match map_value(inner, names) {
        RustTy::Opaque(_) | RustTy::Unit => RustTy::Opaque(type_to_text(whole)),
        // The importer admits the type parameter of a parameter only as
        // `&T` (milestone 5b4 spec 2.7).
        RustTy::Param if reference.mutability.is_none() => RustTy::SerializeParam,
        value if reference.mutability.is_some() => RustTy::RefMut(Box::new(value)),
        value => RustTy::Ref(Box::new(value)),
    }
}

/// Maps a type in value position (a field, a generic argument, or a
/// non-reference parameter or return type): a reference here is always
/// [`RustTy::Opaque`].
pub(super) fn map_value(ty: &Type, names: &Names) -> RustTy {
    // A type parameter hides any type of its name.
    if names.param.is_some() && plain_path_ident(ty) == names.param {
        return RustTy::Param;
    }
    if let Some(prim) = primitive_ty(ty, names) {
        return prim;
    }
    if is_named(ty, "String", names) {
        return RustTy::String;
    }
    if matches!(ty, Type::Tuple(tuple) if tuple.elems.is_empty()) {
        return RustTy::Unit;
    }
    if let Some(found) = plain_path_ident(ty).and_then(|name| named(&name, names)) {
        return found;
    }
    if let Some(path) = crate_path(ty) {
        return RustTy::Named(RustPath::Crate(path));
    }
    if let Some(found) = generic_std(ty, names) {
        return found;
    }
    // A `.rs` file's own `mod varyk_std` would be read as varyk-std's
    // here too; accepted for the MVP.
    if let Type::Path(type_path) = ty {
        if type_path.qself.is_none() {
            match std_item(&type_path.path) {
                Some(StdItem::Error) => return RustTy::Error,
                Some(StdItem::Time) => return RustTy::Time,
                Some(StdItem::Uuid) => return RustTy::Uuid,
                Some(StdItem::Bytes) => return RustTy::Bytes,
                _ => {}
            }
        }
    }
    RustTy::Opaque(type_to_text(ty))
}

/// Maps a parameter type; `last` when it is the last parameter, the one
/// place `Vec<varyk_std::Value>` is [`RustTy::Values`].
pub(super) fn map_param_type(ty: &Type, names: &Names, last: bool) -> RustTy {
    match ty {
        Type::Reference(reference) => map_reference(reference, ty, names),
        _ if last && is_values(ty, names) => RustTy::Values,
        _ => map_value(ty, names),
    }
}

/// Whether `ty` is `Vec<varyk_std::Value>` (an unshadowed `Vec`).
fn is_values(ty: &Type, names: &Names) -> bool {
    let Type::Path(type_path) = ty else {
        return false;
    };
    if type_path.qself.is_some() || type_path.path.leading_colon.is_some() {
        return false;
    }
    let [segment] = type_path.path.segments.iter().collect::<Vec<_>>()[..] else {
        return false;
    };
    let name = ident_name(&segment.ident);
    if name != "Vec" || names.shadowed.contains(&name) {
        return false;
    }
    let PathArguments::AngleBracketed(arguments) = &segment.arguments else {
        return false;
    };
    let [GenericArgument::Type(Type::Path(inner))] = arguments.args.iter().collect::<Vec<_>>()[..]
    else {
        return false;
    };
    inner.qself.is_none() && std_item(&inner.path) == Some(StdItem::Value)
}

/// Maps a return type. A reference in return position is
/// [`RustTy::Opaque`] (spec 4.5) unless `elided` says lifetime elision
/// roots it at a parameter (M4 spec 2.12), and it is `&str` or `&S` for a
/// named type `S`: then [`RustTy::Str`] or [`RustTy::Ref`].
pub(super) fn map_return_type(ty: &Type, names: &Names, elided: bool) -> RustTy {
    if let (true, Type::Reference(reference)) = (elided, ty) {
        if reference.lifetime.is_none() && reference.mutability.is_none() {
            let inner = reference.elem.as_ref();
            if is_named(inner, "str", names) {
                return RustTy::Str;
            }
            if let RustTy::Named(path) = map_value(inner, names) {
                return RustTy::Ref(Box::new(RustTy::Named(path)));
            }
        }
        return RustTy::Opaque(type_to_text(ty));
    }
    map_value(ty, names)
}
