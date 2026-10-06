//! Classifying one rustc compiler message (M3 spec 5): the promised net
//! under M1 §6.5's two-layer model. Varyk's own passes catch what they
//! understand; whatever reaches rustc and is rejected lands here, sorted
//! by where its primary span points:
//!
//! - a file this crate generated: reported as a Varyk diagnostic, V0900,
//!   at the mapped Varyk span (warnings are dropped — section 2.3's
//!   item-level lint allows should already have silenced them), or V0901
//!   when rustc says a value cannot go to another thread, which only a
//!   started call's task (milestone 5b1 spec 5), a route's or hook's
//!   adapter, and `App::new`'s state ask of it (milestone 5b4 spec 7.5);
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

use varyk_syntax::Span;

use super::PackageCrate;
use crate::backend::{Mark, SourceMap};
use crate::diagnostics::Diagnostic;
use crate::diagnostics::codes::{V0900, V0901};

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
    /// An error in the crate of a Varyk package of the build: V0900 at
    /// the package's `Cargo.toml` (M5b2 spec 4.3).
    Package(Diagnostic),
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
            let mark = map.mark(tree_path.as_ref(), line_start);
            if let Some(diagnostic) = thread_safety(msg, &text, span, mark) {
                return Some(Message::Generated(diagnostic));
            }
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

/// Classifies one rustc compiler message from the crate written for
/// `package`, a Varyk package of the build (M5b2 spec 4.3). Its warnings
/// are dropped. An error is V0900 naming the package, at its `Cargo.toml`,
/// since the source map covers only the program's own crate, followed by
/// rustc's own text as a note: when the error is in one of the package's
/// own `.rs` modules, it is that package's Rust, and no Varyk bug is
/// reported; anywhere else in the crate, Varyk generated it, so it is.
/// An error with no place in a file (rustc's "aborting" line, say) is
/// shown as for the program's own crate.
pub fn classify_package(msg: &serde_json::Value, package: &PackageCrate) -> Vec<Message> {
    let (Some(level), Some(raw_rendered)) = (
        msg.get("level").and_then(|l| l.as_str()),
        msg.get("rendered").and_then(|r| r.as_str()),
    ) else {
        return Vec::new();
    };
    if level != "error" {
        return Vec::new();
    }
    let copied = &package.generated.copied;
    let text = msg
        .get("message")
        .and_then(|m| m.as_str())
        .unwrap_or(raw_rendered)
        .to_string();
    let file_name = primary_span(msg)
        .and_then(|primary| primary.get("file_name"))
        .and_then(|f| f.as_str());
    let Some(file_name) = file_name else {
        let level = level.to_string();
        return vec![other(raw_rendered, level, text, copied, &package.dir)];
    };
    let tree_path = tree_relative(file_name, &package.dir);
    let headline = match msg
        .get("code")
        .and_then(|c| c.get("code"))
        .and_then(|c| c.as_str())
    {
        Some(code) => format!("{text} ({code})"),
        None => text.clone(),
    };
    let mut diagnostic = Diagnostic::new(
        V0900,
        Span::new(package.manifest_file, 0, 0),
        format!(
            "the package `{}` {} did not compile: {headline}",
            package.name, package.version
        ),
    );
    diagnostic = match copied.iter().find(|(path, _)| path == tree_path.as_ref()) {
        Some((_, source)) => diagnostic.with_note(format!(
            "the error is in the package's own Rust, `{}`, which Varyk builds as it is; \
             the package's author can fix it",
            source.display()
        )),
        None => diagnostic.with_note(format!(
            "{BUG_NOTE}: its own checks should have caught this before rustc ran; \
             please report it at {ISSUES_URL}"
        )),
    };
    vec![
        Message::Package(diagnostic),
        Message::Other {
            rendered: rewrite_copied_paths(raw_rendered, copied, &package.dir),
            level: "note".to_string(),
            message: text,
        },
    ]
}

