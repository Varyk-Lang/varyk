use std::path::Path;

use varyk_syntax::{FileId, SourceFile, Span};

use super::typecheck;
use crate::diagnostics::{Diagnostic, codes};
use crate::hir::{
    AssertKind, HirExpr, HirExprKind, HirForHead, HirFunction, HirProgram, HirStmt, MethodRef,
    TryKind, VariantRef,
};
use crate::resolve::{Callee, resolve};
use crate::types::{BUILTIN_TYPE_NAMES, Derives, FloatKind, IntKind, ParamMode, Serde, Ty};

const I32: Ty = Ty::Int(IntKind::I32);
const I64: Ty = Ty::Int(IntKind::I64);

// --- Helpers ----------------------------------------------------------------

fn check_source(entry: SourceFile) -> (Result<HirProgram, Vec<Diagnostic>>, Vec<SourceFile>) {
    let mut sources = Vec::new();
    let result = resolve(entry, &mut sources).and_then(|resolved| typecheck(resolved, &sources));
    (result, sources)
}

/// Resolves and type-checks a single-file program with a dummy path.
fn check_str(text: &str) -> (Result<HirProgram, Vec<Diagnostic>>, Vec<SourceFile>) {
    check_source(SourceFile::new(FileId(0), "dummy/test.vr", text))
}

/// Resolves and type-checks `<rel>`, relative to the workspace root.
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
    (only(&diagnostics).clone(), sources)
}

fn only(diagnostics: &[Diagnostic]) -> &Diagnostic {
    assert_eq!(
        diagnostics.len(),
        1,
        "expected one diagnostic, got {diagnostics:#?}"
    );
    &diagnostics[0]
}

/// The span of the first occurrence of `needle` in file 0.
fn span_of(sources: &[SourceFile], needle: &str) -> Span {
    span_in(sources, 0, needle)
}

fn span_in(sources: &[SourceFile], file: u32, needle: &str) -> Span {
    let text = &sources[file as usize].text;
    let start = text
        .find(needle)
        .unwrap_or_else(|| panic!("{needle:?} not in file"));
    Span::new(FileId(file), start as u32, (start + needle.len()) as u32)
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

fn function<'a>(program: &'a HirProgram, name: &str) -> &'a HirFunction {
    program
        .functions()
        .find(|f| f.name == name)
        .unwrap_or_else(|| panic!("no function {name}"))
}

/// The type of the last local called `name` in `function`.
fn local_ty(function: &HirFunction, name: &str) -> Ty {
    function
        .locals
        .iter()
        .rev()
        .find(|local| local.name == name)
        .unwrap_or_else(|| panic!("no local {name}"))
        .ty
        .clone()
}

/// The value of `function`'s `index`-th statement: a `let` value or an
/// expression statement.
fn stmt_expr(function: &HirFunction, index: usize) -> &HirExpr {
    match &function.body.stmts[index] {
        HirStmt::Let { value, .. } => value,
        HirStmt::Expr { expr, .. } => expr,
        other => panic!("statement {index} is {other:?}"),
    }
}

fn call_args(expr: &HirExpr) -> &[HirExpr] {
    match &expr.kind {
        HirExprKind::Call { args, .. } => args,
        other => panic!("expected a call, got {other:?}"),
    }
}

/// Each of `BUILTIN_TYPE_NAMES` resolves as a type with no declaration.
#[test]
fn every_builtin_type_name_resolves_as_a_type() {
    for name in BUILTIN_TYPE_NAMES {
        let written = match *name {
            "Option" | "Vec" => format!("{name}<i32>"),
            "Result" | "HashMap" => format!("{name}<i32, i32>"),
            // `Shared` holds a struct.
            "Shared" => format!("{name}<S>"),
            _ => name.to_string(),
        };
        ok(&format!(
            "struct S {{\n    n: i32,\n}}\n\nfn f(x: {written}) {{}}\n\nfn main() {{}}\n"
        ));
    }
}

// --- Paths (spec 2.10, 3.1) --------------------------------------------------

/// Every path position (type, call, associated call, struct literal,
/// variant value, variant pattern) accepts a `crate::`/`self::`/`super::`
/// prefix and any depth (spec 3.1).
#[test]
fn paths_through_nested_modules_check_in_every_position() {
    let (result, _) = check_path("crates/varyk/tests/fixtures/resolve/tree/main.vr");
    let program = result.unwrap_or_else(|d| panic!("tree should check: {d:#?}"));
    let main = function(&program, "main");
    let HirExprKind::Call { callee, .. } = &stmt_expr(main, 0).kind else {
        panic!("shop::cart::Cart::new() is a call")
    };
    let Callee::Varyk(new) = callee else {
        panic!("a Varyk callee")
    };
    assert_eq!(program.function(*new).name, "new");
    let HirExprKind::StructLit { id, .. } = &stmt_expr(main, 1).kind else {
        panic!("crate::shop::cart::Cart {{ .. }} is a struct literal")
    };
    assert_eq!(program.structs[id.0 as usize].name, "Cart");
}

#[test]
fn super_in_the_crate_root_is_v0111() {
    let (d, sources) = one_error("fn helper() {}\nfn main() {\n    super::helper();\n}\n");
    assert_eq!(d.code, codes::V0111);
    assert_eq!(d.span, span_of(&sources, "super"));
}

#[test]
fn an_unknown_nested_module_in_a_call_or_pattern_is_v0100_naming_the_path() {
    let (d, _) = one_error("fn main() {\n    shop::cart::Cart::new();\n}\n");
    assert_eq!(d.code, codes::V0100);
    assert!(d.message.contains("shop::cart::Cart"), "{d:#?}");
    let (d, _) = one_error(
        "enum Shape {\n    Point,\n}\nfn main() {\n    match Shape::Point {\n        \
         shop::cart::Kind::Word => {}\n        _ => {}\n    }\n}\n",
    );
    assert_eq!(d.code, codes::V0100);
    assert!(d.message.contains("shop::cart::Kind"), "{d:#?}");
}

// --- Literal typing ---------------------------------------------------------

#[test]
fn unannotated_integer_literal_infers_i32() {
    let program = ok("fn main() {\n    let x = 10;\n}\n");
    let main = function(&program, "main");
    assert_eq!(local_ty(main, "x"), I32);
    let value = stmt_expr(main, 0);
    assert_eq!(value.kind, HirExprKind::Int("10".to_string()));
    assert_eq!(value.ty, I32);
}

#[test]
fn annotation_types_the_literal() {
    let program = ok("fn main() {\n    let x: i64 = 10;\n}\n");
    let main = function(&program, "main");
    assert_eq!(local_ty(main, "x"), I64);
    assert_eq!(stmt_expr(main, 0).ty, I64);
}

#[test]
fn parameter_type_types_the_literal_argument() {
    let program = ok("fn add64(x: i64) {}\nfn main() {\n    add64(10);\n}\n");
    let main = function(&program, "main");
    assert_eq!(call_args(stmt_expr(main, 0))[0].ty, I64);
}

#[test]
fn binary_operator_types_the_literal_by_the_other_operand() {
    let program = ok(
        "fn f(x: i64) -> i64 {\n    x + 1\n}\nfn g(x: i64) -> i64 {\n    1 + x\n}\nfn main() {}\n",
    );
    for name in ["f", "g"] {
        let tail = function(&program, name).body.tail.as_deref().unwrap();
        let HirExprKind::Binary { lhs, rhs, .. } = &tail.kind else {
            panic!("expected a binary, got {tail:?}");
        };
        assert_eq!(
            (&lhs.ty, &rhs.ty, &tail.ty),
            (&I64, &I64, &I64),
            "in {name}"
        );
    }
}

#[test]
fn no_backward_inference_from_later_use() {
    let text = "fn takes_i64(x: i64) {}\nfn main() {\n    let x = 10;\n    takes_i64(x);\n}\n";
    let (d, sources) = one_error(text);
    assert_eq!(d.code, codes::V0200);
    assert_eq!(d.span, part_of(&sources, "takes_i64(x);", "x"));
}

#[test]
fn float_literal_defaults_to_f64_or_takes_its_context() {
    let program = ok("fn main() {\n    let x = 1.5;\n    let y: f32 = 2.5;\n}\n");
    let main = function(&program, "main");
    assert_eq!(local_ty(main, "x"), Ty::Float(FloatKind::F64));
    assert_eq!(local_ty(main, "y"), Ty::Float(FloatKind::F32));
    assert_eq!(
        stmt_expr(main, 0).kind,
        HirExprKind::Float("1.5".to_string())
    );
}

#[test]
fn float_literal_over_f32_max_is_v0200() {
    let text = "fn main() {\n    let x: f32 = 340282360000000000000000000000000000000.0;\n}\n";
    let (d, _) = one_error(text);
    assert_eq!(d.code, codes::V0200);
    assert_eq!(
        d.message,
        "`340282360000000000000000000000000000000.0` does not fit in `f32`"
    );
}

#[test]
fn float_literal_over_f32_max_fits_f64() {
    ok("fn main() {\n    let x: f64 = 340282360000000000000000000000000000000.0;\n}\n");
}

#[test]
fn ordinary_f32_literal_is_accepted() {
    ok("fn main() {\n    let x: f32 = 2.5;\n}\n");
}

#[test]
fn integer_literal_in_float_context_is_v0200() {
    let (d, sources) = one_error("fn main() {\n    let x: f64 = 1;\n}\n");
    assert_eq!(d.code, codes::V0200);
    assert_eq!(d.span, part_of(&sources, "= 1;", "1"));
}

#[test]
fn integer_literal_out_of_range_for_i8_parameter_is_v0200() {
    let (d, sources) = one_error("fn f(x: i8) {}\nfn main() {\n    f(128);\n}\n");
    assert_eq!(d.code, codes::V0200);
    assert_eq!(d.message, "`128` does not fit in `i8` (-128 to 127)");
    assert_eq!(d.span, span_of(&sources, "128"));
}

#[test]
fn integer_literal_at_u8_max_is_accepted() {
    ok("fn main() {\n    let x: u8 = 255;\n}\n");
}

#[test]
fn integer_literal_over_u8_max_is_v0200() {
    let (d, _) = one_error("fn main() {\n    let x: u8 = 256;\n}\n");
    assert_eq!(d.code, codes::V0200);
    assert_eq!(d.message, "`256` does not fit in `u8` (0 to 255)");
}

#[test]
fn large_integer_literal_fits_i64() {
    ok("fn main() {\n    let x: i64 = 9_000_000_000_000;\n}\n");
}

#[test]
fn integer_literal_over_u64_max_is_too_large() {
    let (d, _) = one_error("fn main() {\n    let x: u64 = 18446744073709551616;\n}\n");
    assert_eq!(d.code, codes::V0200);
    assert_eq!(d.message, "integer literal is too large");
}

#[test]
fn negated_literal_may_reach_the_signed_minimum() {
    ok("fn main() {\n    let x: i8 = -128;\n}\n");
    let (d, _) = one_error("fn main() {\n    let z: i8 = -129;\n}\n");
    assert_eq!(d.code, codes::V0200);
    assert_eq!(d.message, "`-129` does not fit in `i8` (-128 to 127)");
}

#[test]
fn double_negated_minimum_is_v0200() {
    let (d, _) = one_error("fn main() {\n    let x: i8 = --128;\n}\n");
    assert_eq!(d.code, codes::V0200);
}

#[test]
fn double_negated_127_is_accepted() {
    ok("fn main() {\n    let x: i8 = --127;\n}\n");
}

#[test]
fn bool_and_string_literals() {
    let program = ok("fn main() {\n    let b = true;\n    let s = \"hi\";\n}\n");
    let main = function(&program, "main");
    assert_eq!(local_ty(main, "b"), Ty::Bool);
    assert_eq!(local_ty(main, "s"), Ty::String);
    assert_eq!(
        stmt_expr(main, 1).kind,
        HirExprKind::String("hi".to_string())
    );
}

#[test]
fn shadowing_let_gets_a_new_local() {
    let program = ok(
        "fn takes(s: string) {}\nfn main() {\n    let x = 1;\n    let x = \"s\";\n    takes(x);\n}\n",
    );
    let main = function(&program, "main");
    assert_eq!(main.locals.len(), 2);
    assert_eq!(main.locals[0].ty, I32);
    assert_eq!(main.locals[1].ty, Ty::String);
}

// --- Operators and conditions -----------------------------------------------

#[test]
fn arithmetic_on_mismatched_numeric_types_is_v0200() {
    let (d, sources) = one_error("fn f(a: i32, b: i64) -> i32 {\n    a + b\n}\nfn main() {}\n");
    assert_eq!(d.code, codes::V0200);
    assert_eq!(d.span, part_of(&sources, "a + b", "b"));
}

#[test]
fn ordering_on_strings_is_v0200() {
    let (d, sources) =
        one_error("fn f(a: string, b: string) -> bool {\n    a < b\n}\nfn main() {}\n");
    assert_eq!(d.code, codes::V0200);
    assert_eq!(d.span, span_of(&sources, "a < b"));
}

#[test]
fn plus_on_strings_is_v0200() {
    let (d, sources) =
        one_error("fn f(a: string, b: string) -> string {\n    a + b\n}\nfn main() {}\n");
    assert_eq!(d.code, codes::V0200);
    assert_eq!(d.span, span_of(&sources, "a + b"));
}

#[test]
fn string_equality_is_bool() {
    let program = ok("fn f(a: string) -> bool {\n    a == \"x\"\n}\nfn main() {}\n");
    assert_eq!(function(&program, "f").body.ty, Ty::Bool);
}

#[test]
fn non_bool_if_and_while_conditions_are_v0200() {
    let (diagnostics, sources) =
        errors("fn main() {\n    if 1 {\n    }\n    while \"s\" {\n    }\n}\n");
    assert_eq!(diagnostics.len(), 2, "{diagnostics:#?}");
    assert_eq!(diagnostics[0].code, codes::V0200);
    assert_eq!(diagnostics[0].span, part_of(&sources, "if 1", "1"));
    assert_eq!(diagnostics[1].code, codes::V0200);
    assert_eq!(diagnostics[1].span, span_of(&sources, "\"s\""));
}

#[test]
fn struct_equality_is_accepted_when_every_field_compares() {
    let program = ok(
        "struct P {\n    x: i32,\n    s: string,\n}\nfn f(a: P, b: P) -> bool {\n    a == b\n}\nfn main() {}\n",
    );
    assert_eq!(
        program.structs[0].derives,
        Derives {
            clone: true,
            eq: true
        }
    );
}

#[test]
fn clone_of_a_struct_is_an_owned_value_of_its_type() {
    let program = ok(
        "struct P {\n    x: i32,\n    s: string,\n}\nfn f(a: P) -> P {\n    a.clone()\n}\nfn main() {}\n",
    );
    let Some(tail) = function(&program, "f").body.tail.as_deref() else {
        panic!("expected a tail");
    };
    let HirExprKind::MethodCall { method, rooted, .. } = &tail.kind else {
        panic!("expected a method call, got {tail:?}");
    };
    assert!(
        matches!(method, MethodRef::Builtin(id) if id.get().name == "clone"),
        "{method:?}"
    );
    assert_eq!(*rooted, None);
    assert!(matches!(tail.ty, Ty::Struct(_)), "{tail:?}");
}

#[test]
fn clone_of_containers_and_enums_is_accepted() {
    for (ty, value) in [
        ("Vec<P>", "Vec::new()"),
        ("Option<P>", "None"),
        ("Result<P, string>", "Err(\"e\")"),
        ("HashMap<string, P>", "HashMap::new()"),
        ("E", "E::A"),
    ] {
        let text = format!(
            "struct P {{\n    x: i32,\n}}\nenum E {{\n    A,\n    B(Vec<E>),\n}}\nfn main() {{\n    let v: {ty} = {value};\n    let w = v.clone();\n    let same = v == w;\n}}\n"
        );
        ok(&text);
    }
}

#[test]
fn clone_of_a_number_or_bool_is_v0100() {
    for (stmt, name) in [
        ("let c = n.clone();", "clone"),
        ("let b = true.clone();", "clone"),
    ] {
        let (d, sources) = one_error(&format!("fn main() {{\n    let n = 3;\n    {stmt}\n}}\n"));
        assert_eq!(d.code, codes::V0100, "{stmt}: {d:#?}");
        assert_eq!(d.span, part_of(&sources, stmt, name), "{stmt}");
        assert!(
            d.message.contains("copied on use; drop `.clone()`"),
            "{d:#?}"
        );
    }
}

#[test]
fn clone_of_a_struct_with_a_rust_field_is_v0203_naming_it() {
    let (result, sources) =
        check_path("crates/varyk/tests/fixtures/errors/v0203_clone_rust_field/main.vr");
    let Err(diagnostics) = result else {
        panic!("expected diagnostics");
    };
    let d = only(&diagnostics);
    assert_eq!(d.code, codes::V0203, "{d:#?}");
    assert_eq!(d.span, span_of(&sources, "session.clone()"));
    assert!(d.message.contains("`Session`"), "{d:#?}");
    assert!(d.message.contains("its field `handle`"), "{d:#?}");
    assert!(
        d.notes
            .iter()
            .any(|n| n.contains("add `Clone` to its `#[derive(..)]` list")),
        "{d:#?}"
    );
}

#[test]
fn equality_of_a_struct_with_a_rust_field_is_v0203_naming_it() {
    let (result, sources) =
        check_path("crates/varyk/tests/fixtures/errors/v0203_compare_blocked/main.vr");
    let Err(diagnostics) = result else {
        panic!("expected diagnostics");
    };
    let d = only(&diagnostics);
    assert_eq!(d.code, codes::V0203, "{d:#?}");
    assert_eq!(d.span, span_of(&sources, "a == b"));
    assert!(d.message.contains("its field `handle`"), "{d:#?}");
    assert!(
        d.notes.iter().any(|n| n.contains("add `PartialEq`")),
        "{d:#?}"
    );
}

#[test]
fn equality_of_enums_and_containers_of_comparable_types_is_accepted() {
    ok(
        "struct User {\n    name: string,\n}\nenum E {\n    A,\n    B(string),\n}\nfn f(a: E, b: E, u: Vec<User>, v: Vec<User>, m: HashMap<string, i32>, n: HashMap<string, i32>, o: Option<User>, p: Option<User>) -> bool {\n    a == b && u != v && m == n && o == p\n}\nfn main() {}\n",
    );
}

#[test]
fn if_branches_must_agree() {
    let (d, sources) = one_error(
        "fn f(c: bool) -> i32 {\n    if c {\n        1\n    } else {\n        true\n    }\n}\nfn main() {}\n",
    );
    assert_eq!(d.code, codes::V0200);
    assert_eq!(d.span, span_of(&sources, "{\n        true\n    }"));
}

#[test]
fn if_as_value_types_by_its_branches() {
    let program = ok(
        "fn f(c: bool) -> i64 {\n    let x = if c { 1 } else { 2 };\n    let y: i64 = if c { 1 } else { 2 };\n    y\n}\nfn main() {}\n",
    );
    let f = function(&program, "f");
    assert_eq!(local_ty(f, "x"), I32);
    assert_eq!(local_ty(f, "y"), I64);
}

// --- Names, fields, calls ---------------------------------------------------

#[test]
fn unknown_name_is_v0100() {
    let (d, sources) = one_error("fn main() {\n    let x = y;\n}\n");
    assert_eq!(d.code, codes::V0100);
    assert_eq!(d.span, part_of(&sources, "= y;", "y"));
}

#[test]
fn unknown_function_is_v0100_at_the_path() {
    let (d, sources) = one_error("fn main() {\n    nope(1);\n}\n");
    assert_eq!(d.code, codes::V0100);
    assert_eq!(d.span, span_of(&sources, "nope"));
}

#[test]
fn unknown_associated_call_matching_another_modules_type_is_v0100_with_a_did_you_mean_note() {
    let (result, _) = check_path(
        "crates/varyk/tests/fixtures/resolve/unknown_associated_call_other_module/main.vr",
    );
    let diagnostics = result.expect_err("should fail");
    let d = only(&diagnostics);
    assert_eq!(d.code, codes::V0100);
    assert_eq!(d.message, "cannot find function `Item::new`");
    assert!(
        d.notes.iter().any(|n| n == "did you mean `m::Item`?"),
        "{d:#?}"
    );
}

#[test]
fn unknown_field_is_v0102() {
    let text = "struct P {\n    x: i32,\n}\nfn f(p: P) -> i32 {\n    p.y\n}\nfn main() {}\n";
    let (d, sources) = one_error(text);
    assert_eq!(d.code, codes::V0102);
    assert_eq!(d.span, part_of(&sources, "p.y", "y"));
}

#[test]
fn field_access_carries_its_name() {
    let text = "struct P {\n    x: i32,\n    y: string,\n}\nfn f(p: P) -> string {\n    p.y\n}\nfn main() {}\n";
    let program = ok(text);
    let tail = function(&program, "f").body.tail.as_deref().unwrap();
    let HirExprKind::Field { name, .. } = &tail.kind else {
        panic!("expected a field, got {tail:?}");
    };
    assert_eq!((name.as_str(), &tail.ty), ("y", &Ty::String));
}

#[test]
fn wrong_argument_count_is_v0201() {
    let (d, sources) = one_error("fn f(a: i32) {}\nfn main() {\n    f(1, 2);\n}\n");
    assert_eq!(d.code, codes::V0201);
    assert_eq!(d.span, span_of(&sources, "f(1, 2)"));
}

#[test]
fn argument_type_mismatch_is_v0200() {
    let (d, sources) = one_error("fn f(a: i32) {}\nfn main() {\n    f(true);\n}\n");
    assert_eq!(d.code, codes::V0200);
    assert_eq!(d.span, span_of(&sources, "true"));
}

#[test]
fn struct_literal_extra_field_is_v0102_and_missing_field_is_v0200() {
    let text = "struct P {\n    x: i32,\n    y: i32,\n}\nfn main() {\n    let a = P { x: 1, y: 2, z: 3 };\n    let b = P { x: 1 };\n}\n";
    let (diagnostics, sources) = errors(text);
    assert_eq!(diagnostics.len(), 2, "{diagnostics:#?}");
    assert_eq!(diagnostics[0].code, codes::V0102);
    assert_eq!(diagnostics[0].span, part_of(&sources, "z: 3", "z"));
    assert_eq!(diagnostics[1].code, codes::V0200);
    assert_eq!(diagnostics[1].span, span_of(&sources, "P { x: 1 }"));
}

#[test]
fn struct_literal_keeps_source_order_with_field_indices() {
    let text =
        "struct P {\n    x: i32,\n    y: i64,\n}\nfn main() {\n    let p = P { y: 2, x: 1 };\n}\n";
    let program = ok(text);
    let value = stmt_expr(function(&program, "main"), 0);
    let HirExprKind::StructLit { fields, .. } = &value.kind else {
        panic!("expected a struct literal, got {value:?}");
    };
    let shape: Vec<(usize, Ty)> = fields.iter().map(|(i, e)| (*i, e.ty.clone())).collect();
    assert_eq!(shape, vec![(1, I64), (0, I32)]);
}

#[test]
fn unknown_struct_in_literal_is_v0101() {
    let (d, sources) = one_error("fn main() {\n    let p = Nope { x: 1 };\n}\n");
    assert_eq!(d.code, codes::V0101);
    assert_eq!(d.span, span_of(&sources, "Nope"));
}

#[test]
fn braces_on_a_variant_with_values_by_position_are_v0201() {
    let text = "enum Shape {\n    Circle(f64),\n}\nfn main() {\n    let s = Shape::Circle { r: 1.0 };\n}\n";
    let (d, sources) = one_error(text);
    assert_eq!(d.code, codes::V0201);
    assert_eq!(
        d.message,
        "`Shape::Circle` holds values by position; write them in parentheses: \
         `Shape::Circle(...)`"
    );
    assert_eq!(d.span, span_of(&sources, "Shape::Circle { r: 1.0 }"));
}

/// A closure is only the argument of a row whose parameter is a closure
/// (M4 spec 2.2): anywhere else it is V0001 naming those calls.
#[test]
fn a_closure_anywhere_but_a_closure_argument_is_v0001_naming_the_calls() {
    let cases = [
        (
            "fn f(n: i32) -> i32 {\n    n\n}\nfn main() {\n    let x = f(|n| n);\n}\n",
            "|n| n",
        ),
        ("fn main() {\n    let f = |n| n;\n}\n", "|n| n"),
        (
            "fn main() {\n    let o: Option<i32> = Some(1);\n    let x = o.unwrap_or(|n| n);\n}\n",
            "|n| n",
        ),
    ];
    for (text, construct) in cases {
        let (d, sources) = one_error(text);
        assert_eq!(d.code, codes::V0001, "{text}");
        assert_eq!(
            d.message,
            "a closure can only be the argument of a call that takes one: `map` on an \
             `Option`, `map_err` on a `Result`, and `map`, `filter`, `any`, `all`, and `find` \
             on a chain",
            "{text}"
        );
        assert_eq!(d.span, span_of(&sources, construct), "{text}");
    }
}

/// The closure's parameter takes its type from the row: the payload of
/// the `Option`, the error of the `Result` (M4 spec 2.2, 2.7).
#[test]
fn a_closure_parameter_is_typed_from_the_row() {
    let program = ok(
        "fn f(r: Result<i32, bool>) -> Result<i32, string> {\n    r.map_err(|e| if e { \"yes\" } else { \"no\" })\n}\nfn main() {\n    let o: Option<string> = Some(\"a\");\n    let n = o.map(|s| s.len());\n}\n",
    );
    let main = function(&program, "main");
    assert_eq!(local_ty(main, "s"), Ty::String);
    assert_eq!(
        local_ty(main, "n"),
        Ty::Option(Box::new(Ty::Int(IntKind::Usize)))
    );
    assert_eq!(local_ty(function(&program, "f"), "e"), Ty::Bool);
}

#[test]
fn option_map_and_map_err_take_their_result_from_the_closure() {
    let program = ok(
        "fn g(r: Result<bool, i32>) {\n    let e = r.map_err(|e| format!(\"{}!\", e));\n}\nfn main() {\n    let d = Some(4).map(|n| n * 2);\n    let o: Option<Option<i64>> = Some(1).map(|n| None);\n}\n",
    );
    let main = function(&program, "main");
    assert_eq!(local_ty(main, "d"), Ty::Option(Box::new(I32)));
    assert_eq!(
        local_ty(main, "o"),
        Ty::Option(Box::new(Ty::Option(Box::new(I64))))
    );
    assert_eq!(
        local_ty(function(&program, "g"), "e"),
        Ty::Result(Box::new(Ty::Bool), Box::new(Ty::String))
    );
    let HirExprKind::MethodCall { args, .. } = &stmt_expr(main, 0).kind else {
        panic!("a method call");
    };
    let HirExprKind::Closure { body, captures, .. } = &args[0].kind else {
        panic!("a closure, got {:?}", args[0].kind);
    };
    assert!(captures.is_empty());
    assert_eq!(body.ty, I32);
}

/// A capture is recorded in every closure it is used from, the parameter
/// of an enclosing closure included; a local of the closure is not one.
#[test]
fn captures_are_the_outer_locals_a_closure_uses() {
    let program = ok(
        "fn main() {\n    let k = 2;\n    let a = Some(1).map(|n| {\n        let m = n + k;\n        Some(m).map(|x| x + n + k)\n    });\n}\n",
    );
    let main = function(&program, "main");
    let id = |name: &str| {
        let index = main
            .locals
            .iter()
            .position(|local| local.name == name)
            .unwrap_or_else(|| panic!("no local {name}"));
        crate::hir::LocalId(index as u32)
    };
    let closure = |expr: &HirExpr| match &expr.kind {
        HirExprKind::MethodCall { args, .. } => match &args[0].kind {
            HirExprKind::Closure {
                param,
                captures,
                body,
                ..
            } => (*param, captures.clone(), body.clone()),
            other => panic!("a closure, got {other:?}"),
        },
        other => panic!("a method call, got {other:?}"),
    };
    let (outer, captures, body) = closure(stmt_expr(main, 1));
    assert_eq!(outer, id("n"));
    assert_eq!(captures, [id("k")]);
    let (inner, captures, _) = closure(body.tail.as_deref().expect("a tail"));
    assert_eq!(inner, id("x"));
    assert_eq!(captures, [id("n"), id("k")]);
    assert_eq!(
        main.locals[id("n").0 as usize].kind,
        crate::hir::LocalKind::ClosureParam { deref: false }
    );
    assert_eq!(
        main.locals[id("k").0 as usize].kind,
        crate::hir::LocalKind::Plain
    );
}

#[test]
fn a_closure_with_other_than_one_parameter_is_v0201() {
    for (closure, count) in [("|| 1", "none"), ("|a, b| a", "2")] {
        let text = format!("fn main() {{\n    let x = Some(1).map({closure});\n}}\n");
        let (d, sources) = one_error(&text);
        assert_eq!(d.code, codes::V0201, "{closure}: {d:#?}");
        assert_eq!(d.span, span_of(&sources, closure), "{closure}");
        assert!(d.message.contains(count), "{closure}: {}", d.message);
        assert!(d.message.contains("`|x| ..`"), "{closure}: {}", d.message);
    }
}

/// A closure body is a value of its own: `return`, `break`, `continue`,
/// and `?` would leave the closure, not the function (M4 spec 2.2).
#[test]
fn leaving_a_closure_early_is_v0001() {
    let cases = [
        (
            "fn main() {\n    while true {\n        let x = Some(1).map(|n| {\n            break;\n        });\n    }\n}\n",
            "break",
        ),
        (
            "fn main() {\n    while true {\n        let x = Some(1).map(|n| {\n            continue;\n        });\n    }\n}\n",
            "continue",
        ),
        (
            "fn f() -> i32 {\n    let x = Some(1).map(|n| {\n        return 2;\n    });\n    1\n}\nfn main() {}\n",
            "return",
        ),
        (
            "fn f() -> Option<i32> {\n    let x = Some(1).map(|n| Some(n)?);\n    x\n}\nfn main() {}\n",
            "Some(n)?",
        ),
    ];
    for (text, at) in cases {
        let (d, sources) = one_error(text);
        assert_eq!(d.code, codes::V0001, "{text}: {d:#?}");
        assert!(
            d.span.start >= span_of(&sources, at).start,
            "{text}: {d:#?}"
        );
        assert!(d.message.contains("closure"), "{text}: {}", d.message);
    }
    // A loop inside the closure is the closure's own.
    ok(
        "fn main() {\n    let x = Some(3).map(|n| {\n        let mut i = 0;\n        while i < n {\n            i = i + 1;\n            if i == 2 {\n                break;\n            }\n        }\n        i\n    });\n}\n",
    );
}

#[test]
fn a_closure_body_with_nothing_to_take_a_type_from_is_v0207() {
    for body in ["None", "Vec::new()", "Ok(n)", "Err(n)"] {
        let text = format!("fn main() {{\n    let x = Some(1).map(|n| {body});\n}}\n");
        let (d, sources) = one_error(&text);
        assert_eq!(d.code, codes::V0207, "{body}: {d:#?}");
        assert_eq!(d.span, span_of(&sources, body), "{body}");
    }
}

#[test]
fn using_underscore_as_a_value_is_v0100() {
    let text = "fn main() {\n    let _ = 1;\n    println!(\"{}\", _);\n}\n";
    let (d, _) = one_error(text);
    assert_eq!(d.code, codes::V0100);
    assert_eq!(d.message, "nothing named `_` exists here");
}

#[test]
fn calling_a_local_that_shadows_a_function_is_v0100() {
    let text = "fn area(w: i32, h: i32) -> i32 {\n    w * h\n}\nfn main() {\n    let area = area(2, 3);\n    let bigger = area(4, 5);\n}\n";
    let (d, sources) = one_error(text);
    assert_eq!(d.code, codes::V0100);
    assert_eq!(d.message, "`area` is a variable here, not a function");
    assert_eq!(d.span, part_of(&sources, "area(4, 5)", "area"));
}

#[test]
fn break_and_continue_outside_a_loop_are_v0002() {
    let (d, sources) = one_error("fn main() {\n    if true {\n        break;\n    }\n}\n");
    assert_eq!(d.code, codes::V0002);
    assert_eq!(
        d.message,
        "`break` is only allowed inside a `while` or `for` loop"
    );
    assert_eq!(d.span, span_of(&sources, "break"));

    let (d, sources) = one_error("fn main() {\n    continue;\n}\n");
    assert_eq!(d.code, codes::V0002);
    assert_eq!(
        d.message,
        "`continue` is only allowed inside a `while` or `for` loop"
    );
    assert_eq!(d.span, span_of(&sources, "continue"));

    ok(
        "fn main() {\n    while true {\n        if true {\n            break;\n        }\n        continue;\n    }\n}\n",
    );
}

// --- Return types -----------------------------------------------------------

#[test]
fn return_type_mismatch_is_v0200() {
    let (d, sources) = one_error("fn f() -> i32 {\n    true\n}\nfn main() {}\n");
    assert_eq!(d.code, codes::V0200);
    assert_eq!(d.span, span_of(&sources, "true"));

    let (d, sources) = one_error("fn f() -> i32 {\n    return true;\n}\nfn main() {}\n");
    assert_eq!(d.code, codes::V0200);
    assert_eq!(d.span, span_of(&sources, "true"));
}

#[test]
fn missing_tail_expression_is_v0200_at_the_closing_brace() {
    let (d, sources) = one_error("fn f() -> i32 {\n    let x = 1;\n}\nfn main() {}\n");
    assert_eq!(d.code, codes::V0200);
    let brace = span_of(&sources, "}\nfn main").start;
    assert_eq!(d.span, Span::new(FileId(0), brace, brace + 1));
}

#[test]
fn returning_on_every_path_needs_no_tail() {
    ok(
        "fn f(c: bool) -> i32 {\n    if c {\n        return 1;\n    }\n    return 2;\n}\nfn main() {}\n",
    );
    ok(
        "fn g(c: bool) -> i32 {\n    if c {\n        return 1;\n    } else {\n        return 2;\n    }\n}\nfn main() {}\n",
    );
}

#[test]
fn errors_in_several_functions_are_reported_together() {
    let (diagnostics, _) = errors("fn f() -> i32 {\n    true\n}\nfn main() {\n    let x = y;\n}\n");
    let found: Vec<&str> = diagnostics.iter().map(|d| d.code).collect();
    assert_eq!(found, vec![codes::V0200, codes::V0100]);
}

#[test]
fn assignment_type_mismatch_is_v0200() {
    let (d, sources) = one_error("fn main() {\n    let mut x = 1;\n    x = \"s\";\n}\n");
    assert_eq!(d.code, codes::V0200);
    assert_eq!(d.span, span_of(&sources, "\"s\""));
}

// --- println! ---------------------------------------------------------------

#[test]
fn println_placeholder_count_mismatch_is_v0202() {
    let (d, sources) = one_error("fn main() {\n    println!(\"{} {}\", 1);\n}\n");
    assert_eq!(d.code, codes::V0202);
    assert_eq!(d.span, span_of(&sources, "println!(\"{} {}\", 1)"));
}

#[test]
fn escaped_braces_count_as_zero_placeholders() {
    ok("fn main() {\n    println!(\"{{}}\");\n}\n");
    let (d, _) = one_error("fn main() {\n    println!(\"{{}}\", 1);\n}\n");
    assert_eq!(d.code, codes::V0202);
}

#[test]
fn unsupported_placeholders_are_v0202() {
    for placeholder in ["{:?}", "{0}"] {
        let text = format!("fn main() {{\n    println!(\"a {placeholder}\", 1);\n}}\n");
        let (d, sources) = one_error(&text);
        assert_eq!(d.code, codes::V0202, "for {placeholder}");
        assert_eq!(d.span, span_of(&sources, placeholder), "for {placeholder}");
    }
}

