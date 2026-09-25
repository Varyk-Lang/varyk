use std::path::Path;

use varyk_syntax::{FileId, SourceFile, Span};

use super::analyze;
use crate::diagnostics::{Diagnostic, codes};
use crate::hir::{HirFunction, HirProgram, LocalId, Origin, PlaceInfo, StringRepr};
use crate::resolve::resolve;
use crate::types::typecheck;

// --- Helpers ----------------------------------------------------------------

fn check_source(entry: SourceFile) -> (Result<HirProgram, Vec<Diagnostic>>, Vec<SourceFile>) {
    let mut sources = Vec::new();
    let result = resolve(entry, &mut sources)
        .and_then(|resolved| typecheck(resolved, &sources))
        .and_then(analyze);
    (result, sources)
}

/// Resolves, type-checks, and analyzes a single-file program.
fn check_str(text: &str) -> (Result<HirProgram, Vec<Diagnostic>>, Vec<SourceFile>) {
    check_source(SourceFile::new(FileId(0), "dummy/test.vr", text))
}

/// Resolves, type-checks, and analyzes `<rel>`, relative to the workspace
/// root.
fn check_path(rel: &str) -> (Result<HirProgram, Vec<Diagnostic>>, Vec<SourceFile>) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel);
    let text = std::fs::read_to_string(&path).expect("file should exist");
    check_source(SourceFile::new(FileId(0), path, text))
}

fn ok(text: &str) -> HirProgram {
    match check_str(text).0 {
        Ok(program) => program,
        Err(diagnostics) => panic!("expected success for:\n{text}\ngot {diagnostics:#?}"),
    }
}

fn errors(text: &str) -> (Vec<Diagnostic>, Vec<SourceFile>) {
    match check_str(text) {
        (Ok(_), _) => panic!("expected diagnostics for:\n{text}"),
        (Err(diagnostics), sources) => (diagnostics, sources),
    }
}

/// The only diagnostic for `text`, with the program's sources.
fn one_error(text: &str) -> (Diagnostic, Vec<SourceFile>) {
    let (diagnostics, sources) = errors(text);
    assert_eq!(
        diagnostics.len(),
        1,
        "expected one diagnostic, got {diagnostics:#?}"
    );
    (diagnostics[0].clone(), sources)
}

/// The span of the first occurrence of `needle` in file 0.
fn span_of(sources: &[SourceFile], needle: &str) -> Span {
    let text = &sources[0].text;
    let start = text
        .find(needle)
        .unwrap_or_else(|| panic!("{needle:?} not in file"));
    Span::new(FileId(0), start as u32, (start + needle.len()) as u32)
}

/// The sub-span of `needle`'s first occurrence covering its `part`.
fn part_of(sources: &[SourceFile], needle: &str, part: &str) -> Span {
    let outer = span_of(sources, needle);
    let offset = needle
        .find(part)
        .unwrap_or_else(|| panic!("{part:?} not in {needle:?}")) as u32;
    Span::new(
        outer.file,
        outer.start + offset,
        outer.start + offset + part.len() as u32,
    )
}

/// Asserts `d` carries a fix-it inserting `mut ` right before `part` of
/// `needle`.
fn assert_mut_fix_it(d: &Diagnostic, sources: &[SourceFile], needle: &str, part: &str) {
    let at = part_of(sources, needle, part).start;
    let fix = d.fix_it.as_ref().expect("fix-it");
    assert_eq!(fix.span, Span::new(FileId(0), at, at), "{d:#?}");
    assert_eq!(fix.replacement, "mut ");
}

fn has_note(d: &Diagnostic, fragment: &str) -> bool {
    d.notes.iter().any(|n| n.contains(fragment))
}

fn function<'a>(program: &'a HirProgram, name: &str) -> &'a HirFunction {
    program
        .functions()
        .find(|f| f.name == name)
        .unwrap_or_else(|| panic!("no function {name}"))
}

/// The place info of the last local called `name` in `function`.
fn place(function: &HirFunction, name: &str) -> PlaceInfo {
    function
        .locals
        .iter()
        .rev()
        .find(|local| local.name == name)
        .unwrap_or_else(|| panic!("no local {name}"))
        .place
}

/// The string representation of the last local called `name` in `function`.
fn repr(function: &HirFunction, name: &str) -> Option<StringRepr> {
    function
        .locals
        .iter()
        .rev()
        .find(|local| local.name == name)
        .unwrap_or_else(|| panic!("no local {name}"))
        .repr
}

const P: &str = "struct P {\n    x: i32,\n    name: string,\n}\n";
const MAIN: &str = "fn main() {}\n";

fn with_p(body: &str) -> String {
    format!("{P}{body}{MAIN}")
}

// --- V0300: mutation through a non-`mut` parameter --------------------------

#[test]
fn assigning_a_field_of_a_non_mut_parameter_is_v0300() {
    let (d, sources) = one_error(&with_p("fn f(p: P) {\n    p.x = 1;\n}\n"));
    assert_eq!(d.code, codes::V0300);
    assert_eq!(d.span, span_of(&sources, "p.x"));
    assert!(d.message.contains("`p`"), "{d:#?}");
    assert_mut_fix_it(&d, &sources, "fn f(p: P)", "p: P");
    assert!(d.notes.is_empty(), "{d:#?}");
}

#[test]
fn assigning_a_whole_non_mut_parameter_is_v0300() {
    let (d, sources) = one_error("fn f(x: i32) {\n    x = 2;\n}\nfn main() {}\n");
    assert_eq!(d.code, codes::V0300);
    assert_eq!(d.span, part_of(&sources, "    x = 2", "x"));
    assert_mut_fix_it(&d, &sources, "fn f(x: i32)", "x: i32");
}

#[test]
fn assigning_through_a_let_mut_bound_from_a_non_mut_parameter_is_v0300() {
    let (d, sources) = one_error(&with_p(
        "fn f(p: P) {\n    let mut q = p;\n    q.name = \"x\";\n}\n",
    ));
    assert_eq!(d.code, codes::V0300);
    assert_eq!(d.span, span_of(&sources, "q.name"));
    assert_mut_fix_it(&d, &sources, "fn f(p: P)", "p: P");
    assert!(has_note(&d, "`q`"), "{d:#?}");
    assert!(has_note(&d, "`p`"), "{d:#?}");
}

#[test]
fn assigning_through_a_let_bound_from_a_field_of_a_non_mut_parameter_is_v0300() {
    let (d, sources) = one_error(&with_p(
        "fn f(p: P) {\n    let mut n = p.name;\n    n = \"x\";\n}\n",
    ));
    assert_eq!(d.code, codes::V0300);
    assert_mut_fix_it(&d, &sources, "fn f(p: P)", "p: P");
    assert!(
        has_note(
            &d,
            "`n` is another name for a field of `p`; to keep your own copy, write `let mut n = p.name.clone();`"
        ),
        "{d:#?}"
    );
}

#[test]
fn assigning_through_a_let_mut_bound_from_a_whole_non_mut_string_parameter_is_v0300() {
    let (d, sources) = one_error(
        "fn f(a: string, b: string) {\n    let mut r = a;\n    r = b;\n}\nfn main() {}\n",
    );
    assert_eq!(d.code, codes::V0300);
    assert_mut_fix_it(&d, &sources, "fn f(a: string, b: string)", "a: string");
    assert!(
        has_note(
            &d,
            "`r` is another name for `a`; to keep your own copy, write `let mut r = a.clone();`"
        ),
        "{d:#?}"
    );
}

#[test]
fn assigning_through_a_let_mut_bound_from_an_element_of_a_non_mut_parameter_is_v0300() {
    let (d, sources) = one_error(
        "fn f(words: Vec<string>) {\n    let mut best = words[0];\n    best = words[1];\n}\nfn main() {}\n",
    );
    assert_eq!(d.code, codes::V0300);
    assert_mut_fix_it(
        &d,
        &sources,
        "fn f(words: Vec<string>)",
        "words: Vec<string>",
    );
    assert!(
        has_note(
            &d,
            "`best` is another name for an element of `words`; to keep your own copy, write `let mut best = words[0].clone();`"
        ),
        "{d:#?}"
    );
}

#[test]
fn assigning_through_mut_parameters_and_let_mut_is_accepted() {
    ok(&with_p(
        "fn f(mut p: P, mut x: i32) {\n    p.x = 1;\n    p.name = \"n\";\n    x = 3;\n    let mut q = p;\n    q.x = 2;\n    let mut y = x;\n    y = 4;\n}\n",
    ));
    ok(&with_p(
        "fn g(p: P) {\n    let mut x = p.x;\n    x = 5;\n    let mut local = P { x: 1, name: \"a\" };\n    local.name = \"b\";\n    local = P { x: 2, name: \"c\" };\n}\n",
    ));
}

#[test]
fn assigning_a_field_of_self_in_a_non_mut_self_method_is_v0300() {
    let (d, sources) = one_error(&with_p(
        "impl P {\n    fn reset(self) {\n        self.x = 0;\n    }\n}\n",
    ));
    assert_eq!(d.code, codes::V0300);
    assert_eq!(d.span, span_of(&sources, "self.x"));
    assert!(d.message.contains("`self`"), "{d:#?}");
    assert_mut_fix_it(&d, &sources, "fn reset(self)", "self)");
}

#[test]
fn assigning_a_field_of_self_in_a_mut_self_method_is_accepted() {
    let program = ok(&with_p(
        "impl P {\n    fn bump(mut self, by: i32) {\n        self.x = self.x + by;\n        self.name = \"n\";\n    }\n}\n",
    ));
    let bump = function(&program, "bump");
    assert_eq!(
        place(bump, "self"),
        PlaceInfo {
            borrowed: true,
            mutable: true,
            origin: Some(Origin::Param(LocalId(0))),
        }
    );
}

// --- V0301: assignment to an immutable `let` --------------------------------

#[test]
fn assigning_to_an_immutable_let_is_v0301() {
    let (d, sources) = one_error("fn main() {\n    let x = 1;\n    x = 2;\n}\n");
    assert_eq!(d.code, codes::V0301);
    assert_eq!(d.span, part_of(&sources, "    x = 2", "x"));
    assert!(d.message.contains("`x`"), "{d:#?}");
    assert_mut_fix_it(&d, &sources, "let x = 1", "x = 1");
}

#[test]
fn assigning_a_field_of_an_immutable_let_is_v0301() {
    let (d, sources) = one_error(&format!(
        "{P}fn main() {{\n    let p = P {{ x: 1, name: \"a\" }};\n    p.x = 2;\n}}\n"
    ));
    assert_eq!(d.code, codes::V0301);
    assert_eq!(d.span, span_of(&sources, "p.x"));
    assert_mut_fix_it(&d, &sources, "let p = P", "p = P");
}

// --- V0302 and V0303: arguments to `mut` parameters -------------------------

const TAKERS: &str = "fn g(mut p: P) {}\nfn h(mut s: string) {}\nfn inc(mut x: i32) {}\nfn make() -> P {\n    P { x: 0, name: \"m\" }\n}\n";

fn with_takers(body: &str) -> String {
    format!("{P}{TAKERS}{body}{MAIN}")
}

#[test]
fn passing_an_immutable_let_to_a_mut_parameter_is_v0302() {
    let (d, sources) = one_error(&with_takers(
        "fn f() {\n    let q = P { x: 1, name: \"a\" };\n    g(q);\n}\n",
    ));
    assert_eq!(d.code, codes::V0302);
    assert_eq!(d.span, part_of(&sources, "g(q)", "q"));
    assert!(d.message.contains("`q`"), "{d:#?}");
    assert_mut_fix_it(&d, &sources, "let q = P", "q = P");
}

#[test]
fn passing_an_immutable_let_to_a_mut_parameter_inside_a_range_for_uses_its_own_name() {
    // A range-`for`'s own variable never gets the V0302 wording (it is a
    // `for` variable, V0303's business); the `let` blamed here is a
    // different local, `j`, and the message must name it, not the loop's
    // own variable.
    let (d, _) = one_error(
        "fn change(mut n: i32) {\n}\nfn main() {\n    let j = 1;\n    for i in 0..3 {\n        change(j);\n    }\n}\n",
    );
    assert_eq!(d.code, codes::V0302, "{d:#?}");
    assert!(d.message.contains("`j`"), "{d:#?}");
    assert!(!d.message.contains("`i`"), "{d:#?}");
}

#[test]
fn passing_an_immutable_let_to_an_imported_mut_reference_is_v0302() {
    let (result, sources) =
        check_path("crates/varyk/tests/fixtures/interop/mut_i32_immutable/main.vr");
    let diagnostics = result.expect_err("should fail");
    assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    let d = &diagnostics[0];
    assert_eq!(d.code, codes::V0302);
    assert_eq!(d.span, part_of(&sources, "bump(n)", "n"));
    assert_mut_fix_it(d, &sources, "let n = 1", "n = 1");
}

#[test]
fn passing_a_non_mut_parameter_to_a_mut_parameter_is_v0303() {
    let (d, sources) = one_error(&with_takers("fn f(p: P) {\n    g(p);\n}\n"));
    assert_eq!(d.code, codes::V0303);
    assert_eq!(d.span, part_of(&sources, "g(p)", "p"));
    assert!(d.message.contains("`p`"), "{d:#?}");
    assert_mut_fix_it(&d, &sources, "fn f(p: P)", "p: P");
    assert!(has_note(&d, "every caller of `f`"), "{d:#?}");
    assert!(has_note(&d, "allowed to change"), "{d:#?}");
}

#[test]
fn passing_a_let_bound_from_a_non_mut_parameter_is_v0303() {
    for binding in ["let q = p;", "let mut q = p;"] {
        let (d, sources) = one_error(&with_takers(&format!(
            "fn f(p: P) {{\n    {binding}\n    g(q);\n}}\n"
        )));
        assert_eq!(d.code, codes::V0303, "{binding}");
        assert_eq!(d.span, part_of(&sources, "g(q)", "q"));
        assert_mut_fix_it(&d, &sources, "fn f(p: P)", "p: P");
        assert!(has_note(&d, "every caller of `f`"), "{d:#?}");
    }
}

#[test]
fn passing_a_field_of_a_non_mut_parameter_is_v0303() {
    let (d, sources) = one_error(&with_takers("fn f(p: P) {\n    h(p.name);\n}\n"));
    assert_eq!(d.code, codes::V0303);
    assert_eq!(d.span, span_of(&sources, "p.name"));
    assert_mut_fix_it(&d, &sources, "fn f(p: P)", "p: P");
}

#[test]
fn mutable_places_and_temporaries_are_accepted_by_mut_parameters() {
    ok(&with_takers(
        "fn f(mut p: P, mut n: i32) {\n    g(p);\n    h(p.name);\n    inc(p.x);\n    inc(n);\n    let mut q = p;\n    g(q);\n    let mut local = P { x: 1, name: \"a\" };\n    g(local);\n    h(local.name);\n    g(make());\n    g(P { x: 2, name: \"b\" });\n    h(\"lit\");\n    inc(1);\n}\n",
    ));
}

