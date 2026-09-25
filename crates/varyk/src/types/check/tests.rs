use std::path::Path;

use varyk_syntax::{FileId, SourceFile, Span};

use super::typecheck;
use crate::diagnostics::{Diagnostic, codes};
use crate::hir::{
    HirExpr, HirExprKind, HirForHead, HirFunction, HirProgram, HirStmt, MethodRef, VariantRef,
};
use crate::resolve::{Callee, resolve};
use crate::types::{FloatKind, IntKind, ParamMode, Ty};

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
fn struct_equality_is_v0203() {
    let text =
        "struct P {\n    x: i32,\n}\nfn f(a: P, b: P) -> bool {\n    a == b\n}\nfn main() {}\n";
    let (d, sources) = one_error(text);
    assert_eq!(d.code, codes::V0203);
    assert_eq!(d.span, span_of(&sources, "a == b"));
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
    assert_eq!(d.message, "cannot find function `Task::new`");
    assert!(
        d.notes.iter().any(|n| n == "did you mean `m::Task`?"),
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
fn struct_literal_naming_an_enum_variant_is_v0001() {
    let text = "enum Shape {\n    Circle(f64),\n}\nfn main() {\n    let s = Shape::Circle { r: 1.0 };\n}\n";
    let (d, sources) = one_error(text);
    assert_eq!(d.code, codes::V0001);
    assert!(d.message.contains("milestone 4"), "{}", d.message);
    assert_eq!(d.span, span_of(&sources, "Shape::Circle { r: 1.0 }"));
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
        .position(|s| s.name == "Task")
        .expect("Task");
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
fn comparing_or_printing_an_enum_or_a_generic_type_is_v0203() {
    let text = "enum E {\n    A,\n}\nfn f(a: E, b: E, o: Option<i32>, v: Vec<i32>) -> bool {\n    println!(\"{} {}\", a, v);\n    o == o\n}\nfn main() {}\n";
    let (diagnostics, sources) = errors(text);
    let found: Vec<(&str, Span)> = diagnostics.iter().map(|d| (d.code, d.span)).collect();
    assert_eq!(
        found,
        vec![
            (codes::V0203, part_of(&sources, "a, v)", "a")),
            (codes::V0203, part_of(&sources, "a, v)", "v")),
            (codes::V0203, span_of(&sources, "o == o")),
        ],
        "{diagnostics:#?}"
    );
    assert_eq!(
        diagnostics[2].message,
        "`==` cannot be applied to `Option<i32>`"
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
            &["Vec<i32>", "`push`, `pop`, and `len`"][..],
        ),
        (
            "let x = \"a\";\n    x.nope();",
            "nope",
            &["string", "`len` and `clone`"][..],
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

const MATCH_ITEMS: &str = "enum Shape {\n    Circle(f64),\n    Rect(f64, f64),\n    Point,\n}\nenum Color {\n    Red,\n    Blue,\n}\nstruct Task {\n    status: Color,\n}\nfn mk() -> Task {\n    Task { status: Color::Red }\n}\n";

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
fn matching_something_that_is_not_an_enum_option_or_result_is_v0205() {
    let (d, sources) = one_error(&with_match(
        "",
        "    let n = 1;\n    match n {\n        _ => {}\n    }",
    ));
    assert_eq!(d.code, codes::V0205, "{d:#?}");
    assert_eq!(d.span, part_of(&sources, "match n", "n"));
    assert!(d.message.contains("`i32`"), "{d:#?}");
    assert!(d.message.contains("`if`"), "{d:#?}");
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

#[test]
fn a_variant_arm_repeated_after_an_identical_one_is_accepted() {
    ok(&with_match(
        "",
        "    match s {\n        Shape::Point => {}\n        Shape::Point => {}\n        _ => {}\n    }",
    ));
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

const LOOP_ITEMS: &str = "struct Task {\n    title: string,\n}\nfn mk() -> Task {\n    Task { title: \"t\" }\n}\nfn all() -> Vec<Task> {\n    vec![mk()]\n}\n";

/// A program with `LOOP_ITEMS` and `fn f(values: Vec<i32>, tasks: Vec<Task>,
/// n: i64, s: string, o: Option<i32>, c: bool) { body }`.
fn with_loop(body: &str) -> String {
    format!(
        "{LOOP_ITEMS}fn f(values: Vec<i32>, tasks: Vec<Task>, n: i64, s: string, o: Option<i32>, c: bool) {{\n{body}\n}}\nfn main() {{}}\n"
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
    let HirForHead::Range { start, end } = for_head(f, 0) else {
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
        HirExprKind::Try(operand) => {
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
        d.notes.iter().any(|n| n.contains("later milestone")),
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
