//! Integration tests for `varyk lsp` (language server spec 9): sessions
//! played against the built binary over its standard input and output.
//!
//! Every byte the server writes to standard output is read as a frame, so
//! a stray line printed there by checking fails the test. The server
//! handles messages in order, so a request sent right after a
//! notification is answered by the next frame unless the notification
//! published something first.

mod common;

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, ExitStatus, Stdio};
use std::sync::mpsc::{Receiver, channel};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use serde_json::{Value, json};

/// How long to wait for the server's next frame before failing.
const TIMEOUT: Duration = Duration::from_secs(60);

/// A running `varyk lsp` and the frames it has written.
struct Session {
    child: Child,
    stdin: Option<ChildStdin>,
    frames: Receiver<Value>,
    reader: Option<JoinHandle<Result<(), String>>>,
    next_id: u64,
}

impl Session {
    /// Starts `varyk lsp` in `dir`.
    fn start(dir: &Path) -> Session {
        let mut child = Command::new(env!("CARGO_BIN_EXE_varyk"))
            .arg("lsp")
            .current_dir(dir)
            .env("VARYK_STD_PATH", common::std_path())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .expect("failed to spawn varyk lsp");
        let stdin = child.stdin.take();
        let stdout = child.stdout.take().expect("piped stdout");
        let (sender, frames) = channel();
        let reader = thread::spawn(move || {
            let mut stdout = BufReader::new(stdout);
            while let Some(frame) = read_frame(&mut stdout)? {
                if sender.send(frame).is_err() {
                    break;
                }
            }
            Ok(())
        });
        Session {
            child,
            stdin,
            frames,
            reader: Some(reader),
            next_id: 1,
        }
    }

    /// Sends `message` framed with its `Content-Length`.
    fn send(&mut self, message: Value) {
        let body = message.to_string();
        let stdin = self.stdin.as_mut().expect("stdin is open");
        write!(stdin, "Content-Length: {}\r\n\r\n{body}", body.len()).unwrap();
        stdin.flush().unwrap();
    }

    /// Sends the notification `method` with `params`.
    fn notify(&mut self, method: &str, params: Value) {
        self.send(json!({ "jsonrpc": "2.0", "method": method, "params": params }));
    }

    /// Sends the request `method` with `params` and returns its id.
    fn request(&mut self, method: &str, params: Value) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        self.send(json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }));
        id
    }

    /// The server's next frame; fails if none comes in time.
    fn next(&self) -> Value {
        self.frames
            .recv_timeout(TIMEOUT)
            .expect("the server sent no frame in time")
    }

    /// Sends the request and returns its answer, which must be the next
    /// frame.
    fn call(&mut self, method: &str, params: Value) -> Value {
        let id = self.request(method, params);
        let answer = self.next();
        assert_eq!(answer["id"], id, "expected the answer, got: {answer}");
        answer
    }

    /// The next frame, which must publish diagnostics for `path`; returns
    /// them.
    fn published(&self, path: &Path) -> Vec<Value> {
        let frame = self.next();
        assert_eq!(
            frame["method"], "textDocument/publishDiagnostics",
            "{frame}"
        );
        assert_eq!(frame["params"]["uri"], uri(path), "{frame}");
        frame["params"]["diagnostics"].as_array().unwrap().clone()
    }

    /// The code actions answered for a cursor at `line` and `character`
    /// in `path`.
    fn actions_at(&mut self, path: &Path, line: u64, character: u64) -> Vec<Value> {
        let at = json!({ "line": line, "character": character });
        let answer = self.call(
            "textDocument/codeAction",
            json!({
                "textDocument": { "uri": uri(path) },
                "range": { "start": at, "end": at },
                "context": { "diagnostics": [] },
            }),
        );
        answer["result"].as_array().unwrap().clone()
    }

    /// Closes standard input, waits for the server to end, and checks
    /// that everything it wrote was a frame and that no frame was left
    /// unread.
    fn finish(mut self) -> ExitStatus {
        drop(self.stdin.take());
        let status = self.child.wait().unwrap();
        let read = self.reader.take().unwrap().join().unwrap();
        assert_eq!(read, Ok(()), "the server wrote something not a frame");
        let rest: Vec<Value> = self.frames.try_iter().collect();
        assert!(rest.is_empty(), "unread frames: {rest:?}");
        status
    }
}

