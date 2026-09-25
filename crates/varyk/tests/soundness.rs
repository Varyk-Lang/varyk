//! Soundness: a program that passes `varyk check` must compile under rustc
//! (AGENTS.md, "Fail loudly"). Each case below is one small function, made
//! by putting a value into a context. The values are the kinds a
//! borrowing rule treats differently: literals, owned locals, parameters,
//! fields, elements, call results, fields and elements of temporaries,
//! locals declared inside a block, `match` bindings, and `if`s, blocks,
//! and `match`es mixing them (an element of a temporary or of a
//! block-local `Vec` as a branch is covered as a string and a struct
//! argument, in `println!`, discarded, and as a field base). The contexts
//! are the places listed in [`CONTEXTS`]: `let`, assignment, arguments of
//! each mode, imported Rust parameters, struct fields, returns,
//! `println!`, operators, field access, discarded statements, receivers,
//! `match` and `for` heads, and `?`. Not every value is tried in every
//! context.
//!
//! Every case is checked on its own. `check` may reject it (that is fine:
//! it is the analysis doing its job) but must not crash. The cases it
//! accepts are then built together as one program, which must compile;
//! on failure each accepted case is built alone to name the culprits.
//! The generated sweep this subset comes from covered several thousand
//! programs; these keep each cause it found covered.
//!
//! The [`MUST_REJECT`] cases are programs rustc rejects: `check` must
//! reject each with its expected code. Each one that type-checks is also
//! generated to Rust past borrow analysis and built as one `[[bin]]` of a
//! single crate, and rustc must reject every one of them.

mod common;

use std::fs;
use std::path::{Path, PathBuf};

use common::varyk;
use varyk::backend::{Backend, RustBackend};
use varyk::borrow::analyze_unchecked;
use varyk::resolve::resolve;
use varyk::types::typecheck;
use varyk_syntax::{FileId, SourceFile};

/// Items every case can use; `ext` is the Rust module below.
const PRELUDE: &str = "mod ext;

struct P {
    s: string,
    n: i32,
}

