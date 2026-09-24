//! Soundness: a program that passes `varyk check` must compile under rustc
//! (AGENTS.md, "Fail loudly"). Each case below is one small function, made
//! by putting a value into a context: every kind of value a borrowing rule
//! treats differently (literals, owned locals, parameters, fields, call
//! results, fields of temporaries, locals declared inside a block, `if`s
//! mixing them) in every kind of place a value goes (`let`, assignment,
//! arguments of each mode, imported Rust parameters, struct fields,
//! returns, `println!`, operators, field access, discarded statements).
//!
//! Every case is checked on its own. `check` may reject it (that is fine:
//! it is the analysis doing its job) but must not crash. The cases it
//! accepts are then built together as one program, which must compile;
//! on failure each accepted case is built alone to name the culprits.
//! The generated sweep this subset comes from covered several thousand
//! programs; these keep each cause it found covered.

mod common;

use std::fs;
use std::path::{Path, PathBuf};

use common::varyk;

/// Items every case can use; `ext` is the Rust module below.
const PRELUDE: &str = "mod ext;

struct P {
    s: string,
    n: i32,
}

struct Q {
    p: P,
    n: i32,
}

fn mk_s() -> string {
    \"made\"
}

fn mk_i() -> i32 {
    7
}

fn mk_p() -> P {
    P { s: \"mp\", n: 1 }
}

fn mk_q() -> Q {
    Q { p: mk_p(), n: 2 }
}

fn read_s(s: string) {}

fn change_s(mut s: string) {
    s = \"changed\";
}

fn read_i(x: i32) {}

fn change_i(mut x: i32) {
    x = 3;
}

fn read_p(p: P) {}

fn change_p(mut p: P) {
    p.n = 5;
}

fn sm(a: string, mut b: string) {}

fn ps2(a: P, b: string) {}
";

const EXT_RS: &str = "pub fn take_str(s: &str) -> usize { s.len() }
pub fn take_mut_string(s: &mut String) { s.push('!'); }
pub fn take_string(s: String) -> usize { s.len() }
pub fn take_i32(x: i32) -> i32 { x }
pub fn take_ref_i32(x: &i32) -> i32 { *x }
pub fn take_mut_i32(x: &mut i32) { *x += 1; }
";

/// Every case function's parameters: one of each kind of place.
const PARAMS: &str =
    "c: bool, ps: string, mut ms: string, pp: P, mut mp: P, pi: i32, mut mi: i32, mut mq: Q";

/// Owned locals every case starts with.
const SETUP: &str = "    let mut li = mk_i();
    let mut ls = mk_s();
    let mut ll = \"lit\";
    let mut lp = mk_p();
    let mut lq = mk_q();
";

/// Uses of the `mut` parameters after the case: a case that moved a
/// `&mut` reference instead of reborrowing it fails here.
const EPILOGUE: &str = "    read_s(ms);
    read_p(mp);
    read_i(mi);
    read_p(mq.p);
";

/// A context: a statement template around `{v}`, the case function's
/// return type (empty for none), and the values put into it.
struct Context {
    name: &'static str,
    ret: &'static str,
    body: &'static str,
    values: &'static [&'static str],
}

