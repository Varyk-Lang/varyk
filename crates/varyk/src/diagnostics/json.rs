//! One-JSON-object-per-line diagnostic output for `--message-format=json`
//! (spec section 6.6). This is the interface editors and AI agents consume.

use serde_json::{Value, json};
use varyk_syntax::SourceFile;

use super::{Diagnostic, source_file};

/// Renders each diagnostic as one JSON object per line.
///
/// `sources` is indexed by `FileId.0`: `sources[span.file.0 as usize]` is
/// always the file a span points into, because the file-resolution pass
/// (`resolve`) pushes files in id order.
pub fn render_json(diagnostics: &[Diagnostic], sources: &[SourceFile]) -> String {
    diagnostics
        .iter()
        .map(|diagnostic| diagnostic_to_value(diagnostic, sources).to_string())
        .collect::<Vec<_>>()
        .join("\n")
}

fn position_fields(file: &SourceFile, span: varyk_syntax::Span) -> (u32, u32, u32, u32) {
    let (line, column) = file.line_col(span.start);
    let (end_line, end_column) = file.line_col(span.end);
    (line, column, end_line, end_column)
}

fn diagnostic_to_value(diagnostic: &Diagnostic, sources: &[SourceFile]) -> Value {
    let file = source_file(sources, diagnostic.span.file.0);
    let (line, column, end_line, end_column) = position_fields(file, diagnostic.span);

    let labels: Vec<Value> = diagnostic
        .labels
        .iter()
        .map(|label| {
            let label_file = source_file(sources, label.span.file.0);
            let (l_line, l_column, l_end_line, l_end_column) =
                position_fields(label_file, label.span);
            json!({
                "file": label_file.path.to_string_lossy().into_owned(),
                "line": l_line,
                "column": l_column,
                "end_line": l_end_line,
                "end_column": l_end_column,
                "text": label.text,
            })
        })
        .collect();

    let fix_it = diagnostic.fix_it.as_ref().map(|fix_it| {
        let fix_it_file = source_file(sources, fix_it.span.file.0);
        let (f_line, f_column, f_end_line, f_end_column) =
            position_fields(fix_it_file, fix_it.span);
        json!({
            "file": fix_it_file.path.to_string_lossy().into_owned(),
            "line": f_line,
            "column": f_column,
            "end_line": f_end_line,
            "end_column": f_end_column,
            "replacement": fix_it.replacement,
        })
    });

    json!({
        "code": diagnostic.code,
        "severity": "error",
        "message": diagnostic.message,
        "file": file.path.to_string_lossy().into_owned(),
        "line": line,
        "column": column,
        "end_line": end_line,
        "end_column": end_column,
        "labels": labels,
        "notes": diagnostic.notes,
        "fix_it": fix_it,
    })
}