/// V0901 at `span` when rustc's error `text` (`msg`'s headline) says a
/// value cannot be sent or shared between threads. Generated code has
/// three such bounds: `Task::start`, so the task of a started call holds
/// a value from Rust code that cannot go to another thread (milestone
/// 5b1 spec 5); a route's or hook's adapter, given to the package at its
/// registration, so its handler holds one; and `App::new`, so the state
/// holds one (milestone 5b4 spec 3, 7.5). `mark`, what the line belongs
/// to, picks the words. The type is the one rustc's `help` says lacks
/// `Send` or `Sync`, or the headline's own; `None` for any other error.
fn thread_safety(
    msg: &serde_json::Value,
    text: &str,
    span: Span,
    mark: Option<Mark>,
) -> Option<Diagnostic> {
    let sent = text.contains("cannot be sent between threads safely");
    if !sent && !text.contains("cannot be shared between threads safely") {
        return None;
    }
    let lacking = |child: &str| {
        ["Send", "Sync"].into_iter().find_map(|name| {
            let (_, rest) =
                child.split_once(&format!("the trait `{name}` is not implemented for `"))?;
            let ty = rest.strip_suffix('`').unwrap_or(rest);
            Some((ty.to_string(), name == "Send"))
        })
    };
    let from_help = child_notes(msg).iter().find_map(|child| lacking(child));
    let from_headline = || {
        let ty = text.strip_prefix('`')?.split_once("` cannot be ")?.0;
        Some((ty.to_string(), sent))
    };
    let (what, sent) = match from_help.or_else(from_headline) {
        Some((ty, sent)) => (format!("`{ty}`"), sent),
        None => ("a value from Rust code".to_string(), sent),
    };
    let (cannot, missing) = if sent {
        ("cannot be sent to another thread", "Send")
    } else {
        ("cannot be shared between threads", "Sync")
    };
    let (holder, why, instead) = match mark {
        Some(Mark::Route) => (
            "the handler or hook added here holds",
            "each request may be answered on another thread, so what a handler or hook is given, \
             and what it holds while it waits, must be able to go there",
            "",
        ),
        Some(Mark::State) => (
            "the state given here holds",
            "every request may be answered on its own thread, and every one of them reads \
             the app's state, so the state must be able to be shared between threads",
            "",
        ),
        None => (
            "the task started here holds",
            "a started task may run on another thread, so what it is given, and what it \
             holds while it waits, must be able to go there",
            ", or wait for the call with `.await` instead of starting it",
        ),
    };
    Some(
        Diagnostic::new(V0901, span, format!("{holder} {what}, which {cannot}"))
            .with_note(format!(
                "{why}; this value comes from Rust code, a `.rs` module or a crate it uses"
            ))
            .with_note(format!(
                "in Rust terms, {what} is not `{missing}`; in the Rust code, use `Arc` in place \
                 of `Rc`, and `Mutex` or an atomic in place of `Cell` or `RefCell`{instead}"
            )),
    )
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
        Message::Generated(mut diagnostic) if diagnostic.code == V0900 => {
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
    use crate::backend::{GeneratedCrate, GeneratedFile, Mark, Writer};
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

    /// A generated line with `span(10, 20)`, and rustc's error there:
    /// `message` and its `children`, with no code, as rustc gives a
    /// future's thread-safety errors.
    fn thread_error(message: &str, children: serde_json::Value) -> Option<Message> {
        marked_thread_error(None, message, children)
    }

    /// [`thread_error`] at a line that belongs to `mark`: a route's
    /// adapter or registration, or an `App::new` call (milestone 5b4
    /// spec 7.5).
    fn marked_thread_error(
        mark: Option<Mark>,
        message: &str,
        children: serde_json::Value,
    ) -> Option<Message> {
        let mut writer = Writer::new("src/main.rs");
        writer.marked_line(0, "let t = match (s,) { .. };", Some(span(10, 20)), mark);
        let generated = crate_with(vec![writer.finish()], Vec::new());
        let map = generated.source_map();
        let msg = json!({
            "message": message,
            "level": "error",
            "code": serde_json::Value::Null,
            "spans": [{"file_name": "src/main.rs", "line_start": 1, "is_primary": true}],
            "children": children,
            "rendered": format!("error: {message}\n --> src/main.rs:1:9\n"),
        });
        classify(&msg, &map, &generated.copied, Path::new("/build"))
    }

    /// rustc's "cannot be sent between threads safely" and "cannot be
    /// shared between threads safely" at a started call (milestone 5b1
    /// spec 5) are V0901 at the Varyk line, naming the Rust type, from
    /// the headline or from the `help` saying which trait it lacks, with
    /// no request for a bug report.
    #[test]
    fn a_thread_safety_error_in_a_generated_file_is_v0901_naming_the_type() {
        let cases = [
            (
                "future cannot be sent between threads safely",
                json!([
                    {"level": "help", "message": "within `{async block@src/main.rs:1:75: 1:85}`, \
                        the trait `Send` is not implemented for `Rc<String>`", "spans": []},
                    {"level": "note", "message": "required by a bound in `Task::<T>::start`", "spans": []},
                ]),
                "`Rc<String>`, which cannot be sent to another thread",
            ),
            (
                "future cannot be sent between threads safely",
                json!([{"level": "help", "message": "within `Counter`, the trait `Sync` is not \
                    implemented for `Cell<i64>`", "spans": []}]),
                "`Cell<i64>`, which cannot be shared between threads",
            ),
            (
                "`Rc<i64>` cannot be sent between threads safely",
                json!([]),
                "`Rc<i64>`, which cannot be sent to another thread",
            ),
            (
                "`RefCell<i64>` cannot be shared between threads safely",
                json!([]),
                "`RefCell<i64>`, which cannot be shared between threads",
            ),
            (
                "future cannot be sent between threads safely",
                json!([]),
                "a value from Rust code, which cannot be sent to another thread",
            ),
        ];
        for (message, children, headline) in cases {
            match thread_error(message, children) {
                Some(Message::Generated(diagnostic)) => {
                    assert_eq!(diagnostic.code, V0901, "{message}");
                    assert_eq!(diagnostic.span, span(10, 20));
                    assert!(
                        diagnostic.message.contains(headline),
                        "{message}: {}",
                        diagnostic.message
                    );
                    assert!(
                        !diagnostic.notes.iter().any(|note| note.contains("report")),
                        "{:?}",
                        diagnostic.notes
                    );
                }
                other => panic!("expected Generated, got {other:?}"),
            }
        }
    }

    /// A thread-safety error at a route's line, its adapter or its
    /// registration, where rustc puts "future cannot be sent", is V0901
    /// about the route's handler; one at an `App::new` line, where a
    /// state that cannot be shared fails, is V0901 about the state
    /// (milestone 5b4 spec 3, 7.5).
    #[test]
    fn a_thread_safety_error_at_a_route_or_app_new_is_v0901_in_their_words() {
        let sent = json!([{"level": "help", "message": "within `Session`, the trait `Send` is not \
            implemented for `Rc<String>`", "spans": []}]);
        let Some(Message::Generated(route)) = marked_thread_error(
            Some(Mark::Route),
            "future cannot be sent between threads safely",
            sent,
        ) else {
            panic!("expected Generated");
        };
        assert_eq!(route.code, V0901);
        assert_eq!(route.span, span(10, 20));
        assert_eq!(
            route.message,
            "the handler or hook added here holds `Rc<String>`, which cannot be sent to another thread"
        );
        assert!(
            route.notes.iter().any(|note| note.contains("each request")),
            "{:?}",
            route.notes
        );
        assert!(
            !route.notes.iter().any(|note| note.contains("started")),
            "{:?}",
            route.notes
        );

        let Some(Message::Generated(state)) = marked_thread_error(
            Some(Mark::State),
            "`Cell<i64>` cannot be shared between threads safely",
            json!([]),
        ) else {
            panic!("expected Generated");
        };
        assert_eq!(state.code, V0901);
        assert_eq!(state.span, span(10, 20));
        assert_eq!(
            state.message,
            "the state given here holds `Cell<i64>`, which cannot be shared between threads"
        );
        assert!(
            state
                .notes
                .iter()
                .any(|note| note.contains("every request")),
            "{:?}",
            state.notes
        );
        assert!(
            !state.notes.iter().any(|note| note.contains("started")),
            "{:?}",
            state.notes
        );
    }

    /// Any other rustc error in a generated file stays V0900, and a V0901
    /// does not take the note that it may follow from a user's error.
    #[test]
    fn only_thread_safety_errors_are_v0901() {
        match thread_error("mismatched types", json!([])) {
            Some(Message::Generated(diagnostic)) => assert_eq!(diagnostic.code, V0900),
            other => panic!("expected Generated, got {other:?}"),
        }
        let Some(v0901) =
            thread_error("`Rc<i64>` cannot be sent between threads safely", json!([]))
        else {
            panic!("expected a message");
        };
        let user = Message::User {
            rendered: String::new(),
            level: "error".to_string(),
            file: PathBuf::from("util.rs"),
            line: 1,
            column: 1,
            message: String::new(),
            code: None,
            notes: Vec::new(),
        };
        let both = after_user_errors(vec![v0901.clone(), user]);
        assert_eq!(both[1], v0901);
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

    /// The crate of the package `route` 0.1.0, whose `Cargo.toml` is file
    /// 3, with its `.rs` module `src/helper.rs` copied from
    /// `../route/src/helper.rs`.
    fn route_crate() -> PackageCrate {
        PackageCrate {
            name: "route".to_string(),
            version: "0.1.0".to_string(),
            manifest_file: FileId(3),
            dir: PathBuf::from("/app/target/varyk/packages/route-0.1.0"),
            generated: crate_with(
                Vec::new(),
                vec![(
                    "src/helper.rs".to_string(),
                    PathBuf::from("../route/src/helper.rs"),
                )],
            ),
        }
    }

    fn package_message(level: &str, file: &str) -> serde_json::Value {
        json!({
            "message": "mismatched types",
            "level": level,
            "code": {"code": "E0308"},
            "spans": [{"file_name": file, "line_start": 2, "is_primary": true}],
            "rendered": format!("{level}[E0308]: mismatched types\n --> {file}:2:5\n"),
        })
    }

    #[test]
    fn a_warning_in_a_package_crate_is_dropped() {
        let crate_ = route_crate();
        assert_eq!(
            classify_package(&package_message("warning", "src/lib.rs"), &crate_),
            []
        );
        assert_eq!(
            classify_package(&package_message("warning", "src/helper.rs"), &crate_),
            []
        );
    }

    #[test]
    fn an_error_in_a_package_s_generated_rust_is_v0900_naming_it_and_a_varyk_bug() {
        let messages = classify_package(&package_message("error", "src/lib.rs"), &route_crate());
        let [
            Message::Package(diagnostic),
            Message::Other { rendered, .. },
        ] = &messages[..]
        else {
            panic!("expected a V0900 and rustc's text, got {messages:?}");
        };
        assert_eq!(diagnostic.code, V0900);
        assert_eq!(diagnostic.span, Span::new(FileId(3), 0, 0));
        assert_eq!(
            diagnostic.message,
            "the package `route` 0.1.0 did not compile: mismatched types (E0308)"
        );
        assert!(
            diagnostic
                .notes
                .iter()
                .any(|note| note.starts_with(BUG_NOTE)),
            "{:?}",
            diagnostic.notes
        );
        assert!(rendered.contains("src/lib.rs:2:5"), "{rendered}");
    }

    #[test]
    fn an_error_in_a_package_s_rs_module_is_its_rust_and_no_varyk_bug() {
        let messages = classify_package(&package_message("error", "src/helper.rs"), &route_crate());
        let [
            Message::Package(diagnostic),
            Message::Other { rendered, .. },
        ] = &messages[..]
        else {
            panic!("expected a V0900 and rustc's text, got {messages:?}");
        };
        assert!(
            diagnostic.message.contains("the package `route` 0.1.0"),
            "{}",
            diagnostic.message
        );
        assert_eq!(
            diagnostic.notes,
            [
                "the error is in the package's own Rust, `../route/src/helper.rs`, which Varyk \
              builds as it is; the package's author can fix it"
            ]
        );
        assert!(
            rendered.contains("../route/src/helper.rs:2:5"),
            "{rendered}"
        );
    }
}