#[test]
fn println_of_a_struct_is_v0203() {
    let text =
        "struct P {\n    x: i32,\n}\nfn f(p: P) {\n    println!(\"{}\", p);\n}\nfn main() {}\n";
    let (d, sources) = one_error(text);
    assert_eq!(d.code, codes::V0203);
    assert_eq!(d.span, part_of(&sources, ", p)", "p"));
}

// --- Imported functions -----------------------------------------------------

#[test]
fn calling_a_not_callable_import_is_v0108_with_the_signature() {
    let (result, sources) = check_path("crates/varyk/tests/fixtures/interop/not_callable/main.vr");
    let diagnostics = result.expect_err("should fail");
    assert_eq!(diagnostics.len(), 3, "{diagnostics:#?}");

    let by_ref = &diagnostics[0];
    assert_eq!(by_ref.code, codes::V0108);
    assert_eq!(by_ref.span, span_of(&sources, "ext::by_ref_string(s)"));
    assert!(
        by_ref
            .notes
            .iter()
            .any(|n| n.contains("`fn by_ref_string(s: &String) -> usize`")),
        "{by_ref:#?}"
    );
    assert!(
        by_ref.notes.iter().any(|n| n.contains("&str")),
        "{by_ref:#?}"
    );

    let generic = &diagnostics[1];
    assert_eq!(generic.code, codes::V0108);
    assert_eq!(generic.span, span_of(&sources, "ext::generic(1)"));
    assert!(
        generic
            .notes
            .iter()
            .any(|n| n.contains("`fn generic<T>(value: T) -> T`")),
        "{generic:#?}"
    );
    assert!(!generic.notes.iter().any(|n| n.contains("&str")));

    // A `-> &String` return gets no `&str` hint: only parameters do.
    let ref_return = &diagnostics[2];
    assert_eq!(ref_return.code, codes::V0108);
    assert_eq!(ref_return.span, span_of(&sources, "ext::ref_return(s)"));
    assert!(
        !ref_return.notes.iter().any(|n| n.contains("take `&str`")),
        "{ref_return:#?}"
    );
}

#[test]
fn calling_a_callable_import_checks_against_the_mapped_types() {
    let (result, _) = check_path("crates/varyk/tests/fixtures/interop/callable/main.vr");
    let program = result.expect("should type-check");
    let main = function(&program, "main");
    let call = stmt_expr(main, 1);
    assert!(matches!(
        call.kind,
        HirExprKind::Call {
            callee: Callee::Imported(_),
            ..
        }
    ));
    assert_eq!(call.ty, Ty::Int(IntKind::U8));
    let tys: Vec<Ty> = call_args(call).iter().map(|a| a.ty.clone()).collect();
    assert_eq!(tys, vec![I64, I32, Ty::String, Ty::String]);
    assert_eq!(local_ty(main, "r"), Ty::Int(IntKind::U8));
}

/// An imported async function or method is called as a Varyk one is
/// (milestone 5b1 spec 2.8): awaited, or started into a `Task`.
#[test]
fn an_imported_async_function_or_method_is_awaited_or_started() {
    let (result, _) = check_path("crates/varyk/tests/fixtures/interop/async_calls/main.vr");
    let program = result.expect("should type-check");
    let main = function(&program, "main");
    assert_eq!(local_ty(main, "a"), Ty::String);
    assert_eq!(local_ty(main, "t"), Ty::Task(Box::new(Ty::String)));
    assert_eq!(local_ty(main, "b"), Ty::String);
    assert_eq!(local_ty(main, "n"), I64);
    assert_eq!(local_ty(main, "u"), Ty::Task(Box::new(I64)));
    assert_eq!(local_ty(main, "m"), I64);
    assert!(program.uses_std);
}

#[test]
fn an_imported_async_function_follows_the_async_call_rules() {
    let (result, sources) = check_path("crates/varyk/tests/fixtures/interop/async_misuse/main.vr");
    let diagnostics = result.expect_err("should fail");
    let found: Vec<(&str, Span)> = diagnostics.iter().map(|d| (d.code, d.span)).collect();
    assert_eq!(
        found,
        vec![
            (codes::V0211, span_of(&sources, "ext::fetch(1)")),
            (codes::V0213, span_of(&sources, "ext::fetch(2)")),
            (codes::V0108, span_of(&sources, "ext::first(\"x\")")),
        ],
        "{diagnostics:#?}"
    );
    assert!(
        diagnostics[2]
            .notes
            .iter()
            .any(|n| n.contains("an async Rust function that returns a reference")),
        "{:#?}",
        diagnostics[2]
    );
}

#[test]
fn imported_argument_and_return_mismatches_are_v0200() {
    let (result, sources) =
        check_path("crates/varyk/tests/fixtures/interop/callable_mismatch/main.vr");
    let diagnostics = result.expect_err("should fail");
    assert_eq!(diagnostics.len(), 2, "{diagnostics:#?}");
    assert_eq!(diagnostics[0].code, codes::V0200);
    assert_eq!(diagnostics[0].span, span_of(&sources, "true"));
    assert_eq!(diagnostics[1].code, codes::V0200);
    assert_eq!(
        diagnostics[1].span,
        span_of(&sources, "ext::mix(10, 2, s, \"x\")")
    );
}

/// The fixed and trailing arguments' types of the call `expr`, a free
/// or method call.
fn split_tys(expr: &HirExpr) -> (Vec<Ty>, Vec<Ty>) {
    let (args, trailing) = match &expr.kind {
        HirExprKind::Call { args, trailing, .. }
        | HirExprKind::MethodCall { args, trailing, .. } => (args, trailing),
        other => panic!("expected a call, got {other:?}"),
    };
    let tys = |exprs: &[HirExpr]| exprs.iter().map(|e| e.ty.clone()).collect();
    (tys(args), tys(trailing))
}

/// Milestone 5b3 spec 2.2: zero or more trailing values of the admitted
/// types after the fixed arguments, from a free, method, and associated
/// call; a place as a value checks.
#[test]
fn trailing_values_of_each_admitted_type_check() {
    let (result, _) = check_path("crates/varyk/tests/fixtures/interop/trailing_values/main.vr");
    let program = result.expect("should type-check");
    assert!(program.uses_std);
    let main = function(&program, "main");
    let opt = |ty: Ty| Ty::Option(Box::new(ty));
    let ints = [
        IntKind::I8,
        IntKind::I16,
        IntKind::I32,
        IntKind::I64,
        IntKind::U8,
        IntKind::U16,
        IntKind::U32,
    ];
    let mut scalars = vec![
        Ty::Bool,
        Ty::String,
        Ty::Float(FloatKind::F32),
        Ty::Float(FloatKind::F64),
    ];
    scalars.extend(ints.iter().map(|kind| Ty::Int(*kind)));
    let options: Vec<Ty> = scalars.iter().cloned().map(opt).collect();
    let fixed = vec![Ty::String];
    assert_eq!(split_tys(stmt_expr(main, 0)), (fixed.clone(), vec![]));
    assert_eq!(split_tys(stmt_expr(main, 1)), (fixed.clone(), vec![I32]));
    assert_eq!(
        split_tys(stmt_expr(main, 2)),
        (
            fixed.clone(),
            vec![Ty::Bool, Ty::String, Ty::Float(FloatKind::F64)]
        )
    );
    assert_eq!(split_tys(stmt_expr(main, 14)), (fixed.clone(), scalars));
    assert_eq!(split_tys(stmt_expr(main, 26)), (fixed.clone(), options));
    assert_eq!(
        split_tys(stmt_expr(main, 29)),
        (fixed, vec![I64, Ty::String, opt(Ty::String), Ty::String])
    );
    assert_eq!(
        split_tys(stmt_expr(main, 32)),
        (vec![I64], vec![I64, Ty::String])
    );
    assert_eq!(
        split_tys(stmt_expr(main, 33)),
        (vec![], vec![I32, I32, I32])
    );
    // `Some(x)` is passed as `x` (the same value), so `x` is only read.
    assert_eq!(
        split_tys(stmt_expr(main, 34)),
        (vec![Ty::String], vec![Ty::String, Ty::String, I64])
    );
}

/// Milestone 5c spec 2.4: a `Time`, a `Uuid`, and a `Bytes`, and an
/// `Option` of each, are trailing values; a `Bytes` is lent, so the local
/// and the parameters passed stay usable.
#[test]
fn time_uuid_and_bytes_are_trailing_values() {
    let (result, sources) =
        check_path("crates/varyk/tests/fixtures/interop/trailing_std_types/main.vr");
    let program = result.unwrap_or_else(|diagnostics| panic!("{diagnostics:#?}"));
    let opt = |ty: Ty| Ty::Option(Box::new(ty));
    let main = function(&program, "main");
    assert_eq!(
        split_tys(stmt_expr(main, 6)),
        (
            vec![Ty::String],
            vec![
                Ty::Time,
                Ty::Uuid,
                Ty::Bytes,
                opt(Ty::Time),
                opt(Ty::Uuid),
                opt(Ty::Bytes)
            ]
        )
    );
    // `Some(x)` is passed as `x`.
    assert_eq!(
        split_tys(stmt_expr(main, 7)),
        (vec![Ty::String], vec![Ty::Time, Ty::Uuid, Ty::Bytes])
    );
    let pass = function(&program, "pass");
    let tail = pass.body.tail.as_deref().expect("a tail");
    assert_eq!(
        split_tys(tail),
        (vec![Ty::String], vec![Ty::Bytes, opt(Ty::Bytes)])
    );
    if let Err(diagnostics) = crate::borrow::analyze(program, &sources) {
        panic!("{diagnostics:#?}");
    }
}

/// A struct, a `Vec`, a `HashMap`, and a `u64` are V0218 listing the
/// admitted types; a bare `None` is V0207; too few fixed arguments is
/// V0201 saying "at least".
#[test]
fn trailing_values_of_other_types_are_refused() {
    let (result, sources) =
        check_path("crates/varyk/tests/fixtures/interop/trailing_values_refused/main.vr");
    let diagnostics = result.expect_err("should fail");
    let found: Vec<(&str, Span)> = diagnostics.iter().map(|d| (d.code, d.span)).collect();
    assert_eq!(
        found,
        vec![
            (codes::V0218, part_of(&sources, "\"q\", user)", "user")),
            (codes::V0218, part_of(&sources, "\"q\", list)", "list")),
            (codes::V0218, part_of(&sources, "\"q\", counts)", "counts")),
            (codes::V0218, part_of(&sources, "\"q\", big)", "big")),
            (
                codes::V0218,
                part_of(&sources, "\"q\", big as u64)", "big as u64")
            ),
            (
                codes::V0218,
                part_of(
                    &sources,
                    "\"q\", Some(user.name.len()))",
                    "Some(user.name.len())"
                )
            ),
            (codes::V0207, part_of(&sources, "\"q\", None)", "None")),
            (codes::V0201, span_of(&sources, "ext::run()")),
            (codes::V0218, part_of(&sources, "\"q\", blobs)", "blobs")),
        ],
        "{diagnostics:#?}"
    );
    for diagnostic in diagnostics[..6].iter().chain(&diagnostics[8..]) {
        assert!(
            diagnostic.notes.iter().any(|n| n.contains(
                "`bool`, `string`, `f32`, `f64`, `i8`, `i16`, `i32`, `i64`, `u8`, `u16`, `u32`, \
                 `Time`, `Uuid`, `Bytes`, or an `Option` of one of those"
            )),
            "{diagnostic:#?}"
        );
    }
    assert!(
        diagnostics[8].message.contains("`Vec<Bytes>`"),
        "{:#?}",
        diagnostics[8]
    );
    assert!(
        diagnostics[3].notes.iter().any(|n| n.contains("as i64")),
        "{:#?}",
        diagnostics[3]
    );
    assert!(diagnostics[3].fix_it.is_some(), "{:#?}", diagnostics[3]);
    // No fix-it that would read `big as u64 as i64` or `Some(..) as i64`.
    assert!(diagnostics[4].fix_it.is_none(), "{:#?}", diagnostics[4]);
    assert!(diagnostics[5].fix_it.is_none(), "{:#?}", diagnostics[5]);
    assert!(
        diagnostics[5].message.contains("`Option<usize>`"),
        "{:#?}",
        diagnostics[5]
    );
    assert!(
        diagnostics[7].message.contains("at least 1 argument"),
        "{:#?}",
        diagnostics[7]
    );
}

/// The type filled in for the type parameter of the call `expr`, through
/// `?` and `.await`.
fn type_arg(expr: &HirExpr) -> Option<Ty> {
    match &expr.kind {
        HirExprKind::Call { type_arg, .. } | HirExprKind::MethodCall { type_arg, .. } => {
            type_arg.clone()
        }
        HirExprKind::Await(inner) => type_arg(inner),
        HirExprKind::Try { operand, .. } => type_arg(operand),
        other => panic!("expected a call, got {other:?}"),
    }
}

/// Milestone 5b3 spec 2.1: `T` is the type the result is used as, from a
/// `let` with a type, a parameter, a return (a tail and an explicit
/// `return`), and a struct literal's field, through `?` and `.await`; a
/// number, `string`, `Option`, `Vec`, `HashMap`, struct, or enum; from a
/// method, a free, and an associated call. The types it reaches are read
/// by serde.
#[test]
fn a_type_parameter_is_filled_from_where_the_result_goes() {
    let (result, _) = check_path("crates/varyk/tests/fixtures/interop/typed_results/main.vr");
    let program = result.expect("should type-check");
    assert!(program.uses_std);
    let user = Ty::Struct(crate::resolve::StructId(0));
    let role = Ty::Enum(crate::resolve::EnumId(0));
    let load = function(&program, "load");
    let map = Ty::HashMap(Box::new(Ty::String), Box::new(I64));
    let filled: Vec<Option<Ty>> = (0..7).map(|at| type_arg(stmt_expr(load, at))).collect();
    assert_eq!(
        filled,
        vec![
            Some(I64),
            Some(Ty::String),
            Some(Ty::Option(Box::new(I64))),
            Some(user.clone()),
            Some(map.clone()),
            Some(role.clone()),
            Some(user.clone()),
        ]
    );
    assert_eq!(local_ty(load, "maybe"), Ty::Option(Box::new(I64)));
    assert_eq!(local_ty(load, "users"), Ty::Vec(Box::new(user.clone())));
    assert_eq!(local_ty(load, "counts"), map);
    assert_eq!(local_ty(load, "user"), Ty::Option(Box::new(user.clone())));
    let awaited = Ty::Result(Box::new(user.clone()), Box::new(Ty::Error));
    assert_eq!(local_ty(load, "awaited"), awaited);
    // The parameter of `show`, and the field of `Holder`.
    let HirExprKind::Call { args, .. } = &stmt_expr(load, 7).kind else {
        panic!("a call of show");
    };
    assert_eq!(type_arg(&args[0]), Some(user.clone()));
    let HirExprKind::StructLit { fields, .. } = &stmt_expr(load, 8).kind else {
        panic!("a struct literal");
    };
    assert_eq!(type_arg(&fields[0].1), Some(user.clone()));
    // A tail `.await` and an explicit `return`.
    let tail = function(&program, "tail");
    let Some(tail_expr) = &tail.body.tail else {
        panic!("a tail");
    };
    assert_eq!(type_arg(tail_expr), Some(user.clone()));
    let early = function(&program, "early");
    let early_text = format!("{:?}", early.body);
    assert!(
        early_text.contains("type_arg: Some(Struct(StructId(0)))"),
        "{early_text}"
    );
    assert_eq!(
        program.structs[0].serde,
        Serde {
            serialize: false,
            deserialize: true
        }
    );
    assert_eq!(
        program.enums[0].serde,
        Serde {
            serialize: false,
            deserialize: true
        }
    );
}

/// Milestone 5b3 spec 2.1: with nothing expected, or started, the call
/// is V0207 (the started one with a note); a `Result` of another shape
/// is V0200; a struct from a `.rs` module is V0210; a read struct's
/// attributes are checked (V0209).
#[test]
fn a_type_parameter_with_nowhere_to_come_from_is_refused() {
    let (result, sources) =
        check_path("crates/varyk/tests/fixtures/interop/typed_results_refused/main.vr");
    let diagnostics = result.expect_err("should fail");
    let found: Vec<(&str, Span)> = diagnostics.iter().map(|d| (d.code, d.span)).collect();
    assert_eq!(
        found,
        vec![
            (codes::V0207, span_of(&sources, "db.one(\"1\")")),
            (codes::V0207, span_of(&sources, "db.first(\"2\")")),
            (codes::V0207, span_of(&sources, "ext::fetch(\"3\")")),
            (codes::V0200, span_of(&sources, "db.first(\"4\")")),
            (codes::V0210, span_of(&sources, "ext::read(\"5\")")),
            (codes::V0209, span_of(&sources, "secret: string")),
        ],
        "{diagnostics:#?}"
    );
    assert!(
        diagnostics[0].message.contains(
            "the type `db.one` reads cannot be worked out here; write the type, as in \
                       `let u: User = db.one(..).await?;`"
        ),
        "{:#?}",
        diagnostics[0]
    );
    let started = "a call that takes its type from where its result goes cannot be started; \
                   add `.await`";
    assert!(
        diagnostics[2].notes.iter().any(|n| n == started),
        "{:#?}",
        diagnostics[2]
    );
    assert!(!diagnostics[0].notes.iter().any(|n| n == started));
}

/// Milestone 5b4 spec 2.7: the argument of a `&T` parameter with `T:
/// Serialize + ?Sized` is any type `json::stringify` writes, typed from
/// itself: a struct, a `Vec` of structs, a `string` local and literal, an
/// `i64`, and an `Option`, from a free call and a method. A struct it
/// reaches derives `Serialize`, and it is lent, so each local is usable
/// after the call.
#[test]
fn a_serialize_parameter_takes_any_type_json_writes() {
    let (result, sources) =
        check_path("crates/varyk/tests/fixtures/interop/serialize_args/main.vr");
    let program = result.expect("should type-check");
    assert!(program.uses_std);
    let user = Ty::Struct(crate::resolve::StructId(0));
    let main = function(&program, "main");
    let args: Vec<(Vec<Ty>, Vec<Ty>)> = (5..=10).map(|at| split_tys(stmt_expr(main, at))).collect();
    let one = |ty: Ty| (vec![ty], Vec::new());
    assert_eq!(
        args,
        vec![
            one(user.clone()),
            one(Ty::Vec(Box::new(user.clone()))),
            one(Ty::String),
            one(Ty::String),
            one(I64),
            one(Ty::Option(Box::new(Ty::String))),
        ]
    );
    assert_eq!(
        split_tys(stmt_expr(main, 12)),
        (vec![Ty::String, user], Vec::new())
    );
    assert_eq!(local_ty(main, "a"), Ty::String);
    assert_eq!(local_ty(main, "g"), Ty::Int(IntKind::Usize));
    assert_eq!(
        program.structs[0].serde,
        Serde {
            serialize: true,
            deserialize: false
        }
    );
    if let Err(diagnostics) = crate::borrow::analyze(program, &sources) {
        panic!("each local stays usable after the call: {diagnostics:#?}");
    }
}

/// Milestone 5c spec 2.5: a type parameter bounded by `DeserializeOwned`
/// or `Serialize` reaches a struct holding `Time`, `Uuid`, and `Bytes`
/// with no change, since they pass the JSON check.
#[test]
fn a_serde_type_parameter_reaches_time_uuid_and_bytes() {
    let (result, sources) =
        check_path("crates/varyk/tests/fixtures/interop/serde_std_types/main.vr");
    let program = result.unwrap_or_else(|diagnostics| panic!("{diagnostics:#?}"));
    assert!(program.uses_std);
    let record = Ty::Struct(crate::resolve::StructId(0));
    let main = function(&program, "main");
    assert_eq!(
        local_ty(main, "read"),
        Ty::Result(Box::new(record), Box::new(Ty::Error))
    );
    assert_eq!(
        program.structs[0].serde,
        Serde {
            serialize: true,
            deserialize: true
        }
    );
    if let Err(diagnostics) = crate::borrow::analyze(program, &sources) {
        panic!("{diagnostics:#?}");
    }
}

/// Milestone 5b4 spec 2.7: a Rust type from a `.rs` module cannot be the
/// argument of a `Serialize` parameter (V0210, at the argument).
#[test]
fn a_rust_type_for_a_serialize_parameter_is_v0210() {
    let (result, sources) =
        check_path("crates/varyk/tests/fixtures/errors/v0210_serialize_rust_type/main.vr");
    let diagnostics = result.expect_err("should fail");
    let found: Vec<(&str, Span)> = diagnostics.iter().map(|d| (d.code, d.span)).collect();
    assert_eq!(
        found,
        vec![(
            codes::V0210,
            part_of(&sources, "ext::json(handle)", "handle")
        )],
        "{diagnostics:#?}"
    );
    assert_eq!(
        diagnostics[0].message,
        "`Handle` cannot be turned into JSON"
    );
}

// --- Examples and modes -----------------------------------------------------

fn modes(function: &HirFunction) -> Vec<ParamMode> {
    function.params.iter().map(|p| p.mode).collect()
}

#[test]
fn parameter_modes_of_the_examples() {
    let (result, _) = check_path("examples/structs.vr");
    let program = result.unwrap();
    assert_eq!(
        modes(function(&program, "print_user")),
        vec![ParamMode::SharedBorrow]
    );

    let (result, _) = check_path("examples/borrowing.vr");
    let program = result.unwrap();
    assert_eq!(
        modes(function(&program, "rename")),
        vec![ParamMode::MutableBorrow]
    );
    assert_eq!(
        modes(function(&program, "print_user")),
        vec![ParamMode::SharedBorrow]
    );

    let (result, _) = check_path("examples/functions.vr");
    let program = result.unwrap();
    let add = function(&program, "add");
    assert_eq!(modes(add), vec![ParamMode::Owned, ParamMode::Owned]);
    assert_eq!(add.params[0].local.0, 0);
    assert_eq!(add.locals[1].name, "b");
}

// --- Enums, methods, and module types -----------------------------------------

const COUNTER: &str = "struct Counter {\n    count: i32,\n}\n";

#[test]
fn a_mut_self_method_body_assigning_a_field_types() {
    let program = ok(&format!(
        "{COUNTER}impl Counter {{\n    fn add(mut self, by: i32) {{\n        self.count = self.count + by;\n    }}\n\n    fn value(self) -> i32 {{\n        self.count\n    }}\n}}\nfn main() {{}}\n"
    ));
    let counter = Ty::Struct(crate::resolve::StructId(0));
    for (name, mode) in [
        ("add", ParamMode::MutableBorrow),
        ("value", ParamMode::SharedBorrow),
    ] {
        let method = function(&program, name);
        assert!(method.owner.is_some());
        let receiver = &method.params[0];
        assert_eq!(
            (
                receiver.local.0,
                receiver.name.as_str(),
                &receiver.ty,
                receiver.mode
            ),
            (0, "self", &counter, mode)
        );
        assert_eq!(method.locals[0].mutable, mode == ParamMode::MutableBorrow);
    }
    let add = function(&program, "add");
    assert_eq!(add.params[1].name, "by");
    assert_eq!(add.params[1].local.0, 1);
    assert_eq!(function(&program, "value").body.ty, I32);
}

#[test]
fn self_outside_a_method_is_v0100() {
    for text in [
        "fn f() -> i32 {\n    self.count\n}\nfn main() {}\n".to_string(),
        format!(
            "{COUNTER}impl Counter {{\n    fn new() -> i32 {{\n        self.count\n    }}\n}}\nfn main() {{}}\n"
        ),
    ] {
        let (d, sources) = one_error(&text);
        assert_eq!(d.code, codes::V0100);
        assert_eq!(d.span, part_of(&sources, "self.count", "self"));
        assert_eq!(d.message, "`self` is only available inside a method");
    }
}

#[test]
fn reserved_parameter_and_let_names_are_v0103() {
    let (diagnostics, sources) =
        errors("fn f(Some: i32) {\n    let None = 1;\n    let mut Vec = 2;\n}\nfn main() {}\n");
    let spans: Vec<Span> = diagnostics.iter().map(|d| d.span).collect();
    assert_eq!(
        spans,
        vec![
            span_of(&sources, "Some"),
            span_of(&sources, "None"),
            span_of(&sources, "Vec")
        ]
    );
    assert!(diagnostics.iter().all(|d| d.code == codes::V0103));
}

#[test]
fn a_module_struct_literal_resolves_its_struct() {
    let (result, _) = check_path("crates/varyk/tests/fixtures/resolve/module_types/main.vr");
    let program = result.expect("module_types should check");
    let task = program
        .structs
        .iter()
        .position(|s| s.name == "Item")
        .expect("Item");
    let make = function(&program, "make");
    let tail = make.body.tail.as_deref().expect("a tail");
    let HirExprKind::StructLit { id, .. } = &tail.kind else {
        panic!("expected a struct literal, got {tail:?}");
    };
    assert_eq!(id.0 as usize, task);
    assert_eq!(local_ty(function(&program, "main"), "t"), tail.ty);
}

#[test]
fn a_struct_literal_naming_an_enum_is_v0101() {
    let (d, sources) = one_error("enum E {\n    A,\n}\nfn main() {\n    let e = E { x: 1 };\n}\n");
    assert_eq!(d.code, codes::V0101);
    assert_eq!(d.span, part_of(&sources, "E { x", "E"));
}

#[test]
fn printing_an_enum_or_a_generic_type_is_v0203() {
    let text = "enum E {\n    A,\n}\nfn f(a: E, b: E, o: Option<i32>, v: Vec<i32>) -> bool {\n    println!(\"{} {}\", a, v);\n    o == o\n}\nfn main() {}\n";
    let (diagnostics, sources) = errors(text);
    let found: Vec<(&str, Span)> = diagnostics.iter().map(|d| (d.code, d.span)).collect();
    assert_eq!(
        found,
        vec![
            (codes::V0203, part_of(&sources, "a, v)", "a")),
            (codes::V0203, part_of(&sources, "a, v)", "v")),
        ],
        "{diagnostics:#?}"
    );
}

#[test]
fn generic_types_are_named_in_mismatches() {
    let (d, _) = one_error("fn f(x: Result<Vec<i32>, string>) -> i32 {\n    x\n}\nfn main() {}\n");
    assert_eq!(d.code, codes::V0200);
    assert_eq!(
        d.message,
        "mismatched types: expected `i32`, found `Result<Vec<i32>, string>`"
    );
}

#[test]
fn an_integer_literal_takes_the_usize_type() {
    let program = ok("fn f() -> usize {\n    18446744073709551615\n}\nfn main() {}\n");
    assert_eq!(function(&program, "f").body.ty, Ty::Int(IntKind::Usize));
}

#[test]
fn primitive_names_are_accepted_as_value_names() {
    let program = ok(
        "struct S {\n    u8: i32,\n}\nenum E {\n    i32,\n}\nimpl S {\n    fn str(self) -> i32 {\n        self.u8\n    }\n}\nfn string(i64: i32) -> i32 {\n    let string = \"x\";\n    let mut str = 1;\n    str = i64;\n    str\n}\nfn main() {}\n",
    );
    assert_eq!(local_ty(function(&program, "string"), "string"), Ty::String);
}

// --- Values and constructors (spec 2.2, 2.6, 2.7, 2.9) ----------------------

const SHAPE: &str = "enum Shape {\n    Circle(f64),\n    Rect(f64, f64),\n    Point,\n}\n";

fn opt(ty: Ty) -> Ty {
    Ty::Option(Box::new(ty))
}

fn vec_of(ty: Ty) -> Ty {
    Ty::Vec(Box::new(ty))
}

fn result(ok: Ty, err: Ty) -> Ty {
    Ty::Result(Box::new(ok), Box::new(err))
}

#[test]
fn variant_values_are_enum_literals() {
    let program = ok(&format!(
        "{SHAPE}fn main() {{\n    let a = Shape::Circle(1.0);\n    let b = Shape::Rect(1.0, 2.0);\n    let c = Shape::Point;\n}}\n"
    ));
    let main = function(&program, "main");
    let shape = Ty::Enum(crate::resolve::EnumId(0));
    for (index, variant, count) in [(0, 0, 1), (1, 1, 2), (2, 2, 0)] {
        let value = stmt_expr(main, index);
        assert_eq!(value.ty, shape);
        let HirExprKind::EnumLit {
            variant: found,
            args,
            ..
        } = &value.kind
        else {
            panic!("expected an enum literal, got {value:?}");
        };
        assert_eq!(*found, VariantRef::User(crate::resolve::EnumId(0), variant));
        assert_eq!(args.len(), count);
    }
}

#[test]
fn unknown_variant_wrong_payload_count_and_bare_tuple_variant() {
    let cases = [
        ("let s = Shape::Nope;", codes::V0100, "Shape::Nope"),
        ("let s = Shape::Nope(1.0);", codes::V0100, "Shape::Nope"),
        ("let s = Shape::Circle();", codes::V0201, "Shape::Circle()"),
        (
            "let s = Shape::Point(1.0);",
            codes::V0201,
            "Shape::Point(1.0)",
        ),
        ("let s = Shape::Circle;", codes::V0201, "Shape::Circle"),
    ];
    for (stmt, code, at) in cases {
        let (d, sources) = one_error(&format!("{SHAPE}fn main() {{\n    {stmt}\n}}\n"));
        assert_eq!(d.code, code, "{stmt}: {d:#?}");
        assert_eq!(d.span, span_of(&sources, at), "{stmt}");
    }
    let (d, _) = one_error(&format!(
        "{SHAPE}fn main() {{\n    let s = Shape::Circle;\n}}\n"
    ));
    assert!(d.message.contains("Shape::Circle("), "{d:#?}");
}

#[test]
fn a_payload_is_typed_against_its_variant() {
    let (d, sources) = one_error(&format!(
        "{SHAPE}fn main() {{\n    let s = Shape::Circle(\"x\");\n}}\n"
    ));
    assert_eq!(d.code, codes::V0200);
    assert_eq!(d.span, span_of(&sources, "\"x\""));
}

#[test]
fn module_variants_and_struct_literals_resolve_through_the_module() {
    let (result, _) = check_path("crates/varyk/tests/fixtures/codegen/values/main.vr");
    let program = result.expect("codegen/values should check");
    let main = function(&program, "main");
    let geo_shape = program
        .enums
        .iter()
        .position(|e| e.name == "Shape" && e.module.0 == 1)
        .expect("geo::Shape");
    let geo_shape = crate::resolve::EnumId(geo_shape as u32);
    assert_eq!(local_ty(main, "far"), Ty::Enum(geo_shape));
    assert_eq!(local_ty(main, "near"), Ty::Enum(geo_shape));
    let user = program
        .structs
        .iter()
        .position(|s| s.name == "User")
        .expect("geo::User");
    assert_eq!(
        local_ty(main, "user"),
        Ty::Struct(crate::resolve::StructId(user as u32))
    );
}

#[test]
fn a_private_module_enum_variant_is_v0105() {
    let (result, sources) =
        check_path("crates/varyk/tests/fixtures/resolve/module_enum_private/main.vr");
    let diagnostics = result.expect_err("a private enum's variant is not visible");
    let d = only(&diagnostics);
    assert_eq!(d.code, codes::V0105, "{d:#?}");
    assert_eq!(d.span, span_of(&sources, "m::Hidden"));
}

#[test]
fn constructors_know_their_type_from_their_contents() {
    let program = ok(
        "fn main() {\n    let a = Some(\"x\");\n    let b = vec![1, 2];\n    let c = Some(vec![true]);\n}\n",
    );
    let main = function(&program, "main");
    assert_eq!(local_ty(main, "a"), opt(Ty::String));
    assert_eq!(local_ty(main, "b"), vec_of(I32));
    assert_eq!(local_ty(main, "c"), opt(vec_of(Ty::Bool)));
    assert!(matches!(
        stmt_expr(main, 0).kind,
        HirExprKind::EnumLit {
            variant: VariantRef::Some,
            ..
        }
    ));
    assert!(matches!(stmt_expr(main, 1).kind, HirExprKind::VecLit(_)));
}

#[test]
fn type_holes_take_the_type_a_let_annotation_expects() {
    let program = ok(
        "fn main() {\n    let a: Option<i64> = None;\n    let b: Vec<string> = vec![];\n    let c: Result<i64, string> = Ok(1);\n    let d: Result<i32, string> = Err(\"no\");\n    let e: Option<Option<u8>> = Some(None);\n}\n",
    );
    let main = function(&program, "main");
    assert_eq!(local_ty(main, "a"), opt(I64));
    assert_eq!(local_ty(main, "b"), vec_of(Ty::String));
    assert_eq!(local_ty(main, "c"), result(I64, Ty::String));
    assert_eq!(local_ty(main, "d"), result(I32, Ty::String));
    assert_eq!(local_ty(main, "e"), opt(opt(Ty::Int(IntKind::U8))));
    let HirExprKind::EnumLit { args, .. } = &stmt_expr(main, 2).kind else {
        panic!("expected `Ok`");
    };
    assert_eq!(args[0].ty, I64);
}

#[test]
fn type_holes_take_the_type_of_a_parameter_a_return_and_a_field() {
    ok(
        "struct S {\n    o: Option<i32>,\n    v: Vec<bool>,\n}\nenum W {\n    A(Option<string>),\n}\nfn take(o: Option<i32>, v: Vec<i32>, r: Result<i32, string>) {}\nfn none() -> Option<string> {\n    None\n}\nfn early(c: bool) -> Result<i32, string> {\n    if c {\n        return Ok(1);\n    }\n    Err(\"late\")\n}\nfn main() {\n    take(None, vec![], Err(\"x\"));\n    let s = S { o: None, v: vec![] };\n    let w = W::A(None);\n}\n",
    );
}

#[test]
fn a_vec_element_after_a_typed_one_takes_its_type() {
    let program = ok(
        "fn main() {\n    let v = vec![Some(1), None];\n    let w = vec![vec![true], vec![]];\n}\n",
    );
    let main = function(&program, "main");
    assert_eq!(local_ty(main, "v"), vec_of(opt(I32)));
    assert_eq!(local_ty(main, "w"), vec_of(vec_of(Ty::Bool)));
}

#[test]
fn a_type_hole_with_nothing_expected_is_v0207() {
    let cases = [
        ("let x = None;", "None"),
        ("let v = vec![];", "vec![]"),
        ("let r = Ok(1);", "Ok(1)"),
        ("let e = Err(\"no\");", "Err(\"no\")"),
        // No backward inference: the first element fixes nothing.
        ("let v = vec![None, Some(1)];", "None"),
        ("None;", "None"),
    ];
    for (stmt, at) in cases {
        let (d, sources) = one_error(&format!("fn main() {{\n    {stmt}\n}}\n"));
        assert_eq!(d.code, codes::V0207, "{stmt}: {d:#?}");
        assert_eq!(d.span, span_of(&sources, at), "{stmt}");
    }
}

#[test]
fn v0207_for_none_shows_the_annotation_to_write() {
    let (d, _) = one_error("fn main() {\n    let x = None;\n}\n");
    assert_eq!(d.code, codes::V0207);
    assert!(
        d.message.contains("let x: Option<i32> = None;")
            || d.notes
                .iter()
                .any(|n| n.contains("let x: Option<i32> = None;")),
        "{d:#?}"
    );
}