// --- V0304: borrowed places into owned slots --------------------------------

fn assert_v0304(d: &Diagnostic, span: Span, names: &str) {
    assert_eq!(d.code, codes::V0304, "{d:#?}");
    assert_eq!(d.span, span, "{d:#?}");
    assert!(d.message.contains(names), "{d:#?}");
    assert!(!has_note(d, "milestone"), "{d:#?}");
    assert!(has_note(d, "in Rust terms"), "{d:#?}");
}

#[test]
fn returning_a_parameter_is_v0304_naming_it() {
    let (d, sources) = one_error(&with_p("fn f(user: P) -> P {\n    user\n}\n"));
    assert_v0304(&d, part_of(&sources, "    user\n}", "user"), "`user`");

    let (d, sources) = one_error(&with_p("fn f(user: P) -> P {\n    return user;\n}\n"));
    assert_v0304(&d, part_of(&sources, "return user", "user"), "`user`");

    let (d, sources) = one_error("fn f(s: string) -> string {\n    s\n}\nfn main() {}\n");
    assert_v0304(&d, part_of(&sources, "    s\n}", "s"), "`s`");
}

#[test]
fn returning_a_field_is_v0304_naming_the_struct() {
    let (d, sources) = one_error(&with_p("fn f(user: P) -> string {\n    user.name\n}\n"));
    assert_v0304(&d, span_of(&sources, "user.name"), "`P`");

    let (d, sources) = one_error(&format!(
        "{P}fn f() -> string {{\n    let local = P {{ x: 1, name: \"a\" }};\n    local.name\n}}\n{MAIN}"
    ));
    assert_v0304(&d, span_of(&sources, "local.name"), "`P`");
}

#[test]
fn a_binding_of_a_borrowed_place_keeps_its_origin() {
    let (d, sources) = one_error(&with_p(
        "fn f(user: P) -> string {\n    let n = user.name;\n    n\n}\n",
    ));
    assert_v0304(&d, part_of(&sources, "    n\n}", "n"), "`P`");

    let (d, sources) = one_error(&with_p(
        "fn f(user: P) -> P {\n    let u = user;\n    u\n}\n",
    ));
    assert_v0304(&d, part_of(&sources, "    u\n}", "u"), "`user`");
}

#[test]
fn a_binding_of_a_field_of_an_owned_local_has_that_local_as_its_origin() {
    let text = format!(
        "{P}fn f() -> string {{\n    let local = P {{ x: 1, name: \"a\" }};\n    let n = local.name;\n    n\n}}\n{MAIN}"
    );
    let (d, sources) = one_error(&text);
    assert_v0304(&d, part_of(&sources, "    n\n}", "n"), "`local`");

    let program = ok(&with_p(
        "fn f() {\n    let local = P { x: 1, name: \"a\" };\n    let n = local.name;\n}\n",
    ));
    let f = function(&program, "f");
    let local = f.locals.iter().position(|l| l.name == "local").unwrap();
    assert_eq!(
        place(f, "n").origin,
        Some(Origin::Local(LocalId(local as u32)))
    );
}

#[test]
fn storing_a_field_into_another_structs_field_is_v0304() {
    let text = format!(
        "{P}struct W {{\n    name: string,\n}}\nfn f(user: P) -> W {{\n    W {{ name: user.name }}\n}}\n{MAIN}"
    );
    let (d, sources) = one_error(&text);
    assert_v0304(&d, span_of(&sources, "user.name"), "`P`");
}

#[test]
fn storing_a_parameter_into_a_struct_field_is_v0304() {
    let text = format!(
        "{P}struct W {{\n    inner: P,\n}}\nfn f(user: P) -> W {{\n    W {{ inner: user }}\n}}\n{MAIN}"
    );
    let (d, sources) = one_error(&text);
    assert_v0304(&d, part_of(&sources, "inner: user", "user"), "`user`");
}

#[test]
fn borrowed_places_into_imported_owned_parameters_are_v0304() {
    let (result, sources) =
        check_path("crates/varyk/tests/fixtures/interop/borrowed_into_owned/main.vr");
    let diagnostics = result.expect_err("should fail");
    assert_eq!(diagnostics.len(), 3, "{diagnostics:#?}");
    assert_v0304(
        &diagnostics[0],
        part_of(&sources, "    ext::take(name)\n}\n\nfn from_mut", "name"),
        "`name`",
    );
    let second = span_of(&sources, "fn from_mut_param");
    let offset = sources[0].text[second.start as usize..]
        .find("take(name)")
        .unwrap() as u32
        + second.start
        + "take(".len() as u32;
    assert_v0304(
        &diagnostics[1],
        Span::new(FileId(0), offset, offset + "name".len() as u32),
        "`name`",
    );
    let d = &diagnostics[2];
    assert_eq!(d.code, codes::V0303);
    assert_eq!(d.span, part_of(&sources, "ext::push(label)", "label"));
    assert_mut_fix_it(d, &sources, "fn push_shared(label", "label");
}

#[test]
fn owned_values_and_copy_parameters_into_owned_slots_are_accepted() {
    ok("fn f(x: i32) -> i32 {\n    x\n}\nfn g(x: i32) -> i32 {\n    return x;\n}\nfn main() {}\n");
    ok(&format!(
        "{P}struct W {{\n    inner: P,\n    name: string,\n    x: i32,\n}}\nfn make() -> P {{\n    P {{ x: 0, name: \"m\" }}\n}}\nfn a(user: P) -> W {{\n    let local = P {{ x: user.x, name: \"l\" }};\n    W {{ inner: local, name: \"lit\", x: user.x }}\n}}\nfn b() -> P {{\n    make()\n}}\nfn c() -> string {{\n    \"lit\"\n}}\nfn d() -> P {{\n    let p = P {{ x: 1, name: \"n\" }};\n    p\n}}\n{MAIN}"
    ));
}

// --- Examples and place info -----------------------------------------------

#[test]
fn place_info_of_the_borrowing_example() {
    let program = check_path("examples/borrowing.vr").0.unwrap();
    let print_user = function(&program, "print_user");
    let rename = function(&program, "rename");
    let param = |f: &HirFunction| Some(Origin::Param(f.params[0].local));

    assert_eq!(
        place(print_user, "user"),
        PlaceInfo {
            borrowed: true,
            mutable: false,
            origin: param(print_user),
        }
    );
    assert_eq!(
        place(rename, "user"),
        PlaceInfo {
            borrowed: true,
            mutable: true,
            origin: param(rename),
        }
    );
    let main_user = place(function(&program, "main"), "user");
    assert!(!main_user.borrowed && main_user.mutable, "{main_user:?}");
}

#[test]
fn copy_and_borrowed_bindings_get_their_place_info() {
    let program = ok(&with_p(
        "fn f(p: P, mut q: P, n: i32) {\n    let a = p;\n    let mut b = p;\n    let mut c = q;\n    let d = q.name;\n    let mut e = p.x;\n    let s = \"lit\";\n}\n",
    ));
    let f = function(&program, "f");
    let from_p = Some(Origin::Param(f.params[0].local));
    let from_q = Some(Origin::Param(f.params[1].local));
    let shared = |origin| PlaceInfo {
        borrowed: true,
        mutable: false,
        origin,
    };
    assert_eq!(place(f, "n"), PlaceInfo::default());
    assert_eq!(place(f, "a"), shared(from_p));
    assert_eq!(place(f, "b"), shared(from_p));
    assert_eq!(
        place(f, "c"),
        PlaceInfo {
            borrowed: true,
            mutable: true,
            origin: from_q,
        }
    );
    let Some(Origin::Struct(_)) = place(f, "d").origin else {
        panic!("d should derive from the struct: {:?}", place(f, "d"));
    };
    assert!(place(f, "d").borrowed && !place(f, "d").mutable);
    assert_eq!(
        place(f, "e"),
        PlaceInfo {
            borrowed: false,
            mutable: true,
            origin: None,
        }
    );
    assert_eq!(place(f, "s"), PlaceInfo::default());
}

// --- String representation --------------------------------------------------

/// Helpers for the string tests: a maker, a `string` reader, a `mut string`
/// taker, and a struct reader.
const STRS: &str = "fn mk() -> string {\n    \"m\"\n}\nfn read(s: string) {}\nfn fill(mut s: string) {}\nfn show(p: P) {}\n";

fn with_strs(body: &str) -> String {
    format!("{P}{STRS}{body}{MAIN}")
}

#[test]
fn a_literal_let_that_is_only_read_stays_borrowed() {
    let program = ok(&with_strs(
        "fn f() -> bool {\n    let s = \"x\";\n    read(s);\n    println!(\"{}\", s);\n    let same = s == \"y\";\n    let t = s;\n    same\n}\n",
    ));
    let f = function(&program, "f");
    assert_eq!(repr(f, "s"), Some(StringRepr::Str));
    assert_eq!(repr(f, "t"), Some(StringRepr::Str));
}

#[test]
fn a_let_becomes_owned_when_used_as_an_owned_value() {
    let cases = [
        (
            "assigned a call result",
            "fn f() {\n    let mut s = \"x\";\n    s = mk();\n}\n",
        ),
        (
            "passed to a mut string",
            "fn f() {\n    let mut s = \"x\";\n    fill(s);\n}\n",
        ),
        (
            "stored into a field",
            "fn f() {\n    let s = \"x\";\n    let p = P { x: 1, name: s };\n}\n",
        ),
        (
            "assigned to a field",
            "fn f() {\n    let s = \"x\";\n    let mut p = P { x: 1, name: \"a\" };\n    p.name = s;\n}\n",
        ),
        (
            "returned",
            "fn f() -> string {\n    let s = \"x\";\n    s\n}\n",
        ),
        (
            "returned early",
            "fn f() -> string {\n    let s = \"x\";\n    return s;\n}\n",
        ),
        (
            "initialized from a call",
            "fn f() {\n    let s = mk();\n}\n",
        ),
    ];
    for (what, body) in cases {
        let program = ok(&with_strs(body));
        assert_eq!(
            repr(function(&program, "f"), "s"),
            Some(StringRepr::Owned),
            "{what}"
        );
    }
}

#[test]
fn ownership_propagates_through_lets_both_ways() {
    let program = ok(&with_strs(
        "fn f() -> string {\n    let a = \"x\";\n    let b = a;\n    b\n}\nfn g() {\n    let c = mk();\n    let mut d = \"y\";\n    d = c;\n    let e = d;\n}\n",
    ));
    let f = function(&program, "f");
    assert_eq!(repr(f, "a"), Some(StringRepr::Owned));
    assert_eq!(repr(f, "b"), Some(StringRepr::Owned));
    let g = function(&program, "g");
    for name in ["c", "d", "e"] {
        assert_eq!(repr(g, name), Some(StringRepr::Owned), "{name}");
    }
}

#[test]
fn parameters_and_borrowed_bindings_get_their_representation() {
    let program = ok(&with_strs(
        "fn f(s: string, mut t: string, n: i32, p: P) {\n    let a = s;\n    let b = t;\n    let c = p.name;\n    let q = p;\n}\n",
    ));
    let f = function(&program, "f");
    assert_eq!(repr(f, "s"), Some(StringRepr::Str));
    assert_eq!(repr(f, "t"), Some(StringRepr::MutOwned));
    assert_eq!(repr(f, "n"), None);
    assert_eq!(repr(f, "p"), None);
    assert_eq!(repr(f, "a"), Some(StringRepr::Str));
    assert_eq!(repr(f, "b"), Some(StringRepr::RefOwned));
    assert_eq!(repr(f, "c"), Some(StringRepr::RefOwned));
    assert_eq!(repr(f, "q"), None);
}

#[test]
fn representation_of_the_examples() {
    let program = check_path("examples/interop/main.vr").0.unwrap();
    assert_eq!(
        repr(function(&program, "main"), "name"),
        Some(StringRepr::Str)
    );
    let program = check_path("examples/borrowing.vr").0.unwrap();
    assert_eq!(repr(function(&program, "main"), "user"), None);
    assert_eq!(repr(function(&program, "rename"), "user"), None);
}

/// One case per row of the M1 section 4.3 table that binds a `string`
/// local, with the representation recorded for it. The rows for a literal
/// into an owned slot, a field to a `string` or `mut string` parameter,
/// and a borrowed place into an owned slot (rejected) bind none.
#[test]
fn representation_of_each_string_table_row() {
    use StringRepr::{MutOwned, Owned, RefOwned, Str};
    /// The row, the function `f`, and each local's expected representation.
    type Case = (
        &'static str,
        &'static str,
        &'static [(&'static str, StringRepr)],
    );
    let cases: [Case; 11] = [
        (
            "literal bound by `let`, binding stays borrowed",
            "fn f() {\n    let s = \"x\";\n    read(s);\n}\n",
            &[("s", Str)],
        ),
        (
            "literal bound by `let`, binding needs to be owned",
            "fn f() -> string {\n    let s = \"x\";\n    s\n}\n",
            &[("s", Owned)],
        ),
        (
            "owned local into a field",
            "fn f() {\n    let s = mk();\n    let p = P { x: 1, name: s };\n}\n",
            &[("s", Owned)],
        ),
        (
            "borrowed local to a `string` parameter",
            "fn f(s: string) {\n    let t = s;\n    read(t);\n}\n",
            &[("s", Str), ("t", Str)],
        ),
        (
            "owned local to a `string` parameter",
            "fn f() {\n    let s = mk();\n    read(s);\n}\n",
            &[("s", Owned)],
        ),
        (
            "owned local to a `mut string` parameter",
            "fn f() {\n    let mut s = \"x\";\n    fill(s);\n}\n",
            &[("s", Owned)],
        ),
        (
            "`let` from a field",
            "fn f(p: P) {\n    let n = p.name;\n}\n",
            &[("n", RefOwned)],
        ),
        (
            "changeable `let` from a field of a mutable place",
            "fn f(mut p: P) {\n    let mut n = p.name;\n    n = \"y\";\n}\n",
            &[("n", MutOwned)],
        ),
        (
            "`let` from a field or a literal",
            "fn f(p: P, c: bool) {\n    let n = if c { p.name } else { \"x\" };\n}\n",
            &[("n", Str)],
        ),
        (
            "assignment through a `mut string` parameter",
            "fn f(mut name: string) {\n    name = \"x\";\n    let b = name;\n}\n",
            &[("name", MutOwned), ("b", RefOwned)],
        ),
        (
            "`==` between any two strings",
            "fn f() -> bool {\n    let a = \"x\";\n    let b = mk();\n    a == b\n}\n",
            &[("a", Str), ("b", Owned)],
        ),
    ];
    for (row, body, expected) in cases {
        let program = ok(&with_strs(body));
        let f = function(&program, "f");
        for &(name, want) in expected {
            assert_eq!(repr(f, name), Some(want), "{row}: `{name}`");
        }
    }
}

// --- V0304: borrowed places into owned `let`s -------------------------------

