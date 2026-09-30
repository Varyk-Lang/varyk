//! Attributes (M5a spec 2.2, 2.7): which names exist, where each may go,
//! and whether its literal fits. V0112 for a name, a place, a repeat, or
//! a missing or unexpected value; V0209 for a literal that does not fit.
//! Nothing here depends on whether a `json` or `env` call reaches the
//! type: those checks come with the calls.

use std::collections::HashMap;

use varyk_syntax::{AttrArg, Attribute, Span, TypeExpr};

use crate::diagnostics::{Diagnostic, codes};
use crate::types::{FloatKind, Ty};

/// What a struct field's attributes ask for, once checked. They have no
/// effect unless a `json` or `env` call reaches the struct.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FieldAttrs {
    /// `#[rename("key")]`: the key's text between the quotes, as written.
    pub rename: Option<String>,
    /// `#[default(literal)]`, fitting the field's type.
    pub default: Option<HirDefault>,
    /// `#[skip]`.
    pub skip: bool,
}

/// The value of a `#[default]`: an integer or float with its sign
/// applied, or a string's text between the quotes, as written.
#[derive(Debug, Clone, PartialEq)]
pub enum HirDefault {
    Int(i128),
    Float(f64),
    Str(String),
    Bool(bool),
}

/// Where attributes were written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Place {
    Struct,
    Enum,
    Impl,
    Mod,
    Use,
    Function,
    Method,
    Field,
    UnitVariant,
    DataVariant,
    VariantField,
}

impl Place {
    /// "a struct", for "cannot go on a struct".
    fn text(self) -> &'static str {
        match self {
            Place::Struct => "a struct",
            Place::Enum => "an enum",
            Place::Impl => "an `impl` block",
            Place::Mod => "a `mod` line",
            Place::Use => "a `use` line",
            Place::Function => "a function",
            Place::Method => "a method",
            Place::Field => "a struct field",
            Place::UnitVariant => "a variant",
            Place::DataVariant => "a variant that carries data",
            Place::VariantField => "a field of a variant",
        }
    }
}

/// The four attributes: name, where each may go, the note saying so, and
/// whether it takes a value, with an example of one.
const KNOWN: &[(&str, &[Place], &str, Option<&str>)] = &[
    (
        "rename",
        &[Place::Field, Place::UnitVariant],
        "a struct field or a variant that carries no data",
        Some("`#[rename(\"userName\")]`"),
    ),
    (
        "default",
        &[Place::Field],
        "a struct field",
        Some("`#[default(8080)]`"),
    ),
    ("skip", &[Place::Field], "a struct field", None),
    ("test", &[Place::Function], "a top-level function", None),
];

/// V0112 for each attribute in `attrs` with an unknown name, in a place
/// it cannot go, repeated, or with a value missing or unexpected; returns
/// the others, which are one of the four, each once, where it may go.
pub(super) fn placed<'a>(
    attrs: &'a [Attribute],
    place: Place,
    diagnostics: &mut Vec<Diagnostic>,
) -> Vec<&'a Attribute> {
    let mut seen: HashMap<&str, Span> = HashMap::new();
    let mut kept = Vec::new();
    for attr in attrs {
        let name = attr.name.name.as_str();
        let Some(&(_, places, where_, value)) = KNOWN.iter().find(|known| known.0 == name) else {
            let mut diagnostic = Diagnostic::new(
                codes::V0112,
                attr.span,
                format!("there is no attribute named `{name}`"),
            )
            .with_note(
                "the attributes are `#[rename(\"key\")]`, `#[default(value)]`, `#[skip]`, and \
                 `#[test]`",
            );
            if name == "derive" {
                diagnostic = diagnostic.with_note(
                    "`.clone()`, `==`, and JSON work without a derive: Varyk gives a type each \
                     of them when it can",
                );
            }
            diagnostics.push(diagnostic);
            continue;
        };
        if !places.contains(&place) {
            diagnostics.push(
                Diagnostic::new(
                    codes::V0112,
                    attr.span,
                    format!("`#[{name}]` cannot go on {}", place.text()),
                )
                .with_note(format!("`#[{name}]` goes before {where_}")),
            );
            continue;
        }
        if let Some(&first) = seen.get(name) {
            diagnostics.push(
                Diagnostic::new(
                    codes::V0112,
                    attr.span,
                    format!("`#[{name}]` is written twice here"),
                )
                .with_label(first, "first written here"),
            );
            continue;
        }
        seen.insert(name, attr.span);
        match (value, &attr.arg) {
            (Some(example), None) => {
                diagnostics.push(Diagnostic::new(
                    codes::V0112,
                    attr.span,
                    format!("`#[{name}]` needs a value, as in {example}"),
                ));
                continue;
            }
            (None, Some(_)) => {
                diagnostics.push(
                    Diagnostic::new(
                        codes::V0112,
                        attr.span,
                        format!("`#[{name}]` takes no value"),
                    )
                    .with_note(format!("write `#[{name}]`")),
                );
                continue;
            }
            _ => {}
        }
        kept.push(attr);
    }
    kept
}

