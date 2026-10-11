//! `varyk lsp`: a Language Server Protocol server that shows `varyk
//! check`'s diagnostics in an editor, on open and on save, and offers
//! each fix-it as a quick fix (language server spec).
//!
//! JSON-RPC 2.0 over standard input and output, each message framed by a
//! `Content-Length` header and a blank line (spec 3). Standard output
//! belongs to the protocol: only [`write_frame`] writes there, and log
//! lines for a person go to standard error.

use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::io::{self, BufRead, Read, Write};
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use serde_json::{Value, json};
use varyk_syntax::{SourceFile, Span};

use crate::cli::{self, Failure};
use crate::diagnostics::Diagnostic;
use crate::package::{self, StdRule};

/// JSON-RPC's error for a request that is not valid now (after
/// `shutdown`, or a second `initialize`).
const INVALID_REQUEST: i64 = -32600;
/// JSON-RPC's error for a request the server does not know.
const METHOD_NOT_FOUND: i64 = -32601;
/// JSON-RPC's error for a request whose parameters the server cannot read.
const INVALID_PARAMS: i64 = -32602;
/// LSP's error for a request before `initialize`.
const SERVER_NOT_INITIALIZED: i64 = -32002;

/// What the editor shows when a check panics (spec 7).
const CRASHED: &str = "the Varyk compiler crashed while checking this program; please report it with the file that caused it";

/// Reads one message's body from `input`: header lines up to a blank
/// line, then `Content-Length` bytes. `Ok(None)` is the end of input
/// before a message starts. Headers other than `Content-Length` are
/// ignored; a missing or unreadable length is an error, since nothing
/// then says where the next message starts.
fn read_frame(input: &mut impl BufRead) -> Result<Option<Vec<u8>>, String> {
    let mut length: Option<usize> = None;
    let mut started = false;
    loop {
        let mut line = Vec::new();
        let read = input
            .read_until(b'\n', &mut line)
            .map_err(|err| format!("cannot read standard input: {err}"))?;
        if read == 0 {
            if started {
                return Err("the input ended inside a message's header".to_string());
            }
            return Ok(None);
        }
        started = true;
        let line = String::from_utf8_lossy(&line);
        let line = line.trim_end_matches(['\r', '\n']);
        if line.is_empty() {
            break;
        }
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        if name.trim().eq_ignore_ascii_case("Content-Length") {
            let value = value.trim();
            length =
                Some(value.parse().map_err(|_| {
                    format!("a message's `Content-Length` is not a number: `{value}`")
                })?);
        }
    }
    let length = length.ok_or("a message has no `Content-Length` header")?;
    let mut body = Vec::new();
    input
        .take(length as u64)
        .read_to_end(&mut body)
        .map_err(|err| format!("cannot read standard input: {err}"))?;
    if body.len() != length {
        return Err("the input ended inside a message".to_string());
    }
    Ok(Some(body))
}

/// Writes `message` to `output` with its `Content-Length` header, and
/// flushes it so the client sees it at once.
fn write_frame(output: &mut impl Write, message: &Value) -> io::Result<()> {
    let body = message.to_string();
    write!(output, "Content-Length: {}\r\n\r\n{body}", body.len())?;
    output.flush()
}

/// What the server knows between messages.
#[derive(Default)]
struct Server {
    /// `initialize` has been answered.
    initialized: bool,
    /// `shutdown` has been answered.
    shut_down: bool,
    /// `exit` has arrived.
    exited: bool,
    /// The workspace folders `initialize` named, as paths.
    workspace_folders: Vec<PathBuf>,
    /// Each program checked so far, by its key (see [`program_of`]).
    programs: BTreeMap<PathBuf, Program>,
    /// The open documents changed since they were last saved, or opened
    /// with a text that is not the file's (spec 6).
    changed: HashSet<PathBuf>,
}

/// What the server keeps of a program's last check.
#[derive(Default)]
struct Program {
    /// The URIs it published diagnostics for.
    published: Vec<String>,
    /// Its diagnostics that have a fix-it.
    fixes: Vec<Fix>,
}

/// A diagnostic with a fix-it, from a program's last check (spec 6).
struct Fix {
    /// The file the diagnostic is in.
    document: PathBuf,
    /// The diagnostic in its LSP form.
    diagnostic: Value,
    /// The file the fix-it edits.
    file: PathBuf,
    /// `file`'s text as the check read it, which the fix-it's offsets are
    /// into.
    text: String,
    /// The fix-it's byte range in `text`.
    span: Span,
    /// What the fix-it puts in place of `span`.
    replacement: String,
}

impl Server {
    /// Handles one message and returns the messages to send for it, in
    /// order.
    fn handle(&mut self, message: &Value) -> Vec<Value> {
        let method = message.get("method").and_then(Value::as_str);
        match (message.get("id"), method) {
            (Some(id), Some(method)) => vec![self.request(id, method, message.get("params"))],
            (Some(id), None)
                if message.get("result").is_none() && message.get("error").is_none() =>
            {
                vec![error(id, INVALID_REQUEST, "a request needs a method")]
            }
            // A response: the server sends no requests, so none is awaited.
            (Some(_), None) => Vec::new(),
            (None, Some(method)) => self.notification(method, message.get("params")),
            (None, None) => {
                eprintln!("varyk lsp: skipped a message with neither a method nor an id");
                Vec::new()
            }
        }
    }

    /// Answers the request `id` for `method`.
    fn request(&mut self, id: &Value, method: &str, params: Option<&Value>) -> Value {
        if self.shut_down {
            return error(id, INVALID_REQUEST, "the server has been shut down");
        }
        if !self.initialized {
            if method != "initialize" {
                return error(
                    id,
                    SERVER_NOT_INITIALIZED,
                    "the server has not been initialized",
                );
            }
            self.initialized = true;
            self.workspace_folders = workspace_folders(params);
            return response(id, initialize_result());
        }
        match method {
            "initialize" => error(id, INVALID_REQUEST, "the server is already initialized"),
            "shutdown" => {
                self.shut_down = true;
                response(id, Value::Null)
            }
            "textDocument/codeAction" => match self.code_actions(params) {
                Some(actions) => response(id, actions),
                None => error(
                    id,
                    INVALID_PARAMS,
                    "a code action request needs a document and a range",
                ),
            },
            _ => error(id, METHOD_NOT_FOUND, &format!("unknown method `{method}`")),
        }
    }