#[test]
fn assigning_a_borrowed_place_to_an_owned_let_is_v0304() {
    let (d, sources) = one_error(&with_strs(
        "fn f(p: P) -> string {\n    let mut s = \"x\";\n    s = p.name;\n    s\n}\n",
    ));
    assert_v0304(&d, span_of(&sources, "p.name"), "`P`");
    assert!(d.message.contains("`s`"), "{d:#?}");

    let (d, sources) = one_error(&with_strs(
        "fn f(t: string) {\n    let mut s = \"x\";\n    s = t;\n    fill(s);\n}\n",
    ));
    assert_v0304(&d, part_of(&sources, "s = t;", "t"), "`t`");
}

#[test]
fn assigning_a_borrowed_place_to_a_whole_owned_local_is_v0304() {
    let (d, sources) = one_error(&with_strs(
        "fn f(p: P) {\n    let mut q = P { x: 0, name: \"q\" };\n    q = p;\n}\n",
    ));
    assert_v0304(&d, part_of(&sources, "q = p;", "p"), "`p`");
    assert!(d.message.contains("cannot be kept in `q`"), "{d:#?}");

    let (d, sources) = one_error(&with_strs(
        "fn f(name: string) {\n    let mut s = mk();\n    s = name;\n}\n",
    ));
    assert_v0304(&d, part_of(&sources, "s = name;", "name"), "`name`");
    assert!(d.message.contains("cannot be kept in `s`"), "{d:#?}");
}

#[test]
fn assigning_new_values_to_a_whole_owned_local_is_accepted() {
    ok(&with_strs(
        "fn mkp() -> P {\n    P { x: 1, name: \"p\" }\n}\nfn f() {\n    let mut q = P { x: 0, name: \"q\" };\n    q = mkp();\n    q = P { x: 2, name: \"r\" };\n    let mut s = mk();\n    s = \"lit\";\n    s = mk();\n}\n",
    ));
}

#[test]
fn assigning_a_borrowed_place_to_a_borrowed_let_is_accepted() {
    ok(&with_strs(
        "fn f(p: P, t: string) {\n    let mut s = \"x\";\n    s = p.name;\n    s = t;\n    read(s);\n}\n",
    ));
}

#[test]
fn a_let_mixing_a_borrowed_and_an_owned_branch_is_v0304_on_the_borrowed_one() {
    let (d, sources) = one_error(&with_strs(
        "fn f(c: bool, p: P) {\n    let s = if c {\n        p.name\n    } else {\n        mk()\n    };\n}\n",
    ));
    assert_v0304(&d, span_of(&sources, "p.name"), "`P`");

    let (d, sources) = one_error(&with_strs(
        "fn f(c: bool, t: string) {\n    let o = mk();\n    let s = if c {\n        o\n    } else {\n        t\n    };\n}\n",
    ));
    assert_v0304(&d, part_of(&sources, "        t\n", "t"), "`t`");
}

#[test]
fn a_mixed_let_already_rejected_by_its_use_gets_one_code() {
    let (d, sources) = one_error(&with_strs(
        "fn f(c: bool, p: P) -> string {\n    let s = if c {\n        p.name\n    } else {\n        mk()\n    };\n    s\n}\n",
    ));
    assert_v0304(&d, part_of(&sources, "    s\n}", "s"), "`P`");
}

// --- V0304: text that is sometimes new and sometimes borrowed -------------

const SOMETIMES: &str = "this value is sometimes new text and sometimes borrowed from `t`";

#[test]
fn printing_an_if_mixing_a_call_and_a_parameter_is_v0304() {
    let (d, sources) = one_error(&with_strs(
        "fn f(c: bool, t: string) {\n    println!(\"{}\", if c { mk() } else { t });\n}\n",
    ));
    assert_v0304(&d, span_of(&sources, "if c { mk() } else { t }"), "`t`");
    assert_eq!(d.message, SOMETIMES);
}

#[test]
fn comparing_an_if_mixing_a_call_and_a_parameter_is_v0304() {
    let (d, sources) = one_error(&with_strs(
        "fn f(c: bool, t: string) -> bool {\n    (if c { mk() } else { t }) == \"y\"\n}\n",
    ));
    assert_v0304(&d, span_of(&sources, "if c { mk() } else { t }"), "`t`");
    assert_eq!(d.message, SOMETIMES);
}

#[test]
fn discarding_or_lending_an_if_mixing_a_call_and_a_parameter_is_v0304() {
    let (d, sources) = one_error(&with_strs(
        "fn f(c: bool, t: string) {\n    if c { mk() } else { t };\n}\n",
    ));
    assert_v0304(&d, span_of(&sources, "if c { mk() } else { t }"), "`t`");

    let (d, sources) = one_error(&with_strs(
        "fn f(c: bool, t: string) {\n    read(if c { mk() } else { t });\n}\n",
    ));
    assert_v0304(&d, span_of(&sources, "if c { mk() } else { t }"), "`t`");
}

#[test]
fn passing_an_if_mixing_a_call_and_a_parameter_to_a_mut_parameter_is_only_v0304() {
    let (d, sources) = one_error(&with_strs(
        "fn f(c: bool, t: string) {\n    fill(if c { mk() } else { t });\n}\n",
    ));
    assert_v0304(&d, span_of(&sources, "if c { mk() } else { t }"), "`t`");
}

#[test]
fn an_if_mixing_a_call_and_an_owned_let_keeps_the_let_hint() {
    let (d, sources) = one_error(&with_strs(
        "fn f(c: bool) {\n    let s = mk();\n    println!(\"{}\", if c { mk() } else { s });\n}\n",
    ));
    assert_v0304(&d, span_of(&sources, "if c { mk() } else { s }"), "`s`");
    assert_eq!(
        d.message,
        "this value is sometimes new text and sometimes the value of `s`; store it with `let` first"
    );
}

#[test]
fn an_if_mixing_a_call_and_a_field_has_no_let_hint() {
    let (d, sources) = one_error(&with_strs(
        "fn f(c: bool) {\n    let p = P { x: 1, name: mk() };\n    println!(\"{}\", if c { mk() } else { p.name });\n}\n",
    ));
    assert_v0304(
        &d,
        span_of(&sources, "if c { mk() } else { p.name }"),
        "`p`",
    );
    assert_eq!(
        d.message,
        "this value is sometimes new text and sometimes borrowed from `p`"
    );
}

#[test]
fn an_if_mixing_a_call_with_literals_or_places_alone_is_accepted() {
    ok(&with_strs(
        "fn f(c: bool, t: string) -> bool {\n    println!(\"{}\", if c { mk() } else { \"y\" });\n    println!(\"{}\", if c { t } else { \"y\" });\n    read(if c { mk() } else { mk() });\n    (if c { mk() } else { \"y\" }) == t\n}\n",
    ));
}

#[test]
fn a_let_mixing_a_call_and_a_parameter_keeps_the_owned_let_check() {
    // The `let` form is caught by the owned-`let` check at the parameter,
    // not by the new rule.
    let (d, sources) = one_error(&with_strs(
        "fn f(c: bool, t: string) {\n    let s = if c { mk() } else { t };\n    read(s);\n}\n",
    ));
    assert_v0304(&d, part_of(&sources, "else { t }", "t"), "`t`");
    assert!(d.message.contains("cannot be kept in `s`"), "{d:#?}");
}

#[test]
fn lending_a_struct_if_mixing_a_new_value_and_a_parameter_is_v0304() {
    const MKP: &str = "fn mkp() -> P {\n    P { x: 1, name: \"p\" }\n}\n";
    let (d, sources) = one_error(&with_strs(&format!(
        "{MKP}fn f(c: bool, p: P) {{\n    show(if c {{ mkp() }} else {{ p }});\n}}\n"
    )));
    assert_v0304(&d, span_of(&sources, "if c { mkp() } else { p }"), "`p`");
    assert_eq!(
        d.message,
        "this value is sometimes new and sometimes borrowed from `p`; store it with `let` first"
    );

    let (d, sources) = one_error(&with_strs(
        "fn f(c: bool, p: P) {\n    show(if c { P { x: 1, name: \"n\" } } else { p });\n}\n",
    ));
    assert_v0304(
        &d,
        span_of(&sources, "if c { P { x: 1, name: \"n\" } } else { p }"),
        "`p`",
    );

    // The `let` form is accepted (the binding is a reference to either
    // value, and Rust extends the new one's life). A discarded one is not:
    // each branch would be borrowed, and the new value dies with its
    // branch.
    ok(&with_strs(&format!(
        "{MKP}fn f(c: bool, p: P) {{\n    let v = if c {{ mkp() }} else {{ p }};\n    show(v);\n}}\n"
    )));
    let (d, sources) = one_error(&with_strs(&format!(
        "{MKP}fn f(c: bool, p: P) {{\n    if c {{ mkp() }} else {{ p }};\n}}\n"
    )));
    assert_v0304(&d, span_of(&sources, "if c { mkp() } else { p }"), "`p`");
}

// --- V0304: values gone once their block ends --------------------------------

/// Helpers for the block tests: a struct maker, readers, and `mut` takers.
const BLOCKS: &str = "fn mkp() -> P {\n    P { x: 1, name: \"p\" }\n}\nfn fill_i(mut n: i32) {}\nfn both(a: string, mut b: string) {}\n";

fn with_blocks(body: &str) -> String {
    format!("{P}{STRS}{BLOCKS}{body}{MAIN}")
}

/// A V0304 for a value gone once its block ends, at `span`, whose message
/// contains `fragment`.
fn assert_gone(d: &Diagnostic, span: Span, fragment: &str) {
    assert_eq!(d.code, codes::V0304, "{d:#?}");
    assert_eq!(d.span, span, "{d:#?}");
    assert!(d.message.contains(fragment), "{d:#?}");
    assert!(has_note(d, "cannot outlive the value it borrows"), "{d:#?}");
}

#[test]
fn a_field_of_a_temporary_in_a_branch_counts_as_new_text() {
    // Borrowed as a whole with other new values, and rejected with a place.
    ok(&with_blocks(
        "fn f(c: bool) {\n    read(if c { mk() } else { mkp().name });\n    read({ mkp().name });\n    println!(\"{}\", if c { mkp().name } else { \"y\" });\n}\n",
    ));
    let (d, sources) = one_error(&with_blocks(
        "fn f(c: bool, t: string) {\n    read(if c { mkp().name } else { t });\n}\n",
    ));
    assert_v0304(
        &d,
        span_of(&sources, "if c { mkp().name } else { t }"),
        "`t`",
    );
    assert_eq!(d.message, SOMETIMES);
}

#[test]
fn a_local_declared_inside_a_block_counts_as_new() {
    ok(&with_blocks(
        "fn f(c: bool, t: string) {\n    let s = mk();\n    read({ let tmp = s; tmp });\n    show({ let a = mkp(); a });\n    read({ let a = mkp(); a.name });\n    read({ let r = t; r });\n}\n",
    ));
    let (d, sources) = one_error(&with_blocks(
        "fn f(c: bool, p: P) {\n    show(if c { let a = mkp(); a } else { p });\n}\n",
    ));
    assert_v0304(
        &d,
        span_of(&sources, "if c { let a = mkp(); a } else { p }"),
        "`p`",
    );
}

#[test]
fn assigning_a_field_of_a_block_local_to_an_outer_let_is_v0304() {
    for body in [
        "if c {\n        let t = mkp();\n        s = t.name;\n    }",
        "while c {\n        let t = mkp();\n        s = t.name;\n    }",
        "{\n        let t = mkp();\n        s = t.name;\n    }",
        "if c {\n        let t = mkp();\n        let u = t.name;\n        s = u;\n    }",
    ] {
        let (d, sources) = one_error(&with_blocks(&format!(
            "fn f(c: bool) {{\n    let mut s = \"a\";\n    {body}\n    read(s);\n}}\n"
        )));
        let target = if body.contains("s = u;") {
            "u"
        } else {
            "t.name"
        };
        assert_gone(
            &d,
            part_of(&sources, &format!("s = {target};"), target),
            "only exists inside this block, so it cannot be kept in `s`",
        );
    }
    // The same assignment from a place declared outside the block is fine,
    // and so is a `let` that lives in the block with the struct.
    ok(&with_blocks(
        "fn f(c: bool, p: P) {\n    let mut s = \"a\";\n    if c {\n        let t = p.name;\n        s = t;\n    }\n    read(s);\n}\n",
    ));
    ok(&with_blocks(
        "fn f(c: bool) {\n    if c {\n        let mut s = \"a\";\n        let t = mkp();\n        s = t.name;\n        read(s);\n    }\n}\n",
    ));
}

#[test]
fn a_part_of_a_match_or_for_binding_over_a_temporary_kept_outside_is_v0304() {
    const HEADS: &str = "fn mkps() -> Vec<P> {\n    vec![mkp()]\n}\nfn op() -> Option<P> {\n    None\n}\nfn mkv() -> Vec<string> {\n    vec![\"v\"]\n}\n";
    let program = |body: &str| {
        with_blocks(&format!(
            "{HEADS}fn f() {{\n    let mut s = \"a\";\n    {body}\n    read(s);\n}}\n"
        ))
    };
    for (body, stmt, leaf) in [
        ("for t in mkps() { s = t.name; }", "s = t.name;", "t.name"),
        (
            "match op() { Some(t) => { s = t.name; } None => {} }",
            "s = t.name;",
            "t.name",
        ),
        ("for t in mkps() { let u = t.name; s = u; }", "s = u;", "u"),
        (
            "for v in vec![mkps()] { s = v[0].name; }",
            "s = v[0].name;",
            "v[0].name",
        ),
        (
            "match op() { Some(t) => { let mut z = \"z\"; z = t.name; s = z; } None => {} }",
            "s = z;",
            "z",
        ),
    ] {
        let (d, sources) = one_error(&program(body));
        assert_gone(&d, part_of(&sources, stmt, leaf), "only exists inside this");
    }
    // Over a place, the binding's roots outlive the loop or arm; an owned
    // `String` moved out of a temporary is moved on.
    for body in [
        "let ps = mkps();\n    for t in ps { s = t.name; }",
        "let o = op();\n    match o { Some(t) => { s = t.name; } None => {} }",
        "let ps = mkps();\n    for t in ps { let u = t.name; s = u; }",
        "for n in mkv() { s = n; }",
    ] {
        ok(&program(body));
    }
}

#[test]
fn a_binding_inside_a_block_that_refers_to_something_gone_is_v0304() {
    for (body, name) in [
        ("{ let r = mkp().name; r }", "r"),
        ("{ let a = mkp(); let r = a.name; r }", "r"),
    ] {
        let (d, sources) = one_error(&with_blocks(&format!("fn f() {{\n    read({body});\n}}\n")));
        let tail = part_of(&sources, body, &format!("{name} }}"));
        assert_gone(
            &d,
            Span::new(tail.file, tail.start, tail.start + name.len() as u32),
            &format!("`{name}` refers to something that only exists inside this block"),
        );
    }
    // A binding inside a block that refers to a place outside it is fine.
    ok(&with_blocks(
        "fn f(p: P) {\n    read({ let r = p.name; r });\n}\n",
    ));
}

