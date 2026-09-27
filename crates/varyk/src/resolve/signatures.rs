//! Mapping an imported Rust signature or field type to a Varyk one, per
//! the M1 spec 4.5 table extended by M3 spec 4.4.

use crate::interop::{ImportedFn, RustPath, RustTy, SelfMode, glob_reason};
use crate::types::{FloatKind, IntKind, ParamMode, Ty};

use super::imports::skipped_note;
use super::{ImportedSig, ModuleId, StructId, Symbols, Unusable, UserType};

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
        let params: Mapped<Vec<_>> = imported.params.iter().map(|ty| self.param(ty)).collect();
        let ret = self.ret(&imported.ret);
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
        ImportedSig {
            name: imported.name,
            module: self.module,
            owner,
            self_mode: match imported.receiver {
                Some(SelfMode::Shared) => Some(ParamMode::SharedBorrow),
                Some(SelfMode::Mutable) => Some(ParamMode::MutableBorrow),
                Some(SelfMode::None) | None => None,
            },
            params,
            ret,
            signature: imported.signature,
            callable,
            note,
            within: None,
            redefined_in,
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
            RustTy::Str => (Ty::String, ParamMode::SharedBorrow),
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
    match &imported.ret {
        RustTy::Opaque(text) if *text == imported.signature => Some(
            "Varyk cannot call a generic function; add a `pub fn` without type parameters to \
             the Rust module that calls it"
                .to_string(),
        ),
        RustTy::Opaque(text) if text.starts_with('&') => Some(
            "a function that returns a borrow is not supported yet; return an owned value, such \
             as `String` instead of `&str`"
                .to_string(),
        ),
        _ => None,
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
