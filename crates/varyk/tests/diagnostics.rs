//! Tests for the `Diagnostic` type and the JSON writer.

use varyk::diagnostics::{Diagnostic, codes, render_json};
use varyk_syntax::{FileId, FixIt, SourceFile, Span, SyntaxError};

/// Builds the source and the V0301 diagnostic the JSON test reads: a
/// primary label, a secondary label on a different line, a note, and an
/// insertion fix-it, exactly as the borrow check emits them.
///
/// Source (one-based lines):
/// 1: `fn f(mut p: Point) {`
/// 2: `    let x = p;`
/// 3: `    let mut y = x;`
/// 4: `    y.n = 2;`
/// 5: `}`
fn fixture() -> (SourceFile, Diagnostic) {
    let text = "fn f(mut p: Point) {\n    let x = p;\n    let mut y = x;\n    y.n = 2;\n}\n";
    let file = SourceFile::new(FileId(0), "src/main.vr", text);

    // The `x` declared on line 2 (the label and the fix-it target).
    let decl_start = text.find("let x").unwrap() as u32 + 4;
    let decl_span = Span::new(FileId(0), decl_start, decl_start + 1);

    // The assignment target `y.n` on line 4 (the primary span).
    let assign_start = text.find("y.n").unwrap() as u32;
    let assign_span = Span::new(FileId(0), assign_start, assign_start + 3);

    let diagnostic = Diagnostic::new(
        codes::V0301,
        assign_span,
        "`x` cannot be changed because it was declared without `mut`".to_string(),
    )
    .with_label(decl_span, "`x` is declared here without `mut`".to_string())
    .with_note("`y` comes from variable `x`".to_string())
    .with_fix_it(FixIt {
        span: Span::new(FileId(0), decl_start, decl_start),
        replacement: "mut ".to_string(),
    });

    (file, diagnostic)
}

#[test]
fn json_line_round_trips_every_field_with_one_based_positions() {
    let (file, diagnostic) = fixture();
    let rendered = render_json(&[diagnostic], &[file]);

    // Exactly one line, one JSON object.
    let mut lines = rendered.lines();
    let line = lines.next().expect("one JSON line");
    assert!(lines.next().is_none(), "expected exactly one JSON line");

    let value: serde_json::Value = serde_json::from_str(line).expect("valid JSON");

    assert_eq!(value["code"], codes::V0301);
    assert_eq!(value["severity"], "error");
    assert_eq!(
        value["message"],
        "`x` cannot be changed because it was declared without `mut`"
    );
    assert_eq!(value["file"], "src/main.vr");
    // "    y.n = 2;" -> `y.n` is on line 4, columns 5 to 8.
    assert_eq!(value["line"], 4);
    assert_eq!(value["column"], 5);
    assert_eq!(value["end_line"], 4);
    assert_eq!(value["end_column"], 8);

    let labels = value["labels"].as_array().expect("labels array");
    assert_eq!(labels.len(), 1);
    // "    let x = p;" -> `x` is on line 2, column 9.
    assert_eq!(labels[0]["file"], "src/main.vr");
    assert_eq!(labels[0]["line"], 2);
    assert_eq!(labels[0]["column"], 9);
    assert_eq!(labels[0]["end_line"], 2);
    assert_eq!(labels[0]["end_column"], 10);
    assert_eq!(labels[0]["text"], "`x` is declared here without `mut`");

    let notes = value["notes"].as_array().expect("notes array");
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0], "`y` comes from variable `x`");

    // An insertion: an empty span just before `x`.
    let fix_it = &value["fix_it"];
    assert!(!fix_it.is_null());
    assert_eq!(fix_it["file"], "src/main.vr");
    assert_eq!(fix_it["line"], 2);
    assert_eq!(fix_it["column"], 9);
    assert_eq!(fix_it["end_line"], 2);
    assert_eq!(fix_it["end_column"], 9);
    assert_eq!(fix_it["replacement"], "mut ");
}

/// A V0105 label and fix-it point into another module's file; each JSON
/// object names its own file, so an agent edits the right one.
#[test]
fn json_label_and_fix_it_in_a_second_file_name_that_file() {
    let main = SourceFile::new(FileId(0), "src/main.vr", "fn main() {\n    math::f();\n}\n");
    let math = SourceFile::new(FileId(1), "src/math.vr", "fn f() {}\n");
    let call = Span::new(FileId(0), 16, 23);
    let decl = Span::new(FileId(1), 0, 2);
    let diagnostic = Diagnostic::new(codes::V0105, call, "function `math::f` is not `pub`")
        .with_label(decl, "declared here without `pub`")
        .with_fix_it(FixIt {
            span: Span::new(FileId(1), 0, 0),
            replacement: "pub ".to_string(),
        });

    let rendered = render_json(&[diagnostic], &[main, math]);
    let value: serde_json::Value = serde_json::from_str(&rendered).expect("valid JSON");
    assert_eq!(value["file"], "src/main.vr");
    assert_eq!(value["line"], 2);
    assert_eq!(value["labels"][0]["file"], "src/math.vr");
    assert_eq!(value["labels"][0]["line"], 1);
    assert_eq!(value["fix_it"]["file"], "src/math.vr");
    assert_eq!(value["fix_it"]["column"], 1);
}

#[test]
fn json_fix_it_is_null_when_absent() {
    let file = SourceFile::new(FileId(0), "src/main.vr", "fn main() {}\n");
    let span = Span::new(FileId(0), 0, 2);
    let diagnostic = Diagnostic::new(codes::V0100, span, "unknown name `fo`".to_string());

    let rendered = render_json(&[diagnostic], &[file]);
    let value: serde_json::Value = serde_json::from_str(rendered.lines().next().unwrap()).unwrap();
    assert!(value["fix_it"].is_null());
    assert_eq!(value["labels"].as_array().unwrap().len(), 0);
    assert_eq!(value["notes"].as_array().unwrap().len(), 0);
}

#[test]
fn from_syntax_error_preserves_span_code_message_and_fix_it() {
    let span = Span::new(FileId(0), 3, 7);
    let fix_it_span = Span::new(FileId(0), 3, 7);
    let syntax_error = SyntaxError::new(span, codes::V0010, "`&x` at a call site".to_string())
        .with_fix_it(FixIt {
            span: fix_it_span,
            replacement: "x".to_string(),
        });

    let diagnostic: Diagnostic = syntax_error.into();

    assert_eq!(diagnostic.code, codes::V0010);
    assert_eq!(diagnostic.span, span);
    assert_eq!(diagnostic.message, "`&x` at a call site");
    let fix_it = diagnostic.fix_it.expect("fix-it preserved");
    assert_eq!(fix_it.span, fix_it_span);
    assert_eq!(fix_it.replacement, "x");
}
