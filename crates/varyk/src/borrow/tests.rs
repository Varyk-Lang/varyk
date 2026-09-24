use std::path::Path;

use varyk_syntax::{FileId, SourceFile, Span};

use super::analyze;
use crate::diagnostics::{Diagnostic, codes};
use crate::hir::{HirFunction, HirProgram, Origin, PlaceInfo, StringRepr};
use crate::resolve::resolve;
use crate::types::typecheck;

// --- Helpers ----------------------------------------------------------------

fn check_source(entry: SourceFile) -> (Result<HirProgram, Vec<Diagnostic>>, Vec<SourceFile>) {
    let mut sources = Vec::new();
    let result = resolve(entry, &mut sources)
        .and_then(typecheck)
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
    assert!(has_note(d, "milestone 2 will allow this"), "{d:#?}");
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
    assert_eq!(repr(f, "s"), Some(StringRepr::Borrowed));
    assert_eq!(repr(f, "t"), Some(StringRepr::Borrowed));
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
    assert_eq!(repr(f, "s"), Some(StringRepr::Borrowed));
    assert_eq!(repr(f, "t"), Some(StringRepr::Owned));
    assert_eq!(repr(f, "n"), None);
    assert_eq!(repr(f, "p"), None);
    assert_eq!(repr(f, "a"), Some(StringRepr::Borrowed));
    assert_eq!(repr(f, "b"), Some(StringRepr::Owned));
    assert_eq!(repr(f, "c"), Some(StringRepr::Owned));
    assert_eq!(repr(f, "q"), None);
}

#[test]
fn representation_of_the_examples() {
    let program = check_path("examples/interop/main.vr").0.unwrap();
    assert_eq!(
        repr(function(&program, "main"), "name"),
        Some(StringRepr::Borrowed)
    );
    let program = check_path("examples/borrowing.vr").0.unwrap();
    assert_eq!(repr(function(&program, "main"), "user"), None);
    assert_eq!(repr(function(&program, "rename"), "user"), None);
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
