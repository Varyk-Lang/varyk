//! Mapping an imported Rust signature or field type to a Varyk one, per
//! the M1 spec 4.5 table extended by M3 spec 4.4.

use crate::interop::{ImportedFn, RustPath, RustTy, SelfMode, glob_reason};
use crate::types::{FloatKind, IntKind, ParamMode, Ty};

use super::imports::skipped_note;
use super::{ImportedSig, ModuleId, ResultShape, StructId, Symbols, Unusable, UserType};

/// A type that does not map, with what to change when there is more to
/// say than the type itself.
type Mapped<T> = Result<T, Option<String>>;

/// Maps the types of one `.rs` module, whose bare names are its own items.
pub(super) struct Mapper<'a> {
    symbols: &'a Symbols,
    module: ModuleId,
}

impl<'a> Mapper<'a> {
    pub(super) fn new(symbols: &'a Symbols, module: ModuleId) -> Self {
        Mapper { symbols, module }
    }

    /// Maps an imported signature; `owner` is the struct of a method or
    /// associated function. A signature with any unmapped type is not
    /// callable, keeps no types, and says what to change when it can.
    pub(super) fn sig(&self, owner: Option<StructId>, imported: ImportedFn) -> ImportedSig {
        let names_std = imported.params.iter().chain([&imported.ret]).any(names_std);
        // The importer makes `Values` of the last parameter only.
        let (fixed, variadic) = match imported.params.split_last() {
            Some((RustTy::Values, fixed)) => (fixed, true),
            _ => (imported.params.as_slice(), false),
        };
        let params: Mapped<Vec<_>> = fixed.iter().map(|ty| self.param(ty)).collect();
        // A return with the type parameter has no type of its own: each
        // call builds it from the shape and the type it fills in.
        let hole = imported
            .type_param
            .as_ref()
            .and_then(|_| result_shape(&imported.ret));
        let ret = match hole {
            Some(_) => Ok(Ty::Unit),
            None => self.ret(&imported.ret),
        };
        let (params, ret, callable, note, redefined_in) = match (params, ret) {
            (Ok(params), Ok(ret)) => (params, ret, true, None, None),
            (params, ret) => {
                let special = uncallable_note(&imported);
                // The glob or macro that could redefine a name is the
                // reason, unless the signature is unsupported anyway.
                let redefined = special.is_none()
                    && imported
                        .params
                        .iter()
                        .chain([&imported.ret])
                        .any(redefinable);
                let note = special
                    .or_else(|| params.err().flatten())
                    .or_else(|| ret.err().flatten());
                let file = redefined
                    .then(|| self.symbols.scopes.get(self.module.0 as usize))
                    .flatten()
                    .map(|scope| scope.file.clone());
                (Vec::new(), Ty::Unit, false, note, file)
            }
        };
        let literal = if callable {
            fixed.iter().map(|ty| *ty == RustTy::Literal).collect()
        } else {
            Vec::new()
        };
        ImportedSig {
            name: imported.name,
            module: self.module,
            owner: owner.map(UserType::Struct),
            self_mode: match imported.receiver {
                Some(SelfMode::Shared) => Some(ParamMode::SharedBorrow),
                Some(SelfMode::Mutable) => Some(ParamMode::MutableBorrow),
                Some(SelfMode::None) | None => None,
            },
            params,
            literal,
            variadic: variadic && callable,
            ret,
            result_hole: hole.filter(|_| callable),
            signature: imported.signature,
            ret_root: imported.ret_root.filter(|_| callable),
            callable,
            note,
            within: None,
            redefined_in,
            is_async: imported.is_async,
            names_std,
            private: None,
            package: None,
        }
    }

    /// Maps a field's type: a value type, or why it is unusable.
    pub(super) fn field(&self, ty: &RustTy) -> Result<Ty, Unusable> {
        self.value(ty).map_err(|note| Unusable {
            rust_ty: ty.text(),
            note,
            within: None,
        })
    }