#[test]
fn a_type_hole_meeting_another_type_is_v0200() {
    let (d, _) = one_error("fn main() {\n    let x: i32 = None;\n}\n");
    assert_eq!(d.code, codes::V0200);
    assert_eq!(
        d.message,
        "mismatched types: expected `i32`, found `Option<_>`"
    );
    let (d, _) = one_error("fn main() {\n    let x: Option<i32> = Some(\"s\");\n}\n");
    assert_eq!(d.code, codes::V0200);
    let (d, _) = one_error("fn main() {\n    let v = vec![1, \"s\"];\n}\n");
    assert_eq!(d.code, codes::V0200);
}

#[test]
fn integer_literals_take_usize_and_i32_meeting_usize_is_v0200() {
    let program =
        ok("fn f(n: usize) {}\nfn main() {\n    let v: Vec<usize> = vec![1, 2];\n    f(3);\n}\n");
    let main = function(&program, "main");
    assert_eq!(call_args(stmt_expr(main, 1))[0].ty, Ty::Int(IntKind::Usize));

    let (d, sources) = one_error("fn f(n: usize) {}\nfn main() {\n    let i = 3;\n    f(i);\n}\n");
    assert_eq!(d.code, codes::V0200);
    assert_eq!(d.span, part_of(&sources, "f(i)", "i"));
    assert!(
        d.notes
            .iter()
            .any(|n| n.contains("declare the count as `usize`")),
        "{d:#?}"
    );
    let (d, _) = one_error("fn f(n: usize, i: i32) -> bool {\n    n < i\n}\nfn main() {}\n");
    assert_eq!(d.code, codes::V0200);
    assert!(
        d.notes
            .iter()
            .any(|n| n.contains("declare the count as `usize`")),
        "{d:#?}"
    );
}

#[test]
fn format_is_a_new_string_checked_like_println() {
    let program = ok("fn main() {\n    let s = format!(\"{} and {}\", 1, \"a\");\n}\n");
    let main = function(&program, "main");
    assert_eq!(local_ty(main, "s"), Ty::String);
    let HirExprKind::Format { format, args } = &stmt_expr(main, 0).kind else {
        panic!("expected `format!`");
    };
    assert_eq!(format, "{} and {}");
    assert_eq!(args.len(), 2);

    let (d, sources) = one_error("fn main() {\n    let s = format!(\"{}\");\n}\n");
    assert_eq!(d.code, codes::V0202);
    assert_eq!(d.span, span_of(&sources, "format!(\"{}\")"));
    let (d, _) =
        one_error("fn f(o: Option<i32>) {\n    let s = format!(\"{}\", o);\n}\nfn main() {}\n");
    assert_eq!(d.code, codes::V0203);
}

#[test]
fn plus_on_strings_is_v0200_with_a_format_fix_it() {
    let (d, sources) =
        one_error("fn f(a: string, b: string) -> string {\n    a + b\n}\nfn main() {}\n");
    assert_eq!(d.code, codes::V0200);
    assert_eq!(d.span, span_of(&sources, "a + b"));
    assert!(d.message.contains("format!"), "{d:#?}");
    let fix = d.fix_it.as_ref().expect("a fix-it");
    assert_eq!(fix.span, span_of(&sources, "a + b"));
    assert_eq!(fix.replacement, "format!(\"{}{}\", a, b)");
}

// --- Methods, the built-in table, and indexing (spec 2.5, 2.6) ---------------

const COUNTER_IMPL: &str = "struct Counter {\n    count: i32,\n}\nimpl Counter {\n    fn new() -> Counter {\n        Counter { count: 0 }\n    }\n\n    fn add(mut self, by: i32) {\n        self.count = self.count + by;\n    }\n\n    fn value(self) -> i32 {\n        self.count\n    }\n}\n";

const USIZE: Ty = Ty::Int(IntKind::Usize);

fn method_of(expr: &HirExpr) -> MethodRef {
    match &expr.kind {
        HirExprKind::MethodCall { method, .. } => *method,
        other => panic!("expected a method call, got {other:?}"),
    }
}

#[test]
fn methods_and_associated_functions_type_through_the_impl() {
    let program = ok(&format!(
        "{COUNTER_IMPL}fn main() {{\n    let c = Counter::new();\n    let n = Counter::new().value();\n    let mut d = Counter::new();\n    d.add(2);\n}}\n"
    ));
    let main = function(&program, "main");
    let new = function(&program, "new").id;
    let value = function(&program, "value").id;
    let add = function(&program, "add").id;
    assert!(
        matches!(stmt_expr(main, 0).kind, HirExprKind::Call { callee: Callee::Varyk(id), .. } if id == new)
    );
    assert_eq!(local_ty(main, "n"), I32);
    assert_eq!(method_of(stmt_expr(main, 1)), MethodRef::Varyk(value));
    let d_add = stmt_expr(main, 3);
    assert_eq!(method_of(d_add), MethodRef::Varyk(add));
    assert_eq!(d_add.ty, Ty::Unit);
    let HirExprKind::MethodCall { args, .. } = &d_add.kind else {
        unreachable!()
    };
    assert_eq!(args[0].ty, I32);
}

#[test]
fn a_method_declared_in_an_impl_on_an_enum_is_callable() {
    let program = ok(&format!(
        "{SHAPE}impl Shape {{\n    fn sides(self) -> usize {{\n        0\n    }}\n\n    fn origin() -> Shape {{\n        Shape::Point\n    }}\n}}\nfn main() {{\n    let s = Shape::origin();\n    let n = s.sides();\n}}\n"
    ));
    let main = function(&program, "main");
    assert!(matches!(local_ty(main, "s"), Ty::Enum(_)));
    assert_eq!(local_ty(main, "n"), USIZE);
}

#[test]
fn built_in_calls_type_from_the_table() {
    let program = ok(
        "fn main() {\n    let mut v: Vec<string> = Vec::new();\n    v.push(\"a\");\n    let last = v.pop();\n    let n = v.len();\n    let s = \"x\";\n    let m = s.len();\n    let t = s.clone();\n}\n",
    );
    let main = function(&program, "main");
    assert_eq!(local_ty(main, "v"), vec_of(Ty::String));
    assert_eq!(local_ty(main, "last"), opt(Ty::String));
    assert_eq!(local_ty(main, "n"), USIZE);
    assert_eq!(local_ty(main, "m"), USIZE);
    assert_eq!(local_ty(main, "t"), Ty::String);
    assert!(matches!(
        stmt_expr(main, 0).kind,
        HirExprKind::Call {
            callee: Callee::Builtin(_),
            ..
        }
    ));
    let MethodRef::Builtin(push) = method_of(stmt_expr(main, 1)) else {
        panic!("push is a built-in")
    };
    assert_eq!(push.get().name, "push");
}

#[test]
fn vec_new_takes_its_type_from_where_it_goes() {
    let program = ok(
        "fn f(v: Vec<i32>) {}\nfn g() -> Vec<string> {\n    Vec::new()\n}\nfn main() {\n    f(Vec::new());\n}\n",
    );
    let main = function(&program, "main");
    assert_eq!(call_args(stmt_expr(main, 0))[0].ty, vec_of(I32));
}

#[test]
fn vec_new_with_nothing_expected_is_v0207_showing_the_annotated_shape() {
    let (d, sources) = one_error("fn main() {\n    let v = Vec::new();\n}\n");
    assert_eq!(d.code, codes::V0207, "{d:#?}");
    assert_eq!(d.span, span_of(&sources, "Vec::new()"));
    assert!(
        d.message.contains("`let v: Vec<i32> = Vec::new();`"),
        "{d:#?}"
    );
}

#[test]
fn an_unknown_method_or_associated_function_is_v0100_naming_the_type() {
    let cases = [
        (
            "let x: Vec<i32> = vec![1];\n    x.nope();",
            "nope",
            &["Vec<i32>", "`push`, `pop`, `len`, `is_empty`"][..],
        ),
        (
            "let x = \"a\";\n    x.nope();",
            "nope",
            &["string", "`len`, `clone`, `is_empty`"][..],
        ),
        (
            "let c = Counter::new();\n    c.nope();",
            "nope",
            &["Counter"][..],
        ),
        ("let n = 1;\n    n.len();", "len", &["i32"][..]),
        (
            "let c = Counter::nope();",
            "Counter::nope",
            &["Counter"][..],
        ),
        (
            "let v: Vec<i32> = Vec::nope();",
            "Vec::nope",
            &["Vec::new"][..],
        ),
        (
            "let c = Counter::value();",
            "Counter::value",
            &[".value()"][..],
        ),
        (
            "let c = Counter::new();\n    c.new();",
            "new",
            &["Counter::new"][..],
        ),
        (
            "let f = Counter::new;",
            "Counter::new",
            &["Counter::new("][..],
        ),
    ];
    for (stmts, at, words) in cases {
        let (d, sources) = one_error(&format!("{COUNTER_IMPL}fn main() {{\n    {stmts}\n}}\n"));
        assert_eq!(d.code, codes::V0100, "{stmts}: {d:#?}");
        let span = if at.contains("::") {
            span_of(&sources, at)
        } else {
            // The method's name, after the dot.
            part_of(&sources, &format!(".{at}("), at)
        };
        assert_eq!(d.span, span, "{stmts}: {d:#?}");
        for word in words {
            assert!(d.message.contains(word), "{stmts}: {word}: {d:#?}");
        }
    }
}

#[test]
fn wrong_argument_counts_on_methods_are_v0201() {
    for stmt in [
        "v.push();",
        "v.len(1);",
        "let w: Vec<i32> = Vec::new(1);",
        "c.add();",
        "let d = Counter::new(1);",
    ] {
        let (d, _) = one_error(&format!(
            "{COUNTER_IMPL}fn main() {{\n    let mut v: Vec<i32> = vec![];\n    let mut c = Counter::new();\n    {stmt}\n}}\n"
        ));
        assert_eq!(d.code, codes::V0201, "{stmt}: {d:#?}");
    }
}

#[test]
fn arguments_are_typed_against_the_method() {
    let (d, sources) =
        one_error("fn main() {\n    let mut v: Vec<i32> = vec![];\n    v.push(\"x\");\n}\n");
    assert_eq!(d.code, codes::V0200, "{d:#?}");
    assert_eq!(d.span, span_of(&sources, "\"x\""));
}

#[test]
fn a_private_method_or_associated_function_from_another_module_is_v0105() {
    let (result, sources) =
        check_path("crates/varyk/tests/fixtures/resolve/module_method_private/main.vr");
    let diagnostics = result.expect_err("private members are not visible");
    assert_eq!(diagnostics.len(), 2, "{diagnostics:#?}");
    for (d, at) in diagnostics.iter().zip(["secret", "m::Shape::origin"]) {
        assert_eq!(d.code, codes::V0105, "{d:#?}");
        assert_eq!(d.span, span_of(&sources, at), "{d:#?}");
        assert!(d.fix_it.is_some(), "{d:#?}");
    }
}

// --- Field-level `pub` (spec 3.4) --------------------------------------------

#[test]
fn reading_a_private_field_from_another_module_is_v0105_with_the_two_way_note() {
    let (result, sources) =
        check_path("crates/varyk/tests/fixtures/resolve/field_private_read/main.vr");
    let diagnostics = result.expect_err("a private field is not visible from another module");
    let d = only(&diagnostics);
    assert_eq!(d.code, codes::V0105, "{d:#?}");
    assert_eq!(d.span, part_of(&sources, "c.n", "n"));
    assert!(
        d.notes
            .iter()
            .any(|n| n.contains("make the field public with `pub`")
                && n.contains("a `pub fn` on the struct")),
        "{d:#?}"
    );
    assert!(d.fix_it.is_some(), "{d:#?}");
}

#[test]
fn assigning_a_private_field_from_another_module_is_v0105() {
    let (result, sources) =
        check_path("crates/varyk/tests/fixtures/resolve/field_private_assign/main.vr");
    let diagnostics = result.expect_err("a private field is not visible from another module");
    let d = only(&diagnostics);
    assert_eq!(d.code, codes::V0105, "{d:#?}");
    assert_eq!(d.span, part_of(&sources, "c.n = 2", "n"));
    assert!(d.fix_it.is_some(), "{d:#?}");
}

#[test]
fn a_struct_literal_naming_a_private_field_from_another_module_is_v0105_with_literal_wording() {
    let (result, sources) =
        check_path("crates/varyk/tests/fixtures/errors/v0105_private_field_in_literal/main.vr");
    let diagnostics =
        result.expect_err("a private field cannot be named in another module's literal");
    let d = only(&diagnostics);
    assert_eq!(d.code, codes::V0105, "{d:#?}");
    assert_eq!(d.span, part_of(&sources, "Cart { n: 1 }", "n"));
    assert!(
        d.message.contains("`Cart { .. }` cannot be written here"),
        "{d:#?}"
    );
    assert!(d.fix_it.is_some(), "{d:#?}");
}

#[test]
fn the_same_literal_check_applies_when_the_struct_is_named_through_a_use_alias() {
    let (result, sources) =
        check_path("crates/varyk/tests/fixtures/resolve/field_private_literal_alias/main.vr");
    let diagnostics = result.expect_err("a `use` alias does not bypass field privacy");
    let d = only(&diagnostics);
    assert_eq!(d.code, codes::V0105, "{d:#?}");
    assert_eq!(d.span, part_of(&sources, "Cart { n: 1 }", "n"));
}

#[test]
fn a_struct_method_reads_its_own_private_field_when_called_from_another_module() {
    let (result, _) =
        check_path("crates/varyk/tests/fixtures/resolve/field_private_via_method/main.vr");
    result.expect("a method of the struct's own module reads its private field freely");
}

#[test]
fn a_child_module_reads_its_parents_private_field() {
    let (result, _) =
        check_path("crates/varyk/tests/fixtures/resolve/field_private_child_module/main.vr");
    result.expect("a descendant module reads a private field of its ancestor's struct");
}

#[test]
fn indexing_types_the_element_and_takes_a_usize() {
    let program = ok(&format!(
        "{COUNTER_IMPL}fn main() {{\n    let mut v = vec![1, 2];\n    let x = v[0];\n    let i: usize = 1;\n    let y = v[i];\n    v[0] = 3;\n    let mut cs = vec![Counter::new()];\n    cs[0].count = 1;\n    cs[0].add(1);\n}}\n"
    ));
    let main = function(&program, "main");
    assert_eq!(local_ty(main, "x"), I32);
    assert_eq!(local_ty(main, "y"), I32);
    assert!(matches!(stmt_expr(main, 1).kind, HirExprKind::Index { .. }));
}

#[test]
fn indexing_anything_but_a_vec_or_with_an_i32_is_v0200() {
    let (d, sources) =
        one_error("fn main() {\n    let v = vec![1, 2];\n    let j = 1;\n    let x = v[j];\n}\n");
    assert_eq!(d.code, codes::V0200, "{d:#?}");
    assert_eq!(d.span, part_of(&sources, "v[j]", "j"));
    assert!(d.notes.iter().any(|n| n.contains("usize")), "{d:#?}");

    let (d, sources) = one_error("fn main() {\n    let n = 5;\n    let x = n[0];\n}\n");
    assert_eq!(d.code, codes::V0200, "{d:#?}");
    assert_eq!(d.span, span_of(&sources, "n[0]"));
    assert!(d.message.contains("Vec"), "{d:#?}");
}

#[test]
fn indexing_with_an_i32_range_variable_says_to_make_a_range_end_usize() {
    let (d, sources) = one_error(
        "fn main() {\n    let v = vec![1, 2];\n    for i in 0..3 {\n        let x = v[i];\n    }\n}\n",
    );
    assert_eq!(d.code, codes::V0200, "{d:#?}");
    assert_eq!(d.span, part_of(&sources, "v[i]", "i"));
    assert!(
        d.notes
            .iter()
            .any(|n| n.contains("make a range end `usize`")
                && n.contains("`0..v.len()`")
                && !n.contains("let mut i")),
        "{d:#?}"
    );
}

#[test]
fn a_range_variable_used_as_a_call_argument_says_to_make_a_range_end_usize() {
    let (d, sources) = one_error(
        "fn make(n: usize) {}\nfn main() {\n    for i in 0..3 {\n        make(i);\n    }\n}\n",
    );
    assert_eq!(d.code, codes::V0200, "{d:#?}");
    assert_eq!(d.span, part_of(&sources, "make(i)", "i"));
    assert!(
        d.notes
            .iter()
            .any(|n| n.contains("make a range end `usize`")
                && n.contains("`0..v.len()`")
                && !n.contains("let mut i")),
        "{d:#?}"
    );
}

#[test]
fn a_range_variable_compared_to_a_len_says_to_make_a_range_end_usize() {
    let (d, sources) = one_error(
        "fn main() {\n    let v = vec![1, 2];\n    for i in 0..3 {\n        i == v.len();\n    }\n}\n",
    );
    assert_eq!(d.code, codes::V0200, "{d:#?}");
    assert_eq!(d.span, part_of(&sources, "i == v.len()", "v.len()"));
    assert!(
        d.notes
            .iter()
            .any(|n| n.contains("make a range end `usize`")
                && n.contains("`0..v.len()`")
                && !n.contains("let mut i")),
        "{d:#?}"
    );
}

#[test]
fn a_range_variable_used_to_type_a_let_says_to_make_a_range_end_usize() {
    let (d, sources) =
        one_error("fn main() {\n    for i in 0..3 {\n        let n: usize = i;\n    }\n}\n");
    assert_eq!(d.code, codes::V0200, "{d:#?}");
    assert_eq!(d.span, part_of(&sources, "= i;", "i"));
    assert!(
        d.notes
            .iter()
            .any(|n| n.contains("make a range end `usize`")
                && n.contains("`0..v.len()`")
                && !n.contains("let mut i")),
        "{d:#?}"
    );
}

#[test]
fn a_plain_local_compared_to_a_len_still_says_let_mut() {
    let (d, sources) = one_error(
        "fn main() {\n    let v = vec![1, 2];\n    let mut j = 0;\n    while j < v.len() {}\n}\n",
    );
    assert_eq!(d.code, codes::V0200, "{d:#?}");
    assert_eq!(d.span, part_of(&sources, "j < v.len()", "v.len()"));
    assert!(
        d.notes
            .iter()
            .any(|n| n.contains("declare the count as `usize`")
                && n.contains("let mut i: usize = 0;")),
        "{d:#?}"
    );
}

// --- `match` and patterns (spec 2.3) ------------------------------------------

const MATCH_ITEMS: &str = "enum Shape {\n    Circle(f64),\n    Rect(f64, f64),\n    Point,\n}\nenum Color {\n    Red,\n    Blue,\n}\nstruct Item {\n    status: Color,\n}\nfn mk() -> Item {\n    Item { status: Color::Red }\n}\n";

/// A program with [`MATCH_ITEMS`] and `body` as the body of `f`, whose
/// parameters are a `Shape`, an `Option<i32>`, and a `bool`.
fn with_match(ret: &str, body: &str) -> String {
    format!(
        "{MATCH_ITEMS}fn f(s: Shape, o: Option<i32>, c: bool){ret} {{\n{body}\n}}\nfn main() {{}}\n"
    )
}

#[test]
fn a_match_on_an_enum_types_its_arms_and_binds_its_payloads() {
    let program = ok(&with_match(
        " -> f64",
        "    match s {\n        Shape::Circle(r) => 3.14 * r * r,\n        Shape::Rect(w, _) => w,\n        Shape::Point => 0.0,\n    }",
    ));
    let f = function(&program, "f");
    let tail = f.body.tail.as_deref().expect("a tail");
    assert_eq!(tail.ty, Ty::Float(FloatKind::F64));
    let HirExprKind::Match { arms, .. } = &tail.kind else {
        panic!("expected a match, got {tail:?}");
    };
    assert_eq!(arms.len(), 3);
    assert_eq!(local_ty(f, "r"), Ty::Float(FloatKind::F64));
    assert_eq!(local_ty(f, "w"), Ty::Float(FloatKind::F64));
}

#[test]
fn a_match_on_option_and_result_uses_their_variants_and_a_catch_all_binds_the_value() {
    let program = ok(&with_match(
        " -> i32",
        "    let r: Result<i32, string> = Ok(1);\n    let a = match r {\n        Ok(n) => n,\n        Err(e) => 0,\n    };\n    let b = match o {\n        None => 0,\n        other => 1,\n    };\n    match o {\n        Some(n) => n,\n        None => a + b,\n    }",
    ));
    let f = function(&program, "f");
    assert_eq!(local_ty(f, "e"), Ty::String);
    assert_eq!(local_ty(f, "other"), Ty::Option(Box::new(I32)));
}

#[test]
fn matching_a_struct_is_v0205() {
    let (d, sources) = one_error(&with_match(
        "",
        "    let t = mk();\n    match t {\n        _ => {}\n    }",
    ));
    assert_eq!(d.code, codes::V0205, "{d:#?}");
    let head = part_of(&sources, "match t {", "t {");
    assert_eq!(d.span, Span::new(head.file, head.start, head.start + 1));
    assert!(d.message.contains("`Item`"), "{d:#?}");
    assert!(d.notes.iter().any(|note| note.contains("`if`")), "{d:#?}");
}

#[test]
fn a_pattern_that_does_not_fit_the_value_is_v0205() {
    let cases: &[(&str, &str, &str)] = &[
        // A variant of another enum.
        (
            "match s {\n        Color::Red => {}\n        _ => {}\n    }",
            "Color::Red",
            "Color",
        ),
        // `Some` on an enum, `Ok` on an `Option`.
        (
            "match s {\n        Some(x) => {}\n        _ => {}\n    }",
            "Some(x)",
            "Shape",
        ),
        (
            "match o {\n        Ok(x) => {}\n        _ => {}\n    }",
            "Ok(x)",
            "Option<i32>",
        ),
        // The wrong number of positions.
        (
            "match s {\n        Shape::Rect(w) => {}\n        _ => {}\n    }",
            "Shape::Rect(w)",
            "2",
        ),
        (
            "match s {\n        Shape::Circle => {}\n        _ => {}\n    }",
            "Shape::Circle",
            "1",
        ),
        (
            "match s {\n        Shape::Point(p) => {}\n        _ => {}\n    }",
            "Shape::Point(p)",
            "no values",
        ),
        (
            "match o {\n        Some => {}\n        _ => {}\n    }",
            "Some",
            "1",
        ),
        (
            "match o {\n        Some(a, b) => {}\n        _ => {}\n    }",
            "Some(a, b)",
            "1",
        ),
    ];
    for (stmt, at, word) in cases {
        let (d, sources) = one_error(&with_match("", &format!("    {stmt}")));
        assert_eq!(d.code, codes::V0205, "{stmt}: {d:#?}");
        let arm = format!("{at} =>");
        assert_eq!(d.span, part_of(&sources, &arm, at), "{stmt}: {d:#?}");
        assert!(d.message.contains(word), "{stmt}: {word}: {d:#?}");
    }
}

#[test]
fn a_name_bound_twice_in_one_pattern_is_v0103() {
    let (d, sources) = one_error(&with_match(
        "",
        "    match s {\n        Shape::Rect(a, a) => {}\n        _ => {}\n    }",
    ));
    assert_eq!(d.code, codes::V0103, "{d:#?}");
    let second = part_of(&sources, "Rect(a, a)", ", a");
    assert_eq!(d.span, Span::new(second.file, second.start + 2, second.end));
}

#[test]
fn a_match_missing_a_variant_is_v0204_naming_it() {
    let (d, sources) = one_error(&with_match(
        "",
        "    match s {\n        Shape::Circle(r) => {}\n        Shape::Rect(w, h) => {}\n    }",
    ));
    assert_eq!(d.code, codes::V0204, "{d:#?}");
    assert_eq!(d.span, span_of(&sources, "match s"));
    assert!(d.message.contains("`Shape::Point`"), "{d:#?}");

    let (d, _) = one_error(&with_match(
        "",
        "    match o {\n        Some(n) => {}\n    }",
    ));
    assert_eq!(d.code, codes::V0204, "{d:#?}");
    assert!(d.message.contains("`None`"), "{d:#?}");
}

#[test]
fn a_catch_all_covers_the_rest_but_only_as_the_last_arm() {
    ok(&with_match(
        "",
        "    match s {\n        Shape::Point => {}\n        _ => {}\n    }\n    match o {\n        Some(n) => {}\n        rest => {}\n    }",
    ));
    let (d, sources) = one_error(&with_match(
        "",
        "    match s {\n        _ => {}\n        Shape::Point => {}\n    }",
    ));
    assert_eq!(d.code, codes::V0205, "{d:#?}");
    assert_eq!(d.span, span_of(&sources, "_ => {}"));
    let (d, _) = one_error(&with_match(
        "",
        "    match o {\n        n => {}\n        None => {}\n    }",
    ));
    assert_eq!(d.code, codes::V0205, "{d:#?}");
}

/// M2 let a repeated variant arm through; it can never run, so it is
/// V0205 now (M4 spec 2.5).
#[test]
fn a_variant_arm_repeated_after_an_identical_one_is_v0205() {
    let (d, sources) = one_error(&with_match(
        "",
        "    match s {\n        Shape::Point => {}\n        Shape::Point => {}\n        _ => {}\n    }",
    ));
    assert_eq!(d.code, codes::V0205, "{d:#?}");
    let second = span_of(&sources, "Shape::Point => {}\n        _");
    assert_eq!(
        d.span,
        Span::new(second.file, second.start, second.start + 18)
    );
}

#[test]
fn match_arms_of_different_types_are_v0200() {
    let (d, sources) = one_error(&with_match(
        "",
        "    let x = match o {\n        Some(n) => n,\n        None => \"none\",\n    };",
    ));
    assert_eq!(d.code, codes::V0200, "{d:#?}");
    assert_eq!(d.span, span_of(&sources, "\"none\""));

    // A value arm beside an arm with no value.
    let (d, _) = one_error(&with_match(
        "",
        "    let x = match o {\n        Some(n) => n,\n        None => println!(\"none\"),\n    };",
    ));
    assert_eq!(d.code, codes::V0200, "{d:#?}");
}

#[test]
fn arms_with_no_value_and_arms_that_always_return() {
    ok(&with_match(
        "",
        "    match o {\n        Some(n) => println!(\"{}\", n),\n        None => {\n            println!(\"none\");\n        }\n    }",
    ));
    ok(&with_match(
        " -> i32",
        "    let n = match o {\n        None => {\n            return 0;\n        }\n        Some(n) => n,\n    };\n    n",
    ));
}

#[test]
fn a_type_hole_in_an_arm_takes_the_expected_type_or_an_earlier_arms() {
    ok(&with_match(
        " -> Option<i32>",
        "    let a = match o {\n        Some(n) => Some(n + 1),\n        None => None,\n    };\n    match o {\n        Some(n) => None,\n        None => a,\n    }",
    ));
    // A later arm does not type an earlier one.
    let (d, sources) = one_error(&with_match(
        "",
        "    let a = match o {\n        None => None,\n        Some(n) => Some(n),\n    };",
    ));
    assert_eq!(d.code, codes::V0207, "{d:#?}");
    let arm = part_of(&sources, "None => None", "> None");
    assert_eq!(d.span, Span::new(arm.file, arm.start + 2, arm.end));
}

#[test]
fn an_if_a_block_or_a_field_of_a_temporary_as_the_head_is_v0001() {
    for (stmt, at) in [
        ("match { s } {\n        _ => {}\n    }", "{ s }"),
        ("match mk().status {\n        _ => {}\n    }", "mk().status"),
    ] {
        let (d, sources) = one_error(&with_match("", &format!("    {stmt}")));
        assert_eq!(d.code, codes::V0001, "{stmt}: {d:#?}");
        assert_eq!(d.span, span_of(&sources, at), "{stmt}: {d:#?}");
        assert!(d.message.contains("`let`"), "{stmt}: {d:#?}");
    }
    let (d, _) = one_error(&with_match(
        "",
        "    match (if c { s } else { Shape::Point }) {\n        _ => {}\n    }",
    ));
    assert_eq!(d.code, codes::V0001, "{d:#?}");
}

#[test]
fn a_field_of_an_element_is_a_head() {
    ok(&with_match(
        "",
        "    let tasks = vec![mk()];\n    match tasks[0].status {\n        Color::Red => {}\n        Color::Blue => {}\n    }\n    match Some(mk()) {\n        t => {}\n    }",
    ));
}

#[test]
fn a_pattern_binding_is_scoped_to_its_arm_like_a_let() {
    // The `Some(n)` binding shadows the outer `n`, which is intact after
    // the `match`.
    let program = ok(&with_match(
        " -> i32",
        "    let n = 5;\n    match o {\n        Some(n) => println!(\"{}\", n),\n        None => {}\n    }\n    n",
    ));
    let f = function(&program, "f");
    let tail = f.body.tail.as_deref().expect("a tail");
    let HirExprKind::Local(id) = tail.kind else {
        panic!("expected a local, got {tail:?}");
    };
    let outer = f
        .locals
        .iter()
        .position(|l| l.name == "n")
        .expect("outer n");
    assert_eq!(id.0 as usize, outer, "the tail is the outer `n`");

    let (d, sources) = one_error(&with_match(
        "",
        "    match o {\n        Some(k) => {}\n        None => {}\n    }\n    println!(\"{}\", k);",
    ));
    assert_eq!(d.code, codes::V0100, "{d:#?}");
    assert_eq!(d.span, part_of(&sources, "\", k)", "k"));
}

// --- `for` and ranges (spec 2.4) ----------------------------------------------

const LOOP_ITEMS: &str = "struct Item {\n    title: string,\n}\nfn mk() -> Item {\n    Item { title: \"t\" }\n}\nfn all() -> Vec<Item> {\n    vec![mk()]\n}\n";

/// A program with `LOOP_ITEMS` and `fn f(values: Vec<i32>, tasks: Vec<Item>,
/// n: i64, s: string, o: Option<i32>, c: bool) { body }`.
fn with_loop(body: &str) -> String {
    format!(
        "{LOOP_ITEMS}fn f(values: Vec<i32>, tasks: Vec<Item>, n: i64, s: string, o: Option<i32>, c: bool) {{\n{body}\n}}\nfn main() {{}}\n"
    )
}

/// The `index`-th statement of `function`'s body, which must be a `for`.
fn for_head(function: &HirFunction, index: usize) -> &HirForHead {
    match &function.body.stmts[index] {
        HirStmt::For { head, .. } => head,
        other => panic!("statement {index} is {other:?}"),
    }
}

#[test]
fn a_range_variable_takes_the_type_of_the_ends() {
    let program = ok(&with_loop(
        "    for i in 0..values.len() {\n        println!(\"{}\", i);\n    }\n    for j in 0..10 {\n        println!(\"{}\", j);\n    }\n    for k in n..0 {\n        println!(\"{}\", k);\n    }",
    ));
    let f = function(&program, "f");
    assert_eq!(local_ty(f, "i"), USIZE);
    assert_eq!(local_ty(f, "j"), I32);
    assert_eq!(local_ty(f, "k"), I64);
    let HirForHead::Range { start, end, .. } = for_head(f, 0) else {
        panic!("a range head");
    };
    assert_eq!((&start.ty, &end.ty), (&USIZE, &USIZE));
    assert!(
        matches!(end.kind, HirExprKind::MethodCall { .. }),
        "{end:?}"
    );
}

#[test]
fn a_vec_variable_is_one_element() {
    let program = ok(&with_loop(
        "    for value in values {\n        println!(\"{}\", value);\n    }\n    for t in tasks {\n        println!(\"{}\", t.title);\n    }\n    for u in all() {\n        println!(\"{}\", u.title);\n    }",
    ));
    let f = function(&program, "f");
    assert_eq!(local_ty(f, "value"), I32);
    let task = local_ty(f, "u");
    assert!(matches!(task, Ty::Struct(_)), "{task:?}");
    assert_eq!(local_ty(f, "t"), task);
    assert!(matches!(for_head(f, 2), HirForHead::Vec(_)));
}

#[test]
fn a_for_over_anything_else_is_v0200_naming_what_was_expected() {
    for (head, found) in [("s", "string"), ("o", "Option<i32>"), ("n", "i64")] {
        let (d, sources) = one_error(&with_loop(&format!("    for x in {head} {{}}")));
        assert_eq!(d.code, codes::V0200, "{d:#?}");
        let at = span_of(&sources, &format!("x in {head} {{")).start + 5;
        assert_eq!(d.span, Span::new(FileId(0), at, at + head.len() as u32));
        assert!(d.message.contains("a `Vec`"), "{d:#?}");
        assert!(d.message.contains(&format!("`{found}`")), "{d:#?}");
    }
}

#[test]
fn range_ends_are_integers_of_one_type() {
    let (d, sources) = one_error(&with_loop("    for i in n..values.len() {}"));
    assert_eq!(d.code, codes::V0200, "{d:#?}");
    assert_eq!(d.span, span_of(&sources, "values.len()"));
    assert!(
        d.message.contains("expected `i64`, found `usize`"),
        "{d:#?}"
    );

    let (d, sources) = one_error(&with_loop("    for x in 0.0..1.5 {}"));
    assert_eq!(d.code, codes::V0200, "{d:#?}");
    assert_eq!(d.span, span_of(&sources, "0.0..1.5"));
    assert!(d.message.contains("integers"), "{d:#?}");
    assert!(d.message.contains("`f64`"), "{d:#?}");
}

#[test]
fn an_if_or_a_part_of_a_temporary_as_the_head_is_v0001() {
    for head in ["(if c { values } else { values })", "mk().title"] {
        let (d, sources) = one_error(&with_loop(&format!("    for x in {head} {{}}")));
        assert_eq!(d.code, codes::V0001, "{head}: {d:#?}");
        let unwrapped = head.trim_start_matches('(').trim_end_matches(')');
        assert_eq!(d.span, span_of(&sources, unwrapped), "{d:#?}");
        assert!(d.message.contains("with `let` first"), "{d:#?}");
    }
    // A part of a place is fine.
    ok(&with_loop(
        "    let lists = vec![values];\n    for x in lists[0] {\n        println!(\"{}\", x);\n    }",
    ));
}

#[test]
fn break_and_continue_work_in_a_for() {
    ok(&with_loop(
        "    for i in 0..3 {\n        if i == 1 {\n            continue;\n        }\n        break;\n    }",
    ));
    let (d, _) = one_error("fn main() {\n    break;\n}\n");
    assert_eq!(
        d.message,
        "`break` is only allowed inside a `while` or `for` loop"
    );
}

#[test]
fn the_loop_variable_lives_only_in_the_body() {
    let (d, sources) = one_error(&with_loop("    for i in 0..3 {}\n    println!(\"{}\", i);"));
    assert_eq!(d.code, codes::V0100, "{d:#?}");
    assert_eq!(d.span, part_of(&sources, "\", i)", "i"));

    let (d, _) = one_error(&with_loop("    for i in 0..3 {\n        i\n    }"));
    assert_eq!(d.code, codes::V0200, "{d:#?}");
}

// --- `?` ----------------------------------------------------------------------

/// `body` as the body of `fn run() -> ret`, after a `parse` returning
/// `Result<i32, string>` and a `limit` returning `Result<i32, i64>`.
fn with_parse(ret: &str, body: &str) -> String {
    format!(
        "fn parse() -> Result<i32, string> {{\n    Ok(1)\n}}\n\nfn limit() -> Result<i32, i64> {{\n    Ok(2)\n}}\n\nfn run(){ret} {{\n{body}\n}}\n\nfn main() {{}}\n"
    )
}

#[test]
fn question_yields_the_ok_value() {
    let program = ok(&with_parse(
        " -> Result<i32, string>",
        "    let v = parse()?;\n    Ok(v + parse()?)",
    ));
    let run = function(&program, "run");
    assert_eq!(local_ty(run, "v"), I32);
    let value = stmt_expr(run, 0);
    assert_eq!(value.ty, I32);
    match &value.kind {
        HirExprKind::Try { operand, kind } => {
            assert_eq!(*kind, TryKind::Result);
            assert_eq!(operand.ty, Ty::Result(Box::new(I32), Box::new(Ty::String)));
        }
        other => panic!("expected `?`, got {other:?}"),
    }
}