#[test]
fn a_mixed_if_or_block_read_in_place_is_v0304_for_every_type_and_use() {
    // Discarded struct, field base, and Copy or literal values lent mutably.
    for (body, value, name) in [
        (
            "fn f(c: bool, p: P) {\n    if c { mkp() } else { p };\n}\n",
            "if c { mkp() } else { p }",
            "`p`",
        ),
        (
            "fn f(c: bool, p: P) {\n    println!(\"{}\", (if c { mkp() } else { p }).name);\n}\n",
            "if c { mkp() } else { p }",
            "`p`",
        ),
        (
            "fn f(c: bool, mut n: i32) {\n    fill_i(if c { 5 } else { n });\n}\n",
            "if c { 5 } else { n }",
            "`n`",
        ),
        (
            "fn f(c: bool) {\n    let mut s = mk();\n    fill(if c { \"v\" } else { s });\n}\n",
            "if c { \"v\" } else { s }",
            "`s`",
        ),
    ] {
        let (d, sources) = one_error(&with_blocks(body));
        assert_v0304(&d, span_of(&sources, value), name);
    }
    // Copy values lent to a shared parameter are copied, so they may mix.
    ok(&with_blocks(
        "fn f(c: bool, n: i32) {\n    println!(\"{}\", if c { 5 } else { n });\n}\n",
    ));
}

#[test]
fn a_reference_let_of_a_block_keeps_nothing_declared_inside_it() {
    let (d, sources) = one_error(&with_blocks(
        "fn f() {\n    let x = { let a = mkp(); a.name };\n    read(x);\n}\n",
    ));
    assert_gone(
        &d,
        span_of(&sources, "a.name"),
        "`a` only exists inside this block, so it cannot be kept in `x`",
    );
    // A field of a temporary is kept when every branch is a reference to a
    // `String`, and not when another branch makes the binding a `&str`.
    ok(&with_blocks(
        "fn f(c: bool, p: P) {\n    let x = if c { mkp().name } else { p.name };\n    read(x);\n    let y = if c { mkp() } else { p };\n    show(y);\n}\n",
    ));
    let (d, sources) = one_error(&with_blocks(
        "fn f(c: bool) {\n    let x = if c { \"v\" } else { mkp().name };\n    read(x);\n}\n",
    ));
    assert_gone(
        &d,
        span_of(&sources, "mkp().name"),
        "this value is kept inside a `P` that is not stored anywhere, so it cannot be kept in `x`; store the `P` with `let` first",
    );
}

#[test]
fn assigning_a_field_of_a_temporary_to_a_let_is_v0304() {
    let (d, sources) = one_error(&with_blocks(
        "fn f() {\n    let mut x = \"a\";\n    x = mkp().name;\n    read(x);\n}\n",
    ));
    assert_gone(&d, span_of(&sources, "mkp().name"), "cannot be kept in `x`");
    ok(&with_blocks(
        "fn f(t: string) {\n    let mut x = \"a\";\n    x = { let r = t; r };\n    read(x);\n}\n",
    ));
}

#[test]
fn a_field_of_an_if_is_mutable_only_when_every_branch_is() {
    ok(&with_blocks(
        "fn f(c: bool, mut p: P) {\n    let mut q = mkp();\n    fill((if c { q } else { p }).name);\n    fill((if c { mkp() } else { mkp() }).name);\n}\n",
    ));
    let (d, sources) = one_error(&with_blocks(
        "fn f(c: bool, p: P, mut q: P) {\n    fill((if c { p } else { q }).name);\n}\n",
    ));
    assert_eq!(d.code, codes::V0303, "{d:#?}");
    assert_eq!(d.span, part_of(&sources, "{ p } else", "p"));
}

#[test]
fn v0306_looks_through_a_field_of_an_if() {
    let (d, _) = one_error(&with_blocks(
        "fn f(c: bool, mut p: P, mut q: P) {\n    both(p.name, (if c { p } else { q }).name);\n}\n",
    ));
    assert_eq!(d.code, codes::V0306, "{d:#?}");
}

#[test]
fn a_changeable_binding_of_text_is_never_a_str() {
    // The literal branch becomes a `String`; the binding is a mutable place.
    let program = ok(&with_blocks(
        "fn f(c: bool) {\n    let mut p = mkp();\n    let mut x = if c { \"v\" } else { p.name };\n    fill(x);\n    x = \"q\";\n}\n",
    ));
    assert!(place(function(&program, "f"), "x").mutable);
    // A `let` it may refer to becomes a `String`, and so cannot be one
    // branch of a reference.
    let (d, sources) = one_error(&with_blocks(
        "fn f(c: bool) {\n    let mut p = mkp();\n    let mut s = \"s\";\n    let mut x = if c { s } else { p.name };\n    fill(x);\n}\n",
    ));
    assert_v0304(
        &d,
        part_of(&sources, "p.name }", "p.name"),
        "cannot be kept in `x`",
    );
}

#[test]
fn changing_a_value_while_an_earlier_argument_holds_it_is_v0306() {
    for (body, at) in [
        (
            "fn f(mut p: P) {\n    both(p.name, { show(p); fill(p.name); mk() });\n}\n",
            "p.name); mk",
        ),
        (
            "fn f() {\n    let s = mk();\n    let b = s == { let t = s; t };\n}\n",
            "s; t",
        ),
    ] {
        let (d, sources) = one_error(&with_blocks(body));
        assert_eq!(d.code, codes::V0306, "{d:#?}");
        let name = if at.starts_with('p') { "p.name" } else { "s" };
        assert_eq!(d.span, part_of(&sources, at, name), "{d:#?}");
    }
    // Reading it again on the way, or changing it before it is lent, is fine.
    ok(&with_blocks(
        "fn f(mut p: P) {\n    both({ fill(p.name); p.name }, mk());\n    read2(p.name, { read(p.name); mk() });\n}\nfn read2(a: string, b: string) {}\n",
    ));
}

#[test]
fn a_let_that_only_renames_a_lent_value_is_not_v0306() {
    // A `let` from a field or a parameter is another name, not a move.
    ok(&with_blocks(
        "fn pt(a: P, b: string) {}\nfn f(p: P) {\n    let o = mkp();\n    pt(o, { let t = o.name; t });\n    pt(o, { let t = o.name; read(t); mk() });\n    pt(p, { let q = p; read(q.name); mk() });\n}\n",
    ));
    // A `let` that moves an owned local still gives it away.
    let (d, _) = one_error(&with_blocks(
        "fn f() {\n    let s = mk();\n    let b = s == { let t = s; t };\n}\n",
    ));
    assert_eq!(d.code, codes::V0306, "{d:#?}");
}

// --- V0305: use after move --------------------------------------------------

fn assert_v0305(d: &Diagnostic, at: Span, moved: Span, name: &str) {
    assert_eq!(d.code, codes::V0305, "{d:#?}");
    assert_eq!(d.span, at, "{d:#?}");
    assert!(d.message.contains(&format!("`{name}`")), "{d:#?}");
    assert!(
        d.labels
            .iter()
            .any(|l| l.span == moved && l.text == format!("`{name}` is given away here")),
        "{d:#?}"
    );
}

#[test]
fn using_a_struct_after_let_moves_it_is_v0305() {
    let (d, sources) = one_error(&with_strs(
        "fn f() {\n    let a = P { x: 1, name: \"a\" };\n    let b = a;\n    show(a);\n}\n",
    ));
    assert_v0305(
        &d,
        part_of(&sources, "show(a)", "a"),
        part_of(&sources, "let b = a", "a"),
        "a",
    );
}

#[test]
fn using_an_owned_string_after_a_move_is_v0305() {
    let (d, sources) = one_error(&with_strs(
        "fn f() {\n    let a = mk();\n    let b = a;\n    println!(\"{}\", a);\n}\n",
    ));
    assert_v0305(
        &d,
        part_of(&sources, "\", a)", "a"),
        part_of(&sources, "let b = a", "a"),
        "a",
    );
}

#[test]
fn reading_a_field_of_a_moved_struct_is_v0305() {
    let (d, sources) = one_error(&with_strs(
        "fn f() -> P {\n    let a = P { x: 1, name: \"a\" };\n    let w = P { x: 2, name: \"w\" };\n    let b = a;\n    let x = a.x;\n    b\n}\n",
    ));
    assert_v0305(
        &d,
        part_of(&sources, "a.x", "a"),
        part_of(&sources, "let b = a", "a"),
        "a",
    );
}

#[test]
fn borrowing_parameters_do_not_move() {
    ok(&with_strs(
        "fn rename(mut p: P) {}\nfn f() {\n    let mut user = P { x: 1, name: \"a\" };\n    show(user);\n    show(user);\n    rename(user);\n    show(user);\n    let s = mk();\n    read(s);\n    read(s);\n    println!(\"{}\", s);\n    let same = s == s;\n}\n",
    ));
}

#[test]
fn passing_an_owned_string_to_an_imported_string_parameter_moves_it() {
    let (result, sources) =
        check_path("crates/varyk/tests/fixtures/interop/moved_into_owned/main.vr");
    let diagnostics = result.expect_err("should fail");
    assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    assert_v0305(
        &diagnostics[0],
        part_of(&sources, "\"{}\", s)", "s"),
        part_of(&sources, "take(s)", "s"),
        "s",
    );
}

#[test]
fn copy_values_and_borrowed_places_do_not_move() {
    ok(&with_strs(
        "fn f(p: P, t: string) {\n    let x = 1;\n    let y = x;\n    println!(\"{}\", x);\n    let a = p;\n    let b = a;\n    show(a);\n    let n = p.name;\n    let m = n;\n    read(n);\n    let u = t;\n    let v = u;\n    read(u);\n}\n",
    ));
}

#[test]
fn a_move_inside_an_if_counts_afterwards() {
    let (d, sources) = one_error(&with_strs(
        "fn f(c: bool) {\n    let a = P { x: 1, name: \"a\" };\n    if c {\n        let b = a;\n    }\n    show(a);\n}\n",
    ));
    assert_v0305(
        &d,
        part_of(&sources, "show(a)", "a"),
        part_of(&sources, "let b = a", "a"),
        "a",
    );
    let (d, _) = one_error(&with_strs(
        "fn f(c: bool) {\n    let a = P { x: 1, name: \"a\" };\n    if c {\n        show(a);\n    } else {\n        let b = a;\n    }\n    show(a);\n}\n",
    ));
    assert_eq!(d.code, codes::V0305);
}

#[test]
fn a_move_in_a_branch_that_returns_does_not_count_afterwards() {
    ok(&with_strs(
        "fn f(c: bool) -> P {\n    let a = P { x: 1, name: \"a\" };\n    if c {\n        return a;\n    }\n    show(a);\n    a\n}\n",
    ));
}

#[test]
fn a_move_inside_a_while_is_v0305_on_the_next_iteration() {
    let (d, sources) = one_error(&with_strs(
        "fn f(c: bool) {\n    let a = P { x: 1, name: \"a\" };\n    while c {\n        let b = a;\n    }\n}\n",
    ));
    let at = part_of(&sources, "let b = a", "a");
    assert_v0305(&d, at, at, "a");
    assert!(has_note(&d, "loop"), "{d:#?}");

    ok(&with_strs(
        "fn f(c: bool) {\n    while c {\n        let a = P { x: 1, name: \"a\" };\n        let b = a;\n    }\n    let d = P { x: 1, name: \"d\" };\n    while c {\n        let e = d;\n        break;\n    }\n}\n",
    ));
}

#[test]
fn reassigning_a_moved_local_revives_it() {
    ok(&with_strs(
        "fn f(c: bool) {\n    let mut a = P { x: 1, name: \"a\" };\n    let b = a;\n    a = P { x: 2, name: \"b\" };\n    show(a);\n    let mut s = mk();\n    while c {\n        let t = s;\n        s = mk();\n    }\n    read(s);\n}\n",
    ));
}

#[test]
fn moving_into_owned_slots_moves() {
    let (d, sources) = one_error(&with_strs(
        "fn f() {\n    let s = mk();\n    let p = P { x: 1, name: s };\n    read(s);\n}\n",
    ));
    assert_v0305(
        &d,
        part_of(&sources, "read(s)", "s"),
        part_of(&sources, "name: s }", "s"),
        "s",
    );
    let (d, _) = one_error(&with_strs(
        "fn f() {\n    let s = mk();\n    let mut p = P { x: 1, name: \"p\" };\n    p.name = s;\n    read(s);\n}\n",
    ));
    assert_eq!(d.code, codes::V0305);
}

#[test]
fn a_third_consuming_use_still_points_at_the_first_move() {
    // The first `let` moves `a`; the next two are uses after that move and
    // must each report V0305 pointing back at the *first* move, not at the
    // previous use-after-move.
    let (diagnostics, sources) = errors(&with_strs(
        "fn f() {\n    let a = mk();\n    let b = a;\n    let c = a;\n    let d = a;\n}\n",
    ));
    assert_eq!(diagnostics.len(), 2, "{diagnostics:#?}");
    let moved = part_of(&sources, "let b = a", "a");
    assert_v0305(
        &diagnostics[0],
        part_of(&sources, "let c = a", "a"),
        moved,
        "a",
    );
    assert_v0305(
        &diagnostics[1],
        part_of(&sources, "let d = a", "a"),
        moved,
        "a",
    );
}

#[test]
fn using_an_enum_or_a_vec_after_let_moves_it_is_v0305() {
    for ty in ["E", "Vec<i32>"] {
        let (d, sources) = one_error(&format!(
            "enum E {{\n    A,\n}}\nfn make(x: {ty}) -> {ty} {{\n    make(x)\n}}\nfn f(x: {ty}) {{\n    let a = make(x);\n    let b = a;\n    let c = a;\n}}\nfn main() {{}}\n"
        ));
        assert_v0305(
            &d,
            part_of(&sources, "let c = a", "a"),
            part_of(&sources, "let b = a", "a"),
            "a",
        );
    }
}

// --- V0306: one value passed twice to a call, once to `mut` -------------------

const TWICE: &str =
    "fn both(mut a: P, mut b: P) {}\nfn mixed(mut a: P, s: string) {}\nfn shared(a: P, b: P) {}\n";

fn with_twice(body: &str) -> String {
    format!("{P}{TWICE}{body}{MAIN}")
}

#[test]
fn same_local_to_two_mut_parameters_is_v0306() {
    let (d, sources) = one_error(&with_twice(
        "fn f() {\n    let mut p = P { x: 1, name: \"a\" };\n    both(p, p);\n}\n",
    ));
    assert_eq!(d.code, codes::V0306);
    assert_eq!(
        d.message,
        "`p` is passed to this call more than once, and one of those may change it"
    );
    let call = span_of(&sources, "both(p, p)");
    assert_eq!(d.span.start, call.start + 8);
    assert_eq!(d.labels.len(), 1);
    assert_eq!(d.labels[0].span.start, call.start + 5);
}