    fn param(&self, ty: &RustTy) -> Mapped<(Ty, ParamMode)> {
        Ok(match ty {
            RustTy::Str | RustTy::Literal => (Ty::String, ParamMode::SharedBorrow),
            RustTy::String => (Ty::String, ParamMode::Owned),
            RustTy::RefMutString => (Ty::String, ParamMode::MutableBorrow),
            RustTy::Ref(inner) => (self.value(inner)?, ParamMode::SharedBorrow),
            RustTy::RefMut(inner) => (self.value(inner)?, ParamMode::MutableBorrow),
            other => (self.value(other)?, ParamMode::Owned),
        })
    }

    fn ret(&self, ty: &RustTy) -> Mapped<Ty> {
        match ty {
            RustTy::Unit => Ok(Ty::Unit),
            // A borrowed return, rooted by the importer (M4 spec 2.12).
            RustTy::Str => Ok(Ty::String),
            RustTy::Ref(inner) => self.value(inner),
            // `varyk_std::Error` as the error of the returned `Result`
            // (milestone 5b3 spec 2.4), and nowhere else.
            RustTy::Result(ok, err) if **err == RustTy::Error => {
                Ok(Ty::Result(Box::new(self.value(ok)?), Box::new(Ty::Error)))
            }
            other => self.value(other),
        }
    }

    /// A type a value can have: a primitive, `String`, an imported
    /// struct, or `Vec`, `Option`, or `Result` of those.
    fn value(&self, ty: &RustTy) -> Mapped<Ty> {
        Ok(match ty {
            RustTy::String => Ty::String,
            RustTy::Named(path) => self.named(path)?,
            RustTy::Vec(inner) => Ty::Vec(Box::new(self.value(inner)?)),
            RustTy::Option(inner) => Ty::Option(Box::new(self.value(inner)?)),
            RustTy::Result(ok, err) => {
                Ty::Result(Box::new(self.value(ok)?), Box::new(self.value(err)?))
            }
            RustTy::Error => {
                return Err(Some(
                    "`varyk_std::Error` can only be the error of a returned `Result`, as in \
                     `Result<i64, varyk_std::Error>`"
                        .to_string(),
                ));
            }
            RustTy::Opaque(text) if names_static_str(text) => {
                return Err(Some(LITERAL_NOTE.to_string()));
            }
            RustTy::Values => return Err(Some(VALUES_NOTE.to_string())),
            RustTy::Opaque(text) if names_value(text) => {
                return Err(Some(VALUES_NOTE.to_string()));
            }
            RustTy::Unit => {
                return Err(Some(
                    "`()` inside another type is not supported yet; use `bool` or a struct \
                     instead, as in `Result<bool, String>`"
                        .to_string(),
                ));
            }
            other => map_primitive(other).ok_or(None)?,
        })
    }

    /// An imported struct of this module (a bare name) or of another
    /// `.rs` module (a `crate::` path).
    fn named(&self, path: &RustPath) -> Mapped<Ty> {
        let (module, name) = match path {
            RustPath::Local(name) => (self.module, name.as_str()),
            RustPath::Used(name) => {
                return Err(Some(format!(
                    "`{name}` is brought in by a `use` line in the Rust file, which Varyk does \
                     not follow; write the full path, as in `crate::module::{name}`"
                )));
            }
            RustPath::Glob(name) => return Err(Some(glob_reason(name))),
            RustPath::Macro(_) => {
                return Err(self.symbols.scopes[self.module.0 as usize].expander.clone());
            }
            RustPath::Crate(segments) => {
                let (name, modules) = segments.split_last().expect("a crate path names an item");
                let mut module = ModuleId(0);
                for segment in modules {
                    let scope = &self.symbols.scopes[module.0 as usize];
                    module = *scope.children.get(segment).ok_or(None)?;
                }
                (module, name.as_str())
            }
        };
        let scope = &self.symbols.scopes[module.0 as usize];
        if let Some((kind, why)) = scope.skipped.get(name) {
            return Err(Some(skipped_note(kind, name, why)));
        }
        match scope.types.get(name) {
            Some(UserType::Struct(id)) if self.symbols.structs[id.0 as usize].imported => {
                Ok(Ty::Struct(*id))
            }
            Some(UserType::Enum(id)) if self.symbols.enums[id.0 as usize].imported => {
                Ok(Ty::Enum(*id))
            }
            Some(_) => Err(Some(format!(
                "`{name}` is declared in Varyk (module `{}`); a Rust signature or field cannot \
                 name a Varyk type yet",
                scope.name
            ))),
            // A bare name of this file that is no imported struct or enum:
            // a `type` alias, a trait, a union, or a private type.
            None if matches!(path, RustPath::Local(_)) => Err(Some(if is_mapped_name(name) {
                format!(
                    "`{name}` here is the Rust file's own `{name}` (a `type` alias or another \
                     item it declares), not the standard one; rename it in the Rust file"
                )
            } else {
                format!(
                    "`{name}` is declared in the Rust file, but not as a `pub` struct or enum \
                     Varyk can use"
                )
            })),
            None => Err(None),
        }
    }
}

