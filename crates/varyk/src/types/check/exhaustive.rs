//! Exhaustiveness and reachability of a `match` (M4 spec 2.5, 5): one
//! usefulness check over a constructor set per type.
//!
//! The constructors of an enum, an `Option`, or a `Result` are its
//! variants, and those of a `bool` are `true` and `false`. Every number
//! type and `string` has the literals and ranges written for it, split at
//! every written end so they are disjoint, plus one "anything else"
//! constructor no pattern but a catch-all covers: Varyk treats them as
//! having infinitely many values, so a `match` on one always needs a
//! catch-all, which is never unreachable. Every other type (a float, a
//! struct, a `Vec`, an imported enum Varyk cannot look inside) has only
//! "anything else". This is stricter than rustc (`0..=255` on a `u8`
//! still needs `_`) and never looser.
//!
//! A pattern is useful after some rows when some value fits it and none
//! of them (Maranget's algorithm): the `match` is exhaustive when `_` is
//! not useful after every arm (V0204 names a value shape, a witness, that
//! no arm fits), and each arm must be useful after those above it
//! (V0205).

use varyk_syntax::Span;

use crate::diagnostics::{Diagnostic, codes};
use crate::hir::{HirLiteral, HirPattern, VariantRef};
use crate::resolve::{Symbols, VariantFieldsDef};
use crate::types::Ty;

/// A constructor of a value.
#[derive(Debug, Clone, PartialEq)]
enum Ctor {
    /// A variant by its index: for `Option`, `Some` is 0 and `None` 1;
    /// for `Result`, `Ok` is 0 and `Err` 1.
    Variant(usize),
    Bool(bool),
    /// The integers `lo..=hi`.
    Int(i128, i128),
    Str(String),
}

/// A pattern as the check sees it: a catch-all, or a constructor with a
/// pattern for each value it holds, in declaration order.
#[derive(Debug, Clone)]
enum Pat {
    Wild,
    Ctor(Ctor, Vec<Pat>),
}

/// Reports, for a `match` at `at` looking at a `ty` with the arms
/// `arms` (each pattern with its arm's span): V0204 when some value fits
/// no arm, naming it, and V0205 for each arm no value can reach.
pub(super) fn check(
    symbols: &Symbols,
    ty: &Ty,
    arms: &[(&HirPattern, Span)],
    at: Span,
) -> Vec<Diagnostic> {
    let cx = Cx { symbols };
    let rows: Vec<Vec<Pat>> = arms
        .iter()
        .map(|(pattern, _)| vec![lower(pattern)])
        .collect();
    let tys = [ty.clone()];
    let mut diagnostics = Vec::new();

    // A catch-all arm above an unreachable one gets the blame, once.
    let mut blamed_catch_all = false;
    for (index, (_, span)) in arms.iter().enumerate() {
        if !cx.useful(&rows[..index], &rows[index], &tys).is_empty() {
            continue;
        }
        let catch_all = arms[..index]
            .iter()
            .find(|(pattern, _)| matches!(pattern, HirPattern::Wildcard | HirPattern::Binding(_)));
        match catch_all {
            Some((_, first)) if !blamed_catch_all => {
                blamed_catch_all = true;
                diagnostics.push(
                    Diagnostic::new(
                        codes::V0205,
                        *first,
                        "this arm matches every value, so the arms after it could never run; \
                         make it the last arm",
                    )
                    .with_note("in Rust terms, the later patterns are unreachable"),
                );
            }
            Some(_) => {}
            None => diagnostics.push(
                Diagnostic::new(
                    codes::V0205,
                    *span,
                    "this arm can never run, because the arms above it already handle every \
                     value it fits; remove it",
                )
                .with_note("in Rust terms, the pattern is unreachable"),
            ),
        }
    }
    if !diagnostics.is_empty() {
        return diagnostics;
    }

    // With no arms, even a type without values needs one: Rust matches a
    // place through a reference, which it never takes to be empty.
    let witnesses = if arms.is_empty() {
        vec![vec![Pat::Wild]]
    } else {
        cx.useful(&rows, &[Pat::Wild], &tys)
    };
    if witnesses.is_empty() {
        return diagnostics;
    }
    let shapes: Vec<String> = witnesses
        .iter()
        .filter_map(|witness| witness.first())
        .map(|pattern| cx.show(pattern, ty))
        .collect();
    let diagnostic = if shapes.iter().all(|shape| shape == "_") {
        let what = match ty {
            Ty::Int(_) | Ty::Float(_) => "number",
            Ty::String => "string",
            _ => "value",
        };
        Diagnostic::new(
            codes::V0204,
            at,
            format!(
                "this `match` does not handle every {what}; end it with a catch-all arm, `_ => \
                 ...`"
            ),
        )
        .with_note(
            "Varyk counts every number and every string as having more values than arms can \
             list, so a `match` on one always ends with a catch-all; in Rust terms, the \
             patterns are not exhaustive",
        )
    } else {
        let shapes: Vec<&str> = shapes.iter().map(String::as_str).collect();
        Diagnostic::new(
            codes::V0204,
            at,
            format!("this `match` does not handle {}", either(&shapes)),
        )
        .with_note(
            "add an arm for each missing variant, or end with `_ => ...` to handle every \
             other value; in Rust terms, the patterns are not exhaustive",
        )
    };
    diagnostics.push(diagnostic);
    diagnostics
}