#[test]
fn same_local_to_a_mut_and_a_shared_parameter_is_v0306() {
    let (d, sources) = one_error(&with_twice(
        "fn f() {\n    let mut p = P { x: 1, name: \"a\" };\n    mixed(p, p.name);\n}\n",
    ));
    assert_eq!(d.code, codes::V0306);
    assert_eq!(d.span, part_of(&sources, "p, p.name", "p.name"));
}

#[test]
fn distinct_locals_or_shared_twice_are_accepted() {
    ok(&with_twice(
        "fn f() {\n    let mut p = P { x: 1, name: \"a\" };\n    let mut q = P { x: 2, name: \"b\" };\n    both(p, q);\n    shared(p, p);\n}\n",
    ));
}

#[test]
fn same_local_through_an_if_branch_is_v0306() {
    let (d, sources) = one_error(&with_twice(
        "fn f(c: bool) {\n    let mut p = P { x: 1, name: \"a\" };\n    both(if c { p } else { p }, p);\n}\n",
    ));
    assert_eq!(d.code, codes::V0306);
    assert_eq!(
        d.message,
        "`p` is passed to this call more than once, and one of those may change it"
    );
    let call_args = span_of(&sources, "if c { p } else { p }, p");
    assert_eq!(d.labels.len(), 1);
    assert_eq!(d.labels[0].span, span_of(&sources, "if c { p } else { p }"));
    assert_eq!(d.span.start, call_args.end - 1);
}

#[test]
fn if_branches_with_distinct_locals_are_accepted() {
    ok(&with_twice(
        "fn f(c: bool) {\n    let mut p = P { x: 1, name: \"a\" };\n    let mut q = P { x: 2, name: \"b\" };\n    let mut r = P { x: 3, name: \"c\" };\n    both(if c { p } else { q }, r);\n}\n",
    ));
}

#[test]
fn owned_non_copy_argument_after_a_shared_borrow_of_the_same_local_is_v0306() {
    let (result, sources) =
        check_path("crates/varyk/tests/fixtures/interop/owned_after_borrow/main.vr");
    let diagnostics = result.expect_err("should fail");
    assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    let d = &diagnostics[0];
    assert_eq!(d.code, codes::V0306);
    assert_eq!(
        d.message,
        "`s` is passed to this call more than once, and one of those may change it"
    );
    let call = span_of(&sources, "both(s, s)");
    assert_eq!(d.span.start, call.start + 8);
    // `good` passes distinct locals `s` and `t` to the same signature and
    // is not flagged: the file's only diagnostic is `bad`'s.
    assert!(sources[0].text.contains("ext::both(s, t)"), "{sources:#?}");
}

#[test]
fn copy_argument_beside_a_mutable_borrow_of_the_same_local_is_v0306_either_order() {
    let (result, sources) =
        check_path("crates/varyk/tests/fixtures/interop/copy_beside_mut_borrow/main.vr");
    let diagnostics = result.expect_err("should fail");
    assert_eq!(diagnostics.len(), 2, "{diagnostics:#?}");
    for d in &diagnostics {
        assert_eq!(d.code, codes::V0306);
        assert_eq!(
            d.message,
            "`x` is passed to this call more than once, and one of those may change it"
        );
    }
    // `f(_: &mut i32, _: i32)` called `f(x, x)`: the mutable borrow comes
    // first, the Copy read second.
    let f_call = span_of(&sources, "f(x, x)");
    assert!(
        diagnostics.iter().any(|d| d.span.start == f_call.start + 5),
        "{diagnostics:#?}"
    );
    // `rev(_: i32, _: &mut i32)` called `rev(x, x)`: the Copy read comes
    // first, the mutable borrow second. The conflict is still caught.
    let rev_call = span_of(&sources, "rev(x, x)");
    assert!(
        diagnostics
            .iter()
            .any(|d| d.span.start == rev_call.start + 7),
        "{diagnostics:#?}"
    );
    // `good` passes `x` and `y` to `f`, and `good_shared` passes a Copy
    // read of `x` beside a shared borrow of `x`: neither is flagged.
    assert!(sources[0].text.contains("ext::f(x, y)"), "{sources:#?}");
    assert!(
        sources[0].text.contains("ext::shared(x, x)"),
        "{sources:#?}"
    );
}

#[test]
fn same_local_to_a_varyk_declared_mut_and_owned_copy_parameter_is_v0306() {
    let (d, sources) = one_error(
        "fn g(mut a: i32, b: i32) {}\nfn main() {\n    let mut x = 1;\n    g(x, x);\n}\n",
    );
    assert_eq!(d.code, codes::V0306);
    assert_eq!(
        d.message,
        "`x` is passed to this call more than once, and one of those may change it"
    );
    let call = span_of(&sources, "g(x, x)");
    assert_eq!(d.span.start, call.start + 5);
}

#[test]
fn two_copy_reads_of_the_same_local_are_accepted() {
    ok("fn h(a: i32, b: i32) {}\nfn main() {\n    let x = 1;\n    h(x, x);\n}\n");
}

#[test]
fn changing_method_with_a_copy_element_of_the_receiver_argument_notes_a_plain_let() {
    let (d, _) = one_error(
        "fn main() {\n    let mut v: Vec<i32> = Vec::new();\n    v.push(1);\n    v.push(v[0]);\n}\n",
    );
    assert_eq!(d.code, codes::V0306);
    assert!(
        has_note(&d, "store it with `let` first, as in `let x = v[0];`"),
        "{d:#?}"
    );
}

#[test]
fn changing_method_with_a_struct_element_of_the_receiver_argument_notes_looping_by_index() {
    let (d, _) = one_error(
        "struct Item {\n    n: i32,\n}\nstruct Bag {\n    items: Vec<Item>,\n}\nimpl Bag {\n    fn add(mut self, item: Item) {}\n}\nfn main() {\n    let mut b = Bag { items: Vec::new() };\n    b.items.push(Item { n: 1 });\n    b.add(b.items[0]);\n}\n",
    );
    assert_eq!(d.code, codes::V0306);
    assert!(
        has_note(
            &d,
            "store a copy first, or loop by index and change the element in place"
        ),
        "{d:#?}"
    );
}

#[test]
fn changing_method_with_a_struct_field_of_the_receiver_argument_never_mentions_a_loop() {
    let (d, _) = one_error(
        "struct Item {\n    n: i32,\n}\nstruct Bag {\n    current: Item,\n}\nimpl Bag {\n    fn add(mut self, item: Item) {}\n}\nfn main() {\n    let mut b = Bag { current: Item { n: 1 } };\n    b.add(b.current);\n}\n",
    );
    assert_eq!(d.code, codes::V0306);
    assert!(has_note(&d, "store a copy first"), "{d:#?}");
    assert!(!has_note(&d, "loop"), "{d:#?}");
}

#[test]
fn changing_method_with_a_string_element_of_the_receiver_argument_notes_the_clone_fix() {
    let (d, _) = one_error(
        "struct Bag {\n    items: Vec<string>,\n}\nimpl Bag {\n    fn add(mut self, item: string) {}\n}\nfn main() {\n    let mut b = Bag { items: Vec::new() };\n    b.items.push(\"a\");\n    b.add(b.items[0]);\n}\n",
    );
    assert_eq!(d.code, codes::V0306);
    assert!(
        has_note(
            &d,
            "store this value with `let` first, as in `let x = b.items[0].clone();`"
        ),
        "{d:#?}"
    );
}

// --- V0306: println! arguments ------------------------------------------------

#[test]
fn println_argument_that_changes_an_earlier_arguments_root_is_v0306() {
    let (d, sources) = one_error(
        "struct P {\n    x: i32,\n    name: string,\n}\nfn change(mut p: P) {\n    p.x = 2;\n}\n\
         fn main() {\n    let mut p = P { x: 1, name: \"a\" };\n    \
         println!(\"{} {}\", p.name, { change(p); \"x\" });\n}\n",
    );
    assert_eq!(d.code, codes::V0306);
    assert_eq!(
        d.message,
        "`p` is used here while it is still lent out earlier in the same expression"
    );
    assert_eq!(d.span, part_of(&sources, "change(p)", "p"));
    assert_eq!(d.labels.len(), 1);
    assert_eq!(d.labels[0].span, span_of(&sources, "p.name"));
}

#[test]
fn println_reading_the_same_field_twice_is_accepted() {
    ok(
        "struct P {\n    x: i32,\n    name: string,\n}\nfn main() {\n    \
         let p = P { x: 1, name: \"a\" };\n    println!(\"{} {}\", p.name, p.name);\n}\n",
    );
}

// A Copy value is still lent for the whole `println!` call, unlike an
// ordinary call, because the format machinery borrows every argument (spec
// 4.2): a later argument that assigns to a Copy-typed earlier one is
// V0306 even though rustc would accept the same argument passed by value
// to an ordinary function.

#[test]
fn println_argument_that_assigns_a_copy_local_read_earlier_is_v0306() {
    let (d, sources) = one_error(
        "fn main() {\n    let mut x = 0;\n    \
         println!(\"{} {}\", x, { x = 1; 2 });\n}\n",
    );
    assert_eq!(d.code, codes::V0306);
    assert_eq!(
        d.message,
        "`x` is used here while it is still lent out earlier in the same expression"
    );
    assert_eq!(d.span, part_of(&sources, "x = 1", "x"));
    assert_eq!(d.labels.len(), 1);
    assert_eq!(d.labels[0].span, part_of(&sources, "\", x, {", "x"));
}

#[test]
fn println_reading_the_same_copy_local_twice_is_accepted() {
    ok("fn main() {\n    let x = 0;\n    println!(\"{} {}\", x, x);\n}\n");
}

#[test]
fn println_argument_that_only_renames_a_copy_local_read_earlier_is_accepted() {
    ok("fn main() {\n    let x = 0;\n    \
         println!(\"{} {}\", x, { let y = x; y });\n}\n");
}

#[test]
fn call_argument_that_assigns_a_copy_local_read_earlier_is_accepted() {
    // Unlike `println!`, an ordinary call passes a Copy argument by value:
    // nothing of the caller's stays borrowed once it is evaluated, so a
    // later argument reassigning it is fine (rustc accepts it too).
    ok(
        "fn f(a: i32, b: i32) {}\nfn main() {\n    let mut x = 0;\n    \
         f(x, { x = 1; 2 });\n}\n",
    );
}

// --- V0307: a place changed while another name for it is still used ---------

const ALIASES: &str = "struct P {\n    s: string,\n    n: i32,\n}\nfn read_s(s: string) {}\nfn read_p(p: P) {}\nfn change_p(mut p: P) {\n    p.n = 5;\n}\n";

fn with_aliases(body: &str) -> String {
    format!("{ALIASES}{body}{MAIN}")
}

/// Asserts `d` is V0307 at `changed`, naming `root` and `alias`, with a
/// label at `used` and a note giving the Rust term.
fn assert_v0307(d: &Diagnostic, changed: Span, used: Span, root: &str, alias: &str) {
    assert_eq!(d.code, codes::V0307, "{d:#?}");
    assert_eq!(d.span, changed, "{d:#?}");
    assert!(d.message.contains(&format!("`{root}`")), "{d:#?}");
    assert!(d.message.contains(&format!("`{alias}`")), "{d:#?}");
    assert!(
        d.labels
            .iter()
            .any(|l| l.span == used && l.text.contains(&format!("`{alias}`"))),
        "{d:#?}"
    );
    assert!(has_note(d, "in Rust terms"), "{d:#?}");
}

#[test]
fn changing_the_root_of_an_alias_used_later_is_v0307() {
    let (d, sources) = one_error(&with_aliases(
        "fn f(mut p: P) {\n    let n = p.s;\n    change_p(p);\n    read_s(n);\n}\n",
    ));
    assert_v0307(
        &d,
        part_of(&sources, "(p);\n    read_s(n)", "p"),
        part_of(&sources, "read_s(n);\n}", "n"),
        "p",
        "n",
    );
}

#[test]
fn changing_the_root_after_the_alias_is_last_used_is_accepted() {
    ok(&with_aliases(
        "fn f(mut p: P) {\n    let n = p.s;\n    read_s(n);\n    change_p(p);\n}\n",
    ));
}

#[test]
fn using_the_root_of_a_mutable_alias_used_later_is_v0307() {
    let (d, sources) = one_error(&with_aliases(
        "fn f(mut p: P) {\n    let mut m = p.s;\n    read_p(p);\n    m = \"x\";\n}\n",
    ));
    assert_v0307(
        &d,
        part_of(&sources, "(p);\n    m", "p"),
        part_of(&sources, "m = \"x\"", "m"),
        "p",
        "m",
    );
}

#[test]
fn an_alias_used_only_before_the_root_changes_is_accepted() {
    ok(&with_aliases(
        "fn f(mut p: P) {\n    let n = p.s;\n    read_s(n);\n    let mut m = p.s;\n    m = \"y\";\n    read_s(m);\n    change_p(p);\n    read_p(p);\n    p.n = 2;\n}\n",
    ));
}

#[test]
fn changing_the_root_of_an_alias_made_by_assignment_is_v0307() {
    let (d, sources) = one_error(&with_aliases(
        "fn f() {\n    let mut lp = P { s: \"p\", n: 1 };\n    let mut x = \"a\";\n    x = lp.s;\n    change_p(lp);\n    read_s(x);\n}\n",
    ));
    assert_v0307(
        &d,
        part_of(&sources, "(lp);\n    read_s(x)", "lp"),
        part_of(&sources, "read_s(x);\n}", "x"),
        "lp",
        "x",
    );
}

#[test]
fn an_alias_made_by_assignment_used_only_before_the_change_is_accepted() {
    ok(&with_aliases(
        "fn f() {\n    let mut lp = P { s: \"p\", n: 1 };\n    let mut x = \"a\";\n    change_p(lp);\n    x = lp.s;\n    read_s(x);\n    change_p(lp);\n}\n",
    ));
}

#[test]
fn a_copy_of_an_alias_made_by_assignment_is_an_alias_too() {
    for copy in ["let z = x;", "let mut z = \"b\";\n    z = x;"] {
        let (d, sources) = one_error(&with_aliases(&format!(
            "fn f() {{\n    let mut lp = P {{ s: \"p\", n: 1 }};\n    let mut x = \"a\";\n    x = lp.s;\n    {copy}\n    x = \"c\";\n    change_p(lp);\n    read_s(z);\n}}\n"
        )));
        assert_v0307(
            &d,
            part_of(&sources, "(lp);\n    read_s(z)", "lp"),
            part_of(&sources, "read_s(z);\n}", "z"),
            "lp",
            "z",
        );
    }
    ok(&with_aliases(
        "fn f() {\n    let mut x = \"a\";\n    let z = x;\n    x = \"c\";\n    read_s(z);\n    read_s(x);\n}\n",
    ));
}

