use std::path::Path;

use varyk_syntax::{FileId, SourceFile, Span};

use super::typecheck;
use crate::diagnostics::{Diagnostic, codes};
use crate::hir::{HirExpr, HirExprKind, HirFunction, HirProgram, HirStmt};
use crate::resolve::{Callee, resolve};
use crate::types::{FloatKind, IntKind, ParamMode, Ty};

const I32: Ty = Ty::Int(IntKind::I32);
const I64: Ty = Ty::Int(IntKind::I64);

// --- Helpers ----------------------------------------------------------------

fn check_source(entry: SourceFile) -> (Result<HirProgram, Vec<Diagnostic>>, Vec<SourceFile>) {
    let mut sources = Vec::new();
    let result = resolve(entry, &mut sources).and_then(typecheck);
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
        assert_eq!((lhs.ty, rhs.ty, tail.ty), (I64, I64, I64), "in {name}");
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
    assert_eq!((name.as_str(), tail.ty), ("y", Ty::String));
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
    let shape: Vec<(usize, Ty)> = fields.iter().map(|(i, e)| (*i, e.ty)).collect();
    assert_eq!(shape, vec![(1, I64), (0, I32)]);
}

#[test]
fn unknown_struct_in_literal_is_v0101() {
    let (d, sources) = one_error("fn main() {\n    let p = Nope { x: 1 };\n}\n");
    assert_eq!(d.code, codes::V0101);
    assert_eq!(d.span, span_of(&sources, "Nope"));
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
    assert_eq!(d.message, "`break` is only allowed inside a `while` loop");
    assert_eq!(d.span, span_of(&sources, "break"));

    let (d, sources) = one_error("fn main() {\n    continue;\n}\n");
    assert_eq!(d.code, codes::V0002);
    assert_eq!(
        d.message,
        "`continue` is only allowed inside a `while` loop"
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
    let tys: Vec<Ty> = call_args(call).iter().map(|a| a.ty).collect();
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