#[test]
fn question_works_in_a_match_arm_a_for_body_and_as_a_statement() {
    ok(&with_parse(
        " -> Result<i32, string>",
        "    parse()?;\n    let mut total = 0;\n    for i in 0..3 {\n        total = total + i + parse()?;\n    }\n    let n = match parse() {\n        Ok(n) => n + parse()?,\n        Err(e) => 0,\n    };\n    Ok(total + n)",
    ));
}

#[test]
fn question_in_a_function_that_does_not_return_a_result_is_v0206() {
    let cases = [
        ("", "    let v = parse()?;", "nothing"),
        (" -> i32", "    parse()?", "`i32`"),
    ];
    for (ret, body, returns) in cases {
        let (d, sources) = one_error(&with_parse(ret, body));
        assert_eq!(d.code, codes::V0206, "{ret}: {d:#?}");
        assert_eq!(d.span, span_of(&sources, "parse()?"));
        assert!(d.message.contains(returns), "{d:#?}");
        assert!(d.message.contains("`Result<i32, string>`"), "{d:#?}");
    }

    // `main` returns nothing, so `?` is not allowed there.
    let (d, _) = one_error(
        "fn parse() -> Result<i32, string> {\n    Ok(1)\n}\n\nfn main() {\n    let v = parse()?;\n}\n",
    );
    assert_eq!(d.code, codes::V0206, "{d:#?}");
    assert!(d.message.contains("`main`"), "{d:#?}");
}

#[test]
fn question_on_another_error_type_is_v0206_naming_both() {
    let (d, sources) = one_error(&with_parse(
        " -> Result<i32, string>",
        "    let v = limit()?;\n    Ok(v)",
    ));
    assert_eq!(d.code, codes::V0206, "{d:#?}");
    assert_eq!(d.span, span_of(&sources, "limit()?"));
    assert!(d.message.contains("`Result<i32, i64>`"), "{d:#?}");
    assert!(d.message.contains("`Result<i32, string>`"), "{d:#?}");
}

#[test]
fn question_on_something_that_is_not_a_result_is_v0206() {
    let (d, sources) = one_error(&with_parse(
        " -> Result<i32, string>",
        "    let x: Option<i32> = Some(1);\n    let v = x?;\n    Ok(v)",
    ));
    assert_eq!(d.code, codes::V0206, "{d:#?}");
    assert_eq!(d.span, span_of(&sources, "x?"));
    assert!(d.message.contains("`Option<i32>`"), "{d:#?}");
    assert!(d.message.contains("`Result<i32, string>`"), "{d:#?}");
    assert!(
        d.message.contains("returns `Result<i32, string>`"),
        "{d:#?}"
    );

    let (d, _) = one_error(&with_parse(
        " -> Result<i32, string>",
        "    let v = 5?;\n    Ok(v)",
    ));
    assert_eq!(d.code, codes::V0206, "{d:#?}");
    assert!(d.message.contains("`i32`"), "{d:#?}");
}

#[test]
fn a_name_that_is_a_unit_variant_of_its_own_enum_is_v0103() {
    let cases = [
        (
            "    match s {\n        Shape::Circle(r) => {}\n        Point => {}\n    }",
            "Point =>",
        ),
        ("    let Point = Shape::Point;", "Point ="),
        (
            "    let v: Vec<Shape> = vec![];\n    for Point in v {}",
            "Point in",
        ),
        (
            "    let m: Option<Shape> = None;\n    match m {\n        Some(Point) => {}\n        None => {}\n    }",
            "Point)",
        ),
    ];
    for (body, at) in cases {
        let (d, sources) = one_error(&with_match("", body));
        assert_eq!(d.code, codes::V0103, "{body}: {d:#?}");
        assert_eq!(d.span, part_of(&sources, at, "Point"), "{body}: {d:#?}");
        assert!(d.message.contains("`Shape`"), "{body}: {d:#?}");
        assert!(
            d.notes.iter().any(|n| n.contains("`Shape::Point`")),
            "{body}: {d:#?}"
        );
    }
    let (d, _) = one_error(&format!(
        "{MATCH_ITEMS}fn g(Point: Shape) {{}}\nfn main() {{}}\n"
    ));
    assert_eq!(d.code, codes::V0103, "{d:#?}");
    // A name of another enum's variant, or a variant with a payload, is fine.
    ok(&with_match(
        "",
        "    let Red = Shape::Point;\n    let Circle = s;",
    ));
}

// --- Imported structs (M3 spec 4.1, 4.2) ---------------------------------

#[test]
fn misusing_an_imported_struct_says_what_to_change() {
    let (result, sources) = check_path("crates/varyk/tests/fixtures/interop/struct_misuse/main.vr");
    let diagnostics = result.expect_err("should fail");
    let found: Vec<(&str, Span)> = diagnostics.iter().map(|d| (d.code, d.span)).collect();
    assert_eq!(
        found,
        vec![
            (codes::V0108, span_of(&sources, "into_inner")),
            (codes::V0108, span_of(&sources, "name")),
            (codes::V0100, span_of(&sources, "ext::Matcher::is_match")),
            (codes::V0100, {
                let call = span_of(&sources, "m.new");
                Span::new(call.file, call.start + 2, call.end)
            }),
            (codes::V0105, span_of(&sources, "words")),
        ],
        "{diagnostics:#?}"
    );
    assert!(
        diagnostics[0]
            .notes
            .iter()
            .any(|n| n.contains("`&self` or `&mut self`"))
    );
    assert!(
        diagnostics[1]
            .notes
            .iter()
            .any(|n| n.contains("`fn name(&self) -> &String`"))
    );
    assert!(diagnostics[4].fix_it.is_none(), "{:#?}", diagnostics[4]);
}

#[test]
fn calling_a_rust_method_varyk_skipped_says_why_in_a_note() {
    let (result, _) = check_path("crates/varyk/tests/fixtures/interop/skipped_methods/main.vr");
    let diagnostics = result.expect_err("should fail");
    let found: Vec<(&str, &str)> = diagnostics
        .iter()
        .map(|d| (d.code, d.notes.first().map_or("", String::as_str)))
        .collect();
    assert_eq!(
        found,
        vec![
            (
                codes::V0100,
                "`c` exists in the Rust file but is `const`; Varyk does not import such \
                 methods; call it from a plain `pub fn` of another name in an `impl A` block"
            ),
            (
                codes::V0100,
                "`t` exists in the Rust file but is behind `#[cfg]`; Varyk does not import such \
                 methods; such a method may not exist in the build"
            ),
            (
                codes::V0100,
                "a parameter of `g` is behind `#[cfg]`, so `g` may not exist with this shape in \
                 the build; Varyk does not import such methods"
            ),
            (
                codes::V0203,
                "`A` is a Rust type whose `.rs` file does not derive `Clone` for it; write \
                 `#[derive(Clone)]` above it there, or add `Clone` to its `#[derive(..)]` list \
                 (Varyk reads only `#[derive(..)]`, not a hand-written `impl`)"
            ),
        ],
        "{diagnostics:#?}"
    );
}

#[test]
fn imported_method_calls_carry_their_imported_ref() {
    let (result, _) = check_path("crates/varyk/tests/fixtures/interop/matcher/main.vr");
    let program = result.expect("should type-check");
    let tally = function(&program, "tally");
    let tail = tally.body.tail.as_deref().expect("tally ends in an `if`");
    let HirExprKind::If { cond, .. } = &tail.kind else {
        panic!("tally starts with an `if`");
    };
    assert!(matches!(
        cond.kind,
        HirExprKind::MethodCall {
            method: MethodRef::Imported(_),
            ..
        }
    ));
}

// --- Imported enums (M3 spec 4.3, 4.4) -----------------------------------

/// A Varyk struct field and a Varyk enum payload naming an imported enum
/// type-check without panicking (regression: the resolver's
/// recursive-type check used to index an imported `EnumId` past the end
/// of its per-item table, built for Varyk-declared enums only).
#[test]
fn a_varyk_struct_field_and_enum_payload_naming_an_imported_enum_type_check() {
    let (result, _) = check_path("crates/varyk/tests/fixtures/resolve/rust_enums/main.vr");
    result.expect("should type-check");
}

// --- An imported item naming a type behind a private module (M3 4.2) ------

#[test]
fn an_imported_call_and_field_naming_a_type_behind_a_private_module_work_where_it_is_seen() {
    let (result, _) = check_path("crates/varyk/tests/fixtures/interop/hidden_type_inside/main.vr");
    if let Err(diagnostics) = result {
        panic!("expected success, got {diagnostics:#?}");
    }
}

#[test]
fn an_imported_call_and_field_naming_a_type_behind_a_private_module_are_v0108_elsewhere() {
    let (result, sources) =
        check_path("crates/varyk/tests/fixtures/interop/hidden_type_outside/main.vr");
    let diagnostics = result.expect_err("should fail");
    let found: Vec<(&str, Span)> = diagnostics.iter().map(|d| (d.code, d.span)).collect();
    assert_eq!(
        found,
        vec![
            (codes::V0108, span_of(&sources, "shop::api::make()")),
            (codes::V0108, {
                let field = span_of(&sources, "b.h.x");
                Span::new(field.file, field.start + 2, field.start + 3)
            }),
        ],
        "{diagnostics:#?}"
    );
    assert!(
        diagnostics[0]
            .notes
            .iter()
            .any(|n| n.contains("which only `shop` can see")),
        "{diagnostics:#?}"
    );
}

/// The opaque-variant note puts `an` before a name starting with a vowel.
#[test]
fn opaque_variant_note_uses_the_right_article() {
    let (result, _) = check_path("crates/varyk/tests/fixtures/interop/opaque_enum_article/main.vr");
    let diagnostics = result.expect_err("a variant of an opaque enum is V0100");
    let d = only(&diagnostics);
    assert_eq!(d.code, codes::V0100);
    assert!(
        d.notes.iter().any(|n| n.contains("pass an `E` around")),
        "{d:#?}"
    );
}

/// A free function with a parameter behind `#[cfg]`, and a `#[test]`
/// function, each say which in the note.
#[test]
fn calling_a_rust_function_skipped_for_a_cfg_parameter_or_test_says_which() {
    let (result, _) = check_path("crates/varyk/tests/fixtures/interop/skipped_cfg_fns/main.vr");
    let diagnostics = result.expect_err("should fail");
    let found: Vec<_> = diagnostics
        .iter()
        .map(|d| (d.code, d.notes.first().map_or("", String::as_str)))
        .collect();
    assert_eq!(
        found,
        vec![
            (
                codes::V0100,
                "a parameter of `f` is behind `#[cfg]`, so `f` may not exist with this shape in \
                 the build; Varyk does not import such functions"
            ),
            (
                codes::V0100,
                "`t` exists in the Rust file but is marked `#[test]`; Varyk does not import such \
                 functions"
            ),
        ],
        "{diagnostics:#?}"
    );
}

// --- Milestone 4: patterns, exhaustiveness, `if let`, `while let` -----------

const PATTERN_ITEMS: &str = "enum Shape {\n    Circle(f64),\n    Rect(f64, f64),\n    Point,\n}\nenum Event {\n    Click { x: i32, y: i32 },\n    Key(string),\n    Quit,\n}\nenum Pair {\n    Two(i32, Option<i32>),\n}\nstruct Item {\n    done: bool,\n}\n";

/// A program with [`PATTERN_ITEMS`] and `body` as the body of `f`, whose
/// parameters cover every kind of value a pattern looks at.
fn with_patterns(body: &str) -> String {
    format!(
        "{PATTERN_ITEMS}fn f(o: Option<Shape>, n: i32, b: bool, s: string, u: u8, e: Event, r: Result<Option<i32>, string>, t: Option<Item>, p: Pair, fl: f64, so: Option<string>) {{\n{body}\n}}\nfn main() {{}}\n"
    )
}

#[test]
fn a_named_field_variant_value_names_every_field_once() {
    let program = ok(&with_patterns(
        "    let a = Event::Click { y: 2, x: 1 };\n    let b = Event::Quit;",
    ));
    let f = function(&program, "f");
    let value = stmt_expr(f, 0);
    let HirExprKind::EnumLit {
        variant,
        args,
        fields,
        ..
    } = &value.kind
    else {
        panic!("expected an enum literal, got {value:?}");
    };
    assert!(matches!(variant, VariantRef::User(_, 0)));
    assert_eq!(args.len(), 2);
    // In source order, each with its field's position in the declaration.
    assert_eq!(fields.as_deref(), Some(&[1, 0][..]));

    let cases = [
        (
            "let a = Event::Click { x: 1 };",
            codes::V0201,
            "Event::Click { x: 1 }",
            "`y`",
        ),
        (
            "let a = Event::Click { x: 1, y: 2, z: 3 };",
            codes::V0102,
            "z",
            "`z`",
        ),
        (
            "let a = Event::Key { k: 1 };",
            codes::V0201,
            "Event::Key { k: 1 }",
            "Event::Key(",
        ),
        (
            "let a = Event::Click(1, 2);",
            codes::V0201,
            "Event::Click(1, 2)",
            "{",
        ),
        ("let a = Event::Click;", codes::V0201, "Event::Click", "{"),
    ];
    for (stmt, code, at, word) in cases {
        let (d, sources) = one_error(&with_patterns(&format!("    {stmt}")));
        assert_eq!(d.code, code, "{stmt}: {d:#?}");
        assert_eq!(d.span, span_of(&sources, at), "{stmt}: {d:#?}");
        assert!(d.message.contains(word), "{stmt}: {d:#?}");
    }
    let (d, sources) = one_error(&with_patterns(
        "    let a = Event::Click { x: 1, x: 2, y: 3 };",
    ));
    assert_eq!(d.code, codes::V0103, "{d:#?}");
    let second = part_of(&sources, "x: 1, x: 2", "x: 2");
    assert_eq!(
        d.span,
        Span::new(second.file, second.start, second.start + 1)
    );
}

#[test]
fn a_variant_with_two_fields_of_one_name_is_v0103() {
    let (d, sources) = one_error("enum E {\n    A { x: i32, x: bool },\n}\nfn main() {}\n");
    assert_eq!(d.code, codes::V0103, "{d:#?}");
    assert_eq!(d.span, part_of(&sources, "x: bool", "x"));
}

#[test]
fn a_named_field_variant_pattern_names_every_field() {
    let program = ok(&with_patterns(
        "    match e {\n        Event::Click { x: 0, y: _ } => {}\n        Event::Click { y, x } => println!(\"{}\", x + y),\n        Event::Key(k) => {}\n        Event::Quit => {}\n    }",
    ));
    let f = function(&program, "f");
    assert_eq!(local_ty(f, "x"), I32);
    assert_eq!(local_ty(f, "k"), Ty::String);

    let cases = [
        (
            "Event::Click { x }",
            codes::V0205,
            "Event::Click { x }",
            "`y`",
        ),
        ("Event::Click { x, y, z }", codes::V0102, "z", "`z`"),
        ("Event::Click { x, x: _, y }", codes::V0103, "x: _", "`x`"),
        (
            "Event::Key { k }",
            codes::V0205,
            "Event::Key { k }",
            "Event::Key(",
        ),
        (
            "Event::Click(a, b)",
            codes::V0205,
            "Event::Click(a, b)",
            "{",
        ),
    ];
    for (pattern, code, at, word) in cases {
        let (d, sources) = one_error(&with_patterns(&format!(
            "    match e {{\n        {pattern} => {{}}\n        _ => {{}}\n    }}"
        )));
        assert_eq!(d.code, code, "{pattern}: {d:#?}");
        let at = if code == codes::V0103 {
            part_of(&sources, at, "x")
        } else {
            span_of(&sources, at)
        };
        assert_eq!(d.span, at, "{pattern}: {d:#?}");
        assert!(d.message.contains(word), "{pattern}: {d:#?}");
    }
}

#[test]
fn a_struct_pattern_on_a_plain_struct_is_v0001() {
    let (d, sources) = one_error(&with_patterns(
        "    match t {\n        Some(Item { done }) => {}\n        _ => {}\n    }",
    ));
    assert_eq!(d.code, codes::V0001, "{d:#?}");
    assert_eq!(d.span, span_of(&sources, "Item { done }"));
    assert!(d.message.contains("`if`"), "{d:#?}");
}

#[test]
fn nested_patterns_type_their_bindings() {
    let program = ok(&with_patterns(
        "    match o {\n        Some(Shape::Circle(radius)) => {}\n        Some(Shape::Rect(w, _)) => {}\n        Some(Shape::Point) => {}\n        None => {}\n    }\n    match r {\n        Ok(Some(inner)) => {}\n        Ok(None) => {}\n        Err(message) => {}\n    }\n    match p {\n        Pair::Two(a, Some(b)) => {}\n        Pair::Two(a, None) => {}\n    }",
    ));
    let f = function(&program, "f");
    assert_eq!(local_ty(f, "radius"), Ty::Float(FloatKind::F64));
    assert_eq!(local_ty(f, "w"), Ty::Float(FloatKind::F64));
    assert_eq!(local_ty(f, "inner"), I32);
    assert_eq!(local_ty(f, "message"), Ty::String);
    assert_eq!(local_ty(f, "b"), I32);
}

#[test]
fn a_name_bound_twice_inside_a_nested_pattern_is_v0103() {
    let (d, sources) = one_error(&with_patterns(
        "    match p {\n        Pair::Two(a, Some(a)) => {}\n        _ => {}\n    }",
    ));
    assert_eq!(d.code, codes::V0103, "{d:#?}");
    assert_eq!(d.span, part_of(&sources, "Some(a)", "a"));
}

#[test]
fn literal_and_range_patterns_must_fit_the_value() {
    let cases = [
        // (head, pattern, where the V0205 points, a word of the message)
        ("o", "Some(true)", "true", "bool"),
        ("n", "1.5", "1.5", "`if`"),
        ("n", "5..=1", "5..=1", "5"),
        ("u", "0..=300", "0..=300", "u8"),
        ("u", "300", "300", "u8"),
        ("u", "-1", "-1", "u8"),
        ("so", "Some(\"a\")", "\"a\"", "=="),
        ("n", "\"a\"", "\"a\"", "string"),
        ("s", "1", "1", "string"),
        ("n", "true..=false", "true..=false", "integer"),
        ("fl", "1", "1", "`if`"),
    ];
    for (head, pattern, at, word) in cases {
        let (d, sources) = one_error(&with_patterns(&format!(
            "    match {head} {{\n        {pattern} => {{}}\n        _ => {{}}\n    }}"
        )));
        assert_eq!(d.code, codes::V0205, "{pattern}: {d:#?}");
        assert_eq!(d.span, span_of(&sources, at), "{pattern}: {d:#?}");
        assert!(d.message.contains(word), "{pattern}: {word}: {d:#?}");
    }
}

#[test]
fn match_on_numbers_bools_and_strings_is_accepted() {
    ok(&with_patterns(
        "    match n {\n        -5..=-1 => {}\n        0 => {}\n        _ => {}\n    }\n    match b {\n        true => {}\n        false => {}\n    }\n    match s {\n        \"yes\" => {}\n        other => println!(\"{}\", other),\n    }\n    match u {\n        0..=9 => {}\n        255 => {}\n        _ => {}\n    }\n    match fl {\n        x => {}\n    }",
    ));
}

#[test]
fn a_match_on_a_struct_or_a_vec_is_v0205() {
    let (d, sources) = one_error(&with_patterns(
        "    let v = vec![1];\n    match v {\n        _ => {}\n    }",
    ));
    assert_eq!(d.code, codes::V0205, "{d:#?}");
    assert_eq!(d.span, part_of(&sources, "match v", "v"));
    assert!(d.message.contains("Vec<i32>"), "{d:#?}");
}