// --- Values and constructors: owned slots (spec 3.4) and `format!` --------

const E: &str = "enum E {\n    A(string),\n    B(P),\n}\n";

fn with_e(body: &str) -> String {
    format!("{P}{E}{STRS}{body}{MAIN}")
}

#[test]
fn a_payload_of_a_borrowed_field_is_v0304() {
    let (d, sources) = one_error(&with_e("fn f(p: P) -> E {\n    E::A(p.name)\n}\n"));
    assert_v0304(&d, span_of(&sources, "p.name"), "`P`");
    assert!(d.message.contains("`E::A`"), "{d:#?}");
}

#[test]
fn every_constructor_argument_and_vec_element_is_an_owned_slot() {
    for (value, at) in [("E::B(p)", "p)"), ("Some(p)", "p)"), ("vec![p]", "p]")] {
        let text = with_e(&format!("fn f(p: P) {{\n    let v = {value};\n}}\n"));
        let (d, sources) = one_error(&text);
        assert_v0304(&d, part_of(&sources, at, "p"), "`p`");
    }
    for value in ["Ok(s)", "Err(s)"] {
        let text = with_e(&format!(
            "fn f(s: string) -> Result<string, string> {{\n    let r: Result<string, string> = {value};\n    r\n}}\n"
        ));
        let (d, sources) = one_error(&text);
        assert_v0304(&d, part_of(&sources, "(s)", "s"), "`s`");
    }
}

#[test]
fn an_owned_local_moves_into_a_payload_or_an_element() {
    for value in ["Some(s)", "E::A(s)", "vec![s]"] {
        let text = with_e(&format!(
            "fn f() {{\n    let s = mk();\n    let v = {value};\n    read(s);\n}}\n"
        ));
        let (d, _) = one_error(&text);
        assert_eq!(d.code, codes::V0305, "{value}: {d:#?}");
    }
    let (d, _) = one_error(&with_e(
        "fn f() {\n    let p = P { x: 1, name: \"a\" };\n    let v = vec![p, p];\n}\n",
    ));
    assert_eq!(d.code, codes::V0305, "{d:#?}");
}

#[test]
fn a_literal_let_flowing_into_a_payload_or_an_element_is_owned() {
    let program = ok(&with_e(
        "fn f() {\n    let a = \"a\";\n    let b = \"b\";\n    let c = \"c\";\n    let x = Some(a);\n    let y = E::A(b);\n    let z = vec![c];\n    let lit = Some(\"d\");\n}\n",
    ));
    let f = function(&program, "f");
    for name in ["a", "b", "c"] {
        assert_eq!(repr(f, name), Some(StringRepr::Owned), "{name}");
    }
}

#[test]
fn a_let_holding_a_format_result_is_owned() {
    let program = ok(&with_e(
        "fn f() {\n    let s = format!(\"{}\", 1);\n    let mut t = \"a\";\n    t = format!(\"{}!\", s);\n    read(t);\n}\n",
    ));
    let f = function(&program, "f");
    assert_eq!(repr(f, "s"), Some(StringRepr::Owned));
    assert_eq!(repr(f, "t"), Some(StringRepr::Owned));
}

#[test]
fn a_format_argument_changed_by_a_later_one_is_v0306() {
    let (d, _) = one_error(&with_takers(
        "fn f() {\n    let mut n = 1;\n    let s = format!(\"{} {}\", n, { inc(n); 2 });\n}\n",
    ));
    assert_eq!(d.code, codes::V0306, "{d:#?}");
}

#[test]
fn giving_away_the_root_of_an_alias_into_a_payload_is_v0307() {
    let (d, _) = one_error(&with_e(
        "fn f() {\n    let p = P { x: 1, name: \"a\" };\n    let n = p.name;\n    let e = E::B(p);\n    read(n);\n}\n",
    ));
    assert_eq!(d.code, codes::V0307, "{d:#?}");
}

// --- Methods, the built-in table, and indexing (spec 2.5, 2.6, 3.1) ---------

const TASK: &str = "struct Task {\n    label: string,\n    done: bool,\n}\nimpl Task {\n    fn new(label: string) -> Task {\n        Task { label: label.clone(), done: false }\n    }\n\n    fn complete(mut self) {\n        self.done = true;\n    }\n\n    fn label(self) -> string {\n        self.label.clone()\n    }\n}\nfn read(s: string) {}\nfn show(t: Task) {}\nfn mk() -> Task {\n    Task::new(\"m\")\n}\n";

fn with_task(body: &str) -> String {
    format!("{TASK}{body}{MAIN}")
}

#[test]
fn an_index_that_changes_the_vec_it_reads_is_v0306() {
    let (d, sources) = one_error(
        "fn g(mut v: Vec<i32>) -> usize {\n    0\n}\nfn main() {\n    let mut v = vec![1];\n    let x = v[g(v)];\n}\n",
    );
    assert_eq!(d.code, codes::V0306, "{d:#?}");
    assert_eq!(d.span, part_of(&sources, "g(v)", "v"));
    assert!(d.message.contains("`v` is changed in this index"), "{d:#?}");
    let (d, _) = one_error(
        "struct S {\n    v: Vec<string>,\n}\nimpl S {\n    fn idx(mut self) -> usize {\n        0\n    }\n}\nfn main() {\n    let mut s = S { v: vec![\"a\"] };\n    println!(\"{}\", s.v[s.idx()]);\n}\n",
    );
    assert_eq!(d.code, codes::V0306, "{d:#?}");
    // Reading the `Vec` in its own index is fine.
    ok("fn main() {\n    let v = vec![1];\n    let x = v[v.len() - 1];\n}\n");
}

#[test]
fn a_mut_self_method_on_a_let_binding_is_v0302_with_its_fix_it() {
    let (d, sources) = one_error(&with_task(
        "fn f() {\n    let t = mk();\n    t.complete();\n}\n",
    ));
    assert_eq!(d.code, codes::V0302, "{d:#?}");
    assert_eq!(d.span, part_of(&sources, "t.complete()", "t"));
    assert!(d.message.contains("`complete`"), "{d:#?}");
    assert_mut_fix_it(&d, &sources, "let t = mk()", "t = mk()");
}

#[test]
fn a_mut_self_method_on_a_non_mut_parameter_is_v0303() {
    let (d, sources) = one_error(&with_task("fn f(t: Task) {\n    t.complete();\n}\n"));
    assert_eq!(d.code, codes::V0303, "{d:#?}");
    assert_mut_fix_it(&d, &sources, "fn f(t: Task)", "t: Task");
}

#[test]
fn a_changing_built_in_on_a_non_mut_place_is_v0300_or_v0301() {
    let (d, sources) = one_error("fn f(v: Vec<i32>) {\n    v.push(1);\n}\nfn main() {}\n");
    assert_eq!(d.code, codes::V0300, "{d:#?}");
    assert_eq!(d.span, part_of(&sources, "v.push", "v"));
    assert_mut_fix_it(&d, &sources, "fn f(v: Vec<i32>)", "v: Vec");

    let (d, sources) =
        one_error("fn main() {\n    let v: Vec<i32> = vec![];\n    let x = v.pop();\n}\n");
    assert_eq!(d.code, codes::V0301, "{d:#?}");
    assert_mut_fix_it(&d, &sources, "let v: Vec", "v: Vec");
}

#[test]
fn reading_methods_and_changes_on_mutable_places_are_accepted() {
    ok(&with_task(
        "fn f(t: Task, v: Vec<Task>, mut w: Vec<Task>) {\n    read(t.label());\n    let n = v.len();\n    w.push(mk());\n    w[0].complete();\n    mk().complete();\n    let mut tasks = vec![mk()];\n    tasks[0].complete();\n    tasks.push(Task::new(\"b\"));\n    let last = tasks.pop();\n}\n",
    ));
}

#[test]
fn an_element_let_is_an_alias_of_its_vec_unless_copy() {
    let program = ok(&with_task(
        "fn f(v: Vec<Task>) {\n    let mut tasks = vec![mk()];\n    let first = tasks[0];\n    show(first);\n    let mut second = tasks[0];\n    second.complete();\n    let from_param = v[0];\n    let ns = vec![1, 2];\n    let n = ns[1];\n    show(from_param);\n}\n",
    ));
    let f = function(&program, "f");
    let tasks = f
        .locals
        .iter()
        .position(|l| l.name == "tasks")
        .expect("tasks");
    let tasks = LocalId(tasks as u32);
    assert_eq!(
        place(f, "first"),
        PlaceInfo {
            borrowed: true,
            mutable: false,
            origin: Some(Origin::Local(tasks)),
        }
    );
    assert_eq!(
        place(f, "second"),
        PlaceInfo {
            borrowed: true,
            mutable: true,
            origin: Some(Origin::Local(tasks)),
        }
    );
    assert_eq!(
        place(f, "from_param").origin,
        Some(Origin::Param(LocalId(0)))
    );
    assert!(!place(f, "n").borrowed);
}

#[test]
fn an_element_is_changed_only_through_a_mutable_vec() {
    let (d, _) = one_error(&with_task(
        "fn f(v: Vec<Task>) {\n    v[0].complete();\n}\n",
    ));
    assert_eq!(d.code, codes::V0303, "{d:#?}");
    let (d, _) = one_error("fn main() {\n    let v = vec![1];\n    v[0] = 2;\n}\n");
    assert_eq!(d.code, codes::V0301, "{d:#?}");
}

#[test]
fn an_element_is_an_owned_slot_when_assigned() {
    let (d, sources) = one_error(&with_task(
        "fn f(t: Task) {\n    let mut v = vec![mk()];\n    v[0] = t;\n}\n",
    ));
    assert_v0304(&d, part_of(&sources, "= t;", "t"), "`t`");
    let program = ok(&with_task(
        "fn f() {\n    let s = \"a\";\n    let mut v = vec![\"x\"];\n    v[0] = s;\n}\n",
    ));
    assert_eq!(repr(function(&program, "f"), "s"), Some(StringRepr::Owned));
}

#[test]
fn push_takes_an_owned_slot() {
    let (d, sources) = one_error(&with_task(
        "fn f(t: Task) {\n    let mut v = vec![mk()];\n    v.push(t);\n}\n",
    ));
    assert_v0304(&d, part_of(&sources, "push(t)", "t"), "`t`");

    let (d, _) = one_error(&with_task(
        "fn f() {\n    let t = mk();\n    let mut v = vec![mk()];\n    v.push(t);\n    show(t);\n}\n",
    ));
    assert_eq!(d.code, codes::V0305, "{d:#?}");

    let program = ok(&with_task(
        "fn f() {\n    let s = \"a\";\n    let mut v: Vec<string> = Vec::new();\n    v.push(s);\n    v.push(\"b\");\n}\n",
    ));
    assert_eq!(repr(function(&program, "f"), "s"), Some(StringRepr::Owned));
}

#[test]
fn a_clone_is_new_text() {
    let program = ok(&with_task(
        "fn f(s: string) {\n    let t = s.clone();\n    let u = \"lit\".clone();\n}\n",
    ));
    let f = function(&program, "f");
    assert_eq!(repr(f, "t"), Some(StringRepr::Owned));
    assert_eq!(repr(f, "u"), Some(StringRepr::Owned));
}

#[test]
fn changing_a_vec_while_an_element_alias_is_used_later_is_v0307() {
    for change in ["tasks[0].complete();", "tasks.push(mk());"] {
        let (d, sources) = one_error(&with_task(&format!(
            "fn f() {{\n    let mut tasks = vec![mk(), mk()];\n    let first = tasks[1];\n    {change}\n    read(first.label());\n}}\n"
        )));
        assert_eq!(d.code, codes::V0307, "{change}: {d:#?}");
        assert!(d.message.contains("`tasks`"), "{d:#?}");
        assert_eq!(d.labels[0].span, part_of(&sources, "read(first", "first"));
    }
}

#[test]
fn using_the_receiver_inside_a_changing_call_is_v0306() {
    let (d, sources) =
        one_error("fn main() {\n    let mut v: Vec<usize> = vec![];\n    v.push(v.len());\n}\n");
    assert_eq!(d.code, codes::V0306, "{d:#?}");
    assert_eq!(d.span, part_of(&sources, "(v.len", "v"));
    assert!(d.message.contains("`v`"), "{d:#?}");
    assert!(d.message.contains("`push`"), "{d:#?}");
    assert!(d.message.contains("`let`"), "{d:#?}");
}

#[test]
fn a_v0304_on_a_string_value_carries_the_clone_fix_it() {
    let (d, sources) = one_error(&with_p("fn f(user: P) -> string {\n    user.name\n}\n"));
    assert_eq!(d.code, codes::V0304, "{d:#?}");
    let end = span_of(&sources, "user.name").end;
    let fix = d.fix_it.as_ref().expect("a `.clone()` fix-it");
    assert_eq!(fix.span, Span::new(FileId(0), end, end));
    assert_eq!(fix.replacement, ".clone()");

    // Not for a struct value, which has no `clone`.
    let (d, _) = one_error(&with_p("fn f(user: P) -> P {\n    user\n}\n"));
    assert_eq!(d.code, codes::V0304, "{d:#?}");
    assert!(d.fix_it.is_none(), "{d:#?}");
}

#[test]
fn an_index_using_its_vec_in_a_changing_place_is_v0306() {
    for (stmts, at) in [
        (
            "let mut ln = vec![1, 2];\n    ln[ln.len() - 1] = 2;",
            "ln.len",
        ),
        (
            "let mut ws = vec![\"a\"];\n    ws[ws.len() - 1] = \"b\";",
            "ws.len",
        ),
        (
            "let mut tasks = vec![mk()];\n    tasks[tasks.len() - 1].complete();",
            "tasks.len",
        ),
        (
            "let mut tasks = vec![mk()];\n    change(tasks[tasks.len() - 1]);",
            "tasks.len",
        ),
        (
            "let mut tasks = vec![mk()];\n    let mut t = tasks[tasks.len() - 1];\n    t.complete();",
            "tasks.len",
        ),
    ] {
        let (d, sources) = one_error(&with_task(&format!(
            "fn change(mut t: Task) {{}}\nfn f() {{\n    {stmts}\n}}\n"
        )));
        assert_eq!(d.code, codes::V0306, "{stmts}: {d:#?}");
        let root = at.split('.').next().expect("root");
        assert_eq!(d.span, part_of(&sources, at, root), "{stmts}");
        assert!(d.message.contains("index"), "{d:#?}");
        assert!(d.message.contains("`let`"), "{d:#?}");
    }
    // Reading such an element, or storing the index first, is fine.
    ok(&with_task(
        "fn f() {\n    let mut tasks = vec![mk()];\n    let n = tasks[tasks.len() - 1].label();\n    let i = tasks.len() - 1;\n    tasks[i].complete();\n}\n",
    ));
}

