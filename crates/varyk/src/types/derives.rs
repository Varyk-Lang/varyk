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

    #[test]
    fn mutually_recursive_structs_through_vec_derive_both() {
        let resolved = resolved_str(
            "struct A {\n    bs: Vec<B>,\n}\nstruct B {\n    a: Option<A>,\n    as_: Vec<A>,\n}\nfn main() {}\n",
        );
        assert_eq!(derives_of(&resolved, "A"), BOTH);
        assert_eq!(derives_of(&resolved, "B"), BOTH);
    }
}