struct Q {
    p: P,
    n: i32,
    e: E,
    v: Vec<P>,
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
    Q { p: mk_p(), n: 2, e: E::C, v: vec![mk_p()] }
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

fn change_q(mut q: Q) {
    q.n = 6;
}

fn sm(a: string, mut b: string) {}

fn ps2(a: P, b: string) {}

enum E {
    A(string),
    B(P),
    C,
}

fn mk_e() -> E {
    E::C
}

fn read_e(e: E) {}

fn read_v(v: Vec<P>) {}

fn pick(mut v: Vec<P>) -> usize {
    0
}

fn mk_vs() -> Vec<string> {
    vec![\"v\"]
}

fn mk_o2() -> Option<Vec<string>> {
    Some(mk_vs())
}

fn mk_ov() -> Option<Vec<P>> {
    Some(vec![mk_p()])
}

fn mk_o() -> Option<P> {
    Some(mk_p())
}

fn mk_r() -> Result<P, string> {
    Ok(mk_p())
}

fn mk_rs() -> Result<string, string> {
    Err(\"failed\")
}

impl Q {
    fn at(mut self) -> usize {
        0
    }
}

impl P {
    fn get_n(self) -> i32 {
        self.n
    }

    fn bump(mut self) {
        self.n = self.n + 1;
    }

    fn label(self) -> string {
        self.s.clone()
    }

    fn fresh() -> P {
        mk_p()
    }
}
";

const EXT_RS: &str = "pub fn take_str(s: &str) -> usize { s.len() }
pub fn take_mut_string(s: &mut String) { s.push('!'); }
pub fn take_string(s: String) -> usize { s.len() }
pub fn take_i32(x: i32) -> i32 { x }
pub fn take_ref_i32(x: &i32) -> i32 { *x }
pub fn take_mut_i32(x: &mut i32) { *x += 1; }
";

/// Every case function's parameters: one of each kind of place.
const PARAMS: &str = "c: bool, ps: string, mut ms: string, pp: P, mut mp: P, pi: i32, mut mi: i32, mut mq: Q, pe: E, mut me: E, pv: Vec<P>, mut mv: Vec<P>, pr: Result<P, string>";

/// Owned locals every case starts with.
const SETUP: &str = "    let mut li = mk_i();
    let mut ls = mk_s();
    let mut ll = \"lit\";
    let mut lp = mk_p();
    let mut lq = mk_q();
    let mut le = mk_e();
    let mut lv = vec![mk_p()];
    let mut lw = vec![mk_s()];
    let mut lo = mk_o();
    let mut lve = vec![mk_e()];
    let mut lk = Some(mk_i());
    let mut ln = vec![1, 2];
    let mut lr = mk_r();
    let mut lrv = vec![mk_r()];
";

/// Uses of the `mut` parameters after the case: a case that moved a
/// `&mut` reference instead of reborrowing it fails here.
const EPILOGUE: &str = "    read_s(ms);
    read_p(mp);
    read_i(mi);
    read_p(mq.p);
    read_e(me);
    read_v(mv);
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
            // A `match` read as a value follows the `if` rule (spec 3.2).
            "match le { E::A(s) => s, _ => \"x\" }",
            "match mk_e() { E::A(s) => s, _ => mk_s() }",
            "match pe { E::A(s) => s, _ => ps }",
            "match le { E::A(s) => s, _ => mk_s() }",
            // An element of a `Vec` gone after the value is gone with it.
            "if c { mk_vs()[0] } else { \"z\" }",
            "{ let a = mk_vs(); a[0] }",
            "match mk_o2() { Some(v) => v[0], None => \"z\" }",
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
            "if c { mk_q().v[0] } else { mk_p() }",
            "{ let a = mk_q(); a.v[0] }",
            "match mk_ov() { Some(v) => v[0], None => mk_p() }",
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
            "match le { E::A(s) => s, _ => \"z\" }",
            "if c { mk_vs()[0] } else { \"z\" }",
            "{ let a = mk_vs(); a[0] }",
            "match mk_o2() { Some(v) => v[0], None => \"z\" }",
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
            "if c { mk_vs()[0] } else { \"z\" }",
            "{ let a = mk_q(); a.v[0] }",
        ],
    },
    Context {
        name: "field base",
        ret: "",
        body: "let n = ({v}).n;\n    read_s(({v}).s);",
        values: &[
            "if c { mk_p() } else { lq.p }",
            "{ let a = mk_q(); a.p }",
            "if c { mk_q().v[0] } else { mk_p() }",
            "match mk_ov() { Some(v) => v[0], None => mk_p() }",
        ],
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
            "match pe { E::A(s) => s, _ => \"d\" }",
            "match mk_e() { E::A(s) => s, _ => mk_s() }",
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
            "match lo { Some(p) => p, None => pp }",
            "match mk_o() { Some(p) => p, None => mk_p() }",
            "match lo { Some(p) => p, None => mk_p() }",
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
    // Owned slots of milestone 2: variant payloads, constructor
    // arguments, and `vec!` elements.
    Context {
        name: "payload, constructor argument, and element",
        ret: "",
        body: "let e = E::A({v});\n    let o = Some({v});\n    let v = vec![{v}];",
        values: &[
            "if c { \"v\" } else { mk_s() }",
            "{ let a = mk_s(); a }",
            "if c { mk_s() } else { mk_p().s }",
        ],
    },
    // Variant values and `vec!` as values of their own.
    Context {
        name: "enum value",
        ret: "",
        body: "let x = {v};\n    read_e(x);\n    read_e({v});",
        values: &[
            "if c { E::B(mk_p()) } else { E::A(\"w\") }",
            "{ let a = E::A(\"a\"); a }",
            "if c { mk_e() } else { E::C }",
        ],
    },
    Context {
        name: "vec value",
        ret: "",
        body: "read_v({v});",
        values: &["if c { vec![mk_p()] } else { vec![] }", "vec![lp, mk_p()]"],
    },
    // Method receivers of each mode (spec 2.5), the built-in table, and
    // elements as places (spec 2.6).
    Context {
        name: "reading receiver",
        ret: "",
        body: "let n = {v}.get_n();\n    read_s({v}.label());",
        values: &[
            "pp",
            "mp",
            "lp",
            "lv[0]",
            "mq.p",
            "mk_p()",
            "if c { lp } else { pp }",
            "P::fresh()",
        ],
    },
    Context {
        name: "changing receiver",
        ret: "",
        body: "{v}.bump();",
        values: &[
            "mp",
            "lp",
            "lv[0]",
            "mq.p",
            "lq.p",
            "mk_p()",
            "if c { lp } else { mp }",
            "pp",
        ],
    },
    Context {
        name: "built-in calls on a vec",
        ret: "",
        body: "lv.push({v});\n    let n: usize = lv.len();\n    let o = lv.pop();",
        values: &[
            "mk_p()",
            "lp",
            "if c { mk_p() } else { lp }",
            "P::fresh()",
            "pp",
        ],
    },
    Context {
        name: "string clone and len",
        ret: "",
        body: "let t = {v}.clone();\n    read_s(t);\n    let n: usize = {v}.len();\n    lw.push({v}.clone());",
        values: &[
            "ps",
            "ms",
            "ls",
            "ll",
            "\"lit\"",
            "lp.s",
            "lw[0]",
            "if c { ps } else { ll }",
            "mk_s()",
        ],
    },
    Context {
        name: "element read",
        ret: "",
        body: "let x = {v};\n    read_p(x);\n    read_p({v});\n    let n = {v}.n;\n    println!(\"{}\", {v}.s);",
        values: &["lv[0]", "(if c { lv } else { vec![mk_p()] })[0]", "mq.p"],
    },
    Context {
        name: "element assignment",
        ret: "",
        body: "lw[0] = {v};\n    read_s(lw[0]);",
        values: &[
            "mk_s()",
            "\"w\"",
            "{ let a = mk_s(); a }",
            "if c { \"v\" } else { mk_s() }",
            "ls",
        ],
    },
    Context {
        name: "changeable element alias",
        ret: "",
        body: "let mut x = {v};\n    x.bump();\n    change_p(x);",
        values: &["lv[0]", "mp", "pp"],
    },
    Context {
        name: "returned clone",
        ret: " -> string",
        body: "{v}.clone()",
        values: &["ps", "lp.s", "lw[0]", "ll"],
    },
    // `match` on heads of each kind (spec 3.2): a place is looked at, and
    // its bindings are aliases or copies; a temporary is owned.
    Context {
        name: "match head",
        ret: "",
        body: "match {v} {\n        E::A(s) => read_s(s),\n        E::B(p) => read_p(p),\n        E::C => {}\n    }",
        values: &[
            "le",
            "pe",
            "me",
            "mq.e",
            "lve[0]",
            "mk_e()",
            "E::A(ls)",
            "mk_q().e",
            "if c { le } else { mk_e() }",
        ],
    },
    Context {
        name: "match bindings as values",
        ret: "",
        body: "match {v} {\n        E::A(s) => {\n            println!(\"{}\", s == \"x\");\n            let t = s;\n            read_s(t);\n        }\n        E::B(p) => {\n            let n = p.n;\n            read_s(p.s);\n            let q = p;\n            read_p(q);\n        }\n        E::C => {}\n    }",
        values: &["le", "pe", "me", "mq.e", "lve[0]", "mk_e()"],
    },
    Context {
        name: "match bindings into owned slots",
        ret: "",
        body: "match {v} {\n        E::A(s) => lw.push(s),\n        E::B(p) => lv.push(p),\n        E::C => {}\n    }",
        values: &["le", "mk_e()", "E::B(mk_p())"],
    },
    Context {
        name: "match copy bindings",
        ret: "",
        body: "match {v} {\n        Some(n) => {\n            lk = None;\n            change_i(mi);\n            read_i(n);\n        }\n        None => {}\n    }",
        values: &["lk", "Some(mk_i())"],
    },
    // `for` over heads of each kind (spec 2.4, 3.2): a place is looked at
    // for the whole loop, and its variable is an alias or a copy; a
    // temporary is owned; a range's ends are evaluated once.
    Context {
        name: "for head",
        ret: "",
        body: "for x in {v} {\n        read_p(x);\n        let n = x.n;\n        let y = x;\n        println!(\"{}\", y.s);\n    }",
        values: &[
            "lv",
            "pv",
            "mv",
            "mq.v",
            "lq.v",
            "vec![mk_p()]",
            "mk_q().v",
            "if c { lv } else { pv }",
        ],
    },
    Context {
        name: "for string variable",
        ret: "",
        body: "for s in {v} {\n        read_s(s);\n        let t = s;\n        println!(\"{}\", t == \"x\");\n    }",
        values: &["lw", "mk_vs()"],
    },
    Context {
        name: "for copy variable",
        ret: "",
        body: "for n in {v} {\n        change_i(mi);\n        li = n;\n        read_i(n);\n    }",
        values: &["ln", "vec![mk_i()]"],
    },
    Context {
        name: "for range",
        ret: "",
        body: "for i in {v} {\n        println!(\"{}\", i);\n        lv.push(mk_p());\n    }",
        values: &[
            "0..li",
            "mi..mk_i()",
            "0..lv.len()",
            "lv.len()..0",
            "{ li }..if c { 3 } else { li }",
            "0..{ li }",
        ],
    },
    Context {
        name: "struct field and return",
        ret: " -> P",
        body: "let q = Q { p: {v}, n: 1, e: E::C, v: Vec::new() };\n    {v}",
        values: &["{ let a = mk_p(); a }", "if c { mk_p() } else { mk_q().p }"],
    },
    // `?` (spec 2.8): its operand is an owned slot (spec 3.4), and its
    // value is a new value, like a call result.
    Context {
        name: "question operand",
        ret: RESULT_RET,
        body: "let x = {v}?;\n    read_p(x);\n    Ok(x.n)",
        values: &[
            "mk_r()",
            "lr",
            "if c { lr } else { mk_r() }",
            "{ let a = mk_r(); a }",
            "match mk_o() { None => mk_r(), Some(p) => Ok(p) }",
            "pr",
            "lrv[0]",
        ],
    },
    Context {
        name: "question text value",
        ret: RESULT_RET,
        body: "{v};\n    read_s({v});\n    let t = {v};\n    lw.push({v});\n    println!(\"{}\", {v});\n    ls = {v};\n    Ok(1)",
        values: &[
            "mk_rs()?",
            "{ let r = mk_rs(); r? }",
            "if c { mk_rs()? } else { \"lit\" }",
        ],
    },
    Context {
        name: "question struct value",
        ret: RESULT_RET,
        body: "read_p({v});\n    read_s({v}.s);\n    let x = {v};\n    lv.push({v});\n    let n = {v}.n;\n    Ok(n + {v}.get_n())",
        values: &[
            "mk_r()?",
            "{ let a = mk_r()?; a }",
            "if c { mk_r()? } else { mk_p() }",
        ],
    },
];

/// The return type of a case using `?`.
const RESULT_RET: &str = " -> Result<i32, string>";

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
    // `==` on strings in every mix of representations: `String`, `&str`,
    // `&mut String`, a `&String` alias, a field, an element, a call result.
    "let n = lp.s;\n    let b = ls == ps && ps == ls && ms == ls && ls == ms && n == ls && ls == n && n == ps && lp.s == n && ms != n && lw[0] == ls && mk_s() == n && pp.s == ms && ll == lp.s;",
    "for s in lw {\n        let b = s == ls && ls != s && s == lp.s && ps == s && s == mk_s();\n    }",
    "ps2(lp, { let t = lp.s; t });",
    "if c {\n        let t = lp.s;\n        ll = t;\n    }\n    read_s(ll);",
    "if c {\n        ll = lp.s;\n    }\n    read_s(ll);",
    "ps2(lp, { let t = lp.s; read_s(t); mk_s() });",
    "ps2(pp, { let q = pp; read_s(q.s); mk_s() });",
    // An alias and its root (spec 3.1): the root changes after the alias's
    // last use, and a read-only alias's root is read while it is in use.
    "let n = lp.s;\n    read_s(n);\n    change_p(lp);\n    read_p(lp);",
    "let n = mp.s;\n    read_p(mp);\n    read_s(n);\n    println!(\"{}\", mp.s);",
    // An alias made by assignment, last used before its root changes.
    "ll = lp.s;\n    read_s(ll);\n    change_p(lp);",
    // Elements and receivers (spec 2.6, 3.1): an element alias last used
    // before its `Vec` changes.
    "lv[0].bump();\n    let n = lv[0].get_n();\n    lv.push(mk_p());",
    "let first = lv[0];\n    read_p(first);\n    lv[0].bump();",
    "let n = lw.len();\n    lw.push(mk_s());",
    // A text element of a temporary `Vec` is borrowed as a `String`.
    "let t = mk_vs()[0];\n    read_s(t);",
    // `match` (spec 2.3, 3.2): a variant arm repeated after an identical
    // one (rustc's unreachable-pattern warning), a unit arm changing the
    // root, a binding shadowing an outer local, and a temporary's binding
    // given away.
    "match le {\n        E::C => {}\n        E::C => {}\n        _ => {}\n    }",
    "match le {\n        E::C => {\n            le = mk_e();\n        }\n        _ => {}\n    }\n    read_e(le);",
    "match le {\n        E::A(ls) => read_s(ls),\n        _ => {}\n    }\n    read_s(ls);",
    "match mk_o() {\n        Some(p) => lv.push(p),\n        None => {}\n    }",
    // `for` (spec 2.4, 3.2): the `Vec` changes after the loop or through
    // a range's index, a temporary's elements are given away, and a
    // `break` leaves the loop.
    "for p in lv {\n        read_p(p);\n    }\n    lv.push(mk_p());",
    "for i in 0..lv.len() {\n        lv[i].bump();\n    }",
    "for p in vec![mk_p()] {\n        lv.push(p);\n    }",
    "for p in mv {\n        if p.n == 1 {\n            break;\n        }\n        continue;\n    }\n    mv.push(mk_p());",
    // A mutable alias made through another one reborrows it: the first
    // may change once the second is no longer used.
    "let mut a = lv[0];\n    let mut s = a.s;\n    s = \"x\";\n    a.bump();",
    // A part of a `match` or `for` binding over a place assigned to an
    // outer text `let`: the place outlives the arm or loop.
    "match le {\n        E::B(p) => {\n            ll = p.s;\n        }\n        _ => {}\n    }\n    read_s(ll);",
    "match lo {\n        Some(p) => {\n            ll = p.s;\n        }\n        None => {}\n    }\n    read_s(ll);",
    "match lo {\n        Some(p) => {\n            let t = p.s;\n            ll = t;\n        }\n        None => {}\n    }\n    read_s(ll);",
    "for p in lv {\n        ll = p.s;\n    }\n    read_s(ll);",
    "for p in lv {\n        let t = p.s;\n        ll = t;\n    }\n    read_s(ll);",
    "for p in lq.v {\n        ll = p.s;\n    }\n    read_s(ll);",
];