/// `pattern` as the check sees it.
fn lower(pattern: &HirPattern) -> Pat {
    match pattern {
        HirPattern::Wildcard | HirPattern::Binding(_) => Pat::Wild,
        HirPattern::Variant { variant, fields } => Pat::Ctor(
            Ctor::Variant(variant_index(*variant)),
            fields.iter().map(lower).collect(),
        ),
        HirPattern::Struct { variant, fields } => Pat::Ctor(
            Ctor::Variant(variant_index(*variant)),
            fields.iter().map(|(_, field)| lower(field)).collect(),
        ),
        HirPattern::Literal(HirLiteral::Int(value)) => Pat::Ctor(Ctor::Int(*value, *value), vec![]),
        HirPattern::Literal(HirLiteral::Bool(value)) => Pat::Ctor(Ctor::Bool(*value), vec![]),
        HirPattern::Literal(HirLiteral::Str(text)) => Pat::Ctor(Ctor::Str(text.clone()), vec![]),
        HirPattern::Range { lo, hi } => Pat::Ctor(Ctor::Int(*lo, *hi), vec![]),
    }
}

fn variant_index(variant: VariantRef) -> usize {
    match variant {
        VariantRef::User(_, index) => index,
        VariantRef::Some | VariantRef::Ok => 0,
        VariantRef::None | VariantRef::Err => 1,
    }
}

/// Whether `row`, a row's constructor, covers `ctor`, a constructor
/// from the same column split against every row's (so an integer range
/// is either inside a row's range or apart from it).
fn covers(row: &Ctor, ctor: &Ctor) -> bool {
    match (row, ctor) {
        (Ctor::Int(lo, hi), Ctor::Int(start, end)) => lo <= start && end <= hi,
        _ => row == ctor,
    }
}

fn wilds(count: usize) -> Vec<Pat> {
    vec![Pat::Wild; count]
}

/// The rows whose first pattern fits `ctor`, which holds `arity` values:
/// that pattern replaced by its own patterns, or by `arity` catch-alls.
fn specialize(rows: &[Vec<Pat>], ctor: &Ctor, arity: usize) -> Vec<Vec<Pat>> {
    rows.iter()
        .filter_map(|row| {
            let (first, rest) = row.split_first()?;
            let mut out = match first {
                Pat::Wild => wilds(arity),
                Pat::Ctor(own, fields) if covers(own, ctor) => fields.clone(),
                Pat::Ctor(..) => return None,
            };
            out.extend_from_slice(rest);
            Some(out)
        })
        .collect()
}

/// The rows whose first pattern is a catch-all, without it.
fn default_rows(rows: &[Vec<Pat>]) -> Vec<Vec<Pat>> {
    rows.iter()
        .filter_map(|row| match row.split_first()? {
            (Pat::Wild, rest) => Some(rest.to_vec()),
            (Pat::Ctor(..), _) => None,
        })
        .collect()
}

/// `witness`, whose first `arity` patterns are the values `ctor` holds,
/// with those put back inside `ctor`.
fn rebuild(ctor: &Ctor, arity: usize, witness: Vec<Pat>) -> Vec<Pat> {
    let mut witness = witness;
    let rest = witness.split_off(arity.min(witness.len()));
    let mut out = vec![Pat::Ctor(ctor.clone(), witness)];
    out.extend(rest);
    out
}

