//! Human-readable rendering via `annotate-snippets`, in rustc's style:
//! an `error[V0200]: message` header, a `--> file:line:col` line, the
//! source with a caret underline and label, `= note:` lines, and a
//! `help:` line for the fix-it.
//!
//! No color is used, so output is stable across terminals and snapshots.

use std::collections::HashMap;
use std::ops::Range;

use annotate_snippets::{AnnotationKind, Group, Level, Renderer, Snippet};
use varyk_syntax::SourceFile;

use super::{Diagnostic, source_file};

/// A single annotated span within one file's snippet.
struct Mark {
    range: Range<usize>,
    label: String,
    primary: bool,
}

/// Renders `diagnostics` in rustc's style.
///
/// `sources` is indexed by `FileId.0`: `sources[span.file.0 as usize]` is
/// always the file a span points into, because the file-resolution pass
/// (`resolve`) pushes files in id order. Multiple diagnostics are separated
/// by a blank line.
pub fn render_human(diagnostics: &[Diagnostic], sources: &[SourceFile]) -> String {
    let renderer = Renderer::plain();
    diagnostics
        .iter()
        .map(|diagnostic| render_one(diagnostic, sources, &renderer))
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn render_one(diagnostic: &Diagnostic, sources: &[SourceFile], renderer: &Renderer) -> String {
    // Group the primary span and every label by file id, keeping the
    // order files are first seen in (the primary span's file always
    // comes first).
    let mut file_order: Vec<u32> = Vec::new();
    let mut marks_by_file: HashMap<u32, Vec<Mark>> = HashMap::new();
    let mut push_mark = |file: u32, mark: Mark| {
        if !file_order.contains(&file) {
            file_order.push(file);
        }
        marks_by_file.entry(file).or_default().push(mark);
    };

    push_mark(
        diagnostic.span.file.0,
        Mark {
            range: diagnostic.span.start as usize..diagnostic.span.end as usize,
            label: diagnostic.message.clone(),
            primary: true,
        },
    );
    for label in &diagnostic.labels {
        push_mark(
            label.span.file.0,
            Mark {
                range: label.span.start as usize..label.span.end as usize,
                label: label.text.clone(),
                primary: false,
            },
        );
    }

    let title = Level::ERROR
        .primary_title(diagnostic.message.clone())
        .id(diagnostic.code);
    let mut group = Group::with_title(title);

    for file_id in &file_order {
        let file = source_file(sources, *file_id);
        let path = file.path.to_string_lossy().into_owned();
        let mut snippet = Snippet::source(file.text.as_str()).path(path).line_start(1);
        for mark in &marks_by_file[file_id] {
            let kind = if mark.primary {
                AnnotationKind::Primary
            } else {
                AnnotationKind::Context
            };
            snippet = snippet.annotation(kind.span(mark.range.clone()).label(mark.label.clone()));
        }
        group = group.element(snippet);
    }

    for note in &diagnostic.notes {
        group = group.element(Level::NOTE.message(note.clone()));
    }

    let mut groups = vec![group];

    if let Some(fix_it) = &diagnostic.fix_it {
        let file = source_file(sources, fix_it.span.file.0);
        let old = &file.text[fix_it.span.start as usize..fix_it.span.end as usize];
        let help = if fix_it.replacement.is_empty() {
            format!("remove `{old}`")
        } else if old.is_empty() {
            let (before, after) = file.text.split_at(fix_it.span.end as usize);
            insertion_help(fix_it.replacement.trim_start(), before, after)
        } else {
            format!("replace `{old}` with `{}`", fix_it.replacement)
        };
        groups.push(Group::with_title(Level::HELP.secondary_title(help)));
    }

    renderer.render(&groups)
}

/// The help text for an insertion fix-it (an empty span): "insert `mut `
/// before `user`", naming the word that follows the insertion point, else
/// "insert `.clone()` after `name`", naming the word that precedes it, or
/// just "insert `mut `" when neither is a word.
fn insertion_help(inserted: &str, before: &str, after: &str) -> String {
    let is_word = |c: &char| c.is_alphanumeric() || *c == '_';
    let next: String = after.chars().take_while(is_word).collect();
    if !next.is_empty() {
        return format!("insert `{inserted}` before `{next}`");
    }
    let previous: String = before.chars().rev().take_while(is_word).collect();
    let previous: String = previous.chars().rev().collect();
    if previous.is_empty() {
        format!("insert `{inserted}`")
    } else {
        format!("insert `{inserted}` after `{previous}`")
    }
}
