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
//! single crate, and rustc must reject every one of them. The
//! [`MUST_BUILD_TREES`] and [`MUST_REJECT_TREES`] do the same for
//! programs of several modules.

mod common;

use std::fs;
use std::path::{Path, PathBuf};

use common::{varyk, varyk_in};
use varyk::backend::{Backend, CrateInfo, RustBackend};
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

struct Words {
    w: Vec<string>,
    n: i32,
}

struct W {
    t: ext::Tally,
}

fn mk_words() -> Words {
    Words { w: vec![\"a\"], n: 1 }
}

fn mk_w() -> W {
    W { t: ext::Tally::new() }
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
pub struct Tally { total: i32, pub label: String }
impl Tally {
    pub fn new() -> Tally { Tally { total: 0, label: String::new() } }
    pub fn count(&self) -> i32 { self.total }
    pub fn add(&mut self, n: i32) { self.total += n; }
    pub fn set(&mut self, s: &str) { self.label = s.to_string(); }
}
pub fn keep_tally(t: Tally) -> i32 { t.total }
pub fn bump_tally(t: &mut Tally) { t.total += 1; }
pub fn swap(a: &mut Tally, b: &Tally) { a.total = b.total; }
pub fn add_word(v: &mut Vec<String>) { v.push(String::from(\"w\")); }
pub fn maybe_tally() -> Option<Tally> { Some(Tally::new()) }
pub fn tallies() -> Vec<Tally> { vec![Tally::new()] }
pub enum Kind { Word(String), Number(i32), Tallied(Tally) }
pub fn make_kind(s: &str) -> Kind { Kind::Word(s.to_string()) }
";

/// Every case function's parameters: one of each kind of place.
const PARAMS: &str = "c: bool, ps: string, mut ms: string, pp: P, mut mp: P, pi: i32, mut mi: i32, mut mq: Q, pe: E, mut me: E, pv: Vec<P>, mut mv: Vec<P>, pr: Result<P, string>, pt: ext::Tally";

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
    // An imported struct (M3 spec 4.1, 4.2): a `&mut self` method on a
    // `let mut`, `&self` methods on a `let` and on a `mut` parameter's
    // field of the struct's type, and a `pub` field read and assigned.
    "let mut t = ext::Tally::new();\n    t.add(1);\n    t.label = \"x\";\n    read_i(t.count());",
    "let t = ext::Tally::new();\n    read_i(t.count());\n    read_s(t.label);\n    read_i(ext::keep_tally(t));",
    // An imported enum (M3 spec 4.3): matched as returned by a `.rs`
    // function and as constructed directly in Varyk, moving a payload
    // (owned, whether the scrutinee is a temporary or a place) into a
    // call.
    "match ext::make_kind(\"w\") {\n        ext::Kind::Word(s) => read_s(s),\n        ext::Kind::Number(n) => read_i(n),\n        _ => {}\n    }",
    "let k = ext::Kind::Word(mk_s());\n    match k {\n        ext::Kind::Word(s) => read_s(s),\n        ext::Kind::Number(n) => read_i(n),\n        _ => {}\n    }",
    // A `&mut Vec<String>` parameter of a `.rs` function fed a field; an
    // element of a returned `Vec` of an imported struct and the payload of
    // a returned `Option` of one, used while valid; an imported struct
    // moved, inside the Varyk struct that holds it (M3 spec 4.4).
    "let mut ws = mk_words();\n    ext::add_word(ws.w);\n    read_i(ws.n);",
    "let v = ext::tallies();\n    let first = v[0];\n    read_i(first.count());\n    match ext::maybe_tally() {\n        Some(t) => read_i(t.count()),\n        None => {}\n    }",
    "let w = mk_w();\n    let v = vec![w];\n    read_i(v[0].t.count());",
];

/// Programs `check` must reject, as (body, the code `check` must reject
/// it with, the code rustc rejects the Rust the backend makes of it
/// with): accepting one is unsound. The rustc code is empty for a case
/// in [`STRICTER_THAN_RUST`] or [`UNEMITTED`].
const MUST_REJECT: &[(&str, &str, &str)] = &[
    // The root of an alias changed or given away while the alias is still
    // used, or used while a mutable alias is still used (spec 3.1).
    (
        "let n = mp.s;\n    change_p(mp);\n    read_s(n);",
        "V0307",
        "E0502",
    ),
    (
        "let mut m = mp;\n    read_p(mp);\n    m = mk_p();",
        "V0307",
        "E0502",
    ),
    (
        "let n = lq.p.s;\n    change_q(lq);\n    read_s(n);",
        "V0307",
        "E0502",
    ),
    (
        "let a = lp.s;\n    let b = lp.s;\n    read_s(a);\n    let moved = lp;\n    read_s(b);",
        "V0307",
        "E0505",
    ),
    (
        "let n = lq.p.s;\n    lq.p = mk_p();\n    read_s(n);",
        "V0307",
        "E0506",
    ),
    (
        "let n = lp.s;\n    change_p(lp);\n    read_s(n);",
        "V0307",
        "E0502",
    ),
    ("let n = lp.s;\n    sm(n, lp.s);", "V0307", "E0502"),
    (
        "let n = lp.s;\n    while c {\n        read_s(n);\n        change_p(lp);\n    }",
        "V0307",
        "E0502",
    ),
    (
        "ll = lp.s;\n    change_p(lp);\n    read_s(ll);",
        "V0307",
        "E0502",
    ),
    (
        "ll = lp.s;\n    let z = ll;\n    change_p(lp);\n    read_s(z);",
        "V0307",
        "E0502",
    ),
    (
        "let mut y = \"b\";\n    ll = lp.s;\n    y = ll;\n    change_p(lp);\n    read_s(y);",
        "V0307",
        "E0502",
    ),
    // A read-only alias made through a mutable alias keeps it in use: a
    // `let`, a `match` binding, or a loop over it reborrows the `&mut`.
    (
        "let mut a = mv[0];\n    let s = a.s;\n    println!(\"{} {}\", mv.len(), s);",
        "V0307",
        "E0502",
    ),
    (
        "let mut a = mv[0];\n    let s = a.s;\n    let t = s;\n    println!(\"{} {}\", mv.len(), t);",
        "V0307",
        "E0502",
    ),
    (
        "let mut a = lve[0];\n    match a {\n        E::A(s) => println!(\"{} {}\", lve.len(), s),\n        _ => {}\n    }",
        "V0307",
        "E0502",
    ),
    (
        "let mut d = vec![vec![1]];\n    let mut row = d[0];\n    for t in row {\n        println!(\"{} {}\", t, d.len());\n    }",
        "V0307",
        "E0502",
    ),
    (
        "let mut d = vec![mk_vs()];\n    let mut row = d[0];\n    for t in row {\n        read_s(t);\n        println!(\"{}\", d.len());\n    }",
        "V0307",
        "E0502",
    ),
    // Borrowed places into the owned slots of milestone 2 (spec 3.4).
    ("let e = E::A(ps);", "V0304", ""),
    ("let e = E::B(lq.p);", "V0304", ""),
    ("let o = Some(pp);", "V0304", "E0308"),
    ("let v = vec![lp.s];", "V0304", ""),
    // Owned locals move into them.
    ("let e = E::B(lp);\n    read_p(lp);", "V0305", "E0382"),
    ("let v = vec![ls, ls];", "V0305", "E0382"),
    ("let o = Some(le);\n    read_e(le);", "V0305", "E0382"),
    // Giving away an alias's root into one.
    (
        "let n = lp.s;\n    let o = Some(lp);\n    read_s(n);",
        "V0307",
        "E0505",
    ),
    // `push` keeps its argument; an element stays in its `Vec` (spec 3.4).
    ("lv.push(lp);\n    read_p(lp);", "V0305", "E0382"),
    ("lv.push(pp);", "V0304", "E0308"),
    ("lw.push(lw[0]);", "V0304", "E0507"),
    ("let e = E::A(lw[0]);", "V0304", "E0507"),
    (
        "lp.bump();\n    let q = lp;\n    lp.bump();",
        "V0305",
        "E0382",
    ),
    // A changing receiver needs a mutable place (spec 2.5).
    ("pp.bump();", "V0303", "E0596"),
    // An element alias used after its `Vec` changed (spec 3.1).
    (
        "let first = lv[0];\n    lv[0].bump();\n    read_p(first);",
        "V0307",
        "E0502",
    ),
    (
        "let first = lv[0];\n    lv.push(mk_p());\n    read_p(first);",
        "V0307",
        "E0502",
    ),
    (
        "let t = lw[0];\n    let o = lw.pop();\n    read_s(t);",
        "V0307",
        "E0502",
    ),
    (
        "let mut x = lv[0];\n    read_p(lv[0]);\n    x.bump();",
        "V0307",
        "E0502",
    ),
    // An index that uses the `Vec` whose element is changed (E0502).
    ("lv[lv.len() - 1].bump();", "V0306", "E0502"),
    ("lw[lw.len() - 1] = mk_s();", "V0306", "E0502"),
    // An index that changes the `Vec` whose element is read (E0502).
    ("let x = lv[pick(lv)];\n    read_p(x);", "V0306", "E0502"),
    ("println!(\"{}\", lq.v[lq.at()].n);", "V0306", "E0502"),
    // `match` on a place: its bindings stay inside it and are read-only,
    // and its root cannot change while one is used (spec 3.1, 3.2, 3.4).
    (
        "match lo {\n        Some(p) => lv.push(p),\n        None => {}\n    }",
        "V0304",
        "E0308",
    ),
    (
        "match le {\n        E::A(s) => {\n            le = mk_e();\n            read_s(s);\n        }\n        _ => {}\n    }",
        "V0307",
        "E0506",
    ),
    (
        "match le {\n        E::B(p) => {\n            let q = le;\n            read_p(p);\n        }\n        _ => {}\n    }",
        "V0307",
        "E0505",
    ),
    (
        "match lve[0] {\n        E::A(s) => {\n            lve.push(mk_e());\n            read_s(s);\n        }\n        _ => {}\n    }",
        "V0307",
        "E0502",
    ),
    (
        "match me {\n        E::B(p) => change_p(p),\n        _ => {}\n    }",
        "V0303",
        "E0596",
    ),
    (
        "let x = lo;\n    match lo {\n        _ => {}\n    }",
        "V0305",
        "E0382",
    ),
    // `for` over a place holds it for the whole loop, used or not, and its
    // variable stays inside it and is read-only (spec 3.1, 3.2).
    (
        "for p in lv {\n        lv.push(mk_p());\n    }",
        "V0307",
        "E0502",
    ),
    (
        "for n in ln {\n        ln.push(1);\n    }",
        "V0307",
        "E0502",
    ),
    (
        "for p in mv {\n        mv[0].bump();\n    }",
        "V0307",
        "E0502",
    ),
    (
        "for s in lw {\n        lw = mk_vs();\n    }",
        "V0307",
        "E0506",
    ),
    (
        "for p in pv {\n        lv.push(p);\n    }",
        "V0304",
        "E0308",
    ),
    ("for p in lv {\n        p.bump();\n    }", "V0303", "E0596"),
    ("let y = lv;\n    for p in lv {}", "V0305", "E0382"),
    (
        "for p in vec![mk_p()] {\n        lv.push(p);\n        read_p(p);\n    }",
        "V0305",
        "E0382",
    ),
    // A part of a `match` or `for` binding over a temporary assigned to
    // an outer text `let`: the binding is gone once its arm or loop ends.
    (
        "match mk_e() {\n        E::B(p) => {\n            ll = p.s;\n        }\n        _ => {}\n    }\n    read_s(ll);",
        "V0304",
        "E0597",
    ),
    (
        "match mk_e() {\n        E::B(p) => {\n            let t = p.s;\n            ll = t;\n        }\n        _ => {}\n    }\n    read_s(ll);",
        "V0304",
        "E0597",
    ),
    (
        "match mk_o() {\n        Some(p) => {\n            ll = p.s;\n        }\n        None => {}\n    }\n    read_s(ll);",
        "V0304",
        "E0597",
    ),
    (
        "match mk_o() {\n        Some(p) => {\n            let t = p.s;\n            ll = t;\n        }\n        None => {}\n    }\n    read_s(ll);",
        "V0304",
        "E0597",
    ),
    (
        "for p in vec![mk_p()] {\n        ll = p.s;\n    }\n    read_s(ll);",
        "V0304",
        "E0597",
    ),
    (
        "for p in vec![mk_p()] {\n        let t = p.s;\n        ll = t;\n    }\n    read_s(ll);",
        "V0304",
        "E0597",
    ),
    (
        "match mk_ov() {\n        Some(v) => {\n            ll = v[0].s;\n        }\n        None => {}\n    }\n    read_s(ll);",
        "V0304",
        "E0597",
    ),
    (
        "match mk_ov() {\n        Some(v) => {\n            let t = v[0].s;\n            ll = t;\n        }\n        None => {}\n    }\n    read_s(ll);",
        "V0304",
        "E0597",
    ),
    // An element of a `Vec` gone after an `if`, block, or `match` read in
    // place cannot be moved out of it (E0507).
    (
        "read_s(if c { mk_vs()[0] } else { \"z\" });",
        "V0304",
        "E0507",
    ),
    ("read_s({ let a = mk_vs(); a[0] });", "V0304", "E0507"),
    (
        "println!(\"{}\", match mk_o2() { Some(v) => v[0], None => \"z\" });",
        "V0304",
        "E0507",
    ),
    (
        "read_p(if c { mk_q().v[0] } else { mk_p() });",
        "V0304",
        "E0507",
    ),
    ("if c { mk_q().v[0] } else { mk_p() };", "V0304", "E0507"),
    (
        "let n = (match mk_ov() { Some(v) => v[0], None => mk_p() }).s;",
        "V0304",
        "E0507",
    ),
    // A name of a unit variant of the value's own enum is the variant in
    // Rust, not a binding (E0170).
    (
        "match le {\n        E::A(s) => {}\n        C => {}\n    }",
        "V0103",
        "",
    ),
    // `?` needs a function returning a `Result` (spec 2.8).
    ("let x = mk_r()?;", "V0206", ""),
    // An imported struct's `&mut self` method needs a `let mut`, and one
    // given away by value is moved (M3 spec 4.2, 4.5).
    (
        "let t = ext::Tally::new();\n    t.add(1);",
        "V0302",
        "E0596",
    ),
    (
        "let t = ext::Tally::new();\n    read_i(ext::keep_tally(t));\n    read_i(t.count());",
        "V0305",
        "E0382",
    ),
    // An imported enum's variant holding an imported struct: matching a
    // temporary binds the struct payload only for the arm, so its
    // (imported) string field cannot be stored in an outer owned slot,
    // same as a Varyk-declared enum's struct payload (M3 spec 4.3).
    (
        "match ext::Kind::Tallied(ext::Tally::new()) {\n        ext::Kind::Tallied(t) => {\n            ll = t.label;\n        }\n        _ => {}\n    }\n    read_s(ll);",
        "V0304",
        "E0597",
    ),
    // An imported struct (M3 spec 4.1): a `pub` field's alias while a
    // `&mut self` method changes the struct, and the field given to a
    // `&mut self` method of the same struct.
    (
        "let mut t = ext::Tally::new();\n    let l = t.label;\n    t.add(1);\n    read_s(l);",
        "V0307",
        "E0502",
    ),
    (
        "let mut t = ext::Tally::new();\n    t.set(t.label);",
        "V0306",
        "E0502",
    ),
    // Its `&mut self` method on a read-only parameter (M3 spec 4.2).
    ("pt.add(1);", "V0303", "E0596"),
    // An imported enum matched as a place (M3 spec 4.3): a payload into an
    // owned slot, and a `&mut self` method on a struct payload.
    (
        "let k = ext::make_kind(\"w\");\n    match k {\n        ext::Kind::Word(s) => {\n            let n = ext::take_string(s);\n        }\n        _ => {}\n    }",
        "V0304",
        "E0308",
    ),
    (
        "let mut k = ext::make_kind(\"w\");\n    match k {\n        ext::Kind::Tallied(t) => t.add(1),\n        _ => {}\n    }",
        "V0303",
        "E0596",
    ),
    // An imported function taking `&mut Tally` (M3 spec 4.4): on a `let`
    // without `mut`, after an alias of its field, and with the same value
    // as its `&Tally` argument.
    (
        "let t = ext::Tally::new();\n    ext::bump_tally(t);",
        "V0302",
        "E0596",
    ),
    (
        "let mut t = ext::Tally::new();\n    let l = t.label;\n    ext::bump_tally(t);\n    read_s(l);",
        "V0307",
        "E0502",
    ),
    (
        "let mut t = ext::Tally::new();\n    ext::swap(t, t);",
        "V0306",
        "E0502",
    ),
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

/// The [`MUST_REJECT`] and [`MUST_REJECT_QUESTION`] cases that have no
/// Rust, because they do not type-check: exactly these, so a case the
/// type checker newly rejects does not silently leave the rustc
/// cross-check.
const UNEMITTED: &[&str] = &[
    "match le {\n        E::A(s) => {}\n        C => {}\n    }",
    "let x = mk_r()?;",
];

/// Programs using `?` that `check` must reject, in a function returning
/// [`RESULT_RET`]: its operand is an owned slot (spec 3.4). As
/// [`MUST_REJECT`].
const MUST_REJECT_QUESTION: &[(&str, &str, &str)] = &[
    ("let x = pr?;\n    Ok(x.n)", "V0304", "E0277"),
    ("let x = lrv[0]?;\n    Ok(x.n)", "V0304", "E0507"),
    (
        "let a = lrv[0];\n    let x = a?;\n    Ok(x.n)",
        "V0304",
        "E0277",
    ),
    (
        "for r in lrv {\n        let x = r?;\n    }\n    Ok(1)",
        "V0304",
        "E0277",
    ),
    (
        "let x = lr?;\n    let y = lr?;\n    Ok(x.n)",
        "V0305",
        "E0382",
    ),
    (
        "while c {\n        let x = lr?;\n    }\n    Ok(1)",
        "V0305",
        "E0382",
    ),
    (
        "let x = lr?;\n    read_p(lr?);\n    Ok(x.n)",
        "V0305",
        "E0382",
    ),
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

/// This test process's scratch directory, `soundness/<pid>`, cleared once
/// per process before any test writes into it, so a file an earlier run
/// left behind (a stale case, a stale `ext.rs`) cannot linger and hide a
/// broken harness. The process id keeps two concurrent `cargo test`
/// processes from deleting each other's files; the directory of an
/// earlier process is removed once that process has ended (or after an
/// hour), and anything else there once it is a day old. Every test writes its
/// own directory names (the rustc cross-checks use an `emit_` prefix),
/// so no two tests ever write the same file.
fn scratch_root() -> &'static Path {
    static ROOT: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    ROOT.get_or_init(|| {
        let parent = Path::new(env!("CARGO_TARGET_TMPDIR")).join("soundness");
        let dir = parent.join(std::process::id().to_string());
        remove_dir(&dir);
        common::sweep(&parent);
        dir
    })
}

/// Removes `dir` and everything in it, if it exists.
fn remove_dir(dir: &Path) {
    match fs::remove_dir_all(dir) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => panic!("remove {}: {e}", dir.display()),
    }
}

/// The cargo target directory of a rustc cross-check, shared by all
/// processes (cargo locks it) so that each run does not rebuild from
/// nothing and leave a copy behind.
fn cargo_target(name: &str) -> PathBuf {
    Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("soundness-target")
        .join(name)
}

/// Writes `files`, each a path relative to the program's directory and
/// its text, into their own directory under this test's scratch
/// directory, and returns the entry path, `main.vr` there. Each file is
/// written to a temporary name and renamed into place, so a reader never
/// sees it half written.
fn write_program(dir: &str, files: &[(&str, &str)]) -> PathBuf {
    let dir = scratch_root().join(dir);
    for (path, text) in files {
        write_file(&dir.join(path), text);
    }
    dir.join("main.vr")
}

/// Writes `text` to `path` through a temporary file and a rename.
fn write_file(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().expect("a file has a directory"))
        .expect("create the case directory");
    let mut temporary = path.as_os_str().to_owned();
    temporary.push(".tmp");
    fs::write(&temporary, text).expect("write a program file");
    fs::rename(&temporary, path).expect("move a program file into place");
}

/// Writes a program made of the prelude, `functions`, and an empty
/// `main`, with `ext.rs` beside it, and returns the entry path.
fn write_cases(dir: &str, functions: &[String]) -> PathBuf {
    let mut source = format!("{PRELUDE}\n");
    for function in functions {
        source.push_str(function);
        source.push('\n');
    }
    source.push_str("fn main() {}\n");
    write_program(dir, &[("main.vr", &source), ("ext.rs", EXT_RS)])
}

/// Runs `varyk <command> <entry>`: whether it succeeded, and its stderr.
fn run(command: &str, entry: &Path) -> (bool, String) {
    let output = varyk(&[command, entry.to_str().expect("utf-8 path")]);
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert_no_crash(
        &output,
        &format!("`varyk {command}` on {}", entry.display()),
    );
    (output.status.success(), stderr)
}

/// Asserts `varyk` (`what`) did not crash: it exits 0 or 1, never with a
/// panic's 101 or by a signal, and prints no panic message.
fn assert_no_crash(output: &std::process::Output, what: &str) {
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        matches!(output.status.code(), Some(0 | 1)) && !stderr.contains("panicked"),
        "{what} crashed ({:?}):\n{stderr}",
        output.status
    );
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
    assert!(cases.len() <= 260, "keep this test small: {}", cases.len());

    let mut accepted = Vec::new();
    for (index, (label, body, ret)) in cases.iter().enumerate() {
        let function = function("t", body, ret);
        let entry = write_cases(&format!("case{index:02}"), &[function]);
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
    let entry = write_cases("all", &functions);
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
        let entry = write_cases(&format!("case{index:02}"), std::slice::from_ref(function));
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

/// Programs spread over nested modules (spec 3.1), as (label, files):
/// each must pass `check` and build.
const MUST_BUILD_TREES: &[(&str, &[(&str, &str)])] = &[
    (
        "imported functions and an enum payload of a file with item macros that define no types",
        &[
            (
                "main.vr",
                "mod ext;\n\nfn main() {\n    println!(\"{}\", ext::bump(2));\n    match ext::make() {\n        ext::Note::Text(s) => println!(\"{}\", s),\n        ext::Note::Empty => {}\n    }\n}\n",
            ),
            (
                "ext.rs",
                "use std::cell::Cell;\n\nthread_local! {\n    static TOTAL: Cell<i32> = Cell::new(0);\n}\n\nmacro_rules! twice {\n    ($e:expr) => {\n        $e * 2\n    };\n}\n\npub enum Note {\n    Text(String),\n    Empty,\n}\n\npub fn make() -> Note {\n    Note::Text(String::from(\"hi\"))\n}\n\npub fn bump(n: i32) -> i32 {\n    TOTAL.with(|t| t.set(t.get() + twice!(n)));\n    TOTAL.with(|t| t.get())\n}\n",
            ),
        ],
    ),
    (
        "an imported enum with a variant behind `#[cfg]`, passed along as an opaque value",
        &[
            (
                "main.vr",
                "mod ext;\n\nfn main() {\n    let e = ext::make();\n    ext::show(e);\n}\n",
            ),
            ("ext.rs", CFG_ENUM_RS),
        ],
    ),
    (
        "a payload taken out of a temporary of an enum whose other file calls macros that never spell `Drop`",
        &[
            ("main.vr", G2_MAIN_VR),
            ("ext.rs", G2_RS),
            (
                "hook.rs",
                "pub fn noise() -> String {\n    println!(\"{}\", vec![1].len());\n    format!(\"{}\", 2)\n}\n",
            ),
        ],
    ),
    (
        "an imported call and field naming a type behind a private module, used where it is seen",
        &[
            (
                "main.vr",
                "mod shop;\n\nfn main() {\n    println!(\"{}\", shop::total());\n}\n",
            ),
            (
                "shop.vr",
                "mod hidden;\npub mod api;\n\npub fn total() -> i32 {\n    let h = api::make();\n    let b = api::boxed();\n    h.x + b.h.x\n}\n",
            ),
            ("shop/api.rs", HIDDEN_API_RS),
            ("shop/hidden.rs", "pub struct H {\n    pub x: i32,\n}\n"),
        ],
    ),
    (
        "a payload of a temporary of an enum beside an enum and a struct with destructors, kept",
        &[
            (
                "main.vr",
                "mod ext;\n\nfn main() {\n    match ext::make_plain() {\n        ext::Plain::Held(s) => ext::keep(s),\n        ext::Plain::Empty => {}\n    }\n}\n",
            ),
            (
                "ext.rs",
                "pub enum Plain {\n    Held(String),\n    Empty,\n}\n\npub enum Guard {\n    Held(String),\n}\n\nimpl Drop for Guard {\n    fn drop(&mut self) {}\n}\n\npub struct Res;\n\nimpl Drop for Res {\n    fn drop(&mut self) {}\n}\n\npub fn make_plain() -> Plain {\n    Plain::Held(String::from(\"hi\"))\n}\n\npub fn keep(s: String) {\n    println!(\"{s}\");\n}\n",
            ),
        ],
    ),
    (
        "a payload of a temporary of a Varyk enum a .rs file gives a destructor, printed",
        &[
            (
                "main.vr",
                "mod ev;\nmod hook;\n\nfn main() {\n    match ev::make() {\n        ev::Ev::Msg(s) => println!(\"{}\", s),\n        ev::Ev::Quit => {}\n    }\n}\n",
            ),
            ("ev.vr", VARYK_EV_VR),
            ("hook.rs", EV_HOOK_RS),
        ],
    ),
    (
        "a payload of a temporary of an imported enum with a destructor, printed",
        &[
            (
                "main.vr",
                "mod res;\n\nfn main() {\n    match res::make() {\n        res::Ev::Msg(s) => println!(\"{}\", s),\n        res::Ev::Quit => res::keep(\"q\"),\n    }\n}\n",
            ),
            ("res.rs", DROP_ENUM_RS),
        ],
    ),
    (
        "an imported function returning a type of a pub module",
        &[
            (
                "main.vr",
                "mod shop;\nmod user;\n\nfn main() {\n    user::run();\n}\n",
            ),
            ("shop.vr", "pub mod facade;\npub mod other;\n"),
            (
                "shop/facade.rs",
                "pub fn list() -> Vec<crate::shop::other::T> {\n    vec![crate::shop::other::T { x: 1 }]\n}\n",
            ),
            ("shop/other.rs", "pub struct T {\n    pub x: i32,\n}\n"),
            (
                "user.vr",
                "pub fn run() {\n    let v = crate::shop::facade::list();\n    println!(\"{}\", v[0].x);\n}\n",
            ),
        ],
    ),
    (
        "a pub mod chain reached by a full path",
        &[
            (
                "main.vr",
                "mod shop;\n\nfn main() {\n    let c = crate::shop::cart::Cart { n: crate::shop::cart::two() };\n    let d = shop::cart::Cart { n: 1 };\n    let k = crate::shop::cart::Kind::Big(c.n + d.n);\n    match k {\n        shop::cart::Kind::Big(n) => println!(\"{}\", n),\n        crate::shop::cart::Kind::Small => {}\n    }\n}\n",
            ),
            ("shop.vr", "pub mod cart;\n"),
            (
                "shop/cart.vr",
                "pub struct Cart {\n    pub n: i32,\n}\n\npub enum Kind {\n    Small,\n    Big(i32),\n}\n\npub fn two() -> i32 {\n    2\n}\n",
            ),
        ],
    ),
    (
        "`super::` from a module of a `mod.vr` directory",
        &[
            (
                "main.vr",
                "mod shop;\n\nfn main() {\n    println!(\"{}\", shop::cart::Cart::new().n);\n}\n",
            ),
            (
                "shop/mod.vr",
                "pub mod cart;\n\npub struct Base {\n    n: i32,\n}\n\npub fn base() -> Base {\n    Base { n: 1 }\n}\n",
            ),
            (
                "shop/cart.vr",
                "pub struct Cart {\n    pub n: i32,\n}\n\nimpl Cart {\n    pub fn new() -> Cart {\n        let b: super::Base = super::base();\n        Cart { n: b.n + self::one() }\n    }\n}\n\nfn one() -> i32 {\n    1\n}\n",
            ),
        ],
    ),
    (
        "a private module and a private item used from inside their module",
        &[
            (
                "main.vr",
                "mod shop;\n\nfn main() {\n    println!(\"{}\", shop::total());\n}\n",
            ),
            (
                "shop.vr",
                "mod cart;\n\nfn secret() -> i32 {\n    2\n}\n\npub fn total() -> i32 {\n    let c = cart::Cart::new();\n    c.n + cart::count()\n}\n",
            ),
            (
                "shop/cart.vr",
                "pub struct Cart {\n    pub n: i32,\n}\n\nimpl Cart {\n    pub fn new() -> Cart {\n        Cart { n: super::secret() }\n    }\n}\n\npub fn count() -> i32 {\n    crate::shop::secret() + self::one()\n}\n\nfn one() -> i32 {\n    1\n}\n",
            ),
        ],
    ),
    (
        "a pub fn returning a type of a pub mod, its value kept in a local",
        &[
            (
                "main.vr",
                "mod shop;\n\nfn main() {\n    let v = shop::carts();\n    println!(\"{}\", v.len());\n}\n",
            ),
            (
                "shop.vr",
                "pub mod cart;\n\npub fn carts() -> Vec<cart::Cart> {\n    vec![cart::Cart { n: 1 }]\n}\n",
            ),
            ("shop/cart.vr", "pub struct Cart {\n    pub n: i32,\n}\n"),
        ],
    ),
    (
        "a pub fn of a module returning a private type of the crate root",
        &[
            (
                "main.vr",
                "mod shop;\n\nstruct Config {\n    n: i32,\n}\n\nfn main() {\n    let c = shop::get();\n    println!(\"{}\", c.n);\n}\n",
            ),
            (
                "shop.vr",
                "pub fn get() -> crate::Config {\n    crate::Config { n: 1 }\n}\n",
            ),
        ],
    ),
    (
        "a pub fn of a private module returning a private type of its parent",
        &[
            (
                "main.vr",
                "mod shop;\n\nfn main() {\n    println!(\"{}\", shop::count());\n}\n",
            ),
            (
                "shop.vr",
                "mod cart;\n\nstruct Base {\n    n: i32,\n}\n\npub fn count() -> i32 {\n    let v = cart::make();\n    v[0].n\n}\n",
            ),
            (
                "shop/cart.vr",
                "pub fn make() -> Vec<super::Base> {\n    vec![super::Base { n: 1 }]\n}\n",
            ),
        ],
    ),
    (
        "a `use` alias for a type and for a function",
        &[
            (
                "main.vr",
                "mod shop;\n\nuse crate::shop::cart::Cart;\nuse crate::shop::helper;\n\nfn main() {\n    let c = Cart { n: helper() };\n    println!(\"{}\", c.n);\n}\n",
            ),
            (
                "shop.vr",
                "pub mod cart;\n\npub fn helper() -> i32 {\n    1\n}\n",
            ),
            ("shop/cart.vr", "pub struct Cart {\n    pub n: i32,\n}\n"),
        ],
    ),
    (
        "`use` declarations of two modules sharing a local name across namespaces",
        &[
            (
                "main.vr",
                "mod a;\nmod b;\n\nuse crate::a::Foo;\nuse crate::b::Foo;\n\nfn main() {\n    let x = Foo { n: 1 };\n    let y = Foo();\n    println!(\"{} {}\", x.n, y);\n}\n",
            ),
            ("a.vr", "pub struct Foo {\n    pub n: i32,\n}\n"),
            ("b.vr", "pub fn Foo() -> i32 {\n    1\n}\n"),
        ],
    ),
    (
        "a pub method of a private type returning a type of a private module",
        &[
            (
                "main.vr",
                "mod shop;\n\nfn main() {\n    println!(\"{}\", shop::count());\n}\n",
            ),
            (
                "shop.vr",
                "mod cart;\n\nstruct Helper {\n    n: i32,\n}\n\nimpl Helper {\n    pub fn make() -> Vec<cart::Cart> {\n        vec![cart::Cart { n: 1 }]\n    }\n}\n\npub fn count() -> usize {\n    let v = Helper::make();\n    v.len()\n}\n",
            ),
            ("shop/cart.vr", "pub struct Cart {\n    pub n: i32,\n}\n"),
        ],
    ),
    (
        "a pub field read across modules (spec 3.4)",
        &[
            (
                "main.vr",
                "mod shop;\n\nfn main() {\n    let c = shop::Cart { n: 1 };\n    println!(\"{}\", c.n);\n}\n",
            ),
            ("shop.vr", "pub struct Cart {\n    pub n: i32,\n}\n"),
        ],
    ),
    (
        "a private field read through the struct's own method (spec 3.4)",
        &[
            (
                "main.vr",
                "mod shop;\n\nfn main() {\n    let c = shop::Cart::new();\n    println!(\"{}\", c.get());\n}\n",
            ),
            (
                "shop.vr",
                "pub struct Cart {\n    n: i32,\n}\n\nimpl Cart {\n    pub fn new() -> Cart {\n        Cart { n: 1 }\n    }\n\n    pub fn get(self) -> i32 {\n        self.n\n    }\n}\n",
            ),
        ],
    ),
    (
        "`use` with `as`, from `self::` and `super::`, and of a private sibling module (spec 3.3)",
        &[
            (
                "main.vr",
                "mod shop;\n\nuse shop::cart::Cart as Basket;\n\nfn main() {\n    let b = Basket { n: shop::total() };\n    println!(\"{}\", b.n);\n}\n",
            ),
            (
                "shop.vr",
                "pub mod cart;\nmod util;\n\nuse self::util::one as unit;\n\npub fn total() -> i32 {\n    unit() + cart::two()\n}\n",
            ),
            (
                "shop/cart.vr",
                "use super::util::one;\n\npub struct Cart {\n    pub n: i32,\n}\n\npub fn two() -> i32 {\n    one() + one()\n}\n",
            ),
            ("shop/util.vr", "pub fn one() -> i32 {\n    1\n}\n"),
        ],
    ),
    (
        "`use` of an imported Rust struct, built and changed through its methods (spec 3.3, 4.1, 4.2)",
        &[
            (
                "main.vr",
                "mod ext;\n\nuse ext::Tally;\n\nfn main() {\n    let mut t = Tally::new();\n    t.add(2);\n    println!(\"{}\", t.count());\n}\n",
            ),
            ("ext.rs", TALLY_RS),
        ],
    ),
    (
        "a facade with `thread_local!`, an expression macro, and an item macro of its own that writes nothing",
        &[
            (
                "main.vr",
                "mod ext;\n\nfn main() {\n    println!(\"{}\", ext::bump(\"ab\"));\n}\n",
            ),
            (
                "ext.rs",
                "use std::cell::Cell;\n\nthread_local! {\n    static TOTAL: Cell<usize> = Cell::new(0);\n}\n\nmacro_rules! twice {\n    ($e:expr) => {\n        $e * 2\n    };\n}\n\nmacro_rules! nothing {\n    () => {};\n}\n\nnothing!();\n\npub fn bump(s: &str) -> String {\n    TOTAL.with(|t| t.set(t.get() + twice!(s.len())));\n    TOTAL.with(|t| t.get()).to_string()\n}\n",
            ),
        ],
    ),
    (
        "a payload taken out of a temporary of an enum whose file calls `println!`, `format!`, `assert!`, and a local expression macro in function bodies",
        &[
            ("main.vr", G2_MAIN_VR),
            ("ext.rs", G2_RS),
            (
                "hook.rs",
                "macro_rules! twice {\n    ($e:expr) => {\n        $e * 2\n    };\n}\n\npub fn noise(n: i32) -> String {\n    assert!(n >= 0, \"{}\", format!(\"{n}\"));\n    println!(\"{}\", twice!(n));\n    format!(\"{}\", vec![n].len())\n}\n",
            ),
        ],
    ),
    (
        "a payload taken out of a temporary of an enum whose file calls `thread_local!`, `cfg!`, `format_args!`, `option_env!`, and `module_path!` in function bodies",
        &[
            ("main.vr", G2_MAIN_VR),
            ("ext.rs", G2_RS),
            (
                "hook.rs",
                "pub fn noise() -> usize {\n    thread_local! {\n        static N: std::cell::Cell<usize> = const { std::cell::Cell::new(1) };\n    }\n    let debug = cfg!(debug_assertions) as usize;\n    let text = std::fmt::format(format_args!(\"{}\", module_path!()));\n    let home = option_env!(\"HOME\").map_or(0, str::len);\n    N.with(|n| n.get()) + debug + text.len() + home\n}\n",
            ),
        ],
    ),
];

#[test]
fn nested_module_programs_build() {
    for (index, (label, files)) in MUST_BUILD_TREES.iter().enumerate() {
        let entry = write_program(&format!("tree{index:02}"), files);
        let (checked, stderr) = run("check", &entry);
        assert!(checked, "a valid program fails check: {label}\n{stderr}");
        let (built, stderr) = run("build", &entry);
        assert!(
            built,
            "a program passes check but fails to build: {label}\n{stderr}"
        );
    }
}

/// Programs that must run and print what the Varyk code says, as (label,
/// files, expected stdout): each would build as Rust but do something
/// else if the generated code let Rust pick another item than the one
/// Varyk checked.
const MUST_RUN_TREES: &[(&str, Files, &str)] = &[
    (
        "an inherent `count` of an imported struct that implements `Iterator`",
        &[
            (
                "main.vr",
                "mod ext;\n\nfn main() {\n    let s = ext::S::new();\n    println!(\"{}\", s.count());\n}\n",
            ),
            (
                "ext.rs",
                "pub struct S {\n    left: u32,\n}\n\nimpl S {\n    pub fn new() -> S {\n        S { left: 3 }\n    }\n\n    pub fn count(&self) -> usize {\n        42\n    }\n}\n\nimpl Iterator for S {\n    type Item = u32;\n\n    fn next(&mut self) -> Option<u32> {\n        if self.left == 0 {\n            return None;\n        }\n        self.left -= 1;\n        Some(self.left)\n    }\n}\n",
            ),
        ],
        "42\n",
    ),
    (
        "an inherent `&mut self` `eq` of an imported struct deriving `PartialEq`",
        &[
            (
                "main.vr",
                "mod ext;\n\nfn main() {\n    let mut a = ext::S::new(1);\n    let b = ext::S::new(1);\n    println!(\"{}\", a.eq(b));\n}\n",
            ),
            (
                "ext.rs",
                "#[derive(PartialEq)]\npub struct S {\n    pub n: i32,\n}\n\nimpl S {\n    pub fn new(n: i32) -> S {\n        S { n }\n    }\n\n    pub fn eq(&mut self, other: &S) -> bool {\n        self.n != other.n\n    }\n}\n",
            ),
        ],
        "false\n",
    ),
    (
        "a Varyk method named `into`, which `Into::into` would shadow",
        &[(
            "main.vr",
            "struct T {\n    n: i32,\n}\n\nimpl T {\n    fn into(self) -> i32 {\n        self.n\n    }\n}\n\nfn main() {\n    let t = T { n: 5 };\n    println!(\"{}\", t.into());\n}\n",
        )],
        "5\n",
    ),
    (
        "a Varyk struct's `count` with a facade implementing `Iterator` for it",
        &[
            (
                "main.vr",
                "mod facade;\n\npub struct Counter {\n    pub left: u32,\n}\n\nimpl Counter {\n    pub fn count(self) -> usize {\n        42\n    }\n}\n\nfn main() {\n    let c = Counter { left: 3 };\n    println!(\"{}\", c.count());\n}\n",
            ),
            (
                "facade.rs",
                "impl Iterator for crate::Counter {\n    type Item = u32;\n\n    fn next(&mut self) -> Option<u32> {\n        if self.left == 0 {\n            return None;\n        }\n        self.left -= 1;\n        Some(self.left)\n    }\n}\n",
            ),
        ],
        "42\n",
    ),
    (
        "std macros beside a `.rs` module exporting macros of the same names",
        &[
            (
                "main.vr",
                "mod ext;\n\nfn main() {\n    let v = vec![1, 2];\n    let s = format!(\"{} items\", v.len());\n    println!(\"{}\", s);\n}\n",
            ),
            (
                "ext.rs",
                "#[macro_export]\nmacro_rules! println {\n    ($($t:tt)*) => {\n        ::std::println!(\"HIJACKED\")\n    };\n}\n\n#[macro_export]\nmacro_rules! vec {\n    ($($t:tt)*) => {\n        ::std::vec::Vec::<i32>::new()\n    };\n}\n\n#[macro_export]\nmacro_rules! format {\n    ($($t:tt)*) => {\n        ::std::string::String::from(\"HIJACKED\")\n    };\n}\n",
            ),
        ],
        "2 items\n",
    ),
];

#[test]
fn programs_run_as_written() {
    for (index, (label, files, expected)) in MUST_RUN_TREES.iter().enumerate() {
        let entry = write_program(&format!("run{index:02}"), files);
        let output = varyk(&["run", entry.to_str().expect("utf-8 path")]);
        assert_no_crash(&output, &format!("`varyk run` on {label}"));
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            output.status.success() && stdout == *expected,
            "expected {expected:?}, got {stdout:?}: {label}\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

/// A library package (M3 spec 2.2) whose public functions name a type
/// through a chain of `pub mod`s: it must pass `check` and build as a
/// library, since a user of the library names the type the same way.
#[test]
fn a_library_with_a_pub_mod_chain_builds() {
    let entry = write_program(
        "lib_tree",
        &[
            (
                "Cargo.toml",
                "[package]\nname = \"shop\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
            ),
            (
                "src/lib.vr",
                "pub mod store;\n\npub fn total() -> i32 {\n    store::cart::make().n\n}\n\npub fn fresh() -> store::cart::Cart {\n    store::cart::make()\n}\n",
            ),
            ("src/store.vr", "pub mod cart;\n"),
            (
                "src/store/cart.vr",
                "pub struct Cart {\n    pub n: i32,\n}\n\npub fn make() -> Cart {\n    Cart { n: 1 }\n}\n",
            ),
        ],
    );
    let dir = entry.parent().expect("the package directory");
    for command in ["check", "build"] {
        let output = varyk_in(dir, &[command]);
        assert_no_crash(&output, &format!("`varyk {command}` on a library"));
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            output.status.success(),
            "`varyk {command}` fails:\n{stderr}"
        );
    }
    assert!(dir.join("target/varyk/shop/src/lib.rs").is_file());
}

/// A payload of a stored enum with a destructor cannot be taken out by
/// matching on the call either, so V0304 advises `.clone()` instead.
#[test]
fn a_drop_enum_payload_kept_advises_clone() {
    let entry = write_program(
        "drop_enum_note",
        &[
            (
                "main.vr",
                "mod ev;\nmod hook;\n\nstruct Holder {\n    s: string,\n}\n\nfn main() {\n    let e = ev::make();\n    match e {\n        ev::Ev::Msg(s) => {\n            let h = Holder { s: s };\n            println!(\"{}\", h.s);\n        }\n        ev::Ev::Quit => {}\n    }\n}\n",
            ),
            ("ev.vr", VARYK_EV_VR),
            ("hook.rs", EV_HOOK_RS),
        ],
    );
    let (checked, stderr) = run("check", &entry);
    assert!(!checked && stderr.contains("error[V0304]"), "{stderr}");
    assert!(
        stderr.contains("copy it with `.clone()` to keep it")
            && !stderr.contains("the call that made"),
        "{stderr}"
    );
}

/// A `use` aliasing something to a built-in type's name (spec 2.1) must
/// be rejected (V0103), the same as declaring a struct, enum, or fn with
/// that name would be: the alias is a declaration too, and the generated
/// Rust would otherwise shadow the real `Vec` for the rest of the file.
/// Plain Rust does not reject this in isolation (renaming an import to
/// `Vec` is legal syntax on its own; only a later use of the bare `Vec`
/// prelude type in the same file would break, e.g. `Vec::new()` no longer
/// naming the standard type), so this is not run through
/// `trees_that_break_a_rule_fail_rustc`'s privacy-only rustc cross-check.
#[test]
fn use_alias_reusing_a_built_in_type_name_is_v0103_and_rejected() {
    let entry = write_program(
        "use_reserved_alias",
        &[
            (
                "main.vr",
                "mod shop;\n\nuse crate::shop::Cart as Vec;\n\nfn main() {\n    let c = Vec { n: 1 };\n    println!(\"{}\", c.n);\n}\n",
            ),
            ("shop.vr", "pub struct Cart {\n    pub n: i32,\n}\n"),
        ],
    );
    let (checked, stderr) = run("check", &entry);
    assert!(!checked, "expected `check` to reject this: {stderr}");
    assert!(
        stderr.contains("error[V0103]"),
        "expected a V0103 diagnostic: {stderr}"
    );
}

/// A program's files, each as (path relative to `main.vr`'s directory,
/// text).
type Files = &'static [(&'static str, &'static str)];

/// Programs spread over nested modules that `check` must reject, as
/// (label, expected code, files, rustc's own code for the same rejection).
/// They are written in the part of Varyk that is also Rust, so
/// [`trees_that_break_a_rule_fail_rustc`] builds each unchanged, `.vr`
/// renamed to `.rs`, and rustc must reject it too with that code, except
/// for those in [`STRICTER_THAN_RUST_TREES`], which it must build. A
/// private module or item is rustc's E0603, a private method or
/// associated function E0624; a private field (spec 3.4),
/// Varyk-declared or imported (spec 4.1), is E0616 read or assigned as a
/// place and E0451 named in a literal.
const MUST_REJECT_TREES: &[(&str, &str, Files, &str)] = &[
    (
        "an imported call naming a type behind a private module, from outside that module",
        "V0108",
        &[
            (
                "main.vr",
                "mod shop;\n\nfn main() {\n    let h = shop::api::make();\n    println!(\"{}\", h.x);\n}\n",
            ),
            ("shop.vr", "mod hidden;\npub mod api;\n"),
            ("shop/api.rs", HIDDEN_API_RS),
            ("shop/hidden.rs", "pub struct H {\n    pub x: i32,\n}\n"),
        ],
        "E0603",
    ),
    (
        "a pub function behind a private module",
        "V0105",
        &[
            (
                "main.vr",
                "mod shop;\n\nfn main() {\n    println!(\"{}\", shop::cart::count());\n}\n",
            ),
            ("shop.vr", "mod cart;\n"),
            ("shop/cart.vr", "pub fn count() -> i32 {\n    1\n}\n"),
        ],
        "E0603",
    ),
    (
        "a struct literal and a variant behind a private module",
        "V0105",
        &[
            (
                "main.vr",
                "mod shop;\n\nfn main() {\n    let c = shop::cart::Cart { n: 1 };\n    match shop::cart::pick() {\n        shop::cart::Kind::Big => println!(\"{}\", c.n),\n    }\n}\n",
            ),
            ("shop.vr", "mod cart;\n"),
            (
                "shop/cart.vr",
                "pub struct Cart {\n    pub n: i32,\n}\n\npub enum Kind {\n    Big,\n}\n\npub fn pick() -> Kind {\n    Kind::Big\n}\n",
            ),
        ],
        "E0603",
    ),
    (
        "a private function used from a sibling module",
        "V0105",
        &[
            (
                "main.vr",
                "mod shop;\nmod other;\n\nfn main() {\n    other::run();\n}\n",
            ),
            ("shop.vr", "fn secret() {}\n"),
            (
                "other.vr",
                "pub fn run() {\n    crate::shop::secret();\n}\n",
            ),
        ],
        "E0603",
    ),
    (
        "a private field read from a sibling module",
        "V0105",
        &[
            (
                "main.vr",
                "mod shop;\nmod other;\n\nfn main() {\n    other::run();\n}\n",
            ),
            (
                "shop.vr",
                "pub struct Cart {\n    n: i32,\n}\n\npub fn mk() -> Cart {\n    Cart { n: 1 }\n}\n",
            ),
            (
                "other.vr",
                "pub fn run() {\n    let c = crate::shop::mk();\n    println!(\"{}\", c.n);\n}\n",
            ),
        ],
        "E0616",
    ),
    (
        "a struct literal naming a private field from a sibling module",
        "V0105",
        &[
            (
                "main.vr",
                "mod shop;\nmod other;\n\nfn main() {\n    other::run();\n}\n",
            ),
            ("shop.vr", "pub struct Cart {\n    n: i32,\n}\n"),
            (
                "other.vr",
                "pub fn run() {\n    let c = crate::shop::Cart { n: 1 };\n}\n",
            ),
        ],
        "E0451",
    ),
    (
        "a pub function returning a type its callers cannot name",
        "V0105",
        &[
            (
                "main.vr",
                "mod shop;\n\nfn main() {\n    let v = shop::carts();\n    println!(\"{}\", v.len());\n}\n",
            ),
            (
                "shop.vr",
                "mod cart;\n\npub fn carts() -> Vec<cart::Cart> {\n    vec![cart::Cart { n: 1 }]\n}\n",
            ),
            ("shop/cart.vr", "pub struct Cart {\n    pub n: i32,\n}\n"),
        ],
        "E0603",
    ),
    (
        "an imported struct's `&mut self` method on a `let`",
        "V0302",
        &[
            (
                "main.vr",
                "mod ext;\n\nfn main() {\n    let t = ext::Tally::new();\n    t.add(1);\n}\n",
            ),
            ("ext.rs", TALLY_RS),
        ],
        "E0596",
    ),
    (
        "an imported struct used after it was given away",
        "V0305",
        &[
            (
                "main.vr",
                "mod ext;\n\nfn main() {\n    let t = ext::Tally::new();\n    let n = ext::keep_tally(t);\n    println!(\"{}\", t.count());\n}\n",
            ),
            ("ext.rs", TALLY_RS),
        ],
        "E0382",
    ),
    (
        "a `use` of a private function of another module",
        "V0105",
        &[
            (
                "main.vr",
                "mod shop;\n\nuse crate::shop::secret;\n\nfn main() {\n    secret();\n}\n",
            ),
            ("shop.vr", "fn secret() {}\n"),
        ],
        "E0603",
    ),
    (
        "a private field assigned from a sibling module",
        "V0105",
        &[
            (
                "main.vr",
                "mod shop;\nmod other;\n\nfn main() {\n    other::run();\n}\n",
            ),
            (
                "shop.vr",
                "pub struct Cart {\n    n: i32,\n}\n\npub fn mk() -> Cart {\n    Cart { n: 1 }\n}\n",
            ),
            (
                "other.vr",
                "pub fn run() {\n    let mut c = crate::shop::mk();\n    c.n = 2;\n}\n",
            ),
        ],
        "E0616",
    ),
    (
        "a private field of an imported struct read from Varyk",
        "V0105",
        &[
            (
                "main.vr",
                "mod ext;\n\nfn main() {\n    let t = ext::Tally::new();\n    println!(\"{}\", t.total);\n}\n",
            ),
            ("ext.rs", TALLY_RS),
        ],
        "E0616",
    ),
    (
        "a literal of an imported struct with a private field",
        "V0105",
        &[
            (
                "main.vr",
                "mod ext;\n\nfn main() {\n    let t = ext::Tally { total: 1 };\n    println!(\"{}\", t.count());\n}\n",
            ),
            ("ext.rs", TALLY_RS),
        ],
        "E0451",
    ),
    (
        "an imported function returning a type behind a private module",
        "V0108",
        &[
            (
                "main.vr",
                "mod shop;\nmod user;\n\nfn main() {\n    user::run();\n}\n",
            ),
            ("shop.vr", "pub mod facade;\nmod other;\n"),
            (
                "shop/facade.rs",
                "pub fn list() -> Vec<crate::shop::other::T> {\n    vec![crate::shop::other::T { x: 1 }]\n}\n",
            ),
            ("shop/other.rs", "pub struct T {\n    pub x: i32,\n}\n"),
            (
                "user.vr",
                "pub fn run() {\n    let v = crate::shop::facade::list();\n    println!(\"{}\", v.len());\n}\n",
            ),
        ],
        "E0603",
    ),
    (
        "an imported field of a type behind a private module",
        "V0108",
        &[
            (
                "main.vr",
                "mod shop;\nmod user;\n\nfn main() {\n    user::run();\n}\n",
            ),
            ("shop.vr", "pub mod facade;\nmod other;\n"),
            (
                "shop/facade.rs",
                "pub struct Holder {\n    pub t: crate::shop::other::T,\n}\n\npub fn make() -> Holder {\n    Holder { t: crate::shop::other::T { x: 1 } }\n}\n",
            ),
            ("shop/other.rs", "pub struct T {\n    pub x: i32,\n}\n"),
            (
                "user.vr",
                "pub fn run() {\n    let h = crate::shop::facade::make();\n    let t = h.t;\n    println!(\"{}\", t.x);\n}\n",
            ),
        ],
        "E0603",
    ),
    (
        "an imported enum with a payload of a type behind a private module",
        "V0100",
        &[
            (
                "main.vr",
                "mod shop;\nmod user;\n\nfn main() {\n    user::run();\n}\n",
            ),
            ("shop.vr", "pub mod facade;\nmod other;\n"),
            (
                "shop/facade.rs",
                "pub enum Ev {\n    Got(crate::shop::other::T),\n    Quit,\n}\n\npub fn make() -> Ev {\n    Ev::Got(crate::shop::other::T { x: 1 })\n}\n",
            ),
            ("shop/other.rs", "pub struct T {\n    pub x: i32,\n}\n"),
            (
                "user.vr",
                "pub fn run() {\n    match crate::shop::facade::make() {\n        crate::shop::facade::Ev::Got(t) => println!(\"{}\", t.x),\n        crate::shop::facade::Ev::Quit => {}\n    }\n}\n",
            ),
        ],
        "E0603",
    ),
    (
        "a use of a function and struct of one name beside a local struct",
        "V0103",
        &[
            (
                "main.vr",
                "mod a;\n\nuse a::T;\n\nstruct T {\n    n: i32,\n}\n\nfn main() {\n    let t = T { n: T() };\n    println!(\"{}\", t.n);\n}\n",
            ),
            (
                "a.vr",
                "pub fn T() -> i32 {\n    1\n}\n\npub struct T {\n    pub n: i32,\n}\n",
            ),
        ],
        "E0255",
    ),
    (
        "a payload taken out of a temporary of an imported enum with a destructor",
        "V0304",
        &[
            (
                "main.vr",
                "mod res;\n\nfn main() {\n    match res::make() {\n        res::Ev::Msg(s) => res::keep(s),\n        res::Ev::Quit => {}\n    }\n}\n",
            ),
            ("res.rs", DROP_ENUM_RS),
        ],
        "E0509",
    ),
    (
        "a payload taken out of a temporary of an enum given a destructor in another file after `use crate::ext::G2`",
        "V0304",
        &[
            ("main.vr", G2_MAIN_VR),
            ("ext.rs", G2_RS),
            (
                "hook.rs",
                "use crate::ext::G2;\n\nimpl Drop for G2 {\n    fn drop(&mut self) {}\n}\n",
            ),
        ],
        "E0509",
    ),
    (
        "a payload taken out of a temporary of an enum given a destructor under a `use ... as` name",
        "V0304",
        &[
            ("main.vr", G2_MAIN_VR),
            ("ext.rs", G2_RS),
            (
                "hook.rs",
                "use crate::ext::G2 as Other;\n\nimpl Drop for Other {\n    fn drop(&mut self) {}\n}\n",
            ),
        ],
        "E0509",
    ),
    (
        "a payload taken out of a temporary of an enum given a destructor under a name another file renamed `Drop` to",
        "V0304",
        &[
            (
                "main.vr",
                "mod ext;\nmod ext2;\nmod hook;\n\nfn main() {\n    match ext::make() {\n        ext::G2::Held(s) => ext::keep(s),\n        ext::G2::Empty => {}\n    }\n}\n",
            ),
            ("ext.rs", G2_RS),
            ("ext2.rs", "pub use std::ops::Drop as D;\n"),
            (
                "hook.rs",
                "impl crate::ext2::D for crate::ext::G2 {\n    fn drop(&mut self) {}\n}\n",
            ),
        ],
        "E0509",
    ),
    (
        "a payload taken out of a temporary of an enum given a destructor after a glob `use`",
        "V0304",
        &[
            ("main.vr", G2_MAIN_VR),
            ("ext.rs", G2_RS),
            (
                "hook.rs",
                "use crate::ext::*;\n\nimpl Drop for G2 {\n    fn drop(&mut self) {}\n}\n",
            ),
        ],
        "E0509",
    ),
    (
        "a payload taken out of a temporary of an enum given a destructor through a `type` alias",
        "V0304",
        &[
            ("main.vr", G2_MAIN_VR),
            ("ext.rs", G2_RS),
            (
                "hook.rs",
                "type G = crate::ext::G2;\n\nimpl Drop for G {\n    fn drop(&mut self) {}\n}\n",
            ),
        ],
        "E0509",
    ),
    (
        "a payload taken out of a temporary of an enum given a destructor inside an inline module",
        "V0304",
        &[
            ("main.vr", G2_MAIN_VR),
            ("ext.rs", G2_RS),
            (
                "hook.rs",
                "mod inner {\n    impl Drop for crate::ext::G2 {\n        fn drop(&mut self) {}\n    }\n}\n",
            ),
        ],
        "E0509",
    ),
    (
        "a payload taken out of a temporary of an enum given a destructor inside a function body",
        "V0304",
        &[
            ("main.vr", G2_MAIN_VR),
            ("ext.rs", G2_RS),
            (
                "hook.rs",
                "pub fn unused() {\n    impl Drop for crate::ext::G2 {\n        fn drop(&mut self) {}\n    }\n}\n",
            ),
        ],
        "E0509",
    ),
    (
        "a payload taken out of a temporary of an enum given a destructor from a macro",
        "V0304",
        &[
            ("main.vr", G2_MAIN_VR),
            ("ext.rs", G2_RS),
            (
                "hook.rs",
                "macro_rules! give_drop {\n    ($t:ty) => {\n        impl Drop for $t {\n            fn drop(&mut self) {}\n        }\n    };\n}\n\ngive_drop!(crate::ext::G2);\n",
            ),
        ],
        "E0509",
    ),
    (
        "a payload taken out of a temporary of an enum given a destructor inside a `println!` argument",
        "V0304",
        &[
            ("main.vr", G2_MAIN_VR),
            ("ext.rs", G2_RS),
            (
                "hook.rs",
                "pub fn noise() {\n    println!(\"{}\", {\n        impl Drop for crate::ext::G2 {\n            fn drop(&mut self) {}\n        }\n        0\n    });\n}\n",
            ),
        ],
        "E0509",
    ),
    (
        "a payload taken out of a temporary of an enum given a destructor inside a `format!` argument",
        "V0304",
        &[
            ("main.vr", G2_MAIN_VR),
            ("ext.rs", G2_RS),
            (
                "hook.rs",
                "pub fn noise() -> String {\n    format!(\"{}\", {\n        impl Drop for crate::ext::G2 {\n            fn drop(&mut self) {}\n        }\n        0\n    })\n}\n",
            ),
        ],
        "E0509",
    ),
    (
        "a payload of a temporary of a Varyk enum given a destructor inside a `vec!` argument, kept",
        "V0304",
        &[
            (
                "main.vr",
                "mod ev;\nmod hook;\n\nfn main() {\n    let mut kept: Vec<i32> = Vec::new();\n    match ev::make() {\n        ev::Ev::Msg(v) => {\n            kept = v;\n        }\n        ev::Ev::Quit => {}\n    }\n    println!(\"{}\", kept.len());\n}\n",
            ),
            ("ev.vr", VARYK_EV_VEC_VR),
            (
                "hook.rs",
                "pub fn noise() -> usize {\n    vec![{\n        impl Drop for crate::ev::Ev {\n            fn drop(&mut self) {}\n        }\n        0\n    }]\n    .len()\n}\n",
            ),
        ],
        "E0509",
    ),
    (
        "a payload of a temporary of a Varyk enum a .rs file gives a destructor, kept",
        "V0304",
        &[
            (
                "main.vr",
                "mod ev;\nmod hook;\n\nfn main() {\n    let mut kept: Vec<i32> = Vec::new();\n    match ev::make() {\n        ev::Ev::Msg(v) => {\n            kept = v;\n        }\n        ev::Ev::Quit => {}\n    }\n    println!(\"{}\", kept.len());\n}\n",
            ),
            ("ev.vr", VARYK_EV_VEC_VR),
            ("hook.rs", EV_HOOK_RS),
        ],
        "E0509",
    ),
    (
        "a private method and a private associated function called from another module",
        "V0105",
        &[
            (
                "main.vr",
                "mod shop;\n\nfn main() {\n    let c = shop::make();\n    println!(\"{}\", c.secret());\n    let d = shop::Cart::hidden();\n}\n",
            ),
            (
                "shop.vr",
                "pub struct Cart {\n    pub n: i32,\n}\n\nimpl Cart {\n    fn secret(self) -> i32 {\n        self.n\n    }\n\n    fn hidden() -> Cart {\n        Cart { n: 0 }\n    }\n}\n\npub fn make() -> Cart {\n    Cart { n: 1 }\n}\n",
            ),
        ],
        "E0624",
    ),
    (
        "a private struct in a type position from another module",
        "V0105",
        &[
            (
                "main.vr",
                "mod shop;\n\nfn show(c: shop::Cart) {}\n\nfn main() {}\n",
            ),
            ("shop.vr", "struct Cart {\n    n: i32,\n}\n"),
        ],
        "E0603",
    ),
    (
        "a `use` of a private module renamed with `as`",
        "V0105",
        &[
            (
                "main.vr",
                "mod shop;\n\nuse shop::cart as c;\n\nfn main() {\n    println!(\"{}\", c::count());\n}\n",
            ),
            ("shop.vr", "mod cart;\n"),
            ("shop/cart.vr", "pub fn count() -> i32 {\n    1\n}\n"),
        ],
        "E0603",
    ),
    (
        "a struct literal naming a private field through a `use` alias",
        "V0105",
        &[
            (
                "main.vr",
                "mod shop;\n\nuse shop::Cart as C;\n\nfn main() {\n    let c = C { n: 1 };\n}\n",
            ),
            ("shop.vr", "pub struct Cart {\n    n: i32,\n}\n"),
        ],
        "E0451",
    ),
    (
        "an imported function with a parameter behind `#[cfg]`",
        "V0100",
        &[
            (
                "main.vr",
                "mod ext;\n\nfn main() {\n    println!(\"{}\", ext::f(1, 2));\n}\n",
            ),
            (
                "ext.rs",
                "pub fn f(#[cfg(test)] x: i32, y: i32) -> i32 {\n    y\n}\n",
            ),
        ],
        "E0061",
    ),
    (
        "an imported method whose receiver is behind `#[cfg]`",
        "V0100",
        &[
            (
                "main.vr",
                "mod ext;\n\nfn main() {\n    let s = ext::S::new();\n    println!(\"{}\", s.f(2));\n}\n",
            ),
            (
                "ext.rs",
                "pub struct S {\n    pub n: i32,\n}\n\nimpl S {\n    pub fn new() -> S {\n        S { n: 1 }\n    }\n\n    pub fn f(#[cfg(any())] &self, y: i32) -> i32 {\n        y\n    }\n}\n",
            ),
        ],
        "E0599",
    ),
    (
        "a variant of an imported enum with a field behind `#[cfg]`",
        "V0100",
        &[
            (
                "main.vr",
                "mod ext;\n\nfn main() {\n    match ext::make() {\n        ext::E::A(s, n) => println!(\"{}\", n),\n        ext::E::B => {}\n    }\n}\n",
            ),
            (
                "ext.rs",
                "pub enum E {\n    A(#[cfg(any())] String, i32),\n    B,\n}\n\npub fn make() -> E {\n    E::A(1)\n}\n",
            ),
        ],
        "E0023",
    ),
    (
        "a variant of an imported enum that is behind `#[cfg]`",
        "V0100",
        &[
            (
                "main.vr",
                "mod ext;\n\nfn main() {\n    match ext::make() {\n        ext::E::A => {}\n        ext::E::B => {}\n    }\n}\n",
            ),
            ("ext.rs", CFG_ENUM_RS),
        ],
        "E0599",
    ),
    (
        "a call to a Rust function whose file invokes, by path, a macro of another file that defines `String`",
        "V0108",
        &[
            (
                "main.vr",
                "mod defs;\nmod user;\n\nfn main() {\n    println!(\"{}\", user::take(\"x\"));\n}\n",
            ),
            (
                "defs.rs",
                "#[macro_export]\nmacro_rules! shadow {\n    () => {\n        pub struct String;\n    };\n}\n",
            ),
            (
                "user.rs",
                "crate::shadow!();\n\npub fn take(_s: String) -> usize {\n    0\n}\n",
            ),
        ],
        "E0308",
    ),
    (
        "a call to a Rust function whose file calls a macro defined in an inline module that defines `String`",
        "V0108",
        &[
            (
                "main.vr",
                "mod ext;\n\nfn main() {\n    println!(\"{}\", ext::f().len());\n}\n",
            ),
            (
                "ext.rs",
                "#[macro_use]\nmod inner {\n    macro_rules! mk {\n        () => {\n            pub type String = i32;\n        };\n    }\n}\n\nmk!();\n\npub fn f() -> String {\n    1\n}\n",
            ),
        ],
        "E0599",
    ),
    (
        "a call to a Rust function whose file calls a local macro relaying to another file's macro that defines `String`",
        "V0108",
        &[
            (
                "main.vr",
                "mod defs;\nmod ext;\n\nfn main() {\n    println!(\"{}\", ext::take(\"x\"));\n}\n",
            ),
            (
                "defs.rs",
                "#[macro_export]\nmacro_rules! mk_string {\n    () => {\n        pub struct String;\n    };\n}\n",
            ),
            (
                "ext.rs",
                "macro_rules! a {\n    () => {\n        crate::mk_string!();\n    };\n}\n\na!();\n\npub fn take(_s: String) -> usize {\n    0\n}\n",
            ),
        ],
        "E0308",
    ),
    (
        "a call to a Rust function whose file passes another file's macro that defines `String` through a local macro",
        "V0108",
        &[
            (
                "main.vr",
                "mod defs;\nmod ext;\n\nfn main() {\n    println!(\"{}\", ext::take(\"x\"));\n}\n",
            ),
            (
                "defs.rs",
                "#[macro_export]\nmacro_rules! mk {\n    () => {\n        pub struct String;\n    };\n}\n",
            ),
            (
                "ext.rs",
                "macro_rules! fwd {\n    ($($t:tt)*) => {\n        $($t)*\n    };\n}\n\nfwd! { crate::mk!(); }\n\npub fn take(_s: String) -> usize {\n    0\n}\n",
            ),
        ],
        "E0308",
    ),
    (
        "a field of a `#[repr(packed)]` Rust struct, borrowed",
        "V0101",
        &[
            (
                "main.vr",
                "mod ext;\n\nfn show(s: ext::S) {\n    println!(\"{}\", s.x);\n}\n\nfn main() {}\n",
            ),
            (
                "ext.rs",
                "#[repr(packed)]\npub struct S {\n    pub a: u8,\n    pub x: i32,\n}\n",
            ),
        ],
        "E0793",
    ),
    (
        "a Rust struct with no fixed size in a `Vec`",
        "V0101",
        &[
            (
                "main.vr",
                "mod ext;\n\nfn keep(v: Vec<ext::S>) {}\n\nfn main() {}\n",
            ),
            (
                "ext.rs",
                "pub struct S {\n    pub n: i32,\n    pub bytes: [u8],\n}\n",
            ),
        ],
        "E0277",
    ),
    (
        "a private method of an imported struct",
        "V0100",
        &[
            (
                "main.vr",
                "mod ext;\n\nfn main() {\n    let t = ext::Tally::new();\n    println!(\"{}\", t.secret());\n}\n",
            ),
            ("ext.rs", TALLY_RS),
        ],
        "E0624",
    ),
];

/// An imported enum with a variant configured out of the build: Varyk
/// imports it as an opaque type.
const CFG_ENUM_RS: &str = "pub enum E {\n    A,\n    #[cfg(any())]\n    B,\n}\n\npub fn make() -> E {\n    E::A\n}\n\npub fn show(e: E) {\n    if let E::A = e {\n        println!(\"a\");\n    }\n}\n";

/// A Varyk enum with a `String` payload, which [`EV_HOOK_RS`] gives a
/// destructor.
const VARYK_EV_VR: &str = "pub enum Ev {\n    Msg(string),\n    Quit,\n}\n\npub fn make() -> Ev {\n    Ev::Msg(\"hi\")\n}\n";

/// The same enum with a `Vec` payload, written in the part of Varyk that
/// is also Rust.
const VARYK_EV_VEC_VR: &str = "pub enum Ev {\n    Msg(Vec<i32>),\n    Quit,\n}\n\npub fn make() -> Ev {\n    Ev::Msg(Vec::new())\n}\n";

/// A `.rs` file giving the Varyk enum `crate::ev::Ev` a destructor by its
/// full path.
const EV_HOOK_RS: &str = "impl Drop for crate::ev::Ev {\n    fn drop(&mut self) {}\n}\n";

/// A `.rs` module whose function and field name a type of `shop`'s
/// private module `hidden`.
const HIDDEN_API_RS: &str = "pub struct Boxed {
    pub h: crate::shop::hidden::H,
}

pub fn make() -> crate::shop::hidden::H {
    crate::shop::hidden::H { x: 1 }
}

pub fn boxed() -> Boxed {
    Boxed { h: make() }
}
";

/// The program of the `impl Drop` form cases: it takes the payload out of
/// a temporary of `ext::G2`, which `hook.rs` gives a destructor.
const G2_MAIN_VR: &str = "mod ext;\nmod hook;\n\nfn main() {\n    match ext::make() {\n        ext::G2::Held(s) => ext::keep(s),\n        ext::G2::Empty => {}\n    }\n}\n";

/// An imported enum with no destructor of its own file.
const G2_RS: &str = "pub enum G2 {
    Held(String),
    Empty,
}

pub fn make() -> G2 {
    G2::Held(String::from(\"hi\"))
}

pub fn keep(s: String) {
    println!(\"{s}\");
}
";

/// An imported enum with a destructor: a `match` on a temporary of it
/// only looks inside it (rustc E0509 forbids moving a payload out).
const DROP_ENUM_RS: &str = "pub enum Ev {
    Msg(String),
    Quit,
}

impl Drop for Ev {
    fn drop(&mut self) {}
}

pub fn make() -> Ev {
    Ev::Msg(String::from(\"hi\"))
}

pub fn keep(s: String) {
    println!(\"{s}\");
}
";

/// The imported struct of the [`MUST_REJECT_TREES`] cases: a `&self` and
/// a `&mut self` method, and a function that takes it by value.
const TALLY_RS: &str = "pub struct Tally {
    total: i32,
}

impl Tally {
    pub fn new() -> Tally {
        Tally { total: 0 }
    }

    pub fn count(&self) -> i32 {
        self.total
    }

    pub fn add(&mut self, n: i32) {
        self.total += n;
    }

    fn secret(&self) -> i32 {
        self.total
    }
}

pub fn keep_tally(t: Tally) -> i32 {
    t.total
}
";

/// The [`MUST_REJECT_TREES`] rustc accepts as written (the harness checks
/// that they build): the backend would annotate the local `v` with the
/// type's full path, `Vec<crate::shop::cart::Cart>`, which `main` cannot
/// name (rustc E0603), so Varyk rejects the signature instead.
///
/// The same holds for an imported `.rs` function or field whose type is
/// behind a private module (M3 spec 4.2): Varyk makes it usable only
/// where that module is seen.
const STRICTER_THAN_RUST_TREES: &[&str] = &[
    "a pub function returning a type its callers cannot name",
    "an imported function returning a type behind a private module",
    "an imported call naming a type behind a private module, from outside that module",
    "an imported field of a type behind a private module",
    "an imported enum with a payload of a type behind a private module",
];

#[test]
fn trees_that_break_a_rule_fail_check() {
    for (index, (label, code, files, _)) in MUST_REJECT_TREES.iter().enumerate() {
        let entry = write_program(&format!("reject_tree{index:02}"), files);
        let (checked, stderr) = run("check", &entry);
        let codes: Vec<&str> = stderr
            .lines()
            .filter_map(|line| line.strip_prefix("error[")?.split(']').next())
            .collect();
        assert!(
            !checked && !codes.is_empty() && codes.iter().all(|c| c == code),
            "expected `check` to reject with {code} only: {label}\n{stderr}"
        );
    }
}

#[test]
fn trees_that_break_a_rule_fail_rustc() {
    let dir = scratch_root().join("rustc_rejects_trees");
    let mut manifest = "[package]\nname = \"rejects\"\nversion = \"0.0.0\"\nedition = \"2024\"\nautobins = false\n\n[workspace]\n".to_string();
    let mut write = |name: &str, files: &[(&str, &str)]| {
        let bin = dir.join("src").join("bin").join(name);
        for (path, text) in files {
            let path = bin.join(path.replace(".vr", ".rs"));
            fs::create_dir_all(path.parent().expect("a file has a directory"))
                .expect("create the case directory");
            fs::write(&path, text).expect("write a case file");
        }
        manifest.push_str(&format!(
            "\n[[bin]]\nname = \"{name}\"\npath = \"src/bin/{name}/main.rs\"\n"
        ));
    };
    // The fixed form of a case must build: a failure of the whole crate
    // fails it too.
    write(
        "control",
        &[
            (
                "main.vr",
                "mod shop;\n\nfn main() {\n    println!(\"{}\", shop::cart::count());\n}\n",
            ),
            ("shop.vr", "pub mod cart;\n"),
            ("shop/cart.vr", "pub fn count() -> i32 {\n    1\n}\n"),
        ],
    );
    let mut bins = Vec::new();
    let mut accepted = Vec::new();
    for (index, (label, _, files, rustc_code)) in MUST_REJECT_TREES.iter().enumerate() {
        let name = format!("reject{index:02}");
        write(&name, files);
        if STRICTER_THAN_RUST_TREES.contains(label) {
            accepted.push((name, *label));
        } else {
            bins.push((name, *label, *rustc_code));
        }
    }
    fs::write(dir.join("Cargo.toml"), manifest).expect("write Cargo.toml");

    let output = std::process::Command::new(env!("CARGO"))
        .args(["build", "--keep-going", "--bins", "--message-format=json"])
        .arg("--manifest-path")
        .arg(dir.join("Cargo.toml"))
        .arg("--target-dir")
        .arg(cargo_target("rustc_rejects_trees"))
        .output()
        .expect("run cargo");
    let messages = cargo_messages(&output);
    let unbuilt: Vec<&str> = accepted
        .iter()
        .filter(|(name, _)| !messages.built.contains(name))
        .map(|(_, label)| *label)
        .collect();
    assert!(
        messages.built.contains("control"),
        "the control case, which breaks no rule, fails to build:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        unbuilt.is_empty(),
        "trees listed as stricter than Rust that rustc rejects: {unbuilt:?}\n\ncargo stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    // Each bin must fail with its own listed rustc code, and no other
    // coded error: a failure of the whole crate names no bin, and any
    // other error would hide what rustc thinks of the rejection.
    let unrejected: Vec<&str> = bins
        .iter()
        .filter(|(name, _, rustc_code)| !messages.fails_with(name, rustc_code))
        .map(|(_, label, _)| *label)
        .collect();
    assert!(
        unrejected.is_empty(),
        "must-reject trees rustc does not reject for privacy alone: {unrejected:?}\n\ncargo stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// What a `cargo build --message-format=json` said, per bin.
struct CargoMessages {
    /// The bins that built.
    built: std::collections::HashSet<String>,
    /// Per bin, the codes of its errors.
    errors: std::collections::HashMap<String, Vec<String>>,
}

impl CargoMessages {
    /// Whether bin `name` failed, with errors of code `code` only.
    fn fails_with(&self, name: &str, code: &str) -> bool {
        self.errors
            .get(name)
            .is_some_and(|codes| !codes.is_empty() && codes.iter().all(|c| c == code))
    }
}

/// Parses cargo's JSON messages on `output`'s stdout.
fn cargo_messages(output: &std::process::Output) -> CargoMessages {
    let mut messages = CargoMessages {
        built: Default::default(),
        errors: Default::default(),
    };
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let Ok(message) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let Some(name) = message["target"]["name"].as_str() else {
            continue;
        };
        match message["reason"].as_str() {
            Some("compiler-artifact") => {
                messages.built.insert(name.to_string());
            }
            Some("compiler-message") if message["message"]["level"] == "error" => {
                let codes = messages.errors.entry(name.to_string()).or_default();
                // rustc's closing "aborting due to ..." is an error without
                // a code; every real error has one.
                if let Some(code) = message["message"]["code"]["code"].as_str() {
                    codes.push(code.to_string());
                }
            }
            _ => {}
        }
    }
    messages
}

#[test]
fn programs_that_break_a_rule_fail_check() {
    let cases = MUST_REJECT
        .iter()
        .map(|(body, code, _)| (body, code, ""))
        .chain(
            MUST_REJECT_QUESTION
                .iter()
                .map(|(body, code, _)| (body, code, RESULT_RET)),
        );
    for (index, (body, code, ret)) in cases.enumerate() {
        let entry = write_cases(&format!("reject{index:02}"), &[function("t", body, ret)]);
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
    let mut accepted = Vec::new();
    let mut unemitted = Vec::new();
    let cases = MUST_REJECT
        .iter()
        .map(|(body, _, rustc_code)| (*body, "", *rustc_code))
        .chain(
            MUST_REJECT_QUESTION
                .iter()
                .map(|(body, _, rustc_code)| (*body, RESULT_RET, *rustc_code)),
        );
    // Writes the case as bin `name`; false when it does not type-check (a
    // `?` outside a `Result` function has no Rust).
    let mut emit = |name: &str, body: &str, ret: &str| {
        let entry = write_cases(&format!("emit_{name}"), &[function("t", body, ret)]);
        let text = fs::read_to_string(&entry).expect("read main.vr");
        let mut sources = Vec::new();
        let Ok(program) = resolve(SourceFile::new(FileId(0), &entry, text), &mut sources)
            .and_then(|resolved| typecheck(resolved, &sources))
        else {
            return false;
        };
        let (program, _) = analyze_unchecked(program, &sources);
        let generated =
            RustBackend.generate(&program, &CrateInfo::single_file("rejects".to_string()));
        let main = &generated
            .files
            .iter()
            .find(|file| file.path == "src/main.rs")
            .expect("a main.rs")
            .text;
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
    for (index, (body, ret, rustc_code)) in cases.enumerate() {
        let name = format!("reject{index:02}");
        if !emit(&name, body, ret) {
            unemitted.push(body);
        } else if STRICTER_THAN_RUST.contains(&body) {
            accepted.push((name, body));
        } else {
            bins.push((name, body, rustc_code));
        }
    }
    assert_eq!(
        unemitted, UNEMITTED,
        "the cases that do not type-check are not the listed ones"
    );
    fs::write(dir.join("Cargo.toml"), manifest).expect("write Cargo.toml");

    let output = std::process::Command::new(env!("CARGO"))
        .args(["build", "--keep-going", "--bins", "--message-format=json"])
        .arg("--manifest-path")
        .arg(dir.join("Cargo.toml"))
        .arg("--target-dir")
        .arg(cargo_target("rustc_rejects"))
        .output()
        .expect("run cargo");
    let messages = cargo_messages(&output);
    assert!(
        messages.built.contains("control"),
        "the control case, which breaks no rule, fails to build:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let unbuilt: Vec<&str> = accepted
        .iter()
        .filter(|(name, _)| !messages.built.contains(name))
        .map(|(_, body)| *body)
        .collect();
    assert!(
        unbuilt.is_empty(),
        "cases listed as stricter than Rust that rustc rejects: {unbuilt:?}\n\ncargo stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    // Each bin must fail with its own listed rustc code, and no other
    // coded error: a failure of the whole crate (a broken manifest, a
    // missing `ext.rs`) names no bin.
    let unrejected: Vec<String> = bins
        .iter()
        .filter(|(name, _, rustc_code)| !messages.fails_with(name, rustc_code))
        .map(|(name, body, rustc_code)| {
            format!(
                "{name} (expected {rustc_code}, got {:?}): {body}",
                messages.errors.get(name)
            )
        })
        .collect();
    assert!(
        unrejected.is_empty(),
        "must-reject cases without their own rustc error:\n{}\n\ncargo stderr:\n{}",
        unrejected.join("\n---\n"),
        String::from_utf8_lossy(&output.stderr)
    );
}