/// Whether `name` is one a Rust signature spells for a type Varyk maps:
/// a primitive, `String`, `str`, `Vec`, `Option`, or `Result`.
fn is_mapped_name(name: &str) -> bool {
    matches!(
        name,
        "bool"
            | "i8"
            | "i16"
            | "i32"
            | "i64"
            | "u8"
            | "u16"
            | "u32"
            | "u64"
            | "usize"
            | "f32"
            | "f64"
            | "String"
            | "str"
            | "Vec"
            | "Option"
            | "Result"
    )
}

/// What to change in a signature that is not callable for a reason of its
/// own rather than one of its types (M3 spec 4.2).
fn uncallable_note(imported: &ImportedFn) -> Option<String> {
    if let Some(RustTy::Opaque(receiver)) = imported.params.first() {
        if imported.owner.is_some() && receiver.contains("self") {
            return Some(
                "Varyk calls a method that takes `&self` or `&mut self`; a method that takes \
                 `self` any other way is not supported"
                    .to_string(),
            );
        }
    }
    if let Some(why) = imported.type_param_refused {
        return Some(format!(
            "Varyk calls a generic function only with one type parameter `T: \
             serde::de::DeserializeOwned` (or `varyk_std::serde::de::DeserializeOwned`), written \
             inline by that full path, and `T` only in the return, as in `Result<T, \
             varyk_std::Error>`, `Result<Option<T>, varyk_std::Error>`, or `Result<Vec<T>, \
             varyk_std::Error>`; this one has {why}"
        ));
    }
    match &imported.ret {
        RustTy::Opaque(text) if imported.is_async && text.starts_with('&') => Some(
            "Varyk does not import an async Rust function that returns a reference (its result \
             outlives the call); return an owned value, such as `String` instead of `&str`"
                .to_string(),
        ),
        RustTy::Opaque(text) if names_static_str(text) => Some(LITERAL_NOTE.to_string()),
        RustTy::Opaque(text) if text.starts_with('&') => Some(
            "Varyk imports a returned `&str` or `&S` only from a `&self` method, or from a `pub \
             fn` with exactly one `&T` parameter, when the signature writes no lifetime; \
             otherwise return an owned value, such as `String` instead of `&str`"
                .to_string(),
        ),
        _ => None,
    }
}

/// The shape of a return the importer made with the type parameter
/// (milestone 5b3 spec 2.1).
fn result_shape(ret: &RustTy) -> Option<ResultShape> {
    let RustTy::Result(ok, _) = ret else {
        return None;
    };
    match ok.as_ref() {
        RustTy::Param => Some(ResultShape::Plain),
        RustTy::Option(inner) if **inner == RustTy::Param => Some(ResultShape::Option),
        RustTy::Vec(inner) if **inner == RustTy::Param => Some(ResultShape::Vec),
        _ => None,
    }
}

/// The note for a `&'static str` anywhere but a parameter (milestone
/// 5b3 spec 2.3).
const LITERAL_NOTE: &str = "`&'static str` can only be a parameter, which then takes only text \
                            written in the program; return or hold an owned `String` instead";