/// What the usefulness check says about a `match`: accepted, V0204 with a
/// witness, or V0205 at an arm.
enum Verdict {
    Ok,
    Missing(&'static str),
    Useless(&'static str),
}

/// Exhaustiveness and reachability, each case with rustc's verdict on the
/// same match (M4 spec 7): Varyk may be stricter than rustc (a
/// catch-all a number needs, an unreachable arm rustc only warns about),
/// never looser.
#[test]
fn exhaustiveness_and_reachability_agree_with_rustc() {
    use Verdict::{Missing, Ok as Accepted, Useless};
    let cases: &[(&str, Verdict)] = &[
        // rustc: E0004, `Some(Shape::Point)` not covered.
        (
            "match o {\n        Some(Shape::Circle(_)) => {}\n        Some(Shape::Rect(_, _)) => {}\n        None => {}\n    }",
            Missing("`Some(Shape::Point)`"),
        ),
        // rustc: accepted.
        (
            "match o {\n        Some(Shape::Circle(_)) => {}\n        Some(_) => {}\n        None => {}\n    }",
            Accepted,
        ),
        // rustc: E0004, `false` not covered.
        ("match b {\n        true => {}\n    }", Missing("`false`")),
        // rustc: accepted.
        (
            "match b {\n        true => {}\n        false => {}\n    }",
            Accepted,
        ),
        // rustc: accepted, the `_` unreachable (a warning).
        (
            "match b {\n        true => {}\n        false => {}\n        _ => {}\n    }",
            Useless("_ => {}"),
        ),
        // rustc: E0004, the other numbers not covered.
        (
            "match n {\n        1 => {}\n        2 => {}\n    }",
            Missing("`_ => ...`"),
        ),
        // rustc: accepted.
        (
            "match n {\n        1 => {}\n        _ => {}\n    }",
            Accepted,
        ),
        // rustc: E0004, `&_` not covered.
        (
            "match s {\n        \"a\" => {}\n    }",
            Missing("`_ => ...`"),
        ),
        // rustc: accepted; the other strings go to the catch-all.
        (
            "match s {\n        \"a\" => {}\n        _ => {}\n    }",
            Accepted,
        ),
        // rustc: accepted; Varyk still wants the catch-all (stricter).
        (
            "match u {\n        0..=255 => {}\n    }",
            Missing("`_ => ...`"),
        ),
        // rustc: accepted, the `_` unreachable (a warning); Varyk never
        // calls a number's catch-all unreachable.
        (
            "match u {\n        0..=255 => {}\n        _ => {}\n    }",
            Accepted,
        ),
        // rustc: accepted, `4` unreachable (a warning).
        (
            "match n {\n        1..=5 => {}\n        3..=8 => {}\n        4 => {}\n        _ => {}\n    }",
            Useless("4 => {}"),
        ),
        // rustc: accepted, `6` reachable.
        (
            "match n {\n        1..=5 => {}\n        6 => {}\n        _ => {}\n    }",
            Accepted,
        ),
        // rustc: accepted, the repeated range unreachable (a warning).
        (
            "match n {\n        1..=5 => {}\n        2..=3 => {}\n        _ => {}\n    }",
            Useless("2..=3 => {}"),
        ),
        // rustc: accepted, the second `\"a\"` unreachable (a warning).
        (
            "match s {\n        \"a\" => {}\n        \"a\" => {}\n        _ => {}\n    }",
            Useless("\"a\" => {}\n        _"),
        ),
        // rustc: accepted, the repeated `None` unreachable (a warning).
        (
            "match o {\n        None => {}\n        None => {}\n        Some(_) => {}\n    }",
            Useless("None => {}\n        Some"),
        ),
        // rustc: accepted, the arm after the catch-all unreachable.
        (
            "match o {\n        _ => {}\n        None => {}\n    }",
            Useless("_ => {}"),
        ),
        // rustc: accepted, the last arm unreachable.
        (
            "match o {\n        Some(_) => {}\n        None => {}\n        x => {}\n    }",
            Useless("x => {}"),
        ),
        // rustc: accepted, `Some(Shape::Point)` unreachable.
        (
            "match o {\n        Some(_) => {}\n        Some(Shape::Point) => {}\n        None => {}\n    }",
            Useless("Some(Shape::Point) => {}"),
        ),
        // rustc: E0004, `Event::Click { x: .., .. }` not covered.
        (
            "match e {\n        Event::Click { x: 0, y: _ } => {}\n        Event::Key(_) => {}\n        Event::Quit => {}\n    }",
            Missing("`Event::Click { x: _, y: _ }`"),
        ),
        // rustc: accepted.
        (
            "match e {\n        Event::Click { x: 0, y } => {}\n        Event::Click { x, y: _ } => {}\n        Event::Key(k) => {}\n        Event::Quit => {}\n    }",
            Accepted,
        ),
        // rustc: E0004, `Ok(None)` not covered.
        (
            "match r {\n        Ok(Some(_)) => {}\n        Err(_) => {}\n    }",
            Missing("`Ok(None)`"),
        ),
        // rustc: accepted.
        (
            "match n {\n        -5..=-1 => {}\n        0 => {}\n        _ => {}\n    }",
            Accepted,
        ),
    ];
    for (stmt, verdict) in cases {
        let text = with_patterns(&format!("    {stmt}"));
        match verdict {
            Accepted => {
                ok(&text);
            }
            Missing(witness) => {
                let (d, sources) = one_error(&text);
                assert_eq!(d.code, codes::V0204, "{stmt}: {d:#?}");
                assert!(d.message.contains(witness), "{stmt}: {d:#?}");
                let head = stmt.split(" {").next().unwrap_or_default();
                assert_eq!(d.span, span_of(&sources, head), "{stmt}: {d:#?}");
            }
            Useless(arm) => {
                let (d, sources) = one_error(&text);
                assert_eq!(d.code, codes::V0205, "{stmt}: {d:#?}");
                // `arm` may run on into the next arm to be unique; the
                // span is its first line.
                let at = span_of(&sources, stmt);
                let offset = stmt.find(arm).unwrap_or_default() as u32;
                let len = arm.split('\n').next().unwrap_or_default().len() as u32;
                assert_eq!(
                    d.span,
                    Span::new(at.file, at.start + offset, at.start + offset + len),
                    "{stmt}: {d:#?}"
                );
            }
        }
    }
}

#[test]
fn if_let_is_an_expression_typed_like_if() {
    let program = ok(&with_patterns(
        "    let x = if let Some(Shape::Circle(radius)) = o {\n        radius\n    } else {\n        0.0\n    };\n    if let Some(Shape::Point) = o {\n        println!(\"point\");\n    }\n    if let \"yes\" = s {\n    } else if let Some(n) = so {\n    }",
    ));
    let f = function(&program, "f");
    assert_eq!(local_ty(f, "x"), Ty::Float(FloatKind::F64));
    let value = stmt_expr(f, 0);
    assert!(matches!(value.kind, HirExprKind::IfLet { .. }), "{value:?}");

    let (d, _) = one_error(&with_patterns(
        "    let x = if let Some(y) = o {\n        1\n    };",
    ));
    assert_eq!(d.code, codes::V0200, "{d:#?}");
    let (d, _) = one_error(&with_patterns(
        "    let x = if let Some(y) = o {\n        1\n    } else {\n        \"no\"\n    };",
    ));
    assert_eq!(d.code, codes::V0200, "{d:#?}");
    // The binding lives only in the first block.
    let (d, _) = one_error(&with_patterns(
        "    if let Some(y) = o {\n    } else {\n        let z = y;\n    }",
    ));
    assert_eq!(d.code, codes::V0100, "{d:#?}");
}

#[test]
fn while_let_is_a_loop() {
    ok(&with_patterns(
        "    let mut v: Vec<i32> = vec![1, 2];\n    while let Some(n) = v.pop() {\n        if n == 1 {\n            break;\n        }\n        continue;\n    }",
    ));
    let (d, _) = one_error(&with_patterns(
        "    let mut v: Vec<i32> = vec![1, 2];\n    while let Some(n) = v.pop() {\n        n\n    }",
    ));
    assert_eq!(d.code, codes::V0200, "{d:#?}");
}

#[test]
fn if_let_and_while_let_heads_follow_the_rules_of_match() {
    for stmt in [
        "if let Some(x) = { o } {\n    }",
        "while let Some(x) = { o } {\n    }",
        "if let Some(x) = (if b { o } else { None }) {\n    }",
    ] {
        let (d, _) = one_error(&with_patterns(&format!("    {stmt}")));
        assert_eq!(d.code, codes::V0001, "{stmt}: {d:#?}");
        assert!(d.message.contains("`let`"), "{stmt}: {d:#?}");
    }
    // A nested string literal is V0205 in an `if let` too.
    let (d, _) = one_error(&with_patterns("    if let Some(\"a\") = so {\n    }"));
    assert_eq!(d.code, codes::V0205, "{d:#?}");
}

#[test]
fn a_string_head_is_marked() {
    let program = ok(&with_patterns(
        "    match s {\n        \"a\" => {}\n        _ => {}\n    }\n    match so {\n        Some(x) => {}\n        None => {}\n    }",
    ));
    let f = function(&program, "f");
    let tail = f.body.tail.as_deref().expect("a tail");
    let heads: Vec<bool> = [stmt_expr(f, 0), tail]
        .iter()
        .map(|expr| match &expr.kind {
            HirExprKind::Match { head_is_str, .. } => *head_is_str,
            other => panic!("expected a match, got {other:?}"),
        })
        .collect();
    assert_eq!(heads, vec![true, false]);
}

// --- `?` on `Option`, expected types through `?`, `as`, `..=` (M4 spec 2.6, 2.9, 2.11) ---

fn with_fn(params: &str, ret: &str, body: &str) -> String {
    format!("fn run({params}){ret} {{\n{body}\n}}\n\nfn main() {{}}\n")
}

#[test]
fn question_on_an_option_yields_the_payload() {
    let program = ok(&with_fn(
        "o: Option<i32>",
        " -> Option<i32>",
        "    let v = o?;\n    Some(v + 1)",
    ));
    let run = function(&program, "run");
    assert_eq!(local_ty(run, "v"), I32);
    match &stmt_expr(run, 0).kind {
        HirExprKind::Try { operand, kind } => {
            assert_eq!(*kind, TryKind::Option);
            assert_eq!(operand.ty, Ty::Option(Box::new(I32)));
        }
        other => panic!("expected `?`, got {other:?}"),
    }
}

#[test]
fn question_on_the_wrong_one_of_option_and_result_is_v0206_naming_both() {
    let (d, sources) = one_error(&with_fn(
        "o: Option<i32>",
        " -> Result<i32, string>",
        "    let v = o?;\n    Ok(v)",
    ));
    assert_eq!(d.code, codes::V0206, "{d:#?}");
    assert_eq!(d.span, span_of(&sources, "o?"));
    assert!(d.message.contains("`Option<i32>`"), "{d:#?}");
    assert!(d.message.contains("`Result<i32, string>`"), "{d:#?}");

    let (d, _) = one_error(&with_fn(
        "r: Result<i32, string>",
        " -> Option<i32>",
        "    let v = r?;\n    Some(v)",
    ));
    assert_eq!(d.code, codes::V0206, "{d:#?}");
    assert!(d.message.contains("`Result<i32, string>`"), "{d:#?}");
    assert!(d.message.contains("`Option<i32>`"), "{d:#?}");
}

#[test]
fn the_expected_type_flows_through_question() {
    let program = ok(&with_fn(
        "x: i32",
        " -> Option<i32>",
        "    let n: i32 = Some(x)?;\n    Some(n)",
    ));
    let run = function(&program, "run");
    assert_eq!(local_ty(run, "n"), I32);
    let program = ok(&with_fn(
        "x: i32",
        " -> Result<i32, string>",
        "    let n: i32 = Ok(x)?;\n    Ok(n)",
    ));
    let HirExprKind::Try { operand, .. } = &stmt_expr(function(&program, "run"), 0).kind else {
        panic!("expected `?`");
    };
    assert_eq!(operand.ty, Ty::Result(Box::new(I32), Box::new(Ty::String)));
    let HirExprKind::EnumLit {
        write_full_type, ..
    } = &operand.kind
    else {
        panic!("expected `Ok`");
    };
    assert!(*write_full_type);

    let (d, _) = one_error(&with_fn(
        "x: i32",
        " -> Option<i64>",
        "    let n: i64 = Some(x)?;\n    Some(n)",
    ));
    assert_eq!(d.code, codes::V0200, "{d:#?}");
}

#[test]
fn ok_question_without_an_expected_type_takes_the_functions_error_type() {
    let program = ok(&with_fn(
        "x: i32",
        " -> Result<i32, string>",
        "    let v = Ok(x)?;\n    Ok(v)",
    ));
    let run = function(&program, "run");
    assert_eq!(local_ty(run, "v"), I32);
    let HirExprKind::Try { operand, .. } = &stmt_expr(run, 0).kind else {
        panic!("expected `?`");
    };
    assert_eq!(operand.ty, Ty::Result(Box::new(I32), Box::new(Ty::String)));
}

#[test]
fn question_on_a_constructor_of_the_wrong_kind_is_v0206_whatever_is_expected() {
    let cases = [
        // In a function returning nothing.
        (
            "x: i32",
            "",
            "    let n = Ok(x)?;",
            "`Result<_, _>`",
            "nothing",
        ),
        (
            "x: i32",
            "",
            "    let n: i32 = Some(x)?;",
            "`Option<_>`",
            "nothing",
        ),
        ("e: string", "", "    Err(e)?;", "`Result<_, _>`", "nothing"),
        // In a function of the other family, with an expected type.
        (
            "x: i32",
            " -> Result<i32, string>",
            "    let n: i32 = Some(x)?;\n    Ok(n)",
            "`Option<_>`",
            "`Result<i32, string>`",
        ),
        (
            "x: i32",
            " -> Option<i32>",
            "    let n: i32 = Ok(x)?;\n    Some(n)",
            "`Result<_, _>`",
            "`Option<i32>`",
        ),
        (
            "",
            " -> Result<i32, string>",
            "    let n: i32 = None?;\n    Ok(n)",
            "`Option<_>`",
            "`Result<i32, string>`",
        ),
    ];
    for (params, ret, body, found, returns) in cases {
        let (d, _) = one_error(&with_fn(params, ret, body));
        assert_eq!(d.code, codes::V0206, "{body}: {d:#?}");
        assert!(d.message.contains(found), "{body}: {d:#?}");
        assert!(d.message.contains(returns), "{body}: {d:#?}");
    }
}

#[test]
fn err_question_as_a_statement_is_v0207() {
    let (d, sources) = one_error(&with_fn(
        "e: string",
        " -> Result<i32, string>",
        "    Err(e)?;\n    Ok(1)",
    ));
    assert_eq!(d.code, codes::V0207, "{d:#?}");
    assert_eq!(d.span, span_of(&sources, "Err(e)"));
}

#[test]
fn as_converts_between_number_types() {
    let program = ok(&with_fn(
        "x: i32, v: Vec<i32>, f: f64",
        "",
        "    let a = x as i64;\n    let n = v.len() as i32;\n    let g = f as u8;\n    let h = x as f32;",
    ));
    let run = function(&program, "run");
    assert_eq!(local_ty(run, "a"), I64);
    assert_eq!(local_ty(run, "n"), I32);
    assert_eq!(local_ty(run, "g"), Ty::Int(IntKind::U8));
    assert_eq!(local_ty(run, "h"), Ty::Float(FloatKind::F32));
    assert!(matches!(stmt_expr(run, 0).kind, HirExprKind::Cast { .. }));
}

#[test]
fn as_on_something_that_is_not_a_number_is_v0200() {
    for (params, body) in [
        ("b: bool", "    let n = b as i32;"),
        ("x: i32", "    let n = x as bool;"),
        ("s: string", "    let n = s as i32;"),
        ("x: i32", "    let n = x as string;"),
    ] {
        let (d, _) = one_error(&with_fn(params, "", body));
        assert_eq!(d.code, codes::V0200, "{body}: {d:#?}");
    }
    let (d, _) = one_error(
        "struct P {\n    x: i32,\n}\n\nfn main() {\n    let p = P { x: 1 };\n    let n = p as i32;\n}\n",
    );
    assert_eq!(d.code, codes::V0200, "{d:#?}");
}

#[test]
fn as_binds_tighter_than_multiplication() {
    let program = ok(&with_fn("a: i64", "", "    let n = a as i32 * 2;"));
    let run = function(&program, "run");
    assert_eq!(local_ty(run, "n"), I32);
    let HirExprKind::Binary { lhs, .. } = &stmt_expr(run, 0).kind else {
        panic!("expected a product");
    };
    assert!(matches!(lhs.kind, HirExprKind::Cast { .. }), "{lhs:?}");
}

#[test]
fn an_inclusive_range_counts_in_the_type_of_its_ends() {
    let program = ok(&with_fn(
        "n: usize",
        "",
        "    for i in 1..=n {\n        let k: usize = i;\n    }",
    ));
    let run = function(&program, "run");
    assert_eq!(local_ty(run, "i"), Ty::Int(IntKind::Usize));
    match &run.body.stmts[0] {
        HirStmt::For { head, .. } => assert!(matches!(
            head,
            HirForHead::Range {
                inclusive: true,
                ..
            }
        )),
        other => panic!("expected `for`, got {other:?}"),
    }

    let (d, _) = one_error(&with_fn(
        "x: i64",
        "",
        "    let s: i32 = 1;\n    for i in s..=x {\n    }",
    ));
    assert_eq!(d.code, codes::V0200, "{d:#?}");
}

// --- Table rows and `HashMap` (M4 spec 2.7) ---------------------------------

fn map_of(key: Ty, value: Ty) -> Ty {
    Ty::HashMap(Box::new(key), Box::new(value))
}

#[test]
fn the_value_rows_type_as_the_table_says() {
    let program = ok("fn main() {
    let mut v: Vec<i32> = vec![3, 1];
    let a = v.is_empty();
    v.insert(0, 5);
    let b = v.remove(1);
    let c = v.contains(3);
    v.sort();
    let words = vec![\"a\", \"b\"];
    let d = words.join(\", \");
    let mut s = \"text\";
    let e = s.is_empty();
    let f = s.contains(\"x\");
    let g = s.starts_with(\"t\");
    let h = s.to_uppercase();
    let i = s.replace(\"t\", \"T\");
    s.push_str(\"!\");
    let j: Result<i32, Error> = s.parse();
    let o: Option<i32> = None;
    let k = o.is_some();
    let r: Result<i32, string> = Ok(1);
    let l = r.is_ok();
    let m = r.is_err();
    let mut counts: HashMap<string, i32> = HashMap::new();
    let n = counts.insert(\"a\", 1);
    let p = counts.contains_key(\"a\");
    let q = counts.len();
}
");
    let main = function(&program, "main");
    for (name, ty) in [
        ("a", Ty::Bool),
        ("b", I32),
        ("c", Ty::Bool),
        ("d", Ty::String),
        ("e", Ty::Bool),
        ("f", Ty::Bool),
        ("g", Ty::Bool),
        ("h", Ty::String),
        ("i", Ty::String),
        ("j", Ty::Result(Box::new(I32), Box::new(Ty::Error))),
        ("k", Ty::Bool),
        ("l", Ty::Bool),
        ("m", Ty::Bool),
        ("counts", map_of(Ty::String, I32)),
        ("n", opt(I32)),
        ("p", Ty::Bool),
        ("q", USIZE),
    ] {
        assert_eq!(local_ty(main, name), ty, "{name}");
    }
}

#[test]
fn element_rules_are_v0200_naming_the_row() {
    for (stmts, at, words) in [
        (
            "let mut v: Vec<f64> = vec![1.5];\n    v.sort();",
            "sort",
            &[
                "`sort`",
                "Vec<f64>",
                "an integer type, `bool`, `string`, or `Time`",
            ][..],
        ),
        (
            "let mut v = vec![P { n: 1 }];\n    v.sort();",
            "sort",
            &["`sort`", "Vec<P>"][..],
        ),
        (
            "let v = vec![P { n: 1 }];\n    let b = v.contains(P { n: 1 });",
            "contains",
            &[
                "`contains`",
                "Vec<P>",
                "a number type, `bool`, `string`, `Time`, `Uuid`, or `Bytes`",
            ][..],
        ),
        (
            "let v = vec![1, 2];\n    let s = v.join(\",\");",
            "join",
            &["`join`", "Vec<i32>", "`Vec<string>`"][..],
        ),
    ] {
        let text = format!("struct P {{\n    n: i32,\n}}\nfn main() {{\n    {stmts}\n}}\n");
        let (d, sources) = one_error(&text);
        assert_eq!(d.code, codes::V0200, "{stmts}: {d:#?}");
        assert_eq!(
            d.span,
            part_of(&sources, &format!(".{at}("), at),
            "{stmts}: {d:#?}"
        );
        for word in words {
            assert!(d.message.contains(word), "{stmts}: {word}: {d:#?}");
        }
    }
}

#[test]
fn contains_and_sort_accept_integers_and_strings() {
    ok("fn main() {
    let mut v = vec![2, 1];
    v.sort();
    let a = v.contains(1);
    let mut w = vec![\"b\", \"a\"];
    w.sort();
    let b = w.contains(\"a\");
    let mut f = vec![true];
    f.sort();
    let x = vec![1.5];
    let c = x.contains(1.5);
}
");
}

#[test]
fn parse_into_a_string_is_v0200_and_with_nothing_expected_v0207() {
    let (d, sources) = one_error(
        "fn main() {\n    let s = \"a\";\n    let t: Result<string, Error> = s.parse();\n}\n",
    );
    assert_eq!(d.code, codes::V0200, "{d:#?}");
    assert_eq!(d.span, span_of(&sources, "s.parse()"));
    assert!(d.message.contains("`string`"), "{d:#?}");
    assert!(
        d.message.contains("a number, `bool`, `Time`, or `Uuid`"),
        "{d:#?}"
    );

    let (d, sources) = one_error("fn main() {\n    let s = \"a\";\n    let n = s.parse();\n}\n");
    assert_eq!(d.code, codes::V0207, "{d:#?}");
    assert_eq!(d.span, span_of(&sources, "s.parse()"));
    assert!(
        d.message
            .contains("`let parsed: Result<i32, Error> = text.parse();` then `parsed.ok()`"),
        "{d:#?}"
    );
}

#[test]
fn hash_map_new_takes_its_type_from_where_it_goes() {
    let program = ok("fn f(m: HashMap<i32, string>) {}
fn g() -> HashMap<string, Vec<i32>> {
    HashMap::new()
}
fn main() {
    f(HashMap::new());
}
");
    let main = function(&program, "main");
    assert_eq!(call_args(stmt_expr(main, 0))[0].ty, map_of(I32, Ty::String));
    let (d, sources) = one_error("fn main() {\n    let m = HashMap::new();\n}\n");
    assert_eq!(d.code, codes::V0207, "{d:#?}");
    assert_eq!(d.span, span_of(&sources, "HashMap::new()"));
    assert!(
        d.message
            .contains("`let m: HashMap<string, i32> = HashMap::new();`"),
        "{d:#?}"
    );
    let (d, _) = one_error("fn main() {\n    let m: Vec<i32> = HashMap::new();\n}\n");
    assert_eq!(d.code, codes::V0200, "{d:#?}");
    let (d, _) = one_error("fn main() {\n    let m: HashMap<i32, i32> = HashMap::nope();\n}\n");
    assert_eq!(d.code, codes::V0100, "{d:#?}");
    assert!(d.message.contains("`HashMap::new()`"), "{d:#?}");
}

#[test]
fn methods_of_option_result_and_hash_map_are_listed_for_v0100() {
    for (stmts, words) in [
        (
            "let o: Option<i32> = None;\n    o.nope();",
            &[
                "Option<i32>",
                "the methods of an `Option` are `is_some`, `unwrap_or`, `ok_or`, and `map`",
            ][..],
        ),
        (
            "let r: Result<i32, string> = Ok(1);\n    r.nope();",
            &[
                "the methods of a `Result` are `is_ok`, `is_err`, `ok`, `unwrap_or`, and \
                 `map_err`",
            ][..],
        ),
        (
            "let m: HashMap<i32, i32> = HashMap::new();\n    m.nope();",
            &[
                "the methods of a `HashMap` are `insert`, `contains_key`, `len`, `get`, `keys`, \
                 and `values`",
            ][..],
        ),
    ] {
        let (d, _) = one_error(&format!("fn main() {{\n    {stmts}\n}}\n"));
        assert_eq!(d.code, codes::V0100, "{stmts}: {d:#?}");
        for word in words {
            assert!(d.message.contains(word), "{stmts}: {word}: {d:#?}");
        }
    }
}

#[test]
fn for_over_a_hash_map_is_v0001_naming_keys_and_values() {
    let (d, sources) = one_error(
        "fn main() {\n    let m: HashMap<string, i32> = HashMap::new();\n    for k in m {\n    }\n}\n",
    );
    assert_eq!(d.code, codes::V0001, "{d:#?}");
    assert_eq!(d.span, part_of(&sources, "in m {", "m"));
    assert!(d.message.contains("`keys()`"), "{d:#?}");
    assert!(d.message.contains("`values()`"), "{d:#?}");
}

#[test]
fn a_hash_map_is_not_printed() {
    let (d, _) = one_error(
        "fn main() {\n    let m: HashMap<i32, i32> = HashMap::new();\n    println!(\"{}\", m);\n}\n",
    );
    assert_eq!(d.code, codes::V0203, "{d:#?}");
    assert!(d.message.contains("HashMap<i32, i32>"), "{d:#?}");
}

#[test]
fn a_hash_map_node_type_checks() {
    ok("struct Node {
    children: HashMap<string, Node>,
}
fn main() {
    let n = Node { children: HashMap::new() };
    let k = n.children.len();
}
");
}

#[test]
fn hash_map_as_a_let_name_is_v0103() {
    let (d, _) = one_error("fn main() {\n    let HashMap = 1;\n}\n");
    assert_eq!(d.code, codes::V0103, "{d:#?}");
    assert_eq!(
        d.message,
        "the name `HashMap` is already taken by a built-in type"
    );
}

#[test]
fn parse_where_another_result_or_an_option_is_expected_is_a_mismatch_or_v0206() {
    // A `Result` of another error type, or an `Option`, expected anywhere
    // but under `?` is a plain mismatch.
    for written in ["Result<i32, string>", "Option<i32>"] {
        let (d, _) = one_error(&format!(
            "fn main() {{\n    let s = \"1\";\n    let r: {written} = s.parse();\n}}\n"
        ));
        assert_eq!(d.code, codes::V0200, "{d:#?}");
    }
    let (d, _) = one_error(
        "fn f(text: string) -> Result<i32, string> {\n    let r: Result<i32, string> = Ok(text.parse()?);\n    r\n}\nfn main() {}\n",
    );
    assert_eq!(d.code, codes::V0206, "{d:#?}");
}

#[test]
fn ok_or_takes_its_error_type_from_an_expected_result_else_from_its_argument() {
    let program = ok(
        "fn f(o: Option<i32>) -> Result<i32, i64> {\n    let a: Result<i32, i64> = o.ok_or(1);\n    let b = o.ok_or(2);\n    let c = o.ok_or(\"none\");\n    let n: i32 = o.ok_or(3)?;\n    Ok(n)\n}\nfn main() {}\n",
    );
    let f = function(&program, "f");
    let result = |e: Ty| Ty::Result(Box::new(I32), Box::new(e));
    assert_eq!(local_ty(f, "a"), result(I64));
    assert_eq!(local_ty(f, "b"), result(I32));
    assert_eq!(local_ty(f, "c"), result(Ty::String));
    // Through `?`, from the function's error type.
    let HirExprKind::Try { operand, .. } = &stmt_expr(f, 3).kind else {
        panic!("a `?`");
    };
    assert_eq!(operand.ty, result(I64));
    let (d, _) = one_error(
        "fn f(o: Option<i32>) {\n    let a: Result<i32, i64> = o.ok_or(\"x\");\n}\nfn main() {}\n",
    );
    assert_eq!(d.code, codes::V0200, "{d:#?}");
}

#[test]
fn parse_then_ok_or_under_question_is_v0207_showing_two_statements() {
    let (d, sources) = one_error(
        "fn f(text: string) -> Result<i32, string> {\n    let n = text.parse().ok_or(\"bad\")?;\n    Ok(n)\n}\nfn main() {}\n",
    );
    assert_eq!(d.code, codes::V0207, "{d:#?}");
    assert_eq!(d.span, span_of(&sources, "text.parse()"));
    assert!(
        d.notes
            .iter()
            .chain(std::iter::once(&d.message))
            .any(|n| n.contains("parsed.ok()")),
        "{d:#?}"
    );
}

#[test]
fn a_get_is_rooted_at_its_receiver_only_when_its_payload_is_not_copy() {
    let program = ok(
        "fn f(v: Vec<string>, n: Vec<i32>, m: HashMap<string, Vec<i32>>, c: HashMap<i32, bool>) {\n    let a = v.get(0);\n    let b = n.get(0);\n    let d = m.get(\"k\");\n    let e = c.get(1);\n}\nfn main() {}\n",
    );
    let f = function(&program, "f");
    let rooted = |index| match &stmt_expr(f, index).kind {
        HirExprKind::MethodCall { rooted, .. } => *rooted,
        other => panic!("a method call, got {other:?}"),
    };
    assert_eq!(rooted(0), Some(0));
    assert_eq!(rooted(1), None);
    assert_eq!(rooted(2), Some(0));
    assert_eq!(rooted(3), None);
    assert_eq!(local_ty(f, "b"), Ty::Option(Box::new(I32)));
    assert_eq!(
        local_ty(f, "d"),
        Ty::Option(Box::new(Ty::Vec(Box::new(I32))))
    );
}

#[test]
fn trim_is_rooted_at_its_receiver_and_calls_are_left_unrooted() {
    let program = ok(
        "struct U {\n    name: string,\n}\nimpl U {\n    fn name_ref(self) -> string {\n        self.name.clone()\n    }\n}\nfn first(s: string) -> string {\n    s.clone()\n}\nfn f(s: string, u: U) {\n    let a = s.trim();\n    let b = \"  x \".trim();\n    let c = u.name_ref();\n    let d = first(s);\n}\nfn main() {}\n",
    );
    let f = function(&program, "f");
    let rooted = |index| match &stmt_expr(f, index).kind {
        HirExprKind::MethodCall { rooted, .. } | HirExprKind::Call { rooted, .. } => *rooted,
        other => panic!("a call, got {other:?}"),
    };
    assert_eq!(rooted(0), Some(0));
    assert_eq!(rooted(1), Some(0));
    // Borrow analysis decides what a Varyk function returns (M4 spec 3.1).
    assert_eq!(rooted(2), None);
    assert_eq!(rooted(3), None);
    assert_eq!(local_ty(f, "a"), Ty::String);
    assert_eq!(f.ret_root, None);
}

/// Deliberately over-strict: a head is judged by its shape, before borrow
/// analysis knows which calls are borrowed returns, so a field of one is
/// "part of a value that is not stored anywhere" (M4 spec 3.1).
#[test]
fn a_field_of_a_call_as_a_head_stays_v0001() {
    let (d, sources) = one_error(
        "enum K {\n    A,\n    B,\n}\nstruct Info {\n    kind: K,\n}\nstruct Obj {\n    info: Info,\n}\nimpl Obj {\n    fn info(self) -> Info {\n        self.info\n    }\n}\nfn f(obj: Obj) {\n    match obj.info().kind {\n        K::A => {}\n        K::B => {}\n    }\n}\nfn main() {}\n",
    );
    assert_eq!(d.code, codes::V0001, "{d:#?}");
    assert_eq!(d.span, span_of(&sources, "obj.info().kind"));
}

// --- Chains (M4 spec 2.3, 2.7) ----------------------------------------------

/// `main` with a `Vec<i32>` `v`, a `Vec<string>` `n`, a `HashMap` `m`, and
/// a `string` `text`, around `stmts`, beside helpers taking and returning
/// values.
fn with_chains(stmts: &str) -> String {
    format!(
        "struct S {{\n    n: i32,\n}}\nfn take(n: i32) {{}}\nfn tail(v: Vec<i32>) -> i32 {{\n    0\n}}\nfn main() {{\n    let v = vec![1, 2];\n    let n = vec![\"a\", \"b\"];\n    let mut m: HashMap<string, i32> = HashMap::new();\n    let text = \"a b\";\n    {stmts}\n}}\n"
    )
}

/// An unfinished chain has no Varyk type: anywhere but the receiver of
/// the next call it is V0208, "finish the chain here" (M4 spec 2.3).
#[test]
fn an_unfinished_chain_anywhere_but_the_next_call_is_v0208() {
    let cases = [
        ("let c = v.iter();", "v.iter()"),
        ("let c: Vec<i32> = v.iter();", "v.iter()"),
        ("take(v.iter());", "v.iter()"),
        ("v.iter().map(|x| x);", "v.iter().map(|x| x)"),
        ("let s = S { n: v.iter() };", "v.iter()"),
        ("let o = Some(v.iter());", "v.iter()"),
        ("let w = vec![v.iter()];", "v.iter()"),
        ("println!(\"{}\", v.iter());", "v.iter()"),
        ("while true {\n        v.iter()\n    }", "v.iter()"),
        ("for x in v {\n        v.iter()\n    }", "v.iter()"),
        (
            "let k = (if true { v.iter() } else { v.iter() }).count();",
            "if true { v.iter() } else { v.iter() }",
        ),
        ("let d = v.iter().map(|x| n.iter()).count();", "n.iter()"),
        ("let e = Some(1).map(|x| n.iter());", "n.iter()"),
        ("match v.iter() {\n        _ => {}\n    }", "v.iter()"),
        ("if let x = v.iter() {}", "v.iter()"),
        (
            "while let x = v.iter() {\n        break;\n    }",
            "v.iter()",
        ),
    ];
    for (stmt, at) in cases {
        let (d, sources) = one_error(&with_chains(stmt));
        assert_eq!(d.code, codes::V0208, "{stmt}: {d:#?}");
        assert_eq!(d.span, span_of(&sources, at), "{stmt}: {d:#?}");
        assert!(
            d.message.contains("finish the chain here"),
            "{stmt}: {d:#?}"
        );
    }
    let returns = [
        "fn f(v: Vec<i32>) -> i32 {\n    v.iter()\n}\nfn main() {}\n",
        "fn f(v: Vec<i32>) -> i32 {\n    return v.iter();\n}\nfn main() {}\n",
    ];
    for text in returns {
        let (d, sources) = one_error(text);
        assert_eq!(d.code, codes::V0208, "{text}: {d:#?}");
        assert_eq!(d.span, span_of(&sources, "v.iter()"), "{text}");
    }
    // Finished, a chain is an ordinary value.
    ok(&with_chains(
        "let a = v.iter().count();\n    take(v.iter().sum());\n    let b = v.iter().map(|x| x * 2).filter(|x| x > 2).any(|x| x == 4);",
    ));
}

#[test]
fn a_chain_types_its_items_from_its_source() {
    let program = ok(&with_chains(
        "let a = v.iter().collect();\n    let b = n.iter().map(|s| s.len()).collect();\n    let c = text.split(\" \").filter(|w| w.len() > 0).count();\n    let d = m.keys().all(|k| k.len() > 0);\n    let e = m.values().sum();\n    let f = v.iter().find(|x| x > 1);\n    let g = n.iter().map(|s| s.clone()).collect();",
    ));
    let main = function(&program, "main");
    let vec = |ty: Ty| Ty::Vec(Box::new(ty));
    assert_eq!(local_ty(main, "a"), vec(I32));
    assert_eq!(local_ty(main, "b"), vec(Ty::Int(IntKind::Usize)));
    assert_eq!(local_ty(main, "s"), Ty::String);
    assert_eq!(local_ty(main, "w"), Ty::String);
    assert_eq!(local_ty(main, "c"), Ty::Int(IntKind::Usize));
    assert_eq!(local_ty(main, "k"), Ty::String);
    assert_eq!(local_ty(main, "d"), Ty::Bool);
    assert_eq!(local_ty(main, "e"), I32);
    assert_eq!(local_ty(main, "f"), Ty::Option(Box::new(I32)));
    assert_eq!(local_ty(main, "g"), vec(Ty::String));
    // `find` is looked into or not by its items, which borrow analysis
    // decides: the checker roots nothing.
    let HirExprKind::MethodCall {
        rooted,
        looked_into,
        ..
    } = &stmt_expr(main, 9).kind
    else {
        panic!("a method call");
    };
    assert_eq!(*rooted, None);
    assert!(!looked_into);
}

#[test]
fn sum_on_items_that_are_not_numbers_is_v0200() {
    let (d, sources) = one_error(&with_chains("let t = n.iter().sum();"));
    assert_eq!(d.code, codes::V0200, "{d:#?}");
    assert_eq!(d.span, span_of(&sources, "sum"));
    assert!(d.message.contains("number"), "{d:#?}");
    assert!(d.message.contains("`string`"), "{d:#?}");
}

#[test]
fn a_test_closure_of_a_chain_must_return_bool() {
    for call in ["filter", "any", "all", "find"] {
        let stmt = format!("let t = v.iter().{call}(|x| x + 1);");
        let (d, sources) = one_error(&with_chains(&stmt));
        assert_eq!(d.code, codes::V0200, "{call}: {d:#?}");
        assert_eq!(d.span, span_of(&sources, "x + 1"), "{call}");
        assert!(d.message.contains("expected `bool`"), "{call}: {d:#?}");
    }
}

#[test]
fn a_method_a_chain_lacks_is_v0100_listing_its_calls() {
    let (d, sources) = one_error(&with_chains("let t = v.iter().len();"));
    assert_eq!(d.code, codes::V0100, "{d:#?}");
    assert_eq!(d.span, span_of(&sources, "len"));
    assert!(
        d.message.contains(
            "the calls of a chain are `map`, `filter`, `collect`, `count`, `sum`, `any`, \
             `all`, and `find`"
        ),
        "{d:#?}"
    );
}

/// A chain as a `for` head binds one item per round (M4 spec 2.3, 3.3):
/// the variable has the item type, and the head is no unfinished chain.
#[test]
fn a_for_over_a_chain_types_its_variable_as_the_item() {
    let program = ok(&with_chains(
        "for x in v.iter().filter(|y| y > 1) {\n        take(x);\n    }\n    for w in text.split(\" \") {\n        let k = w.len();\n    }",
    ));
    let main = function(&program, "main");
    assert_eq!(local_ty(main, "x"), I32);
    assert_eq!(local_ty(main, "w"), Ty::String);
    // Anything but a chain call as the head is still unfinished.
    let (d, sources) = one_error(&with_chains(
        "for x in if true { v.iter() } else { v.iter() } {\n    }",
    ));
    assert_eq!(d.code, codes::V0208, "{d:#?}");
    assert_eq!(
        d.span,
        span_of(&sources, "if true { v.iter() } else { v.iter() }")
    );
}

// --- `Error` and `parse` as a `Result` (M5a spec 2.3, 2.8) ------------------

fn result_of(ok: Ty) -> Ty {
    Ty::Result(Box::new(ok), Box::new(Ty::Error))
}

#[test]
fn error_new_makes_an_error_and_message_is_part_of_it() {
    let program = ok("fn main() {
    let e = Error::new(\"x\");
    let m = e.message();
    println!(\"{}\", e);
    let same = e == Error::new(\"y\");
    let copy = e.clone();
}
");
    let main = function(&program, "main");
    assert_eq!(local_ty(main, "e"), Ty::Error);
    assert_eq!(local_ty(main, "m"), Ty::String);
    assert_eq!(local_ty(main, "same"), Ty::Bool);
    assert_eq!(local_ty(main, "copy"), Ty::Error);
    // `message` is part of its receiver, as `trim` is (M4 spec 3.1).
    let HirExprKind::MethodCall { rooted, .. } = &stmt_expr(main, 1).kind else {
        panic!("a method call");
    };
    assert_eq!(*rooted, Some(0));
    assert!(program.uses_std);
}

#[test]
fn error_with_status_makes_an_error() {
    let program = ok("fn gone() -> Error {
    Error::with_status(404, \"gone\")
}
fn copy(text: string) -> Error {
    Error::with_status(409, text.clone())
}
fn main() {
    let e = Error::with_status(404, \"gone\");
    let s = e.status();
}
");
    let main = function(&program, "main");
    assert_eq!(local_ty(main, "e"), Ty::Error);
    assert_eq!(
        local_ty(main, "s"),
        Ty::Option(Box::new(Ty::Int(IntKind::U16)))
    );
    assert!(program.uses_std);
}

#[test]
fn error_with_status_wants_a_u16_status() {
    let (d, _) = one_error("fn main() {\n    let e = Error::with_status(\"404\", \"gone\");\n}\n");
    assert_eq!(d.code, codes::V0200, "{d:#?}");
    let (d, _) = one_error(
        "fn main() {\n    let n: i32 = 404;\n    let e = Error::with_status(n, \"gone\");\n}\n",
    );
    assert_eq!(d.code, codes::V0200, "{d:#?}");
}

#[test]
fn a_struct_holding_an_error_keeps_clone_and_eq() {
    let program = ok("struct Failure {
    error: Error,
    code: i32,
}
fn main() {
    let a = Failure { error: Error::new(\"x\"), code: 1 };
    let b = a.clone();
    let same = a == b;
}
");
    assert_eq!(
        program.structs[0].derives,
        Derives {
            clone: true,
            eq: true
        }
    );
}

#[test]
fn parse_gives_a_result_with_an_error() {
    let program = ok("fn number(text: string) -> Result<i32, Error> {
    let n: i32 = text.parse()?;
    Ok(n)
}
fn maybe(text: string) -> Option<i32> {
    let r: Result<i32, Error> = text.parse();
    r.ok()
}
fn take(r: Result<u8, Error>) {}
fn main() {
    let a: Result<f64, Error> = \"1.5\".parse();
    take(\"7\".parse());
}
");
    let number = function(&program, "number");
    assert_eq!(local_ty(number, "n"), I32);
    let maybe = function(&program, "maybe");
    assert_eq!(local_ty(maybe, "r"), result_of(I32));
    let main = function(&program, "main");
    assert_eq!(local_ty(main, "a"), result_of(Ty::Float(FloatKind::F64)));
    assert_eq!(
        call_args(stmt_expr(main, 1))[0].ty,
        result_of(Ty::Int(IntKind::U8))
    );
    assert!(program.uses_std);
}

#[test]
fn parse_under_question_with_another_error_type_is_v0206() {
    let (d, sources) = one_error(
        "fn f(text: string) -> Result<i32, string> {\n    let n: i32 = text.parse()?;\n    Ok(n)\n}\nfn main() {}\n",
    );
    assert_eq!(d.code, codes::V0206, "{d:#?}");
    assert_eq!(d.span, span_of(&sources, "text.parse()?"));
    assert!(d.message.contains("`Result<i32, Error>`"), "{d:#?}");
    assert!(d.message.contains("`Result<i32, string>`"), "{d:#?}");
}

#[test]
fn parse_under_question_in_an_option_function_is_v0206_showing_two_statements() {
    let (d, sources) = one_error(
        "fn f(text: string) -> Option<i32> {\n    let n: i32 = text.parse()?;\n    Some(n)\n}\nfn main() {}\n",
    );
    assert_eq!(d.code, codes::V0206, "{d:#?}");
    assert_eq!(d.span, span_of(&sources, "text.parse()"));
    assert!(d.message.contains("`Result<_, Error>`"), "{d:#?}");
    assert!(
        d.notes
            .iter()
            .any(|n| n
                .contains("`let parsed: Result<i32, Error> = text.parse();` then `parsed.ok()?`")),
        "{d:#?}"
    );
}

#[test]
fn a_program_without_error_or_parse_does_not_use_varyk_std() {
    let program = ok("fn main() {\n    let s = \"a\";\n    println!(\"{}\", s.trim());\n}\n");
    assert!(!program.uses_std);
    // `Error` named only in a signature is enough (M5a spec 1).
    let program = ok("fn f() -> Result<i32, Error> {\n    Ok(1)\n}\nfn main() {}\n");
    assert!(program.uses_std);
    let program = ok("struct S {\n    e: Option<Error>,\n}\nfn main() {}\n");
    assert!(program.uses_std);
}

#[test]
fn a_type_named_error_is_v0113() {
    for text in [
        "struct Error {\n    n: i32,\n}\nfn main() {}\n",
        "enum Error {\n    Bad,\n}\nfn main() {}\n",
    ] {
        let (d, sources) = one_error(text);
        assert_eq!(d.code, codes::V0113, "{d:#?}");
        assert_eq!(d.span, part_of(&sources, "Error {", "Error"));
        assert!(d.message.contains("`Error`"), "{d:#?}");
    }
}

#[test]
fn a_call_error_does_not_have_is_v0100_listing_its_calls() {
    let (d, _) =
        one_error("fn main() {\n    let e = Error::new(\"x\");\n    let n = e.len();\n}\n");
    assert_eq!(d.code, codes::V0100, "{d:#?}");
    assert!(
        d.message
            .contains("the methods of an `Error` are `message`"),
        "{d:#?}"
    );
    let (d, _) = one_error("fn main() {\n    let e = Error::other(\"x\");\n}\n");
    assert_eq!(d.code, codes::V0100, "{d:#?}");
    assert!(
        d.message
            .contains("its functions are `Error::new(..)` and `Error::with_status(..)`"),
        "{d:#?}"
    );
}

// --- `Time`, `Uuid`, and `Bytes` (milestone 5c spec 2.1 to 2.3) -------------

/// A program calling every method of `Time` on `t` and of `Bytes` on `b`,
/// which a local, a field, or a parameter stands in for, and every
/// associated function of the three.
fn time_calls(t: &str, b: &str) -> String {
    format!(
        "struct Stamp {{\n    at: Time,\n    data: Bytes,\n}}\n\
         fn calls(p: Time, d: Bytes, s: Stamp) {{\n    \
         let now = Time::now();\n    \
         let bytes = Bytes::from_text(\"hi\");\n    \
         let iso = {t}.to_iso();\n    \
         let unix = {t}.to_unix();\n    \
         let micros = {t}.to_unix_micros();\n    \
         let added = {t}.add_seconds(-5);\n    \
         let since = {t}.seconds_since(now);\n    \
         let text = {b}.to_text();\n    \
         let base = {b}.to_base64();\n    \
         let size = {b}.len();\n    \
         let empty = {b}.is_empty();\n    \
         let read = Time::from_iso(\"2026-10-07T12:00:00Z\");\n    \
         let seconds = Time::from_unix(0);\n    \
         let exact = Time::from_unix_micros(0);\n    \
         let id = Uuid::new();\n    \
         let ordered = Uuid::v7();\n    \
         let random = Uuid::v4();\n    \
         let decoded = Bytes::from_base64(\"aGk=\");\n}}\n\
         fn main() {{}}\n"
    )
}

#[test]
fn every_time_uuid_and_bytes_call_types_as_its_table_says() {
    for (t, b) in [("now", "bytes"), ("s.at", "s.data"), ("p", "d")] {
        let program = ok(&time_calls(t, b));
        let calls = function(&program, "calls");
        let expected = [
            ("now", Ty::Time),
            ("bytes", Ty::Bytes),
            ("iso", Ty::String),
            ("unix", I64),
            ("micros", I64),
            ("added", result_of(Ty::Time)),
            ("since", I64),
            ("text", result_of(Ty::String)),
            ("base", Ty::String),
            ("size", Ty::Int(IntKind::Usize)),
            ("empty", Ty::Bool),
            ("read", result_of(Ty::Time)),
            ("seconds", result_of(Ty::Time)),
            ("exact", result_of(Ty::Time)),
            ("id", Ty::Uuid),
            ("ordered", Ty::Uuid),
            ("random", Ty::Uuid),
            ("decoded", result_of(Ty::Bytes)),
        ];
        for (name, ty) in expected {
            assert_eq!(local_ty(calls, name), ty, "{name} with {t} and {b}");
        }
        assert!(program.uses_std, "{t}");
    }
}

#[test]
fn naming_time_uuid_or_bytes_alone_uses_varyk_std() {
    for name in ["Time", "Uuid", "Bytes"] {
        let program = ok(&format!(
            "struct S {{\n    x: {name},\n}}\nfn main() {{}}\n"
        ));
        assert!(program.uses_std, "{name}");
    }
}

#[test]
fn a_call_time_uuid_or_bytes_lacks_is_v0100_listing_its_calls() {
    let cases = [
        (
            "let t = Time::now();\n    let x = t.later();",
            "type `Time` has no method `later`; the methods of a `Time` are `to_iso`, \
             `to_unix`, `to_unix_micros`, `add_seconds`, and `seconds_since`",
        ),
        (
            "let x = Time::today();",
            "`Time` has no function `today`; its functions are `Time::now()`, \
             `Time::from_iso(..)`, `Time::from_unix(..)`, and `Time::from_unix_micros(..)`",
        ),
        (
            "let x = Uuid::v5();",
            "`Uuid` has no function `v5`; its functions are `Uuid::new()`, `Uuid::v7()`, \
             and `Uuid::v4()`",
        ),
        (
            "let u = Uuid::new();\n    let x = u.version();",
            "type `Uuid` has no methods",
        ),
        (
            "let x = Bytes::from_hex(\"00\");",
            "`Bytes` has no function `from_hex`; its functions are `Bytes::from_text(..)` and \
             `Bytes::from_base64(..)`",
        ),
        (
            "let b = Bytes::from_text(\"x\");\n    let x = b.slice();",
            "type `Bytes` has no method `slice`; the methods of a `Bytes` are `to_text`, \
             `to_base64`, `len`, and `is_empty`",
        ),
        // A method is not a function of the type.
        (
            "let x = Time::to_iso();",
            "`Time` has no function `to_iso`; its functions are `Time::now()`, \
             `Time::from_iso(..)`, `Time::from_unix(..)`, and `Time::from_unix_micros(..)`",
        ),
    ];
    for (body, message) in cases {
        let text = format!("fn main() {{\n    {body}\n}}\n");
        let (d, _) = one_error(&text);
        assert_eq!(d.code, codes::V0100, "{text}\n{d:#?}");
        assert_eq!(d.message, message, "{text}");
    }
}

#[test]
fn a_wrong_argument_to_a_time_uuid_or_bytes_call_is_v0200_and_a_wrong_count_v0201() {
    for body in [
        "let x = Time::from_unix(\"0\");",
        "let x = Time::from_unix_micros(1.5);",
        "let x = Time::from_iso(1);",
        "let t = Time::now();\n    let x = t.add_seconds(true);",
        "let t = Time::now();\n    let x = t.seconds_since(1);",
        "let n: i32 = 5;\n    let x = Time::now().add_seconds(n);",
        "let x = Bytes::from_text(1);",
        "let x = Bytes::from_base64(Bytes::from_text(\"x\"));",
        // A `Time` where a `Uuid` is expected, and the other way.
        "let u: Uuid = Time::now();",
        "let t = Time::now();\n    let x = t.seconds_since(Uuid::new());",
        "take(Time::now());",
    ] {
        let text = format!("fn take(id: Uuid) {{}}\nfn main() {{\n    {body}\n}}\n");
        let (d, _) = one_error(&text);
        assert_eq!(d.code, codes::V0200, "{text}\n{d:#?}");
    }
    for body in [
        "let x = Time::now(1);",
        "let x = Time::from_unix();",
        "let x = Uuid::v4(1);",
        "let x = Bytes::from_text();",
        "let t = Time::now();\n    let x = t.to_iso(1);",
        "let t = Time::now();\n    let x = t.add_seconds();",
        "let b = Bytes::from_text(\"x\");\n    let x = b.len(1);",
    ] {
        let text = format!("fn main() {{\n    {body}\n}}\n");
        let (d, _) = one_error(&text);
        assert_eq!(d.code, codes::V0201, "{text}\n{d:#?}");
    }
}

// --- Comparing, printing, parsing, and copying `Time`, `Uuid`, and `Bytes`
// (milestone 5c spec 2.1 to 2.4) ---------------------------------------------

/// `body` inside `main`, after a `Time` `t` and `u`, a `Uuid` `i` and `j`,
/// and a `Bytes` `b` and `c`.
fn with_values(body: &str) -> String {
    format!(
        "fn main() {{\n    let t = Time::now();\n    let u = Time::now();\n    \
         let i = Uuid::new();\n    let j = Uuid::v4();\n    \
         let b = Bytes::from_text(\"x\");\n    let c = Bytes::from_text(\"y\");\n    \
         {body}\n}}\n"
    )
}

#[test]
fn times_are_ordered_and_every_one_of_the_three_compared() {
    let program = ok(&with_values(
        "let a = t < u;\n    let d = t <= u;\n    let e = t > u;\n    let f = t >= u;\n    \
         let g = t == u;\n    let h = t != u;\n    let k = i == j;\n    let l = i != j;\n    \
         let m = b == c;\n    let n = b != c;",
    ));
    let main = function(&program, "main");
    for name in ["a", "d", "e", "f", "g", "h", "k", "l", "m", "n"] {
        assert_eq!(local_ty(main, name), Ty::Bool, "{name}");
    }
}

#[test]
fn ordering_a_uuid_or_bytes_is_v0200_naming_numbers_and_time() {
    for (expr, ty) in [
        ("i < j", "Uuid"),
        ("i >= j", "Uuid"),
        ("b > c", "Bytes"),
        ("b <= c", "Bytes"),
    ] {
        let text = with_values(&format!("let x = {expr};"));
        let (d, _) = one_error(&text);
        assert_eq!(d.code, codes::V0200, "{expr}: {d:#?}");
        assert!(d.message.contains(&format!("`{ty}`")), "{expr}: {d:#?}");
        assert!(
            d.notes
                .iter()
                .any(|n| n.contains("numbers and `Time` only")),
            "{expr}: {d:#?}"
        );
    }
}

/// Review Focus 2: admitting `Time` to the orderings leaves arithmetic
/// on it refused.
#[test]
fn arithmetic_on_a_time_stays_v0200() {
    for expr in ["t + u", "t - u", "t * u"] {
        let text = with_values(&format!("let x = {expr};"));
        let (d, _) = one_error(&text);
        assert_eq!(d.code, codes::V0200, "{expr}: {d:#?}");
        assert!(d.message.contains("`Time`"), "{expr}: {d:#?}");
        assert!(
            d.notes.iter().any(|n| n.contains("numeric types only")),
            "{expr}: {d:#?}"
        );
    }
}

#[test]
fn a_time_and_a_uuid_print_with_braces_and_bytes_is_v0203() {
    ok(&with_values(
        "println!(\"{} {}\", t, i);\n    let s = format!(\"{}{}\", u, j);",
    ));
    let (d, sources) = one_error(&with_values("println!(\"{}\", b);"));
    assert_eq!(d.code, codes::V0203, "{d:#?}");
    assert_eq!(d.span, part_of(&sources, "\"{}\", b)", "b"));
    assert!(d.message.contains("`Bytes`"), "{d:#?}");
    assert!(
        d.notes
            .iter()
            .any(|n| n.contains("`to_text`") && n.contains("`to_base64`")),
        "{d:#?}"
    );
}

#[test]
fn the_v0203_note_of_other_types_lists_time_and_uuid() {
    let (d, _) = one_error("fn main() {\n    let v = vec![1];\n    println!(\"{}\", v);\n}\n");
    assert_eq!(d.code, codes::V0203, "{d:#?}");
    assert!(
        d.notes.iter().any(|n| n
            == "only numbers, `bool`, `string`, `Error`, `Time`, and `Uuid` have a printed form"),
        "{d:#?}"
    );
}

#[test]
fn assert_eq_shows_a_time_and_a_uuid_and_not_bytes() {
    for (ty, value, show) in [
        ("Time", "Time::now()", true),
        ("Uuid", "Uuid::new()", true),
        ("Bytes", "Bytes::from_text(\"x\")", false),
    ] {
        let text = format!(
            "fn main() {{}}\n#[test]\nfn checks() {{\n    let a: {ty} = {value};\n    \
             let b: {ty} = {value};\n    assert_eq(a, b);\n}}\n"
        );
        let program = ok(&text);
        let checks = function(&program, "checks");
        let HirExprKind::Assert {
            kind: AssertKind::Eq { show: shown },
            ..
        } = stmt_expr(checks, 2).kind
        else {
            panic!("an assert_eq: {:?}", stmt_expr(checks, 2));
        };
        assert_eq!(shown, show, "{ty}");
    }
}

#[test]
fn parse_reads_a_time_and_a_uuid_and_not_bytes() {
    let program = ok(
        "fn f(text: string) -> Result<Uuid, Error> {\n    let id: Uuid = text.parse()?;\n    Ok(id)\n}\n\
         fn main() {\n    let s = \"2026-10-07T12:00:00Z\";\n    let t: Result<Time, Error> = s.parse();\n}\n",
    );
    assert_eq!(
        local_ty(function(&program, "main"), "t"),
        result_of(Ty::Time)
    );
    assert_eq!(local_ty(function(&program, "f"), "id"), Ty::Uuid);
    assert!(program.uses_std);

    let (d, sources) = one_error(
        "fn main() {\n    let s = \"a\";\n    let b: Result<Bytes, Error> = s.parse();\n}\n",
    );
    assert_eq!(d.code, codes::V0200, "{d:#?}");
    assert_eq!(d.span, span_of(&sources, "s.parse()"));
    assert_eq!(
        d.message,
        "`parse` reads a number, `bool`, `Time`, or `Uuid` from text, and cannot make a `Bytes`"
    );
}

#[test]
fn clone_of_a_time_or_uuid_is_v0100_and_of_bytes_a_copy() {
    for (value, name) in [("Time::now()", "Time"), ("Uuid::new()", "Uuid")] {
        let text = format!("fn main() {{\n    let x = {value};\n    let c = x.clone();\n}}\n");
        let (d, sources) = one_error(&text);
        assert_eq!(d.code, codes::V0100, "{name}: {d:#?}");
        assert_eq!(d.span, part_of(&sources, "x.clone()", "clone"), "{name}");
        assert_eq!(
            d.message,
            format!(
                "`{name}` needs no `.clone()`: numbers, `bool`, `Time`, and `Uuid` are copied \
                 on use; drop `.clone()`"
            )
        );
    }
    let program = ok("fn copy(b: Bytes) -> Bytes {\n    b.clone()\n}\n\
         fn main() {\n    let b = Bytes::from_text(\"x\");\n    let c = b.clone();\n}\n");
    assert_eq!(local_ty(function(&program, "main"), "c"), Ty::Bytes);
}

#[test]
fn a_struct_holding_each_of_the_three_derives_eq_and_clone() {
    let program = ok(
        "struct Stamp {\n    at: Time,\n    id: Uuid,\n    data: Bytes,\n}\n\
         enum Event {\n    At(Time),\n    Id(Uuid),\n    Data(Bytes),\n}\n\
         fn main() {\n    let s = Stamp { at: Time::now(), id: Uuid::new(), data: Bytes::from_text(\"x\") };\n    \
         let c = s.clone();\n    let same = s == c;\n    let e = Event::At(Time::now());\n    \
         let f = e.clone();\n    let both = e == f;\n}\n",
    );
    let all = Derives {
        clone: true,
        eq: true,
    };
    assert_eq!(program.structs[0].derives, all);
    assert_eq!(program.enums[0].derives, all);
}

#[test]
fn sort_takes_times_and_contains_takes_each_of_the_three() {
    ok(&with_values(
        "let mut times = vec![t, u];\n    times.sort();\n    let a = times.contains(t);\n    \
         let ids = vec![i, j];\n    let d = ids.contains(i);\n    \
         let data = vec![Bytes::from_text(\"x\")];\n    let e = data.contains(b);",
    ));
    for (stmts, ty) in [
        ("let mut ids = vec![i, j];\n    ids.sort();", "Vec<Uuid>"),
        ("let mut data = vec![b, c];\n    data.sort();", "Vec<Bytes>"),
    ] {
        let (d, _) = one_error(&with_values(stmts));
        assert_eq!(d.code, codes::V0200, "{ty}: {d:#?}");
        assert_eq!(
            d.message,
            format!(
                "`sort` needs a `Vec` of an integer type, `bool`, `string`, or `Time`, and this \
                 is `{ty}`"
            )
        );
    }
}

#[test]
fn a_uuid_is_a_map_key_and_a_time_or_bytes_is_v0101() {
    let program = ok(
        "fn main() {\n    let mut m: HashMap<Uuid, string> = HashMap::new();\n    \
         let id = Uuid::new();\n    let old = m.insert(id, \"a\");\n    let has = m.contains_key(id);\n}\n",
    );
    assert_eq!(
        local_ty(function(&program, "main"), "m"),
        map_of(Ty::Uuid, Ty::String)
    );
    for (key, at) in [("Time", "Time"), ("Bytes", "Bytes")] {
        let text = format!("fn main() {{\n    let m: HashMap<{key}, i32> = HashMap::new();\n}}\n");
        let (d, sources) = one_error(&text);
        assert_eq!(d.code, codes::V0101, "{key}: {d:#?}");
        assert_eq!(d.span, part_of(&sources, &format!("<{key},"), at), "{key}");
        assert_eq!(
            d.message,
            "a `HashMap` key must be of an integer type, `bool`, `string`, or `Uuid`"
        );
    }
}

// --- JSON (M5a spec 2.4, 2.9) -----------------------------------------------

const WRITE: Serde = Serde {
    serialize: true,
    deserialize: false,
};
const READ: Serde = Serde {
    serialize: false,
    deserialize: true,
};

#[test]
fn json_calls_type_from_the_expected_type_and_the_argument() {
    let program = ok("struct Address {
    city: string,
}
struct User {
    name: string,
    home: Option<Address>,
}
fn load(body: string) -> Result<User, Error> {
    let u: User = json::parse(body)?;
    Ok(u)
}
fn take(r: Result<Vec<i32>, Error>) {}
fn main() {
    let r: Result<User, Error> = json::parse(\"{}\");
    take(json::parse(\"[1]\"));
    let text = json::stringify(vec![1, 2]);
    let n = json::stringify(3);
}
");
    let load = function(&program, "load");
    assert_eq!(local_ty(load, "u"), Ty::Struct(crate::resolve::StructId(1)));
    let main = function(&program, "main");
    assert_eq!(
        local_ty(main, "r"),
        result_of(Ty::Struct(crate::resolve::StructId(1)))
    );
    assert_eq!(
        call_args(stmt_expr(main, 1))[0].ty,
        result_of(Ty::Vec(Box::new(I32)))
    );
    assert_eq!(local_ty(main, "text"), Ty::String);
    assert!(program.uses_std);
    // Both structs are read; neither is written.
    assert_eq!(program.structs[0].serde, READ);
    assert_eq!(program.structs[1].serde, READ);
}

#[test]
fn stringify_marks_what_it_reaches_serialize_only() {
    let program = ok("enum Role {
    Admin,
}
struct Address {
    city: string,
}
struct User {
    home: Address,
    role: Role,
}
struct Unused {
    n: i32,
}
fn main() {
    let u = User { home: Address { city: \"x\" }, role: Role::Admin };
    println!(\"{}\", json::stringify(u));
}
");
    assert_eq!(program.structs[0].serde, WRITE);
    assert_eq!(program.structs[1].serde, WRITE);
    assert_eq!(program.structs[2].serde, Serde::default());
    assert_eq!(program.enums[0].serde, WRITE);
    // `Derives` is untouched.
    assert!(program.structs[1].derives.clone);
}

#[test]
fn json_parse_with_nothing_expected_is_v0207_showing_a_typed_let() {
    let (d, sources) = one_error("fn main() {\n    let u = json::parse(\"{}\");\n}\n");
    assert_eq!(d.code, codes::V0207, "{d:#?}");
    assert_eq!(d.span, span_of(&sources, "json::parse(\"{}\")"));
    assert!(
        d.message.contains("`let u: User = json::parse(..)?;`"),
        "{d:#?}"
    );
}

#[test]
fn json_parse_under_question_in_an_option_function_is_v0206() {
    let (d, _) = one_error(
        "fn f(text: string) -> Option<i32> {\n    let n: i32 = json::parse(text)?;\n    Some(n)\n}\nfn main() {}\n",
    );
    assert_eq!(d.code, codes::V0206, "{d:#?}");
    assert!(d.message.contains("`Result<_, Error>`"), "{d:#?}");
}

#[test]
fn an_unknown_json_call_is_v0100_listing_the_two() {
    let (d, _) = one_error("fn main() {\n    let s = json::write(1);\n}\n");
    assert_eq!(d.code, codes::V0100, "{d:#?}");
    assert!(
        d.message.contains("`json::parse` and `json::stringify`"),
        "{d:#?}"
    );
}

#[test]
fn an_enum_with_data_through_json_is_v0210_at_the_call() {
    let (d, sources) = one_error(
        "enum Payment {\n    Cash,\n    Card(string),\n}\nstruct Order {\n    payment: Payment,\n}\n\
         fn main() {\n    let o = Order { payment: Payment::Cash };\n    let s = json::stringify(o);\n}\n",
    );
    assert_eq!(d.code, codes::V0210, "{d:#?}");
    assert_eq!(d.span, span_of(&sources, "json::stringify(o)"));
    assert_eq!(d.message, "`Order` cannot be turned into JSON");
    assert_eq!(d.labels[0].text, "`Card` carries data");
    assert_eq!(d.labels[0].span, span_of(&sources, "payment: Payment"));
    assert!(
        d.notes.iter().any(|n| n.contains("#[rename(\"type\")]")),
        "{d:#?}"
    );
    let (d, _) = one_error(
        "enum Payment {\n    Card(string),\n}\nfn main() {\n    let p: Result<Vec<Payment>, Error> = json::parse(\"[]\");\n}\n",
    );
    assert_eq!(d.code, codes::V0210, "{d:#?}");
    assert_eq!(d.message, "`Vec<Payment>` cannot be read from JSON");
}

#[test]
fn a_map_with_integer_keys_and_an_error_are_v0210() {
    for (text, label) in [
        (
            "fn main() {\n    let m: HashMap<i32, string> = HashMap::new();\n    let s = json::stringify(m);\n}\n",
            "`HashMap<i32, string>` has keys that are not `string`",
        ),
        (
            "struct Failed {\n    error: Error,\n}\nfn main() {\n    let r: Result<Failed, Error> = json::parse(\"{}\");\n}\n",
            "`Error` holds a message, not data",
        ),
    ] {
        let (d, _) = one_error(text);
        assert_eq!(d.code, codes::V0210, "{d:#?}");
        let said = d.labels.iter().map(|l| &l.text).chain(&d.notes);
        assert!(said.into_iter().any(|t| t == label), "{d:#?}");
    }
}

#[test]
fn an_unreached_type_with_an_enum_with_data_is_fine() {
    let program = ok(
        "enum Payment {\n    Card(string),\n}\nstruct Order {\n    payment: Payment,\n}\n\
         struct Point {\n    x: i32,\n}\nfn main() {\n    let s = json::stringify(Point { x: 1 });\n}\n",
    );
    assert_eq!(program.structs[0].serde, Serde::default());
    assert_eq!(program.structs[1].serde, WRITE);
}

const SKIPPED_HASH: &str = "struct User {\n    name: string,\n    #[skip]\n    hash: string,\n}\n";

/// Review focus 4: a `#[skip]` field that is not an `Option` and has no
/// default is fine when only `json::stringify` reaches its struct.
#[test]
fn a_skipped_field_with_no_default_is_fine_when_only_written() {
    let program = ok(&format!(
        "{SKIPPED_HASH}fn main() {{\n    let u = User {{ name: \"a\", hash: \"h\" }};\n    println!(\"{{}}\", json::stringify(u));\n}}\n"
    ));
    assert_eq!(program.structs[0].serde, WRITE);
}

#[test]
fn a_skipped_field_with_no_default_is_v0209_once_parsed() {
    let (d, sources) = one_error(&format!(
        "{SKIPPED_HASH}fn main() {{\n    let u: Result<User, Error> = json::parse(\"{{}}\");\n}}\n"
    ));
    assert_eq!(d.code, codes::V0209, "{d:#?}");
    assert_eq!(d.span, span_of(&sources, "hash: string"));
    assert_eq!(d.labels[0].span, span_of(&sources, "json::parse(\"{}\")"));
}

#[test]
fn two_keys_the_same_after_rename_are_v0209_only_when_reached() {
    let types = "enum Role {\n    #[rename(\"Member\")]\n    Admin,\n    Member,\n}\nstruct User {\n    \
                 #[rename(\"name\")]\n    user_name: string,\n    name: string,\n    role: Role,\n}\n";
    ok(&format!("{types}fn main() {{}}\n"));
    let (diagnostics, _) = errors(&format!(
        "{types}fn main() {{\n    let u: Result<User, Error> = json::parse(\"{{}}\");\n}}\n"
    ));
    assert_eq!(diagnostics.len(), 2, "{diagnostics:#?}");
    assert!(diagnostics.iter().all(|d| d.code == codes::V0209));
}

// --- Tests (M5a spec 2.7) ---------------------------------------------------

#[test]
fn a_call_to_a_test_is_v0114_at_the_callee() {
    let (d, sources) = one_error("#[test]\nfn checks() {}\nfn main() { checks(); }");
    assert_eq!(d.code, codes::V0114);
    assert_eq!(d.message, "`checks` is a test and cannot be called");
    assert_eq!(d.span, part_of(&sources, "checks();", "checks"));
}

#[test]
fn a_test_lowers_marked_and_is_not_called() {
    let program = ok("#[test]\nfn checks() {}\nfn helper() {}\nfn main() { helper(); }");
    assert!(function(&program, "checks").is_test);
    assert!(!function(&program, "helper").is_test);
}

// --- env::parse (M5a spec 2.5) ----------------------------------------------

const CONFIG: &str = "enum Mode {\n    Dev,\n    Prod,\n}\nstruct Config {\n    port: u16,\n    \
                      mode: Mode,\n    url: Option<string>,\n    \
                      #[skip]\n    #[default(3)]\n    retries: i32,\n}\n";

#[test]
fn env_parse_types_from_the_expected_type_and_marks_reads() {
    let program = ok(&format!(
        "{CONFIG}fn load() -> Result<Config, Error> {{\n    let c: Config = env::parse()?;\n    Ok(c)\n}}\n\
         fn main() {{\n    let r: Result<Config, Error> = env::parse();\n}}\n"
    ));
    let load = function(&program, "load");
    assert_eq!(local_ty(load, "c"), Ty::Struct(crate::resolve::StructId(0)));
    assert!(program.uses_std);
    assert_eq!(program.structs[0].serde, READ);
    assert_eq!(program.enums[0].serde, READ);
}

#[test]
fn a_nested_struct_vec_or_map_field_is_v0210_naming_it() {
    for (field, ty) in [
        ("db: Db", "Db"),
        ("tags: Vec<string>", "Vec<string>"),
        ("m: HashMap<string, i32>", "HashMap<string, i32>"),
        ("db: Option<Db>", "Db"),
    ] {
        let text = format!(
            "struct Db {{\n    host: string,\n}}\nstruct Config {{\n    {field},\n}}\n\
             fn main() {{\n    let r: Result<Config, Error> = env::parse();\n}}\n"
        );
        let (d, sources) = one_error(&text);
        assert_eq!(d.code, codes::V0210, "{d:#?}");
        assert_eq!(d.message, "`Config` cannot be read from the environment");
        assert_eq!(d.labels[0].text, format!("`{ty}` is more than one value"));
        assert_eq!(d.labels[0].span, span_of(&sources, field));
        let said = d.notes.join(" ");
        assert!(said.contains("a variable holds one value"), "{d:#?}");
        assert!(!said.contains("JSON"), "{d:#?}");
    }
}

/// `Time`, `Uuid`, and `Bytes` go through JSON both ways, and `env::parse`
/// reads `Time` and an `Option<Uuid>` but refuses `Bytes` (milestone 5c
/// spec 2.4).
#[test]
fn time_uuid_and_bytes_go_through_json_and_env_but_bytes_not_env() {
    let program = ok(
        "struct Record {\n    at: Time,\n    id: Uuid,\n    data: Bytes,\n}\n\
         struct Settings {\n    start: Time,\n    owner: Option<Uuid>,\n}\n\
         fn main() {\n    let r: Result<Record, Error> = json::parse(\"{}\");\n    \
         match r {\n        Ok(v) => println!(\"{}\", json::stringify(v)),\n        \
         Err(e) => {}\n    }\n    let s: Result<Settings, Error> = env::parse();\n}\n",
    );
    let both = Serde {
        serialize: true,
        deserialize: true,
    };
    assert_eq!(program.structs[0].serde, both);
    assert_eq!(program.structs[1].serde, READ);
    let (d, sources) = one_error(
        "struct Keys {\n    key: Bytes,\n}\n\
         fn main() {\n    let r: Result<Keys, Error> = env::parse();\n}\n",
    );
    assert_eq!(d.code, codes::V0210, "{d:#?}");
    assert_eq!(
        d.labels[0].text,
        "`Bytes` is not a type that the environment can hold"
    );
    assert_eq!(d.labels[0].span, span_of(&sources, "key: Bytes"));
}

#[test]
fn a_skipped_nested_field_is_not_read() {
    ok(
        "struct Config {\n    n: i32,\n    #[skip]\n    tags: Option<Vec<string>>,\n    #[skip]\n    db: Option<Db>,\n}\nstruct Db {\n    host: string,\n}\n\
        fn main() {\n    let r: Result<Config, Error> = env::parse();\n}\n",
    );
}

#[test]
fn env_parse_of_a_non_struct_or_an_enum_with_data_field_is_v0210() {
    let (d, _) = one_error("fn main() {\n    let r: Result<i32, Error> = env::parse();\n}\n");
    assert_eq!(d.code, codes::V0210, "{d:#?}");
    assert_eq!(d.message, "`i32` cannot be read from the environment");
    let (d, sources) = one_error(
        "enum Card {\n    Visa(string),\n}\nstruct Config {\n    card: Card,\n}\n\
         fn main() {\n    let r: Result<Config, Error> = env::parse();\n}\n",
    );
    assert_eq!(d.code, codes::V0210, "{d:#?}");
    assert_eq!(d.labels[0].text, "`Visa` carries data");
    assert_eq!(d.labels[0].span, span_of(&sources, "card: Card"));
}

#[test]
fn two_fields_with_the_same_upper_cased_key_are_v0209() {
    let (d, sources) = one_error(
        "struct Config {\n    user_name: string,\n    #[rename(\"USER_NAME\")]\n    other: string,\n}\n\
         fn main() {\n    let r: Result<Config, Error> = env::parse();\n}\n",
    );
    assert_eq!(d.code, codes::V0209, "{d:#?}");
    assert_eq!(
        d.message,
        "two fields of `Config` have the variable `USER_NAME`: `user_name` and `other`"
    );
    assert_eq!(d.labels[0].span, span_of(&sources, "user_name: string"));
    // The same struct is fine when only json reaches it.
    ok(
        "struct Config {\n    user_name: string,\n    #[rename(\"USER_NAME\")]\n    other: string,\n}\n\
        fn main() {\n    let r: Result<Config, Error> = json::parse(\"{}\");\n}\n",
    );
}

#[test]
fn env_parse_with_nothing_expected_is_v0207() {
    let (d, sources) = one_error("fn main() {\n    let c = env::parse();\n}\n");
    assert_eq!(d.code, codes::V0207, "{d:#?}");
    assert_eq!(d.span, span_of(&sources, "env::parse()"));
    assert!(
        d.message.contains("`let c: Config = env::parse()?;`"),
        "{d:#?}"
    );
}

#[test]
fn the_head_of_a_match_needs_a_typed_let_for_env_parse() {
    let (d, _) = one_error(&format!(
        "{CONFIG}fn main() {{\n    match env::parse() {{\n        Ok(c) => {{}}\n        Err(e) => {{}}\n    }}\n}}\n"
    ));
    assert_eq!(d.code, codes::V0207, "{d:#?}");
    ok(&format!(
        "{CONFIG}fn main() {{\n    let r: Result<Config, Error> = env::parse();\n    match r {{\n        Ok(c) => {{}}\n        Err(e) => {{}}\n    }}\n}}\n"
    ));
}

#[test]
fn an_unknown_env_call_is_v0100_and_env_parse_takes_no_arguments() {
    let (d, _) = one_error("fn main() {\n    let s = env::get(\"A\");\n}\n");
    assert_eq!(d.code, codes::V0100, "{d:#?}");
    assert!(d.message.contains("`env::parse()`"), "{d:#?}");
    let (d, _) = one_error(&format!(
        "{CONFIG}fn main() {{\n    let r: Result<Config, Error> = env::parse(1);\n}}\n"
    ));
    assert_eq!(d.code, codes::V0201, "{d:#?}");
}

// --- log (M5a spec 2.6) ------------------------------------------------------

#[test]
fn log_calls_are_unit_and_mark_the_program() {
    let program = ok("fn main() {
    let e = Error::new(\"boom\");
    log::debug(\"a\");
    log::info(\"n = {}\", 1);
    log::warn(\"{} and {}\", \"x\", 2.5);
    log::error(\"failed: {}\", e);
}
");
    assert!(program.logs && program.uses_std);
    let main = function(&program, "main");
    let HirExprKind::Log { level, args, .. } = &stmt_expr(main, 3).kind else {
        panic!("a log call");
    };
    assert_eq!((*level, args.len()), ("warn", 2));
    assert_eq!(stmt_expr(main, 3).ty, Ty::Unit);
}

#[test]
fn a_program_without_log_calls_does_not_log() {
    assert!(!ok("fn main() {\n    println!(\"hi\");\n}\n").logs);
}

#[test]
fn log_placeholder_count_mismatch_is_v0202() {
    let (d, _) = one_error("fn main() {\n    log::info(\"{} {}\", 1);\n}\n");
    assert_eq!(d.code, codes::V0202, "{d:#?}");
}

#[test]
fn log_text_must_be_a_string_literal() {
    let (d, sources) = one_error("fn main() {\n    let msg = \"hi\";\n    log::info(msg);\n}\n");
    assert_eq!(d.code, codes::V0202, "{d:#?}");
    assert!(d.message.contains("written in quotes"), "{}", d.message);
    assert_eq!(d.span, part_of(&sources, "info(msg)", "msg"));
}

#[test]
fn log_of_a_struct_is_v0203() {
    let (d, _) = one_error(
        "struct P {\n    x: i32,\n}\nfn main() {\n    let p = P { x: 1 };\n    log::error(\"{}\", p);\n}\n",
    );
    assert_eq!(d.code, codes::V0203, "{d:#?}");
}

#[test]
fn an_unknown_log_function_is_v0100() {
    let (d, _) = one_error("fn main() {\n    log::trace(\"x\");\n}\n");
    assert_eq!(d.code, codes::V0100, "{d:#?}");
}

// --- assert and assert_eq (M5a spec 2.7) -------------------------------------

#[test]
fn assert_and_assert_eq_in_a_test_carry_their_location() {
    let program = ok(
        "fn main() {}\n#[test]\nfn checks() {\n    let x = 2;\n    assert(x > 1);\n    assert_eq(x, 2);\n    assert_eq(\"a\", \"a\");\n}\n",
    );
    let checks = function(&program, "checks");
    let HirExprKind::Assert {
        cond,
        location,
        kind: AssertKind::Plain,
    } = &stmt_expr(checks, 1).kind
    else {
        panic!("an assert: {:?}", stmt_expr(checks, 1));
    };
    assert_eq!(location, "test.vr:5");
    assert_eq!(cond.ty, Ty::Bool);
    assert_eq!(stmt_expr(checks, 1).ty, Ty::Unit);
    let HirExprKind::Assert {
        cond,
        location,
        kind: AssertKind::Eq { show: true },
    } = &stmt_expr(checks, 2).kind
    else {
        panic!("an assert_eq: {:?}", stmt_expr(checks, 2));
    };
    assert_eq!(location, "test.vr:6");
    assert!(matches!(cond.kind, HirExprKind::Binary { .. }));
    assert!(!program.uses_std);
}

#[test]
fn assert_eq_of_values_without_a_printed_form_shows_no_values() {
    let program = ok(
        "enum E {\n    A,\n    B,\n}\nfn main() {}\n#[test]\nfn checks() {\n    assert_eq(E::A, E::A);\n}\n",
    );
    let checks = function(&program, "checks");
    assert!(matches!(
        stmt_expr(checks, 0).kind,
        HirExprKind::Assert {
            kind: AssertKind::Eq { show: false },
            ..
        }
    ));
}

#[test]
fn assert_outside_a_test_is_v0114() {
    let (d, sources) = one_error("fn main() {\n    assert(1 > 0);\n}\n");
    assert_eq!(d.code, codes::V0114, "{d:#?}");
    assert_eq!(d.span, part_of(&sources, "assert(1 > 0)", "assert"));
    let (d, _) =
        one_error("fn helper() {\n    assert_eq(1, 1);\n}\nfn main() {\n    helper();\n}\n");
    assert_eq!(d.code, codes::V0114, "{d:#?}");
}

#[test]
fn assert_eq_on_differing_types_is_v0200() {
    let (d, _) = one_error("fn main() {}\n#[test]\nfn checks() {\n    assert_eq(1, \"one\");\n}\n");
    assert_eq!(d.code, codes::V0200, "{d:#?}");
}

#[test]
fn assert_eq_on_a_struct_without_eq_is_v0203() {
    let (result, sources) =
        check_path("crates/varyk/tests/fixtures/errors/v0203_assert_eq_blocked/main.vr");
    let Err(diagnostics) = result else {
        panic!("expected diagnostics");
    };
    let d = only(&diagnostics);
    assert_eq!(d.code, codes::V0203, "{d:#?}");
    assert!(d.message.contains("its field `handle`"), "{d:#?}");
    assert_eq!(d.span, span_of(&sources, "assert_eq(a, b)"));
}

#[test]
fn assert_takes_a_bool_and_counts_its_arguments() {
    let (d, _) = one_error("fn main() {}\n#[test]\nfn checks() {\n    assert(1);\n}\n");
    assert_eq!(d.code, codes::V0200, "{d:#?}");
    let (d, _) = one_error("fn main() {}\n#[test]\nfn checks() {\n    assert_eq(1);\n}\n");
    assert_eq!(d.code, codes::V0201, "{d:#?}");
}

// --- Async functions (milestone 5b1 spec 2.2, 2.3, 2.7) ---------------------

#[test]
fn an_awaited_call_types_as_what_the_function_returns() {
    let program = ok(
        "struct S {\n    n: i64,\n}\nimpl S {\n    async fn get(self) -> i64 {\n        self.n\n    }\n}\n\
         async fn load(n: i64) -> i64 {\n    n\n}\n\
         async fn main() {\n    let x = load(1).await;\n    let s = S { n: 2 };\n    let y = s.get().await;\n    time::sleep(10).await;\n}\n",
    );
    let main = function(&program, "main");
    assert!(main.is_async);
    assert!(function(&program, "load").is_async && function(&program, "get").is_async);
    assert_eq!(local_ty(main, "x"), I64);
    assert_eq!(local_ty(main, "y"), I64);
    let HirExprKind::Await(operand) = &stmt_expr(main, 0).kind else {
        panic!("an await: {:?}", stmt_expr(main, 0));
    };
    assert!(
        matches!(operand.kind, HirExprKind::Call { .. }),
        "{operand:?}"
    );
    let sleep = stmt_expr(main, 3);
    assert_eq!(sleep.ty, Ty::Unit);
    assert!(matches!(sleep.kind, HirExprKind::Await(_)), "{sleep:?}");
    assert!(program.uses_std);
}

#[test]
fn an_async_main_or_test_alone_uses_std() {
    assert!(ok("async fn main() {}\n").uses_std);
    assert!(ok("fn main() {}\n#[test]\nasync fn t() {}\n").uses_std);
    assert!(!ok("async fn f() {}\nfn main() {}\n").uses_std);
    let program = ok("async fn main() {}\n#[test]\nasync fn t() {\n    assert(true);\n}\n");
    assert!(function(&program, "t").is_test && function(&program, "t").is_async);
}

#[test]
fn an_async_call_in_an_ordinary_function_is_v0211_adding_async() {
    for (text, header) in [
        (
            "async fn load() {}\nfn main() {\n    load().await;\n}\n",
            "fn main",
        ),
        (
            "async fn load() {}\npub fn run() {\n    load().await;\n}\nfn main() {}\n",
            "pub fn run",
        ),
        (
            "async fn load() {}\nfn main() {\n    load();\n}\n",
            "fn main",
        ),
    ] {
        let (d, sources) = one_error(text);
        assert_eq!(d.code, codes::V0211, "{d:#?}");
        assert_eq!(
            d.message,
            "`load` waits for something, so only an `async fn` can call it"
        );
        assert_eq!(d.span, part_of(&sources, "    load()", "load()"));
        let fix = d.fix_it.as_ref().expect("a fix-it");
        assert_eq!(fix.replacement, "async ");
        let at = part_of(&sources, header, "fn").start;
        assert_eq!((fix.span.start, fix.span.end), (at, at), "{text}");
    }
}

#[test]
fn await_inside_a_closure_is_v0211() {
    let (d, sources) = one_error(
        "async fn get(n: i64) -> i64 {\n    n\n}\nasync fn main() {\n    let v: Vec<i64> = vec![1];\n    \
         let w: Vec<i64> = v.iter().map(|x| get(x).await).collect();\n}\n",
    );
    assert_eq!(d.code, codes::V0211, "{d:#?}");
    assert_eq!(d.message, "`.await` cannot be used inside a closure");
    assert_eq!(d.span, part_of(&sources, "get(x).await", ".await"));
}

#[test]
fn await_on_anything_but_an_async_call_is_v0212() {
    for (text, needle, message) in [
        (
            "async fn main() {\n    let x = 5.await;\n}\n",
            "5.await",
            "only a call to an async function can be waited for with `.await`",
        ),
        (
            "fn g() -> i32 {\n    1\n}\nasync fn main() {\n    let x = g().await;\n}\n",
            "g().await",
            "`g` is not an async function, so there is nothing to wait for",
        ),
    ] {
        let (d, sources) = one_error(text);
        assert_eq!(d.code, codes::V0212, "{d:#?}");
        assert_eq!(d.message, message);
        assert_eq!(d.span, part_of(&sources, needle, ".await"));
        let fix = d.fix_it.as_ref().expect("a fix-it");
        assert_eq!((fix.span, fix.replacement.as_str()), (d.span, ""));
    }
}

#[test]
fn an_async_call_not_awaited_is_v0213() {
    let (d, sources) = one_error("async fn work() {}\nasync fn main() {\n    work();\n}\n");
    assert_eq!(d.code, codes::V0213, "{d:#?}");
    assert_eq!(
        d.message,
        "this starts `work` and then throws its task away, which stops it"
    );
    assert_eq!(d.span, part_of(&sources, "    work()", "work()"));
    let fix = d.fix_it.as_ref().expect("a fix-it");
    assert_eq!(fix.replacement, ".await");
    let end = d.span.end;
    assert_eq!((fix.span.start, fix.span.end), (end, end));
    let (d, _) = one_error("async fn main() {\n    time::sleep(1);\n}\n");
    assert_eq!(d.code, codes::V0213, "{d:#?}");
}

#[test]
fn async_functions_calling_each_other_are_v0214_naming_the_cycle() {
    let (d, sources) = one_error(
        "async fn a() {\n    b().await;\n}\nasync fn b() {\n    a().await;\n}\nasync fn main() {\n    a().await;\n}\n",
    );
    assert_eq!(d.code, codes::V0214, "{d:#?}");
    assert_eq!(
        d.message,
        "async functions cannot call each other in a cycle: `a` calls `b`, which calls `a`"
    );
    assert_eq!(d.span, span_of(&sources, "b()"));
    let (d, sources) = one_error(
        "struct S {\n    n: i64,\n}\nimpl S {\n    async fn count(self) -> i64 {\n        self.count().await\n    }\n}\n\
         async fn main() {}\n",
    );
    assert_eq!(d.code, codes::V0214, "{d:#?}");
    assert_eq!(
        d.message,
        "async functions cannot call each other in a cycle: `S::count` calls itself"
    );
    assert_eq!(d.span, span_of(&sources, "self.count()"));
}

#[test]
fn a_call_to_an_async_main_is_v0106() {
    let (d, sources) = one_error("async fn helper() {\n    main().await;\n}\nasync fn main() {}\n");
    assert_eq!(d.code, codes::V0106, "{d:#?}");
    assert_eq!(
        d.message,
        "`main` is where the program starts and cannot be called"
    );
    assert_eq!(d.span, part_of(&sources, "main().await", "main"));
}

#[test]
fn a_call_to_a_main_that_returns_a_result_is_v0106() {
    let (d, sources) = one_error(
        "fn helper() -> bool {\n    main().is_ok()\n}\nfn main() -> Result<bool, Error> {\n    Ok(true)\n}\n",
    );
    assert_eq!(d.code, codes::V0106, "{d:#?}");
    assert_eq!(d.span, part_of(&sources, "main().is_ok()", "main"));
}

#[test]
fn question_mark_in_a_main_that_returns_a_result_checks() {
    ok("fn port() -> Result<i64, Error> {\n    Ok(1)\n}\n\
        fn main() -> Result<i64, Error> {\n    let p = port()?;\n    Ok(p)\n}\n");
}

#[test]
fn a_path_through_http_or_sql_says_how_to_add_the_package() {
    let note = |name: &str| {
        let package = format!("varyk-{name}");
        format!("`{name}` is not a package of this build; `varyk add {name}` adds {package}")
    };
    for (body, name) in [
        ("let p = sql::connect(\"x\");", "sql"),
        ("let a = http::App::new(1);", "http"),
        ("let r = http::Thing { a: 1 };", "http"),
        ("let k = sql::Kind::A;", "sql"),
    ] {
        let (d, _) = one_error(&format!("fn main() {{\n    {body}\n}}\n"));
        assert_eq!(d.notes, [note(name)], "{body}: {d:#?}");
    }
    let (d, _) = one_error("fn f(p: sql::Pool) {}\nfn main() {}\n");
    assert_eq!(d.notes, [note("sql")], "{d:#?}");
    let (d, _) = one_error("use sql::Pool;\nfn main() {}\n");
    assert_eq!(d.notes, [note("sql")], "{d:#?}");
}

#[test]
fn another_unknown_path_gets_no_package_note() {
    let (d, _) = one_error("fn main() {\n    let p = db::connect(\"x\");\n}\n");
    assert!(d.notes.is_empty(), "{d:#?}");
}

#[test]
fn a_result_where_its_value_is_wanted_gets_a_question_mark() {
    let load = "fn load(id: i64) -> Result<i64, Error> {\n    Ok(id)\n}\n";
    let (d, sources) = one_error(&format!(
        "{load}fn total() -> Result<i64, Error> {{\n    let n: i64 = load(1);\n    Ok(n)\n}}\n\
         fn main() {{}}\n"
    ));
    assert_eq!(d.code, codes::V0200, "{d:#?}");
    let call = span_of(&sources, "load(1)");
    let fix_it = d.fix_it.as_ref().expect("a fix-it");
    assert_eq!(fix_it.span, Span::new(call.file, call.end, call.end));
    assert_eq!(fix_it.replacement, "?");

    // Awaited, the `?` goes after the `.await`.
    let (d, sources) = one_error(
        "async fn load(id: i64) -> Result<i64, Error> {\n    Ok(id)\n}\n\
         async fn total() -> Result<i64, Error> {\n    let n: i64 = load(1).await;\n    Ok(n)\n}\n\
         async fn main() {}\n",
    );
    let awaited = span_of(&sources, "load(1).await");
    let fix_it = d.fix_it.as_ref().expect("a fix-it");
    assert_eq!(
        fix_it.span,
        Span::new(awaited.file, awaited.end, awaited.end)
    );

    // A function that returns no `Result` gets a note instead.
    let (d, _) = one_error(&format!(
        "{load}fn main() {{\n    let n: i64 = load(1);\n    println!(\"{{}}\", n);\n}}\n"
    ));
    assert!(d.fix_it.is_none(), "{d:#?}");
    assert!(
        d.notes.iter().any(|n| n.contains("`match` or `if let`")),
        "{d:#?}"
    );

    // Another value type is a plain mismatch.
    let (d, _) = one_error(&format!(
        "{load}fn total() -> Result<string, Error> {{\n    let n: string = load(1);\n    Ok(n)\n}}\n\
         fn main() {{}}\n"
    ));
    assert!(d.fix_it.is_none() && d.notes.is_empty(), "{d:#?}");
}

#[test]
fn a_call_to_an_async_test_is_v0114() {
    let (d, _) = one_error("#[test]\nasync fn t() {}\nasync fn main() {\n    t();\n}\n");
    assert_eq!(d.code, codes::V0114, "{d:#?}");
}

// --- Started calls and tasks (milestone 5b1 spec 2.3, 2.4) -----------------

/// Whether `expr` is a started call (or started method call).
fn started(expr: &HirExpr) -> bool {
    match &expr.kind {
        HirExprKind::Call { started, .. } | HirExprKind::MethodCall { started, .. } => *started,
        _ => false,
    }
}

fn task(ty: Ty) -> Ty {
    Ty::Task(Box::new(ty))
}

#[test]
fn a_call_without_await_starts_a_task_that_await_waits_for() {
    let program = ok(
        "struct S {\n    n: i64,\n}\nimpl S {\n    async fn get(self) -> i64 {\n        self.n\n    }\n}\n\
         async fn load(n: i64) -> i64 {\n    n\n}\n\
         async fn main() {\n    let t = load(1);\n    let s = S { n: 2 };\n    let u = s.get();\n    \
         let w = time::sleep(10);\n    let x = t.await;\n    let y = u.await;\n    w.await;\n}\n",
    );
    let main = function(&program, "main");
    assert_eq!(local_ty(main, "t"), task(I64));
    assert_eq!(local_ty(main, "u"), task(I64));
    assert_eq!(local_ty(main, "w"), task(Ty::Unit));
    assert_eq!(local_ty(main, "x"), I64);
    assert_eq!(local_ty(main, "y"), I64);
    for index in [0, 2, 3] {
        assert!(
            started(stmt_expr(main, index)),
            "{:?}",
            stmt_expr(main, index)
        );
    }
    let sleep = stmt_expr(main, 3);
    assert!(
        matches!(
            sleep.kind,
            HirExprKind::Call {
                callee: Callee::Builtin(_),
                ..
            }
        ),
        "{sleep:?}"
    );
    let HirExprKind::Await(operand) = &stmt_expr(main, 4).kind else {
        panic!("an await: {:?}", stmt_expr(main, 4));
    };
    assert!(matches!(operand.kind, HirExprKind::Local(_)), "{operand:?}");
    // An awaited call is not started.
    let program = ok("async fn load() {}\nasync fn main() {\n    load().await;\n}\n");
    let HirExprKind::Await(operand) = &stmt_expr(function(&program, "main"), 0).kind else {
        panic!("an await");
    };
    assert!(!started(operand));
}

#[test]
fn a_started_call_uses_std() {
    let program = ok(
        "async fn work() {}\nasync fn run() {\n    let t = work();\n    t.await;\n}\nfn main() {}\n",
    );
    assert!(program.uses_std);
    assert!(
        !ok("async fn work() {}\nasync fn run() {\n    work().await;\n}\nfn main() {}\n").uses_std
    );
}

#[test]
fn detach_takes_a_started_call_or_a_task_local() {
    let program = ok(
        "async fn work(n: i64) {}\nasync fn main() {\n    work(1).detach();\n    let t = work(2);\n    \
         t.detach();\n}\n",
    );
    let main = function(&program, "main");
    let detach = stmt_expr(main, 0);
    assert_eq!(detach.ty, Ty::Unit);
    let HirExprKind::MethodCall { receiver, .. } = &detach.kind else {
        panic!("a method call: {detach:?}");
    };
    assert!(started(receiver), "{receiver:?}");
    assert_eq!(stmt_expr(main, 2).ty, Ty::Unit);
}

#[test]
fn a_list_of_tasks_given_nowhere_is_v0213() {
    for (text, needle) in [
        (
            "async fn f(n: i64) -> i64 {\n    n\n}\nasync fn main() {\n    let ts = vec![f(1), f(2)];\n}\n",
            "ts",
        ),
        (
            "async fn f(n: i64) -> i64 {\n    n\n}\nasync fn main() {\n    let ids: Vec<i64> = vec![1];\n    \
             let ts = ids.iter().map(|id| f(id)).collect();\n}\n",
            "ts",
        ),
        (
            "async fn f(n: i64) -> i64 {\n    n\n}\nasync fn main() {\n    vec![f(1)];\n}\n",
            "vec![f(1)]",
        ),
    ] {
        let (d, sources) = one_error(text);
        assert_eq!(d.code, codes::V0213, "{d:#?}");
        assert_eq!(d.span, span_of(&sources, needle), "{text}");
    }
}

#[test]
fn a_cycle_through_a_started_call_is_v0214() {
    let (d, _) = one_error(
        "async fn a() {\n    let t = b();\n    t.await;\n}\nasync fn b() {\n    a().detach();\n}\n\
         async fn main() {}\n",
    );
    assert_eq!(d.code, codes::V0214, "{d:#?}");
}

#[test]
fn a_started_call_anywhere_but_its_four_places_is_v0213() {
    let head = "async fn f(n: i64) -> i64 {\n    n\n}\nfn g(n: i64) -> i64 {\n    n\n}\n";
    for (body, needle) in [
        ("    f(1);\n", "f(1)"),
        ("    let _ = f(1);\n", "f(1)"),
        (
            "    let c = true;\n    let t = if c { f(1) } else { f(2) };\n    t.await;\n",
            "f(1)",
        ),
        ("    let x = g(f(1));\n", "f(1)"),
        ("    let x = f(f(1)).await;\n", "f(1)"),
        (
            "    let ids: Vec<i64> = vec![1];\n    let n = ids.iter().map(|id| f(id)).count();\n",
            "f(id)",
        ),
        (
            "    let ids: Vec<i64> = vec![1];\n    let n: Vec<i64> = ids.iter().map(|id| f(id)).map(|t| 1).collect();\n",
            "f(id)",
        ),
    ] {
        let text = format!("{head}async fn main() {{\n{body}}}\n");
        let (diagnostics, sources) = errors(&text);
        let d = &diagnostics[0];
        assert_eq!(d.code, codes::V0213, "{text}\n{diagnostics:#?}");
        assert_eq!(d.span, span_of(&sources, needle), "{text}");
        assert_eq!(
            d.notes,
            ["wait for it with `.await`, or let it run on its own with `.detach()`"]
        );
    }
}

#[test]
fn a_task_never_awaited_or_detached_is_v0213() {
    let (d, sources) =
        one_error("async fn f() -> i64 {\n    1\n}\nasync fn main() {\n    let t = f();\n}\n");
    assert_eq!(d.code, codes::V0213, "{d:#?}");
    let at = span_of(&sources, "t = f()").start;
    assert_eq!((d.span.start, d.span.end), (at, at + 1));
    assert_eq!(
        d.notes,
        ["wait for it with `t.await`, or let it run on its own with `t.detach()`"]
    );
}

#[test]
fn a_task_or_a_list_of_them_used_anywhere_else_is_v0215() {
    let head = "async fn f(n: i64) -> i64 {\n    n\n}\nfn g(n: i64) -> i64 {\n    n\n}\n";
    let tasks = "    let ts = vec![f(1), f(2)];\n";
    for (body, needle) in [
        (format!("{tasks}    let t = ts[0];\n"), "ts[0]"),
        (format!("{tasks}    ts.push(f(3));\n"), "ts.push"),
        (format!("{tasks}    for t in ts {{\n    }}\n"), "ts {"),
        (format!("{tasks}    let n = ts.len();\n"), "ts.len"),
        (format!("{tasks}    let us = ts;\n"), "ts;"),
        ("    let t = f(1);\n    let x = g(t);\n".to_string(), "t)"),
        ("    let t = f(1);\n    let u = t;\n".to_string(), "t;\n}"),
        (
            "    let t = f(1);\n    println!(\"{}\", t);\n".to_string(),
            "t)",
        ),
    ] {
        let text = format!("{head}async fn main() {{\n{body}}}\n");
        let (diagnostics, sources) = errors(&text);
        let d = &diagnostics[0];
        assert_eq!(d.code, codes::V0215, "{text}\n{diagnostics:#?}");
        let at = span_of(&sources, needle);
        assert_eq!(d.span.start, at.start, "{text}");
    }
    // Returning one.
    let (d, _) = one_error(
        "async fn f() -> i64 {\n    1\n}\nasync fn h() -> i64 {\n    let t = f();\n    return t;\n}\n\
         async fn main() {}\n",
    );
    assert_eq!(d.code, codes::V0215, "{d:#?}");
}

#[test]
fn writing_task_as_a_type_is_v0215() {
    for text in [
        "async fn f() -> i64 {\n    1\n}\nasync fn main() {\n    let t: Task<i64> = f();\n    t.await;\n}\n",
        "async fn f(t: Task) {}\nasync fn main() {}\n",
        "struct S {\n    t: Vec<Task<i64>>,\n}\nfn main() {}\n",
    ] {
        let (diagnostics, _) = errors(text);
        assert_eq!(
            diagnostics[0].code,
            codes::V0215,
            "{text}\n{diagnostics:#?}"
        );
        assert_eq!(
            diagnostics[0].message, "`Task` cannot be written as a type",
            "{text}"
        );
    }
}

#[test]
fn await_on_a_list_of_tasks_is_v0215_naming_task_all() {
    for text in [
        "async fn f() -> i64 {\n    1\n}\nasync fn main() {\n    let ts = vec![f(), f()];\n    let x = ts.await;\n}\n",
        "async fn f() -> i64 {\n    1\n}\nasync fn main() {\n    let x = vec![f(), f()].await;\n}\n",
    ] {
        let (d, _) = one_error(text);
        assert_eq!(d.code, codes::V0215, "{d:#?}");
        assert!(
            d.notes.iter().any(|note| note.contains("Task::all")),
            "{d:#?}"
        );
    }
}

#[test]
fn await_in_an_ordinary_function_is_v0211_whatever_it_follows() {
    let (d, sources) = one_error("fn main() {\n    let x = 5.await;\n}\n");
    assert_eq!(d.code, codes::V0211, "{d:#?}");
    assert_eq!(d.span, part_of(&sources, "5.await", ".await"));
    let fix = d.fix_it.as_ref().expect("a fix-it");
    assert_eq!(fix.replacement, "async ");
}

#[test]
fn task_and_shared_are_reserved_type_names() {
    for name in ["Task", "Shared"] {
        for text in [
            format!("struct {name} {{\n    n: i64,\n}}\nfn main() {{}}\n"),
            format!("enum {name} {{\n    A,\n}}\nfn main() {{}}\n"),
        ] {
            let (diagnostics, _) = errors(&text);
            assert!(
                diagnostics.iter().any(|d| d.code == codes::V0113),
                "{text}\n{diagnostics:#?}"
            );
        }
    }
}

// --- `Task::all` and `Task::all_settled` (milestone 5b1 spec 2.5) ----------

const ALL_HEAD: &str = "async fn f(n: i64) -> i64 {\n    n\n}\n\
    async fn g(n: i64) -> Result<i64, string> {\n    Ok(n)\n}\n\
    async fn h(s: string) -> usize {\n    s.len()\n}\n";

fn all_program(body: &str) -> String {
    format!(
        "{ALL_HEAD}async fn run() -> Result<i64, string> {{\n{body}    Ok(0)\n}}\nasync fn main() {{}}\n"
    )
}

#[test]
fn task_all_gives_the_values_or_a_result_of_them_and_all_settled_every_result() {
    let program = ok(&all_program(
        "    let ts = vec![f(1), f(2)];\n    let a = Task::all(ts).await;\n    \
         let b = Task::all(vec![g(1), g(2)]).await;\n    let c = Task::all(vec![g(3)]).await?;\n    \
         let ids: Vec<i64> = vec![1, 2];\n    \
         let d = Task::all_settled(ids.iter().map(|id| g(id)).collect()).await;\n    \
         let e = Task::all(ids.iter().map(|id| f(id)).collect()).await;\n    \
         let names: Vec<string> = vec![\"a\"];\n    \
         let k = Task::all(names.iter().map(|n| h(n.clone())).collect()).await;\n",
    ));
    assert!(program.uses_std);
    let run = function(&program, "run");
    let string = || Ty::String;
    assert_eq!(local_ty(run, "a"), vec_of(I64));
    assert_eq!(local_ty(run, "b"), result(vec_of(I64), string()));
    assert_eq!(local_ty(run, "c"), vec_of(I64));
    assert_eq!(local_ty(run, "d"), vec_of(result(I64, string())));
    assert_eq!(local_ty(run, "e"), vec_of(I64));
    assert_eq!(local_ty(run, "k"), vec_of(Ty::Int(IntKind::Usize)));
    // The call is awaited, not started.
    let HirExprKind::Await(operand) = &stmt_expr(run, 1).kind else {
        panic!("an await: {:?}", stmt_expr(run, 1));
    };
    assert!(
        matches!(
            operand.kind,
            HirExprKind::Call {
                callee: Callee::Builtin(_),
                started: false,
                ..
            }
        ),
        "{operand:?}"
    );
}

#[test]
fn task_all_settled_on_tasks_that_give_no_result_is_v0200_naming_task_all() {
    let (d, sources) = one_error(&all_program(
        "    let x = Task::all_settled(vec![f(1)]).await;\n",
    ));
    assert_eq!(d.code, codes::V0200, "{d:#?}");
    assert_eq!(d.span, span_of(&sources, "Task::all_settled(vec![f(1)])"));
    assert!(
        d.notes.iter().any(|note| note.contains("`Task::all(")),
        "{d:#?}"
    );
}

#[test]
fn task_all_without_await_is_v0212() {
    for (body, needle) in [
        (
            "    let ts = vec![f(1)];\n    let x = Task::all(ts);\n",
            "Task::all(ts)",
        ),
        (
            "    let x = Task::all_settled(vec![g(1)]);\n",
            "Task::all_settled(vec![g(1)])",
        ),
    ] {
        let text = all_program(body);
        let (d, sources) = one_error(&text);
        assert_eq!(d.code, codes::V0212, "{text}\n{d:#?}");
        let at = span_of(&sources, needle);
        assert_eq!(d.span, at, "{text}");
        let fix = d.fix_it.as_ref().expect("a fix-it");
        assert_eq!(fix.replacement, ".await");
        assert_eq!(fix.span, Span::new(at.file, at.end, at.end));
    }
}

#[test]
fn task_all_takes_only_a_list_of_tasks() {
    for (body, code) in [
        ("    let x = Task::all(vec![1, 2]).await;\n", codes::V0200),
        ("    let x = Task::all(f(1)).await;\n", codes::V0200),
        (
            "    let t = f(1);\n    let x = Task::all(t).await;\n",
            codes::V0215,
        ),
        (
            "    let ts = vec![f(1)];\n    let x = Task::all(ts, ts).await;\n",
            codes::V0201,
        ),
        // An empty `vec![]` has no type to take, and `Task` cannot be
        // written.
        ("    let x = Task::all(vec![]).await;\n", codes::V0207),
        ("    let x = Task::any(vec![f(1)]).await;\n", codes::V0100),
    ] {
        let text = all_program(body);
        let (diagnostics, _) = errors(&text);
        assert_eq!(diagnostics[0].code, code, "{text}\n{diagnostics:#?}");
    }
}

#[test]
fn a_list_of_tasks_given_to_task_all_is_used() {
    // From a local, so nothing is reported unused; and a collected `map`
    // in place.
    ok(&all_program(
        "    let ts = vec![f(1)];\n    let us = vec![g(1)];\n    let a = Task::all(ts).await;\n    \
         let b = Task::all_settled(us).await;\n",
    ));
}

#[test]
fn a_started_method_call_stands_in_every_place_a_started_call_does() {
    ok(
        "struct S {\n    n: i64,\n}\nimpl S {\n    async fn get(self) -> i64 {\n        self.n\n    }\n}\n\
         async fn main() {\n    let s = S { n: 1 };\n    s.clone().get().detach();\n    \
         let a = Task::all(vec![s.clone().get(), S { n: 2 }.get()]).await;\n    \
         let v: Vec<S> = vec![S { n: 3 }];\n    \
         let b = Task::all(v.iter().map(|x| x.clone().get()).collect()).await;\n}\n",
    );
}

// --- `Shared<T>` (milestone 5b1 spec 2.6) -----------------------------------

/// A struct to share, with a read method and a `mut self` one, before
/// `body`, the body of `fn main()`.
fn shared_program(body: &str) -> String {
    format!(
        "struct Config {{\n    factor: i64,\n    name: string,\n    items: Vec<i64>,\n}}\n\
         impl Config {{\n    fn describe(self) -> string {{\n        self.name.clone()\n    }}\n    \
         fn bump(mut self) {{\n        self.factor = self.factor + 1;\n    }}\n}}\n\
         enum Kind {{\n    A,\n}}\n\
         fn takes(c: Config) -> i64 {{\n    c.factor\n}}\n\
         fn main() {{\n    let cfg = Config {{ factor: 2, name: \"a\", items: vec![1] }};\n{body}}}\n"
    )
}

fn is_shared_struct(ty: &Ty) -> bool {
    matches!(ty, Ty::Shared(inner) if matches!(**inner, Ty::Struct(_)))
}

#[test]
fn shared_new_of_a_struct_gives_a_shared_and_clone_another() {
    let program = ok(&shared_program(
        "    let s = Shared::new(cfg);\n    let t: Shared<Config> = Shared::new(Config { factor: 1, name: \"b\", items: vec![] });\n    \
         let c = s.clone();\n",
    ));
    let main = function(&program, "main");
    for name in ["s", "t", "c"] {
        assert!(is_shared_struct(&local_ty(main, name)), "{name}");
    }
    // `Shared` alone is std's `Arc`, not a `varyk-std` call.
    assert!(!program.uses_std);
}

#[test]
fn shared_of_a_rust_struct_is_accepted() {
    let (result, _) = check_path("crates/varyk/tests/fixtures/interop/shared_matcher/main.vr");
    let program = result.unwrap_or_else(|d| panic!("should check: {d:#?}"));
    assert!(is_shared_struct(&local_ty(function(&program, "main"), "m")));
}

#[test]
fn shared_of_anything_but_a_struct_is_v0216() {
    for (body, needle) in [
        ("    let s = Shared::new(5);\n", "Shared::new(5)"),
        (
            "    let s = Shared::new(vec![1]);\n",
            "Shared::new(vec![1])",
        ),
        (
            "    let s = Shared::new(Some(1));\n",
            "Shared::new(Some(1))",
        ),
        (
            "    let s = Shared::new(Kind::A);\n",
            "Shared::new(Kind::A)",
        ),
        ("    let s: Shared<i64> = Shared::new(5);\n", "Shared<i64>"),
        (
            "    let s: Shared<Vec<i64>> = Shared::new(vec![1]);\n",
            "Shared<Vec<i64>>",
        ),
    ] {
        let text = shared_program(body);
        let (d, sources) = one_error(&text);
        assert_eq!(d.code, codes::V0216, "{text}\n{d:#?}");
        assert_eq!(d.span, span_of(&sources, needle), "{text}\n{d:#?}");
    }
}

#[test]
fn shared_written_anywhere_but_a_parameter_or_let_is_v0216() {
    for (text, needle) in [
        (
            "struct C {\n    n: i64,\n}\nstruct Holder {\n    c: Shared<C>,\n}\nfn main() {}\n",
            "Shared<C>",
        ),
        (
            "struct C {\n    n: i64,\n}\nfn make(c: C) -> Shared<C> {\n    Shared::new(c)\n}\nfn main() {}\n",
            "Shared<C>",
        ),
        (
            "struct C {\n    n: i64,\n}\nfn f(v: Vec<Shared<C>>) {}\nfn main() {}\n",
            "Shared<C>",
        ),
        (
            "struct C {\n    n: i64,\n}\nfn main() {\n    let o: Option<Shared<C>> = None;\n}\n",
            "Shared<C>",
        ),
        (
            "enum E {\n    A(Shared<i64>),\n}\nfn main() {}\n",
            "Shared<i64>",
        ),
    ] {
        let (diagnostics, sources) = errors(text);
        let d = &diagnostics[0];
        assert_eq!(d.code, codes::V0216, "{text}\n{diagnostics:#?}");
        assert_eq!(d.span, span_of(&sources, needle), "{text}\n{d:#?}");
    }
    // As a parameter's or a `let`'s type, and inferred inside a `Vec` or
    // an `Option`.
    ok(
        "struct C {\n    n: i64,\n}\nfn f(s: Shared<C>) -> i64 {\n    s.n\n}\nfn main() {\n    \
         let s: Shared<C> = Shared::new(C { n: 1 });\n    let v = vec![s.clone(), s.clone()];\n    \
         let o = Some(s.clone());\n    let n = f(s);\n}\n",
    );
}

#[test]
fn shared_with_the_wrong_arguments_is_reported() {
    let (d, _) = one_error(&shared_program("    let s = Shared::new(cfg, cfg);\n"));
    assert_eq!(d.code, codes::V0201, "{d:#?}");
    let (d, _) = one_error(&shared_program("    let s = Shared::make(cfg);\n"));
    assert_eq!(d.code, codes::V0100, "{d:#?}");
    assert!(d.message.contains("`Shared::new(..)`"), "{d:#?}");
    let (d, _) = one_error("struct C {\n    n: i64,\n}\nfn f(s: Shared<C, C>) {}\nfn main() {}\n");
    assert_eq!(d.code, codes::V0101, "{d:#?}");
}

#[test]
fn fields_and_methods_through_a_shared_type_as_on_the_struct() {
    let program = ok(&shared_program(
        "    let s = Shared::new(cfg);\n    let a = s.factor;\n    let b = s.items.len();\n    \
         let c = s.describe();\n    let d = s.name.len();\n    let e = s.items[0];\n",
    ));
    let main = function(&program, "main");
    assert_eq!(local_ty(main, "a"), I64);
    assert_eq!(local_ty(main, "b"), Ty::Int(IntKind::Usize));
    assert_eq!(local_ty(main, "c"), Ty::String);
    assert_eq!(local_ty(main, "d"), Ty::Int(IntKind::Usize));
    assert_eq!(local_ty(main, "e"), I64);
}

#[test]
fn printing_or_comparing_a_shared_is_v0203() {
    for body in [
        "    let s = Shared::new(cfg);\n    println!(\"{}\", s);\n",
        "    let s = Shared::new(cfg);\n    let same = s == s.clone();\n",
        "    let s = Shared::new(cfg);\n    let same = Some(s.clone()) == Some(s.clone());\n",
    ] {
        let text = shared_program(body);
        let (d, _) = one_error(&text);
        assert_eq!(d.code, codes::V0203, "{text}\n{d:#?}");
        assert!(d.message.contains("Shared<Config>"), "{d:#?}");
    }
}

#[test]
fn a_shared_where_its_struct_is_expected_is_v0200() {
    let text = shared_program("    let s = Shared::new(cfg);\n    let n = takes(s);\n");
    let (d, sources) = one_error(&text);
    assert_eq!(d.code, codes::V0200, "{d:#?}");
    assert_eq!(d.span.start, part_of(&sources, "takes(s)", "s)").start);
}

#[test]
fn a_shared_through_json_or_env_is_v0210() {
    let text = shared_program("    let s = Shared::new(cfg);\n    let t = json::stringify(s);\n");
    let (d, _) = one_error(&text);
    assert_eq!(d.code, codes::V0210, "{d:#?}");
    let (d, _) = one_error(
        "struct Config {\n    port: i64,\n}\nfn load() -> Result<i64, Error> {\n    \
         let c: Shared<Config> = env::parse()?;\n    Ok(c.port)\n}\nfn main() {}\n",
    );
    assert_eq!(d.code, codes::V0210, "{d:#?}");
}

// --- Varyk packages (M5b2 spec 2.3, 2.4) ------------------------------------

mod packages {
    use super::*;
    use crate::test_packages::{Dep, units_and_route};

    const BOTH: [(&str, Dep<'static>); 2] = [
        ("units", Dep::Package("units")),
        ("route", Dep::Package("route")),
    ];

    fn checks(text: &str) {
        let mut build = units_and_route();
        if let Err(diagnostics) = build.check(text, &BOTH) {
            panic!("expected success for:\n{text}\ngot {diagnostics:#?}");
        }
    }

    fn one(text: &str) -> Diagnostic {
        let mut build = units_and_route();
        match build.check(text, &BOTH) {
            Ok(_) => panic!("expected a diagnostic for:\n{text}"),
            Err(diagnostics) => only(&diagnostics).clone(),
        }
    }

    const LEGS: &str = "fn legs() -> Vec<route::Leg> {\n    vec![\n        route::Leg { name: \"a\", length: units::length::Meters { value: 3 } },\n        route::Leg { name: \"b\", length: units::length::Meters { value: 4 } },\n    ]\n}\n";

    #[test]
    fn a_borrowed_return_of_a_package_s_function_is_an_alias() {
        let d = one(&format!(
            "{LEGS}fn main() {{\n    let mut all = legs();\n    let best = route::longest(all);\n    all.push(route::Leg {{ name: \"c\", length: units::length::zero() }});\n    println!(\"{{}}\", best.name);\n}}\n"
        ));
        assert_eq!(d.code, codes::V0307, "{d:#?}");
        checks(&format!(
            "{LEGS}fn main() {{\n    let all = legs();\n    let best = route::longest(all);\n    println!(\"{{}}\", best.name);\n}}\n"
        ));
    }

    #[test]
    fn a_mut_parameter_of_a_package_s_function_needs_a_mut_place() {
        checks(
            "fn main() {\n    let mut m = units::length::zero();\n    units::length::grow(m);\n    println!(\"{}\", m.value);\n}\n",
        );
        let d = one(
            "fn main() {\n    let m = units::length::zero();\n    units::length::grow(m);\n}\n",
        );
        assert_eq!(d.code, codes::V0302, "{d:#?}");
    }

    #[test]
    fn a_package_s_enum_matches_with_named_fields_and_must_be_exhaustive() {
        checks(
            "fn size(shape: units::Shape) -> i32 {\n    match shape {\n        units::Shape::Circle { radius } => radius,\n        units::Shape::Square(side) => side,\n        units::Shape::Point => 0,\n    }\n}\nfn main() {\n    println!(\"{}\", size(units::Shape::Circle { radius: 2 }));\n}\n",
        );
        let d = one(
            "fn size(shape: units::Shape) -> i32 {\n    match shape {\n        units::Shape::Circle { radius } => radius,\n        units::Shape::Point => 0,\n    }\n}\nfn main() {}\n",
        );
        assert_eq!(d.code, codes::V0204, "{d:#?}");
    }

    #[test]
    fn clone_and_equality_work_on_a_package_s_struct() {
        checks(
            "fn main() {\n    let a = units::length::zero();\n    let b = a.clone();\n    if a == b {\n        println!(\"same\");\n    }\n    println!(\"{}\", a.doubled().value);\n}\n",
        );
    }

    #[test]
    fn a_meters_from_route_goes_straight_to_units() {
        checks(&format!(
            "{LEGS}fn main() {{\n    let all = legs();\n    let sum = units::length::add(route::total(all), units::length::zero());\n    println!(\"{{}}\", sum.value);\n}}\n"
        ));
    }

    #[test]
    fn a_single_name_is_never_a_package() {
        let d = one("fn main() {\n    let u = units;\n}\n");
        assert_eq!(d.code, codes::V0100, "{d:#?}");
    }

    #[test]
    fn a_call_through_a_dependency_varyk_code_cannot_name_shows_the_rename() {
        let mut build = units_and_route();
        let deps = [("varyk-units", Dep::Package("units"))];
        let diagnostics = build
            .check("fn main() {\n    let n = varyk_units::one();\n}\n", &deps)
            .err()
            .unwrap_or_default();
        let d = only(&diagnostics);
        assert_eq!(d.code, codes::V0100, "{d:#?}");
        assert!(
            d.notes
                .iter()
                .any(|note| note.contains("`varyk_units_package = { package = \"units\", .. }`")),
            "{d:#?}"
        );
    }

    #[test]
    fn a_rust_dependency_named_alone_or_as_a_method_gets_no_rename_note() {
        let mut build = units_and_route();
        let deps = [("helper", Dep::Rust), ("units", Dep::Package("units"))];
        for text in [
            "fn main() {\n    let n = helper;\n}\n",
            "fn main() {\n    let m = units::length::zero();\n    let n = m.helper();\n}\n",
        ] {
            let diagnostics = build.check(text, &deps).err().unwrap_or_default();
            let d = only(&diagnostics);
            assert_eq!(d.code, codes::V0100, "{d:#?}");
            assert!(
                !d.notes
                    .iter()
                    .any(|note| note.contains("also a dependency")),
                "{d:#?}"
            );
        }
    }

    #[test]
    fn a_value_of_a_package_this_one_does_not_list_is_v0115() {
        let route_only = [("route", Dep::Package("route"))];
        for text in [
            "fn length(leg: route::Leg) -> i32 {\n    leg.length.value\n}\nfn main() {}\n",
            "fn sum(legs: Vec<route::Leg>) {\n    let total = route::total(legs);\n}\nfn main() {}\n",
        ] {
            let mut build = units_and_route();
            let diagnostics = build.check(text, &route_only).err().unwrap_or_default();
            let d = only(&diagnostics);
            assert_eq!(d.code, codes::V0115, "{d:#?}");
            assert_eq!(
                d.message,
                "this is a `Meters` from the package `units`, which this package does not list"
            );
        }
        // A `route::Leg`, which holds a `Meters`, is fine.
        let mut build = units_and_route();
        let text = "fn name(leg: route::Leg) -> string {\n    leg.name.clone()\n}\nfn main() {}\n";
        if let Err(diagnostics) = build.check(text, &route_only) {
            panic!("{diagnostics:#?}");
        }
        // As is the `Meters` for a program that lists `units`.
        checks("fn length(leg: route::Leg) -> i32 {\n    leg.length.value\n}\nfn main() {}\n");
    }

    #[test]
    fn a_value_holding_a_type_of_an_unlisted_package_is_v0115() {
        let mut build = units_and_route();
        let text = "fn count(legs: Vec<route::Leg>) -> usize {\n    route::lengths(legs).len()\n}\nfn main() {}\n";
        let diagnostics = build
            .check(text, &[("route", Dep::Package("route"))])
            .err()
            .unwrap_or_default();
        let d = only(&diagnostics);
        assert_eq!(d.code, codes::V0115, "{d:#?}");
        assert_eq!(
            d.message,
            "this `Vec<Meters>` holds a `Meters` from the package `units`, which this package \
             does not list"
        );
    }

    #[test]
    fn a_package_s_struct_holding_an_error_does_not_make_its_user_need_std() {
        let mut build = units_and_route();
        let program = build
            .check(
                "fn keep(failure: units::Failure) {}\nfn main() {\n    println!(\"{}\", units::one());\n}\n",
                &BOTH,
            )
            .unwrap_or_else(|diagnostics| panic!("{diagnostics:#?}"));
        assert!(!program.uses_std);
        // The package itself, which names `Error`, does.
        assert!(build.checked[0].program.uses_std);
    }

    #[test]
    fn a_private_field_of_a_package_s_struct_is_v0105() {
        let mut build = units_and_route();
        let d = match build.check(
            "fn main() {\n    let s = units::length::Secret { hidden: 1 };\n}\n",
            &BOTH,
        ) {
            Ok(_) => panic!("expected V0105"),
            Err(diagnostics) => only(&diagnostics).clone(),
        };
        assert_eq!(d.code, codes::V0105, "{d:#?}");
    }

    /// Milestone 5c spec 2.5: a package's facade takes and gives `Time`,
    /// `Uuid`, and `Bytes`, and its enum holds a `Bytes`.
    #[test]
    fn a_package_s_facade_takes_and_gives_time_uuid_and_bytes() {
        let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/packages/facade_types");
        let mut build = crate::test_packages::Build::default();
        build.add_dir("facade_types", dir, &[("varyk-std", Dep::Rust)]);
        let text = "fn main() {\n    let b = Bytes::from_text(\"abc\");\n    let n = facade_types::ext::size(b);\n    let kept = facade_types::ext::keep(b);\n    let at = facade_types::ext::read_time(\"2026-10-07T12:00:00Z\");\n    let id = facade_types::ext::same(Uuid::new());\n    match facade_types::ext::next(true) {\n        facade_types::ext::Message::Binary(m) => println!(\"{}\", m.len()),\n        facade_types::ext::Message::Text(text) => println!(\"{}\", text),\n    }\n    println!(\"{} {} {} {}\", n, kept.len(), at.is_some(), id);\n}\n";
        let deps = [("facade_types", Dep::Package("facade_types"))];
        let program = build
            .check(text, &deps)
            .unwrap_or_else(|diagnostics| panic!("{diagnostics:#?}"));
        let main = function(&program, "main");
        assert_eq!(local_ty(main, "n"), Ty::Int(IntKind::Usize));
        assert_eq!(local_ty(main, "kept"), Ty::Bytes);
        assert_eq!(local_ty(main, "at"), Ty::Option(Box::new(Ty::Time)));
        assert_eq!(local_ty(main, "id"), Ty::Uuid);
        assert_eq!(local_ty(main, "m"), Ty::Bytes);
    }
}

/// Programs using the stub `varyk-http` (milestone 5b4 spec 2.1, 6).
mod http {
    use super::*;
    use crate::builtins::Owner;
    use crate::hir::{
        Binding, HirExprKind, HirHook, HirRoute, HirStmt, HookKind, HttpMethod, ReturnShape,
        Segment,
    };
    use crate::resolve::StructId;
    use crate::test_packages::{Build, Dep, varyk_http};
    use crate::types::check::routes::parse_path;

    const HTTP: [(&str, Dep<'static>); 1] = [("http", Dep::Package("varyk-http"))];

    const STATE: &str = "struct State {\n    name: string,\n}\n";

    fn checks(text: &str) -> HirProgram {
        let mut build = varyk_http();
        build.check(text, &HTTP).unwrap_or_else(|diagnostics| {
            panic!("expected success for:\n{text}\ngot {diagnostics:#?}")
        })
    }

    fn one(text: &str) -> Diagnostic {
        let mut build = varyk_http();
        match build.check(text, &HTTP) {
            Ok(_) => panic!("expected a diagnostic for:\n{text}"),
            Err(diagnostics) => only(&diagnostics).clone(),
        }
    }

    /// The value of the first `let` of the function `name`.
    fn first_let<'a>(program: &'a HirProgram, name: &str) -> &'a HirExpr {
        let function = program
            .functions
            .iter()
            .find(|function| function.name == name)
            .unwrap_or_else(|| panic!("no function {name}"));
        function
            .body
            .stmts
            .iter()
            .find_map(|stmt| match stmt {
                HirStmt::Let { value, .. } => Some(value),
                _ => None,
            })
            .unwrap_or_else(|| panic!("no let in {name}"))
    }

    fn struct_named(program: &HirProgram, ty: &Ty) -> String {
        match ty {
            Ty::Struct(id) => program.structs[id.0 as usize].name.clone(),
            other => panic!("not a struct: {other:?}"),
        }
    }

    #[test]
    fn app_new_types_as_the_package_s_app_and_records_the_state_type() {
        let program = checks(&format!(
            "{STATE}fn main() {{\n    let app = http::App::new(Shared::new(State {{ name: \"a\" }}));\n}}\n"
        ));
        let value = first_let(&program, "main");
        assert_eq!(struct_named(&program, &value.ty), "App");
        let Ty::Struct(app) = value.ty else {
            unreachable!()
        };
        let package = program.structs[app.0 as usize].package.as_ref();
        assert_eq!(package.map(|item| item.name.as_str()), Some("varyk-http"));
        let HirExprKind::Call {
            callee: Callee::Builtin(id),
            args,
            ..
        } = &value.kind
        else {
            panic!("expected the intrinsic call, got {value:#?}");
        };
        assert_eq!(id.get().owner, Owner::App);
        let [state] = args.as_slice() else {
            panic!("one argument: {args:#?}");
        };
        let Ty::Shared(inner) = &state.ty else {
            panic!("a Shared: {state:#?}");
        };
        assert_eq!(struct_named(&program, inner), "State");
    }

    #[test]
    fn app_new_is_reached_through_any_path_to_the_package_s_app() {
        checks(&format!(
            "use http::App;\n{STATE}fn main() {{\n    let state = Shared::new(State {{ name: \"a\" }});\n    let app = App::new(state);\n    let other = http::server::App::new(Shared::new(State {{ name: \"b\" }}));\n}}\n"
        ));
    }

    #[test]
    fn app_new_makes_the_program_use_std_and_log() {
        let program = checks(&format!(
            "{STATE}fn main() {{\n    let app = http::App::new(Shared::new(State {{ name: \"a\" }}));\n}}\n"
        ));
        assert!(program.uses_std);
        assert!(program.logs);
        // Naming the package's types alone does neither.
        let program = checks(
            "fn main() {\n    let r = http::Response::empty();\n    println!(\"{}\", r.status());\n}\n",
        );
        assert!(!program.logs);
    }

    #[test]
    fn app_new_takes_a_shared() {
        let d = one("fn main() {\n    let app = http::App::new(3);\n}\n");
        assert_eq!(d.code, codes::V0200, "{d:#?}");
        let d = one(&format!(
            "{STATE}fn main() {{\n    let app = http::App::new(State {{ name: \"a\" }});\n}}\n"
        ));
        assert_eq!(d.code, codes::V0200, "{d:#?}");
        let d = one(&format!(
            "{STATE}fn main() {{\n    let a = http::App::new(Shared::new(State {{ name: \"a\" }}), 1);\n}}\n"
        ));
        assert_eq!(d.code, codes::V0201, "{d:#?}");
    }

    #[test]
    fn app_new_takes_its_state_as_an_owned_slot() {
        // Given away: a second use is V0305, as after `Shared::new`.
        let d = one(&format!(
            "{STATE}fn main() {{\n    let state = Shared::new(State {{ name: \"a\" }});\n    let app = http::App::new(state);\n    let again = http::App::new(state);\n}}\n"
        ));
        assert_eq!(d.code, codes::V0305, "{d:#?}");
        // A borrowed parameter cannot be given away: V0304.
        let d = one(&format!(
            "{STATE}fn build(state: Shared<State>) -> http::App {{\n    http::App::new(state)\n}}\nfn main() {{}}\n"
        ));
        assert_eq!(d.code, codes::V0304, "{d:#?}");
        checks(&format!(
            "{STATE}fn build(state: Shared<State>) -> http::App {{\n    http::App::new(state.clone())\n}}\nfn main() {{}}\n"
        ));
    }

    #[test]
    fn an_app_may_be_passed_returned_and_held_in_a_field() {
        checks(&format!(
            "{STATE}struct Server {{\n    app: http::App,\n}}\nfn build(state: Shared<State>) -> http::App {{\n    let mut app = http::App::new(state.clone());\n    app\n}}\nfn port(app: http::App) -> i32 {{\n    8080\n}}\nfn main() {{\n    let server = Server {{ app: build(Shared::new(State {{ name: \"a\" }})) }};\n    println!(\"{{}}\", port(server.app));\n}}\n"
        ));
    }

    #[test]
    fn app_new_as_a_value_is_v0100() {
        let d = one("fn main() {\n    let f = http::App::new;\n}\n");
        assert_eq!(d.code, codes::V0100, "{d:#?}");
    }

    #[test]
    fn a_struct_app_of_another_package_is_an_ordinary_struct() {
        let mut build = Build::default();
        build.add("not_http", &[]);
        let deps = [("web", Dep::Package("not_http"))];
        let program = build
            .check("fn main() {\n    let app = web::App::new();\n}\n", &deps)
            .unwrap_or_else(|diagnostics| panic!("{diagnostics:#?}"));
        let value = first_let(&program, "main");
        assert!(
            matches!(
                &value.kind,
                HirExprKind::Call {
                    callee: Callee::Imported(_) | Callee::Varyk(_),
                    ..
                }
            ),
            "{value:#?}"
        );
        assert!(!program.logs);
    }

    #[test]
    fn a_varyk_http_without_request_is_v0407() {
        let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/errors/v0407_varyk_http_without_request/varyk-http");
        let mut build = Build::default();
        build.add_dir("varyk-http", dir, &[]);
        let d = match build.check("fn main() {}\n", &HTTP) {
            Ok(_) => panic!("expected V0407"),
            Err(diagnostics) => only(&diagnostics).clone(),
        };
        assert_eq!(d.code, codes::V0407, "{d:#?}");
        assert!(
            d.notes.iter().any(|note| note.contains("`Request`")),
            "{d:#?}"
        );
    }

    // --- Routes (milestone 5b4 spec 2.1 to 2.3) ---------------------------

    const USERS: &str = "struct State {\n    name: string,\n}\nstruct User {\n    id: i64,\n    name: string,\n}\nstruct NewUser {\n    name: string,\n}\n";

    /// `USERS`, then `handlers`, then a `main` making an app and running
    /// `routes`, each a line of its body.
    fn app_program(handlers: &str, routes: &str) -> String {
        format!(
            "{USERS}{handlers}fn main() {{\n    let mut app = http::App::new(Shared::new(State {{ name: \"a\" }}));\n{routes}}}\n"
        )
    }

    /// The routes of `name`'s body, in order.
    fn routes_of<'a>(program: &'a HirProgram, name: &str) -> Vec<&'a HirRoute> {
        function(program, name)
            .body
            .stmts
            .iter()
            .filter_map(|stmt| match stmt {
                HirStmt::Route(route) => Some(route),
                _ => None,
            })
            .collect()
    }

    fn struct_id(program: &HirProgram, name: &str) -> StructId {
        let index = program
            .structs
            .iter()
            .position(|def| def.name == name)
            .unwrap_or_else(|| panic!("no struct {name}"));
        StructId(index as u32)
    }

    // --- The package named `varyk-http` itself (milestone 5b4 spec 2.1) ---

    /// A library whose `src/lib.vr` is `lib`, its `src/server.rs` the
    /// stub `varyk-http`'s, in a scratch directory.
    fn library_with_the_stub_s_server(lib: &str) -> crate::package::tests::TempDir {
        let stub = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/packages/varyk-http/src/server.rs");
        let server = std::fs::read_to_string(stub).expect("the stub's server.rs");
        let dir = crate::package::tests::TempDir::new("own_http");
        dir.write("src/server.rs", &server);
        dir.write("src/lib.vr", lib);
        dir
    }

    /// A library that names its own `App` at its root and adds a route to
    /// it.
    const OWN_APP: &str = "pub mod server;\n\npub use server::App;\npub use server::Request;\npub use server::Response;\n\nstruct State {\n    name: string,\n}\nasync fn home() {}\npub fn build() -> App {\n    let mut app = App::new(Shared::new(State { name: \"a\" }));\n    app.get(\"/\", home);\n    app\n}\n";

    #[test]
    fn the_stub_s_own_test_adds_a_route_to_its_own_app() {
        let build = varyk_http();
        let program = &build.checked[0].program;
        let routes = routes_of(program, "a_route_answers_a_request");
        assert_eq!(routes.len(), 1, "{routes:#?}");
    }

    #[test]
    fn a_library_named_varyk_http_marks_its_own_app() {
        let dir = library_with_the_stub_s_server(OWN_APP);
        let mut build = Build::default();
        build
            .try_add_dir("varyk-http", dir.0.clone(), &[("varyk-std", Dep::Rust)])
            .unwrap_or_else(|diagnostics| panic!("{diagnostics:#?}"));
        let program = &build.checked[0].program;
        assert_eq!(routes_of(program, "build").len(), 1);
        let value = first_let(program, "build");
        assert!(
            matches!(
                &value.kind,
                HirExprKind::Call {
                    callee: Callee::Builtin(id),
                    ..
                } if id.get().owner == Owner::App
            ),
            "{value:#?}"
        );
    }

    #[test]
    fn a_library_named_otherwise_does_not_mark_its_app() {
        let dir = library_with_the_stub_s_server(OWN_APP);
        let mut build = Build::default();
        let diagnostics =
            match build.try_add_dir("not-http", dir.0.clone(), &[("varyk-std", Dep::Rust)]) {
                Ok(()) => panic!("`App::new` is the facade's own, which Varyk cannot call"),
                Err(diagnostics) => diagnostics,
            };
        assert!(
            diagnostics.iter().any(|d| d.code == codes::V0108),
            "{diagnostics:#?}"
        );
    }

    #[test]
    fn a_varyk_http_depending_on_another_marks_both_apps() {
        let lib = format!(
            "{OWN_APP}pub fn build_inner() -> inner::App {{\n    let mut app = inner::App::new(Shared::new(State {{ name: \"b\" }}));\n    app.get(\"/\", home);\n    app\n}}\n"
        );
        let dir = library_with_the_stub_s_server(&lib);
        let mut build = varyk_http();
        build
            .try_add_dir(
                "varyk-http",
                dir.0.clone(),
                &[
                    ("varyk-std", Dep::Rust),
                    ("inner", Dep::Package("varyk-http")),
                ],
            )
            .unwrap_or_else(|diagnostics| panic!("{diagnostics:#?}"));
        let program = &build.checked[1].program;
        let app_of = |name: &str| {
            let routes = routes_of(program, name);
            let [route] = routes.as_slice() else {
                panic!("one route in {name}: {routes:#?}");
            };
            match function(program, name).locals[route.app.0 as usize].ty {
                Ty::Struct(id) => program.structs[id.0 as usize].package.is_some(),
                ref other => panic!("not an app: {other:?}"),
            }
        };
        assert!(!app_of("build"), "the package's own `App`");
        assert!(app_of("build_inner"), "the `App` of `inner`");
    }

    #[test]
    fn a_route_is_a_statement_with_its_method_path_and_handler() {
        let program = checks(&app_program(
            "async fn get_user(id: i64) -> Option<User> {\n    None\n}\nasync fn home() {}\n",
            "    app.get(\"/users/{id}\", get_user);\n    app.delete(\"/\", home);\n",
        ));
        let routes = routes_of(&program, "main");
        let [user, home] = routes.as_slice() else {
            panic!("two routes: {routes:#?}");
        };
        assert_eq!(user.method, HttpMethod::Get);
        assert_eq!(
            user.path,
            vec![
                Segment::Literal("users".to_string()),
                Segment::Param("id".to_string())
            ]
        );
        assert_eq!(program.function(user.handler).name, "get_user");
        assert_eq!(user.params, vec![Binding::Path("id".to_string())]);
        let main = function(&program, "main");
        assert_eq!(main.locals[user.app.0 as usize].name, "app");
        assert_eq!(home.method, HttpMethod::Delete);
        assert!(home.path.is_empty());
        assert_eq!(home.ret, ReturnShape::Nothing);
    }

    #[test]
    fn each_parameter_binds_by_the_first_rule_that_fits() {
        let program = checks(&app_program(
            "async fn update(\n    req: http::Request,\n    q: Option<string>,\n    id: i64,\n    mut user: NewUser,\n    state: Shared<State>,\n    page: u32,\n    name: string,\n    flag: bool,\n) {}\n",
            "    app.put(\"/users/{id}/{name}\", update);\n",
        ));
        let routes = routes_of(&program, "main");
        let request = struct_id(&program, "Request");
        assert_eq!(
            routes[0].params,
            vec![
                Binding::Package(request),
                Binding::Query("q".to_string()),
                Binding::Path("id".to_string()),
                Binding::Body,
                Binding::State,
                Binding::Query("page".to_string()),
                Binding::Path("name".to_string()),
                Binding::Query("flag".to_string()),
            ]
        );
    }

    #[test]
    fn a_body_binds_on_post_put_and_patch() {
        let program = checks(&app_program(
            "async fn add(users: Vec<NewUser>) {}\nasync fn tags(tags: HashMap<string, i64>) {}\n",
            "    app.post(\"/users\", add);\n    app.patch(\"/tags\", tags);\n",
        ));
        let routes = routes_of(&program, "main");
        assert_eq!(routes[0].params, vec![Binding::Body]);
        assert_eq!(routes[1].params, vec![Binding::Body]);
    }

    /// Milestone 5c spec 2.4: a `Time` or `Uuid` is a path parameter, and
    /// one or an `Option` of one a query parameter.
    #[test]
    fn a_time_or_uuid_path_or_query_parameter_binds() {
        let program = checks(&app_program(
            "async fn h(id: Uuid, at: Time, since: Time, until: Option<Time>, owner: Uuid, by: Option<Uuid>) {}\n",
            "    app.get(\"/items/{id}/{at}\", h);\n",
        ));
        let routes = routes_of(&program, "main");
        assert_eq!(
            routes[0].params,
            vec![
                Binding::Path("id".to_string()),
                Binding::Path("at".to_string()),
                Binding::Query("since".to_string()),
                Binding::Query("until".to_string()),
                Binding::Query("owner".to_string()),
                Binding::Query("by".to_string()),
            ]
        );
    }

    /// Milestone 5c spec 2.4: a `Bytes` is neither a path nor a query
    /// parameter (V0219), and the texts list `Time` and `Uuid`.
    #[test]
    fn a_bytes_path_or_query_parameter_is_v0219() {
        let d = one(&app_program(
            "async fn h(data: Bytes) {}\n",
            "    app.get(\"/files/{data}\", h);\n",
        ));
        assert_eq!(d.code, codes::V0219, "{d:#?}");
        assert!(
            d.message
                .contains("an integer, `bool`, `string`, `Time`, or `Uuid`"),
            "{d:#?}"
        );
        for parameter in ["data: Bytes", "data: Option<Bytes>"] {
            let d = one(&app_program(
                &format!("async fn h({parameter}) {{}}\n"),
                "    app.get(\"/files\", h);\n",
            ));
            assert_eq!(d.code, codes::V0219, "{d:#?}");
            assert!(
                d.notes.iter().any(|note| note.contains(
                    "an integer, `bool`, `string`, `Time`, `Uuid`, or an `Option` of one"
                )),
                "{d:#?}"
            );
        }
        let d = one(&app_program(
            "async fn h() {}\n",
            "    app.get(\"/files/{id}\", h);\n",
        ));
        assert!(
            d.notes
                .iter()
                .any(|note| note.contains("an integer type, `bool`, `string`, `Time`, or `Uuid`")),
            "{d:#?}"
        );
    }

    /// Milestone 5c spec 2.4: a struct holding the three is a route's body
    /// and return value.
    #[test]
    fn a_route_s_body_and_return_may_hold_time_uuid_and_bytes() {
        let program = checks(&app_program(
            "struct Record {\n    id: Uuid,\n    at: Time,\n    data: Bytes,\n}\nasync fn put(record: Record) -> Record {\n    Record { id: record.id, at: record.at, data: record.data.clone() }\n}\n",
            "    app.post(\"/records\", put);\n",
        ));
        let routes = routes_of(&program, "main");
        assert_eq!(routes[0].params, vec![Binding::Body]);
        assert_eq!(routes[0].ret, ReturnShape::Json);
        let record = &program.structs[struct_id(&program, "Record").0 as usize];
        assert!(record.serde.serialize && record.serde.deserialize);
    }

    #[test]
    fn each_return_shape_is_recorded() {
        let handlers = "async fn nothing() {}\nasync fn one() -> User {\n    User { id: 1, name: \"a\" }\n}\nasync fn many() -> Vec<User> {\n    Vec::new()\n}\nasync fn text() -> string {\n    \"a\"\n}\nasync fn maybe() -> Option<User> {\n    None\n}\nasync fn built() -> http::Response {\n    http::Response::empty()\n}\nasync fn tried() -> Result<User, Error> {\n    Err(Error::new(\"no\"))\n}\nasync fn tried_maybe() -> Result<Option<User>, Error> {\n    Ok(None)\n}\nasync fn tried_built() -> Result<http::Response, Error> {\n    Ok(http::Response::empty())\n}\n";
        let routes = "    app.get(\"/a\", nothing);\n    app.get(\"/b\", one);\n    app.get(\"/c\", many);\n    app.get(\"/d\", text);\n    app.get(\"/e\", maybe);\n    app.get(\"/f\", built);\n    app.get(\"/g\", tried);\n    app.get(\"/h\", tried_maybe);\n    app.get(\"/i\", tried_built);\n";
        let program = checks(&app_program(handlers, routes));
        let shapes: Vec<ReturnShape> = routes_of(&program, "main")
            .iter()
            .map(|route| route.ret.clone())
            .collect();
        let result = |shape| ReturnShape::Result(Box::new(shape));
        assert_eq!(
            shapes,
            vec![
                ReturnShape::Nothing,
                ReturnShape::Json,
                ReturnShape::Json,
                ReturnShape::Json,
                ReturnShape::Option,
                ReturnShape::Response,
                result(ReturnShape::Json),
                result(ReturnShape::Option),
                result(ReturnShape::Response),
            ]
        );
    }

    #[test]
    fn the_body_is_read_from_json_and_the_return_written() {
        let program = checks(&app_program(
            "async fn add(user: NewUser) -> Result<Option<User>, Error> {\n    Ok(None)\n}\n",
            "    app.post(\"/users\", add);\n",
        ));
        let new_user = &program.structs[struct_id(&program, "NewUser").0 as usize];
        assert!(new_user.serde.deserialize && !new_user.serde.serialize);
        let user = &program.structs[struct_id(&program, "User").0 as usize];
        assert!(user.serde.serialize && !user.serde.deserialize);
    }

    #[test]
    fn a_route_in_one_branch_of_an_if_is_accepted() {
        checks(&app_program(
            "async fn home() {}\n",
            "    let open = true;\n    if open {\n        app.get(\"/\", home);\n    } else {\n        app.get(\"/closed\", home)\n    }\n",
        ));
    }

    #[test]
    fn a_handler_may_be_named_through_a_path() {
        checks(&app_program(
            "async fn home() {}\n",
            "    app.get(\"/\", home);\n    app.get(\"/home\", crate::home);\n",
        ));
    }

    #[test]
    fn a_second_shared_and_two_requests_are_v0219() {
        let d = one(&app_program(
            "async fn h(a: Shared<State>, b: Shared<State>) {}\n",
            "    app.get(\"/\", h);\n",
        ));
        assert_eq!(d.code, codes::V0219, "{d:#?}");
        assert!(d.message.contains("`b`"), "{d:#?}");
        let d = one(&app_program(
            "async fn h(a: http::Request, b: http::Request) {}\n",
            "    app.get(\"/\", h);\n",
        ));
        assert_eq!(d.code, codes::V0219, "{d:#?}");
        assert!(d.message.contains("`b`"), "{d:#?}");
    }

    #[test]
    fn a_method_or_a_rust_function_as_the_handler_is_v0220() {
        let d = one(&app_program(
            "impl State {\n    async fn list(self) {}\n    async fn make() {}\n}\n",
            "    app.get(\"/\", State::make);\n",
        ));
        assert_eq!(d.code, codes::V0220, "{d:#?}");
        let d = one(&app_program(
            "",
            "    app.get(\"/\", http::respond_empty);\n",
        ));
        assert_eq!(d.code, codes::V0220, "{d:#?}");
        let mut build = varyk_http();
        let diagnostics = build
            .resolve_fixture("http_rust_handler", &HTTP)
            .and_then(|resolved| typecheck(resolved, &build.sources))
            .err()
            .unwrap_or_else(|| panic!("expected V0220"));
        let d = only(&diagnostics);
        assert_eq!(d.code, codes::V0220, "{d:#?}");
        assert!(d.message.contains("Rust"), "{d:#?}");
    }

    #[test]
    fn a_repeated_name_and_a_path_without_its_slash_are_v0222() {
        let d = one(&app_program(
            "async fn h(id: i64) {}\n",
            "    app.get(\"/a/{id}/{id}\", h);\n",
        ));
        assert_eq!(d.code, codes::V0222, "{d:#?}");
        let d = one(&app_program(
            "async fn h() {}\n",
            "    app.get(\"users\", h);\n",
        ));
        assert_eq!(d.code, codes::V0222, "{d:#?}");
    }

    #[test]
    fn a_route_in_a_closure_or_used_as_a_value_is_v0221() {
        let d = one(&app_program(
            "async fn h() {}\n",
            "    let x = Some(1).map(|n| {\n        app.get(\"/\", h);\n        n\n    });\n",
        ));
        assert_eq!(d.code, codes::V0221, "{d:#?}");
        let d = one(&app_program(
            "async fn h() {}\n",
            "    let r = app.get(\"/\", h);\n",
        ));
        assert_eq!(d.code, codes::V0221, "{d:#?}");
        let d = one(&app_program(
            "async fn h() {}\n",
            "    let mut other = app;\n    other.get(\"/\", h);\n",
        ));
        assert_eq!(d.code, codes::V0221, "{d:#?}");
        let d = one(&app_program(
            "async fn h() {}\n",
            "    let mut n = 0;\n    while n < 2 {\n        app.post(\"/\", h);\n        n = n + 1;\n    }\n",
        ));
        assert_eq!(d.code, codes::V0221, "{d:#?}");
    }

    #[test]
    fn a_route_in_a_while_condition_is_v0221() {
        let d = one(&app_program(
            "async fn h() {}\n",
            "    while {\n        app.get(\"/\", h);\n        false\n    } {\n    }\n",
        ));
        assert_eq!(d.code, codes::V0221, "{d:#?}");
        assert!(d.message.contains("loop"), "{d:#?}");
    }

    #[test]
    fn the_route_call_shadows_the_facade_s_method_and_others_stay_ordinary() {
        // `get` on any other value is that value's method.
        checks(&app_program(
            "async fn h() {}\n",
            "    let v = vec![1];\n    let first = v.get(0);\n    app.get(\"/\", h);\n",
        ));
    }

    #[test]
    fn the_path_parser_takes_the_spec_s_paths_and_refuses_the_rest() {
        assert_eq!(parse_path("/"), Ok(Vec::new()));
        assert_eq!(
            parse_path("/a-b_c.d~e/{x_1}"),
            Ok(vec![
                Segment::Literal("a-b_c.d~e".to_string()),
                Segment::Param("x_1".to_string())
            ])
        );
        for bad in [
            "", "a", "//", "/a//b", "/a/", "/a*", "/{}", "/{1x}", "/{a}/{a}", "/{a", "/a{b}", "/é",
            "/a b", "/{_}",
        ] {
            assert!(parse_path(bad).is_err(), "{bad:?}");
        }
    }

    // --- Hooks (milestone 5b4 spec 2.4) -----------------------------------

    /// The hooks of `name`'s body, in order.
    fn hooks_of<'a>(program: &'a HirProgram, name: &str) -> Vec<&'a HirHook> {
        function(program, name)
            .body
            .stmts
            .iter()
            .filter_map(|stmt| match stmt {
                HirStmt::Hook(hook) => Some(hook),
                _ => None,
            })
            .collect()
    }

    const HOOKS: &str = "async fn check(req: http::Request) -> Result<bool, Error> {\n    Ok(true)\n}\nasync fn admin(req: http::Request, state: Shared<State>) -> Result<bool, Error> {\n    Ok(false)\n}\nasync fn stamp(req: http::Request, mut res: http::Response) {\n    res.set_header(\"x-a\", \"b\");\n}\nasync fn count(request: http::Request, mut response: http::Response, s: Shared<State>) {\n    response.set_header(\"x-name\", s.name);\n}\nasync fn look(req: http::Request, res: http::Response) {\n    println!(\"{}\", res.status());\n}\n";

    #[test]
    fn each_hook_signature_is_accepted_with_and_without_the_state() {
        let program = checks(&app_program(
            HOOKS,
            "    app.before(check);\n    app.before_on(\"/admin/users\", admin);\n    app.after(stamp);\n    app.after(count);\n    app.after(look);\n",
        ));
        let hooks = hooks_of(&program, "main");
        let shapes: Vec<_> = hooks
            .iter()
            .map(|hook| {
                (
                    hook.kind,
                    hook.prefix.clone(),
                    program.function(hook.hook).name.as_str(),
                    hook.state,
                    hook.call(),
                )
            })
            .collect();
        let admin = vec![
            Segment::Literal("admin".to_string()),
            Segment::Literal("users".to_string()),
        ];
        assert_eq!(
            shapes,
            vec![
                (HookKind::Before, None, "check", false, "before"),
                (HookKind::Before, Some(admin), "admin", true, "before_on"),
                (HookKind::After, None, "stamp", false, "after"),
                (HookKind::After, None, "count", true, "after"),
                // An `after` hook that only reads the response.
                (HookKind::After, None, "look", false, "after"),
            ]
        );
        let main = function(&program, "main");
        assert_eq!(main.locals[hooks[0].app.0 as usize].name, "app");
        // A hook and a route of one app, in any order.
        checks(&app_program(
            &format!("{HOOKS}async fn home() {{}}\n"),
            "    app.before(check);\n    app.get(\"/\", home);\n    app.after(stamp);\n    app.before_on(\"/\", check);\n",
        ));
    }

    #[test]
    fn a_before_in_a_loop_an_after_on_a_parameter_or_a_hook_as_a_value_is_v0221() {
        let d = one(&app_program(
            HOOKS,
            "    for i in 0..2 {\n        app.before(check);\n    }\n",
        ));
        assert_eq!(d.code, codes::V0221, "{d:#?}");
        assert!(
            d.message.contains("a hook cannot be added inside a loop"),
            "{d:#?}"
        );
        let d = one(&format!(
            "{USERS}{HOOKS}fn add(mut app: http::App) {{\n    app.after(stamp);\n}}\nfn main() {{}}\n"
        ));
        assert_eq!(d.code, codes::V0221, "{d:#?}");
        assert!(d.message.starts_with("hooks are added"), "{d:#?}");
        let d = one(&app_program(HOOKS, "    let r = app.after(stamp);\n"));
        assert_eq!(d.code, codes::V0221, "{d:#?}");
        assert!(d.message.contains("a hook is added"), "{d:#?}");
        let d = one(&app_program(
            HOOKS,
            "    let x = Some(1).map(|n| {\n        app.before(check);\n        n\n    });\n",
        ));
        assert_eq!(d.code, codes::V0221, "{d:#?}");
        assert!(d.message.contains("a hook cannot"), "{d:#?}");
    }

    #[test]
    fn a_hook_of_another_shape_is_v0220() {
        let cases = [
            // Out of order.
            "async fn h(state: Shared<State>, req: http::Request) -> Result<bool, Error> {\n    Ok(true)\n}\n",
            // A `mut` request.
            "async fn h(mut req: http::Request) -> Result<bool, Error> {\n    Ok(true)\n}\n",
            // Another return.
            "async fn h(req: http::Request) -> bool {\n    true\n}\n",
            // A `Shared` of another type.
            "async fn h(req: http::Request, s: Shared<User>) -> Result<bool, Error> {\n    Ok(true)\n}\n",
            // Nothing taken.
            "async fn h() -> Result<bool, Error> {\n    Ok(true)\n}\n",
            // Too much taken.
            "async fn h(req: http::Request, s: Shared<State>, n: i64) -> Result<bool, Error> {\n    Ok(true)\n}\n",
            // Not async.
            "fn h(req: http::Request) -> Result<bool, Error> {\n    Ok(true)\n}\n",
        ];
        for handler in cases {
            let d = one(&app_program(handler, "    app.before(h);\n"));
            assert_eq!(d.code, codes::V0220, "{handler}\n{d:#?}");
        }
        let d = one(&app_program(
            "async fn h(mut req: http::Request) -> Result<bool, Error> {\n    Ok(true)\n}\n",
            "    app.before(h);\n",
        ));
        assert!(
            d.notes
                .iter()
                .any(|note| note.contains("does not reach the handler")),
            "{d:#?}"
        );
        let after = [
            // A response it returns rather than changes.
            "async fn h(req: http::Request, res: http::Response) -> http::Response {\n    http::Response::empty()\n}\n",
            // No response.
            "async fn h(req: http::Request) {}\n",
            // The response first.
            "async fn h(res: http::Response, req: http::Request) {}\n",
            // A `mut` request.
            "async fn h(mut req: http::Request, res: http::Response) {}\n",
        ];
        for handler in after {
            let d = one(&app_program(handler, "    app.after(h);\n"));
            assert_eq!(d.code, codes::V0220, "{handler}\n{d:#?}");
        }
    }

    #[test]
    fn a_prefix_is_a_literal_path_without_names() {
        let d = one(&app_program(
            HOOKS,
            "    app.before_on(\"/users/{id}\", check);\n",
        ));
        assert_eq!(d.code, codes::V0222, "{d:#?}");
        let d = one(&app_program(
            HOOKS,
            "    app.before_on(\"/admin/\", check);\n",
        ));
        assert_eq!(d.code, codes::V0222, "{d:#?}");
        let d = one(&app_program(
            HOOKS,
            "    let p = \"/admin\";\n    app.before_on(p, check);\n",
        ));
        assert_eq!(d.code, codes::V0217, "{d:#?}");
        let d = one(&app_program(HOOKS, "    app.before_on(check);\n"));
        assert_eq!(d.code, codes::V0201, "{d:#?}");
        let d = one(&app_program(HOOKS, "    app.before(\"/\", check);\n"));
        assert_eq!(d.code, codes::V0201, "{d:#?}");
    }

    #[test]
    fn a_hook_s_function_is_chosen_as_a_handler_is() {
        let d = one(&app_program("", "    app.before(main);\n"));
        assert_eq!(d.code, codes::V0106, "{d:#?}");
        assert!(d.message.contains("cannot be a hook"), "{d:#?}");
        let d = one(&app_program("", "    app.after(|req, res| {});\n"));
        assert_eq!(d.code, codes::V0220, "{d:#?}");
        assert!(d.message.starts_with("a hook is named here"), "{d:#?}");
    }
}
