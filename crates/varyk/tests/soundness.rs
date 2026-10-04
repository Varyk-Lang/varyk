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
//! [`MUST_REJECT_ITEMS`] do the same with functions of their own beside
//! the case. The
//! [`MUST_BUILD_TREES`] and [`MUST_REJECT_TREES`] do the same for
//! programs of several modules.

mod common;

use std::fs;
use std::path::{Path, PathBuf};

use common::{varyk, varyk_in};
use varyk::backend::{Backend, CrateInfo, RustBackend, StdDependency};
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

fn mk_oe() -> Option<E> {
    Some(E::A(mk_s()))
}

enum F {
    N { s: string, n: i32 },
    M,
}

fn mk_f() -> F {
    F::N { n: 1, s: mk_s() }
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

fn mk_m() -> HashMap<string, i32> {
    let mut m: HashMap<string, i32> = HashMap::new();
    m.insert(\"k\", 1);
    m
}

struct Hm {
    m: HashMap<string, i32>,
    n: i32,
}

fn mk_hm() -> Hm {
    Hm { m: mk_m(), n: 1 }
}

fn read_m(m: HashMap<string, i32>) {}

struct Held {
    o: Option<i32>,
    r: Result<i32, bool>,
    rs: Result<i32, string>,
}

fn mk_held() -> Held {
    Held { o: Some(1), r: Ok(2), rs: Err(\"e\") }
}

enum Shape {
    Circle(f64),
    Dot,
}

fn area(r: f64) -> f64 {
    r * r
}

fn take_os(o: Option<string>) {}

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

    fn tagged(self, o: Option<i32>) -> Option<string> {
        o.map(|n| format!(\"{}{}\", self.s, n))
    }

    fn bumped(mut self, o: Option<i32>) -> Option<i32> {
        o.map(|n| {
            let s = self.s;
            read_s(s);
            n + self.n
        })
    }

    fn fresh() -> P {
        mk_p()
    }

    async fn a_n(self) -> i32 {
        self.n
    }

    async fn a_inc(mut self) {
        self.n = self.n + 1;
    }
}

async fn a_i(n: i32) -> i32 {
    n
}

async fn a_len(s: string) -> usize {
    s.len()
}

async fn a_p(p: P) -> P {
    P { s: p.s.clone(), n: a_i(p.n).await }
}

async fn a_r(n: i32) -> Result<i32, string> {
    if n > 0 {
        Ok(n)
    } else {
        Err(\"negative\")
    }
}

async fn a_try(n: i32) -> Result<i32, string> {
    let m = a_r(n).await?;
    Ok(m + a_r(m).await?)
}

async fn a_bump(mut p: P) {
    p.n = p.n + 1;
}

async fn a_loops(v: Vec<P>, w: Vec<string>) -> i32 {
    let mut n = 0;
    for p in v {
        n = n + p.a_n().await;
    }
    for s in w.iter().filter(|x| x.len() > 0) {
        n = n + a_len(s).await as i32;
    }
    for i in 0..2 {
        n = n + a_i(i).await;
    }
    n
}

async fn a_while(c: bool, s: string) -> i32 {
    let mut n = 0;
    while c {
        n = n + a_len(s).await as i32;
        break;
    }
    while a_i(n).await > 100 {
        n = n - 1;
    }
    n
}

async fn a_branches(n: i32, o: Option<P>) -> i32 {
    let mut m = match a_r(n).await {
        Ok(k) => k,
        Err(e) => a_len(e).await as i32,
    };
    match o {
        Some(p) => {
            m = m + p.a_n().await;
        }
        None => {}
    }
    if let Ok(k) = a_r(m).await {
        m = m + k;
    }
    m
}

async fn a_print(s: string, p: P) -> string {
    println!(\"{} {}\", a_len(s).await, p.a_n().await);
    format!(\"{}\", a_i(p.n).await)
}

struct Cfg {
    factor: i32,
    name: string,
    tags: Vec<string>,
    items: Vec<P>,
    e: E,
}

fn mk_cfg() -> Cfg {
    Cfg { factor: 2, name: \"cfg\", tags: vec![\"a\", \"bb\"], items: vec![mk_p()], e: E::A(mk_s()) }
}

impl Cfg {
    fn twice(self) -> i32 {
        self.factor * 2
    }

    fn cname(self) -> string {
        self.name
    }

    async fn a_twice(self) -> i32 {
        a_i(self.factor).await * 2
    }
}

fn read_shared(c: Shared<Cfg>) -> i32 {
    read_s(c.cname());
    c.factor + c.twice() + c.items.len() as i32
}

async fn a_cfg(c: Shared<Cfg>) -> i32 {
    a_i(c.factor).await + c.twice() + c.a_twice().await
}

struct G {
    name: string,
    tags: Vec<string>,
    shape: Shape,
    items: Vec<P>,
}

fn mk_g() -> G {
    G { name: \"  g  \", tags: vec![\"t\"], shape: Shape::Circle(1.5), items: vec![mk_p()] }
}

impl G {
    fn name_ref(self) -> string {
        self.name
    }

    fn tags(self) -> Vec<string> {
        self.tags
    }

    fn shape(self) -> Shape {
        self.shape
    }

    fn first_item(self) -> P {
        self.items[0]
    }

    fn change(mut self) {
        self.name = \"changed\";
    }
}

fn trimmed(text: string) -> string {
    text.trim()
}

fn first_of(v: Vec<P>) -> P {
    v[0]
}

fn s_of(p: P) -> string {
    p.s
}

fn s_unless(p: P, hidden: string) -> string {
    if p.s == hidden { p.s } else { p.s }
}

fn g_name(g: G) -> string {
    g.name_ref()
}

fn sep_of(s: string) -> string {
    s
}

fn held_o(h: Held) -> Option<i32> {
    h.o
}

fn held_r(h: Held) -> Result<i32, bool> {
    h.r
}

struct Team {
    names: Vec<string>,
    users: Vec<G>,
}

fn mk_team() -> Team {
    Team { names: vec![\"ann\", \"bo\"], users: vec![mk_g()] }
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
pub fn refnum(x: &i32) -> &str { if *x > 0 { \"pos\" } else { \"neg\" } }
pub async fn a_take(s: String) -> usize { s.len() }
pub async fn a_bump_tally(t: &mut Tally) { t.total += 1; }
impl Tally {
    pub async fn a_count(&self) -> i32 { self.total }
}
pub fn bind(q: &'static str, values: Vec<varyk_std::Value>) -> usize { q.len() + values.len() }
pub async fn a_one(q: &'static str) -> usize { q.len() }
pub async fn a_bind(q: &'static str, values: Vec<varyk_std::Value>) -> usize { q.len() + values.len() }
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
    let mut loe = mk_oe();
    let mut lf = mk_f();
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
            // A borrowed-return result is a place of its argument (M4
            // spec 3.1).
            "if c { trimmed(ps) } else { ls.trim() }",
            "if c { s_of(pp) } else { \"z\" }",
            "if c { mk_s() } else { trimmed(ps) }",
            "{ let g = mk_g(); g.name_ref() }",
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
            "if c { first_of(pv) } else { pp }",
            "{ let v = vec![mk_p()]; first_of(v) }",
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
    // Trailing values (milestone 5b3 spec 2.2) are read, never moved.
    Context {
        name: "trailing value",
        ret: "",
        body: "ext::bind(\"q\", {v});",
        values: &[
            "ls",
            "ll",
            "ps",
            "ms",
            "lp.s",
            "pp.s",
            "mq.p.s",
            "lw[0]",
            "mk_s()",
            "mk_p().s",
            "\"lit\"",
            "li, pi, mi, lp.n, lv[0].n, mk_p().n, lk, c",
            "1, -2, 2.5, true, \"lit\", li + 1, li as u8",
            "if c { mk_s() } else { ps }",
            "if c { ls } else { ps }",
            "{ let a = mk_p(); a.s }",
            "match le { E::A(s) => s, _ => \"x\" }",
            "match pe { E::A(s) => s, _ => ps }",
            "if c { lk } else { None }",
            "format!(\"{}\", li)",
        ],
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
            "if c { s_of(lp) } else { \"z\" }",
            "(if c { first_of(lv) } else { pp }).s",
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
            "first_of(lv)",
            "if c { first_of(pv) } else { lq.p }",
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
            "if c { trimmed(ps) } else { lp.s }",
            "if c { \"v\" } else { s_of(lp) }",
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
            "if c { first_of(lv) } else { pp }",
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
            "trimmed(ls)",
            "s_of(lp)",
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
    // Borrowed returns (M4 spec 3.1): the case function returns part of
    // one of its many reference parameters, so a lifetime is written.
    Context {
        name: "borrowed string return",
        ret: " -> string",
        body: "{v}",
        values: &[
            "ps",
            "pp.s",
            "pv[0].s",
            "trimmed(ps)",
            "ps.trim()",
            "s_of(pp)",
            "if c { pp.s } else { pv[0].s }",
            "{ let n = pp.s; n }",
            "if c { pp.s } else { ps }",
            "if c { \"x\" } else { ps }",
            "mp.s",
            "lp.s",
            "\"x\".trim()",
        ],
    },
    Context {
        name: "borrowed struct return",
        ret: " -> P",
        body: "{v}",
        values: &[
            "pp",
            "pv[0]",
            "first_of(pv)",
            "if c { pv[0] } else { first_of(pv) }",
            "if c { pp } else { pv[0] }",
            "mv[0]",
            "lv[0]",
        ],
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
            "Ok(mk_s())?",
            "{ let a = Ok(mk_s())?; a }",
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
            "Ok(mk_p())?",
        ],
    },
    // `?` on an `Option` in a function returning one (M4 spec 2.6), and
    // the constructors that need their type written under `?`.
    Context {
        name: "question option value",
        ret: " -> Option<i32>",
        body: "read_p({v});\n    read_s({v}.s);\n    let x = {v};\n    lv.push({v});\n    let n = {v}.n;\n    Some(n + {v}.get_n())",
        values: &[
            "mk_o()?",
            "{ let a = mk_o()?; a }",
            "if c { mk_o()? } else { mk_p() }",
            "Some(mk_p())?",
            "{ let o = mk_o(); o? }",
        ],
    },
    Context {
        name: "question option with an expected type",
        ret: " -> Option<i32>",
        body: "let a: i32 = {v};\n    let b: i32 = Some({v})?;\n    Some(a + b)",
        values: &["Some(li)?", "Some(pi)?", "Some(mi)?"],
    },
    // `?` on `parse` in a function returning `Result<_, Error>` (M5a spec
    // 2.8), on each kind of text.
    Context {
        name: "question error result with an expected type",
        ret: " -> Result<i32, Error>",
        body: "let a: i32 = {v};\n    Ok(a)",
        values: &[
            "ls.parse()?",
            "ps.parse()?",
            "lw[0].parse()?",
            "lp.s.parse()?",
            "mk_s().parse()?",
            "\"3\".parse()?",
        ],
    },
    Context {
        name: "question result with an expected type",
        ret: RESULT_RET,
        body: "let a: i32 = {v};\n    let b: i64 = Ok(3)?;\n    let c = Ok(a)?;\n    Ok(a + c)",
        values: &["Ok(li)?", "Ok(pi)?", "Ok(mi)?", "Ok(mk_i())?"],
    },
    // A cast (M4 spec 2.9) in each operand position: a cast is written in
    // parentheses, a literal operand with its type suffix.
    Context {
        name: "cast",
        ret: "",
        body: "let a = {v};\n    let b = {v} + {v};\n    let d = {v} < {v};\n    let m = {v} * 2;\n    println!(\"{}\", {v});\n    ls = format!(\"{}\", {v});\n    if {v} == {v} {}",
        values: &[
            "li as i64",
            "mi as i64",
            "pi as u8",
            "(li + pi) as i64",
            "(mi + 1) as i64",
            "-1 as u8",
            "300 as u8",
            "-129 as i8",
            "{ 300 } as u8",
            "- -1 as u8",
            "-{ 1 } as u8",
            "lp.n as i64",
            "lv.len() as i32",
            "mk_i() as f64",
            "1.5 as i32",
            "{ li } as u16",
            "(if c { li } else { pi }) as u8",
        ],
    },
    Context {
        name: "cast operand",
        ret: "",
        body: "let a = li as i64 * 2;\n    let b = 1 + pi as i64;\n    let d = (li as i64) as u8 as f32;\n    read_i({v} as i32);\n    lw.push(format!(\"{}\", {v} as i32));",
        values: &[
            "li",
            "mi",
            "pi",
            "lp.n",
            "mk_i()",
            "lv.len() as i32",
            "-1",
            "(li + pi)",
        ],
    },
    // `..=` in a `for` head (M4 spec 2.11).
    Context {
        name: "inclusive range",
        ret: "",
        body: "for i in 1..={v} {\n        read_i(i);\n    }\n    for j in li..={v} {\n        change_i(j);\n    }\n    for k in 0..={v} + 1 {\n        println!(\"{}\", k);\n    }",
        values: &[
            "li",
            "mi",
            "pi",
            "mk_i()",
            "lv.len() as i32",
            "(li + pi)",
            "{ li }",
        ],
    },
    // The table rows whose result is a value (M4 spec 2.7), on each kind
    // of receiver: read `string` arguments are `&str`, a read `T` a `&T`.
    // The rows on `Option`, `Result`, and a `Vec` of Copy or struct
    // elements are in [`MUST_PASS`].
    Context {
        name: "string reading rows",
        ret: "",
        body: "let a = {v}.is_empty();\n    let b = {v}.contains(ps) && {v}.contains(ls) && {v}.contains(lp.s) && {v}.contains(\"x\");\n    let d = {v}.starts_with(ll) || {v}.starts_with(ms);\n    let e = {v}.to_uppercase();\n    let f = {v}.replace(ps, lp.s);\n    let g: Result<i32, Error> = {v}.parse();\n    let h: Result<bool, Error> = {v}.parse();\n    read_s({v}.replace(\"a\", ls));\n    lw.push({v}.to_uppercase());\n    read_s(e);\n    lw.push(f);",
        values: &[
            "ls",
            "ll",
            "ps",
            "ms",
            "lp.s",
            "mp.s",
            "lw[0]",
            "mk_s()",
            "\"lit\"",
            "(if c { ls } else { ps })",
        ],
    },
    Context {
        name: "string changing rows",
        ret: "",
        body: "{v}.push_str(ps);\n    {v}.push_str(\"x\");\n    {v}.push_str(lp.s);\n    {v}.push_str(lw[0]);\n    read_s({v});",
        values: &[
            "ls",
            "ll",
            "ms",
            "mp.s",
            "lw[0]",
            "mk_s()",
            "\"lit\"",
            "(if c { ls } else { ms })",
            "(if c { mk_s() } else { \"q\" })",
        ],
    },
    Context {
        name: "Vec of strings rows",
        ret: "",
        body: "let e = ls;\n    let a = {v}.is_empty();\n    let b = {v}.contains(ps) && {v}.contains(e) && {v}.contains(\"a\") && {v}.contains(lp.s) && {v}.contains(ms);\n    let d = {v}.join(ps);\n    let f = {v}.join(\", \");\n    read_s({v}.join(lp.s));\n    lw.push(d);",
        values: &[
            "lw",
            "mk_vs()",
            "mk_words().w",
            "(if c { lw } else { mk_vs() })",
        ],
    },
    Context {
        name: "HashMap reading rows",
        ret: "",
        body: "let mut lm = mk_m();\n    let mut lh = mk_hm();\n    let e = ls;\n    let a = {v}.len();\n    let b = {v}.contains_key(ps) && {v}.contains_key(e) && {v}.contains_key(\"k\") && {v}.contains_key(lp.s) && {v}.contains_key(ms) && {v}.contains_key(lw[0]);",
        values: &["lm", "lh.m", "mk_m()", "(if c { lm } else { mk_m() })"],
    },
    // Read inside a closure (M4 spec 3.2): a name from outside keeps its
    // kind and representation, and is only borrowed.
    Context {
        name: "read inside a closure",
        ret: "",
        body: "let a = lk.map(|x| {\n        read_s({v});\n        let y = {v};\n        read_s(y);\n        let b = {v} == ps && ls == {v};\n        x\n    });\n    read_s(ls);",
        values: &[
            "ls",
            "ll",
            "ps",
            "ms",
            "lp.s",
            "mp.s",
            "pp.s",
            "lw[0]",
            "mk_s()",
            "\"lit\"",
            "(if c { ls } else { ps })",
            "{ let t = mk_p(); t.s }",
            "ls.trim()",
            "s_of(lp)",
        ],
    },
    // Returned from an `Option::map` closure, which must return something
    // new (M4 spec 3.2).
    Context {
        name: "returned from a closure",
        ret: "",
        body: "let a = lk.map(|x| {v});\n    read_s(a.unwrap_or(\"none\"));\n    read_s(ls);",
        values: &[
            "mk_s()",
            "\"lit\"",
            "ls.clone()",
            "ls",
            "ll",
            "ps",
            "ms",
            "lp.s",
            "lw[0]",
            "if c { \"a\" } else { mk_s() }",
            "if c { ls } else { mk_s() }",
            "if c { ls } else { ps }",
            "{ let t = mk_p(); t.s }",
            "{ let t = \"t\"; t }",
            "{ let mut t = \"t\"; t = ls; t }",
            "ls.trim()",
            "s_of(lp)",
            "format!(\"{}{}\", ls, x)",
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
    // `match` (spec 2.3, 3.2): a unit arm changing the root, a binding
    // shadowing an outer local, and a temporary's binding given away. (A
    // variant arm repeated after an identical one, rustc's
    // unreachable-pattern warning, is V0205 since M4 spec 2.5.)
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
    // Patterns (M4 spec 2.5): nested, named-field, `bool`, number, and
    // string patterns on places and temporaries; a temporary's bindings
    // at depth given away.
    "match loe {\n        Some(E::A(s)) => read_s(s),\n        Some(E::B(p)) => read_p(p),\n        Some(E::C) => {}\n        None => {}\n    }",
    "match mk_oe() {\n        Some(E::A(s)) => lw.push(s),\n        Some(E::B(p)) => lv.push(p),\n        _ => {}\n    }",
    "match lve[0] {\n        E::B(p) => read_i(p.n),\n        _ => {}\n    }",
    "match lf {\n        F::N { s, n: 0 } => read_s(s),\n        F::N { s: _, n } => read_i(n),\n        F::M => {}\n    }",
    "match mk_f() {\n        F::N { s, n: _ } => lw.push(s),\n        F::M => {}\n    }",
    "match c {\n        true => read_i(1),\n        false => {}\n    }",
    "match li {\n        -5..=-1 => {}\n        -7 => {}\n        0 => read_i(li),\n        _ => {}\n    }",
    "match mi {\n        1 => {}\n        k => read_i(k),\n    }\n    mi = 2;",
    "match mk_i() {\n        1..=3 => {}\n        k => read_i(k),\n    }",
    "match lrv[0] {\n        Ok(p) => read_p(p),\n        Err(e) => read_s(e),\n    }",
    // String heads, written as exactly a `&str`: a `String` local, a
    // `&str` local, a `&str` and a `&mut String` parameter, a field, a
    // `&String` `for` variable, and a temporary.
    "match ls {\n        \"a\" => {}\n        other => read_s(other),\n    }\n    ls = mk_s();",
    "match ll {\n        \"lit\" => {}\n        _ => {}\n    }",
    "match ps {\n        \"a\" => {}\n        other => read_s(other),\n    }",
    "match ms {\n        \"a\" => {}\n        _ => {}\n    }\n    ms = \"b\";",
    "match lp.s {\n        \"mp\" => lp.bump(),\n        _ => {}\n    }",
    "for s in lw {\n        match s {\n            \"a\" => {}\n            other => read_s(other),\n        }\n    }",
    "match mk_s() {\n        \"a\" => {}\n        other => read_s(other),\n    }",
    // Review focus 3: a string-literal `match` on an owned local whose
    // catch-all is copied into a `string`; and a string head with no
    // literal arm whose binding is copied into a `Vec<string>`.
    "let owned = mk_s();\n    let shown: string = match owned {\n        \"admin\" => \"Administrator\",\n        other => other.clone(),\n    };\n    read_s(shown);",
    "match mk_s() {\n        other => lw.push(other.clone()),\n    }",
    // `if let` and `while let` (M4 spec 2.4) on each kind of head, with
    // `else if let`.
    "if let Some(p) = lo {\n        read_p(p);\n    } else if let Ok(q) = lr {\n        read_p(q);\n    } else {\n        read_i(1);\n    }",
    "if let Ok(p) = pr {\n        read_p(p);\n    }",
    "if let Some(p) = mk_o() {\n        lv.push(p);\n    }",
    "if let E::B(p) = lq.e {\n        read_p(p);\n    }",
    "if let E::A(s) = lve[0] {\n        read_s(s);\n    }",
    "if let \"a\" = ls {\n        read_s(ls);\n    }",
    "let n = if let Some(k) = lk {\n        k\n    } else {\n        0\n    };\n    read_i(n);",
    "while let Some(n) = ln.pop() {\n        read_i(n);\n    }",
    "while let Some(k) = lk {\n        lk = None;\n        read_i(k);\n    }",
    "while let Some(p) = mv.pop() {\n        if p.n == 1 {\n            break;\n        }\n        continue;\n    }",
    "while let Some(E::A(s)) = mk_oe() {\n        lw.push(s);\n        break;\n    }",
    // Each text source of the table rows (M4 spec 3.5) is an owned `let`.
    "let a = ls.to_uppercase();\n    let b = ps.replace(\"a\", ls);\n    let d = lw.join(ps);\n    let e = lw.remove(0);\n    lw.push(a);\n    lw.push(b);\n    lw.push(d);\n    lw.push(e);",
    // `parse` into each number type and `bool`; `contains` and `sort` on
    // `bool`s, and `contains` on floats; a `HashMap` of each key kind.
    "let a: Result<u8, Error> = ls.parse();\n    let b: Result<i64, Error> = ps.parse();\n    let d: Result<f32, Error> = lp.s.parse();\n    let e: Result<bool, Error> = mk_s().parse();\n    let f: Result<usize, Error> = \"3\".parse();\n    let g = a.ok();\n    let h = e.is_ok();",
    // `Error` (M5a spec 2.3, 3): `Error::new` takes a literal, a stored
    // `string` (moved), or a new one; `message` is part of its error,
    // used before the error moves; `{}` prints one, and `==` and
    // `.clone()` work on it.
    "let e = Error::new(ls);\n    let f = Error::new(\"lit\");\n    let g = Error::new(mk_s());\n    let m = e.message();\n    read_s(m);\n    println!(\"{} {}\", f, g.message());\n    let b = e == f;\n    let h = e.clone();\n    let es = vec![e, h];\n    read_s(es[0].message());",
    // `json` (M5a spec 2.4, 3): `parse` reads a literal, a stored
    // `string`, a field, or a new one into each kind of convertible type;
    // `stringify` reads a local, a parameter, a `mut` parameter, an
    // element, a field, a temporary, a literal, an `if`, and a container,
    // leaving each usable after it.
    "let r: Result<P, Error> = json::parse(ps);\n    let a: Result<Vec<P>, Error> = json::parse(ls);\n    let b: Result<HashMap<string, Option<u8>>, Error> = json::parse(lp.s);\n    let d: Result<string, Error> = json::parse(mk_s());\n    let e: Result<Vec<f64>, Error> = json::parse(\"[1.5]\");\n    let f = json::stringify(lp);\n    read_s(json::stringify(pp));\n    let g = json::stringify(mp);\n    let h = json::stringify(lv[0]) == json::stringify(mk_p());\n    let i = json::stringify(\"lit\");\n    let j = json::stringify(lp.s);\n    let k = json::stringify(if c { lp } else { pp });\n    let m = json::stringify(mk_m());\n    let n = json::stringify(lk);\n    println!(\"{} {} {}\", f, json::stringify(lw), json::stringify(ll));\n    read_p(lp);\n    lv.push(mk_p());\n    read_s(ls);",
    // `env::parse` (M5a spec 2.5) makes a flat struct where a `Result` is
    // expected; the struct and its `Error` stay usable.
    "let r: Result<P, Error> = env::parse();\n    match r {\n        Ok(v) => read_p(v),\n        Err(e) => read_s(e.message()),\n    }",
    // `log` (M5a spec 2.6) reads its arguments, leaving each usable after.
    "log::debug(\"{}\", ls);\n    log::info(\"{} {}\", lp.s, li);\n    log::warn(\"plain\");\n    log::error(\"{}\", mk_s());\n    read_s(ls);\n    read_p(lp);",
    "let mut bs = vec![true, c];\n    bs.sort();\n    let b = bs.contains(c);\n    let fs = vec![1.5];\n    let f = fs.contains(1.5);",
    "let mut im: HashMap<i32, bool> = HashMap::new();\n    im.insert(li, c);\n    let b = im.contains_key(li) && im.contains_key(pi) && im.contains_key(3);\n    let mut bm: HashMap<bool, HashMap<u8, string>> = HashMap::new();\n    let old = bm.insert(c, HashMap::new());\n    read_i(bm.len() as i32);",
    // The changing rows of a `Vec` of strings, of structs, and of
    // integers, on locals, `mut` parameters, and fields.
    "lw.insert(0, mk_s());\n    let x = lw.remove(0);\n    lw.sort();\n    read_s(x);\n    let mut ws = mk_words();\n    ws.w.insert(0, ls);\n    ws.w.sort();\n    lw.push(ws.w.remove(0));",
    "lv.insert(0, mk_p());\n    read_p(lv.remove(0));\n    mv.insert(0, mk_p());\n    let x = mv.remove(0);\n    mq.v.insert(0, x);\n    let b = lv.is_empty() || mv.is_empty() || pv.is_empty() || lq.v.is_empty();",
    "ln.sort();\n    let b = ln.contains(li) && ln.contains(pi) && ln.contains(mi) && ln.contains(3) && ln.contains(lp.n);\n    ln.insert(0, mi);\n    read_i(ln.remove(0));\n    let e = ln.is_empty();\n    let d = vec![1, 2].contains(li);",
    // `contains` on a `Vec<string>` in each position, a statement's start
    // included.
    "lw.contains(ps) == false;\n    lw.contains(ps);\n    lw.contains(ls) || c;\n    if lw.contains(ps) {\n        read_s(ps);\n    }\n    while !lw.contains(\"zz\") {\n        break;\n    }\n    read_i(if mk_vs().contains(ll) { 1 } else { 2 });",
    // `Option` and `Result` rows on places and temporaries.
    "let a = lo.is_some() && lk.is_some() && loe.is_some() && mk_o().is_some() && lr.is_ok() && pr.is_err() && mk_r().is_ok() && lrv[0].is_err() && mk_rs().is_ok();",
    // The changing rows of a `HashMap`, on a local and a field.
    "let mut lm = mk_m();\n    let a = lm.insert(\"k\", li);\n    let b = lm.insert(mk_s(), 2);\n    lm.insert(ls, 3);\n    let mut lh = mk_hm();\n    lh.m.insert(lp.s.clone(), pi);\n    read_i(lm.len() as i32 + lh.m.len() as i32);",
    // Taking rows (M4 spec 3.4) on temporaries, owned locals, and stored
    // values that are Copy in Rust, an `if` receiver included.
    "let a = mk_o().unwrap_or(mk_p());\n    let b = mk_r().ok();\n    let d = mk_rs().unwrap_or(\"x\");\n    let e = mk_o().ok_or(\"none\");\n    let f = mk_rs().ok();",
    "let a = lo.unwrap_or(mk_p());\n    let b = lr.ok();\n    let d = loe.ok_or(1);\n    let e = lk.unwrap_or(0);\n    read_p(a);",
    "let h = mk_held();\n    let a = h.o.unwrap_or(0) + h.r.unwrap_or(1);\n    let b = h.o.ok_or(false);\n    let d = h.r.ok();\n    let e = h.o.unwrap_or(2);\n    let vo = vec![Some(1), None];\n    for o in vo {\n        read_i(o.unwrap_or(0));\n        read_i((if c { o } else { Some(5) }).unwrap_or(0));\n        read_i((if c { o } else { h.o }).ok_or(1).unwrap_or(3));\n    }",
    // Looked-into `get` heads (M4 spec 2.8): a `Vec<string>` in each head
    // position, on a local, a parameter, and a field; a Copy binding at
    // depth; a `HashMap` with a struct value; and `get` on Copy payloads.
    "match lw.get(0) {\n        Some(s) => read_s(s),\n        None => {}\n    }\n    if let Some(s) = lw.get(1) {\n        read_s(s);\n    }\n    let mut i: usize = 0;\n    while let Some(p) = pv.get(i) {\n        read_p(p);\n        i = i + 1;\n    }\n    if let Some(p) = mv.get(0) {\n        read_s(p.s);\n    }\n    if let Some(p) = lq.v.get(0) {\n        read_p(p);\n    }\n    lw.push(mk_s());",
    "let shapes = vec![Shape::Circle(1.5), Shape::Dot];\n    match shapes.get(0) {\n        Some(Shape::Circle(r)) => {\n            let a = area(r);\n        }\n        _ => {}\n    }\n    let vo = vec![Some(1), None];\n    if let Some(Some(n)) = vo.get(0) {\n        read_i(n);\n    }",
    "let mut hp: HashMap<string, P> = HashMap::new();\n    hp.insert(\"k\", mk_p());\n    if let Some(p) = hp.get(ps) {\n        read_p(p);\n        read_i(p.n);\n    }\n    match hp.get(ls) {\n        Some(p) => read_s(p.s),\n        None => {}\n    }",
    "let m = mk_m();\n    let a = m.get(ps).unwrap_or(0) + m.get(\"k\").unwrap_or(1) + mk_m().get(ls).unwrap_or(2);\n    let b = ln.get(0).unwrap_or(0) + vec![1].get(0).unwrap_or(3);\n    let o = ln.get(1);\n    ln.push(4);\n    read_i(o.unwrap_or(0));",
    // Borrowed returns (M4 spec 3.1) rooted at `self`, a string
    // parameter, a `Vec` parameter, one of two reference parameters, and
    // through another call; on a `for` variable and an element; as `match`,
    // `for`, and `if let` heads (a Copy binding copied out) and as a
    // `trim` receiver; a looked-into `get` on one; `trim` on each receiver.
    "let g = mk_g();\n    read_s(g.name_ref());\n    let n = g.name_ref();\n    read_s(n);\n    read_s(g_name(g));\n    let q = g.first_item();\n    read_p(q);\n    read_i(g.first_item().n);",
    "read_s(trimmed(ps));\n    read_s(trimmed(ms));\n    let t = trimmed(ls);\n    read_s(t);\n    read_s(trimmed(\"  x \"));\n    read_s(trimmed(lp.s));",
    "let f = first_of(pv);\n    read_p(f);\n    read_s(first_of(lv).s);\n    read_s(s_of(first_of(mv)));\n    read_s(s_unless(pp, ps));\n    let n = s_unless(lp, \"x\");\n    read_s(n);\n    read_s(s_unless(mp, ls));",
    "for p in lv {\n        read_s(s_of(p));\n    }\n    read_s(s_of(lv[0]));\n    read_s(s_of(lq.p));\n    let s = s_of(lp);\n    lp.n = 3;\n    ll = s_of(pp);\n    read_s(ll);",
    "let g = mk_g();\n    match g.shape() {\n        Shape::Circle(r) => {\n            let a = area(r);\n        }\n        Shape::Dot => {}\n    }\n    if let Shape::Circle(r) = g.shape() {\n        let a = area(r);\n    }\n    for t in g.tags() {\n        read_s(t);\n    }\n    match g.name_ref() {\n        \"g\" => {}\n        other => read_s(other),\n    }\n    read_s(g.name_ref().trim());\n    match g.tags().get(0) {\n        Some(t) => read_s(t),\n        None => {}\n    }",
    // A borrowed return of a type that is Copy in Rust (`Option<i32>`) as
    // a head, its Copy binding copied out, and taken by `unwrap_or`.
    "let h = mk_held();\n    match held_o(h) {\n        Some(n) => read_i(n),\n        None => {}\n    }\n    if let Some(n) = held_o(h) {\n        read_i(n);\n    }\n    while let Ok(n) = held_r(h) {\n        read_i(n);\n        break;\n    }\n    read_i(held_o(h).unwrap_or(0));\n    let o = held_o(h);\n    read_i(o.unwrap_or(1));",
    "read_s(ps.trim());\n    read_s(ms.trim());\n    read_s(ls.trim());\n    read_s(ll.trim());\n    read_s(\"  x \".trim());\n    read_s(lp.s.trim());\n    read_s(lw[0].trim());\n    let t = ls.trim();\n    read_s(t);\n    let mut u = \"u\";\n    u = ps.trim();\n    read_s(u);",
    // Closures (M4 spec 2.2, 3.2): a block body capturing each kind of
    // name, reading, comparing, and passing each on; `let y = name;`
    // inside one, which leaves `name` usable after it; `self` and `mut
    // self` captured by a method; `map_err` capturing a parameter; a
    // parameter owning its payload; a string literal made owned; nested
    // closures; `match`, `for`, and a loop of the closure's own inside
    // one.
    "let n = lp.s;\n    let a = lk.map(|x| {\n        let y = ls;\n        read_s(y);\n        read_s(ls);\n        read_s(ps);\n        read_s(ms);\n        read_s(n);\n        read_s(ll);\n        read_i(li);\n        read_p(lp);\n        read_p(pp);\n        read_p(mp);\n        read_s(mp.s);\n        let b = ls == ps && ms == n && ll == ls && lp.s == ps && y == ms && n == ll;\n        x + li + lp.n + mp.n + pi + mi\n    });\n    read_s(ls);\n    read_s(n);\n    ls = mk_s();\n    change_s(ms);",
    "let t = lp.tagged(lk);\n    read_i(lp.bumped(lk).unwrap_or(0));\n    read_s(pp.tagged(Some(1)).unwrap_or(\"x\"));",
    "let r = mk_rs().map_err(|e| format!(\"{}: {}\", ps, e));\n    let o = Some(mk_s()).map(|s| {\n        let t = s;\n        t.len()\n    });\n    let p = mk_o().map(|p| p.s.clone());\n    let q = mk_o().map(|p| {\n        read_p(p);\n        p\n    });",
    "let b = lk.map(|x| if x > 1 { \"big\" } else { \"small\" });\n    read_s(b.unwrap_or(\"none\"));\n    let d = Some(4).map(|n| n * 2).is_some();",
    "let a = Some(1).map(|x| Some(2).map(|y| x + y + li));\n    let b = Some(3).map(|x| match lo {\n        Some(p) => p.n + x,\n        None => x,\n    });\n    let d = lk.map(|x| {\n        let mut t = x;\n        for p in lv {\n            t = t + p.n;\n        }\n        while t > 100 {\n            t = t - 1;\n            if t == 50 {\n                break;\n            }\n        }\n        t\n    });\n    lv.push(mk_p());",
    // Chains (M4 spec 2.3, 3.3): borrowed items, copies, and owned items
    // through every terminal; a `find` on copies and on owned items is a
    // plain `Option`, and `keys()` over integer keys copies.
    "let t = mk_team();\n    let a = t.names.iter().count();\n    let b = t.names.iter().any(|w| w.len() > 1) && t.names.iter().all(|w| w == ps);\n    let d = t.names.iter().filter(|w| w.len() > 1).count();\n    if let Some(w) = t.names.iter().find(|w| w.len() > 1) {\n        read_s(w);\n    }\n    let e = ln.iter().sum();\n    let f: Vec<i32> = ln.iter().collect();\n    let g = ln.iter().filter(|n| n > 1).count() + ln.iter().map(|n| n as usize).sum();\n    let h = ln.iter().any(|n| n > li) || ln.iter().all(|n| n < 3);\n    read_i(ln.iter().find(|n| n > 1).unwrap_or(0));\n    let i: Vec<string> = t.names.iter().map(|w| w.clone()).collect();\n    let j = t.names.iter().map(|w| w.clone()).find(|w| w.len() > 0);\n    read_s(j.unwrap_or(\"none\"));\n    let k = t.names.iter().map(|w| w.clone()).filter(|w| w.len() > 0).count();\n    let m = t.names.iter().map(|w| w.clone()).all(|w| w.len() > 0);\n    let mut hm: HashMap<i32, string> = HashMap::new();\n    hm.insert(1, \"a\");\n    let n = hm.keys().sum();\n    let o = hm.keys().filter(|k| k > 0).count();\n    let q: Vec<string> = hm.values().map(|v| v.clone()).collect();\n    let mm = mk_m();\n    let r: Vec<string> = mm.keys().map(|k| k.clone()).collect();\n    let s = mm.values().sum() + mm.values().filter(|v| v > 1).count() as i32;\n    if let Some(key) = mm.keys().find(|k| k.len() > 0) {\n        read_s(key);\n    }",
    // What a `map` closure returns decides the items: part of the item,
    // part of a capture, a borrowed return, a copy, something new, a
    // literal made owned; the parts read on, never collected.
    "let t = mk_team();\n    let a = t.users.iter().map(|u| u.name).any(|n| n.len() > 0);\n    let b = t.users.iter().map(|u| u.name).filter(|n| n.len() > 0).count();\n    let d = t.users.iter().map(|u| u.name.len()).sum();\n    let e: Vec<string> = t.users.iter().map(|u| u.name.clone()).collect();\n    let f = ln.iter().map(|x| lp.s).count() + ln.iter().map(|x| ps).count();\n    let g = t.users.iter().map(|u| u.shape).count();\n    let h: Vec<string> = ln.iter().map(|n| if n > 1 { \"big\" } else { \"small\" }).collect();\n    let i = t.users.iter().map(|u| u.name_ref()).all(|n| n == ps);\n    let j = lw.iter().map(|w| w.trim()).all(|w| w.len() > 0);\n    let k = t.users.iter().map(|u| u.items).map(|v| v.len()).sum();\n    let m = t.users.iter().map(|u| if c { u.name } else { u.name }).count();\n    let n = lv.iter().map(|p| p).filter(|p| p.n > 0).map(|p| p.s).any(|s| s == ls);\n    let o = lw.iter().map(|w| {\n        let x = w;\n        x\n    }).count();",
    // A `find` on borrowed items looked into by `match`, `if let`, and
    // `while let`; after `split` and after a borrowed return, its binding
    // passed to a `string` parameter.
    "let t = mk_team();\n    match t.names.iter().find(|w| w.len() > 1) {\n        Some(w) => read_s(w),\n        None => {}\n    }\n    if let Some(w) = t.names.iter().find(|w| w.len() > 1) {\n        read_s(w);\n    }\n    while let Some(w) = t.names.iter().find(|w| w.len() > 1) {\n        read_s(w);\n        break;\n    }\n    if let Some(piece) = ls.split(\" \").find(|p| p.len() > 0) {\n        read_s(piece);\n    }\n    if let Some(piece) = \"a b\".split(\" \").find(|p| p.len() > 0) {\n        read_s(piece);\n    }\n    if let Some(n) = t.users.iter().map(|u| u.name_ref()).find(|n| n.len() > 0) {\n        read_s(n);\n    }\n    if let Some(u) = t.users.iter().find(|u| u.name.len() > 0) {\n        read_s(u.name);\n        read_s(g_name(u));\n    }\n    if let Some(p) = lv.iter().find(|p| p.n > 0) {\n        read_p(p);\n        read_i(p.n);\n    }\n    match lve.iter().find(|e| true) {\n        Some(E::A(s)) => read_s(s),\n        Some(E::B(p)) => read_i(p.n),\n        _ => {}\n    }",
    // `filter` on pieces of text (`&&str`), `any` over owned items passed
    // to a Varyk function, a chain inside a closure body, and the source's
    // root changed once the chain is done.
    "let a = ls.split(\" \").filter(|w| w.len() > 0).count() + ps.split(\",\").filter(|w| w == ps).count() + ms.split(\",\").filter(|w| w.len() > 0).count();\n    let b = ls.split(sep_of(ps)).map(|w| w).any(|w| w == ll);\n    let d = lw.iter().map(|w| w.clone()).any(|w| {\n        read_s(w);\n        w.len() > 0\n    });\n    let e = lk.map(|x| lw.iter().filter(|w| w.len() > 0).count());\n    let f = Some(ln).map(|v| v.iter().sum());\n    lw.push(mk_s());\n    ls = mk_s();",
    // A `for` over a chain (M4 spec 3.3): borrowed items, copies, and
    // owned items, from every source, one of them given away.
    "let t = mk_team();\n    let mm = mk_m();\n    for w in t.names.iter().filter(|w| w.len() > 1) {\n        read_s(w);\n    }\n    for n in ln.iter() {\n        read_i(n);\n    }\n    for n in ln.iter().filter(|n| n > li) {\n        read_i(n);\n    }\n    for s in t.names.iter().map(|w| w.clone()) {\n        read_s(s);\n        let kept = s;\n    }\n    for n in t.users.iter().map(|u| u.name) {\n        read_s(n);\n    }\n    for k in mm.keys() {\n        read_s(k);\n    }\n    for v in mm.values().filter(|v| v > 0) {\n        read_i(v);\n    }\n    for piece in \"a b\".split(\" \") {\n        read_s(piece);\n    }",
    // What the head reads is free once the loop ends, and a copy of the
    // loop variable taken out of it does not hold it; a piece of text
    // passed to a `string` parameter.
    "let mut limit: usize = 1;\n    let mut best = \"\";\n    for w in lw.iter().filter(|w| w.len() > limit) {\n        best = w;\n    }\n    limit = 0;\n    read_s(best);\n    let mut sep = mk_s();\n    let mut last = \"\";\n    for w in ls.split(sep) {\n        last = w;\n    }\n    sep.push_str(\"x\");\n    read_s(last);\n    for w in ls.split(\" \") {\n        read_s(w);\n    }",
    // A `HashMap` moved into a struct and passed read-only.
    "let m = mk_m();\n    read_m(m);\n    let h = Hm { m: mk_m(), n: 2 };\n    read_m(h.m);\n    let mut v: Vec<HashMap<string, i32>> = Vec::new();\n    v.push(mk_m());\n    let n = v[0].len();",
    // `.clone()` and `==` (M4 spec 2.10) on a struct, an enum, and each
    // container of a comparable type, owned, borrowed, changeable, and
    // new, in every mix: the operands are brought to one reference depth.
    "let a = lv.clone();\n    let b = a == lv && lv == pv && pv == a && pv != mv && mv == lv && mk_ov() == Some(pv.clone());\n    let o = lo.clone();\n    let d = o == lo && lo == mk_o() && mk_o() != lo;\n    let r = lr.clone();\n    let e = r == lr && pr == lr && mk_r() == pr && lr != mk_r();\n    let m = mk_m();\n    let m2 = m.clone();\n    let f = m == m2 && m2 != mk_m() && mk_hm().m == m;\n    let w = lw.clone();\n    let g = w == lw && mk_vs() == w;\n    let q = lq.clone();\n    let h = q == lq && lq == mq && mk_q() == mq && mq.e == pe;\n    let x = le.clone();\n    let i = x == le && pe == le && pe == E::C && me != pe && lf == mk_f() && lf.clone() == lf;\n    let k = lk.clone() == lk && lrv == lrv.clone() && loe == mk_oe();",
    // An owned local, a parameter, a `mut` parameter, a borrowed return,
    // an element, a field, and block-like operands compared, and copies
    // made from each and given away.
    // Async functions and methods awaited (milestone 5b1 spec 2.3), their
    // arguments lent as any call's are; `?` after `.await` is in the
    // prelude's `a_try`.
    "async: let n = a_len(ls).await;\n    let q = a_p(lp).await;\n    let m = lp.a_n().await + pp.a_n().await + mp.a_n().await;\n    let k = a_try(li).await;\n    let j = a_len(ps).await + a_len(ms).await + a_len(lp.s).await;\n    read_p(q);\n    read_s(ls);",
    // `.await` inside `for` (over a `Vec` and over a chain with a
    // closure), `while`, `match`, `if let`, and `println!`.
    "async: for p in lv {\n        let n = p.a_n().await;\n    }\n    for s in lw.iter().filter(|w| w.len() > 0) {\n        let n = a_len(s).await;\n    }\n    for i in 0..2 {\n        let n = a_i(i).await;\n    }",
    "async: while c {\n        let n = a_len(ps).await;\n        break;\n    }\n    while a_i(li).await > 100 {\n        li = li + 1;\n    }",
    "async: match a_r(li).await {\n        Ok(n) => read_i(n),\n        Err(e) => read_s(e),\n    }\n    match lo {\n        Some(p) => {\n            let n = p.a_n().await;\n        }\n        None => {}\n    }",
    "async: if let Ok(n) = a_r(li).await {\n        read_i(n);\n    }\n    if let Some(p) = lo {\n        let q = a_p(p).await;\n    }",
    "async: println!(\"{} {}\", a_len(ls).await, lp.a_n().await);\n    let t = format!(\"{}\", a_i(mi).await);\n    read_s(t);",
    // Started calls (milestone 5b1 spec 2.3, 3): every kind of argument
    // given to a task that owns it (a number, an owned local, a string
    // literal and a literal local, a `.clone()`, a `mut` number
    // parameter), kept in a `let` and awaited.
    "async: let t1 = a_i(li);\n    let t2 = a_len(ls);\n    let t3 = a_p(lp);\n    let t4 = a_len(\"lit\");\n    let t5 = a_p(pp.clone());\n    let t6 = a_len(ms.clone());\n    let t7 = a_i(pi);\n    let t8 = a_len(ll);\n    let t9 = a_r(mi);\n    let n = t1.await + t7.await;\n    let k = t2.await + t4.await + t6.await + t8.await;\n    read_p(t3.await);\n    read_p(t5.await);\n    let r = t9.await;",
    // Detached at once and from a local.
    "async: a_i(li).detach();\n    let t = a_len(ls);\n    t.detach();\n    a_p(lp).detach();\n    let u = a_p(mk_p());\n    u.detach();",
    // A started method, its receiver an owned local, a call result, and
    // a struct literal; an awaited argument; `time::sleep`.
    "async: let t = lp.a_n();\n    let u = mk_p().a_n();\n    let w = P { s: \"x\", n: 2 }.a_n();\n    let n = t.await + u.await + w.await;\n    let a = a_p(a_p(mk_p()).await);\n    read_p(a.await);\n    let s = time::sleep(10);\n    s.await;",
    // Async functions with `.await` in `for` (over a chain with a
    // closure among them), `while`, `match`, `if let`, and `println!`,
    // started, so their futures are checked `Send`.
    "async: let t1 = a_loops(lv, lw);\n    let t2 = a_while(c, ls);\n    let t3 = a_branches(li, lo);\n    let t4 = a_print(mk_s(), lp);\n    let n = t1.await + t2.await + t3.await;\n    read_s(t4.await);",
    // Started inside loops.
    "async: for i in 0..3 {\n        let t = a_i(i);\n        let n = t.await;\n    }\n    let mut k = 0;\n    while k < 2 {\n        a_i(k).detach();\n        k = k + 1;\n    }",
    // `Task::all` and `Task::all_settled` (milestone 5b1 spec 2.5) over
    // a `vec!` of started calls and over a local holding one; plain tasks
    // (`Task::all`), tasks giving a `Result` (`Task::try_all`), and
    // `all_settled`; a started method in `vec!` and detached.
    "async: let a = Task::all(vec![a_i(li), a_i(pi), a_i(mi), lp.clone().a_n()]).await;\n    pp.clone().a_n().detach();\n    let ts = vec![a_r(1), a_try(li)];\n    let b = Task::all(ts).await;\n    let c = Task::all_settled(vec![a_r(li), a_try(3)]).await;\n    let us = vec![a_p(lp), a_p(pp.clone())];\n    let d = Task::all(us).await;\n    read_i(a[0]);\n    read_p(d[0]);",
    // ... and over a collected `map` of started calls: over copied
    // numbers, and over borrowed structs and strings given as `.clone()`,
    // a started method among them.
    "async: let a = Task::all(ln.iter().map(|x| a_i(x)).collect()).await;\n    let b = Task::all(pv.iter().map(|p| a_p(p.clone())).collect()).await;\n    let c = Task::all(lw.iter().map(|w| a_len(w.clone())).collect()).await;\n    let d = Task::all(ln.iter().map(|x| a_r(x)).collect()).await;\n    let e = Task::all_settled(lw.iter().map(|w| a_r(w.len() as i32)).collect()).await;\n    let f = Task::all(lv.iter().map(|p| p.clone().a_n()).collect()).await;\n    let g = Task::all_settled(mv.iter().map(|p| {\n        a_try(p.n)\n    }).collect()).await;\n    read_p(b[0]);",
    // `Shared<T>` (milestone 5b1 spec 2.6, 5): `Arc<T>`, its fields read
    // and lent, copied, and aliased through auto-deref.
    "let s = Shared::new(mk_cfg());\n    let n = s.factor + 1;\n    read_s(s.name);\n    read_p(s.items[0]);\n    let t = s.name;\n    read_s(t);\n    let k = s.items[0].n + s.items[0].s.len() as i32;\n    read_v(s.items);\n    read_e(s.e);\n    let mut v: Vec<string> = Vec::new();\n    v.push(s.name.clone());\n    let p = s.items[0].clone();\n    read_p(p);\n    let u: Shared<Cfg> = Shared::new(Cfg { factor: 1, name: \"lit\", tags: lw.clone(), items: vec![], e: E::C });\n    read_i(u.factor);",
    // ... its methods by path, on a local and a parameter, and the
    // handle lent and cloned.
    "let s = Shared::new(mk_cfg());\n    let n = s.twice() + read_shared(s);\n    read_s(s.cname());\n    let u = s.clone();\n    let m = read_shared(u) + u.twice();\n    let hs = vec![s.clone(), u.clone()];\n    for h in hs {\n        read_i(h.factor);\n    }\n    let o = Some(s.clone());\n    match o {\n        Some(h) => read_s(h.name),\n        None => {}\n    }",
    // ... table reads and chains through it.
    "let s = Shared::new(mk_cfg());\n    let a = s.items.len() + s.tags.len() + s.name.len();\n    let b = s.tags.contains(\"a\") && s.name.contains(\"c\") && !s.items.is_empty();\n    read_s(s.name.trim());\n    let c = s.tags.iter().filter(|t| t.len() > 1).count();\n    let d: Vec<string> = s.tags.iter().map(|t| t.clone()).collect();\n    let w = s.tags.join(\",\");\n    match s.tags.get(0) {\n        Some(t) => read_s(t),\n        None => {}\n    }",
    // ... a `match`, `if let`, and `for` on its fields.
    "let s = Shared::new(mk_cfg());\n    match s.e {\n        E::A(x) => read_s(x),\n        E::B(p) => read_p(p),\n        E::C => {}\n    }\n    if let E::A(x) = s.e {\n        read_s(x);\n    }\n    for p in s.items {\n        read_p(p);\n    }\n    for t in s.tags.iter() {\n        read_s(t);\n    }",
    // ... and given to started calls as `s.clone()`, then moved into the
    // last; a started method through it.
    // `.rs` async functions and methods (milestone 5b1 spec 2.8), awaited
    // and started as Varyk ones are: an owned parameter, a `&self`
    // method, and a `&mut` parameter awaited.
    "async: let n = ext::a_take(ls).await;\n    let t = ext::a_take(mk_s());\n    let u = ext::a_take(ms.clone());\n    let w = ext::a_take(\"lit\");\n    let k = n + t.await + u.await + w.await;",
    "async: let tl = ext::Tally::new();\n    let a = tl.a_count().await + pt.a_count().await;\n    let t = tl.a_count();\n    let u = ext::Tally::new().a_count();\n    let k = a + t.await + u.await;",
    "async: let mut tl = ext::Tally::new();\n    ext::a_bump_tally(tl).await;\n    read_i(tl.count());",
    "async: let s = Shared::new(mk_cfg());\n    let t1 = a_cfg(s.clone());\n    let t2 = a_cfg(s.clone());\n    let n = t1.await + t2.await;\n    let all = Task::all(vec![a_cfg(s.clone()), a_cfg(s.clone())]).await;\n    let m = s.clone().a_twice();\n    let k = m.await + s.a_twice().await;\n    let t3 = a_cfg(s);\n    read_i(t3.await);",
    // Trailing values (milestone 5b3 spec 2.2): a string and an
    // `Option<string>` are read, and usable after the call.
    "ext::bind(\"q\", ls, ll, li, lp.s, lw[0]);\n    read_s(ls);\n    read_s(ll);\n    read_p(lp);\n    lw.push(mk_s());",
    "ext::bind(\"q\", Some(\"Ada\"), Some(ls), Some(lp.s), Some(li));\n    read_s(ls);\n    read_p(lp);",
    "let o: Option<string> = Some(mk_s());\n    let n = ext::bind(\"q\", o, ps) + ext::bind(\"q\", o);\n    match o {\n        Some(t) => read_s(t),\n        None => {}\n    }",
    // A started call passes literal text as it is, and reads its
    // trailing values before the task starts.
    "async: let t = ext::a_one(\"q\");\n    let n = t.await + ext::a_one(\"r\").await;",
    "async: let t = ext::a_bind(\"q\", ls, li);\n    read_s(ls);\n    let n = t.await + ext::a_bind(\"q\", lp.s).await;\n    read_p(lp);",
    "let b = lp == pp && pp == lp && mp == pp && lp == first_of(pv) && first_of(lv) == mp && lv[0] == pp && lq.p == pp && pp == lq.p && (if c { lp } else { pp }) == mp && pp == { mk_p() } && (if c { mk_p() } else { mk_p() }) == lp;\n    let p2 = pp.clone();\n    read_p(p2);\n    let v2 = pv.clone();\n    read_v(v2);\n    let p3 = first_of(pv).clone();\n    let mut q = Q { p: p3, n: 1, e: pe.clone(), v: mv.clone() };\n    change_q(q);\n    let copies: Vec<P> = lv.iter().map(|p| p.clone()).collect();\n    let same = copies == lv && lv.iter().any(|p| p == pp) && lv.iter().filter(|p| p != mp).count() > 0;\n    lv.push(pp.clone());\n    let last = lv[0].clone();\n    read_p(last);",
];

/// Programs `check` must reject, as (body, the code `check` must reject
/// it with, the code rustc rejects the Rust the backend makes of it
/// with): accepting one is unsound. The rustc code is empty for a case
/// in [`STRICTER_THAN_RUST`] or [`UNEMITTED`].
const MUST_REJECT: &[(&str, &str, &str)] = &[
    // Nothing reached through a `Shared` changes (milestone 5b1 spec
    // 2.6): an assignment, a changing table call, and a `mut` parameter.
    (
        "let mut s = Shared::new(mk_cfg());\n    s.factor = 3;",
        "V0310",
        "E0594",
    ),
    (
        "let mut s = Shared::new(mk_cfg());\n    s.items.push(mk_p());",
        "V0310",
        "E0596",
    ),
    (
        "let mut s = Shared::new(mk_cfg());\n    let n = pick(s.items);",
        "V0310",
        "E0596",
    ),
    // A started call's arguments are owned slots the task keeps
    // (milestone 5b1 spec 3): a parameter, an element, a value moved in
    // and used again, a task awaited twice, and one detached inside a
    // closure; and no task may be lent to a `mut` parameter or `mut self`.
    (
        "async: let t = a_p(pp);\n    read_p(t.await);",
        "V0304",
        "E0521",
    ),
    (
        "async: let t = a_p(lv[0]);\n    read_p(t.await);",
        "V0304",
        "E0507",
    ),
    (
        "async: let t = a_len(pp.s);\n    t.await;",
        "V0304",
        "E0507",
    ),
    (
        "async: let t = a_p(lp);\n    read_p(lp);\n    read_p(t.await);",
        "V0305",
        "E0382",
    ),
    (
        "async: let t = a_i(li);\n    let a = t.await;\n    let b = t.await;",
        "V0305",
        "E0382",
    ),
    (
        "async: let t = a_i(li);\n    let n: Vec<i32> = ln.iter().map(|x| {\n        t.detach();\n        x\n    }).collect();",
        "V0304",
        "E0507",
    ),
    ("async: let t = a_bump(lp);\n    t.await;", "V0309", "E0308"),
    // A collected `map` of started calls given borrowed items, and a
    // list of tasks given to `Task::all` twice.
    (
        "async: let c = Task::all(lw.iter().map(|w| a_len(w)).collect()).await;",
        "V0304",
        "E0597",
    ),
    (
        "async: let b = Task::all(lv.iter().map(|p| a_p(p)).collect()).await;",
        "V0304",
        "E0597",
    ),
    (
        "async: let ts = vec![a_i(li)];\n    let a = Task::all(ts).await;\n    let b = Task::all(ts).await;",
        "V0305",
        "E0382",
    ),
    ("async: let t = lp.a_inc();\n    t.await;", "V0309", "E0308"),
    // A started `.rs` async function given a `&mut` parameter: the task
    // lends its own copy as `&`, so the Rust does not build either.
    (
        "async: let mut tl = ext::Tally::new();\n    let t = ext::a_bump_tally(tl);\n    t.await;",
        "V0309",
        "E0308",
    ),
    // A `Vec` of borrowed items has no Varyk type (M4 spec 3.3), and a
    // looked-into `find` on them none of its own (M4 spec 2.8).
    (
        "let v: Vec<string> = lw.iter().collect();",
        "V0304",
        "E0308",
    ),
    (
        "let o: Option<string> = lw.iter().find(|w| w.len() > 0);",
        "V0208",
        "E0308",
    ),
    (
        "take_os(lw.iter().find(|w| w.len() > 0));",
        "V0208",
        "E0308",
    ),
    // The body of a `for` over a chain may not change what the head
    // reads (M4 spec 3.3): its source, a capture of its closures, numbers
    // included, a capture that is an alias, or the source's argument.
    (
        "for w in lw.iter() {\n        lw.push(mk_s());\n    }",
        "V0307",
        "E0502",
    ),
    (
        "let mut seen: Vec<string> = vec![];\n    for w in lw.iter().filter(|w| seen.contains(w)) {\n        seen.push(w.clone());\n    }",
        "V0307",
        "E0502",
    ),
    (
        "for n in ln.iter().filter(|n| n < li) {\n        li = li + 1;\n    }",
        "V0307",
        "E0506",
    ),
    (
        "let first = lw[0];\n    for p in lv.iter().filter(|p| p.s == first) {\n        lw.push(mk_s());\n    }",
        "V0307",
        "E0502",
    ),
    (
        "let mut sep = mk_s();\n    for w in ls.split(sep) {\n        sep.push_str(\"x\");\n    }",
        "V0307",
        "E0502",
    ),
    (
        "let mut a = lv[0];\n    for w in lw.iter().filter(|w| a.n > 0) {\n        let k = lv.len();\n    }",
        "V0307",
        "E0502",
    ),
    (
        "let sep = lp.s;\n    for w in ls.split(sep) {\n        lp.s = \"x\";\n    }",
        "V0307",
        "E0506",
    ),
    // A closure is a value of its own (M4 spec 2.2): `break` inside one
    // would not leave the loop around it.
    (
        "while c {\n        let a = lk.map(|x| {\n            break;\n        });\n    }",
        "V0001",
        "E0267",
    ),
    // A closure only reads a name from outside it (M4 spec 3.2): a
    // captured owned name kept by a `let` of the closure, or returned from
    // an `Option::map` closure, would move it out; so would part of the
    // closure's parameter, which ends with the closure.
    (
        "let a = lk.map(|x| {\n        let mut s = \"\";\n        s = ls;\n        s\n    });\n    read_s(ls);",
        "V0304",
        "E0382",
    ),
    ("let a = lk.map(|x| ls);\n    read_s(ls);", "V0304", "E0382"),
    ("let a = Some(mk_s()).map(|s| s.trim());", "V0304", ""),
    ("let a = lk.map(|x| ps);", "V0304", ""),
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
    ), // A string binding of a `match` on a temporary string kept past the
    // `match`: the head is a `&str` into a temporary (M4 spec 3.5).
    (
        "match mk_s() {\n        \"a\" => {}\n        other => {\n            ll = other;\n        }\n    }\n    read_s(ll);",
        "V0304",
        "E0716",
    ),
    // A string literal below the top of a pattern (M4 spec 2.5): no Rust,
    // since it does not type-check; rustc says E0308 to a `String` matched
    // against a literal there.
    (
        "match mk_rs() {\n        Ok(\"yes\") => {}\n        _ => {}\n    }",
        "V0205",
        "",
    ),
    // A `HashMap` moves like a `Vec` (M4 spec 2.7).
    (
        "let m = mk_m();\n    let m2 = m;\n    let n = m.len();",
        "V0305",
        "E0382",
    ),
    // An owned argument of a row is an owned slot: the backend copies the
    // `&str` rustc would accept, so Varyk is stricter here.
    ("lw.insert(0, ps);", "V0304", ""),
    // A changing row's argument that is its own receiver.
    ("ls.push_str(ls);", "V0306", "E0502"),
    // A taking row on a stored value whose contents are not Copy (M4 spec
    // 3.4).
    (
        "let rv: Vec<Result<i32, string>> = vec![Ok(1)];\n    let n = rv[0].unwrap_or(0);",
        "V0304",
        "E0507",
    ),
    // A looked-into `get` anywhere but a head (M4 spec 2.8), stored and
    // passed as an `Option<string>`. (A `let` of it alone, never used as
    // one, is stricter than Rust.)
    (
        "let o: Option<string> = lw.get(0);\n    take_os(o);",
        "V0208",
        "E0308",
    ),
    ("take_os(lw.get(0));", "V0208", "E0308"),
    // A borrowed-return result is part of its argument (M4 spec 3.1): the
    // argument changed while the result is used, and a result on a local
    // declared inside a block or a branch kept past it.
    (
        "let mut g = mk_g();\n    for t in g.tags() {\n        g.change();\n    }",
        "V0307",
        "E0502",
    ),
    (
        "let mut g = mk_g();\n    let n = g.name_ref();\n    g.change();\n    read_s(n);",
        "V0307",
        "E0502",
    ),
    (
        "let n = {\n        let u = mk_g();\n        u.name_ref()\n    };\n    read_s(n);",
        "V0304",
        "E0597",
    ),
    (
        "println!(\"{}\", if c {\n        let u = mk_g();\n        u.name_ref()\n    } else {\n        \"x\"\n    });",
        "V0304",
        "E0597",
    ),
    (
        "read_p(if c {\n        let u = mk_g();\n        u.first_item()\n    } else {\n        pp\n    });",
        "V0304",
        "E0597",
    ),
    // `trim` on a value made right there, and its result stored.
    ("let t = mk_s().trim();\n    read_s(t);", "V0001", "E0716"),
    ("lw.push(ls.trim());", "V0304", ""),
];

/// The [`MUST_REJECT`] cases rustc accepts in the Rust the backend makes
/// of them: Varyk is stricter than Rust there (no moving a field out of a
/// local, spec 3.4), or the backend makes the text an owned `String`
/// rather than generate a type error. Rejecting those is Varyk's rule, not
/// soundness. The number-parameter and `for`-variable entries at the end
/// are soundness rejections: written as borrowed returns they are rustc's
/// E0515, and the cross-check is off only because the backend then writes
/// an owned copy.
const STRICTER_THAN_RUST: &[&str] = &[
    // The backend makes the closure's value the owned one the `Option`
    // needs, with `.to_string()` on what is a `&str` in Rust.
    "let a = Some(mk_s()).map(|s| s.trim());",
    "let a = lk.map(|x| ps);",
    "let e = E::A(ps);",
    "return Err(match mk_s() {\n        \"a\" => \"x\",\n        other => other,\n    });",
    "let e = E::B(lq.p);",
    "let v = vec![lp.s];",
    "lw.insert(0, ps);",
    "lw.push(ls.trim());",
    // The same for the text a function returns from a number parameter
    // (a [`MUST_REJECT_ITEMS`] case).
    "read_s(sign(pi));",
    "read_s(sign_of(ps, pi));",
    // And from the copy a `for` makes of each number in a `Vec`.
    "read_s(first_sign(vec![1, -2]));",
    "let ns = Ns { ns: vec![1] };\n    read_s(ns.sign());",
    "read_s(grid_sign(vec![vec![1]]));",
];

/// The [`MUST_REJECT`] and [`MUST_REJECT_QUESTION`] cases that have no
/// Rust, because they do not type-check: exactly these, so a case the
/// type checker newly rejects does not silently leave the rustc
/// cross-check.
const UNEMITTED: &[&str] = &[
    "while c {\n        let a = lk.map(|x| {\n            break;\n        });\n    }",
    "match le {\n        E::A(s) => {}\n        C => {}\n    }",
    "let x = mk_r()?;",
    "match mk_rs() {\n        Ok(\"yes\") => {}\n        _ => {}\n    }",
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
    ), // A string binding of a string-literal `match` on a temporary,
    // returned: rustc says E0515 to the reference itself, which the
    // backend copies into the owned `String` the slot needs instead.
    (
        "return Err(match mk_s() {\n        \"a\" => \"x\",\n        other => other,\n    });",
        "V0304",
        "",
    ),
];

/// The return type of a case returning an `Option`.
const OPTION_RET: &str = " -> Option<string>";

/// Programs that `check` must reject, in a function returning
/// [`OPTION_RET`]: a looked-into `get` returned or used with `?` (M4 spec
/// 2.8). As [`MUST_REJECT`].
const MUST_REJECT_OPTION: &[(&str, &str, &str)] = &[
    ("return lw.get(0);", "V0208", "E0308"),
    ("let s = lw.get(0)?;\n    Some(s)", "V0208", "E0308"),
    // A looked-into `find` on borrowed items (M4 spec 3.3), the same.
    ("return lw.iter().find(|w| w.len() > 0);", "V0208", "E0308"),
    (
        "let s: string = lw.iter().find(|w| w.len() > 0)?;\n    Some(s)",
        "V0208",
        "E0308",
    ),
];

/// Programs `check` must reject for a function of their own (M4 spec
/// 3.1), as (items put before the case function, its body, the code
/// `check` must reject it with, the code rustc rejects the Rust the
/// backend makes of it with). The backend writes the borrowed return a
/// rejected function would have had, so rustc judges that.
const MUST_REJECT_ITEMS: &[(&str, &str, &str, &str)] = &[
    // Part of a `mut` parameter: the caller cannot read its argument while
    // the result is used.
    (
        "fn s_mut(mut p: P) -> string {\n    p.s\n}\n",
        "let n = s_mut(lp);\n    read_p(lp);\n    read_s(n);",
        "V0304",
        "E0502",
    ),
    // Parts of two parameters.
    (
        "fn either(a: P, b: P, c: bool) -> string {\n    if c { a.s } else { b.s }\n}\n",
        "read_s(either(pp, lp, c));",
        "V0308",
        "E0621",
    ),
    // A part beside a new value.
    (
        "fn mixed(p: P, c: bool) -> string {\n    if c { mk_s() } else { p.s }\n}\n",
        "read_s(mixed(pp, c));",
        "V0304",
        "E0515",
    ),
    // A part beside a `?`, which returns a new `None` or `Err` early.
    (
        "struct OU {\n    nick: Option<string>,\n}\n\nfn ou_nick(u: OU) -> Option<string> {\n    let p = mk_o()?;\n    u.nick\n}\n",
        "let u = OU { nick: Some(mk_s()) };\n    take_os(ou_nick(u));",
        "V0304",
        "E0277",
    ),
    (
        "fn rs_pick(r: Result<string, string>) -> Result<string, string> {\n    let s = mk_rs()?;\n    r\n}\n",
        "let r = mk_rs();\n    rs_pick(r);",
        "V0304",
        "E0277",
    ),
    // A recursive function, and a cycle of two, returning part of a
    // parameter: a call inside the group counts as new.
    (
        "fn name_of(u: P, depth: i32, mut log: Vec<string>) -> string {\n    if depth > 0 {\n        let n = name_of(u, depth - 1, log);\n        log.push(n);\n    }\n    u.s\n}\n",
        "read_s(name_of(pp, 2, lw));",
        "V0304",
        "E0308",
    ),
    (
        "fn ca(u: P) -> string {\n    cb(u)\n}\n\nfn cb(u: P) -> string {\n    let x = ca(u);\n    u.s\n}\n",
        "read_s(ca(pp));",
        "V0304",
        "E0308",
    ),
    // Part of a number parameter, which Rust passes by value: it roots
    // nothing, so the result is part of a value that ends with the call.
    // As a borrowed return, rustc rejects them (E0106, and E0515 beside a
    // reference parameter); as rejected, the backend makes them owned.
    (
        "fn sign(n: i32) -> string {\n    ext::refnum(n)\n}\n",
        "read_s(sign(pi));",
        "V0304",
        "",
    ),
    (
        "fn sign_of(s: string, n: i32) -> string {\n    ext::refnum(n)\n}\n",
        "read_s(sign_of(ps, pi));",
        "V0304",
        "",
    ),
    // The variable of a `for` over a `Vec` of numbers is a copy: a
    // borrowed return from it is part of a value that ends with the
    // function, in a free function, a method, and a nested loop. As a
    // borrowed return, rustc rejects them (E0515); as rejected, the
    // backend makes them owned.
    (
        "fn first_sign(v: Vec<i32>) -> string {\n    for n in v {\n        if n < 0 {\n            return ext::refnum(n);\n        }\n    }\n    ext::refnum(v[0])\n}\n",
        "read_s(first_sign(vec![1, -2]));",
        "V0304",
        "",
    ),
    (
        "struct Ns {\n    ns: Vec<i32>,\n}\n\nimpl Ns {\n    fn sign(self) -> string {\n        for n in self.ns {\n            return ext::refnum(n);\n        }\n        ext::refnum(self.ns[0])\n    }\n}\n",
        "let ns = Ns { ns: vec![1] };\n    read_s(ns.sign());",
        "V0304",
        "",
    ),
    (
        "fn grid_sign(g: Vec<Vec<i32>>) -> string {\n    for row in g {\n        for n in row {\n            return ext::refnum(n);\n        }\n    }\n    ext::refnum(g[0][0])\n}\n",
        "read_s(grid_sign(vec![vec![1]]));",
        "V0304",
        "",
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

/// The marker starting the body of a case of an async function
/// (milestone 5b1): the harness writes the case as an `async fn`.
const ASYNC_CASE: &str = "async: ";

/// The case function `name`: setup, the case, then the epilogue unless
/// the case ends in a returned value; an `async fn` for a body starting
/// with [`ASYNC_CASE`].
fn function(name: &str, body: &str, ret: &str) -> String {
    let epilogue = if ret.is_empty() { EPILOGUE } else { "" };
    let (asyncness, body) = match body.strip_prefix(ASYNC_CASE) {
        Some(body) => ("async ", body),
        None => ("", body),
    };
    format!("{asyncness}fn {name}({PARAMS}){ret} {{\n{SETUP}    {body}\n{epilogue}}}\n")
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
    assert!(cases.len() <= 500, "keep this test small: {}", cases.len());

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
        "a result whose type is filled from where it goes (milestone 5b3 spec 2.1) is a new value the caller owns: changed after `let mut`, moved into a `Vec`",
        &[
            (
                "main.vr",
                "mod ext;\n\nstruct User {\n    name: string,\n    age: i64,\n}\n\nfn one() -> Result<User, Error> {\n    let mut user: User = ext::one(\"{\\\"name\\\": \\\"ann\\\", \\\"age\\\": 1}\")?;\n    user.age = user.age + 1;\n    user.name = \"bo\";\n    Ok(user)\n}\n\nasync fn all(db: ext::Db) -> Result<Vec<User>, Error> {\n    let mut users: Vec<User> = db.all(\"[]\").await?;\n    let first: User = ext::one(\"{\\\"name\\\": \\\"cy\\\", \\\"age\\\": 2}\")?;\n    users.push(first);\n    users.push(one()?);\n    Ok(users)\n}\n\nasync fn main() {\n    match all(ext::Db::open()).await {\n        Ok(users) => println!(\"{}\", users.len()),\n        Err(e) => println!(\"{}\", e.message()),\n    }\n}\n",
            ),
            (
                "ext.rs",
                "pub struct Db {\n    pub name: String,\n}\n\nimpl Db {\n    pub fn open() -> Db {\n        Db { name: String::new() }\n    }\n\n    pub async fn all<T: varyk_std::serde::de::DeserializeOwned>(&self, q: &'static str) -> Result<Vec<T>, varyk_std::Error> {\n        varyk_std::json::parse(q)\n    }\n}\n\npub fn one<T: varyk_std::serde::de::DeserializeOwned>(q: &'static str) -> Result<T, varyk_std::Error> {\n    varyk_std::json::parse(q)\n}\n",
            ),
        ],
    ),
    (
        "`env::parse` into a struct of a nested module: a rename, a default, an `Option`, a unit enum, and a skipped field with a default",
        &[
            (
                "main.vr",
                "mod settings;\n\nfn load() -> Result<settings::Config, Error> {\n    let c: settings::Config = env::parse()?;\n    Ok(c)\n}\n\nfn main() {\n    match load() {\n        Ok(c) => println!(\"{}\", c.port),\n        Err(e) => println!(\"{}\", e),\n    }\n}\n",
            ),
            (
                "settings.vr",
                "pub enum Mode {\n    Dev,\n    #[rename(\"prod\")]\n    Prod,\n}\n\npub struct Config {\n    #[rename(\"http_port\")]\n    #[default(8080)]\n    pub port: u16,\n    pub mode: Mode,\n    pub token: Option<string>,\n    #[skip]\n    #[default(\"x\")]\n    pub name: string,\n}\n",
            ),
        ],
    ),
    (
        "`#[default]` helpers of struct `A_b` field `c` and struct `A` field `b_c` in one module, both read, beside a method of the type",
        &[(
            "main.vr",
            "struct A_b {\n    #[default(1)]\n    c: i32,\n}\n\nstruct A {\n    #[default(2)]\n    b_c: i32,\n}\n\nimpl A {\n    fn get(self) -> i32 {\n        self.b_c\n    }\n}\n\nfn main() {\n    let x: Result<A_b, Error> = json::parse(\"{}\");\n    let y: Result<A, Error> = json::parse(\"{}\");\n    match x {\n        Ok(v) => println!(\"{}\", v.c),\n        Err(e) => println!(\"{}\", e),\n    }\n    match y {\n        Ok(v) => println!(\"{}\", v.get()),\n        Err(e) => println!(\"{}\", e),\n    }\n}\n",
        )],
    ),
    (
        "`json` on types of a nested module: each derive combination, `rename` on a field and a variant, `skip` with and without a default, a default of each kind, and a struct holding itself through a `Vec`",
        &[
            (
                "main.vr",
                "mod shop;\n\nfn load(body: string) -> Result<shop::Order, Error> {\n    let o: shop::Order = json::parse(body)?;\n    Ok(o)\n}\n\nfn main() {\n    let r = load(\"{}\");\n    let t = shop::Tree { name: \"t\", kids: Vec::new() };\n    println!(\"{}\", json::stringify(t));\n    let n = shop::Note { text: \"x\", cache: \"c\" };\n    println!(\"{}\", json::stringify(n));\n    let both: Result<shop::Tree, Error> = json::parse(\"{}\");\n    match r {\n        Ok(o) => println!(\"{}\", json::stringify(o.lines)),\n        Err(e) => println!(\"{}\", e),\n    }\n}\n",
            ),
            (
                "shop.vr",
                "pub enum Status {\n    #[rename(\"open\")]\n    Open,\n    Closed,\n}\n\npub struct Line {\n    #[rename(\"sku\")]\n    pub code: string,\n    #[default(1)]\n    pub count: u16,\n}\n\npub struct Order {\n    #[default(-1)]\n    pub id: i64,\n    #[default(0.5)]\n    pub rate: f64,\n    #[default(\"n/a\\t\")]\n    pub note: string,\n    #[default(false)]\n    pub paid: bool,\n    pub status: Status,\n    pub lines: Vec<Line>,\n    pub by_name: HashMap<string, Vec<Line>>,\n    #[skip]\n    #[default(7)]\n    pub tries: u8,\n    #[skip]\n    pub seen: Option<Vec<string>>,\n}\n\npub struct Tree {\n    pub name: string,\n    pub kids: Vec<Tree>,\n}\n\npub struct Note {\n    pub text: string,\n    #[skip]\n    pub cache: string,\n}\n",
            ),
        ],
    ),
    (
        "a recursive enum and a struct holding itself through a `HashMap`, cloned and compared",
        &[(
            "main.vr",
            "enum Tree {\n    Leaf(i32),\n    Node(Vec<Tree>),\n}\n\nstruct Dir {\n    name: string,\n    children: HashMap<string, Dir>,\n}\n\nfn same(a: Tree, b: Tree) -> bool {\n    a == b\n}\n\nfn main() {\n    let t = Tree::Node(vec![Tree::Leaf(1), Tree::Node(Vec::new())]);\n    let u = t.clone();\n    println!(\"{} {}\", t == u, same(t, u));\n    let mut d = Dir { name: \"root\", children: HashMap::new() };\n    d.children.insert(\"a\", Dir { name: \"a\", children: HashMap::new() });\n    let e = d.clone();\n    println!(\"{} {}\", d == e, d.children != e.children);\n}\n",
        )],
    ),
    (
        "a Rust struct and enum deriving `Clone` and `PartialEq`, cloned and compared, and held by a Varyk struct",
        &[
            (
                "main.vr",
                "mod ext;\n\nstruct Held {\n    point: ext::Point,\n    kind: ext::Kind,\n}\n\nfn main() {\n    let p = ext::origin();\n    let q = p.clone();\n    println!(\"{}\", p == q);\n    let h = Held { point: q, kind: ext::Kind::Word };\n    let g = h.clone();\n    println!(\"{} {}\", h == g, g.kind != ext::Kind::Number);\n}\n",
            ),
            (
                "ext.rs",
                "#[derive(Debug, Clone, PartialEq)]\npub struct Point {\n    pub x: i32,\n}\n\n#[derive(Clone, Copy, PartialEq, Eq)]\npub enum Kind {\n    Word,\n    Number,\n}\n\npub fn origin() -> Point {\n    Point { x: 0 }\n}\n",
            ),
        ],
    ),
    (
        "a Varyk struct holding a Rust struct without derives, emitted with no `#[derive]`",
        &[
            (
                "main.vr",
                "mod ext;\n\nstruct Holder {\n    name: string,\n    handle: ext::Handle,\n}\n\nenum Slot {\n    Empty,\n    Full(Holder),\n}\n\nfn main() {\n    let h = Holder { name: \"h\", handle: ext::open() };\n    let s = Slot::Full(h);\n    match s {\n        Slot::Full(h) => println!(\"{}\", h.name),\n        Slot::Empty => {}\n    }\n}\n",
            ),
            (
                "ext.rs",
                "pub struct Handle {\n    pub id: i32,\n}\n\npub fn open() -> Handle {\n    Handle { id: 1 }\n}\n",
            ),
        ],
    ),
    (
        "casts before `<`, `..=`, and `?` on `Option` and on `Ok(x)` with its types written out",
        &[(
            "main.vr",
            include_str!("fixtures/codegen/expressions/main.vr"),
        )],
    ),
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
        "a `&self` method of an imported struct returning `&str`, called from Varyk and used as a `trim` receiver",
        &[
            (
                "main.vr",
                "mod ext;\n\nfn main() {\n    let n = ext::Note::new(\"  hello  \");\n    let t = n.text().trim();\n    println!(\"[{}] [{}]\", t, n.text());\n    println!(\"[{}]\", ext::first_word(t));\n}\n",
            ),
            (
                "ext.rs",
                "pub struct Note {\n    body: String,\n}\n\nimpl Note {\n    pub fn new(body: &str) -> Note {\n        Note { body: body.to_string() }\n    }\n\n    pub fn text(&self) -> &str {\n        &self.body\n    }\n}\n\npub fn first_word(s: &str) -> &str {\n    s.split(' ').next().unwrap_or(\"\")\n}\n",
            ),
        ],
        "[hello] [  hello  ]\n[hello]\n",
    ),
    (
        "std macros beside a `.rs` module exporting macros of the same names",
        &[
            (
                "main.vr",
                "mod ext;\n\nfn main() {\n    let v = vec![1, 2];\n    let s = format!(\"{} items\", v.len());\n    println!(\"{}\", s);\n}\n\n#[test]\nfn checks() {\n    assert(1 + 1 == 2);\n    assert_eq(\"a\", \"a\");\n}\n",
            ),
            (
                "ext.rs",
                "#[macro_export]\nmacro_rules! println {\n    ($($t:tt)*) => {\n        ::std::println!(\"HIJACKED\")\n    };\n}\n\n#[macro_export]\nmacro_rules! vec {\n    ($($t:tt)*) => {\n        ::std::vec::Vec::<i32>::new()\n    };\n}\n\n#[macro_export]\nmacro_rules! format {\n    ($($t:tt)*) => {\n        ::std::string::String::from(\"HIJACKED\")\n    };\n}\n\n#[macro_export]\nmacro_rules! assert {\n    ($($t:tt)*) => {\n        ::std::panic!(\"HIJACKED\")\n    };\n}\n",
            ),
        ],
        "2 items\n",
    ),
    (
        "a task started and then cancelled by a `?` that returns before it is awaited (milestone 5b1 spec 2.4)",
        &[(
            "main.vr",
            "async fn shout() {\n    time::sleep(50).await;\n    println!(\"the task ran\");\n}\n\nasync fn fail() -> Result<i32, Error> {\n    Err(Error::new(\"no\"))\n}\n\nasync fn work() -> Result<i32, Error> {\n    let t = shout();\n    let n = fail().await?;\n    t.await;\n    Ok(n)\n}\n\nasync fn main() {\n    match work().await {\n        Ok(n) => println!(\"{}\", n),\n        Err(e) => println!(\"failed: {}\", e.message()),\n    }\n    time::sleep(300).await;\n    println!(\"done\");\n}\n",
        )],
        "failed: no\ndone\n",
    ),
    (
        "a package's items used across the package boundary (M5b2 spec 5): a borrowed return, \
         a `mut` parameter, a struct literal, a `match` on an enum with named fields, methods, \
         and an async function awaited and started",
        &[
            (
                "app/Cargo.toml",
                concat!(
                    "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n",
                    "[[bin]]\nname = \"app\"\npath = \"src/main.vr\"\n\n",
                    "[dependencies]\nkit = { path = \"../kit\" }\nvaryk-std = \"",
                    env!("CARGO_PKG_VERSION"),
                    "\"\n",
                ),
            ),
            (
                "app/src/main.vr",
                "async fn main() {\n    let names = vec![\"ada\", \"grace\"];\n    \
                 let first = kit::first(names);\n    println!(\"{}\", first);\n    \
                 let mut p = kit::Point { x: 1, y: 2 };\n    kit::bump(p);\n    p.shift(5);\n    \
                 println!(\"{} {} {}\", p.x, p.y, p.sum());\n    \
                 for s in vec![kit::Shape::Rect { w: 2, h: 3 }, kit::Shape::Dot] {\n        \
                 match s {\n            kit::Shape::Rect { w, h } => println!(\"rect {}\", w * h),\n            \
                 kit::Shape::Dot => println!(\"dot\"),\n        }\n    }\n    \
                 let started = kit::double(4);\n    let awaited = kit::double(5).await;\n    \
                 println!(\"{} {}\", started.await, awaited);\n}\n",
            ),
            (
                "kit/Cargo.toml",
                "[package]\nname = \"kit\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n\
                 [lib]\npath = \"src/lib.vr\"\n",
            ),
            (
                "kit/src/lib.vr",
                "pub struct Point {\n    pub x: i32,\n    pub y: i32,\n}\n\n\
                 impl Point {\n    pub fn sum(self) -> i32 {\n        self.x + self.y\n    }\n\n    \
                 pub fn shift(mut self, by: i32) {\n        self.x = self.x + by;\n    }\n}\n\n\
                 pub enum Shape {\n    Dot,\n    Rect { w: i32, h: i32 },\n}\n\n\
                 pub fn first(names: Vec<string>) -> string {\n    names[0]\n}\n\n\
                 pub fn bump(mut p: Point) {\n    p.y = p.y + 10;\n}\n\n\
                 pub async fn double(n: i32) -> i32 {\n    n * 2\n}\n",
            ),
        ],
        "ada\n6 12 18\nrect 6\ndot\n8 10\n",
    ),
];

#[test]
fn programs_run_as_written() {
    for (index, (label, files, expected)) in MUST_RUN_TREES.iter().enumerate() {
        let entry = write_program(&format!("run{index:02}"), files);
        // A tree of packages runs its `app` package, which uses the others.
        let app = entry.with_file_name("app");
        let output = if app.join("Cargo.toml").is_file() {
            varyk_in(&app, &["run"])
        } else {
            varyk(&["run", entry.to_str().expect("utf-8 path")])
        };
        assert_no_crash(&output, &format!("`varyk run` on {label}"));
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            output.status.success() && stdout == *expected,
            "expected {expected:?}, got {stdout:?}: {label}\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

/// The label of the [`MUST_RUN_TREES`] case whose `.rs` module exports
/// macros named like the std ones, `assert` among them.
const HIJACK: &str = "std macros beside a `.rs` module exporting macros of the same names";

/// The tests of the [`HIJACK`] program pass under `varyk test`: `assert`
/// and `assert_eq` are written `::std::assert!`, so the module's
/// `assert!`, which always fails, is never the one called.
#[test]
fn std_asserts_beside_a_hijacking_macro_pass() {
    let (index, (_, files, _)) = MUST_RUN_TREES
        .iter()
        .enumerate()
        .find(|(_, (label, _, _))| *label == HIJACK)
        .expect("the hijacking case");
    let entry = write_program(&format!("run{index:02}"), files);
    let output = varyk(&["test", entry.to_str().expect("utf-8 path")]);
    assert!(
        output.status.success(),
        "{:?}\n{}\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Bodies of `#[test]` functions using `assert` and `assert_eq` (M5a spec
/// 2.7) on each kind of value, after [`SETUP`] and a `let c = true;`:
/// each must pass `check`, build under `cargo test`, and pass, so an
/// operand evaluated twice (`lv.pop()`) or compared in a form Rust
/// rejects fails here.
const MUST_TEST: &[&str] = &[
    "assert(li == 7);\n    assert(ls == \"made\" && lp.n > 0);\n    assert(!ln.is_empty());\n    assert(c);\n    read_s(ls);\n    read_p(lp);",
    "let n = lp.s;\n    assert_eq(ls, \"made\");\n    assert_eq(\"made\", ls);\n    assert_eq(lp.s, \"mp\");\n    assert_eq(n, lp.s);\n    assert_eq(mk_s(), ls);\n    assert_eq(lw[0], ls);\n    assert_eq(ll, \"lit\");\n    assert_eq({ let t = mk_p(); t.s }, n);\n    assert_eq(ls.trim(), \"made\");\n    assert_eq(s_of(lp), \"mp\");\n    assert_eq(format!(\"{}!\", ls), \"made!\");\n    read_s(ls);\n    read_p(lp);",
    "assert_eq(if c { ls } else { \"x\" }, \"made\");\n    assert_eq(if c { mk_s() } else { mk_p().s }, \"made\");\n    read_s(ls);",
    "assert_eq(lp, mk_p());\n    assert_eq(mk_p(), lp);\n    assert_eq(le, E::C);\n    assert_eq(lo, Some(mk_p()));\n    assert_eq(lv, vec![mk_p()]);\n    assert_eq(lv[0], lp);\n    assert_eq(lq, mk_q());\n    assert_eq(lk, Some(7));\n    assert_eq(lr, mk_r());\n    assert_eq(if c { lp } else { lq.p }, mk_p());\n    read_p(lp);\n    read_v(lv);\n    read_e(le);",
    "assert_eq(li, 7);\n    assert_eq(mk_i() + 1, 8);\n    assert_eq(ln.len(), 2);\n    assert_eq(area(2.0), 4.0);\n    assert_eq(li > 0, true);\n    let b: u8 = 3;\n    assert_eq(b, 3);\n    assert_eq(Error::new(\"x\"), Error::new(\"x\"));",
    "for x in ln {\n        assert(x > 0);\n    }\n    match lo {\n        Some(p) => assert_eq(p.n, 1),\n        None => assert(false),\n    }",
    "lv.push(mk_p());\n    assert_eq(lv.len(), 2);\n    assert_eq(lv.pop(), Some(mk_p()));\n    assert_eq(lv.len(), 1);",
];

/// Bodies of `#[test] async fn`s (milestone 5b1 spec 2.2), written and run
/// as [`MUST_TEST`]'s are.
const MUST_TEST_ASYNC: &[&str] = &[
    "let t = a_i(3);\n    let u = lp.a_n();\n    assert_eq(t.await + u.await, 4);\n    a_i(1).detach();",
    "time::sleep(1).await;\n    assert_eq(a_len(ls).await, 4);\n    assert_eq(lp.a_n().await, 1);\n    assert_eq(a_p(lp).await, lp);\n    assert_eq(a_try(2).await, Ok(4));\n    assert(a_r(0).await.is_err());\n    read_s(ls);",
    "assert_eq(Task::all(ln.iter().map(|x| a_i(x)).collect()).await, vec![1, 2]);\n    assert_eq(Task::all(vec![a_r(1), a_r(2)]).await, Ok(vec![1, 2]));\n    assert_eq(Task::all(vec![a_r(1), a_r(0)]).await, Err(\"negative\"));\n    let rs = Task::all_settled(vec![a_r(2), a_r(0)]).await;\n    assert_eq(rs, vec![Ok(2), Err(\"negative\")]);",
];

#[test]
fn tests_with_asserts_build_and_pass() {
    let mut source = format!("{PRELUDE}\nfn main() {{}}\n");
    for (index, body) in MUST_TEST.iter().enumerate() {
        source.push_str(&format!(
            "\n#[test]\nfn t{index:02}() {{\n{SETUP}    let c = true;\n    {body}\n}}\n"
        ));
    }
    for (index, body) in MUST_TEST_ASYNC.iter().enumerate() {
        source.push_str(&format!(
            "\n#[test]\nasync fn a{index:02}() {{\n{SETUP}    let c = true;\n    {body}\n}}\n"
        ));
    }
    let entry = write_program("tests", &[("main.vr", &source), ("ext.rs", EXT_RS)]);
    let (checked, stderr) = run("check", &entry);
    assert!(checked, "tests with asserts fail check:\n{stderr}");
    let output = varyk(&["test", entry.to_str().expect("utf-8 path")]);
    assert_no_crash(&output, "`varyk test` on the assert cases");
    assert!(
        output.status.success(),
        "tests with asserts fail to build or pass:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
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
                "[package]\nname = \"shop\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[lib]\npath = \"src/lib.vr\"\n",
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
        "a payload taken out, at depth, of a temporary holding an imported enum with a destructor",
        "V0304",
        &[
            (
                "main.vr",
                "mod res;\n\nfn main() {\n    match res::open() {\n        Some(res::Ev::Msg(s)) => res::keep(s),\n        _ => {}\n    }\n}\n",
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

pub fn open() -> Option<Ev> {
    Some(make())
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

/// The manifest of a crate the harness writes itself, one bin per case
/// appended: it depends on the workspace's `crates/varyk-std` (M5a spec
/// 5.3), which any case may use.
fn rejects_manifest() -> String {
    format!(
        "[package]\nname = \"rejects\"\nversion = \"0.0.0\"\nedition = \"2024\"\nautobins = false\n\n[dependencies]\nvaryk-std = {{ path = {:?} }}\n\n[workspace]\n",
        common::std_path().display().to_string()
    )
}

#[test]
fn trees_that_break_a_rule_fail_rustc() {
    let dir = scratch_root().join("rustc_rejects_trees");
    let mut manifest = rejects_manifest();
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
        )
        .chain(
            MUST_REJECT_OPTION
                .iter()
                .map(|(body, code, _)| (body, code, OPTION_RET)),
        );
    let cases = cases
        .map(|(body, code, ret)| ("", *body, *code, ret))
        .chain(
            MUST_REJECT_ITEMS
                .iter()
                .map(|(items, body, code, _)| (*items, *body, *code, "")),
        );
    for (index, (items, body, code, ret)) in cases.enumerate() {
        let entry = write_cases(
            &format!("reject{index:02}"),
            &[items.to_string(), function("t", body, ret)],
        );
        let (checked, stderr) = run("check", &entry);
        let codes: Vec<&str> = stderr
            .lines()
            .filter_map(|line| line.strip_prefix("error[")?.split(']').next())
            .collect();
        assert!(
            !checked && !codes.is_empty() && codes.iter().all(|c| *c == code),
            "expected `check` to reject with {code} only:\n{items}{body}\n{stderr}"
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
    let mut manifest = rejects_manifest();
    let mut bins = Vec::new();
    let mut accepted = Vec::new();
    let mut unemitted = Vec::new();
    let cases = MUST_REJECT
        .iter()
        .map(|(body, _, rustc_code)| ("", *body, "", *rustc_code))
        .chain(
            MUST_REJECT_QUESTION
                .iter()
                .map(|(body, _, rustc_code)| ("", *body, RESULT_RET, *rustc_code)),
        )
        .chain(
            MUST_REJECT_OPTION
                .iter()
                .map(|(body, _, rustc_code)| ("", *body, OPTION_RET, *rustc_code)),
        )
        .chain(
            MUST_REJECT_ITEMS
                .iter()
                .map(|(items, body, _, rustc_code)| (*items, *body, "", *rustc_code)),
        );
    // Writes the case as bin `name`; false when it does not type-check (a
    // `?` outside a `Result` function has no Rust).
    let mut emit = |name: &str, items: &str, body: &str, ret: &str| {
        let entry = write_cases(
            &format!("emit_{name}"),
            &[items.to_string(), function("t", body, ret)],
        );
        let text = fs::read_to_string(&entry).expect("read main.vr");
        let mut sources = Vec::new();
        let Ok(program) = resolve(SourceFile::new(FileId(0), &entry, text), &mut sources)
            .and_then(|resolved| typecheck(resolved, &sources))
        else {
            return false;
        };
        let (program, _) = analyze_unchecked(program, &sources);
        // Only its `src/main.rs` is used: the manifest is `rejects_manifest`.
        let std = StdDependency::for_program(program.uses_std);
        let generated = RustBackend.generate(
            &program,
            &CrateInfo::single_file("rejects".to_string(), std),
        );
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
        emit("control", "", "", ""),
        "the control case fails to type-check"
    );
    for (index, (items, body, ret, rustc_code)) in cases.enumerate() {
        let name = format!("reject{index:02}");
        if !emit(&name, items, body, ret) {
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