#[test]
fn a_string_element_of_a_temporary_vec_bound_by_let_is_a_reference_to_a_string() {
    let program = ok(&with_task(
        "fn mk_vs() -> Vec<string> {\n    vec![\"a\"]\n}\nfn f() {\n    let s = mk_vs()[0];\n    read(s);\n}\n",
    ));
    assert_eq!(
        repr(function(&program, "f"), "s"),
        Some(StringRepr::RefOwned)
    );
}

// --- `match`: bindings borrow from a place, own from a temporary (3.2) ---

const MATCHING: &str = "enum Status {\n    Open,\n    Done,\n    Named(string),\n    Count(i32),\n    Held(Task),\n}\nstruct Board {\n    status: Status,\n}\nimpl Board {\n    fn reopen(mut self) {\n        match self.status {\n            Status::Done => {\n                self.status = Status::Open;\n            }\n            Status::Count(n) => {\n                self.status = Status::Open;\n                println!(\"{}\", n);\n            }\n            _ => {}\n        }\n    }\n}\nfn mk_o() -> Option<Task> {\n    Some(mk())\n}\nfn mk_os() -> Option<string> {\n    None\n}\nfn change_i(mut n: i32) {}\nfn change_t(mut t: Task) {}\n";

fn with_matching(body: &str) -> String {
    format!("{TASK}{MATCHING}{body}{MAIN}")
}

/// The id of the last local called `name` in `function`.
fn local_id(function: &HirFunction, name: &str) -> LocalId {
    let index = function
        .locals
        .iter()
        .rposition(|local| local.name == name)
        .unwrap_or_else(|| panic!("no local {name}"));
    LocalId(index as u32)
}

#[test]
fn bindings_on_a_place_are_read_only_aliases_and_on_a_temporary_owned() {
    let program = ok(&with_matching(
        "fn f(b: Board, o: Option<Task>) {\n    let lo = mk_o();\n    match lo {\n        Some(a) => show(a),\n        None => {}\n    }\n    match o {\n        Some(p) => show(p),\n        None => {}\n    }\n    match b.status {\n        Status::Held(h) => show(h),\n        Status::Count(n) => println!(\"{}\", n),\n        _ => {}\n    }\n    match mk_o() {\n        Some(t) => show(t),\n        None => {}\n    }\n}\n",
    ));
    let f = function(&program, "f");
    let alias = |origin| PlaceInfo {
        borrowed: true,
        mutable: false,
        origin: Some(origin),
    };
    assert_eq!(place(f, "a"), alias(Origin::Local(local_id(f, "lo"))));
    assert_eq!(place(f, "p"), alias(Origin::Param(local_id(f, "o"))));
    assert!(
        matches!(place(f, "h").origin, Some(Origin::Struct(_))),
        "{:?}",
        place(f, "h")
    );
    assert!(place(f, "h").borrowed && !place(f, "h").mutable);
    for owned in ["n", "t"] {
        assert_eq!(place(f, owned), PlaceInfo::default(), "{owned}");
    }
}

#[test]
fn a_string_binding_refers_to_the_string_in_a_place_and_owns_one_from_a_temporary() {
    let program = ok(&with_matching(
        "fn f(o: Option<string>, s: Status) {\n    match o {\n        Some(a) => read(a),\n        None => {}\n    }\n    match s {\n        Status::Named(b) => read(b),\n        _ => {}\n    }\n    match mk_os() {\n        Some(c) => read(c),\n        None => {}\n    }\n}\n",
    ));
    let f = function(&program, "f");
    assert_eq!(repr(f, "a"), Some(StringRepr::RefOwned));
    assert_eq!(repr(f, "b"), Some(StringRepr::RefOwned));
    assert_eq!(repr(f, "c"), Some(StringRepr::Owned));
}

#[test]
fn a_binding_of_a_matched_owned_local_pushed_into_a_vec_is_v0304_saying_to_match_on_the_call() {
    // `found` holds a call result in a `let`, so matching on `found` and
    // pushing its binding is V0304; matching on the call directly is fine.
    let (d, sources) = one_error(&with_matching(
        "fn f() {\n    let mut tasks: Vec<Task> = Vec::new();\n    let found = mk_o();\n    match found {\n        Some(task) => tasks.push(task),\n        None => {}\n    }\n}\n",
    ));
    assert_v0304(&d, part_of(&sources, "push(task)", "task"), "`found`");
    assert!(
        has_note(&d, "`match` on the call that made `found`"),
        "{d:#?}"
    );
    ok(&with_matching(
        "fn f() {\n    let mut tasks: Vec<Task> = Vec::new();\n    match mk_o() {\n        Some(task) => tasks.push(task),\n        None => {}\n    }\n}\n",
    ));
}

#[test]
fn a_binding_used_after_its_root_is_assigned_in_the_arm_is_v0307() {
    let (d, sources) = one_error(&with_matching(
        "fn f() {\n    let mut lo = mk_o();\n    match lo {\n        Some(t) => {\n            lo = None;\n            show(t);\n        }\n        None => {}\n    }\n}\n",
    ));
    assert_v0307(
        &d,
        part_of(&sources, "lo = None", "lo"),
        part_of(&sources, "show(t)", "t"),
        "lo",
        "t",
    );
}

#[test]
fn an_arm_may_change_the_root_when_no_alias_from_the_pattern_is_used_after() {
    // `Board::reopen` in the prelude: a unit pattern, and a Copy binding.
    ok(&with_matching(
        "fn f() {\n    let mut s = Status::Count(1);\n    match s {\n        Status::Count(n) => {\n            s = Status::Open;\n            println!(\"{}\", n);\n        }\n        Status::Held(t) => {\n            show(t);\n            s = Status::Done;\n        }\n        _ => {}\n    }\n}\n",
    ));
}

#[test]
fn a_match_read_as_a_value_follows_the_if_rule() {
    let program = ok(&with_matching(
        "fn f(o: Option<Task>, d: Task) {\n    let x = match o {\n        Some(t) => t,\n        None => d,\n    };\n    show(x);\n    let y = match mk_o() {\n        Some(t) => t,\n        None => mk(),\n    };\n    show(y);\n}\n",
    ));
    let f = function(&program, "f");
    assert!(place(f, "x").borrowed, "{:?}", place(f, "x"));
    assert!(!place(f, "y").borrowed, "{:?}", place(f, "y"));

    let (d, _) = one_error(&with_matching(
        "fn f(o: Option<string>) {\n    let x = match o {\n        Some(s) => s,\n        None => format!(\"none\"),\n    };\n    read(x);\n}\n",
    ));
    assert_eq!(d.code, codes::V0304, "{d:#?}");
    let (d, sources) = one_error(&with_matching(
        "fn f(o: Option<Task>) {\n    show(match o {\n        Some(t) => t,\n        None => mk(),\n    });\n}\n",
    ));
    assert_eq!(d.code, codes::V0304, "{d:#?}");
    assert_eq!(d.span.start, span_of(&sources, "match o").start, "{d:#?}");
}

#[test]
fn changing_a_binding_is_v0301_or_v0303_worded_for_what_it_is() {
    // An alias: change the original, no `mut` to add.
    for (body, code) in [
        (
            "Some(t) => {\n            t.done = true;\n        }",
            codes::V0301,
        ),
        ("Some(t) => change_t(t),", codes::V0303),
    ] {
        let (d, _) = one_error(&with_matching(&format!(
            "fn f() {{\n    let mut lo = mk_o();\n    match lo {{\n        {body}\n        None => {{}}\n    }}\n}}\n"
        )));
        assert_eq!(d.code, code, "{body}: {d:#?}");
        assert!(d.message.contains("change `lo` itself"), "{d:#?}");
        assert!(d.fix_it.is_none(), "{d:#?}");
    }
    // A copy or an owned value: `let mut n = n;` first.
    for (head, body, code, name) in [
        (
            "s",
            "Status::Count(n) => {\n            n = 2;\n        }\n        _ => {}",
            codes::V0301,
            "n",
        ),
        (
            "mk_o()",
            "Some(t) => {\n            change_t(t);\n        }\n        None => {}",
            codes::V0303,
            "t",
        ),
    ] {
        let (d, sources) = one_error(&with_matching(&format!(
            "fn f(s: Status) {{\n    match {head} {{\n        {body}\n    }}\n}}\n"
        )));
        assert_eq!(d.code, code, "{body}: {d:#?}");
        let fix = format!("let mut {name} = {name};");
        assert!(d.message.contains(&fix), "{d:#?}");
        // The arm in `f`, after the prelude's.
        let arm = format!("({name}) => {{\n");
        let brace = (sources[0].text.rfind(&arm).expect("the arm") + arm.len() - 1) as u32;
        let fix_it = d.fix_it.as_ref().expect("fix-it");
        assert_eq!(fix_it.span, Span::new(FileId(0), brace, brace), "{d:#?}");
        assert_eq!(fix_it.replacement, format!(" {fix}"));
    }
}

#[test]
fn a_temporary_matched_on_gives_its_bindings_away_and_a_moved_head_cannot_be_matched() {
    let (d, _) = one_error(&with_matching(
        "fn f() {\n    let mut tasks: Vec<Task> = Vec::new();\n    match mk_o() {\n        Some(t) => {\n            tasks.push(t);\n            show(t);\n        }\n        None => {}\n    }\n}\n",
    ));
    assert_eq!(d.code, codes::V0305, "{d:#?}");
    let (d, _) = one_error(&with_matching(
        "fn f() {\n    let lo = mk_o();\n    let other = lo;\n    match lo {\n        _ => {}\n    }\n}\n",
    ));
    assert_eq!(d.code, codes::V0305, "{d:#?}");
}

// --- `for`: the variable borrows from a place, owns from a temporary (3.1, 3.2)

const LOOPS: &str = "fn change_t(mut t: Task) {}\nfn change_i(mut n: i32) {}\nfn all() -> Vec<Task> {\n    vec![mk()]\n}\nfn words() -> Vec<string> {\n    vec![\"a\"]\n}\n";

fn with_loops(body: &str) -> String {
    format!("{TASK}{LOOPS}{body}{MAIN}")
}

/// Asserts `d` is the V0307 of a `for` holding `root` for the whole loop:
/// at `changed`, with a label at `head`, where the loop starts.
fn assert_held(d: &Diagnostic, changed: Span, head: Span, root: &str) {
    assert_eq!(d.code, codes::V0307, "{d:#?}");
    assert_eq!(d.span, changed, "{d:#?}");
    assert!(d.message.contains(&format!("`{root}`")), "{d:#?}");
    assert!(d.message.contains("loop"), "{d:#?}");
    assert!(
        d.labels.iter().any(|l| l.span == head),
        "a label at the head: {d:#?}"
    );
    assert!(has_note(d, "in Rust terms"), "{d:#?}");
}

#[test]
fn changing_the_vec_looped_over_is_v0307_even_when_the_variable_is_unused() {
    // The loop holds `v` for the whole loop even though `x` is never read,
    // so changing `v` inside the body is still rejected.
    let (d, sources) = one_error(&with_loops(
        "fn f() {\n    let mut v = vec![1, 2];\n    for x in v {\n        v.push(1);\n    }\n}\n",
    ));
    assert_held(
        &d,
        part_of(&sources, "v.push(1)", "v"),
        part_of(&sources, "in v {", "v"),
        "v",
    );
    // A non-Copy element, a change before `break`, a field of a parameter,
    // and giving the `Vec` away are all rejected too.
    for body in [
        "    let mut tasks = vec![mk()];\n    for t in tasks {\n        tasks.push(mk());\n    }\n",
        "    let mut tasks = vec![mk()];\n    for t in tasks {\n        tasks = all();\n        break;\n    }\n",
        "    let v = vec![1];\n    for x in v {\n        let w = v;\n        break;\n    }\n",
        "    let mut tasks = vec![mk()];\n    for t in tasks {\n        tasks[0].complete();\n        show(t);\n    }\n",
    ] {
        let (d, _) = one_error(&with_loops(&format!("fn f() {{\n{body}}}\n")));
        assert_eq!(d.code, codes::V0307, "{body}: {d:#?}");
    }
}

#[test]
fn an_alias_made_through_a_mutable_alias_keeps_its_root_from_being_used() {
    // A `let` of a part, a `match` binding, and a loop over the mutable
    // alias all reborrow it, so its root cannot be read while they are used.
    let (d, sources) = one_error(&with_loops(
        "fn f(mut ts: Vec<Task>) {\n    let mut a = ts[0];\n    let s = a.label;\n    println!(\"{} {}\", ts.len(), s);\n}\n",
    ));
    assert_eq!(d.code, codes::V0307, "{d:#?}");
    assert_eq!(d.span, part_of(&sources, "ts.len()", "ts"));
    assert!(
        d.message.contains("part of `a`, which can change `ts`"),
        "{d:#?}"
    );
    let (d, _) = one_error(&with_loops(
        "fn f(mut v: Vec<Option<string>>) {\n    let mut a = v[0];\n    match a {\n        Some(s) => println!(\"{} {}\", v.len(), s),\n        None => {}\n    }\n}\n",
    ));
    assert_eq!(d.code, codes::V0307, "{d:#?}");
    let (d, sources) = one_error(&with_loops(
        "fn f(mut rows: Vec<Vec<i32>>) {\n    let mut row = rows[0];\n    for t in row {\n        println!(\"{}\", rows.len());\n    }\n}\n",
    ));
    assert_held(
        &d,
        part_of(&sources, "rows.len()", "rows"),
        part_of(&sources, "in row {", "row"),
        "rows",
    );
    assert!(
        d.message.contains(
            "this loop goes over `row`, which can change `rows`, so `rows` cannot be used inside it"
        ),
        "{d:#?}"
    );
    // Reading the mutable alias itself while its reborrow is used is fine.
    ok(&with_loops(
        "fn f(mut ts: Vec<Task>) {\n    let mut a = ts[0];\n    let s = a.label;\n    println!(\"{} {}\", a.done, s);\n}\n",
    ));
}

#[test]
fn a_mutable_alias_made_through_a_mutable_alias_leaves_it_usable_once_done() {
    // `s` reborrows `a`, so `a` may change once `s` is no longer used.
    ok(&with_aliases(
        "fn f(mut ps: Vec<P>) {\n    let mut a = ps[0];\n    let mut s = a.s;\n    s = \"x\";\n    change_p(a);\n}\n",
    ));
    let (d, sources) = one_error(&with_aliases(
        "fn f(mut ps: Vec<P>) {\n    let mut a = ps[0];\n    let mut s = a.s;\n    change_p(a);\n    s = \"x\";\n}\n",
    ));
    assert_eq!(d.code, codes::V0307, "{d:#?}");
    let call = span_of(&sources, "change_p(a)");
    assert_eq!(
        d.span,
        Span::new(call.file, call.start + 9, call.start + 10),
        "{d:#?}"
    );
}

#[test]
fn the_vec_looped_over_may_change_after_the_loop_or_when_it_is_a_temporary() {
    ok(&with_loops(
        "fn f() {\n    let mut v = vec![1, 2];\n    for x in v {\n        println!(\"{}\", x);\n    }\n    v.push(3);\n    let mut w = vec![1];\n    for y in all() {\n        w.push(1);\n    }\n    for i in 0..v.len() {\n        v[i] = 0;\n    }\n}\n",
    ));
}