const CONTEXTS: &[Context] = &[
    // Borrowed as a whole when every branch is new: fields of temporaries,
    // locals declared inside the block; mixed ones are rejected.
    Context {
        name: "string argument",
        ret: "",
        body: "read_s({v});",
        values: &[
            "if c { mk_s() } else { mk_p().s }",
            "{ mk_p().s }",
            "{ let a = mk_p(); a.s }",
            "{ let r = mk_p().s; r }",
            "{ let a = mk_p(); let r = a.s; r }",
            "if c { mk_p().s } else { ps }",
            "if c { let a = \"q\"; a } else { mk_s() }",
        ],
    },
    Context {
        name: "mut string argument",
        ret: "",
        body: "change_s({v});",
        values: &[
            "if c { \"v\" } else { ls }",
            "if c { mk_s() } else { mk_p().s }",
            "(if c { lp } else { mp }).s",
            "(if c { pp } else { mp }).s",
            "({ pp }).s",
            "(if c { mk_p() } else { mk_p() }).s",
        ],
    },
    Context {
        name: "struct argument",
        ret: "",
        body: "read_p({v});",
        values: &[
            "if c { mk_p() } else { mk_q().p }",
            "{ let a = mk_p(); a }",
            "if c { mk_p() } else { pp }",
        ],
    },
    Context {
        name: "mut struct argument",
        ret: "",
        body: "change_p({v});",
        values: &["if c { lp } else { mp }", "if c { mk_p() } else { lq.p }"],
    },
    Context {
        name: "mut i32 argument",
        ret: "",
        body: "change_i({v});",
        values: &["if c { 5 } else { mi }", "if c { mk_i() } else { 5 }"],
    },
    Context {
        name: "imported arguments",
        ret: "",
        body: "ext::take_ref_i32({v});",
        values: &["if c { mk_i() } else { pi }", "(if c { lp } else { mp }).n"],
    },
    Context {
        name: "imported mut arguments",
        ret: "",
        body: "ext::take_mut_i32({v});\n    ext::take_mut_string((if c { lq } else { mq }).p.s);",
        values: &["if c { mk_i() } else { mi }", "(if c { lp } else { mp }).n"],
    },
    Context {
        name: "imported owned and borrowed text",
        ret: "",
        body: "ext::take_string({v});\n    ext::take_str(if c { mk_p().s } else { \"z\" });",
        values: &["mk_p().s", "{ let a = mk_s(); a }"],
    },
    // Read in place: printed, compared, discarded, or a field base.
    Context {
        name: "println",
        ret: "",
        body: "println!(\"{}\", {v});",
        values: &[
            "if c { mk_p().s } else { \"z\" }",
            "{ let a = mk_p(); a.s }",
            "(if c { lp } else { mp }).s",
            "(if c { lp } else { mk_p() }).s",
        ],
    },
    Context {
        name: "comparison",
        ret: "",
        body: "let b = {v} == ls;",
        values: &[
            "(if c { mk_p().s } else { \"z\" })",
            "({ let tmp = ls; tmp })",
            "(if c { mk_p() } else { lq.p }).s",
        ],
    },
    Context {
        name: "discarded",
        ret: "",
        body: "{v};",
        values: &[
            "if c { mk_p() } else { pp }",
            "if c { mk_p().s } else { \"z\" }",
        ],
    },
    Context {
        name: "field base",
        ret: "",
        body: "let n = ({v}).n;\n    read_s(({v}).s);",
        values: &["if c { mk_p() } else { lq.p }", "{ let a = mk_q(); a.p }"],
    },
    // Kept: `let`, assignment, and owned slots.
    Context {
        name: "let",
        ret: "",
        body: "let x = {v};\n    read_s(x);",
        values: &[
            "if c { \"v\" } else { mk_p().s }",
            "if c { mk_p().s } else { lp.s }",
            "{ let a = mk_p(); a.s }",
            "{ let tmp = mk_p().s; tmp }",
        ],
    },
    Context {
        name: "let struct",
        ret: "",
        body: "let x = {v};\n    read_p(x);",
        values: &[
            "if c { mk_q().p } else { pp }",
            "{ let r = mk_q().p; r }",
            "if c { let a = mk_p(); a } else { pp }",
        ],
    },
    Context {
        name: "changeable alias",
        ret: "",
        body: "let mut x = {v};\n    change_s(x);\n    x = \"q\";",
        values: &[
            "if c { \"v\" } else { lp.s }",
            "if c { ll } else { lp.s }",
            "if c { mp.s } else { lq.p.s }",
        ],
    },
    Context {
        name: "assignment to a borrowed-text let",
        ret: "",
        body: "let mut x = \"a\";\n    x = {v};\n    read_s(x);",
        values: &[
            "mk_p().s",
            "{ let tmp = ps; tmp }",
            "(if c { lp } else { mp }).s",
        ],
    },
    Context {
        name: "assignment through places",
        ret: "",
        body: "ms = {v};\n    mp.s = {v};\n    lq.p.s = {v};",
        values: &["{ let a = mk_s(); a }", "if c { \"v\" } else { mk_s() }"],
    },
    Context {
        name: "struct field and return",
        ret: " -> P",
        body: "let q = Q { p: {v}, n: 1 };\n    {v}",
        values: &["{ let a = mk_p(); a }", "if c { mk_p() } else { mk_q().p }"],
    },
];

/// Statement shapes around moves and repeated arguments.
const CONTROL: &[&str] = &[
    "let y = ls;\n    if c {\n        read_s(ls);\n    }",
    "while c {\n        let y = ls;\n        break;\n    }\n    read_s(ls);",
    "let y = ls;\n    ls = mk_s();\n    read_s(ls);",
    "ps2(lp, { change_p(lp); mk_s() });",
    "let b = ls == { let tmp = ls; tmp };",
    "sm(lp.s, (if c { lp } else { mp }).s);",
    "ps2(lp, (if c { lp } else { mp }).s);",
    "if c {\n        let t = mk_p();\n        ll = t.s;\n    }\n    read_s(ll);",
    "while c {\n        let t = mk_p();\n        ll = t.s;\n        break;\n    }\n    read_s(ll);",
    "if c {\n        let t = mk_p();\n        let u = t.s;\n        ll = u;\n    }\n    read_s(ll);",
];

