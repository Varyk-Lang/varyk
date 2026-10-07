use std::path::Path;

use varyk_syntax::{FileId, SourceFile, Span};

use super::analyze;
use crate::diagnostics::{Diagnostic, codes};
use crate::hir::{
    HirExprKind, HirFunction, HirProgram, LocalId, LocalKind, Origin, PlaceInfo, StringRepr,
};
use crate::resolve::{StructId, resolve};
use crate::types::typecheck;

// --- Helpers ----------------------------------------------------------------

fn check_source(entry: SourceFile) -> (Result<HirProgram, Vec<Diagnostic>>, Vec<SourceFile>) {
    let mut sources = Vec::new();
    let result = resolve(entry, &mut sources)
        .and_then(|resolved| typecheck(resolved, &sources))
        .and_then(|hir| analyze(hir, &sources));
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

/// The name of the parameter the borrowed return of `function_name` is
/// rooted at (M4 spec 3.1).
fn ret_root(program: &HirProgram, function_name: &str) -> Option<String> {
    let f = function(program, function_name);
    f.ret_root
        .map(|root| f.locals[root.0 as usize].name.clone())
}

#[test]
fn returning_a_parameter_is_a_borrowed_return_rooted_at_it() {
    let program = ok(&with_p("fn f(user: P) -> P {\n    user\n}\n"));
    assert_eq!(ret_root(&program, "f").as_deref(), Some("user"));

    let program = ok(&with_p("fn f(user: P) -> P {\n    return user;\n}\n"));
    assert_eq!(ret_root(&program, "f").as_deref(), Some("user"));

    let program = ok("fn f(s: string) -> string {\n    s\n}\nfn main() {}\n");
    assert_eq!(ret_root(&program, "f").as_deref(), Some("s"));
}

#[test]
fn returning_a_field_is_a_borrowed_return_and_of_a_local_v0304() {
    let program = ok(&with_p("fn f(user: P) -> string {\n    user.name\n}\n"));
    assert_eq!(ret_root(&program, "f").as_deref(), Some("user"));

    let (d, sources) = one_error(&format!(
        "{P}fn f() -> string {{\n    let local = P {{ x: 1, name: \"a\" }};\n    local.name\n}}\n{MAIN}"
    ));
    assert_v0304(&d, span_of(&sources, "local.name"), "`local`");
    assert!(
        d.message.contains("ends when this function returns"),
        "{d:#?}"
    );
}

#[test]
fn a_binding_of_a_borrowed_place_keeps_its_origin() {
    let program = ok(&with_p(
        "fn f(user: P) -> string {\n    let n = user.name;\n    n\n}\n",
    ));
    assert_eq!(ret_root(&program, "f").as_deref(), Some("user"));
    assert_eq!(
        place(function(&program, "f"), "n").origin,
        Some(Origin::Struct(StructId(0)))
    );

    let program = ok(&with_p(
        "fn f(user: P) -> P {\n    let u = user;\n    u\n}\n",
    ));
    assert_eq!(ret_root(&program, "f").as_deref(), Some("user"));
    assert_eq!(
        place(function(&program, "f"), "u").origin,
        Some(Origin::Param(LocalId(0)))
    );
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

/// `Error::with_status`'s text is an owned slot as `Error::new`'s is
/// (milestone 5b4 spec 2.6): a borrowed `string` is V0304 with the clone
/// help, and a clone of it is accepted.
#[test]
fn error_with_status_takes_its_text_as_an_owned_slot() {
    let (d, sources) = one_error(
        "fn fail(text: string) -> Error {\n    Error::with_status(400, text)\n}\nfn main() {}\n",
    );
    assert_eq!(d.code, codes::V0304, "{d:#?}");
    assert_eq!(d.span, part_of(&sources, "400, text)", "text"), "{d:#?}");
    let fix = d.fix_it.as_ref().expect("a clone fix-it");
    assert!(fix.replacement.ends_with(".clone()"), "{fix:#?}");
    ok(
        "fn fail(text: string) -> Error {\n    Error::with_status(400, text.clone())\n}\nfn main() {}\n",
    );
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
    // Returned, the `let` is part of `p` (M4 spec 3.1): a `&str`.
    let program = ok(&with_strs(
        "fn f(p: P) -> string {\n    let mut s = \"x\";\n    s = p.name;\n    s\n}\n",
    ));
    assert_eq!(ret_root(&program, "f").as_deref(), Some("p"));
    assert_eq!(repr(function(&program, "f"), "s"), Some(StringRepr::Str));
    // Given new text too, it must own its text.
    let (d, sources) = one_error(&with_strs(
        "fn f(p: P) -> string {\n    let mut s = \"x\";\n    s = p.name;\n    s = mk();\n    s\n}\n",
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
    // Returned, `s` is part of `p`; the `let` itself is the one mistake.
    assert_v0304(&d, span_of(&sources, "p.name"), "`P`");
    assert!(d.message.contains("cannot be kept in `s`"), "{d:#?}");
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

/// A binding of an arm whose body is not a block, passed to a `mut`
/// parameter, gets a fix-it wrapping the body in a block that copies it
/// first, and the fixed program passes.
#[test]
fn a_match_binding_passed_to_mut_in_an_expression_arm_gets_a_block_fix_it() {
    let text = "fn make() -> Option<string> {\n    Some(\"a\")\n}\n\nfn change(mut s: string) {\n    s = \"b\";\n}\n\nfn main() {\n    match make() {\n        Some(s) => change(s),\n        None => {}\n    }\n}\n";
    let (d, _) = one_error(text);
    assert_eq!(d.code, codes::V0303, "{d:#?}");
    let fix_it = d.fix_it.expect("a fix-it");
    assert_eq!(fix_it.replacement, "{ let mut s = s; change(s) }");
    let mut fixed = text.to_string();
    fixed.replace_range(
        fix_it.span.start as usize..fix_it.span.end as usize,
        &fix_it.replacement,
    );
    ok(&fixed);
}

/// Two bindings of one arm whose body is not a block, both passed to
/// `mut` parameters: one fix-it copies both, the other diagnostic has
/// none, so no two fix-its replace the same text, and the fixed program
/// passes.
#[test]
fn two_match_bindings_passed_to_mut_in_one_expression_arm_get_one_fix_it() {
    let text = "enum P {\n    Two(string, string),\n    Empty,\n}\n\nfn make() -> P {\n    P::Two(\"a\", \"b\")\n}\n\nfn both(mut a: string, mut b: string) {\n    a = \"c\";\n    b = \"d\";\n}\n\nfn main() {\n    match make() {\n        P::Two(a, b) => both(a, b),\n        P::Empty => {}\n    }\n}\n";
    let (diagnostics, _) = errors(text);
    assert_eq!(diagnostics.len(), 2, "{diagnostics:#?}");
    assert!(diagnostics.iter().all(|d| d.code == codes::V0303));
    let fix_it = diagnostics[0].fix_it.clone().expect("a fix-it");
    assert_eq!(
        fix_it.replacement,
        "{ let mut a = a; let mut b = b; both(a, b) }"
    );
    assert!(diagnostics[1].fix_it.is_none(), "{diagnostics:#?}");
    let mut fixed = text.to_string();
    fixed.replace_range(
        fix_it.span.start as usize..fix_it.span.end as usize,
        &fix_it.replacement,
    );
    ok(&fixed);
}

/// The note of the index V0306 says, in Rust terms, why the order matters.
#[test]
fn changing_a_vec_in_its_own_index_is_v0306_with_the_borrow_note() {
    let (d, _) = one_error(
        "fn bump(mut v: Vec<i32>) -> usize {\n    v.push(1);\n    0\n}\n\nfn main() {\n    let mut v: Vec<i32> = Vec::new();\n    v[bump(v)] = 2;\n}\n",
    );
    assert_eq!(d.code, codes::V0306, "{d:#?}");
    assert_eq!(
        d.notes,
        vec![
            "in Rust terms, `v` is borrowed mutably to get at the element before the index is \
             worked out"
                .to_string()
        ]
    );
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

/// Milestone 5b3 spec 2.2 and 3: a trailing value is read, never moved,
/// so a local, a field, or an element passed as one stays usable.
#[test]
fn a_trailing_value_is_read_and_its_root_stays_usable() {
    let (result, _) = check_path("crates/varyk/tests/fixtures/interop/trailing_values/main.vr");
    if let Err(diagnostics) = result {
        panic!("{diagnostics:#?}");
    }
}

/// A trailing value read after its local was given to an owned
/// parameter of the same call is V0306 and V0305; one read beside a
/// mutable borrow of its root is V0306, as for any argument.
#[test]
fn a_trailing_value_follows_the_argument_rules() {
    let (result, sources) =
        check_path("crates/varyk/tests/fixtures/interop/trailing_misuse/main.vr");
    let diagnostics = result.expect_err("should fail");
    let codes_found: Vec<&str> = diagnostics.iter().map(|d| d.code).collect();
    assert_eq!(
        codes_found,
        vec![codes::V0306, codes::V0305, codes::V0306],
        "{diagnostics:#?}"
    );
    let keep = span_of(&sources, "keep(s, s)");
    assert_eq!(diagnostics[0].span.start, keep.start + 8);
    assert_eq!(diagnostics[1].span.start, keep.start + 8);
    assert_eq!(
        diagnostics[2].span,
        part_of(&sources, "grow(list, list[0])", "list[0]")
    );
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

const TASK: &str = "struct Item {\n    label: string,\n    done: bool,\n}\nimpl Item {\n    fn new(label: string) -> Item {\n        Item { label: label.clone(), done: false }\n    }\n\n    fn complete(mut self) {\n        self.done = true;\n    }\n\n    fn label(self) -> string {\n        self.label.clone()\n    }\n}\nfn read(s: string) {}\nfn show(t: Item) {}\nfn mk() -> Item {\n    Item::new(\"m\")\n}\n";

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
    let (d, sources) = one_error(&with_task("fn f(t: Item) {\n    t.complete();\n}\n"));
    assert_eq!(d.code, codes::V0303, "{d:#?}");
    assert_mut_fix_it(&d, &sources, "fn f(t: Item)", "t: Item");
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
        "fn f(t: Item, v: Vec<Item>, mut w: Vec<Item>) {\n    read(t.label());\n    let n = v.len();\n    w.push(mk());\n    w[0].complete();\n    mk().complete();\n    let mut tasks = vec![mk()];\n    tasks[0].complete();\n    tasks.push(Item::new(\"b\"));\n    let last = tasks.pop();\n}\n",
    ));
}

#[test]
fn an_element_let_is_an_alias_of_its_vec_unless_copy() {
    let program = ok(&with_task(
        "fn f(v: Vec<Item>) {\n    let mut tasks = vec![mk()];\n    let first = tasks[0];\n    show(first);\n    let mut second = tasks[0];\n    second.complete();\n    let from_param = v[0];\n    let ns = vec![1, 2];\n    let n = ns[1];\n    show(from_param);\n}\n",
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
        "fn f(v: Vec<Item>) {\n    v[0].complete();\n}\n",
    ));
    assert_eq!(d.code, codes::V0303, "{d:#?}");
    let (d, _) = one_error("fn main() {\n    let v = vec![1];\n    v[0] = 2;\n}\n");
    assert_eq!(d.code, codes::V0301, "{d:#?}");
}

#[test]
fn an_element_is_an_owned_slot_when_assigned() {
    let (d, sources) = one_error(&with_task(
        "fn f(t: Item) {\n    let mut v = vec![mk()];\n    v[0] = t;\n}\n",
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
        "fn f(t: Item) {\n    let mut v = vec![mk()];\n    v.push(t);\n}\n",
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
    let (d, sources) = one_error(&with_p(
        "fn f(user: P) -> Vec<string> {\n    vec![user.name]\n}\n",
    ));
    assert_eq!(d.code, codes::V0304, "{d:#?}");
    let end = span_of(&sources, "user.name").end;
    let fix = d.fix_it.as_ref().expect("a `.clone()` fix-it");
    assert_eq!(fix.span, Span::new(FileId(0), end, end));
    assert_eq!(fix.replacement, ".clone()");

    // Not for a struct value, which has no `clone`.
    let (d, _) = one_error(&with_p("fn f(user: P) -> Vec<P> {\n    vec![user]\n}\n"));
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
            "fn change(mut t: Item) {{}}\nfn f() {{\n    {stmts}\n}}\n"
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

const MATCHING: &str = "enum Status {\n    Open,\n    Done,\n    Named(string),\n    Count(i32),\n    Held(Item),\n}\nstruct Board {\n    status: Status,\n}\nimpl Board {\n    fn reopen(mut self) {\n        match self.status {\n            Status::Done => {\n                self.status = Status::Open;\n            }\n            Status::Count(n) => {\n                self.status = Status::Open;\n                println!(\"{}\", n);\n            }\n            _ => {}\n        }\n    }\n}\nfn mk_o() -> Option<Item> {\n    Some(mk())\n}\nfn mk_os() -> Option<string> {\n    None\n}\nfn change_i(mut n: i32) {}\nfn change_t(mut t: Item) {}\n";

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
        "fn f(b: Board, o: Option<Item>) {\n    let lo = mk_o();\n    match lo {\n        Some(a) => show(a),\n        None => {}\n    }\n    match o {\n        Some(p) => show(p),\n        None => {}\n    }\n    match b.status {\n        Status::Held(h) => show(h),\n        Status::Count(n) => println!(\"{}\", n),\n        _ => {}\n    }\n    match mk_o() {\n        Some(t) => show(t),\n        None => {}\n    }\n}\n",
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
        "fn f() {\n    let mut tasks: Vec<Item> = Vec::new();\n    let found = mk_o();\n    match found {\n        Some(task) => tasks.push(task),\n        None => {}\n    }\n}\n",
    ));
    assert_v0304(&d, part_of(&sources, "push(task)", "task"), "`found`");
    assert!(
        has_note(&d, "`match` on the call that made `found`"),
        "{d:#?}"
    );
    ok(&with_matching(
        "fn f() {\n    let mut tasks: Vec<Item> = Vec::new();\n    match mk_o() {\n        Some(task) => tasks.push(task),\n        None => {}\n    }\n}\n",
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
        "fn f(o: Option<Item>, d: Item) {\n    let x = match o {\n        Some(t) => t,\n        None => d,\n    };\n    show(x);\n    let y = match mk_o() {\n        Some(t) => t,\n        None => mk(),\n    };\n    show(y);\n}\n",
    ));
    let f = function(&program, "f");
    assert!(place(f, "x").borrowed, "{:?}", place(f, "x"));
    assert!(!place(f, "y").borrowed, "{:?}", place(f, "y"));

    let (d, _) = one_error(&with_matching(
        "fn f(o: Option<string>) {\n    let x = match o {\n        Some(s) => s,\n        None => format!(\"none\"),\n    };\n    read(x);\n}\n",
    ));
    assert_eq!(d.code, codes::V0304, "{d:#?}");
    let (d, sources) = one_error(&with_matching(
        "fn f(o: Option<Item>) {\n    show(match o {\n        Some(t) => t,\n        None => mk(),\n    });\n}\n",
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
        "fn f() {\n    let mut tasks: Vec<Item> = Vec::new();\n    match mk_o() {\n        Some(t) => {\n            tasks.push(t);\n            show(t);\n        }\n        None => {}\n    }\n}\n",
    ));
    assert_eq!(d.code, codes::V0305, "{d:#?}");
    let (d, _) = one_error(&with_matching(
        "fn f() {\n    let lo = mk_o();\n    let other = lo;\n    match lo {\n        _ => {}\n    }\n}\n",
    ));
    assert_eq!(d.code, codes::V0305, "{d:#?}");
}

// --- `for`: the variable borrows from a place, owns from a temporary (3.1, 3.2)

const LOOPS: &str = "fn change_t(mut t: Item) {}\nfn change_i(mut n: i32) {}\nfn all() -> Vec<Item> {\n    vec![mk()]\n}\nfn words() -> Vec<string> {\n    vec![\"a\"]\n}\n";

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
        "fn f(mut ts: Vec<Item>) {\n    let mut a = ts[0];\n    let s = a.label;\n    println!(\"{} {}\", ts.len(), s);\n}\n",
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
        "fn f(mut ts: Vec<Item>) {\n    let mut a = ts[0];\n    let s = a.label;\n    println!(\"{} {}\", a.done, s);\n}\n",
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
        "fn f(tasks: Vec<Item>) {\n    let mine = all();\n    for t in tasks {\n        show(t);\n    }\n    for m in mine {\n        show(m);\n    }\n    for o in all() {\n        show(o);\n    }\n    let ns = vec![1];\n    for n in ns {\n        println!(\"{}\", n);\n    }\n    for i in 0..3 {\n        println!(\"{}\", i);\n    }\n}\n",
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
        "fn f(tasks: Vec<Item>, values: Vec<i32>) -> i32 {\n    for t in tasks {\n        t.label();\n    }\n    let mut total = 0;\n    for value in values {\n        total = total + value;\n    }\n    total\n}\n",
    ));
}

#[test]
fn a_for_variable_over_a_place_cannot_be_kept_and_one_over_a_temporary_can() {
    let (d, sources) = one_error(&with_loops(
        "fn f(tasks: Vec<Item>) {\n    let mut out: Vec<Item> = Vec::new();\n    for t in tasks {\n        out.push(t);\n    }\n}\n",
    ));
    assert_v0304(&d, part_of(&sources, "push(t)", "t"), "`tasks`");
    ok(&with_loops(
        "fn f() {\n    let mut out: Vec<Item> = Vec::new();\n    for t in all() {\n        out.push(t);\n    }\n}\n",
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
            "fn f(mut tasks: Vec<Item>) {{\n    for t in tasks {{\n        {stmt}\n    }}\n}}\n"
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
        "fn f(mut tasks: Vec<Item>) {\n    for t in tasks {\n        change_t(t);\n    }\n}\n",
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
        "fn f(mut tasks: Vec<Item>) {\n    for t in tasks {\n        t.label = \"x\";\n    }\n}\n",
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
        "fn f() {\n    let t = mk();\n    let mut out: Vec<Item> = Vec::new();\n    for i in 0..3 {\n        out.push(t);\n    }\n}\n",
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
        "fn f() {\n    let mut out: Vec<Item> = Vec::new();\n    for t in all() {\n        out.push(t);\n        show(t);\n    }\n}\n",
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

// --- A payload of a temporary of an enum that runs code when dropped -------

#[test]
fn a_string_payload_of_a_drop_temporary_to_a_mut_param_is_v0303_saying_to_clone() {
    let (result, sources) =
        check_path("crates/varyk/tests/fixtures/errors/v0303_drop_payload_to_mut_param/main.vr");
    let diagnostics = result.expect_err("should fail");
    assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    let d = &diagnostics[0];
    assert_eq!(d.code, codes::V0303);
    assert!(d.message.contains("`s` stays inside `Guard`"), "{d:#?}");
    assert!(d.message.contains("`let mut s = s.clone();`"), "{d:#?}");
    assert!(has_note(d, "`Guard` implements `Drop`"), "{d:#?}");
    let fix = d.fix_it.as_ref().expect("fix-it");
    assert_eq!(fix.replacement, " let mut s = s.clone();");
    let at = span_of(&sources, "=> {").end;
    assert_eq!(fix.span, Span::new(FileId(0), at, at));
}

#[test]
fn an_enum_a_macro_may_give_a_destructor_names_the_file_and_the_macro_not_an_impl() {
    let (result, _) =
        check_path("crates/varyk/tests/fixtures/errors/v0303_drop_unsure_from_macro/main.vr");
    let diagnostics = result.expect_err("should fail");
    let d = &diagnostics[0];
    assert!(d.message.contains("which may run code"), "{d:#?}");
    assert!(
        has_note(
            d,
            "Varyk cannot rule out that `Msg` runs code when it is thrown away (in Rust terms, \
             whether it has an `impl Drop`), \
             because `helper.rs` has a macro, `quiet!`, whose text mentions `Drop`"
        ),
        "{d:#?}"
    );
    assert!(!has_note(d, "`Msg` implements `Drop`"), "{d:#?}");
}

#[test]
fn an_enum_given_a_destructor_through_an_alias_names_the_alias() {
    let (result, _) = check_path("crates/varyk/tests/fixtures/interop/drop_unsure_alias/main.vr");
    let diagnostics = result.expect_err("should fail");
    assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    let d = &diagnostics[0];
    assert_eq!(d.code, codes::V0303);
    assert!(
        has_note(
            d,
            "Varyk cannot rule out that `Msg` runs code when it is thrown away (in Rust terms, \
             whether it has an `impl Drop`), \
             because `hook.rs` implements `Drop` for `G`, a name that could stand for any type"
        ),
        "{d:#?}"
    );
}

#[test]
fn a_drop_temporary_binding_kept_through_a_match_value_says_why_once_and_offers_clone() {
    let (result, sources) =
        check_path("crates/varyk/tests/fixtures/errors/v0304_drop_binding_kept_by_match/main.vr");
    let diagnostics = result.expect_err("should fail");
    // No second error about returning the outer `s`.
    assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    let d = &diagnostics[0];
    assert_eq!(d.code, codes::V0304);
    assert!(
        d.message.ends_with("cannot be kept in the outer `s`"),
        "{d:#?}"
    );
    assert!(
        has_note(
            d,
            "copy it with `.clone()` to keep it (in Rust terms, `Msg` implements `Drop`)"
        ),
        "{d:#?}"
    );
    let fix = d.fix_it.as_ref().expect("fix-it");
    assert_eq!(fix.replacement, ".clone()");
    assert_eq!(fix.span.start, span_of(&sources, "=> s,").start + 4);
}

#[test]
fn a_drop_temporary_binding_assigned_to_an_outer_let_says_why_and_offers_clone() {
    let (result, _) =
        check_path("crates/varyk/tests/fixtures/interop/drop_payload_assigned/main.vr");
    let diagnostics = result.expect_err("should fail");
    assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    let d = &diagnostics[0];
    assert_eq!(d.code, codes::V0304);
    assert!(
        has_note(d, "`s` stays inside the `Msg` matched on"),
        "{d:#?}"
    );
    assert_eq!(
        d.fix_it.as_ref().map(|fix| fix.replacement.as_str()),
        Some(".clone()")
    );
}

#[test]
fn a_struct_payload_of_a_drop_temporary_changed_through_a_let_names_the_let_and_offers_no_fix() {
    let (result, _) =
        check_path("crates/varyk/tests/fixtures/interop/drop_payload_changed/main.vr");
    let diagnostics = result.expect_err("should fail");
    assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    let d = &diagnostics[0];
    assert_eq!(d.code, codes::V0303);
    assert_eq!(
        d.message,
        "`add` may change `b`, but `b` stays inside `Guard`, which runs code when it is thrown \
         away, so it cannot be changed here"
    );
    assert!(d.fix_it.is_none(), "{d:#?}");
}

#[test]
fn a_payload_kept_from_a_drop_enum_offers_clone_only_for_a_string_or_bytes_and_names_the_enum() {
    let (result, _) = check_path("crates/varyk/tests/fixtures/interop/drop_payload_kept/main.vr");
    let diagnostics = result.expect_err("should fail");
    assert_eq!(diagnostics.len(), 4, "{diagnostics:#?}");
    assert!(diagnostics.iter().all(|d| d.code == codes::V0304));
    // A stored enum: the `Acc` payload can be read, not kept.
    let stored = &diagnostics[0];
    assert!(
        has_note(stored, "`a` can be read here, but not kept"),
        "{stored:#?}"
    );
    assert!(
        !stored.notes.iter().any(|n| n.contains(".clone()")),
        "{stored:#?}"
    );
    // A temporary: the note names the enum and says why.
    let temporary = &diagnostics[1];
    assert!(
        has_note(
            temporary,
            "`a` stays inside the `Guard` matched on, which runs code when it is thrown away"
        ),
        "{temporary:#?}"
    );
    assert!(
        !temporary.notes.iter().any(|n| n.contains(".clone()")),
        "{temporary:#?}"
    );
    // A `string` payload can be copied, and so can a `Bytes` one
    // (milestone 5c spec 2.3).
    for copied in &diagnostics[2..] {
        assert!(
            has_note(copied, "copy it with `.clone()` to keep it"),
            "{copied:#?}"
        );
        assert_eq!(
            copied.fix_it.as_ref().map(|fix| fix.replacement.as_str()),
            Some(".clone()"),
            "{copied:#?}"
        );
    }
}

#[test]
fn a_type_name_starting_with_a_vowel_takes_an() {
    let (d, _) = one_error(
        "struct Acc {\n    name: string,\n}\n\nfn mk() -> Acc {\n    Acc { name: \"a\" }\n}\n\nfn f() -> string {\n    mk().name\n}\n\nfn main() {}\n",
    );
    assert!(d.message.contains("kept inside an `Acc`,"), "{d:#?}");
    assert_eq!(super::article("P"), "a");
    assert_eq!(super::article("Order"), "an");
    assert_eq!(super::article("User"), "a");
    assert_eq!(super::article("Unit"), "a");
    assert_eq!(super::article("Item"), "an");
}

// --- Milestone 4: nested patterns, `if let`, `while let`, string heads -----

const SHAPES: &str = "enum Shape {\n    Circle(f64),\n    Named(string),\n}\nfn mko() -> Option<Shape> {\n    None\n}\n";

fn with_shapes(body: &str) -> String {
    format!("{P}{STRS}{SHAPES}{body}{MAIN}")
}

#[test]
fn a_move_inside_a_while_let_body_is_v0305_the_next_time_round() {
    let (d, sources) = one_error(&with_shapes(
        "fn f() {\n    let mut v: Vec<i32> = vec![1];\n    let a = P { x: 1, name: \"a\" };\n    while let Some(n) = v.pop() {\n        let b = a;\n    }\n}\n",
    ));
    let at = part_of(&sources, "let b = a", "a");
    assert_v0305(&d, at, at, "a");
    assert!(
        has_note(&d, "it was given away the previous time through the loop"),
        "{d:#?}"
    );
}

#[test]
fn nested_bindings_on_a_place_are_aliases_and_on_a_temporary_own_their_values() {
    let program = ok(&with_shapes(
        "fn f(o: Option<Shape>) {\n    match o {\n        Some(Shape::Named(n)) => read(n),\n        Some(Shape::Circle(r)) => {}\n        None => {}\n    }\n    if let Some(Shape::Named(t)) = mko() {\n        read(t);\n    }\n    let mut q: Vec<i32> = vec![];\n    while let Some(k) = q.pop() {\n    }\n}\n",
    ));
    let f = function(&program, "f");
    let n = place(f, "n");
    assert!(n.borrowed && !n.mutable, "{n:?}");
    assert_eq!(n.origin, Some(Origin::Param(LocalId(0))));
    assert!(!place(f, "r").borrowed);
    assert!(!place(f, "t").borrowed);
    assert_eq!(repr(f, "t"), Some(StringRepr::Owned));
    assert!(!place(f, "k").borrowed);
}

#[test]
fn a_string_binding_of_a_string_head_is_a_str_whatever_the_head() {
    let program = ok(&with_shapes(
        "fn f(s: string) {\n    let owned = mk();\n    match owned {\n        \"a\" => {}\n        other => read(other),\n    }\n    match mk() {\n        whole => read(whole),\n    }\n    if let \"b\" = s {\n    }\n}\n",
    ));
    let f = function(&program, "f");
    assert_eq!(repr(f, "other"), Some(StringRepr::Str));
    assert_eq!(repr(f, "whole"), Some(StringRepr::Str));
    assert!(place(f, "whole").borrowed);
}

#[test]
fn keeping_a_string_binding_of_a_temporary_string_head_is_v0304() {
    // Returned, from a `match` with a literal arm.
    let (d, sources) = one_error(&with_shapes(
        "fn g() -> string {\n    match mk() {\n        \"a\" => \"x\",\n        other => other,\n    }\n}\n",
    ));
    assert_eq!(d.code, codes::V0304, "{d:#?}");
    assert_eq!(d.span, part_of(&sources, "=> other", "other"));
    // Stored, from a `match` with no literal arm.
    let (d, sources) = one_error(&with_shapes(
        "fn g() {\n    let mut v: Vec<string> = vec![];\n    match mk() {\n        other => v.push(other),\n    }\n}\n",
    ));
    assert_eq!(d.code, codes::V0304, "{d:#?}");
    assert_eq!(d.span, part_of(&sources, "push(other)", "other"));
    // A copy can be kept.
    ok(&with_shapes(
        "fn g() {\n    let mut v: Vec<string> = vec![];\n    match mk() {\n        other => v.push(other.clone()),\n    }\n}\n",
    ));
}

// --- Table rows (M4 spec 2.7, 3.5) --------------------------------------------

#[test]
fn read_arguments_of_the_table_take_a_parameter_or_alias_and_leave_a_let_borrowed() {
    let program = ok(&with_p(
        "fn f(p: P, s: string, sep: string, w: Vec<string>, m: HashMap<string, i32>) {
    let alias = p.name;
    let lit = \"x\";
    let a = s.contains(s);
    let b = s.contains(alias);
    let c = s.replace(sep, alias);
    let d = w.join(sep);
    let e = m.contains_key(s);
    let g = m.contains_key(alias);
    let h = s.starts_with(lit);
    let i = w.contains(alias);
    let j = s.contains(lit);
}
",
    ));
    let f = function(&program, "f");
    assert!(place(f, "alias").borrowed);
    assert_eq!(repr(f, "lit"), Some(StringRepr::Str));
}

#[test]
fn push_str_makes_its_receivers_let_owned() {
    let program = ok(&with_p(
        "fn f(p: P) {
    let mut s = \"a\";
    s.push_str(p.name);
    let mut t = \"b\";
    t.push_str(\"c\");
}
",
    ));
    let f = function(&program, "f");
    assert_eq!(repr(f, "s"), Some(StringRepr::Owned));
    assert_eq!(repr(f, "t"), Some(StringRepr::Owned));
}

#[test]
fn text_made_by_the_table_rows_is_an_owned_let() {
    let program = ok(&with_p(
        "fn f(s: string, mut w: Vec<string>) {
    let a = s.to_uppercase();
    let b = s.replace(\"a\", \"b\");
    let c = w.join(\",\");
    let d = w.remove(0);
}
",
    ));
    let f = function(&program, "f");
    for name in ["a", "b", "c", "d"] {
        assert_eq!(repr(f, name), Some(StringRepr::Owned), "{name}");
    }
}

#[test]
fn an_owned_argument_of_the_table_is_an_owned_slot() {
    for body in [
        "fn f(s: string, mut w: Vec<string>) {\n    w.insert(0, s);\n}\n",
        "fn f(s: string, mut m: HashMap<string, i32>) {\n    m.insert(s, 1);\n}\n",
    ] {
        let (d, _) = one_error(&with_p(body));
        assert_eq!(d.code, codes::V0304, "{body}: {d:#?}");
    }
}

#[test]
fn a_hash_map_moves_like_a_vec() {
    let (d, _) = one_error(&with_p(
        "fn f() {\n    let m: HashMap<i32, i32> = HashMap::new();\n    let m2 = m;\n    let n = m.len();\n}\n",
    ));
    assert_eq!(d.code, codes::V0305, "{d:#?}");
}

// --- Taking rows (M4 spec 3.4) --------------------------------------------------

#[test]
fn a_taking_row_uses_up_a_temporary() {
    ok(&with_p(
        "fn mk_o() -> Option<string> {\n    None\n}\nfn mk_r() -> Result<string, string> {\n    Ok(\"a\")\n}\nfn f() -> Result<string, string> {\n    let a = mk_o().unwrap_or(\"x\");\n    let b = mk_r().ok();\n    let c = mk_r().unwrap_or(\"y\");\n    let d = mk_o().ok_or(\"none\")?;\n    Ok(d)\n}\n",
    ));
}

#[test]
fn a_taking_row_gives_away_an_owned_local() {
    for (body, reused) in [
        (
            "let o: Option<string> = None;\n    let a = o.unwrap_or(\"x\");\n    let b = o.unwrap_or(\"y\");",
            "o.unwrap_or(\"y\")",
        ),
        (
            "let r: Result<string, string> = Ok(\"a\");\n    let a = r.ok();\n    let b = r.ok();",
            "r.ok();\n}",
        ),
        (
            "let o: Option<string> = None;\n    let a = o.ok_or(\"none\");\n    let b = o.is_some();",
            "o.is_some()",
        ),
    ] {
        let (d, sources) = one_error(&with_p(&format!("fn f() {{\n    {body}\n}}\n")));
        assert_eq!(d.code, codes::V0305, "{body}: {d:#?}");
        assert_eq!(
            d.span,
            part_of(&sources, reused, reused.split('.').next().unwrap_or("")),
            "{body}"
        );
    }
}

#[test]
fn a_stored_receiver_copy_in_rust_is_copied_out() {
    ok(&with_p(
        "struct H {\n    o: Option<i32>,\n    r: Result<i32, bool>,\n}\nfn f(o: Option<i32>, h: H, v: Vec<Option<i32>>) -> i32 {\n    let a = o.unwrap_or(0);\n    let b = h.o.unwrap_or(1);\n    let c = h.r.ok();\n    let d = h.r.unwrap_or(2);\n    let e = v[0].ok_or(false);\n    let g = o.unwrap_or(3);\n    a + b + d + g\n}\n",
    ));
}

#[test]
fn a_stored_receiver_with_contents_not_copy_is_v0304_match_on_it_instead() {
    for (params, call) in [
        ("r: Result<i32, string>", "r.unwrap_or(0)"),
        ("o: Option<string>", "o.unwrap_or(\"x\")"),
        ("p: P, v: Vec<Option<P>>", "v[0].ok_or(1)"),
        ("r: Result<string, bool>", "r.ok()"),
    ] {
        let (d, sources) = one_error(&with_p(&format!(
            "fn f({params}) {{\n    let x = {call};\n}}\n"
        )));
        assert_eq!(d.code, codes::V0304, "{call}: {d:#?}");
        let receiver = &call[..call.find('.').unwrap_or(call.len())];
        assert_eq!(d.span, part_of(&sources, call, receiver), "{call}");
        assert!(d.message.contains("used up by"), "{d:#?}");
        assert!(
            d.notes
                .iter()
                .any(|note| note.contains("match on it instead")),
            "{d:#?}"
        );
    }
}

// --- Looked-into rows (M4 spec 2.8) -----------------------------------------------

#[test]
fn a_looked_into_get_head_binds_an_alias_of_the_vec() {
    let program = ok(&with_p(
        "fn f(lines: Vec<string>, ps: Vec<P>) {
    let mut v = vec![\"a\"];
    match v.get(0) {
        Some(first) => read(first),
        None => {}
    }
    if let Some(second) = lines.get(1) {
        read(second);
    }
    let mut i: usize = 0;
    while let Some(p) = ps.get(i) {
        read(p.name);
        i = i + 1;
    }
    v.push(\"b\");
}
fn read(s: string) {}
",
    ));
    let f = function(&program, "f");
    let v = local_id(f, "v");
    let lines = local_id(f, "lines");
    let ps = local_id(f, "ps");
    assert_eq!(
        place(f, "first"),
        PlaceInfo {
            borrowed: true,
            mutable: false,
            origin: Some(Origin::Local(v)),
        }
    );
    assert_eq!(place(f, "second").origin, Some(Origin::Param(lines)));
    assert_eq!(place(f, "p").origin, Some(Origin::Param(ps)));
    assert!(place(f, "second").borrowed && place(f, "p").borrowed);
}

#[test]
fn a_looked_into_get_anywhere_but_a_head_is_v0208() {
    for (body, at) in [
        ("let x = v.get(0);", "v.get(0)"),
        ("take(v.get(0));", "v.get(0)"),
        ("let b = v.get(0).is_some();", "v.get(0)"),
        ("let n = m.get(\"k\");", "m.get(\"k\")"),
    ] {
        let (d, sources) = one_error(&with_p(&format!(
            "fn take(o: Option<string>) {{}}\nfn f(v: Vec<string>, m: HashMap<string, P>) {{\n    {body}\n}}\n"
        )));
        assert_eq!(d.code, codes::V0208, "{body}: {d:#?}");
        assert_eq!(d.span, span_of(&sources, at), "{body}");
        assert!(
            d.message.contains("`match` or `if let`") || d.message.contains("or `match`"),
            "{d:#?}"
        );
    }
    for (ret, body) in [
        ("Option<string>", "return v.get(0);"),
        ("Option<string>", "v.get(0)"),
        ("Option<string>", "let x = v.get(0)?;\n    None"),
    ] {
        let (d, _) = one_error(&with_p(&format!(
            "fn f(v: Vec<string>) -> {ret} {{\n    {body}\n}}\n"
        )));
        assert_eq!(d.code, codes::V0208, "{body}: {d:#?}");
    }
}

#[test]
fn a_whole_binding_of_a_looked_into_get_is_v0208() {
    let (d, _) = one_error(&with_p(
        "fn f(v: Vec<string>) {\n    match v.get(0) {\n        whole => {}\n    }\n}\n",
    ));
    assert_eq!(d.code, codes::V0208, "{d:#?}");
}

#[test]
fn a_looked_into_binding_used_after_its_vec_changes_is_v0307() {
    let (d, sources) = one_error(&with_p(
        "fn read(s: string) {}\nfn f() {\n    let mut v = vec![\"a\"];\n    if let Some(first) = v.get(0) {\n        v.push(\"b\");\n        read(first);\n    }\n}\n",
    ));
    assert_eq!(d.code, codes::V0307, "{d:#?}");
    assert_eq!(d.span, part_of(&sources, "v.push", "v"), "{d:#?}");
}

#[test]
fn a_looked_into_get_on_a_value_made_right_there_is_v0001() {
    let (d, sources) = one_error(&with_p(
        "fn mk() -> Vec<string> {\n    vec![\"a\"]\n}\nfn f() {\n    if let Some(x) = mk().get(0) {}\n}\n",
    ));
    assert_eq!(d.code, codes::V0001, "{d:#?}");
    assert_eq!(d.span, part_of(&sources, "mk().get", "mk()"), "{d:#?}");
    assert!(d.message.contains("store it with `let` first"), "{d:#?}");
}

#[test]
fn a_get_of_a_copy_payload_is_a_plain_option_whatever_the_receiver() {
    ok(&with_p(
        "fn mk() -> Vec<i32> {\n    vec![1]\n}\nfn mk_m() -> HashMap<string, i32> {\n    HashMap::new()\n}\nfn f(v: Vec<i32>, m: HashMap<string, i32>) -> Option<i32> {\n    let a = v.get(0);\n    let b = mk().get(0).unwrap_or(0);\n    let c = mk_m().get(\"k\").is_some();\n    let d = m.get(\"k\")?;\n    let mut w = vec![1];\n    let e = w.get(0);\n    w.push(2);\n    e\n}\n",
    ));
}

#[test]
fn a_get_keyed_by_a_parameter_or_alias_leaves_a_let_borrowed() {
    let program = ok(&with_p(
        "fn f(p: P, word: string, counts: HashMap<string, i32>, people: HashMap<string, P>) -> i32 {
    let alias = p.name;
    let a = counts.get(word).unwrap_or(0);
    let b = counts.get(alias).unwrap_or(0);
    if let Some(found) = people.get(alias) {
        let n = found.x;
    }
    a + b
}
",
    ));
    let f = function(&program, "f");
    assert!(place(f, "alias").borrowed);
    assert!(place(f, "found").borrowed);
}

// --- Borrowed returns at the call (M4 spec 3.1) ------------------------------

/// A struct with getters that are borrowed returns, and the helpers the
/// tests below call.
const GETTERS: &str = "struct Obj {
    name: string,
    tags: Vec<string>,
    status: Status,
    n: i32,
}
enum Status {
    On(string),
    Off,
}
impl Obj {
    fn name_ref(self) -> string {
        self.name
    }
    fn display_name(self) -> string {
        if self.name.is_empty() { self.name } else { self.name }
    }
    fn tags(self) -> Vec<string> {
        self.tags
    }
    fn status(self) -> Status {
        self.status
    }
    fn change(mut self) {
        self.n = self.n + 1;
    }
}
fn mk() -> Obj {
    Obj { name: \"o\", tags: vec![\"t\"], status: Status::Off, n: 0 }
}
fn trimmed(text: string) -> string {
    text.trim()
}
fn read(s: string) {}
fn show(o: Obj) {}
fn first(objs: Vec<Obj>) -> Obj {
    objs[0]
}
";

fn with_getters(body: &str) -> String {
    format!("{GETTERS}{body}{MAIN}")
}

#[test]
fn a_let_of_a_borrowed_return_is_an_alias_of_the_argument() {
    let program = ok(&with_getters(
        "fn f(p: Obj) {\n    let mut user = mk();\n    let n = user.name_ref();\n    read(n);\n    user.change();\n    read(user.name_ref());\n}\n",
    ));
    let f = function(&program, "f");
    assert!(place(f, "n").borrowed);
    assert!(!place(f, "n").mutable);

    let (d, sources) = one_error(&with_getters(
        "fn f() {\n    let mut user = mk();\n    let n = user.name_ref();\n    user.change();\n    read(n);\n}\n",
    ));
    assert_eq!(d.code, codes::V0307, "{d:#?}");
    assert_eq!(d.span, part_of(&sources, "user.change", "user"));
    assert!(
        d.message.contains("`n` is another name for part of `user`"),
        "{d:#?}"
    );
}

#[test]
fn a_borrowed_return_into_an_owned_slot_is_v0304_with_clone() {
    let (d, sources) = one_error(&with_getters(
        "fn f(user: Obj) {\n    let mut v = vec![\"a\"];\n    v.push(user.name_ref());\n}\n",
    ));
    assert_eq!(d.code, codes::V0304, "{d:#?}");
    assert_eq!(d.span, span_of(&sources, "user.name_ref()"));
    let fix = d.fix_it.as_ref().expect("a `.clone()` fix-it");
    assert_eq!(fix.replacement, ".clone()");
}

#[test]
fn a_rooted_argument_made_right_there_is_v0001_but_a_literal_is_fine() {
    let (d, sources) = one_error(&with_getters(
        "fn g() -> string {\n    \"  x \"\n}\nfn f() {\n    read(trimmed(g()));\n}\n",
    ));
    assert_eq!(d.code, codes::V0001, "{d:#?}");
    assert_eq!(d.span, part_of(&sources, "trimmed(g())", "g()"));
    assert!(d.message.contains("store it with `let` first"), "{d:#?}");
    let (d, sources) = one_error(&with_getters("fn f() {\n    read(mk().name_ref());\n}\n"));
    assert_eq!(d.code, codes::V0001, "{d:#?}");
    assert_eq!(d.span, part_of(&sources, "read(mk()", "mk()"));
    assert!(d.message.contains("this `Obj`"), "{d:#?}");

    ok(&with_getters("fn f() {\n    read(trimmed(\"  x \"));\n}\n"));
}

#[test]
fn heads_on_a_borrowed_return_bind_aliases_rooted_at_its_argument() {
    let program = ok(&with_getters(
        "fn f(obj: Obj) {\n    match obj.status() {\n        Status::On(s) => read(s),\n        Status::Off => {}\n    }\n    for t in obj.tags() {\n        read(t);\n    }\n    match obj.tags().get(0) {\n        Some(first) => read(first),\n        None => {}\n    }\n}\n",
    ));
    let f = function(&program, "f");
    for name in ["s", "t", "first"] {
        assert!(place(f, name).borrowed, "{name}");
        assert_eq!(
            place(f, name).origin,
            Some(Origin::Param(LocalId(0))),
            "{name}"
        );
    }

    let (d, sources) = one_error(&with_getters(
        "fn f() {\n    let mut obj = mk();\n    for t in obj.tags() {\n        obj.change();\n    }\n}\n",
    ));
    assert_eq!(d.code, codes::V0307, "{d:#?}");
    assert_eq!(d.span, part_of(&sources, "obj.change", "obj"));
    let (d, _) = one_error(&with_getters(
        "fn f() {\n    let mut obj = mk();\n    match obj.status() {\n        Status::On(s) => {\n            obj.change();\n            read(s);\n        }\n        Status::Off => {}\n    }\n}\n",
    ));
    assert_eq!(d.code, codes::V0307, "{d:#?}");
}

#[test]
fn trim_is_part_of_its_receiver() {
    // On a place: an alias of it.
    let (d, sources) = one_error(&with_getters(
        "fn f() {\n    let mut text = \"  a \".clone();\n    let t = text.trim();\n    text.push_str(\"b\");\n    read(t);\n}\n",
    ));
    assert_eq!(d.code, codes::V0307, "{d:#?}");
    assert_eq!(d.span, part_of(&sources, "text.push_str", "text"));
    let (d, sources) = one_error(&with_getters(
        "fn f(text: string) {\n    let mut v = vec![\"a\"];\n    v.push(text.trim());\n}\n",
    ));
    assert_eq!(d.code, codes::V0304, "{d:#?}");
    assert_eq!(
        d.span,
        part_of(&sources, "push(text.trim())", "text.trim()")
    );
    assert!(d.fix_it.is_some(), "{d:#?}");
    // On a literal: rootless, so returning it copies nothing.
    let (d, sources) = one_error(&with_getters(
        "fn f() -> string {\n    \"  a \".trim()\n}\n",
    ));
    assert_eq!(d.code, codes::V0304, "{d:#?}");
    assert_eq!(d.span, span_of(&sources, "\"  a \".trim()"));
    // On a call result: V0001.
    let (d, sources) = one_error(&with_getters(
        "fn g() -> string {\n    \"a\"\n}\nfn f() {\n    read(g().trim());\n}\n",
    ));
    assert_eq!(d.code, codes::V0001, "{d:#?}");
    assert_eq!(d.span, part_of(&sources, "g().trim", "g()"));
}

#[test]
fn borrowed_text_chains_and_binds_as_str() {
    let program = ok(&with_getters(
        "fn f(user: Obj) {\n    let s = \"  x \".clone();\n    let t = s.trim();\n    let owned = mk();\n    let n = owned.display_name();\n    let c = user.name_ref().trim();\n    let mut m = \"x\";\n    m = owned.name_ref();\n    read(t);\n    read(n);\n    read(c);\n    read(m);\n}\n",
    ));
    let f = function(&program, "f");
    for name in ["t", "n", "c", "m"] {
        assert_eq!(repr(f, name), Some(StringRepr::Str), "{name}");
    }
    assert_eq!(repr(f, "s"), Some(StringRepr::Owned));
    // A chained result is still part of the first receiver.
    let (d, _) = one_error(&with_getters(
        "fn f() {\n    let mut user = mk();\n    let c = user.name_ref().trim();\n    user.change();\n    read(c);\n}\n",
    ));
    assert_eq!(d.code, codes::V0307, "{d:#?}");
}

#[test]
fn a_function_whose_returns_are_in_error_is_reported_once_and_new_to_callers() {
    let (d, sources) = one_error(&with_getters(
        "fn pick(a: Obj, b: Obj, c: bool) -> string {\n    if c { a.name } else { b.name }\n}\nfn f(a: Obj, b: Obj) {\n    let n = pick(a, b, true);\n    let mut v = vec![\"x\"];\n    v.push(n);\n}\n",
    ));
    assert_eq!(d.code, codes::V0308, "{d:#?}");
    assert_eq!(d.span, span_of(&sources, "b.name"));
    assert!(
        d.message
            .contains("part of `a` in one place and part of `b` in another"),
        "{d:#?}"
    );
}

#[test]
fn a_borrowed_return_on_a_local_of_a_block_is_gone_with_it() {
    let (d, sources) = one_error(&with_getters(
        "fn f() {\n    let n = {\n        let u = mk();\n        u.name_ref()\n    };\n    read(n);\n}\n",
    ));
    assert_eq!(d.code, codes::V0304, "{d:#?}");
    assert_eq!(d.span, span_of(&sources, "u.name_ref()"));
    let (d, sources) = one_error(&with_getters(
        "fn f(c: bool) {\n    println!(\"{}\", if c {\n        let u = mk();\n        u.name_ref()\n    } else {\n        \"x\"\n    });\n}\n",
    ));
    assert_eq!(d.code, codes::V0304, "{d:#?}");
    assert_eq!(d.span, span_of(&sources, "u.name_ref()"));
    let (d, sources) = one_error(&with_getters(
        "fn f(c: bool, o: Obj) {\n    show(if c {\n        let us = vec![mk()];\n        first(us)\n    } else {\n        o\n    });\n}\n",
    ));
    assert_eq!(d.code, codes::V0304, "{d:#?}");
    assert_eq!(d.span, span_of(&sources, "first(us)"));
}

#[test]
fn a_let_mut_of_a_borrowed_return_cannot_be_changed_whatever_its_mut() {
    for (body, code) in [
        ("n = \"x\";", codes::V0301),
        ("let mut v = vec![\"a\"];\n    fill(n);", codes::V0302),
    ] {
        let (d, _) = one_error(&with_getters(&format!(
            "fn fill(mut s: string) {{}}\nfn f() {{\n    let mut user = mk();\n    let mut n = user.name_ref();\n    {body}\n    read(n);\n}}\n"
        )));
        assert_eq!(d.code, code, "{d:#?}");
        assert!(
            d.message.contains("`n` is another name for part of `user`"),
            "{d:#?}"
        );
        assert!(!d.message.contains("without `mut`"), "{d:#?}");
        assert!(d.fix_it.is_none(), "{d:#?}");
        assert!(has_note(&d, ".clone()"), "{d:#?}");
    }
}

#[test]
fn a_changed_borrowed_return_says_so_before_blaming_a_missing_mut() {
    // Neither the root nor the result is `mut`: adding either would not
    // help, so the read-only result is the one diagnostic.
    let first = "fn first(v: Vec<string>) -> string {\n    v[0]\n}\n";
    for (body, root) in [
        (
            "fn f() {\n    let v = vec![\"a\"];\n    let q = first(v);\n    q.push_str(\"y\");\n}\n",
            "v",
        ),
        (
            "fn f(v: Vec<string>) {\n    let q = first(v);\n    q.push_str(\"y\");\n}\n",
            "v",
        ),
        (
            "fn f() {\n    let mut v = vec![\"a\"];\n    let q = first(v);\n    q.push_str(\"y\");\n}\n",
            "v",
        ),
    ] {
        let (d, _) = one_error(&format!("{first}{body}fn main() {{}}\n"));
        assert_eq!(d.code, codes::V0301, "{d:#?}");
        assert!(
            d.message.contains(&format!(
                "`q` is another name for part of `{root}`, given by a call"
            )),
            "{d:#?}"
        );
        assert!(d.fix_it.is_none(), "{d:#?}");
        assert!(has_note(&d, ".clone()"), "{d:#?}");
    }
    // A field of the result, `mut` or not, changed or assigned.
    let first = "struct U {\n    name: string,\n    tags: Vec<string>,\n}\nfn first(v: Vec<U>) -> U {\n    v[0]\n}\n";
    for change in [
        "let mut q = first(v).name;\n    q.push_str(\"y\");",
        "let q = first(v).name;\n    q.push_str(\"y\");",
        "let mut q = first(v).tags;\n    q.push(\"z\");",
        "let mut q = first(v).name;\n    q = \"z\";",
    ] {
        let body = format!(
            "fn f() {{\n    let mut v = vec![U {{ name: \"a\", tags: vec![\"t\"] }}];\n    {change}\n}}\n"
        );
        let (d, _) = one_error(&format!("{first}{body}fn main() {{}}\n"));
        let root = "v";
        assert_eq!(d.code, codes::V0301, "{d:#?}");
        assert!(
            d.message.contains(&format!(
                "`q` is another name for part of `{root}`, given by a call"
            )),
            "{d:#?}"
        );
        assert!(d.fix_it.is_none(), "{d:#?}");
        assert!(has_note(&d, ".clone()"), "{d:#?}");
    }
}

// --- Closures (M4 spec 2.2, 3.2) -------------------------------------------

/// A function `f` with an owned `name`, a literal `lit`, and a Copy `k`
/// to capture, around `body`.
fn with_captures(body: &str) -> String {
    format!(
        "struct P {{\n    s: string,\n}}\nfn mk() -> string {{\n    \"made\"\n}}\nfn mk_o() -> Option<P> {{\n    Some(P {{ s: mk() }})\n}}\nfn read(s: string) {{}}\nfn fill(mut s: string) {{}}\nfn f(o: Option<i32>) {{\n    let mut name = mk();\n    let lit = \"lit\";\n    let mut k = 1;\n    {body}\n    read(name);\n    read(lit);\n}}\nfn main() {{}}\n"
    )
}

#[test]
fn a_captured_local_keeps_its_representation_and_stays_usable() {
    let program = ok(&with_captures(
        "let a = o.map(|n| {\n        read(name);\n        read(lit);\n        let y = name;\n        read(y);\n        n + k\n    });\n    let b = o.map(|n| name == lit);",
    ));
    let f = function(&program, "f");
    assert_eq!(repr(f, "name"), Some(StringRepr::Owned));
    assert_eq!(repr(f, "lit"), Some(StringRepr::Str));
    // `let y = name;` inside the closure is another name for `name`.
    let y = place(f, "y");
    assert!(y.borrowed, "{y:?}");
    assert!(!y.mutable, "{y:?}");
    let name = f
        .locals
        .iter()
        .position(|l| l.name == "name")
        .expect("name");
    assert_eq!(y.origin, Some(Origin::Captured(LocalId(name as u32))));
    assert_eq!(repr(f, "y"), Some(StringRepr::RefOwned));
    // The parameter is an owned copy of the payload.
    assert!(!place(f, "n").borrowed);
}

#[test]
fn changing_a_captured_name_is_v0301_or_v0303_worded_for_a_closure() {
    let cases = [
        ("name = \"z\";", "name = \"z\"", codes::V0301),
        ("k = 2;", "k = 2", codes::V0301),
        ("fill(name);", "name);", codes::V0303),
        ("fill(lit);", "lit);", codes::V0303),
    ];
    for (stmt, at, code) in cases {
        let (d, sources) = one_error(&with_captures(&format!(
            "let a = o.map(|n| {{\n        {stmt}\n        n\n    }});"
        )));
        assert_eq!(d.code, code, "{stmt}: {d:#?}");
        let at = span_of(&sources, at);
        assert_eq!(d.span.start, at.start, "{stmt}: {d:#?}");
        assert!(
            d.message
                .contains("inside a closure a name from outside can only be read"),
            "{stmt}: {d:#?}"
        );
        assert!(d.fix_it.is_none(), "{stmt}: {d:#?}");
    }
}

#[test]
fn a_captured_name_kept_in_an_owned_slot_is_v0304() {
    let (d, sources) = one_error(&with_captures(
        "let a = o.map(|n| {\n        let mut v: Vec<string> = Vec::new();\n        v.push(name);\n        n\n    });",
    ));
    assert_eq!(d.code, codes::V0304, "{d:#?}");
    assert_eq!(d.span, part_of(&sources, "v.push(name)", "name"));
    assert!(
        d.message
            .starts_with("`name` belongs to the function around this closure"),
        "{d:#?}"
    );
    assert_eq!(
        d.fix_it.as_ref().map(|f| f.replacement.as_str()),
        Some(".clone()")
    );
}

#[test]
fn changing_a_closure_parameter_is_v0301_or_v0303_with_the_copy_fix_it() {
    let (d, _) = one_error(&with_captures(
        "let a = o.map(|n| {\n        n = 2;\n        n\n    });",
    ));
    assert_eq!(d.code, codes::V0301, "{d:#?}");
    assert!(
        d.message.contains(
            "`n` is a closure's parameter, so it cannot be changed; to change it, first make a \
             changeable copy with `let mut n = n;`"
        ),
        "{d:#?}"
    );
    let fix = d.fix_it.as_ref().expect("a fix-it");
    assert_eq!(fix.replacement, " let mut n = n;");
    let (d, _) = one_error(&with_captures("let a = mk_o().map(|p| fill(p.s));"));
    assert_eq!(d.code, codes::V0303, "{d:#?}");
    assert!(d.message.contains("`p` is a closure's parameter"), "{d:#?}");
}

#[test]
fn an_option_map_closure_returning_part_of_something_is_v0304_or_v0308() {
    let (d, sources) = one_error(&with_captures("let a = o.map(|n| name);"));
    assert_eq!(d.code, codes::V0304, "{d:#?}");
    assert_eq!(d.span, part_of(&sources, "|n| name", "name"));
    assert!(
        d.message
            .starts_with("`name` belongs to the function around this closure"),
        "{d:#?}"
    );
    assert_eq!(
        d.fix_it.as_ref().map(|f| f.replacement.as_str()),
        Some(".clone()")
    );
    let (d, sources) = one_error(&with_captures("let a = mk_o().map(|p| p.s);"));
    assert_eq!(d.code, codes::V0304, "{d:#?}");
    assert_eq!(d.span, span_of(&sources, "p.s"));
    assert!(d.message.contains("`p`"), "{d:#?}");
    let (d, _) = one_error(&with_captures(
        "let a = o.map(|n| if n > 0 { name } else { lit });",
    ));
    assert_eq!(d.code, codes::V0308, "{d:#?}");
    let (d, _) = one_error(&with_captures(
        "let a = o.map(|n| if n > 0 { name } else { mk() });",
    ));
    assert_eq!(d.code, codes::V0304, "{d:#?}");
    // Something new is fine: a copy, a call result, the parameter itself.
    ok(&with_captures(
        "let a = o.map(|n| name.clone());\n    let b = mk_o().map(|p| p);\n    let c = mk_o().map(|p| p.s.clone());",
    ));
}

#[test]
fn a_string_literal_if_body_gives_an_owned_string() {
    let program = ok(&with_captures(
        "let a = o.map(|n| if n > 5 { \"big\" } else { \"small\" });\n    let b: Option<string> = a;",
    ));
    let f = function(&program, "f");
    assert_eq!(
        f.locals
            .iter()
            .find(|l| l.name == "a")
            .map(|l| l.ty.clone()),
        Some(crate::types::Ty::Option(Box::new(crate::types::Ty::String)))
    );
}

// --- Chains (M4 spec 2.3, 3.3) ----------------------------------------------

/// A function `f` over a `Vec<i32>` `v`, a `Vec<string>` `names`, a
/// `Vec<U>` `users`, a `HashMap<string, i32>` `m`, a `U` `other`, a text
/// `text`, and a parameter `p`, around `body`.
fn with_chains(body: &str) -> String {
    format!(
        "struct U {{\n    name: string,\n    age: i32,\n    tags: Vec<string>,\n}}\nimpl U {{\n    fn tags_of(self) -> Vec<string> {{\n        self.tags\n    }}\n    fn name_of(self) -> string {{\n        self.name\n    }}\n}}\nfn mk_u() -> U {{\n    U {{ name: \"u\", age: 1, tags: vec![\"t\"] }}\n}}\nfn mk_vs() -> Vec<string> {{\n    vec![\"a\"]\n}}\nfn mk_m() -> HashMap<string, i32> {{\n    HashMap::new()\n}}\nfn mk_s() -> string {{\n    \"a b\"\n}}\nfn read(s: string) {{}}\nfn read_u(u: U) {{}}\nfn f(c: bool, p: U) {{\n    let mut v = vec![1, 2];\n    let mut names = vec![\"a\", \"b\"];\n    let mut users = vec![mk_u()];\n    let mut m: HashMap<string, i32> = HashMap::new();\n    let mut other = mk_u();\n    let text = \"a b\";\n    {body}\n}}\nfn main() {{}}\n"
    )
}

/// The item table of a function (see `PlacesOutput::items`).
type ItemTable = std::collections::HashMap<Span, (super::chains::ItemKind, Option<StringRepr>)>;

/// The item table of `f` in `text`, which must pass the checker.
fn chain_items(text: &str) -> (ItemTable, HirProgram, Vec<SourceFile>) {
    let mut sources = Vec::new();
    let entry = SourceFile::new(FileId(0), "dummy/test.vr", text);
    let hir = resolve(entry, &mut sources)
        .and_then(|resolved| typecheck(resolved, &sources))
        .unwrap_or_else(|d| panic!("{d:#?}"));
    let (mut hir, diagnostics) = super::analyze_unchecked(hir, &sources);
    assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    let signatures = super::signatures(&hir);
    let cx = super::Context {
        signatures: &signatures,
        imported: &hir.imported,
        structs: &hir.structs,
        enums: &hir.enums,
        sources: &sources,
    };
    let mut functions = hir.functions.clone();
    let index = functions
        .iter()
        .position(|f| f.name == "f")
        .expect("a function `f`");
    let mut classified = super::returns::classify(&cx, &mut functions);
    let items = classified.swap_remove(index).places.items;
    hir.functions = functions;
    (items, hir, sources)
}

/// The `deref` of the closure parameter `name` in `function`.
fn deref(function: &HirFunction, name: &str) -> bool {
    match function.locals[local_id(function, name).0 as usize].kind {
        LocalKind::ClosureParam { deref } => deref,
        LocalKind::Plain => panic!("{name} is not a closure's parameter"),
    }
}

#[test]
fn a_source_on_a_value_made_right_there_is_v0001() {
    for (body, at) in [
        ("let a = mk_vs().iter().count();", "mk_vs()"),
        ("let a = mk_m().keys().count();", "mk_m()"),
        ("let a = mk_m().values().count();", "mk_m()"),
        ("let a = mk_s().split(\" \").count();", "mk_s()"),
    ] {
        let (d, sources) = one_error(&with_chains(body));
        assert_eq!(d.code, codes::V0001, "{body}: {d:#?}");
        assert_eq!(d.span, part_of(&sources, body, at), "{body}");
        assert!(d.message.contains("store it with `let` first"), "{d:#?}");
    }
    // A literal lives for the whole program, and a borrowed return is a
    // place of its argument.
    ok(&with_chains(
        "let a = \"x y\".split(\" \").count();\n    let b = p.tags_of().iter().count();\n    let d = p.name_of().split(\" \").count();",
    ));
}

#[test]
fn a_source_decides_its_items() {
    use super::chains::ItemKind;
    let text = with_chains(
        "let a = names.iter().count();\n    let b = v.iter().count();\n    let d = text.split(\" \").count();\n    let e = m.keys().count();\n    let g = m.values().count();\n    let h = p.tags.iter().count();",
    );
    let (items, program, sources) = chain_items(&text);
    let f = function(&program, "f");
    let item = |at: &str| {
        items
            .get(&span_of(&sources, at))
            .cloned()
            .unwrap_or_else(|| panic!("no items at {at}"))
    };
    assert_eq!(
        item("names.iter()"),
        (
            ItemKind::Borrowed(vec![local_id(f, "names")]),
            Some(StringRepr::RefOwned)
        )
    );
    assert_eq!(item("v.iter()"), (ItemKind::Copy, None));
    assert_eq!(
        item("text.split(\" \")"),
        (
            ItemKind::Borrowed(vec![local_id(f, "text")]),
            Some(StringRepr::Str)
        )
    );
    assert_eq!(
        item("m.keys()"),
        (
            ItemKind::Borrowed(vec![local_id(f, "m")]),
            Some(StringRepr::RefOwned)
        )
    );
    assert_eq!(item("m.values()"), (ItemKind::Copy, None));
    assert_eq!(
        item("p.tags.iter()").0,
        ItemKind::Borrowed(vec![local_id(f, "p")])
    );
}

#[test]
fn closure_parameters_take_the_item_as_their_row_hands_it() {
    let text = with_chains(
        "let a = names.iter().filter(|w1| w1.len() > 1).map(|w2| w2).any(|w3| w3.len() > 2);\n    let b = v.iter().filter(|n1| n1 > 1).map(|n2| n2 + 1).all(|n3| n3 > 0);\n    let d = names.iter().map(|s0| s0.clone()).filter(|s1| s1.len() > 0).map(|s2| s2).any(|s3| s3.len() > 0);\n    let e = text.split(\" \").filter(|t1| t1.len() > 0).map(|t2| t2).count();",
    );
    let (_, program, _) = chain_items(&text);
    let f = function(&program, "f");
    let names = local_id(f, "names");
    // Borrowed items: `filter` looks through one more reference.
    for (name, deref_) in [("w1", true), ("w2", false), ("w3", false)] {
        assert_eq!(deref(f, name), deref_, "{name}");
        let info = place(f, name);
        assert!(info.borrowed && !info.mutable, "{name}: {info:?}");
        assert_eq!(info.origin, Some(Origin::Local(names)), "{name}");
    }
    assert_eq!(repr(f, "w1"), Some(StringRepr::RefOwned));
    assert_eq!(repr(f, "w2"), Some(StringRepr::RefOwned));
    // A part `map` gives on is a `&str`.
    assert_eq!(repr(f, "w3"), Some(StringRepr::Str));
    // Copies: `.copied()` makes them plain values, and `filter` still
    // looks through one reference.
    for (name, deref_) in [("n1", true), ("n2", false), ("n3", false)] {
        assert_eq!(deref(f, name), deref_, "{name}");
        assert!(!place(f, name).borrowed, "{name}");
    }
    // Owned items: `filter` looks at one without a root, `map` and `any`
    // own it.
    assert!(!deref(f, "s1"));
    assert!(place(f, "s1").borrowed);
    assert_eq!(place(f, "s1").origin, None);
    assert_eq!(repr(f, "s1"), Some(StringRepr::RefOwned));
    for name in ["s2", "s3"] {
        assert!(!deref(f, name), "{name}");
        assert!(!place(f, name).borrowed, "{name}");
        assert_eq!(repr(f, name), Some(StringRepr::Owned), "{name}");
    }
    // Pieces of text are `&str`.
    assert!(deref(f, "t1"));
    assert_eq!(repr(f, "t1"), Some(StringRepr::Str));
    assert_eq!(repr(f, "t2"), Some(StringRepr::Str));
}

#[test]
fn a_map_returning_part_of_its_item_gives_borrowed_items() {
    use super::chains::ItemKind;
    let text = with_chains(
        "let a = users.iter().map(|u| u.name).count();\n    let b = users.iter().map(|u| u.name.len()).count();\n    let d = users.iter().map(|u| u.name.clone()).count();\n    let e = v.iter().map(|x| other.name).count();\n    let g = users.iter().map(|u| u.name_of()).count();",
    );
    let (items, program, sources) = chain_items(&text);
    let f = function(&program, "f");
    let item = |at: &str| {
        items
            .get(&span_of(&sources, at))
            .cloned()
            .unwrap_or_else(|| panic!("no items at {at}"))
    };
    assert_eq!(
        item("users.iter().map(|u| u.name)"),
        (
            ItemKind::Borrowed(vec![local_id(f, "users")]),
            Some(StringRepr::Str)
        )
    );
    assert_eq!(item("users.iter().map(|u| u.name.len())").0, ItemKind::Copy);
    assert_eq!(
        item("users.iter().map(|u| u.name.clone())"),
        (ItemKind::Owned, Some(StringRepr::Owned))
    );
    assert_eq!(
        item("v.iter().map(|x| other.name)"),
        (
            ItemKind::Borrowed(vec![local_id(f, "other")]),
            Some(StringRepr::Str)
        )
    );
    assert_eq!(
        item("users.iter().map(|u| u.name_of())").0,
        ItemKind::Borrowed(vec![local_id(f, "users")])
    );
}

#[test]
fn a_map_returning_part_of_an_owned_item_is_v0304() {
    let (d, sources) = one_error(&with_chains(
        "let a = users.iter().map(|u| mk_u()).map(|w| w.name).count();",
    ));
    assert_eq!(d.code, codes::V0304, "{d:#?}");
    assert_eq!(d.span, span_of(&sources, "w.name"));
    assert!(
        d.message
            .contains("part of `w`, the closure's parameter, which ends when the closure returns"),
        "{d:#?}"
    );
    assert_eq!(
        d.fix_it.as_ref().map(|f| f.replacement.as_str()),
        Some(".clone()")
    );
}

#[test]
fn a_map_returning_parts_of_two_names_is_v0308() {
    let (d, sources) = one_error(&with_chains(
        "let a = users.iter().map(|x| if c { x.name } else { other.name }).count();",
    ));
    assert_eq!(d.code, codes::V0308, "{d:#?}");
    assert_eq!(d.span, span_of(&sources, "other.name"));
    assert!(
        d.message
            .contains("part of `x` in one place and part of `other` in another"),
        "{d:#?}"
    );
}

#[test]
fn a_find_over_items_made_from_a_capture_is_rooted_at_the_capture() {
    let (d, sources) = one_error(&with_chains(
        "if let Some(n) = v.iter().map(|x| other.name).find(|n| n.len() > 0) {\n        other.name = \"z\";\n        read(n);\n    }",
    ));
    assert_eq!(d.code, codes::V0307, "{d:#?}");
    assert_eq!(
        d.span,
        part_of(&sources, "other.name = \"z\"", "other"),
        "{d:#?}"
    );
    // A capture that is itself another name for an element: its own root.
    let (d, sources) = one_error(&with_chains(
        "let first = users[0];\n    if let Some(n) = v.iter().map(|x| first.name).find(|n| n.len() > 0) {\n        users.push(mk_u());\n        read(n);\n    }",
    ));
    assert_eq!(d.code, codes::V0307, "{d:#?}");
    assert_eq!(d.span, part_of(&sources, "users.push", "users"), "{d:#?}");
    // Unchanged, both are fine.
    ok(&with_chains(
        "let first = users[0];\n    if let Some(n) = v.iter().map(|x| first.name).find(|n| n.len() > 0) {\n        read(n);\n    }\n    users.push(mk_u());",
    ));
}

#[test]
fn collect_of_borrowed_items_is_v0304_with_the_copy_hint() {
    for (body, root) in [
        ("let a = names.iter().collect();", "`names`"),
        ("let a = text.split(\" \").collect();", "`text`"),
        ("let a = users.iter().map(|u| u.name).collect();", "`users`"),
    ] {
        let (d, sources) = one_error(&with_chains(body));
        assert_eq!(d.code, codes::V0304, "{body}: {d:#?}");
        let chain = &body["let a = ".len()..body.len() - ";".len()];
        let receiver = chain.strip_suffix(".collect()").expect("a collect");
        let whole = span_of(&sources, chain);
        let end = whole.start + receiver.len() as u32;
        assert_eq!(d.span, Span::new(FileId(0), end, whole.end), "{body}");
        assert!(d.message.contains(root), "{body}: {d:#?}");
        assert!(d.message.contains(".map(|w| w.clone())"), "{body}: {d:#?}");
        let fix = d.fix_it.as_ref().expect("a fix-it");
        assert_eq!(fix.span, Span::new(FileId(0), end, end));
        assert_eq!(fix.replacement, ".map(|w| w.clone())");
    }
    // Owned items and copies make a `Vec` of their own.
    ok(&with_chains(
        "let a = v.iter().collect();\n    let b = names.iter().map(|w| w.clone()).collect();\n    let d = users.iter().map(|u| u.age).collect();",
    ));
}

#[test]
fn a_find_on_borrowed_items_is_looked_into() {
    let program = ok(&with_chains(
        "if let Some(w) = names.iter().find(|n| n.len() > 0) {\n        read(w);\n    }\n    match text.split(\" \").find(|t| t.len() > 0) {\n        Some(piece) => read(piece),\n        None => {}\n    }\n    let a = v.iter().find(|n| n > 1);\n    let b = names.iter().map(|s| s.clone()).find(|s| s.len() > 0);",
    ));
    let f = function(&program, "f");
    let w = place(f, "w");
    assert!(w.borrowed && !w.mutable, "{w:?}");
    assert_eq!(w.origin, Some(Origin::Local(local_id(f, "names"))));
    assert_eq!(repr(f, "w"), Some(StringRepr::RefOwned));
    assert_eq!(repr(f, "piece"), Some(StringRepr::Str));
    let looked_into = |index: usize| {
        let expr = match &f.body.stmts[index] {
            crate::hir::HirStmt::Let { value, .. } => value,
            crate::hir::HirStmt::Expr { expr, .. } => expr,
            other => panic!("statement {index} is {other:?}"),
        };
        let head = match &expr.kind {
            HirExprKind::IfLet { value, .. } => &**value,
            HirExprKind::Match { scrutinee, .. } => &**scrutinee,
            _ => expr,
        };
        match &head.kind {
            HirExprKind::MethodCall { looked_into, .. } => *looked_into,
            other => panic!("a method call, got {other:?}"),
        }
    };
    let first = 6;
    assert!(looked_into(first));
    assert!(looked_into(first + 1));
    // Copies and owned items make a plain `Option`.
    assert!(!looked_into(first + 2));
    assert!(!looked_into(first + 3));
    assert_eq!(
        f.locals[local_id(f, "b").0 as usize].ty,
        crate::types::Ty::Option(Box::new(crate::types::Ty::String))
    );
}

#[test]
fn a_stored_find_on_borrowed_items_is_v0208() {
    for body in [
        "let x = names.iter().find(|n| n.len() > 0);",
        "let b = names.iter().find(|n| n.len() > 0).is_some();",
    ] {
        let (d, sources) = one_error(&with_chains(body));
        assert_eq!(d.code, codes::V0208, "{body}: {d:#?}");
        assert_eq!(
            d.span,
            span_of(&sources, "names.iter().find(|n| n.len() > 0)"),
            "{body}"
        );
        assert!(d.message.contains("part of `names`"), "{d:#?}");
    }
}

#[test]
fn a_parameter_that_only_looks_at_an_item_cannot_be_kept() {
    for body in [
        "let a = names.iter().filter(|w| {\n        let k = vec![w];\n        true\n    }).count();",
        "let a = names.iter().any(|w| {\n        let k = vec![w];\n        true\n    });",
        "let a = names.iter().map(|s| s.clone()).find(|w| {\n        let k = vec![w];\n        true\n    });",
        "let a = names.iter().map(|s| s.clone()).any(|w| {\n        let k = vec![w];\n        true\n    });",
        "let a = names.iter().map(|s| s.clone()).all(|w| {\n        let k = w;\n        true\n    });",
    ] {
        let (d, sources) = one_error(&with_chains(body));
        assert_eq!(d.code, codes::V0304, "{body}: {d:#?}");
        let at = if body.contains("vec![w]") {
            part_of(&sources, "vec![w]", "w")
        } else {
            part_of(&sources, "let k = w", "w")
        };
        assert_eq!(d.span, at, "{body}: {d:#?}");
    }
    // Read, it is fine.
    ok(&with_chains(
        "let a = names.iter().map(|s| s.clone()).any(|w| {\n        read(w);\n        let k = w.len();\n        w.len() > 0\n    });",
    ));
}

#[test]
fn changing_the_source_inside_a_closure_of_its_chain_is_v0301() {
    let (d, sources) = one_error(&with_chains(
        "let a = v.iter().map(|x| {\n        v.push(1);\n        x\n    }).count();",
    ));
    assert_eq!(d.code, codes::V0301, "{d:#?}");
    assert_eq!(d.span, part_of(&sources, "v.push(1)", "v"));
    assert!(
        d.message
            .contains("inside a closure a name from outside can only be read"),
        "{d:#?}"
    );
}

#[test]
fn a_chain_inside_a_closure_over_a_captured_vec_works() {
    let program = ok(&with_chains(
        "let a = Some(1).map(|x| names.iter().filter(|n| n.len() > 0).count());\n    let b = Some(2).map(|x| users.iter().map(|u| u.name).any(|k| k.len() > 0));\n    names.push(\"c\");",
    ));
    let f = function(&program, "f");
    assert!(deref(f, "n"));
    assert_eq!(
        place(f, "k").origin,
        Some(Origin::Captured(local_id(f, "users")))
    );
}

// --- `for` over a chain (M4 spec 2.3, 3.3) -----------------------------------

#[test]
fn a_for_variable_over_a_chain_is_bound_as_its_items_are() {
    let program = ok(&with_chains(
        "for w in names.iter().filter(|n| n.len() > 0) {\n        read(w);\n    }\n    for x in v.iter() {\n        let y = x + 1;\n    }\n    for s in names.iter().map(|n| n.clone()) {\n        read(s);\n    }\n    for piece in text.split(\" \") {\n        read(piece);\n    }\n    for k in users.iter().map(|u| other.name) {\n        read(k);\n    }",
    ));
    let f = function(&program, "f");
    let names = local_id(f, "names");
    assert_eq!(
        place(f, "w"),
        PlaceInfo {
            borrowed: true,
            mutable: false,
            origin: Some(Origin::Local(names)),
        }
    );
    assert_eq!(repr(f, "w"), Some(StringRepr::RefOwned));
    assert!(!place(f, "x").borrowed);
    assert!(!place(f, "s").borrowed);
    assert_eq!(repr(f, "s"), Some(StringRepr::Owned));
    assert!(place(f, "piece").borrowed);
    assert_eq!(repr(f, "piece"), Some(StringRepr::Str));
    assert_eq!(
        place(f, "k").origin,
        Some(Origin::Local(local_id(f, "other")))
    );
    assert_eq!(repr(f, "k"), Some(StringRepr::Str));
}

#[test]
fn changing_the_source_of_a_for_over_a_chain_is_v0307() {
    let (d, sources) = one_error(&with_chains(
        "for w in names.iter().filter(|n| n.len() > 0) {\n        names.push(\"c\");\n    }",
    ));
    assert_held(
        &d,
        part_of(&sources, "names.push", "names"),
        span_of(&sources, "names.iter().filter(|n| n.len() > 0)"),
        "names",
    );
    assert!(
        d.message
            .contains("this loop goes over `names`, so `names` cannot be changed inside it"),
        "{d:#?}"
    );
    // Over copies, the loop still holds what it goes over.
    let (d, sources) = one_error(&with_chains(
        "for x in v.iter() {\n        v.push(x);\n    }",
    ));
    assert_held(
        &d,
        part_of(&sources, "v.push", "v"),
        span_of(&sources, "v.iter()"),
        "v",
    );
}

/// The body may not change or give away what the head reads besides its
/// source: the source's argument and every capture of the head's
/// closures, numbers included, an alias's own roots too (M4 spec 3.3).
#[test]
fn changing_what_the_head_of_a_for_over_a_chain_reads_is_v0307() {
    for (body, changed, root) in [
        (
            "let mut seen: Vec<string> = vec![];\n    for w in names.iter().filter(|w| seen.contains(w)) {\n        seen.push(w.clone());\n        read(w);\n    }",
            "seen.push",
            "seen",
        ),
        (
            "let mut limit = 1;\n    for n in v.iter().filter(|n| n < limit) {\n        limit = limit + 1;\n    }",
            "limit = limit",
            "limit",
        ),
        (
            "let mut sep = mk_s();\n    for w in text.split(sep) {\n        sep.push_str(w);\n    }",
            "sep.push_str",
            "sep",
        ),
        (
            "let first = names[0];\n    for u in users.iter().filter(|u| u.name == first) {\n        names.push(\"c\");\n    }",
            "names.push",
            "names",
        ),
        (
            "let sep = other.name;\n    for w in text.split(sep) {\n        other.name = \"x\";\n    }",
            "other.name = ",
            "other",
        ),
        (
            "let owner = mk_u();\n    for w in names.iter().filter(|w| owner.age > 0) {\n        let kept = owner;\n        break;\n    }",
            "kept = owner",
            "owner",
        ),
    ] {
        let (d, sources) = errors(&with_chains(body));
        assert_eq!(d.len(), 1, "{body}: {d:#?}");
        let d = &d[0];
        let head_start = body.find(" in ").expect("a for") + " in ".len();
        let head_end = body.find(" {\n        ").expect("a body");
        assert_held(
            d,
            part_of(&sources, changed, root),
            span_of(&sources, &body[head_start..head_end]),
            root,
        );
        assert!(
            d.message.contains(&format!(
                "this loop's head reads `{root}`, so `{root}` cannot be"
            )),
            "{body}: {d:#?}"
        );
    }
}

/// The head's captures hold only while the loop runs: a copy of the loop
/// variable taken out of the loop does not inherit them, and outside a
/// `for` a chain's closures die at its terminal (M4 spec 3.3).
#[test]
fn what_the_head_of_a_for_over_a_chain_reads_is_free_after_the_loop() {
    ok(&with_chains(
        "let mut limit: usize = 1;\n    let mut best = \"\";\n    for w in names.iter().filter(|w| w.len() > limit) {\n        best = w;\n    }\n    limit = 0;\n    read(best);",
    ));
    ok(&with_chains(
        "let mut sep = mk_s();\n    let mut best = \"\";\n    for w in text.split(sep) {\n        best = w;\n    }\n    sep.push_str(\"x\");\n    read(best);",
    ));
    ok(&with_chains(
        "let mut limit = 1;\n    let c = v.iter().filter(|n| n < limit).count();\n    limit = 2;\n    let mut seen: Vec<string> = vec![];\n    let d = names.iter().filter(|w| seen.contains(w)).count();\n    seen.push(\"x\");",
    ));
}

/// A head reading a mutable alias holds its root as the alias does: the
/// body cannot even read it (M4 spec 3.3).
#[test]
fn reading_what_a_mutable_alias_the_head_reads_changes_is_v0307() {
    let (d, sources) = one_error(&with_chains(
        "let mut a = users[0];\n    for w in names.iter().filter(|w| a.age > 0) {\n        let k = users.len();\n    }",
    ));
    assert_held(
        &d,
        part_of(&sources, "users.len()", "users"),
        span_of(&sources, "names.iter().filter(|w| a.age > 0)"),
        "users",
    );
    assert!(
        d.message.contains(
            "this loop's head reads `a`, which can change `users`, so `users` cannot be used inside it"
        ),
        "{d:#?}"
    );
}

#[test]
fn changing_a_vec_of_numbers_inside_a_for_over_it_is_v0307() {
    // The variable is a copy, yet the loop holds the `Vec` until it ends.
    let (d, sources) = one_error(&with_chains("for n in v {\n        v.push(n);\n    }"));
    assert_held(
        &d,
        part_of(&sources, "v.push", "v"),
        part_of(&sources, "in v {", "v"),
        "v",
    );
}

// --- Async functions (milestone 5b1 spec 2.2) -------------------------------

#[test]
fn an_async_function_returning_part_of_a_parameter_is_v0311() {
    let (d, sources) = one_error(
        "struct User {\n    name: string,\n}\nasync fn name(u: User) -> string {\n    u.name\n}\n\
         async fn main() {\n    let u = User { name: \"a\" };\n    let n = name(u).await;\n}\n",
    );
    assert_eq!(d.code, codes::V0311, "{d:#?}");
    assert_eq!(
        d.message,
        "`name` is an async function, so it cannot return part of `u`; return a copy instead, \
         with `.clone()`"
    );
    assert_eq!(d.span, part_of(&sources, "u.name\n", "u.name"));
    assert!(d.fix_it.is_some(), "{d:#?}");
}

#[test]
fn an_awaited_call_gives_a_new_value() {
    let program = ok(
        "struct User {\n    name: string,\n}\nasync fn name(u: User) -> string {\n    u.name.clone()\n}\n\
         async fn main() {\n    let u = User { name: \"a\" };\n    let n = name(u).await;\n    \
         let again = name(u).await;\n    println!(\"{} {}\", n, again);\n}\n",
    );
    assert!(program.functions().all(|f| f.ret_root.is_none()));
}

// --- Started calls (milestone 5b1 spec 3) ----------------------------------

const STARTED_HEAD: &str = "struct User {\n    name: string,\n}\nimpl User {\n    \
    async fn load(self) -> i64 {\n        1\n    }\n    async fn bump(mut self) {\n        \
    self.name = \"b\";\n    }\n}\nasync fn greet(u: User) -> i64 {\n    1\n}\n\
    async fn rename(mut u: User) {\n    u.name = \"b\";\n}\nasync fn shout(s: string) {}\n";

fn started_program(body: &str) -> String {
    format!(
        "{STARTED_HEAD}async fn run(p: User, ps: Vec<User>) {{\n{body}}}\nasync fn main() {{}}\n"
    )
}

#[test]
fn a_borrowed_place_given_to_a_started_call_is_v0304_with_the_task_note() {
    for (body, needle) in [
        ("    let t = greet(p);\n    t.await;\n", "p)"),
        ("    let t = shout(p.name);\n    t.await;\n", "p.name"),
        ("    let t = greet(ps[0]);\n    t.await;\n", "ps[0]"),
        ("    let t = p.load();\n    t.await;\n", "p.load"),
        (
            "    let mut u = User { name: \"a\" };\n    let v = u;\n    let t = greet(v);\n    t.await;\n",
            "",
        ),
    ] {
        let text = started_program(body);
        if needle.is_empty() {
            // An owned local moved into another is still owned: no error.
            let _ = ok(&text);
            continue;
        }
        let (d, sources) = one_error(&text);
        assert_eq!(d.code, codes::V0304, "{text}\n{d:#?}");
        assert_eq!(d.span.start, span_of(&sources, needle).start, "{text}");
        assert!(
            d.notes
                .iter()
                .any(|note| note.contains("may outlive this function")
                    && note.contains(".clone()")
                    && note.contains("Shared")),
            "{d:#?}"
        );
    }
    // An alias of a parameter's element.
    let text = started_program("    let first = ps[0];\n    let t = greet(first);\n    t.await;\n");
    let (diagnostics, _) = errors(&text);
    assert!(
        diagnostics.iter().any(|d| d.code == codes::V0304),
        "{diagnostics:#?}"
    );
}

#[test]
fn an_owned_local_given_to_a_started_call_moves_into_it() {
    let (d, sources) = one_error(&started_program(
        "    let u = User { name: \"a\" };\n    let t = greet(u);\n    println!(\"{}\", u.name);\n    t.await;\n",
    ));
    assert_eq!(d.code, codes::V0305, "{d:#?}");
    let at = span_of(&sources, "u.name);").start;
    assert_eq!((d.span.start, d.span.end), (at, at + 1));
    // A number, a literal, a new value, and a clone are fine.
    ok(&started_program(
        "    let u = User { name: \"a\" };\n    let a = greet(u.clone());\n    let b = shout(\"x\");\n    \
         let c = greet(User { name: \"c\" });\n    let d = time::sleep(5);\n    let e = u.load();\n    \
         a.await;\n    b.await;\n    c.await;\n    d.await;\n    e.await;\n",
    ));
}

#[test]
fn a_started_call_lending_to_a_mut_parameter_is_v0309() {
    for (body, needle) in [
        (
            "    let mut u = User { name: \"a\" };\n    let t = rename(u);\n    t.await;\n",
            "u)",
        ),
        (
            "    let mut u = User { name: \"a\" };\n    let t = u.bump();\n    t.await;\n",
            "u.bump",
        ),
    ] {
        let text = started_program(body);
        let (d, sources) = one_error(&text);
        assert_eq!(d.code, codes::V0309, "{text}\n{d:#?}");
        assert_eq!(d.span.start, span_of(&sources, needle).start, "{text}");
    }
}

#[test]
fn a_task_detached_inside_a_closure_is_v0304() {
    let (d, sources) = one_error(&started_program(
        "    let t = greet(User { name: \"a\" });\n    let v: Vec<i64> = vec![1];\n    \
         let w: Vec<i64> = v.iter().map(|x| {\n        t.detach();\n        x\n    }).collect();\n",
    ));
    assert_eq!(d.code, codes::V0304, "{d:#?}");
    assert_eq!(d.span, part_of(&sources, "t.detach", "t"));
}

#[test]
fn a_task_awaited_twice_is_v0305() {
    let (d, sources) = one_error(&started_program(
        "    let t = greet(User { name: \"a\" });\n    let a = t.await;\n    let b = t.await;\n",
    ));
    assert_eq!(d.code, codes::V0305, "{d:#?}");
    assert_eq!(d.span, part_of(&sources, "b = t.await", "t"));
    let (d, _) = one_error(&started_program(
        "    let t = greet(User { name: \"a\" });\n    t.detach();\n    t.await;\n",
    ));
    assert_eq!(d.code, codes::V0305, "{d:#?}");
}

// --- `Task::all` and `Task::all_settled` (milestone 5b1 spec 3) -----------

#[test]
fn a_list_of_tasks_given_to_task_all_is_taken() {
    let (d, sources) = one_error(&started_program(
        "    let ts = vec![greet(User { name: \"a\" })];\n    let a = Task::all(ts).await;\n    \
         let b = Task::all(ts).await;\n",
    ));
    assert_eq!(d.code, codes::V0305, "{d:#?}");
    assert_eq!(d.span, part_of(&sources, "b = Task::all(ts)", "ts"));
}

#[test]
fn a_collected_map_of_started_calls_owns_what_it_gives_them() {
    // Copied numbers and `.clone()`d values are the tasks' own.
    ok(&started_program(
        "    let ids: Vec<i64> = vec![1, 2];\n    \
         let a = Task::all(ids.iter().map(|id| time::sleep(id as u64)).collect()).await;\n    \
         let b = Task::all(ps.iter().map(|u| greet(u.clone())).collect()).await;\n    \
         let names: Vec<string> = vec![\"a\"];\n    \
         let c = Task::all(names.iter().map(|n| shout(n.clone())).collect()).await;\n",
    ));
    // A borrowed item is not.
    for (body, needle) in [
        (
            "    let names: Vec<string> = vec![\"a\"];\n    \
             let c = Task::all(names.iter().map(|n| shout(n)).collect()).await;\n",
            "n)).",
        ),
        (
            "    let b = Task::all(ps.iter().map(|u| greet(u)).collect()).await;\n",
            "u)).",
        ),
    ] {
        let text = started_program(body);
        let (d, sources) = one_error(&text);
        assert_eq!(d.code, codes::V0304, "{text}\n{d:#?}");
        assert_eq!(d.span.start, span_of(&sources, needle).start, "{text}");
    }
}

// --- `Shared<T>` (milestone 5b1 spec 2.6, 3) -------------------------------

const SHARED_HEAD: &str = "struct Inner {\n    n: i64,\n}\nimpl Inner {\n    fn grow(mut self) {\n        self.n = self.n + 1;\n    }\n}\n\
    struct Config {\n    factor: i64,\n    name: string,\n    items: Vec<i64>,\n    inner: Inner,\n}\n\
    impl Config {\n    fn describe(self) -> string {\n        self.name.clone()\n    }\n    \
    fn bump(mut self) {\n        self.factor = self.factor + 1;\n    }\n}\n\
    fn add(mut v: Vec<i64>) {\n    v.push(1);\n}\n\
    async fn work(c: Shared<Config>) -> i64 {\n    c.factor\n}\n";

/// `body` inside `async fn run(p: Shared<Config>)`, with a `let mut s`
/// sharing a new `Config` in front of it.
fn shared_program(body: &str) -> String {
    format!(
        "{SHARED_HEAD}async fn run(p: Shared<Config>) {{\n    \
         let mut s = Shared::new(Config {{ factor: 1, name: \"a\", items: vec![1], inner: Inner {{ n: 1 }} }});\n\
         {body}}}\nasync fn main() {{}}\n"
    )
}

#[test]
fn changing_something_reached_through_a_shared_is_v0310() {
    for (body, needle) in [
        ("    s.factor = 2;\n", "s.factor"),
        ("    p.factor = 2;\n", "p.factor"),
        ("    s.items[0] = 2;\n", "s.items[0]"),
        ("    s.inner.n = 2;\n", "s.inner.n"),
        ("    add(s.items);\n", "s.items"),
        ("    s.items.push(2);\n", "s.items"),
        ("    p.items.push(2);\n", "p.items"),
        ("    s.bump();\n", "s"),
        ("    s.inner.grow();\n", "s.inner"),
        ("    let mut v = s.items;\n    v.push(2);\n", "v.push"),
    ] {
        let text = shared_program(body);
        let (d, sources) = one_error(&text);
        assert_eq!(d.code, codes::V0310, "{text}\n{d:#?}");
        let at = text.find(body).expect("the body") + body.find(needle).expect("the needle");
        assert_eq!(
            d.span.start as usize,
            at,
            "{text}\n{d:#?}\n{:?}",
            &sources[0].text[d.span.start as usize..d.span.end as usize]
        );
    }
}

#[test]
fn reading_through_a_shared_and_changing_the_handle_are_accepted() {
    ok(&shared_program(
        "    let a = s.factor + p.factor;\n    let b = s.items.len();\n    let c = s.describe();\n    \
         let d = s.name.clone();\n    let e = s.items[0];\n    \
         s = p.clone();\n    let f = s.name;\n",
    ));
}

#[test]
fn moving_a_field_out_of_a_shared_is_v0304() {
    let text =
        shared_program("    let mut names: Vec<string> = Vec::new();\n    names.push(s.name);\n");
    let (d, sources) = one_error(&text);
    assert_eq!(d.code, codes::V0304, "{d:#?}");
    assert_eq!(d.span, span_of(&sources, "s.name"));
}

#[test]
fn shared_new_takes_its_argument() {
    let (d, sources) = one_error(&shared_program(
        "    let c = Config { factor: 1, name: \"b\", items: vec![], inner: Inner { n: 1 } };\n    \
         let t = Shared::new(c);\n    let n = c.factor;\n",
    ));
    assert_eq!(d.code, codes::V0305, "{d:#?}");
    assert_eq!(d.span, part_of(&sources, "n = c.factor", "c"));
}

#[test]
fn a_shared_clone_given_to_a_started_call_is_its_own_handle() {
    ok(&shared_program(
        "    let a = work(s.clone());\n    let b = work(s.clone());\n    let x = a.await + b.await;\n    \
         let c = work(s);\n    let y = c.await;\n",
    ));
    let (d, sources) = one_error(&shared_program(
        "    let a = work(s);\n    let x = a.await;\n    let n = s.factor;\n",
    ));
    assert_eq!(d.code, codes::V0305, "{d:#?}");
    assert_eq!(d.span, part_of(&sources, "n = s.factor", "s"));
    // A parameter is borrowed: the task needs its own handle.
    let (d, _) = one_error(&shared_program(
        "    let a = work(p);\n    let x = a.await;\n",
    ));
    assert_eq!(d.code, codes::V0304, "{d:#?}");
}

// --- `Bytes` (milestone 5c spec 2.3, 7.4) -----------------------------------

const BLOB: &str = "struct Blob {\n    data: Bytes,\n}\n";

fn with_blob(body: &str) -> String {
    format!("{BLOB}{body}{MAIN}")
}

/// Asserts `d` carries the fix-it `.clone()` right after the value at `at`.
fn assert_clone_fix_it(d: &Diagnostic, at: Span) {
    let end = at.end;
    let fix = d.fix_it.as_ref().expect("a `.clone()` fix-it");
    assert_eq!(fix.span, Span::new(FileId(0), end, end), "{d:#?}");
    assert_eq!(fix.replacement, ".clone()");
}

#[test]
fn a_bytes_parameter_is_a_shared_borrow() {
    let program = ok(&with_blob("fn f(b: Bytes) -> usize {\n    b.len()\n}\n"));
    let f = function(&program, "f");
    assert_eq!(
        place(f, "b"),
        PlaceInfo {
            borrowed: true,
            mutable: false,
            origin: Some(Origin::Param(f.params[0].local)),
        }
    );
}

#[test]
fn a_bytes_field_owns_its_value() {
    ok(&with_blob(
        "fn f() -> Blob {\n    let b = Bytes::from_text(\"a\");\n    Blob { data: b }\n}\nfn g(b: Bytes) -> Blob {\n    Blob { data: b.clone() }\n}\n",
    ));
    let (d, sources) = one_error(&with_blob(
        "fn f(b: Bytes) -> Blob {\n    Blob { data: b }\n}\n",
    ));
    assert_v0304(&d, part_of(&sources, "data: b }", "b"), "`b`");
    assert_clone_fix_it(&d, part_of(&sources, "data: b }", "b"));
}

#[test]
fn using_bytes_after_let_moves_them_is_v0305() {
    let (d, sources) = one_error(&with_blob(
        "fn f() -> usize {\n    let b = Bytes::from_text(\"a\");\n    let c = b;\n    b.len()\n}\n",
    ));
    assert_v0305(
        &d,
        part_of(&sources, "b.len()", "b"),
        part_of(&sources, "let c = b", "b"),
        "b",
    );
}

#[test]
fn returning_a_bytes_parameter_or_part_of_one_is_a_borrowed_return() {
    let program = ok(&with_blob(
        "fn same(b: Bytes) -> Bytes {\n    b\n}\nfn inner(blob: Blob) -> Bytes {\n    blob.data\n}\nfn copy(b: Bytes) -> Bytes {\n    b.clone()\n}\n",
    ));
    assert_eq!(ret_root(&program, "same").as_deref(), Some("b"));
    assert_eq!(ret_root(&program, "inner").as_deref(), Some("blob"));
    assert_eq!(ret_root(&program, "copy"), None);

    // Part of a local is gone when the function returns: copy it out.
    let (d, sources) = one_error(&with_blob(
        "fn f() -> Bytes {\n    let blob = Blob { data: Bytes::from_text(\"a\") };\n    blob.data\n}\n",
    ));
    assert_v0304(&d, part_of(&sources, "blob.data\n", "blob.data"), "`blob`");
    assert!(has_note(&d, "with `.clone()`"), "{d:#?}");
    assert_clone_fix_it(&d, part_of(&sources, "blob.data\n", "blob.data"));
}

#[test]
fn collect_of_borrowed_bytes_is_v0304_with_the_copy_hint() {
    let (d, sources) = one_error(&with_blob(
        "fn f() {\n    let blobs = vec![Bytes::from_text(\"a\")];\n    let a = blobs.iter().collect();\n}\n",
    ));
    assert_eq!(d.code, codes::V0304, "{d:#?}");
    assert!(d.message.contains("`blobs`"), "{d:#?}");
    assert!(d.message.contains(".map(|w| w.clone())"), "{d:#?}");
    let end = span_of(&sources, "blobs.iter()").end;
    let fix = d.fix_it.as_ref().expect("a fix-it");
    assert_eq!(fix.span, Span::new(FileId(0), end, end));
    assert_eq!(fix.replacement, ".map(|w| w.clone())");
    ok(&with_blob(
        "fn f() {\n    let blobs = vec![Bytes::from_text(\"a\")];\n    let a = blobs.iter().map(|w| w.clone()).collect();\n}\n",
    ));
}

#[test]
fn an_element_of_a_block_local_bytes_vec_handed_out_says_to_clone() {
    let (d, sources) = one_error(&with_blob(
        "fn read(b: Bytes) {}\nfn f() {\n    read({ let v = vec![Bytes::from_text(\"a\")]; v[0] });\n}\n",
    ));
    assert_eq!(d.code, codes::V0304, "{d:#?}");
    assert_eq!(d.span, span_of(&sources, "v[0]"), "{d:#?}");
    assert!(
        has_note(
            &d,
            "every branch give a new value instead (for text or `Bytes`, with `.clone()`)"
        ),
        "{d:#?}"
    );
}

/// Review Focus 4 (milestone 5c): `Bytes` returned from a borrowed
/// parameter is still borrowed, and a field owns.
#[test]
fn bytes_returned_from_a_borrowed_parameter_into_a_field_is_v0304_saying_to_clone() {
    let same = "fn same(b: Bytes) -> Bytes {\n    b\n}\n";
    let (d, sources) = one_error(&with_blob(&format!(
        "{same}fn wrap(b: Bytes) -> Blob {{\n    Blob {{ data: same(b) }}\n}}\n"
    )));
    assert_v0304(&d, span_of(&sources, "same(b)"), "`b`");
    assert_clone_fix_it(&d, span_of(&sources, "same(b)"));
    ok(&with_blob(&format!(
        "{same}fn wrap(b: Bytes) -> Blob {{\n    Blob {{ data: same(b).clone() }}\n}}\n"
    )));
}

/// Review Focus 4 (milestone 5c): a `Bytes` bound by a `match` on a
/// borrowed imported enum is part of it, and an owned facade parameter
/// keeps what it is given.
#[test]
fn bytes_bound_from_a_borrowed_imported_enum_into_an_owned_parameter_is_v0304_saying_to_clone() {
    let (result, sources) =
        check_path("crates/varyk/tests/fixtures/errors/v0304_bytes_from_a_borrowed_enum/main.vr");
    let diagnostics = result.expect_err("should fail");
    assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    let d = &diagnostics[0];
    assert_v0304(d, part_of(&sources, "keep(data)", "data"), "`message`");
    assert_clone_fix_it(d, part_of(&sources, "keep(data)", "data"));
}

/// Review Focus 5 (milestone 5c): an `if` giving `Some(b)` or `None` as a
/// trailing value gives `b` away, as `Some(name)` does for a `string`: a
/// local used after is V0305, and a borrowed parameter is V0304 with the
/// `.clone()` fix-it.
#[test]
fn bytes_in_some_in_a_trailing_if_is_given_away() {
    let (result, sources) =
        check_path("crates/varyk/tests/fixtures/interop/trailing_if_gives_bytes_away/main.vr");
    let diagnostics = result.expect_err("should fail");
    assert_eq!(diagnostics.len(), 2, "{diagnostics:#?}");
    let lent = &diagnostics[0];
    let at = part_of(&sources, "Some(b) } else { None })\n}", "b");
    assert_v0304(lent, at, "`b`");
    assert_clone_fix_it(lent, at);
    let moved = &diagnostics[1];
    assert_eq!(moved.code, codes::V0305, "{moved:#?}");
    assert_eq!(moved.span, part_of(&sources, "b.len()", "b"), "{moved:#?}");
}

/// Route calls on a `varyk-http` app (milestone 5b4 spec 7.4).
mod routes {
    use crate::diagnostics::{Diagnostic, codes};
    use crate::hir::HirProgram;
    use crate::test_packages::{Dep, varyk_http};

    const HTTP: [(&str, Dep<'static>); 1] = [("http", Dep::Package("varyk-http"))];

    /// A program with a handler `home`, whose `main` is `body`.
    fn check(body: &str) -> Result<HirProgram, Vec<Diagnostic>> {
        let text = format!(
            "struct State {{\n    name: string,\n}}\nasync fn home(state: Shared<State>) {{}}\nfn main() {{\n{body}}}\n"
        );
        varyk_http().check(&text, &HTTP)
    }

    fn one(body: &str) -> Diagnostic {
        match check(body) {
            Ok(_) => panic!("expected a diagnostic for:\n{body}"),
            Err(diagnostics) => {
                assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
                diagnostics[0].clone()
            }
        }
    }

    const NEW: &str = "http::App::new(Shared::new(State { name: \"a\" }))";

    #[test]
    fn a_route_changes_the_app_so_its_local_is_let_mut() {
        let d = one(&format!(
            "    let app = {NEW};\n    app.get(\"/\", home);\n"
        ));
        assert_eq!(d.code, codes::V0302, "{d:#?}");
        assert!(d.message.contains("`app`"), "{d:#?}");
        check(&format!(
            "    let mut app = {NEW};\n    app.get(\"/\", home);\n    app.post(\"/\", home);\n    let r = app;\n"
        ))
        .unwrap_or_else(|diagnostics| panic!("{diagnostics:#?}"));
    }

    #[test]
    fn a_route_uses_the_app_so_it_cannot_follow_a_move() {
        let d = one(&format!(
            "    let mut app = {NEW};\n    let gone = app;\n    app.get(\"/\", home);\n"
        ));
        assert_eq!(d.code, codes::V0305, "{d:#?}");
    }

    #[test]
    fn the_handler_is_not_a_value_and_moves_nothing() {
        // The same handler on two routes, and the state still usable.
        check(
            "    let state = Shared::new(State { name: \"a\" });\n    let mut app = http::App::new(state.clone());\n    app.get(\"/a\", home);\n    app.get(\"/b\", home);\n    println!(\"{}\", state.name);\n",
        )
        .unwrap_or_else(|diagnostics| panic!("{diagnostics:#?}"));
    }

    #[test]
    fn a_hook_changes_the_app_and_cannot_follow_a_move() {
        let hooks = "async fn check(req: http::Request) -> Result<bool, Error> {\n    Ok(true)\n}\nasync fn stamp(req: http::Request, mut res: http::Response) {}\n";
        let text = |body: &str| {
            format!("struct State {{\n    name: string,\n}}\n{hooks}fn main() {{\n{body}}}\n")
        };
        let one = |body: &str| match varyk_http().check(&text(body), &HTTP) {
            Ok(_) => panic!("expected a diagnostic for:\n{body}"),
            Err(diagnostics) => {
                assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
                diagnostics[0].clone()
            }
        };
        let d = one(&format!("    let app = {NEW};\n    app.before(check);\n"));
        assert_eq!(d.code, codes::V0302, "{d:#?}");
        let d = one(&format!(
            "    let mut app = {NEW};\n    let gone = app;\n    app.after(stamp);\n"
        ));
        assert_eq!(d.code, codes::V0305, "{d:#?}");
        varyk_http()
            .check(
                &text(&format!(
                    "    let mut app = {NEW};\n    app.before(check);\n    app.before_on(\"/a\", check);\n    app.after(stamp);\n    let r = app;\n"
                )),
                &HTTP,
            )
            .unwrap_or_else(|diagnostics| panic!("{diagnostics:#?}"));
    }
}
