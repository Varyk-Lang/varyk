//! The `Diagnostic` type every later pass emits, plus its renderers.
//!
//! There is no severity type: every `Diagnostic` is an error (spec section
//! 6.6). A diagnostic carries a code, a message, a primary span, secondary
//! labels, notes, and an optional fix-it.
//!
//! **Primary label convention.** `Diagnostic` has no separate "primary
//! label" field. The diagnostic's own `message` doubles as the label shown
//! under the primary span's caret when rendered; `labels` holds only
//! secondary/context spans with their own text. This keeps the type small
//! and matches how most Varyk diagnostics read: the header message is
//! exactly what should appear at the point of the error too.

pub mod codes;
mod json;
mod render;

pub use json::render_json;
pub use render::render_human;

use varyk_syntax::{FixIt, SourceFile, Span, SyntaxError};

/// Looks up the source file for `file_id`, indexed by `FileId.0`:
/// `sources[span.file.0 as usize]` is always the file a span points into,
/// because the file-resolution pass (`resolve`) pushes files in id order.
/// Panics with a message naming the id if it is out of range, since an
/// out-of-range id means a caller passed a `sources` slice that doesn't
/// match the diagnostic's files.
pub(crate) fn source_file(sources: &[SourceFile], file_id: u32) -> &SourceFile {
    sources.get(file_id as usize).unwrap_or_else(|| {
        panic!(
            "diagnostic references FileId({file_id}), but only {} source file(s) were given",
            sources.len()
        )
    })
}

/// A secondary span with its own explanatory text, attached to a
/// [`Diagnostic`] in addition to its primary span.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Label {
    pub span: Span,
    pub text: String,
}

/// A single compiler diagnostic: a code, a message, a primary span,
/// secondary labels, notes, and an optional fix-it. See the module docs
/// for the primary-label convention.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub code: &'static str,
    pub message: String,
    pub span: Span,
    pub labels: Vec<Label>,
    pub notes: Vec<String>,
    pub fix_it: Option<FixIt>,
}

impl Diagnostic {
    /// Creates a diagnostic with no labels, notes, or fix-it yet.
    pub fn new(code: &'static str, span: Span, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            span,
            labels: Vec::new(),
            notes: Vec::new(),
            fix_it: None,
        }
    }

    /// Adds a secondary label at `span`.
    pub fn with_label(mut self, span: Span, text: impl Into<String>) -> Self {
        self.labels.push(Label {
            span,
            text: text.into(),
        });
        self
    }

    /// Adds a `= note: ...` line.
    pub fn with_note(mut self, note: impl Into<String>) -> Self {
        self.notes.push(note.into());
        self
    }

    /// Attaches a suggested edit.
    pub fn with_fix_it(mut self, fix_it: FixIt) -> Self {
        self.fix_it = Some(fix_it);
        self
    }
}

/// Converts a lexer/parser error into a diagnostic, preserving its span,
/// code, message, and fix-it.
impl From<SyntaxError> for Diagnostic {
    fn from(error: SyntaxError) -> Self {
        let mut diagnostic = Diagnostic::new(error.code, error.span, error.message);
        diagnostic.fix_it = error.fix_it;
        diagnostic
    }
}