/// Valid programs `check` must accept (and so build): a rule that wrongly
/// rejects one fails the test. A `let` from a field or a parameter is
/// another name for the value, not a move.
const MUST_PASS: &[&str] = &[
    "ps2(lp, { let t = lp.s; t });",
    "if c {\n        let t = lp.s;\n        ll = t;\n    }\n    read_s(ll);",
    "if c {\n        ll = lp.s;\n    }\n    read_s(ll);",
    "ps2(lp, { let t = lp.s; read_s(t); mk_s() });",
    "ps2(pp, { let q = pp; read_s(q.s); mk_s() });",
];

/// Every case as (label, function body, return type); a label starting
/// with `must pass` marks a [`MUST_PASS`] case.
fn cases() -> Vec<(String, String, &'static str)> {
    let mut cases = Vec::new();
    for context in CONTEXTS {
        for value in context.values {
            let body = context.body.replace("{v}", value);
            cases.push((format!("{}: {value}", context.name), body, context.ret));
        }
    }
    for body in CONTROL {
        cases.push((format!("control: {body}"), body.to_string(), ""));
    }
    for body in MUST_PASS {
        cases.push((format!("must pass: {body}"), body.to_string(), ""));
    }
    cases
}

/// The case function `name`: setup, the case, then the epilogue unless
/// the case ends in a returned value.
fn function(name: &str, body: &str, ret: &str) -> String {
    let epilogue = if ret.is_empty() { EPILOGUE } else { "" };
    format!("fn {name}({PARAMS}){ret} {{\n{SETUP}    {body}\n{epilogue}}}\n")
}

/// Writes a program made of `functions` into its own directory under
/// this test's scratch directory and returns the entry path.
fn write_program(dir: &str, functions: &[String]) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("soundness")
        .join(dir);
    fs::create_dir_all(&dir).expect("create the case directory");
    let mut source = format!("{PRELUDE}\n");
    for function in functions {
        source.push_str(function);
        source.push('\n');
    }
    source.push_str("fn main() {}\n");
    let entry = dir.join("main.vr");
    fs::write(&entry, source).expect("write main.vr");
    fs::write(dir.join("ext.rs"), EXT_RS).expect("write ext.rs");
    entry
}

/// Runs `varyk <command> <entry>`: whether it succeeded, and its stderr.
fn run(command: &str, entry: &Path) -> (bool, String) {
    let output = varyk(&[command, entry.to_str().expect("utf-8 path")]);
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(
        !stderr.contains("panicked"),
        "`varyk {command}` crashed on {}:\n{stderr}",
        entry.display()
    );
    (output.status.success(), stderr)
}

/// The first rustc error in cargo's output.
fn first_error(stderr: &str) -> &str {
    stderr
        .lines()
        .find(|line| line.starts_with("error[E"))
        .unwrap_or(stderr)
}

#[test]
fn programs_that_pass_check_compile() {
    let cases = cases();
    assert!(cases.len() <= 80, "keep this test small: {}", cases.len());

    let mut accepted = Vec::new();
    for (index, (label, body, ret)) in cases.iter().enumerate() {
        let function = function("t", body, ret);
        let entry = write_program(&format!("case{index:02}"), &[function]);
        let (checked, stderr) = run("check", &entry);
        assert!(
            checked || !label.starts_with("must pass"),
            "a valid program fails check: {label}\n{stderr}"
        );
        if checked {
            accepted.push((index, label, function_named(index, body, ret)));
        }
    }
    assert!(
        accepted.len() >= cases.len() / 3,
        "too few cases pass check ({} of {}); the prelude may be broken",
        accepted.len(),
        cases.len()
    );

    let functions: Vec<String> = accepted.iter().map(|(_, _, f)| f.clone()).collect();
    let entry = write_program("all", &functions);
    assert!(
        run("check", &entry).0,
        "the accepted cases together fail check"
    );
    let (built, stderr) = run("build", &entry);
    if built {
        return;
    }
    // Name the culprits: build each accepted case alone.
    let mut failures = Vec::new();
    for (index, label, function) in &accepted {
        let entry = write_program(&format!("case{index:02}"), std::slice::from_ref(function));
        let (built, stderr) = run("build", &entry);
        if !built {
            failures.push(format!("{label}\n    {}", first_error(&stderr)));
        }
    }
    panic!(
        "programs pass check but fail to compile:\n{}\n\nfull output of the combined build:\n{stderr}",
        failures.join("\n")
    );
}

/// The case function for case `index`, named uniquely so that all
/// accepted cases fit in one program.
fn function_named(index: usize, body: &str, ret: &str) -> String {
    function(&format!("t{index:02}"), body, ret)
}
