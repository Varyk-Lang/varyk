//! Classifying one rustc compiler message (M3 spec 5): the promised net
//! under M1 §6.5's two-layer model. Varyk's own passes catch what they
//! understand; whatever reaches rustc and is rejected lands here, sorted
//! by where its primary span points:
//!
//! - a file this crate generated: reported as a Varyk diagnostic, V0900,
//!   at the mapped Varyk span (warnings are dropped — section 2.3's
//!   item-level lint allows should already have silenced them);
//! - a copied `.rs` module: the user's own Rust, shown as rustc rendered
//!   it but with every copied file's internal path rewritten to its
//!   user file, errors and warnings alike. rustc's `rendered` text can
//!   name a copied file through a span other than the primary one (a
//!   "function defined here" note, a help suggestion, ...), so every
//!   entry in `copied` is scrubbed from the whole text, not only the
//!   file the primary span points to.
//! - anything else (no span, a dependency, an unmapped generated line):
//!   rustc's rendered text, for the caller to show with M1's one-line
//!   note. This text is scrubbed the same way, since a message classified
//!   here can still mention a copied file in a secondary span.
//!
//! A *generated* file's internal path (`src/main.rs` and so on) is never
//! rewritten when it shows up in a secondary or child span rather than
//! the primary one: doing so honestly would mean converting that span's
//! byte range into a Varyk `file:line`, which needs the Varyk source
//! text, and this module only has [`SourceMap`] (spans, not text). The
//! primary span is already reported accurately (as `Generated` when it
//! points at generated code); a secondary reference to generated code is
//! left as rustc printed it rather than guessed at.

use std::borrow::Cow;
use std::path::{Path, PathBuf};

use crate::backend::SourceMap;
use crate::diagnostics::Diagnostic;
use crate::diagnostics::codes::V0900;

/// Where a V0900 diagnostic asks the reader to file a bug: generated code
/// rejected by rustc means one of Varyk's own passes should have caught
/// it and did not.
const ISSUES_URL: &str = "https://github.com/Varyk-Lang/varyk/issues";

/// What one rustc compiler message becomes, once classified against a
/// build's [`SourceMap`] and copied `.rs` modules (spec 5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Message {
    /// The primary span was in a file this crate generated: V0900 at the
    /// mapped Varyk span.
    Generated(Diagnostic),
    /// The primary span was in a copied `.rs` module: the user's own
    /// Rust, rustc's rendered text as is but with every copied file's
    /// internal path rewritten to its own user file. `file`, `line`,
    /// `column` (the primary span's start in the user's file), `message`
    /// (rustc's headline), `code` (rustc's, when it has one), and `notes`
    /// (rustc's `help:` and `note:` lines, each with its level) are for
    /// `--message-format=json`.
    User {
        rendered: String,
        level: String,
        file: PathBuf,
        line: u32,
        column: u32,
        message: String,
        code: Option<String>,
        notes: Vec<String>,
    },
    /// Nothing to map to. The caller shows `rendered` with M1's one-line
    /// note when the build failed; `level` and `message` (rustc's
    /// headline) are for `--message-format=json`.
    Other {
        rendered: String,
        level: String,
        message: String,
    },
}

/// Each of `msg`'s `children` (rustc's `help:` and `note:` lines) as
/// `<level>: <message>`, as its rendered text shows it.
fn child_notes(msg: &serde_json::Value) -> Vec<String> {
    msg.get("children")
        .and_then(|children| children.as_array())
        .into_iter()
        .flatten()
        .filter_map(|child| {
            let level = child.get("level")?.as_str()?;
            let message = child.get("message")?.as_str()?;
            Some(format!("{level}: {message}"))
        })
        .collect()
}