    /// Acts on the notification `method` and returns the messages to send
    /// for it; any it does not know, and any before `initialize` but
    /// `exit`, it ignores.
    fn notification(&mut self, method: &str, params: Option<&Value>) -> Vec<Value> {
        if method == "exit" {
            self.exited = true;
            return Vec::new();
        }
        if !self.initialized {
            return Vec::new();
        }
        let Some(path) = document_path(params) else {
            return Vec::new();
        };
        match method {
            "textDocument/didOpen" => {
                let text = params
                    .and_then(|params| params.get("textDocument"))
                    .and_then(|document| document.get("text"))
                    .and_then(Value::as_str);
                let on_disk = fs::read_to_string(&path).ok();
                if text.is_some() && text == on_disk.as_deref() {
                    self.changed.remove(&path);
                } else {
                    self.changed.insert(path.clone());
                }
                self.check_vr(&path)
            }
            "textDocument/didChange" => {
                self.changed.insert(path);
                Vec::new()
            }
            "textDocument/didSave" => {
                self.changed.remove(&path);
                self.check_vr(&path)
            }
            "textDocument/didClose" => {
                self.changed.remove(&path);
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    /// [`Self::check`] for a `.vr` file; nothing for any other.
    fn check_vr(&mut self, path: &Path) -> Vec<Value> {
        if path.extension().and_then(|ext| ext.to_str()) == Some("vr") {
            self.check(path)
        } else {
            Vec::new()
        }
    }

    /// The quick fixes for a `textDocument/codeAction` request (spec 6):
    /// one per kept fix of its document whose diagnostic's range overlaps
    /// or touches the requested one, while neither that document nor the
    /// fix-it's file has changed since the check read it. None when the
    /// request has no document or range.
    fn code_actions(&self, params: Option<&Value>) -> Option<Value> {
        let params = params?;
        let uri = params.get("textDocument")?.get("uri")?.as_str()?;
        let range = params.get("range")?;
        let start = point(range.get("start")?)?;
        let end = point(range.get("end")?)?;
        let Some(document) = uri_to_path(uri) else {
            return Some(json!([]));
        };
        let actions: Vec<Value> = self
            .programs
            .values()
            .flat_map(|program| &program.fixes)
            .filter(|fix| fix.document == document && touches(&fix.diagnostic, start, end))
            .filter(|fix| {
                !self.changed.contains(&fix.document) && !self.changed.contains(&fix.file)
            })
            .filter(|fix| fs::read_to_string(&fix.file).is_ok_and(|text| text == fix.text))
            .map(code_action)
            .collect();
        Some(Value::Array(actions))
    }

    /// Checks the program `file` belongs to, afresh from disk, and returns
    /// what to publish (spec 4, 5), through [`Self::guarded`].
    fn check(&mut self, file: &Path) -> Vec<Value> {
        let (key, entry) = program_of(file, &self.workspace_folders);
        // A file that now belongs to `key` leaves every other program that
        // published for it or kept its fixes (a `main.vr` or a package
        // appeared around it), since no later check of those will clear
        // them. A program that was the file checked alone leaves with it
        // entirely: what its check published (a V0106, say) all goes.
        let mut left = Vec::new();
        for (other, program) in &mut self.programs {
            if *other == key {
                continue;
            }
            let alone = other == file;
            let (gone, kept): (Vec<String>, Vec<String>) = program
                .published
                .drain(..)
                .partition(|uri| alone || uri_to_path(uri).as_deref() == Some(file));
            program.published = kept;
            program.fixes.retain(|fix| !alone && fix.document != file);
            left.extend(gone);
        }
        let checked = self.guarded(key, || {
            cli::target_for_file(&entry)
                .and_then(|target| cli::check_target(target, StdRule::Build))
                .map(|_| ())
        });
        let mut sent: Vec<Value> = left
            .iter()
            .filter(|uri| !checked.iter().any(|sent| sent["params"]["uri"] == **uri))
            .map(|uri| publish(uri, Vec::new()))
            .collect();
        sent.extend(checked);
        sent
    }

    /// Runs `check` for the program `key` under `catch_unwind` and returns
    /// what to publish: the full list for every file with diagnostics now,
    /// and an empty one for every file the program's previous check
    /// published for that has none now. An entry that cannot be read is a
    /// warning, and a panic an error message (spec 7); either way nothing
    /// is published and the program's state stays.
    fn guarded(&mut self, key: PathBuf, check: impl FnOnce() -> Result<(), Failure>) -> Vec<Value> {
        let Ok(checked) = panic::catch_unwind(AssertUnwindSafe(check)) else {
            return vec![show_message(1, CRASHED)];
        };
        let (groups, fixes) = match checked {
            Ok(()) => (Vec::new(), Vec::new()),
            Err(Failure::Diagnostics(diagnostics, sources)) => (
                by_uri(&diagnostics, &sources),
                fixes(&diagnostics, &sources),
            ),
            Err(Failure::Message(message)) => return vec![show_message(2, &message)],
        };
        let program = self.programs.entry(key).or_default();
        program.fixes = fixes;
        let mut sent: Vec<Value> = program
            .published
            .iter()
            .filter(|uri| !groups.iter().any(|(group, _)| group == *uri))
            .map(|uri| publish(uri, Vec::new()))
            .collect();
        program.published = groups.iter().map(|(uri, _)| uri.clone()).collect();
        sent.extend(
            groups
                .into_iter()
                .map(|(uri, diagnostics)| publish(&uri, diagnostics)),
        );
        sent
    }
}

/// The path of a notification's `textDocument`, when its URI is a `file:`
/// one.
fn document_path(params: Option<&Value>) -> Option<PathBuf> {
    params?
        .get("textDocument")?
        .get("uri")?
        .as_str()
        .and_then(uri_to_path)
}

/// The program `file` belongs to (spec 4.2), as its key and its entry:
/// the package found upward from `file` when it has a root file, keyed by
/// its directory; else, for a file not named `main.vr`, the nearest
/// `main.vr` in its directory and each parent up to the deepest of
/// `folders` that holds it (only its own directory when none does); else
/// `file` itself. Outside a package the key is the entry.
fn program_of(file: &Path, folders: &[PathBuf]) -> (PathBuf, PathBuf) {
    let dir = file.parent().unwrap_or(Path::new(""));
    if let Some(manifest) = package::find(dir) {
        let root = manifest.parent().unwrap_or(Path::new(""));
        if let Some((_, entry)) = package::root_of(root) {
            return (root.to_path_buf(), entry);
        }
    }
    if file.file_name().and_then(|name| name.to_str()) != Some("main.vr") {
        let holder = folders
            .iter()
            .filter(|folder| dir.starts_with(folder))
            .max_by_key(|folder| folder.components().count());
        for ancestor in dir.ancestors() {
            let main = ancestor.join("main.vr");
            if main.is_file() {
                return (main.clone(), main);
            }
            if holder.is_none_or(|holder| ancestor == holder) {
                break;
            }
        }
    }
    (file.to_path_buf(), file.to_path_buf())
}

/// The fix-its of `diagnostics`, with what a code action needs of them
/// later; one whose files are not among `sources` is left out.
fn fixes(diagnostics: &[Diagnostic], sources: &[SourceFile]) -> Vec<Fix> {
    diagnostics
        .iter()
        .filter_map(|diagnostic| {
            let fix_it = diagnostic.fix_it.as_ref()?;
            let document = sources.get(diagnostic.span.file.0 as usize)?;
            let file = sources.get(fix_it.span.file.0 as usize)?;
            Some(Fix {
                document: absolute(&document.path),
                diagnostic: lsp_diagnostic(diagnostic, sources)?,
                file: absolute(&file.path),
                text: file.text.clone(),
                span: fix_it.span,
                replacement: fix_it.replacement.clone(),
            })
        })
        .collect()
}

/// An LSP position as `(line, character)`, which compare in the order of
/// the text.
fn point(position: &Value) -> Option<(u64, u64)> {
    Some((
        position.get("line")?.as_u64()?,
        position.get("character")?.as_u64()?,
    ))
}

/// Whether the range of the LSP `diagnostic` overlaps or touches the range
/// from `start` to `end`.
fn touches(diagnostic: &Value, start: (u64, u64), end: (u64, u64)) -> bool {
    let range = diagnostic.get("range");
    let from = range.and_then(|range| range.get("start")).and_then(point);
    let to = range.and_then(|range| range.get("end")).and_then(point);
    match (from, to) {
        (Some(from), Some(to)) => from <= end && start <= to,
        _ => false,
    }
}

/// The quick fix that applies `fix`: one edit in the fix-it's file, its
/// range converted against the text the check read.
fn code_action(fix: &Fix) -> Value {
    let edit = json!({
        "range": {
            "start": position(&fix.text, fix.span.start),
            "end": position(&fix.text, fix.span.end),
        },
        "newText": fix.replacement,
    });
    let mut changes = serde_json::Map::new();
    changes.insert(path_to_uri(&fix.file), json!([edit]));
    json!({
        "title": title(fix.span.start == fix.span.end, &fix.replacement),
        "kind": "quickfix",
        "isPreferred": true,
        "diagnostics": [fix.diagnostic],
        "edit": { "changes": changes },
    })
}

/// A quick fix's title (spec 6): `Insert` for an `insert` at an empty
/// range, `Replace with` otherwise, `Remove this` for an empty
/// `replacement`; one over 40 characters or spanning lines is shortened to
/// the first 40 characters of its first line and `…`.
fn title(insert: bool, replacement: &str) -> String {
    if replacement.is_empty() {
        return "Remove this".to_string();
    }
    let shown = if replacement.chars().count() > 40 || replacement.contains('\n') {
        let first = replacement.lines().next().unwrap_or_default();
        format!("{}…", first.chars().take(40).collect::<String>())
    } else {
        replacement.to_string()
    };
    if insert {
        format!("Insert `{shown}`")
    } else {
        format!("Replace with `{shown}`")
    }
}

/// The `textDocument/publishDiagnostics` notification for `uri`.
fn publish(uri: &str, diagnostics: Vec<Value>) -> Value {
    json!({
        "jsonrpc": "2.0",
        "method": "textDocument/publishDiagnostics",
        "params": { "uri": uri, "diagnostics": diagnostics },
    })
}

/// The `window/showMessage` notification of `kind` (1 error, 2 warning)
/// with `message`.
fn show_message(kind: u8, message: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "method": "window/showMessage",
        "params": { "type": kind, "message": message },
    })
}

/// The paths of `initialize`'s `workspaceFolders` that are `file:` URIs,
/// none when it names none.
fn workspace_folders(params: Option<&Value>) -> Vec<PathBuf> {
    params
        .and_then(|params| params.get("workspaceFolders"))
        .and_then(Value::as_array)
        .map_or_else(Vec::new, |folders| {
            folders
                .iter()
                .filter_map(|folder| folder.get("uri").and_then(Value::as_str))
                .filter_map(uri_to_path)
                .collect()
        })
}

/// The LSP position of byte `offset` in `text`: the line, counted at
/// `\n`, and the character, in UTF-16 units from the line's start. An
/// offset past the end is the end; one inside a character is that
/// character's start.
fn position(text: &str, offset: u32) -> Value {
    let mut offset = (offset as usize).min(text.len());
    while !text.is_char_boundary(offset) {
        offset -= 1;
    }
    let before = text.get(..offset).unwrap_or_default();
    let line_start = before.rfind('\n').map_or(0, |newline| newline + 1);
    let line = before.bytes().filter(|byte| *byte == b'\n').count();
    let character = before
        .get(line_start..)
        .unwrap_or_default()
        .encode_utf16()
        .count();
    json!({ "line": line, "character": character })
}

/// The LSP range of `span` in `file`'s text.
fn range(file: &SourceFile, span: Span) -> Value {
    json!({
        "start": position(&file.text, span.start),
        "end": position(&file.text, span.end),
    })
}

/// The path a `file:` URI names, percent-decoded; none for another
/// scheme, another host, or a URI that does not decode.
fn uri_to_path(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    let path = if rest.starts_with('/') {
        rest
    } else {
        rest.strip_prefix("localhost")?
    };
    if !path.starts_with('/') {
        return None;
    }
    let mut bytes = Vec::with_capacity(path.len());
    let mut rest = path.bytes();
    while let Some(byte) = rest.next() {
        if byte == b'%' {
            let high = rest.next().and_then(hex_digit)?;
            let low = rest.next().and_then(hex_digit)?;
            bytes.push(high * 16 + low);
        } else {
            bytes.push(byte);
        }
    }
    String::from_utf8(bytes).ok().map(PathBuf::from)
}

/// The value of an ASCII hexadecimal digit.
fn hex_digit(byte: u8) -> Option<u8> {
    char::from(byte)
        .to_digit(16)
        .and_then(|digit| u8::try_from(digit).ok())
}

/// The `file:` URI of `path`, made absolute against the working directory
/// first (a dependency's files are spelled relative to it), with every
/// byte but `/` and the unreserved ones percent-encoded.
fn path_to_uri(path: &Path) -> String {
    let path = absolute(path);
    let mut uri = String::from("file://");
    for byte in path.to_string_lossy().bytes() {
        if byte.is_ascii_alphanumeric() || b"/-._~".contains(&byte) {
            uri.push(char::from(byte));
        } else {
            uri.push_str(&format!("%{byte:02X}"));
        }
    }
    uri
}

/// `path` made absolute against the working directory, as it is when that
/// cannot be done.
fn absolute(path: &Path) -> PathBuf {
    std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf())
}