/// Programs `check` must reject, each with the code it must reject them
/// with: rustc rejects them too, so accepting one is unsound.
const MUST_REJECT: &[(&str, &str)] = &[
    // The root of an alias changed or given away while the alias is still
    // used, or used while a mutable alias is still used (spec 3.1).
    ("let n = mp.s;\n    change_p(mp);\n    read_s(n);", "V0307"),
    ("let mut m = mp;\n    read_p(mp);\n    m = mk_p();", "V0307"),
    (
        "let n = lq.p.s;\n    change_q(lq);\n    read_s(n);",
        "V0307",
    ),
    (
        "let a = lp.s;\n    let b = lp.s;\n    read_s(a);\n    let moved = lp;\n    read_s(b);",
        "V0307",
    ),
    (
        "let n = lq.p.s;\n    lq.p = mk_p();\n    read_s(n);",
        "V0307",
    ),
    ("let n = lp.s;\n    change_p(lp);\n    read_s(n);", "V0307"),
    ("let n = lp.s;\n    sm(n, lp.s);", "V0307"),
    (
        "let n = lp.s;\n    while c {\n        read_s(n);\n        change_p(lp);\n    }",
        "V0307",
    ),
    ("ll = lp.s;\n    change_p(lp);\n    read_s(ll);", "V0307"),
    (
        "ll = lp.s;\n    let z = ll;\n    change_p(lp);\n    read_s(z);",
        "V0307",
    ),
    (
        "let mut y = \"b\";\n    ll = lp.s;\n    y = ll;\n    change_p(lp);\n    read_s(y);",
        "V0307",
    ),
    // A read-only alias made through a mutable alias keeps it in use: a
    // `let`, a `match` binding, or a loop over it reborrows the `&mut`.
    (
        "let mut a = mv[0];\n    let s = a.s;\n    println!(\"{} {}\", mv.len(), s);",
        "V0307",
    ),
    (
        "let mut a = mv[0];\n    let s = a.s;\n    let t = s;\n    println!(\"{} {}\", mv.len(), t);",
        "V0307",
    ),
    (
        "let mut a = lve[0];\n    match a {\n        E::A(s) => println!(\"{} {}\", lve.len(), s),\n        _ => {}\n    }",
        "V0307",
    ),
    (
        "let mut d = vec![vec![1]];\n    let mut row = d[0];\n    for t in row {\n        println!(\"{} {}\", t, d.len());\n    }",
        "V0307",
    ),
    (
        "let mut d = vec![mk_vs()];\n    let mut row = d[0];\n    for t in row {\n        read_s(t);\n        println!(\"{}\", d.len());\n    }",
        "V0307",
    ),
    // Borrowed places into the owned slots of milestone 2 (spec 3.4).
    ("let e = E::A(ps);", "V0304"),
    ("let e = E::B(lq.p);", "V0304"),
    ("let o = Some(pp);", "V0304"),
    ("let v = vec![lp.s];", "V0304"),
    // Owned locals move into them.
    ("let e = E::B(lp);\n    read_p(lp);", "V0305"),
    ("let v = vec![ls, ls];", "V0305"),
    ("let o = Some(le);\n    read_e(le);", "V0305"),
    // Giving away an alias's root into one.
    (
        "let n = lp.s;\n    let o = Some(lp);\n    read_s(n);",
        "V0307",
    ),
    // `push` keeps its argument; an element stays in its `Vec` (spec 3.4).
    ("lv.push(lp);\n    read_p(lp);", "V0305"),
    ("lv.push(pp);", "V0304"),
    ("lw.push(lw[0]);", "V0304"),
    ("let e = E::A(lw[0]);", "V0304"),
    ("lp.bump();\n    let q = lp;\n    lp.bump();", "V0305"),
    // A changing receiver needs a mutable place (spec 2.5).
    ("pp.bump();", "V0303"),
    // An element alias used after its `Vec` changed (spec 3.1).
    (
        "let first = lv[0];\n    lv[0].bump();\n    read_p(first);",
        "V0307",
    ),
    (
        "let first = lv[0];\n    lv.push(mk_p());\n    read_p(first);",
        "V0307",
    ),
    (
        "let t = lw[0];\n    let o = lw.pop();\n    read_s(t);",
        "V0307",
    ),
    (
        "let mut x = lv[0];\n    read_p(lv[0]);\n    x.bump();",
        "V0307",
    ),
    // An index that uses the `Vec` whose element is changed (E0502).
    ("lv[lv.len() - 1].bump();", "V0306"),
    ("lw[lw.len() - 1] = mk_s();", "V0306"),
    // An index that changes the `Vec` whose element is read (E0502).
    ("let x = lv[pick(lv)];\n    read_p(x);", "V0306"),
    ("println!(\"{}\", lq.v[lq.at()].n);", "V0306"),
    // `match` on a place: its bindings stay inside it and are read-only,
    // and its root cannot change while one is used (spec 3.1, 3.2, 3.4).
    (
        "match lo {\n        Some(p) => lv.push(p),\n        None => {}\n    }",
        "V0304",
    ),
    (
        "match le {\n        E::A(s) => {\n            le = mk_e();\n            read_s(s);\n        }\n        _ => {}\n    }",
        "V0307",
    ),
    (
        "match le {\n        E::B(p) => {\n            let q = le;\n            read_p(p);\n        }\n        _ => {}\n    }",
        "V0307",
    ),
    (
        "match lve[0] {\n        E::A(s) => {\n            lve.push(mk_e());\n            read_s(s);\n        }\n        _ => {}\n    }",
        "V0307",
    ),
    (
        "match me {\n        E::B(p) => change_p(p),\n        _ => {}\n    }",
        "V0303",
    ),
    (
        "let x = lo;\n    match lo {\n        _ => {}\n    }",
        "V0305",
    ),
    // `for` over a place holds it for the whole loop, used or not, and its
    // variable stays inside it and is read-only (spec 3.1, 3.2).
    ("for p in lv {\n        lv.push(mk_p());\n    }", "V0307"),
    ("for n in ln {\n        ln.push(1);\n    }", "V0307"),
    ("for p in mv {\n        mv[0].bump();\n    }", "V0307"),
    ("for s in lw {\n        lw = mk_vs();\n    }", "V0307"),
    ("for p in pv {\n        lv.push(p);\n    }", "V0304"),
    ("for p in lv {\n        p.bump();\n    }", "V0303"),
    ("let y = lv;\n    for p in lv {}", "V0305"),
    (
        "for p in vec![mk_p()] {\n        lv.push(p);\n        read_p(p);\n    }",
        "V0305",
    ),
    // A part of a `match` or `for` binding over a temporary assigned to
    // an outer text `let`: the binding is gone once its arm or loop ends.
    (
        "match mk_e() {\n        E::B(p) => {\n            ll = p.s;\n        }\n        _ => {}\n    }\n    read_s(ll);",
        "V0304",
    ),
    (
        "match mk_e() {\n        E::B(p) => {\n            let t = p.s;\n            ll = t;\n        }\n        _ => {}\n    }\n    read_s(ll);",
        "V0304",
    ),
    (
        "match mk_o() {\n        Some(p) => {\n            ll = p.s;\n        }\n        None => {}\n    }\n    read_s(ll);",
        "V0304",
    ),
    (
        "match mk_o() {\n        Some(p) => {\n            let t = p.s;\n            ll = t;\n        }\n        None => {}\n    }\n    read_s(ll);",
        "V0304",
    ),
    (
        "for p in vec![mk_p()] {\n        ll = p.s;\n    }\n    read_s(ll);",
        "V0304",
    ),
    (
        "for p in vec![mk_p()] {\n        let t = p.s;\n        ll = t;\n    }\n    read_s(ll);",
        "V0304",
    ),
    (
        "match mk_ov() {\n        Some(v) => {\n            ll = v[0].s;\n        }\n        None => {}\n    }\n    read_s(ll);",
        "V0304",
    ),
    (
        "match mk_ov() {\n        Some(v) => {\n            let t = v[0].s;\n            ll = t;\n        }\n        None => {}\n    }\n    read_s(ll);",
        "V0304",
    ),
    // An element of a `Vec` gone after an `if`, block, or `match` read in
    // place cannot be moved out of it (E0507).
    ("read_s(if c { mk_vs()[0] } else { \"z\" });", "V0304"),
    ("read_s({ let a = mk_vs(); a[0] });", "V0304"),
    (
        "println!(\"{}\", match mk_o2() { Some(v) => v[0], None => \"z\" });",
        "V0304",
    ),
    ("read_p(if c { mk_q().v[0] } else { mk_p() });", "V0304"),
    ("if c { mk_q().v[0] } else { mk_p() };", "V0304"),
    (
        "let n = (match mk_ov() { Some(v) => v[0], None => mk_p() }).s;",
        "V0304",
    ),
    // A name of a unit variant of the value's own enum is the variant in
    // Rust, not a binding (E0170).
    (
        "match le {\n        E::A(s) => {}\n        C => {}\n    }",
        "V0103",
    ),
    // `?` needs a function returning a `Result` (spec 2.8).
    ("let x = mk_r()?;", "V0206"),
];