#[test]
fn a_for_variable_is_an_alias_a_copy_or_an_owned_value() {
    let program = ok(&with_loops(
        "fn f(tasks: Vec<Task>) {\n    let mine = all();\n    for t in tasks {\n        show(t);\n    }\n    for m in mine {\n        show(m);\n    }\n    for o in all() {\n        show(o);\n    }\n    let ns = vec![1];\n    for n in ns {\n        println!(\"{}\", n);\n    }\n    for i in 0..3 {\n        println!(\"{}\", i);\n    }\n}\n",
    ));
    let f = function(&program, "f");
    let alias = |origin| PlaceInfo {
        borrowed: true,
        mutable: false,
        origin: Some(origin),
    };
    assert_eq!(place(f, "t"), alias(Origin::Param(local_id(f, "tasks"))));
    assert_eq!(place(f, "m"), alias(Origin::Local(local_id(f, "mine"))));
    for owned in ["o", "n", "i"] {
        assert_eq!(place(f, owned), PlaceInfo::default(), "{owned}");
    }
}

#[test]
fn a_string_for_variable_refers_into_a_place_and_owns_from_a_temporary() {
    let program = ok(&with_loops(
        "fn f(names: Vec<string>) {\n    for a in names {\n        read(a);\n    }\n    for b in words() {\n        read(b);\n    }\n}\n",
    ));
    let f = function(&program, "f");
    assert_eq!(repr(f, "a"), Some(StringRepr::RefOwned));
    assert_eq!(repr(f, "b"), Some(StringRepr::Owned));
}

#[test]
fn reading_through_a_for_variable_passes() {
    ok(&with_loops(
        "fn f(tasks: Vec<Task>, values: Vec<i32>) -> i32 {\n    for t in tasks {\n        t.label();\n    }\n    let mut total = 0;\n    for value in values {\n        total = total + value;\n    }\n    total\n}\n",
    ));
}

#[test]
fn a_for_variable_over_a_place_cannot_be_kept_and_one_over_a_temporary_can() {
    let (d, sources) = one_error(&with_loops(
        "fn f(tasks: Vec<Task>) {\n    let mut out: Vec<Task> = Vec::new();\n    for t in tasks {\n        out.push(t);\n    }\n}\n",
    ));
    assert_v0304(&d, part_of(&sources, "push(t)", "t"), "`tasks`");
    ok(&with_loops(
        "fn f() {\n    let mut out: Vec<Task> = Vec::new();\n    for t in all() {\n        out.push(t);\n    }\n}\n",
    ));
}

#[test]
fn changing_a_for_variable_is_v0301_or_v0303_worded_for_what_it_is() {
    // An alias: change the original, no `mut` to add.
    for (stmt, code) in [
        ("t.label = \"x\";", codes::V0301),
        ("change_t(t);", codes::V0303),
    ] {
        let (d, _) = one_error(&with_loops(&format!(
            "fn f(mut tasks: Vec<Task>) {{\n    for t in tasks {{\n        {stmt}\n    }}\n}}\n"
        )));
        assert_eq!(d.code, code, "{stmt}: {d:#?}");
        assert!(d.message.contains("change `tasks` itself"), "{d:#?}");
        assert!(d.message.contains("`for`"), "{d:#?}");
        assert!(d.fix_it.is_none(), "{d:#?}");
    }
    // A copy or an owned value: `let mut n = n;` first.
    for (head, stmt, code, name) in [
        ("n in ns", "n = 2;", codes::V0301, "n"),
        ("i in 0..3", "change_i(i);", codes::V0303, "i"),
        ("t in all()", "t.complete();", codes::V0303, "t"),
    ] {
        let (d, sources) = one_error(&with_loops(&format!(
            "fn f() {{\n    let ns = vec![1];\n    for {head} {{\n        {stmt}\n    }}\n}}\n"
        )));
        assert_eq!(d.code, code, "{stmt}: {d:#?}");
        let fix = format!("let mut {name} = {name};");
        assert!(d.message.contains(&fix), "{d:#?}");
        let brace = span_of(&sources, &format!("{head} {{")).end - 1;
        let fix_it = d.fix_it.as_ref().expect("fix-it");
        assert_eq!(
            fix_it.span,
            Span::new(FileId(0), brace + 1, brace + 1),
            "{d:#?}"
        );
        assert_eq!(fix_it.replacement, format!(" {fix}"));
    }
}

#[test]
fn a_for_variable_passed_to_a_mut_parameter_suggests_looping_by_index() {
    let (d, _) = one_error(&with_loops(
        "fn f(mut tasks: Vec<Task>) {\n    for t in tasks {\n        change_t(t);\n    }\n}\n",
    ));
    assert_eq!(d.code, codes::V0303, "{d:#?}");
    assert!(
        d.message.ends_with(
            "change `tasks` itself instead, for example by looping with `for i in \
             0..tasks.len()` and using `tasks[i]`"
        ),
        "{d:#?}"
    );
}

#[test]
fn a_for_variable_assigned_to_suggests_looping_by_index() {
    let (d, _) = one_error(&with_loops(
        "fn f(mut tasks: Vec<Task>) {\n    for t in tasks {\n        t.label = \"x\";\n    }\n}\n",
    ));
    assert_eq!(d.code, codes::V0301, "{d:#?}");
    assert!(
        d.message.ends_with(
            "change `tasks` itself instead, for example by looping with `for i in \
             0..tasks.len()` and using `tasks[i]`"
        ),
        "{d:#?}"
    );
}

#[test]
fn a_for_body_follows_itself_for_moves_and_aliases() {
    // Given away in one round, used in the next.
    let (d, _) = one_error(&with_loops(
        "fn f() {\n    let t = mk();\n    let mut out: Vec<Task> = Vec::new();\n    for i in 0..3 {\n        out.push(t);\n    }\n}\n",
    ));
    assert_eq!(d.code, codes::V0305, "{d:#?}");
    assert!(has_note(&d, "previous time through the loop"), "{d:#?}");
    // An alias used in one round after its root changed in the last.
    let (d, sources) = one_error(&with_loops(
        "fn f() {\n    let mut tasks = vec![mk()];\n    let first = tasks[0];\n    for i in 0..3 {\n        show(first);\n        tasks.push(mk());\n    }\n}\n",
    ));
    assert_eq!(d.code, codes::V0307, "{d:#?}");
    assert_eq!(d.labels[0].span, part_of(&sources, "show(first)", "first"));
    // An owned variable given away, then used in the same round.
    let (d, _) = one_error(&with_loops(
        "fn f() {\n    let mut out: Vec<Task> = Vec::new();\n    for t in all() {\n        out.push(t);\n        show(t);\n    }\n}\n",
    ));
    assert_eq!(d.code, codes::V0305, "{d:#?}");
    // A `Vec` given away before the loop.
    let (d, _) = one_error(&with_loops(
        "fn f() {\n    let v = vec![1];\n    let w = v;\n    for x in v {}\n}\n",
    ));
    assert_eq!(d.code, codes::V0305, "{d:#?}");
}

// --- `?`: the operand is an owned slot (spec 3.4) -----------------------------

/// `body` after a `parse` returning `Result<i32, string>` and a struct `H`
/// holding one, with an empty `main`.
fn with_parse(body: &str) -> String {
    format!(
        "struct H {{\n    r: Result<i32, string>,\n}}\n\nfn parse() -> Result<i32, string> {{\n    Ok(1)\n}}\n\n{body}\nfn main() {{}}\n"
    )
}

#[test]
fn question_moves_an_owned_local_so_a_second_one_is_v0305() {
    let (d, sources) = one_error(&with_parse(
        "fn run() -> Result<i32, string> {\n    let r = parse();\n    let v = r?;\n    let w = r?;\n    Ok(v + w)\n}\n",
    ));
    assert_eq!(d.code, codes::V0305, "{d:#?}");
    assert_eq!(d.span, part_of(&sources, "let w = r?", "r"));

    // In a loop, the second round sees the first round's move.
    let (d, _) = one_error(&with_parse(
        "fn run() -> Result<i32, string> {\n    let r = parse();\n    let mut total = 0;\n    for i in 0..3 {\n        total = total + r?;\n    }\n    Ok(total)\n}\n",
    ));
    assert_eq!(d.code, codes::V0305, "{d:#?}");
}

#[test]
fn question_on_a_borrowed_place_is_v0304() {
    let (d, sources) = one_error(&with_parse(
        "fn run(r: Result<i32, string>) -> Result<i32, string> {\n    let v = r?;\n    Ok(v)\n}\n",
    ));
    assert_v0304(&d, part_of(&sources, "r?", "r"), "`r`");
    assert!(d.message.contains("`?`"), "{d:#?}");

    let (d, sources) = one_error(&with_parse(
        "fn run(h: H) -> Result<i32, string> {\n    let v = h.r?;\n    Ok(v)\n}\n",
    ));
    assert_v0304(&d, span_of(&sources, "h.r"), "`H`");

    let (d, sources) = one_error(&with_parse(
        "fn run() -> Result<i32, string> {\n    let rs = vec![parse()];\n    let mut total = 0;\n    for r in rs {\n        total = total + r?;\n    }\n    Ok(total)\n}\n",
    ));
    assert_eq!(d.code, codes::V0304, "{d:#?}");
    assert_eq!(d.span, part_of(&sources, "r?", "r"));
}

#[test]
fn question_on_a_temporary_or_an_owned_binding_is_accepted() {
    ok(&with_parse(
        "fn run() -> Result<i32, string> {\n    parse()?;\n    let r = parse();\n    let mut total = r?;\n    for r in vec![parse(), parse()] {\n        total = total + r?;\n    }\n    let n = match parse() {\n        Ok(n) => n,\n        Err(e) => parse()?,\n    };\n    Ok(total + n)\n}\n",
    ));
}

#[test]
fn a_string_from_question_is_owned() {
    let program = ok(
        "fn name() -> Result<string, string> {\n    Ok(\"a\")\n}\n\nfn f() -> Result<string, string> {\n    let s = name()?;\n    let t = if true { name()? } else { \"b\" };\n    Ok(format!(\"{}{}\", s, t))\n}\n\nfn main() {}\n",
    );
    let f = function(&program, "f");
    assert_eq!(repr(f, "s"), Some(StringRepr::Owned));
    assert_eq!(repr(f, "t"), Some(StringRepr::Owned));
}

#[test]
fn an_element_of_a_gone_vec_read_in_place_is_v0304() {
    const VECS: &str = "fn mkv() -> Vec<string> {\n    vec![\"v\"]\n}\nfn mkps() -> Vec<P> {\n    vec![mkp()]\n}\nfn ops() -> Option<Vec<P>> {\n    None\n}\n";
    for (stmt, leaf) in [
        ("read(if c { mkv()[0] } else { \"x\" });", "mkv()[0]"),
        ("read({ let v = mkv(); v[0] });", "v[0]"),
        (
            "println!(\"{}\", if c { let v = mkv(); v[0] } else { \"z\" });",
            "v[0]",
        ),
        (
            "show(match ops() { Some(list) => list[0], None => mkp() });",
            "list[0]",
        ),
        ("if c { mkps()[0] } else { mkp() };", "mkps()[0]"),
        (
            "println!(\"{}\", (if c { mkps()[0] } else { mkp() }).name);",
            "mkps()[0]",
        ),
        (
            "read(match ops() { Some(list) => list[0].name, None => \"n\" });",
            "list[0].name",
        ),
    ] {
        let (d, sources) = one_error(&with_blocks(&format!(
            "{VECS}fn f(c: bool) {{\n    {stmt}\n}}\n"
        )));
        assert_eq!(d.code, codes::V0304, "{stmt}: {d:#?}");
        assert_eq!(d.span, part_of(&sources, stmt, leaf), "{stmt}: {d:#?}");
        assert!(
            has_note(&d, "cannot outlive the value it borrows"),
            "{stmt}: {d:#?}"
        );
    }
    // A Copy element is copied out, and a `let` keeps the temporary alive.
    ok(&with_blocks(&format!(
        "{VECS}fn f(c: bool) {{\n    println!(\"{{}}\", if c {{ mkps()[0].x }} else {{ 1 }});\n    let z = if c {{ mkps()[0] }} else {{ mkp() }};\n    show(z);\n}}\n"
    )));
}

#[test]
fn v0304_names_the_matched_value_the_arm_and_the_gone_vec() {
    const VECS: &str = "fn mkv() -> Vec<string> {\n    vec![\"v\"]\n}\nfn ovs() -> Option<Vec<string>> {\n    None\n}\n";
    let error = |body: &str| one_error(&with_blocks(&format!("{VECS}fn f() {{\n{body}\n}}\n"))).0;
    // (a) A binding is borrowed from the value matched on.
    let d = error(
        "    let o: Option<string> = None;\n    read(match o { Some(s) => s, None => mk() });",
    );
    assert!(d.message.contains("borrowed from `o`"), "{d:#?}");
    // (b) A branch that a `let` cannot keep either: make every branch new.
    for body in [
        "    let o: Option<P> = None;\n    show(match o { Some(p) => p, None => { let a = mkp(); a } });",
        "    read(match ovs() { Some(list) => list[0], None => mk() });",
    ] {
        let d = error(body);
        assert!(!d.message.contains("store it with `let`"), "{d:#?}");
        assert!(
            has_note(&d, "every branch give a new value instead"),
            "{d:#?}"
        );
    }
    // (c) A match binding lives in its arm; a temporary, until the line ends.
    let d =
        error("    let s = match ovs() { Some(list) => list[0], None => \"none\" };\n    read(s);");
    assert!(
        d.message.starts_with("`list` only exists inside this arm"),
        "{d:#?}"
    );
    let d = error("    let mut t = \"a\";\n    t = mkv()[0];\n    read(t);");
    assert!(
        d.message
            .starts_with("the `Vec` this value is part of is gone after this line"),
        "{d:#?}"
    );
    // (d) An element of a temporary `Vec` put into an owned slot.
    let d = error("    let o = Some(mkv()[0]);");
    assert_eq!(
        d.message,
        "the `Vec` this value is part of is gone after this line, so it cannot be put inside `Some`"
    );
}

#[test]
fn a_binding_of_a_read_only_parameter_passed_to_a_mut_parameter_says_to_mark_it_mut() {
    let (d, sources) = one_error(&with_strs(
        "enum Msg {\n    Text(string),\n    Quit,\n}\nfn f(m: Msg) {\n    match m {\n        Msg::Text(t) => fill(t),\n        Msg::Quit => {}\n    }\n}\n",
    ));
    assert_eq!(d.code, codes::V0303, "{d:#?}");
    assert!(
        d.message
            .ends_with("mark `m` `mut`, then change `m` itself instead"),
        "{d:#?}"
    );
    assert_mut_fix_it(&d, &sources, "f(m: Msg)", "m:");
}