/// Whether the type text `text` (as [`RustTy::Opaque`] holds it) has a
/// `&'static str` in it.
fn names_static_str(text: &str) -> bool {
    let tokens: Vec<&str> = text.split_whitespace().collect();
    tokens.windows(3).any(|w| w == ["&", "'static", "str"])
}

/// The note for `varyk_std::Value` anywhere but in a last
/// `Vec<varyk_std::Value>` parameter (milestone 5b3 spec 2.2).
const VALUES_NOTE: &str = "`Vec<varyk_std::Value>` can only be the last parameter, which then \
                           takes any number of values after the other arguments; \
                           `varyk_std::Value` cannot be used anywhere else";

/// Whether the type text `text` (as [`RustTy::Opaque`] holds it) has a
/// `varyk_std::Value` in it.
fn names_value(text: &str) -> bool {
    let tokens: Vec<&str> = text.split_whitespace().collect();
    tokens.windows(3).any(|w| w == ["varyk_std", "::", "Value"])
}

/// Whether `ty` names, at any depth, an item of `varyk-std`.
fn names_std(ty: &RustTy) -> bool {
    match ty {
        // A call writes `::varyk_std::Value::from` for its values, and
        // the serde derive of a filled type parameter (milestone 5b3 spec
        // 2.4), whichever path its bound is written by.
        RustTy::Error | RustTy::Values | RustTy::Param => true,
        RustTy::Ref(inner) | RustTy::RefMut(inner) | RustTy::Vec(inner) | RustTy::Option(inner) => {
            names_std(inner)
        }
        RustTy::Result(ok, err) => names_std(ok) || names_std(err),
        // Listed in full, so a new variant decides whether it counts.
        RustTy::Bool
        | RustTy::I8
        | RustTy::I16
        | RustTy::I32
        | RustTy::I64
        | RustTy::U8
        | RustTy::U16
        | RustTy::U32
        | RustTy::U64
        | RustTy::Usize
        | RustTy::F32
        | RustTy::F64
        | RustTy::Str
        | RustTy::String
        | RustTy::RefMutString
        | RustTy::Literal
        | RustTy::Unit
        | RustTy::Named(_) => false,
        // A type Varyk cannot use, which makes the signature uncallable,
        // still needs `varyk-std` to compile when it names it (a
        // `varyk_std::Value` alone, a generic signature's text).
        RustTy::Opaque(text) => text.split_whitespace().any(|token| token == "varyk_std"),
    }
}

/// Whether `ty` names, at any depth, a type the file's glob `use` or
/// item macro could redefine.
fn redefinable(ty: &RustTy) -> bool {
    match ty {
        RustTy::Named(RustPath::Glob(_) | RustPath::Macro(_)) => true,
        RustTy::Ref(inner) | RustTy::RefMut(inner) | RustTy::Vec(inner) | RustTy::Option(inner) => {
            redefinable(inner)
        }
        RustTy::Result(ok, err) => redefinable(ok) || redefinable(err),
        _ => false,
    }
}

fn map_primitive(ty: &RustTy) -> Option<Ty> {
    Some(match ty {
        RustTy::Bool => Ty::Bool,
        RustTy::I8 => Ty::Int(IntKind::I8),
        RustTy::I16 => Ty::Int(IntKind::I16),
        RustTy::I32 => Ty::Int(IntKind::I32),
        RustTy::I64 => Ty::Int(IntKind::I64),
        RustTy::U8 => Ty::Int(IntKind::U8),
        RustTy::U16 => Ty::Int(IntKind::U16),
        RustTy::U32 => Ty::Int(IntKind::U32),
        RustTy::U64 => Ty::Int(IntKind::U64),
        RustTy::Usize => Ty::Int(IntKind::Usize),
        RustTy::F32 => Ty::Float(FloatKind::F32),
        RustTy::F64 => Ty::Float(FloatKind::F64),
        _ => return None,
    })
}