/// The [`MUST_REJECT`] cases rustc accepts in the Rust the backend makes
/// of them: Varyk is stricter than Rust there (no moving a field out of a
/// local, spec 3.4), or the backend makes the text an owned `String`
/// rather than generate a type error. Rejecting them is Varyk's rule, not
/// soundness.
const STRICTER_THAN_RUST: &[&str] = &[
    "let e = E::A(ps);",
    "let e = E::B(lq.p);",
    "let v = vec![lp.s];",
];

/// Programs using `?` that `check` must reject, in a function returning
/// [`RESULT_RET`]: its operand is an owned slot (spec 3.4).
const MUST_REJECT_QUESTION: &[(&str, &str)] = &[
    ("let x = pr?;\n    Ok(x.n)", "V0304"),
    ("let x = lrv[0]?;\n    Ok(x.n)", "V0304"),
    ("let a = lrv[0];\n    let x = a?;\n    Ok(x.n)", "V0304"),
    (
        "for r in lrv {\n        let x = r?;\n    }\n    Ok(1)",
        "V0304",
    ),
    ("let x = lr?;\n    let y = lr?;\n    Ok(x.n)", "V0305"),
    ("while c {\n        let x = lr?;\n    }\n    Ok(1)", "V0305"),
    ("let x = lr?;\n    read_p(lr?);\n    Ok(x.n)", "V0305"),
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

/// This test binary's scratch directory, cleared once per run before any
/// test writes into it (not per directory: `programs_that_break_a_rule_
/// fail_check` and the `emit` closure of `programs_that_break_a_rule_
/// fail_rustc` run concurrently and write the same per-case directory
/// names, which is harmless only because both always write the same
/// bytes there — a clear on every call would instead race one test's
/// write against another's delete). A file an earlier `cargo test` run
/// left behind (a stale case, a stale `ext.rs`) could otherwise linger
/// and hide a broken harness.
fn scratch_root() -> &'static Path {
    static ROOT: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    ROOT.get_or_init(|| {
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("soundness");
        match fs::remove_dir_all(&dir) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => panic!("remove {}: {e}", dir.display()),
        }
        dir
    })
}