struct Cx<'a> {
    symbols: &'a Symbols,
}

impl Cx<'_> {
    /// Every constructor of `ty`, when it has finitely many; `None` for a
    /// type with an "anything else".
    fn ctors(&self, ty: &Ty) -> Option<Vec<Ctor>> {
        match ty {
            Ty::Bool => Some(vec![Ctor::Bool(true), Ctor::Bool(false)]),
            Ty::Option(_) | Ty::Result(..) => Some(vec![Ctor::Variant(0), Ctor::Variant(1)]),
            Ty::Enum(id) => {
                let def = &self.symbols.enums[id.0 as usize];
                if def.opaque.is_some() {
                    return None;
                }
                Some((0..def.variants.len()).map(Ctor::Variant).collect())
            }
            _ => None,
        }
    }

    /// The types of the values `ctor` of `ty` holds, in declaration order.
    fn fields(&self, ty: &Ty, ctor: &Ctor) -> Vec<Ty> {
        match (ty, ctor) {
            (Ty::Option(inner), Ctor::Variant(0)) => vec![(**inner).clone()],
            (Ty::Result(ok, _), Ctor::Variant(0)) => vec![(**ok).clone()],
            (Ty::Result(_, err), Ctor::Variant(1)) => vec![(**err).clone()],
            (Ty::Enum(id), Ctor::Variant(index)) => self.symbols.enums[id.0 as usize]
                .variants
                .get(*index)
                .map(|variant| variant.types().into_iter().cloned().collect())
                .unwrap_or_default(),
            _ => Vec::new(),
        }
    }

    /// The witnesses that `v` (patterns for values of `tys`) is useful
    /// after `rows`: value shapes fitting `v` and no row; empty when it is
    /// not useful.
    fn useful(&self, rows: &[Vec<Pat>], v: &[Pat], tys: &[Ty]) -> Vec<Vec<Pat>> {
        let (Some((first, rest)), Some((ty, rest_tys))) = (v.split_first(), tys.split_first())
        else {
            return if rows.is_empty() {
                vec![Vec::new()]
            } else {
                Vec::new()
            };
        };
        match first {
            Pat::Ctor(ctor, fields) => {
                for piece in split(ctor, rows) {
                    let arity = fields.len();
                    let specialized = specialize(rows, &piece, arity);
                    let mut next = fields.clone();
                    next.extend_from_slice(rest);
                    let mut next_tys = self.fields(ty, &piece);
                    next_tys.extend_from_slice(rest_tys);
                    let found = self.useful(&specialized, &next, &next_tys);
                    if !found.is_empty() {
                        return found
                            .into_iter()
                            .map(|witness| rebuild(&piece, arity, witness))
                            .collect();
                    }
                }
                Vec::new()
            }
            Pat::Wild => {
                let heads: Vec<&Ctor> = rows
                    .iter()
                    .filter_map(|row| match row.first() {
                        Some(Pat::Ctor(ctor, _)) => Some(ctor),
                        _ => None,
                    })
                    .collect();
                let all = self.ctors(ty);
                let complete = all
                    .as_ref()
                    .is_some_and(|all| all.iter().all(|ctor| heads.contains(&ctor)));
                if let (Some(all), true) = (all.as_ref(), complete) {
                    for ctor in all {
                        let field_tys = self.fields(ty, ctor);
                        let arity = field_tys.len();
                        let specialized = specialize(rows, ctor, arity);
                        let mut next = wilds(arity);
                        next.extend_from_slice(rest);
                        let mut next_tys = field_tys;
                        next_tys.extend_from_slice(rest_tys);
                        let found = self.useful(&specialized, &next, &next_tys);
                        if !found.is_empty() {
                            return found
                                .into_iter()
                                .map(|witness| rebuild(ctor, arity, witness))
                                .collect();
                        }
                    }
                    return Vec::new();
                }
                let found = self.useful(&default_rows(rows), rest, rest_tys);
                if found.is_empty() {
                    return found;
                }
                // The constructors no row names, each with catch-alls
                // inside; a catch-all when there is an "anything else" or
                // no row names any.
                let missing: Vec<Pat> = match all {
                    Some(all) if !heads.is_empty() => all
                        .iter()
                        .filter(|ctor| !heads.contains(ctor))
                        .map(|ctor| Pat::Ctor(ctor.clone(), wilds(self.fields(ty, ctor).len())))
                        .collect(),
                    _ => vec![Pat::Wild],
                };
                let mut out = Vec::new();
                for witness in found {
                    for head in &missing {
                        let mut row = vec![head.clone()];
                        row.extend(witness.iter().cloned());
                        out.push(row);
                    }
                }
                out
            }
        }
    }

    /// A witness pattern as Varyk writes it, looking at a `ty`.
    fn show(&self, pattern: &Pat, ty: &Ty) -> String {
        let Pat::Ctor(ctor, fields) = pattern else {
            return "_".to_string();
        };
        let inner = |index: usize, field_ty: &Ty| {
            fields
                .get(index)
                .map_or_else(|| "_".to_string(), |field| self.show(field, field_ty))
        };
        match (ctor, ty) {
            (Ctor::Bool(value), _) => value.to_string(),
            (Ctor::Int(lo, hi), _) if lo == hi => lo.to_string(),
            (Ctor::Int(lo, hi), _) => format!("{lo}..={hi}"),
            (Ctor::Str(text), _) => format!("\"{text}\""),
            (Ctor::Variant(0), Ty::Option(value)) => format!("Some({})", inner(0, value)),
            (Ctor::Variant(_), Ty::Option(_)) => "None".to_string(),
            (Ctor::Variant(0), Ty::Result(ok, _)) => format!("Ok({})", inner(0, ok)),
            (Ctor::Variant(_), Ty::Result(_, err)) => format!("Err({})", inner(0, err)),
            (Ctor::Variant(index), Ty::Enum(id)) => {
                let def = &self.symbols.enums[id.0 as usize];
                let Some(variant) = def.variants.get(*index) else {
                    return "_".to_string();
                };
                let name = format!("{}::{}", def.name, variant.name);
                match &variant.fields {
                    VariantFieldsDef::Tuple(types) if types.is_empty() => name,
                    VariantFieldsDef::Tuple(types) => {
                        let shown: Vec<String> = types
                            .iter()
                            .enumerate()
                            .map(|(index, ty)| inner(index, ty))
                            .collect();
                        format!("{name}({})", shown.join(", "))
                    }
                    VariantFieldsDef::Named(named) => {
                        let shown: Vec<String> = named
                            .iter()
                            .enumerate()
                            .map(|(index, (field, ty))| format!("{field}: {}", inner(index, ty)))
                            .collect();
                        format!("{name} {{ {} }}", shown.join(", "))
                    }
                }
            }
            (Ctor::Variant(_), _) => "_".to_string(),
        }
    }
}