/// Classifies one rustc compiler message — the `message` object of a
/// `compiler-message` line, carrying `message`, `code`, `level`,
/// `spans`, and `rendered` — against `map` (the generated crate's
/// line-level source map, spec 2) and `copied` (its copied `.rs`
/// modules, path and source, spec 2.3). `crate_src` is the crate's root
/// as written to disk, used to make an absolute `file_name` tree-relative
/// before it is looked up.
///
/// `None` only for a warning whose primary span is in a generated file:
/// spec 5 drops these, since section 2.3's item-level lint allows should
/// already have silenced them.
pub fn classify(
    msg: &serde_json::Value,
    map: &SourceMap<'_>,
    copied: &[(String, PathBuf)],
    crate_src: &Path,
) -> Option<Message> {
    let level = msg.get("level")?.as_str()?.to_string();
    let raw_rendered = msg.get("rendered")?.as_str()?.to_string();
    let text = msg
        .get("message")
        .and_then(|m| m.as_str())
        .unwrap_or(raw_rendered.as_str())
        .to_string();
    let code = msg
        .get("code")
        .and_then(|c| c.get("code"))
        .and_then(|c| c.as_str());

    let Some(primary) = primary_span(msg) else {
        return Some(other(&raw_rendered, level, text, copied, crate_src));
    };
    let (Some(file_name), Some(line_start)) = (
        primary.get("file_name").and_then(|f| f.as_str()),
        primary.get("line_start").and_then(|l| l.as_u64()),
    ) else {
        return Some(other(&raw_rendered, level, text, copied, crate_src));
    };
    let line_start = line_start as u32;
    let tree_path = tree_relative(file_name, crate_src);

    if let Some((_, source)) = copied.iter().find(|(path, _)| path == tree_path.as_ref()) {
        let rendered = rewrite_copied_paths(&raw_rendered, copied, crate_src);
        let column = primary
            .get("column_start")
            .and_then(|c| c.as_u64())
            .unwrap_or(1) as u32;
        return Some(Message::User {
            rendered,
            level,
            file: source.clone(),
            line: line_start,
            column,
            message: text,
            code: code.map(str::to_string),
            notes: child_notes(msg),
        });
    }

    match map.lookup(tree_path.as_ref(), line_start) {
        Some(span) if level == "error" => {
            let message = match code {
                Some(code) => format!("{text} ({code})"),
                None => text,
            };
            let diagnostic = Diagnostic::new(V0900, span, message)
                .with_note(format!(
                    "{BUG_NOTE}: its own checks should have caught this before rustc ran; \
                     please report it at {ISSUES_URL}"
                ))
                .with_note("run `varyk build --emit-rust` to see the generated Rust");
            Some(Message::Generated(diagnostic))
        }
        // A warning in a generated file: dropped.
        Some(_) => None,
        None => Some(other(&raw_rendered, level, text, copied, crate_src)),
    }
}

/// The first note of every V0900: rustc rejected generated code, so one of
/// Varyk's own checks let something through.
const BUG_NOTE: &str = "this is a bug in Varyk, not in your program";

/// `messages` with a V0900 treated as a possible knock-on when rustc also
/// rejected a user's `.rs` file: an error there (a wrong type, a missing
/// item) can make correct generated code fail too, so such a V0900 drops
/// its "report it" note for one pointing at that error, and the user's
/// errors come first so that "above" holds.
pub fn after_user_errors(messages: Vec<Message>) -> Vec<Message> {
    let first_user_error = messages.iter().find_map(|message| match message {
        Message::User { level, file, .. } if level == "error" => Some(file.clone()),
        _ => None,
    });
    let Some(file) = first_user_error else {
        return messages;
    };
    let (mut user, rest): (Vec<Message>, Vec<Message>) = messages
        .into_iter()
        .partition(|message| matches!(message, Message::User { .. }));
    user.extend(rest.into_iter().map(|message| match message {
        Message::Generated(mut diagnostic) => {
            diagnostic.notes.retain(|note| !note.starts_with(BUG_NOTE));
            diagnostic.notes.insert(
                0,
                format!(
                    "this may follow from the error above in `{}`",
                    file.display()
                ),
            );
            Message::Generated(diagnostic)
        }
        other => other,
    }));
    user
}

/// An [`Message::Other`] of `raw_rendered`, scrubbed of copied paths.
fn other(
    raw_rendered: &str,
    level: String,
    message: String,
    copied: &[(String, PathBuf)],
    crate_src: &Path,
) -> Message {
    Message::Other {
        rendered: rewrite_copied_paths(raw_rendered, copied, crate_src),
        level,
        message,
    }
}