/// Reads one frame from `input` and parses its body; `None` at the end
/// of input between frames.
fn read_frame(input: &mut impl BufRead) -> Result<Option<Value>, String> {
    let mut length = None;
    let mut started = false;
    loop {
        let mut line = String::new();
        if input.read_line(&mut line).map_err(|err| err.to_string())? == 0 {
            return if started {
                Err("the output ended inside a header".to_string())
            } else {
                Ok(None)
            };
        }
        started = true;
        let line = line.trim_end_matches(['\r', '\n']);
        if line.is_empty() {
            break;
        }
        let (name, value) = line
            .split_once(':')
            .ok_or_else(|| format!("not a header: {line:?}"))?;
        if name.eq_ignore_ascii_case("Content-Length") {
            length = Some(
                value
                    .trim()
                    .parse::<usize>()
                    .map_err(|err| err.to_string())?,
            );
        }
    }
    let length = length.ok_or("a frame without Content-Length")?;
    let mut body = vec![0; length];
    input.read_exact(&mut body).map_err(|err| err.to_string())?;
    serde_json::from_slice(&body)
        .map(Some)
        .map_err(|err| err.to_string())
}

/// The `file:` URI of the absolute `path`, percent-encoded as the server
/// encodes it.
fn uri(path: &Path) -> String {
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

/// The `textDocument` parameter for `path`.
fn document(path: &Path) -> Value {
    json!({ "textDocument": { "uri": uri(path) } })
}

/// Initializes the session with `dir` as its workspace folder.
fn initialize(session: &mut Session, dir: &Path) {
    let answer = session.call(
        "initialize",
        json!({
            "capabilities": {},
            "workspaceFolders": [{ "uri": uri(dir), "name": "work" }],
        }),
    );
    assert_eq!(answer["result"]["serverInfo"]["name"], "varyk", "{answer}");
    session.notify("initialized", json!({}));
}

/// Sends `shutdown` and `exit` and expects status 0.
fn shut_down(mut session: Session) {
    let answer = session.call("shutdown", Value::Null);
    assert_eq!(answer["result"], Value::Null, "{answer}");
    session.notify("exit", Value::Null);
    let status = session.finish();
    assert_eq!(status.code(), Some(0));
}

const JOINS_STRINGS: &str =
    "fn main() {\n    let s = \"a\" + \"b\";\n    println!(\"{}\", s);\n}\n";
const FIXED: &str =
    "fn main() {\n    let s = format!(\"{}{}\", \"a\", \"b\");\n    println!(\"{}\", s);\n}\n";

#[test]
fn a_session_in_a_package_shows_diagnostics_and_offers_fixes() {
    let dir = common::empty_dir("lsp_session");
    let init = common::varyk_in(&dir, &["init"]);
    assert!(init.status.success(), "{init:?}");
    let main = dir.join("src/main.vr");
    fs::write(&main, JOINS_STRINGS).unwrap();

    let mut session = Session::start(&dir);
    initialize(&mut session, &dir);

    // Opening a file with a type error shows it, with its note.
    session.notify(
        "textDocument/didOpen",
        json!({ "textDocument": {
            "uri": uri(&main), "languageId": "varyk", "version": 1, "text": JOINS_STRINGS,
        } }),
    );
    let diagnostics = session.published(&main);
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    assert_eq!(diagnostics[0]["code"], "V0200");
    assert_eq!(
        diagnostics[0]["range"],
        json!({
            "start": { "line": 1, "character": 12 },
            "end": { "line": 1, "character": 21 },
        })
    );
    let message = diagnostics[0]["message"].as_str().unwrap();
    assert!(
        message.contains("\nnote: `+` works on numbers only"),
        "{message}"
    );

    // Its fix-it is a quick fix there.
    let actions = session.actions_at(&main, 1, 15);
    assert_eq!(actions.len(), 1, "{actions:?}");
    let edits = &actions[0]["edit"]["changes"][uri(&main)];
    assert_eq!(edits[0]["newText"], "format!(\"{}{}\", \"a\", \"b\")");

    // Not once the text changes without a save.
    session.notify(
        "textDocument/didChange",
        json!({
            "textDocument": { "uri": uri(&main), "version": 2 },
            "contentChanges": [{
                "range": {
                    "start": { "line": 0, "character": 0 },
                    "end": { "line": 0, "character": 0 },
                },
                "text": "// ",
            }],
        }),
    );
    assert!(session.actions_at(&main, 1, 15).is_empty());

    // Saving the fix clears the file.
    fs::write(&main, FIXED).unwrap();
    session.notify("textDocument/didSave", document(&main));
    assert!(session.published(&main).is_empty());

    // A use of an item declared without `pub` is fixed in its module.
    let shop = dir.join("src/shop.vr");
    fs::write(&shop, "fn count() -> i32 {\n    1\n}\n").unwrap();
    fs::write(
        &main,
        "mod shop;\n\nfn main() {\n    println!(\"{}\", shop::count());\n}\n",
    )
    .unwrap();
    session.notify("textDocument/didSave", document(&main));
    let diagnostics = session.published(&main);
    assert_eq!(diagnostics[0]["code"], "V0105", "{diagnostics:?}");
    let actions = session.actions_at(&main, 3, 25);
    assert_eq!(actions.len(), 1, "{actions:?}");
    let changes = actions[0]["edit"]["changes"].as_object().unwrap();
    assert_eq!(changes.len(), 1, "{changes:?}");
    let edits = &changes[&uri(&shop)];
    assert_eq!(edits[0]["newText"], "pub ");
    assert_eq!(
        edits[0]["range"]["start"],
        json!({ "line": 0, "character": 0 })
    );

    shut_down(session);
}

#[test]
fn single_files_are_checked_as_varyk_check_would() {
    let dir = common::empty_dir("lsp_single");
    let mut session = Session::start(&dir);
    initialize(&mut session, &dir);

    // A single file outside any package.
    let tool = dir.join("tool.vr");
    fs::write(&tool, JOINS_STRINGS).unwrap();
    session.notify("textDocument/didSave", document(&tool));
    let diagnostics = session.published(&tool);
    assert_eq!(diagnostics[0]["code"], "V0200", "{diagnostics:?}");

    // A module beside a single-file program's `main.vr` is checked through
    // it, so it is no program of its own (no V0106) and nothing is
    // published: the answer to the next request is the next frame.
    let main = dir.join("app/main.vr");
    let math = dir.join("app/math.vr");
    fs::create_dir_all(dir.join("app")).unwrap();
    fs::write(
        &main,
        "mod math;\n\nfn main() {\n    println!(\"{}\", math::one());\n}\n",
    )
    .unwrap();
    let math_text = "pub fn one() -> i32 {\n    1\n}\n";
    fs::write(&math, math_text).unwrap();
    session.notify(
        "textDocument/didOpen",
        json!({ "textDocument": {
            "uri": uri(&math), "languageId": "varyk", "version": 1, "text": math_text,
        } }),
    );
    assert!(session.actions_at(&math, 0, 0).is_empty());

    shut_down(session);
}

#[test]
fn a_request_before_initialize_and_an_unknown_method_get_errors() {
    let dir = common::empty_dir("lsp_errors");
    let mut session = Session::start(&dir);

    let answer = session.call("textDocument/codeAction", json!({}));
    assert_eq!(answer["error"]["code"], -32002, "{answer}");

    initialize(&mut session, &dir);
    let answer = session.call("varyk/unknown", json!({}));
    assert_eq!(answer["error"]["code"], -32601, "{answer}");

    // The input ends without `shutdown`: status 1.
    let status = session.finish();
    assert_eq!(status.code(), Some(1));
}