/// The LSP form of `diagnostic` (spec 5): at its primary span, an error
/// from `varyk` with its code, its message followed by a `note: ` line per
/// note, and its labels as related information. None when its file is not
/// among `sources`; a label whose file is not there is left out.
fn lsp_diagnostic(diagnostic: &Diagnostic, sources: &[SourceFile]) -> Option<Value> {
    let file = sources.get(diagnostic.span.file.0 as usize)?;
    let mut message = diagnostic.message.clone();
    for note in &diagnostic.notes {
        message.push_str("\nnote: ");
        message.push_str(note);
    }
    let related: Vec<Value> = diagnostic
        .labels
        .iter()
        .filter_map(|label| {
            let file = sources.get(label.span.file.0 as usize)?;
            Some(json!({
                "location": { "uri": path_to_uri(&file.path), "range": range(file, label.span) },
                "message": label.text,
            }))
        })
        .collect();
    Some(json!({
        "range": range(file, diagnostic.span),
        "severity": 1,
        "source": "varyk",
        "code": diagnostic.code,
        "message": message,
        "relatedInformation": related,
    }))
}

/// `diagnostics` in their LSP form, grouped by the URI of their primary
/// span's file, each group in the order of the check and the groups in the
/// order their first diagnostic came. A diagnostic whose file is not among
/// `sources` is skipped.
fn by_uri(diagnostics: &[Diagnostic], sources: &[SourceFile]) -> Vec<(String, Vec<Value>)> {
    let mut groups: Vec<(String, Vec<Value>)> = Vec::new();
    for diagnostic in diagnostics {
        let Some(file) = sources.get(diagnostic.span.file.0 as usize) else {
            continue;
        };
        let Some(value) = lsp_diagnostic(diagnostic, sources) else {
            continue;
        };
        let uri = path_to_uri(&file.path);
        match groups.iter_mut().find(|(group, _)| *group == uri) {
            Some((_, values)) => values.push(value),
            None => groups.push((uri, vec![value])),
        }
    }
    groups
}