/// Replaces every occurrence of a copied module's internal path — its
/// crate-relative tree path (e.g. `src/defs.rs`) or its absolute form
/// under `crate_src`, whichever rustc used — with the user's own file,
/// wherever either appears in `rendered`. Every entry in `copied` is
/// scrubbed, not only the file the message's primary span points to,
/// since a secondary span (a "function defined here" note, a help
/// suggestion) can name a different copied file. Both forms are first
/// replaced by a marker and the markers by the user's path at the end,
/// so that the absolute form's relative suffix is not rewritten a second
/// time, and a user path that itself ends in the tree path (as
/// `/project/src/util.rs` does for `src/util.rs`) is not rewritten into
/// itself.
fn rewrite_copied_paths(rendered: &str, copied: &[(String, PathBuf)], crate_src: &Path) -> String {
    let mut rendered = rendered.to_string();
    let marker = |i: usize| format!("\u{0}{i}\u{0}");
    for (i, (tree_path, _)) in copied.iter().enumerate() {
        let absolute = crate_src.join(tree_path).display().to_string();
        // Anchored on the space rustc prints before a path (` --> p`,
        // ` ::: p`), so a dependency's path that merely ends in the same
        // `src/x.rs` is left alone.
        rendered = rendered.replace(&format!(" {absolute}"), &format!(" {}", marker(i)));
        rendered = rendered.replace(&format!(" {tree_path}"), &format!(" {}", marker(i)));
    }
    for (i, (_, source)) in copied.iter().enumerate() {
        rendered = rendered.replace(&marker(i), &source.display().to_string());
    }
    rendered
}

/// The message's primary span, if it has one.
fn primary_span(msg: &serde_json::Value) -> Option<&serde_json::Value> {
    msg.get("spans")?
        .as_array()?
        .iter()
        .find(|span| span.get("is_primary").and_then(|p| p.as_bool()) == Some(true))
}