/// The key of a `#[rename]` that [`placed`] kept: V0209 unless its value
/// is a non-empty string.
pub(super) fn rename(attr: &Attribute, diagnostics: &mut Vec<Diagnostic>) -> Option<String> {
    match &attr.arg {
        Some(AttrArg::Str(text)) if text.is_empty() => {
            diagnostics.push(
                Diagnostic::new(
                    codes::V0209,
                    attr.span,
                    "`#[rename(\"\")]` gives an empty name",
                )
                .with_note("write the name to use between the quotes"),
            );
            None
        }
        Some(AttrArg::Str(text)) => Some(text.clone()),
        _ => {
            diagnostics.push(Diagnostic::new(
                codes::V0209,
                attr.span,
                "`#[rename]` needs a name in quotes, as in `#[rename(\"userName\")]`",
            ));
            None
        }
    }
}

/// The value of a `#[default]` that [`placed`] kept, on a field of type
/// `ty` written as `written`: V0209 unless the field is a number,
/// `string`, or `bool` and the literal fits it.
pub(super) fn default(
    attr: &Attribute,
    ty: &Ty,
    written: &TypeExpr,
    diagnostics: &mut Vec<Diagnostic>,
) -> Option<HirDefault> {
    let span = attr.span;
    match ty {
        Ty::Int(_) | Ty::Float(_) | Ty::String | Ty::Bool => {}
        Ty::Option(_) => {
            diagnostics.push(
                Diagnostic::new(
                    codes::V0209,
                    span,
                    "`#[default]` cannot go on an `Option` field",
                )
                .with_note("a missing value is already `None`"),
            );
            return None;
        }
        _ => {
            diagnostics.push(
                Diagnostic::new(
                    codes::V0209,
                    span,
                    format!(
                        "`#[default]` cannot go on a field of type `{}`",
                        written.display_name()
                    ),
                )
                .with_note("a default goes on a field that is a number, a `string`, or a `bool`"),
            );
            return None;
        }
    }
    let written_value = match &attr.arg {
        Some(AttrArg::Int { text, negative } | AttrArg::Float { text, negative }) => {
            format!("{}{text}", if *negative { "-" } else { "" })
        }
        Some(AttrArg::Str(text)) => format!("\"{text}\""),
        Some(AttrArg::Bool(value)) => value.to_string(),
        Some(AttrArg::Other) | None => {
            diagnostics.push(Diagnostic::new(
                codes::V0209,
                span,
                "`#[default]` needs a number, a string in quotes, `true`, or `false`",
            ));
            return None;
        }
    };
    let value = match (&attr.arg, ty) {
        (Some(AttrArg::Int { text, negative }), Ty::Int(kind)) => {
            let value = int_value(text, *negative).filter(|value| {
                let (min, max) = kind.range();
                // A `-` on an unsigned type never fits, even `-0`, as in a
                // pattern.
                min <= *value && *value <= max && !(*negative && min == 0)
            });
            match value {
                Some(value) => return Some(HirDefault::Int(value)),
                None => {
                    let (min, max) = kind.range();
                    diagnostics.push(Diagnostic::new(
                        codes::V0209,
                        span,
                        format!(
                            "the default `{written_value}` does not fit in `{}` ({min} to {max})",
                            kind.name()
                        ),
                    ));
                    return None;
                }
            }
        }
        (Some(AttrArg::Float { text, negative }), Ty::Float(kind)) => {
            // A literal with a nonzero digit must not round to zero in its
            // type, which would change the value the writer asked for.
            let nonzero = text
                .split(['e', 'E'])
                .next()
                .is_some_and(|digits| digits.chars().any(|c| ('1'..='9').contains(&c)));
            let value = text
                .replace('_', "")
                .parse::<f64>()
                .ok()
                .filter(|value| value.is_finite())
                .filter(|value| *kind == FloatKind::F64 || value.abs() <= f64::from(f32::MAX))
                .filter(|value| {
                    let zero = match kind {
                        FloatKind::F64 => *value == 0.0,
                        FloatKind::F32 => (*value as f32) == 0.0,
                    };
                    !(nonzero && zero)
                });
            match value {
                Some(value) => {
                    return Some(HirDefault::Float(if *negative { -value } else { value }));
                }
                None => {
                    diagnostics.push(Diagnostic::new(
                        codes::V0209,
                        span,
                        format!(
                            "the default `{written_value}` does not fit in `{}`",
                            kind.name()
                        ),
                    ));
                    return None;
                }
            }
        }
        (Some(AttrArg::Str(text)), Ty::String) => Some(HirDefault::Str(text.clone())),
        (Some(AttrArg::Bool(value)), Ty::Bool) => Some(HirDefault::Bool(*value)),
        _ => None,
    };
    if value.is_some() {
        return value;
    }
    let mut diagnostic = Diagnostic::new(
        codes::V0209,
        span,
        format!(
            "the default `{written_value}` does not fit the type `{}`",
            written.display_name()
        ),
    );
    // A whole number is never a float in Varyk, as anywhere else a
    // literal meets a float type.
    if let (Some(AttrArg::Int { .. }), Ty::Float(_)) = (&attr.arg, ty) {
        diagnostic = diagnostic.with_note(format!(
            "write `{written_value}.0` for a number with a fractional part"
        ));
    }
    diagnostics.push(diagnostic);
    None
}

/// The value of the integer `text`, negated when `negative`; `None` when
/// it is beyond any Varyk integer type.
fn int_value(text: &str, negative: bool) -> Option<i128> {
    let value = text
        .replace('_', "")
        .parse::<u128>()
        .ok()
        .filter(|value| *value <= u128::from(u64::MAX))
        .and_then(|value| i128::try_from(value).ok())?;
    Some(if negative { -value } else { value })
}
