use std::path::Path;

use varyk_syntax::{FileId, SourceFile, Span};

use super::typecheck;
use crate::diagnostics::{Diagnostic, codes};
use crate::hir::{
    HirExpr, HirExprKind, HirForHead, HirFunction, HirProgram, HirStmt, MethodRef, TryKind,
    VariantRef,
};
use crate::resolve::{Callee, resolve};
use crate::types::{BUILTIN_TYPE_NAMES, Derives, FloatKind, IntKind, ParamMode, Ty};

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
            _ => name.to_string(),
        };
        ok(&format!("fn f(x: {written}) {{}}\n\nfn main() {{}}\n"));
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
fn matching_a_struct_is_v0205() {
    let (d, sources) = one_error(&with_match(
        "",
        "    let t = mk();\n    match t {\n        _ => {}\n    }",
    ));
    assert_eq!(d.code, codes::V0205, "{d:#?}");
    let head = part_of(&sources, "match t {", "t {");
    assert_eq!(d.span, Span::new(head.file, head.start, head.start + 1));
    assert!(d.message.contains("`Task`"), "{d:#?}");
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
                "`a` exists in the Rust file but is `async`; Varyk does not import such \
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

const PATTERN_ITEMS: &str = "enum Shape {\n    Circle(f64),\n    Rect(f64, f64),\n    Point,\n}\nenum Event {\n    Click { x: i32, y: i32 },\n    Key(string),\n    Quit,\n}\nenum Pair {\n    Two(i32, Option<i32>),\n}\nstruct Task {\n    done: bool,\n}\n";

/// A program with [`PATTERN_ITEMS`] and `body` as the body of `f`, whose
/// parameters cover every kind of value a pattern looks at.
fn with_patterns(body: &str) -> String {
    format!(
        "{PATTERN_ITEMS}fn f(o: Option<Shape>, n: i32, b: bool, s: string, u: u8, e: Event, r: Result<Option<i32>, string>, t: Option<Task>, p: Pair, fl: f64, so: Option<string>) {{\n{body}\n}}\nfn main() {{}}\n"
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
        "    match t {\n        Some(Task { done }) => {}\n        _ => {}\n    }",
    ));
    assert_eq!(d.code, codes::V0001, "{d:#?}");
    assert_eq!(d.span, span_of(&sources, "Task { done }"));
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
    let j: Option<i32> = s.parse();
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
        ("j", opt(I32)),
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
            &["`sort`", "Vec<f64>", "an integer type, `bool`, or `string`"][..],
        ),
        (
            "let mut v = vec![P { n: 1 }];\n    v.sort();",
            "sort",
            &["`sort`", "Vec<P>"][..],
        ),
        (
            "let v = vec![P { n: 1 }];\n    let b = v.contains(P { n: 1 });",
            "contains",
            &["`contains`", "Vec<P>", "a number type, `bool`, or `string`"][..],
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
fn parse_takes_its_type_from_where_it_goes() {
    let program = ok("fn first(text: string) -> Option<i32> {
    let n: i32 = text.parse()?;
    Some(n)
}
fn flag(text: string) -> Option<bool> {
    text.parse()
}
fn take(n: Option<u8>) {}
fn main() {
    let a: Option<f64> = \"1.5\".parse();
    take(\"7\".parse());
}
");
    let main = function(&program, "main");
    assert_eq!(local_ty(main, "a"), opt(Ty::Float(FloatKind::F64)));
    assert_eq!(
        call_args(stmt_expr(main, 1))[0].ty,
        opt(Ty::Int(IntKind::U8))
    );
    let first = function(&program, "first");
    assert_eq!(local_ty(first, "n"), I32);
}

#[test]
fn parse_into_a_string_is_v0200_and_with_nothing_expected_v0207() {
    let (d, sources) =
        one_error("fn main() {\n    let s = \"a\";\n    let t: Option<string> = s.parse();\n}\n");
    assert_eq!(d.code, codes::V0200, "{d:#?}");
    assert_eq!(d.span, span_of(&sources, "s.parse()"));
    assert!(d.message.contains("`string`"), "{d:#?}");
    assert!(d.message.contains("a number or `bool`"), "{d:#?}");

    let (d, sources) = one_error("fn main() {\n    let s = \"a\";\n    let n = s.parse();\n}\n");
    assert_eq!(d.code, codes::V0207, "{d:#?}");
    assert_eq!(d.span, span_of(&sources, "s.parse()"));
    assert!(
        d.message
            .contains("`let parsed: Option<i32> = text.parse();` then `parsed.ok_or(e)?`"),
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
fn parse_under_question_in_a_result_function_is_v0206_showing_two_statements() {
    let (d, sources) = one_error(
        "fn f(text: string) -> Result<i32, string> {\n    let n: i32 = text.parse()?;\n    Ok(n)\n}\nfn main() {}\n",
    );
    assert_eq!(d.code, codes::V0206, "{d:#?}");
    assert_eq!(d.span, span_of(&sources, "text.parse()"));
    assert!(d.message.contains("`Option<_>`"), "{d:#?}");
    assert!(d.message.contains("`Result<i32, string>`"), "{d:#?}");
    assert!(
        d.notes.iter().any(
            |n| n.contains("`let parsed: Option<i32> = text.parse();` then `parsed.ok_or(e)?`")
        ),
        "{d:#?}"
    );
    // A `Result` expected anywhere else is a plain mismatch.
    let (d, _) = one_error(
        "fn main() {\n    let s = \"1\";\n    let r: Result<i32, string> = s.parse();\n}\n",
    );
    assert_eq!(d.code, codes::V0200, "{d:#?}");
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
            .any(|n| n.contains("parsed.ok_or(e)?")),
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