/// `file_name` relative to the crate root: unchanged if rustc already
/// reported it that way (the common case, since cargo invokes rustc with
/// paths relative to the manifest directory even when `--manifest-path`
/// is absolute), or with `crate_src`'s prefix stripped if it is absolute.
fn tree_relative<'a>(file_name: &'a str, crate_src: &Path) -> Cow<'a, str> {
    let path = Path::new(file_name);
    if path.is_absolute() {
        if let Ok(relative) = path.strip_prefix(crate_src) {
            return relative.to_string_lossy().into_owned().into();
        }
    }
    file_name.into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::{GeneratedCrate, GeneratedFile, Writer};
    use serde_json::json;
    use varyk_syntax::{FileId, Span};

    fn span(start: u32, end: u32) -> Span {
        Span::new(FileId(0), start, end)
    }

    fn crate_with(files: Vec<GeneratedFile>, copied: Vec<(String, PathBuf)>) -> GeneratedCrate {
        GeneratedCrate {
            package_name: "p".to_string(),
            cargo_toml: String::new(),
            files,
            copied,
        }
    }

    #[test]
    fn an_error_in_a_generated_file_becomes_v0900_at_the_mapped_span() {
        let mut writer = Writer::new("src/shop/cart.rs");
        for i in 0..6 {
            writer.line(0, &format!("// line {i}"), None);
        }
        writer.line(0, "fn broken() {}", Some(span(10, 20)));
        let generated = crate_with(vec![writer.finish()], Vec::new());
        let map = generated.source_map();

        let msg = json!({
            "message": "mismatched types",
            "level": "error",
            "code": {"code": "E0308"},
            "spans": [{
                "file_name": "src/shop/cart.rs",
                "line_start": 7,
                "is_primary": true,
            }],
            "rendered": "error[E0308]: mismatched types\n --> src/shop/cart.rs:7:5\n",
        });

        match classify(&msg, &map, &generated.copied, Path::new("/build")) {
            Some(Message::Generated(diagnostic)) => {
                assert_eq!(diagnostic.code, V0900);
                assert_eq!(diagnostic.span, span(10, 20));
                assert!(
                    diagnostic.message.contains("mismatched types"),
                    "{}",
                    diagnostic.message
                );
                assert!(
                    diagnostic.message.contains("E0308"),
                    "{}",
                    diagnostic.message
                );
                assert!(
                    diagnostic
                        .notes
                        .iter()
                        .any(|note| note.contains("report") && note.contains("bug")),
                    "{:?}",
                    diagnostic.notes
                );
                assert!(
                    diagnostic
                        .notes
                        .iter()
                        .any(|note| note.contains("--emit-rust")),
                    "{:?}",
                    diagnostic.notes
                );
            }
            other => panic!("expected Generated, got {other:?}"),
        }
    }

    /// An error in a user's `.rs` file can make generated code fail too:
    /// the V0900 then points at that error instead of asking for a bug
    /// report, and comes after it.
    #[test]
    fn a_v0900_after_an_error_in_a_copied_file_points_at_it() {
        let mut writer = Writer::new("src/main.rs");
        writer.line(0, "fn main() { crate::util::f(); }", Some(span(0, 10)));
        let copied = vec![("src/util.rs".to_string(), PathBuf::from("util.rs"))];
        let generated = crate_with(vec![writer.finish()], copied);
        let map = generated.source_map();
        let error_at = |file: &str, line: u64| {
            json!({
                "message": "mismatched types",
                "level": "error",
                "code": {"code": "E0308"},
                "spans": [{"file_name": file, "line_start": line, "is_primary": true}],
                "rendered": format!("error[E0308]: mismatched types\n --> {file}:{line}:5\n"),
            })
        };
        let classified = |msgs: &[serde_json::Value]| -> Vec<Message> {
            msgs.iter()
                .filter_map(|msg| classify(msg, &map, &generated.copied, Path::new("/build")))
                .collect()
        };

        let generated_only = after_user_errors(classified(&[error_at("src/main.rs", 1)]));
        let [Message::Generated(diagnostic)] = &generated_only[..] else {
            panic!("expected one Generated, got {generated_only:?}");
        };
        assert!(diagnostic.notes[0].starts_with(BUG_NOTE), "{diagnostic:?}");

        let both = after_user_errors(classified(&[
            error_at("src/main.rs", 1),
            error_at("src/util.rs", 3),
        ]));
        let [Message::User { .. }, Message::Generated(diagnostic)] = &both[..] else {
            panic!("expected User then Generated, got {both:?}");
        };
        assert_eq!(
            diagnostic.notes[0],
            "this may follow from the error above in `util.rs`"
        );
        assert!(
            !diagnostic.notes.iter().any(|note| note.contains("report")),
            "{diagnostic:?}"
        );
        assert!(
            diagnostic
                .notes
                .iter()
                .any(|note| note.contains("--emit-rust")),
            "{diagnostic:?}"
        );
    }

    #[test]
    fn a_warning_in_a_generated_file_is_dropped() {
        let mut writer = Writer::new("src/main.rs");
        writer.line(0, "fn main() {}", Some(span(0, 5)));
        let generated = crate_with(vec![writer.finish()], Vec::new());
        let map = generated.source_map();

        let msg = json!({
            "message": "unused variable: `x`",
            "level": "warning",
            "code": {"code": "unused_variables"},
            "spans": [{
                "file_name": "src/main.rs",
                "line_start": 1,
                "is_primary": true,
            }],
            "rendered": "warning: unused variable: `x`\n --> src/main.rs:1:5\n",
        });

        assert_eq!(
            classify(&msg, &map, &generated.copied, Path::new("/build")),
            None
        );
    }

    #[test]
    fn an_error_in_a_copied_file_is_user_with_the_path_rewritten() {
        let copied = vec![("src/util.rs".to_string(), PathBuf::from("util.rs"))];
        let generated = crate_with(Vec::new(), copied);
        let map = generated.source_map();

        let msg = json!({
            "message": "mismatched types",
            "level": "error",
            "code": {"code": "E0308"},
            "spans": [{
                "file_name": "src/util.rs",
                "line_start": 3,
                "is_primary": true,
            }],
            "rendered": "error[E0308]: mismatched types\n --> src/util.rs:3:5\n  |\n",
        });

        match classify(&msg, &map, &generated.copied, Path::new("/build")) {
            Some(Message::User {
                rendered, level, ..
            }) => {
                assert_eq!(level, "error");
                assert!(rendered.contains("util.rs"), "{rendered}");
                assert!(!rendered.contains("src/util.rs"), "{rendered}");
            }
            other => panic!("expected User, got {other:?}"),
        }
    }

    #[test]
    fn a_warning_in_a_copied_file_is_user_at_warning_level() {
        let copied = vec![("src/util.rs".to_string(), PathBuf::from("util.rs"))];
        let generated = crate_with(Vec::new(), copied);
        let map = generated.source_map();

        let msg = json!({
            "message": "unused variable: `x`",
            "level": "warning",
            "code": {"code": "unused_variables"},
            "spans": [{
                "file_name": "src/util.rs",
                "line_start": 3,
                "is_primary": true,
            }],
            "rendered": "warning: unused variable: `x`\n --> src/util.rs:3:5\n",
            "children": [
                {"level": "help", "message": "if this is intentional, prefix it with an underscore: `_x`"},
                {"level": "note", "message": "`#[warn(unused_variables)]` on by default"},
            ],
        });

        match classify(&msg, &map, &generated.copied, Path::new("/build")) {
            Some(Message::User {
                rendered,
                level,
                notes,
                ..
            }) => {
                assert_eq!(level, "warning");
                assert!(rendered.contains("util.rs"), "{rendered}");
                assert_eq!(
                    notes,
                    vec![
                        "help: if this is intentional, prefix it with an underscore: `_x`",
                        "note: `#[warn(unused_variables)]` on by default",
                    ]
                );
            }
            other => panic!("expected User, got {other:?}"),
        }
    }

    /// A cross-module error: the primary span is in `src/use_it.rs`, a
    /// copied module, and a "function defined here" note (a span in the
    /// message's `rendered` text, not the top-level `spans` array — see
    /// the module docs) points at a different copied module,
    /// `src/defs.rs`. Neither internal tree path may survive; both are
    /// rewritten to their own user file (fix round 1).
    #[test]
    fn a_secondary_reference_to_another_copied_file_is_rewritten_too() {
        let copied = vec![
            ("src/use_it.rs".to_string(), PathBuf::from("use_it.rs")),
            ("src/defs.rs".to_string(), PathBuf::from("defs.rs")),
        ];
        let generated = crate_with(Vec::new(), copied);
        let map = generated.source_map();

        let msg = json!({
            "message": "this function takes 2 arguments but 3 arguments were supplied",
            "level": "error",
            "code": {"code": "E0061"},
            "spans": [{
                "file_name": "src/use_it.rs",
                "line_start": 3,
                "is_primary": true,
            }],
            "rendered": "error[E0061]: this function takes 2 arguments but 3 arguments were supplied\n --> src/use_it.rs:3:20\n  |\nnote: function defined here\n --> src/defs.rs:1:8\n  |\n",
        });

        match classify(&msg, &map, &generated.copied, Path::new("/build")) {
            Some(Message::User {
                rendered, level, ..
            }) => {
                assert_eq!(level, "error");
                assert!(rendered.contains("use_it.rs"), "{rendered}");
                assert!(rendered.contains("defs.rs"), "{rendered}");
                assert!(!rendered.contains("src/use_it.rs"), "{rendered}");
                assert!(!rendered.contains("src/defs.rs"), "{rendered}");
            }
            other => panic!("expected User, got {other:?}"),
        }
    }

    /// The absolute form of a copied file's path is rewritten too, and
    /// first, so its relative suffix (also present as a substring) is not
    /// left half-rewritten.
    #[test]
    fn an_absolute_copied_path_is_rewritten_before_its_relative_suffix() {
        let copied = vec![("src/util.rs".to_string(), PathBuf::from("util.rs"))];
        let generated = crate_with(Vec::new(), copied);
        let map = generated.source_map();
        let crate_src = Path::new("/build/pkg");

        let msg = json!({
            "message": "mismatched types",
            "level": "error",
            "code": {"code": "E0308"},
            "spans": [{
                "file_name": "src/util.rs",
                "line_start": 3,
                "is_primary": true,
            }],
            "rendered": "error[E0308]: mismatched types\n --> /build/pkg/src/util.rs:3:5\n",
        });

        match classify(&msg, &map, &generated.copied, crate_src) {
            Some(Message::User { rendered, .. }) => {
                assert!(rendered.contains("util.rs"), "{rendered}");
                assert!(!rendered.contains("/build/pkg"), "{rendered}");
                assert!(!rendered.contains("src/util.rs"), "{rendered}");
            }
            other => panic!("expected User, got {other:?}"),
        }
    }

    #[test]
    fn a_dependency_path_ending_like_a_copied_one_is_left_alone() {
        let copied = vec![(
            "src/util.rs".to_string(),
            PathBuf::from("/proj/src/util.rs"),
        )];

        let rendered = rewrite_copied_paths(
            " --> src/util.rs:3:5\n ::: /home/u/.cargo/registry/src/idx/foo-1.0/src/util.rs:5:1\n",
            &copied,
            Path::new("/build/pkg"),
        );

        assert_eq!(
            rendered,
            " --> /proj/src/util.rs:3:5\n ::: /home/u/.cargo/registry/src/idx/foo-1.0/src/util.rs:5:1\n"
        );
    }

    #[test]
    fn a_user_path_ending_in_the_tree_path_is_rewritten_once() {
        let copied = vec![(
            "src/util.rs".to_string(),
            PathBuf::from("/project/src/util.rs"),
        )];
        let crate_src = Path::new("/build/pkg");

        let rendered = rewrite_copied_paths(
            " --> /build/pkg/src/util.rs:3:5\n --> src/util.rs:9:1\n",
            &copied,
            crate_src,
        );

        assert_eq!(
            rendered,
            " --> /project/src/util.rs:3:5\n --> /project/src/util.rs:9:1\n"
        );
    }

    #[test]
    fn a_message_with_no_span_is_other() {
        let generated = crate_with(Vec::new(), Vec::new());
        let map = generated.source_map();

        let msg = json!({
            "message": "For more information about this error, try `rustc --explain E0308`.",
            "level": "failure-note",
            "code": serde_json::Value::Null,
            "spans": [],
            "rendered": "For more information about this error, try `rustc --explain E0308`.\n",
        });

        match classify(&msg, &map, &generated.copied, Path::new("/build")) {
            Some(Message::Other { rendered, .. }) => {
                assert!(rendered.contains("more information"), "{rendered}");
            }
            other => panic!("expected Other, got {other:?}"),
        }
    }

    #[test]
    fn a_generated_line_with_no_recorded_span_falls_back_to_other() {
        let mut writer = Writer::new("src/main.rs");
        writer.line(0, "fn main() {}", None);
        let generated = crate_with(vec![writer.finish()], Vec::new());
        let map = generated.source_map();

        let msg = json!({
            "message": "mismatched types",
            "level": "error",
            "code": {"code": "E0308"},
            "spans": [{
                "file_name": "src/main.rs",
                "line_start": 1,
                "is_primary": true,
            }],
            "rendered": "error[E0308]: mismatched types\n --> src/main.rs:1:1\n",
        });

        match classify(&msg, &map, &generated.copied, Path::new("/build")) {
            Some(Message::Other { rendered, .. }) => assert!(rendered.contains("mismatched types")),
            other => panic!("expected Other, got {other:?}"),
        }
    }

    /// A V0900 [`Diagnostic`] is an ordinary diagnostic: it renders
    /// through the normal human and JSON renderers like any other code
    /// (self-review: the CLI hands `Generated` diagnostics to
    /// `emit_diagnostics` unchanged).
    #[test]
    fn a_v0900_diagnostic_renders_as_human_text_and_as_json() {
        use crate::diagnostics::{render_human, render_json};
        use varyk_syntax::SourceFile;

        let mut writer = Writer::new("src/main.rs");
        writer.line(0, "fn main() {}", Some(span(0, 4)));
        let generated = crate_with(vec![writer.finish()], Vec::new());
        let map = generated.source_map();

        let msg = json!({
            "message": "mismatched types",
            "level": "error",
            "code": {"code": "E0308"},
            "spans": [{
                "file_name": "src/main.rs",
                "line_start": 1,
                "is_primary": true,
            }],
            "rendered": "error[E0308]: mismatched types\n --> src/main.rs:1:1\n",
        });

        let diagnostic = match classify(&msg, &map, &generated.copied, Path::new("/build")) {
            Some(Message::Generated(diagnostic)) => diagnostic,
            other => panic!("expected Generated, got {other:?}"),
        };
        let sources = vec![SourceFile::new(
            FileId(0),
            PathBuf::from("main.vr"),
            "fn main() {}".to_string(),
        )];

        let human = render_human(std::slice::from_ref(&diagnostic), &sources);
        assert!(human.contains("V0900"), "{human}");
        insta::assert_snapshot!("v0900_human", human);

        let json_out = render_json(std::slice::from_ref(&diagnostic), &sources);
        assert!(json_out.contains("\"V0900\""), "{json_out}");
    }
}