/// Writes a program made of `functions` into its own directory under
/// this test's scratch directory and returns the entry path.
fn write_program(dir: &str, functions: &[String]) -> PathBuf {
    let dir = scratch_root().join(dir);
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
    assert!(cases.len() <= 220, "keep this test small: {}", cases.len());

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

#[test]
fn programs_that_break_a_rule_fail_check() {
    let cases = MUST_REJECT
        .iter()
        .map(|(body, code)| (body, code, ""))
        .chain(
            MUST_REJECT_QUESTION
                .iter()
                .map(|(body, code)| (body, code, RESULT_RET)),
        );
    for (index, (body, code, ret)) in cases.enumerate() {
        let entry = write_program(&format!("reject{index:02}"), &[function("t", body, ret)]);
        let (checked, stderr) = run("check", &entry);
        let codes: Vec<&str> = stderr
            .lines()
            .filter_map(|line| line.strip_prefix("error[")?.split(']').next())
            .collect();
        assert!(
            !checked && !codes.is_empty() && codes.iter().all(|c| c == code),
            "expected `check` to reject with {code} only:\n{body}\n{stderr}"
        );
    }
}

/// The case function for case `index`, named uniquely so that all
/// accepted cases fit in one program.
fn function_named(index: usize, body: &str, ret: &str) -> String {
    function(&format!("t{index:02}"), body, ret)
}

#[test]
fn programs_that_break_a_rule_fail_rustc() {
    let dir = scratch_root().join("rustc_rejects");
    let src = dir.join("src").join("bin");
    fs::create_dir_all(&src).expect("create the crate directory");
    fs::write(src.join("ext.rs"), EXT_RS).expect("write ext.rs");
    let mut manifest = "[package]\nname = \"rejects\"\nversion = \"0.0.0\"\nedition = \"2024\"\nautobins = false\n\n[workspace]\n".to_string();
    let mut bins = Vec::new();
    let cases = MUST_REJECT.iter().map(|(body, _)| (body, "")).chain(
        MUST_REJECT_QUESTION
            .iter()
            .map(|(body, _)| (body, RESULT_RET)),
    );
    // Writes the case as bin `name`; false when it does not type-check (a
    // `?` outside a `Result` function has no Rust).
    let mut emit = |name: &str, body: &str, ret: &str| {
        let entry = write_program(name, &[function("t", body, ret)]);
        let text = fs::read_to_string(&entry).expect("read main.vr");
        let mut sources = Vec::new();
        let Ok(program) = resolve(SourceFile::new(FileId(0), &entry, text), &mut sources)
            .and_then(|resolved| typecheck(resolved, &sources))
        else {
            return false;
        };
        let (program, _) = analyze_unchecked(program);
        let generated = RustBackend.generate(&program, "rejects");
        let main = &generated
            .files
            .iter()
            .find(|(path, _)| path == "src/main.rs")
            .expect("a main.rs")
            .1;
        fs::write(src.join(format!("{name}.rs")), main).expect("write the case");
        manifest.push_str(&format!(
            "\n[[bin]]\nname = \"{name}\"\npath = \"src/bin/{name}.rs\"\n"
        ));
        true
    };
    // A case that breaks no rule must build: a failure of the whole crate
    // (a missing `ext.rs`, a broken prelude) fails it too.
    assert!(
        emit("control", "", ""),
        "the control case fails to type-check"
    );
    for (index, (body, ret)) in cases.enumerate() {
        if STRICTER_THAN_RUST.contains(body) {
            continue;
        }
        let name = format!("reject{index:02}");
        if emit(&name, body, ret) {
            bins.push((name, *body));
        }
    }
    assert!(bins.len() >= 50, "too few cases type-check: {}", bins.len());
    fs::write(dir.join("Cargo.toml"), manifest).expect("write Cargo.toml");

    let output = std::process::Command::new("cargo")
        .args(["build", "--keep-going", "--bins", "--message-format=json"])
        .arg("--manifest-path")
        .arg(dir.join("Cargo.toml"))
        .arg("--target-dir")
        .arg(dir.join("target"))
        .output()
        .expect("run cargo");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.lines().any(|line| {
            line.contains("\"reason\":\"compiler-artifact\"")
                && line.contains("\"name\":\"control\",\"src_path\"")
        }),
        "the control case, which breaks no rule, fails to build:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    // Each bin must fail with an error of its own: a failure of the whole
    // crate (a broken manifest, a missing `ext.rs`) names no bin.
    let unrejected: Vec<String> = bins
        .iter()
        .filter(|(name, _)| {
            !stdout.lines().any(|line| {
                line.contains("\"reason\":\"compiler-message\"")
                    && line.contains(&format!("\"name\":\"{name}\",\"src_path\""))
                    && line.contains("\"level\":\"error\"")
            })
        })
        .map(|(name, body)| format!("{name}: {body}"))
        .collect();
    assert!(
        unrejected.is_empty(),
        "must-reject cases without a rustc error of their own:\n{}\n\ncargo stderr:\n{}",
        unrejected.join("\n---\n"),
        String::from_utf8_lossy(&output.stderr)
    );
}