/// What `initialize` answers: the capabilities of spec 3 and the
/// server's name and version. No `positionEncoding` means UTF-16.
fn initialize_result() -> Value {
    json!({
        "capabilities": {
            "textDocumentSync": { "openClose": true, "change": 2, "save": {} },
            "codeActionProvider": { "codeActionKinds": ["quickfix"] },
        },
        "serverInfo": { "name": "varyk", "version": env!("CARGO_PKG_VERSION") },
    })
}

/// A successful response to the request `id`.
fn response(id: &Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

/// An error response to the request `id`.
fn error(id: &Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

/// Serves LSP on `input` and `output`, message by message, until `exit`
/// or the end of input. Exits 0 after a `shutdown` request, 1 otherwise,
/// as LSP asks.
pub fn serve(mut input: impl BufRead, mut output: impl Write) -> ExitCode {
    let mut server = Server::default();
    while !server.exited {
        let body = match read_frame(&mut input) {
            Ok(Some(body)) => body,
            Ok(None) => break,
            Err(message) => {
                eprintln!("varyk lsp: {message}");
                break;
            }
        };
        let message: Value = match serde_json::from_slice(&body) {
            Ok(message) => message,
            Err(err) => {
                eprintln!("varyk lsp: skipped a message that is not JSON: {err}");
                continue;
            }
        };
        for outgoing in server.handle(&message) {
            if let Err(err) = write_frame(&mut output, &outgoing) {
                eprintln!("varyk lsp: cannot write to standard output: {err}");
                return ExitCode::FAILURE;
            }
        }
    }
    if server.shut_down {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use varyk_syntax::FileId;

    /// `body` framed as a client sends it.
    fn frame(body: &str) -> String {
        format!("Content-Length: {}\r\n\r\n{body}", body.len())
    }

    /// Every frame in `bytes`, parsed.
    fn frames(mut bytes: &[u8]) -> Vec<Value> {
        let mut messages = Vec::new();
        while let Some(body) = read_frame(&mut bytes).unwrap() {
            messages.push(serde_json::from_slice(&body).unwrap());
        }
        messages
    }

    /// Serves `input` and returns the exit code and what was written.
    fn session(input: &str) -> (ExitCode, Vec<Value>) {
        let mut output = Vec::new();
        let code = serve(input.as_bytes(), &mut output);
        (code, frames(&output))
    }

    const INITIALIZE: &str =
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"capabilities":{}}}"#;
    const SHUTDOWN: &str = r#"{"jsonrpc":"2.0","id":2,"method":"shutdown"}"#;
    const EXIT: &str = r#"{"jsonrpc":"2.0","method":"exit"}"#;

    #[test]
    fn reads_one_message() {
        let input = frame(r#"{"a":1}"#);
        let mut bytes = input.as_bytes();
        assert_eq!(
            read_frame(&mut bytes).unwrap(),
            Some(br#"{"a":1}"#.to_vec())
        );
        assert_eq!(read_frame(&mut bytes).unwrap(), None);
    }

    #[test]
    fn reads_two_messages_in_one_read() {
        let input = frame(r#"{"a":1}"#) + &frame(r#"{"b":2}"#);
        assert_eq!(
            frames(input.as_bytes()),
            vec![json!({"a": 1}), json!({"b": 2})]
        );
    }

    #[test]
    fn ignores_a_header_other_than_content_length() {
        let input = "Content-Type: application/vscode-jsonrpc; charset=utf-8\r\nContent-Length: 7\r\n\r\n{\"a\":1}";
        assert_eq!(frames(input.as_bytes()), vec![json!({"a": 1})]);
    }

    #[test]
    fn a_bad_length_is_an_error() {
        let mut bytes = "Content-Length: many\r\n\r\n{}".as_bytes();
        assert!(read_frame(&mut bytes).is_err());
        let mut bytes = "Content-Type: x\r\n\r\n{}".as_bytes();
        assert!(read_frame(&mut bytes).is_err());
    }

    #[test]
    fn a_bad_length_ends_the_server() {
        let input = frame(INITIALIZE) + "Content-Length: many\r\n\r\n" + &frame(SHUTDOWN);
        let (code, messages) = session(&input);
        assert_eq!(messages.len(), 1);
        assert_eq!(format!("{code:?}"), format!("{:?}", ExitCode::FAILURE));
    }

    #[test]
    fn a_body_that_is_not_json_is_skipped() {
        let input = frame("not json") + &frame(INITIALIZE);
        let (_, messages) = session(&input);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0]["id"], 1);
    }

    #[test]
    fn end_of_input_is_no_message() {
        let mut bytes = "".as_bytes();
        assert_eq!(read_frame(&mut bytes).unwrap(), None);
        let mut bytes = "Content-Length: 10\r\n\r\n{}".as_bytes();
        assert!(read_frame(&mut bytes).is_err());
    }

    #[test]
    fn initialize_answers_the_capabilities() {
        let (_, messages) = session(&frame(INITIALIZE));
        let result = &messages[0]["result"];
        assert_eq!(messages[0]["id"], 1);
        assert_eq!(
            result["capabilities"],
            json!({
                "textDocumentSync": { "openClose": true, "change": 2, "save": {} },
                "codeActionProvider": { "codeActionKinds": ["quickfix"] },
            })
        );
        assert_eq!(result["serverInfo"]["name"], "varyk");
        assert_eq!(result["serverInfo"]["version"], env!("CARGO_PKG_VERSION"));
    }

    /// `position` of `offset` in `text`, as `(line, character)`.
    fn at(text: &str, offset: u32) -> (u64, u64) {
        let position = position(text, offset);
        (
            position["line"].as_u64().unwrap(),
            position["character"].as_u64().unwrap(),
        )
    }

    #[test]
    fn positions_in_ascii() {
        let text = "let x = 1\nlet y = 2\n";
        assert_eq!(at(text, 0), (0, 0));
        assert_eq!(at(text, 4), (0, 4));
        assert_eq!(at(text, 10), (1, 0));
        assert_eq!(at(text, 14), (1, 4));
    }

    #[test]
    fn a_two_byte_character_is_one_unit() {
        // `é` is two bytes in UTF-8 and one unit in UTF-16.
        let text = "\"é\" + x";
        assert_eq!(at(text, 4), (0, 3));
        // An offset inside `é` is its start.
        assert_eq!(at(text, 2), (0, 1));
    }

    #[test]
    fn a_character_outside_the_basic_plane_is_two_units() {
        // `🦀` is four bytes in UTF-8 and two units in UTF-16.
        let text = "\"🦀\" + x";
        assert_eq!(at(text, 5), (0, 3));
        assert_eq!(at(text, 9), (0, 7));
    }

    #[test]
    fn a_carriage_return_is_part_of_its_line() {
        let text = "a\r\nbc\r\n";
        assert_eq!(at(text, 1), (0, 1));
        assert_eq!(at(text, 3), (1, 0));
        assert_eq!(at(text, 4), (1, 1));
    }

    #[test]
    fn an_offset_at_or_past_the_end_is_the_end() {
        let text = "ab\ncd";
        assert_eq!(at(text, 5), (1, 2));
        assert_eq!(at(text, 99), (1, 2));
        assert_eq!(at("", 3), (0, 0));
    }

    #[test]
    fn a_range_is_its_two_ends() {
        let file = SourceFile::new(FileId(0), "/work/main.vr", "a\nbcd");
        let span = Span::new(FileId(0), 3, 5);
        assert_eq!(
            range(&file, span),
            json!({
                "start": { "line": 1, "character": 1 },
                "end": { "line": 1, "character": 3 },
            })
        );
    }

    #[test]
    fn a_space_round_trips_as_percent_twenty() {
        let path = Path::new("/work/my shop/main.vr");
        let uri = path_to_uri(path);
        assert_eq!(uri, "file:///work/my%20shop/main.vr");
        assert_eq!(uri_to_path(&uri), Some(path.to_path_buf()));
    }

    #[test]
    fn a_non_ascii_name_round_trips() {
        let path = Path::new("/work/parduotuvė/main.vr");
        let uri = path_to_uri(path);
        assert_eq!(uri, "file:///work/parduotuv%C4%97/main.vr");
        assert_eq!(uri_to_path(&uri), Some(path.to_path_buf()));
    }

    #[test]
    fn uris_that_name_no_file_give_none() {
        assert_eq!(uri_to_path("untitled:Untitled-1"), None);
        assert_eq!(uri_to_path("file://server/share/main.vr"), None);
        assert_eq!(uri_to_path("file:///bad%2"), None);
        assert_eq!(
            uri_to_path("file://localhost/work/main.vr"),
            Some(PathBuf::from("/work/main.vr"))
        );
        assert_eq!(
            uri_to_path("file:///c%3A/work"),
            Some(PathBuf::from("/c:/work"))
        );
    }

    #[test]
    fn a_relative_path_is_made_absolute() {
        let cwd = std::env::current_dir().unwrap();
        assert_eq!(
            uri_to_path(&path_to_uri(Path::new("deps/shop/src/lib.vr"))),
            Some(cwd.join("deps/shop/src/lib.vr"))
        );
    }

    /// Two files, `/work/main.vr` and `/work/shop.vr`, with ids 0 and 1.
    fn two_files() -> Vec<SourceFile> {
        vec![
            SourceFile::new(
                FileId(0),
                "/work/main.vr",
                "fn main() {\n  shop.total()\n}\n",
            ),
            SourceFile::new(FileId(1), "/work/shop.vr", "fn total() -> Int {\n  0\n}\n"),
        ]
    }

    #[test]
    fn a_diagnostic_in_its_lsp_form() {
        let diagnostic =
            Diagnostic::new("V0105", Span::new(FileId(0), 19, 24), "`total` is private")
                .with_label(Span::new(FileId(1), 3, 8), "declared here")
                .with_note("first note")
                .with_note("second note");
        assert_eq!(
            lsp_diagnostic(&diagnostic, &two_files()),
            Some(json!({
                "range": {
                    "start": { "line": 1, "character": 7 },
                    "end": { "line": 1, "character": 12 },
                },
                "severity": 1,
                "source": "varyk",
                "code": "V0105",
                "message": "`total` is private\nnote: first note\nnote: second note",
                "relatedInformation": [{
                    "location": {
                        "uri": "file:///work/shop.vr",
                        "range": {
                            "start": { "line": 0, "character": 3 },
                            "end": { "line": 0, "character": 8 },
                        },
                    },
                    "message": "declared here",
                }],
            }))
        );
    }

    #[test]
    fn diagnostics_are_grouped_by_file_in_order() {
        let at = |file: u32, start: u32, message: &str| {
            Diagnostic::new("V0200", Span::new(FileId(file), start, start + 1), message)
        };
        let diagnostics = [
            at(0, 0, "a"),
            at(1, 0, "b"),
            at(9, 0, "lost"),
            at(0, 3, "c"),
        ];
        let groups = by_uri(&diagnostics, &two_files());
        let messages: Vec<(&str, Vec<&str>)> = groups
            .iter()
            .map(|(uri, values)| {
                let messages = values
                    .iter()
                    .map(|value| value["message"].as_str().unwrap());
                (uri.as_str(), messages.collect())
            })
            .collect();
        assert_eq!(
            messages,
            vec![
                ("file:///work/main.vr", vec!["a", "c"]),
                ("file:///work/shop.vr", vec!["b"]),
            ]
        );
    }

    use crate::package::tests::{MANIFEST, TempDir};

    #[test]
    fn a_package_file_is_checked_as_its_package() {
        let dir = TempDir::new("lsp_package");
        dir.write("Cargo.toml", MANIFEST);
        let main = dir.write("src/main.vr", "fn main() {}\n");
        let module = dir.write("src/shop.vr", "fn count() -> i32 { 1 }\n");
        assert_eq!(program_of(&module, &[]), (dir.0.clone(), main.clone()));
        assert_eq!(program_of(&main, &[]), (dir.0.clone(), main));
    }

    #[test]
    fn a_manifest_without_roots_leaves_a_single_file() {
        let dir = TempDir::new("lsp_stray_manifest");
        dir.write("Cargo.toml", MANIFEST);
        let tool = dir.write("tool.vr", "fn main() {}\n");
        assert_eq!(program_of(&tool, &[]), (tool.clone(), tool));
    }

    #[test]
    fn a_module_beside_a_main_is_checked_through_it() {
        let dir = TempDir::new("lsp_module");
        let main = dir.write("main.vr", "mod math;\nfn main() {}\n");
        let math = dir.write("math.vr", "fn one() -> i32 { 1 }\n");
        assert_eq!(program_of(&math, &[]), (main.clone(), main.clone()));
        let nested = dir.write("app/util/deep.vr", "fn one() -> i32 { 1 }\n");
        assert_eq!(
            program_of(&nested, std::slice::from_ref(&dir.0)),
            (main.clone(), main)
        );
    }

    #[test]
    fn a_main_above_the_workspace_folder_is_not_taken() {
        let dir = TempDir::new("lsp_above");
        dir.write("main.vr", "fn main() {}\n");
        let file = dir.write("work/sub/tool.vr", "fn main() {}\n");
        assert_eq!(
            program_of(&file, &[dir.0.join("work")]),
            (file.clone(), file.clone())
        );
        // With no workspace folder, only the file's own directory is looked in.
        assert_eq!(program_of(&file, &[]), (file.clone(), file));
    }

    #[test]
    fn a_file_named_main_is_taken_as_is() {
        let dir = TempDir::new("lsp_main");
        dir.write("main.vr", "mod sub;\nfn main() {}\n");
        let inner = dir.write("sub/main.vr", "fn main() {}\n");
        assert_eq!(
            program_of(&inner, std::slice::from_ref(&dir.0)),
            (inner.clone(), inner)
        );
    }

    /// A server that has answered `initialize`.
    fn initialized() -> Server {
        let mut server = Server::default();
        server.handle(&json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {} }));
        server
    }

    /// The `didSave` notification for `path`.
    fn did_save(path: &Path) -> Value {
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didSave",
            "params": { "textDocument": { "uri": path_to_uri(path) } },
        })
    }

    const JOINS_STRINGS: &str =
        "fn main() {\n    let s = \"a\" + \"b\";\n    println!(\"{}\", s);\n}\n";
    const FIXED: &str =
        "fn main() {\n    let s = format!(\"{}{}\", \"a\", \"b\");\n    println!(\"{}\", s);\n}\n";

    #[test]
    fn a_check_publishes_its_diagnostics_and_a_fix_clears_them() {
        let dir = TempDir::new("lsp_publish");
        let main = dir.write("main.vr", JOINS_STRINGS);
        let mut server = initialized();

        let sent = server.handle(&did_save(&main));
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0]["method"], "textDocument/publishDiagnostics");
        assert_eq!(sent[0]["params"]["uri"], path_to_uri(&main));
        let diagnostics = &sent[0]["params"]["diagnostics"];
        assert_eq!(diagnostics[0]["code"], "V0200");
        assert_eq!(
            diagnostics[0]["range"]["start"],
            json!({ "line": 1, "character": 12 })
        );

        dir.write("main.vr", FIXED);
        let sent = server.handle(&did_save(&main));
        assert_eq!(
            sent,
            vec![json!({
                "jsonrpc": "2.0",
                "method": "textDocument/publishDiagnostics",
                "params": { "uri": path_to_uri(&main), "diagnostics": [] },
            })]
        );
        // Nothing was published, so a clean check sends nothing.
        assert!(server.handle(&did_save(&main)).is_empty());
    }

    #[test]
    fn a_file_that_joins_a_program_loses_what_its_own_check_published() {
        let dir = TempDir::new("lsp_joins");
        let math = dir.write("math.vr", "pub fn one() -> i32 {\n    1\n}\n");
        let mut server = initialized();
        let sent = server.handle(&did_save(&math));
        assert_eq!(sent[0]["params"]["diagnostics"][0]["code"], "V0106");

        // A `main.vr` appears beside it: the V0106 of its own check goes.
        dir.write("main.vr", "mod math;\n\nfn main() {\n    math::one();\n}\n");
        assert_eq!(
            server.handle(&did_save(&math)),
            vec![publish(&path_to_uri(&math), Vec::new())]
        );
        assert!(server.handle(&did_save(&math)).is_empty());
    }

    #[test]
    fn a_file_that_moves_to_another_program_loses_what_the_first_published() {
        let dir = TempDir::new("lsp_moves");
        let outer = dir.write(
            "main.vr",
            "mod sub;\n\nfn main() {\n    sub::math::one();\n}\n",
        );
        dir.write("sub.vr", "pub mod math;\n");
        let math = dir.write(
            "sub/math.vr",
            "pub fn one() -> i32 {\n    let s = \"a\" + \"b\";\n    1\n}\n",
        );
        let mut server = initialized();
        server.workspace_folders = vec![dir.0.clone()];
        let sent = server.handle(&did_save(&math));
        assert_eq!(sent[0]["params"]["uri"], path_to_uri(&math));
        assert_eq!(sent[0]["params"]["diagnostics"][0]["code"], "V0200");
        assert_eq!(server.programs[&outer].fixes.len(), 1);

        // A clean `sub/main.vr` appears and takes `math.vr` from the outer
        // program: the outer program's diagnostic and fix for it go.
        dir.write(
            "sub/main.vr",
            "mod math;\n\nfn main() {\n    math::one();\n}\n",
        );
        dir.write("sub/math.vr", "pub fn one() -> i32 {\n    1\n}\n");
        assert_eq!(
            server.handle(&did_save(&math)),
            vec![publish(&path_to_uri(&math), Vec::new())]
        );
        assert!(server.programs[&outer].published.is_empty());
        assert!(server.programs[&outer].fixes.is_empty());
        assert!(server.handle(&did_save(&math)).is_empty());
    }

    #[test]
    fn a_panicking_check_is_an_error_and_keeps_the_last_state() {
        let dir = TempDir::new("lsp_panic");
        let main = dir.write("main.vr", JOINS_STRINGS);
        let mut server = initialized();
        server.handle(&did_save(&main));
        let (key, _) = program_of(&main, &server.workspace_folders);
        let before = &server.programs[&key];
        let (published, fixes) = (before.published.clone(), before.fixes.len());
        assert_eq!(fixes, 1);

        let sent = server.guarded(key.clone(), || panic!("a compiler bug"));
        assert_eq!(sent, vec![show_message(1, CRASHED)]);
        assert_eq!(server.programs[&key].published, published);
        assert_eq!(server.programs[&key].fixes.len(), fixes);
    }

    #[test]
    fn an_unreadable_entry_is_a_warning() {
        let dir = TempDir::new("lsp_unreadable");
        let mut server = initialized();
        let sent = server.handle(&did_save(&dir.0.join("gone.vr")));
        assert_eq!(sent[0]["method"], "window/showMessage");
        assert_eq!(sent[0]["params"]["type"], 2);
    }

    #[test]
    fn other_files_and_schemes_are_not_checked() {
        let dir = TempDir::new("lsp_other");
        let manifest = dir.write("Cargo.toml", "not toml [");
        let mut server = initialized();
        assert!(server.handle(&did_save(&manifest)).is_empty());
        let untitled = json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didOpen",
            "params": { "textDocument": { "uri": "untitled:Untitled-1.vr", "text": "" } },
        });
        assert!(server.handle(&untitled).is_empty());
    }

    #[test]
    fn titles_name_the_edit() {
        assert_eq!(
            title(false, "format!(\"{}\", a)"),
            "Replace with `format!(\"{}\", a)`"
        );
        assert_eq!(title(true, "pub "), "Insert `pub `");
        assert_eq!(title(false, ""), "Remove this");
        let long = "a".repeat(41);
        assert_eq!(
            title(false, &long),
            format!("Replace with `{}…`", "a".repeat(40))
        );
        assert_eq!(title(true, "first\nsecond"), "Insert `first…`");
    }

    /// A notification of `method` about `path`, with `text` when given.
    fn about(method: &str, path: &Path, text: Option<&str>) -> Value {
        let mut document = json!({ "uri": path_to_uri(path) });
        if let Some(text) = text {
            document["text"] = json!(text);
        }
        json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": { "textDocument": document },
        })
    }

    /// The code actions the server answers for a cursor at `line` and
    /// `character` in `path`.
    fn actions_at(server: &mut Server, path: &Path, line: u32, character: u32) -> Vec<Value> {
        let at = json!({ "line": line, "character": character });
        let answer = server.handle(&json!({
            "jsonrpc": "2.0", "id": 7, "method": "textDocument/codeAction",
            "params": {
                "textDocument": { "uri": path_to_uri(path) },
                "range": { "start": at, "end": at },
                "context": { "diagnostics": [] },
            },
        }));
        answer[0]["result"].as_array().unwrap().clone()
    }

    #[test]
    fn a_cursor_in_or_touching_a_diagnostic_gets_its_fix() {
        let dir = TempDir::new("lsp_cursor");
        let main = dir.write("main.vr", JOINS_STRINGS);
        let mut server = initialized();
        server.handle(&about("textDocument/didOpen", &main, Some(JOINS_STRINGS)));

        let actions = actions_at(&mut server, &main, 1, 15);
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0]["kind"], "quickfix");
        assert_eq!(actions[0]["isPreferred"], true);
        assert_eq!(actions[0]["diagnostics"][0]["code"], "V0200");
        assert_eq!(
            actions[0]["edit"]["changes"][path_to_uri(&main)],
            json!([{
                "range": {
                    "start": { "line": 1, "character": 12 },
                    "end": { "line": 1, "character": 21 },
                },
                "newText": "format!(\"{}{}\", \"a\", \"b\")",
            }])
        );
        assert_eq!(actions_at(&mut server, &main, 1, 12).len(), 1);
        assert_eq!(actions_at(&mut server, &main, 1, 21).len(), 1);
        assert!(actions_at(&mut server, &main, 1, 22).is_empty());
        assert!(actions_at(&mut server, &main, 0, 0).is_empty());
    }

    #[test]
    fn a_changed_document_gets_no_fix_until_saved() {
        let dir = TempDir::new("lsp_changed");
        let main = dir.write("main.vr", JOINS_STRINGS);
        let mut server = initialized();
        server.handle(&about("textDocument/didOpen", &main, Some(JOINS_STRINGS)));
        server.handle(&about("textDocument/didChange", &main, None));
        assert!(actions_at(&mut server, &main, 1, 15).is_empty());
        server.handle(&about("textDocument/didSave", &main, None));
        assert_eq!(actions_at(&mut server, &main, 1, 15).len(), 1);

        // An unsaved buffer restored on open differs from the disk.
        server.handle(&about(
            "textDocument/didOpen",
            &main,
            Some("fn main() {}\n"),
        ));
        assert!(actions_at(&mut server, &main, 1, 15).is_empty());
        server.handle(&about("textDocument/didClose", &main, None));
        assert_eq!(actions_at(&mut server, &main, 1, 15).len(), 1);
    }

    #[test]
    fn a_file_changed_on_disk_gets_no_fix() {
        let dir = TempDir::new("lsp_on_disk");
        let main = dir.write("main.vr", JOINS_STRINGS);
        let mut server = initialized();
        server.handle(&about("textDocument/didSave", &main, None));
        assert_eq!(actions_at(&mut server, &main, 1, 15).len(), 1);
        dir.write("main.vr", &format!("// moved\n{JOINS_STRINGS}"));
        assert!(actions_at(&mut server, &main, 1, 15).is_empty());
    }

    #[test]
    fn a_fix_can_edit_another_file() {
        let dir = TempDir::new("lsp_other_file");
        let main = dir.write(
            "main.vr",
            "mod shop;\n\nfn main() {\n    println!(\"{}\", shop::count());\n}\n",
        );
        let shop = dir.write("shop.vr", "fn count() -> i32 {\n    1\n}\n");
        let mut server = initialized();
        let sent = server.handle(&about("textDocument/didSave", &shop, None));
        assert_eq!(sent[0]["params"]["uri"], path_to_uri(&main));
        assert_eq!(sent[0]["params"]["diagnostics"][0]["code"], "V0105");

        let actions = actions_at(&mut server, &main, 3, 26);
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0]["title"], "Insert `pub `");
        assert_eq!(
            actions[0]["edit"]["changes"][path_to_uri(&shop)],
            json!([{
                "range": {
                    "start": { "line": 0, "character": 0 },
                    "end": { "line": 0, "character": 0 },
                },
                "newText": "pub ",
            }])
        );
        assert!(actions_at(&mut server, &shop, 0, 0).is_empty());
    }

    #[test]
    fn a_code_action_without_a_range_is_invalid() {
        let mut server = initialized();
        let answer = server.handle(&json!({
            "jsonrpc": "2.0", "id": 7, "method": "textDocument/codeAction",
            "params": { "textDocument": { "uri": "file:///work/main.vr" } },
        }));
        assert_eq!(answer[0]["error"]["code"], INVALID_PARAMS);
    }

    #[test]
    fn initialize_keeps_the_workspace_folders() {
        let mut server = Server::default();
        server.handle(&json!({
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": { "workspaceFolders": [{ "uri": "file:///work", "name": "work" }] },
        }));
        assert_eq!(server.workspace_folders, vec![PathBuf::from("/work")]);
    }

    #[test]
    fn a_request_before_initialize_is_refused() {
        let (_, messages) = session(&frame(SHUTDOWN));
        assert_eq!(messages[0]["id"], 2);
        assert_eq!(messages[0]["error"]["code"], SERVER_NOT_INITIALIZED);
    }

    #[test]
    fn an_unknown_method_is_not_found() {
        let unknown = r#"{"jsonrpc":"2.0","id":3,"method":"textDocument/hover","params":{}}"#;
        let cancel = r#"{"jsonrpc":"2.0","method":"$/cancelRequest","params":{"id":1}}"#;
        let (_, messages) = session(&(frame(INITIALIZE) + &frame(cancel) + &frame(unknown)));
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[1]["id"], 3);
        assert_eq!(messages[1]["error"]["code"], METHOD_NOT_FOUND);
    }

    #[test]
    fn a_request_after_shutdown_is_invalid() {
        let after = r#"{"jsonrpc":"2.0","id":3,"method":"textDocument/codeAction","params":{}}"#;
        let (_, messages) = session(&(frame(INITIALIZE) + &frame(SHUTDOWN) + &frame(after)));
        assert_eq!(
            messages[1],
            json!({ "jsonrpc": "2.0", "id": 2, "result": null })
        );
        assert_eq!(messages[2]["error"]["code"], INVALID_REQUEST);
    }

    #[test]
    fn exit_after_shutdown_is_zero() {
        let after_exit = r#"{"jsonrpc":"2.0","id":3,"method":"shutdown"}"#;
        let input = frame(INITIALIZE) + &frame(SHUTDOWN) + &frame(EXIT) + &frame(after_exit);
        let (code, messages) = session(&input);
        assert_eq!(format!("{code:?}"), format!("{:?}", ExitCode::SUCCESS));
        assert_eq!(messages.len(), 2);
    }

    #[test]
    fn exit_without_shutdown_is_one() {
        let (code, _) = session(&(frame(INITIALIZE) + &frame(EXIT)));
        assert_eq!(format!("{code:?}"), format!("{:?}", ExitCode::FAILURE));
        let (code, _) = session(&frame(INITIALIZE));
        assert_eq!(format!("{code:?}"), format!("{:?}", ExitCode::FAILURE));
    }
}