/// `ctor`, split against the first constructors of `rows`: an integer
/// range cut at every end a row's range has inside it, so that each piece
/// is inside or apart from each row's; any other constructor as it is.
fn split(ctor: &Ctor, rows: &[Vec<Pat>]) -> Vec<Ctor> {
    let Ctor::Int(lo, hi) = ctor else {
        return vec![ctor.clone()];
    };
    let (lo, hi) = (*lo, *hi);
    // Each piece starts at a cut; the last ends at `hi`.
    let mut cuts = vec![lo];
    for row in rows {
        if let Some(Pat::Ctor(Ctor::Int(start, end), _)) = row.first() {
            for cut in [*start, end + 1] {
                if lo < cut && cut <= hi {
                    cuts.push(cut);
                }
            }
        }
    }
    cuts.sort_unstable();
    cuts.dedup();
    cuts.iter()
        .enumerate()
        .map(|(index, &start)| {
            let end = cuts.get(index + 1).map_or(hi, |next| next - 1);
            Ctor::Int(start, end)
        })
        .collect()
}

/// `names` as alternatives in words: "`a`", "`a` or `b`", or "`a`, `b`,
/// or `c`".
fn either(names: &[&str]) -> String {
    let quoted: Vec<String> = names.iter().map(|name| format!("`{name}`")).collect();
    match quoted.as_slice() {
        [] => String::new(),
        [one] => one.clone(),
        [first, second] => format!("{first} or {second}"),
        [init @ .., last] => format!("{}, or {last}", init.join(", ")),
    }
}
