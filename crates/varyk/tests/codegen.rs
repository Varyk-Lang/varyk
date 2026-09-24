//! Rust backend snapshot tests: the generated crate for the six
//! examples, and one focused program per row of the spec 4.3 string table
//! plus the other emission rules of spec 6.3 and 6.4.

use std::path::{Path, PathBuf};

use varyk::backend::{Backend, GeneratedCrate, RustBackend};
use varyk_syntax::{FileId, SourceFile};

// --- Helpers ----------------------------------------------------------------

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn generate_source(entry: SourceFile) -> GeneratedCrate {
    let mut sources = Vec::new();
    let program = match varyk::check_file(entry, &mut sources) {
        Ok(program) => program,
        Err(diagnostics) => panic!("expected the program to check, got {diagnostics:#?}"),
    };
    RustBackend.generate(&program, "test_pkg")
}

/// Generates the crate for `<rel>`, relative to the workspace root.
fn generate_path(rel: &str) -> GeneratedCrate {
    let path = workspace_root().join(rel);
    let text = std::fs::read_to_string(&path).expect("file should exist");
    generate_source(SourceFile::new(FileId(0), path, text))
}

/// Generates the crate for a single-file program.
fn generate_str(text: &str) -> GeneratedCrate {
    generate_source(SourceFile::new(FileId(0), "dummy/test.vr", text))
}

fn file<'a>(krate: &'a GeneratedCrate, path: &str) -> &'a str {
    krate
        .files
        .iter()
        .find(|(p, _)| p == path)
        .map(|(_, content)| content.as_str())
        .unwrap_or_else(|| panic!("no file {path} in {:?}", paths(krate)))
}

fn paths(krate: &GeneratedCrate) -> Vec<&str> {
    krate.files.iter().map(|(p, _)| p.as_str()).collect()
}

fn main_rs(text: &str) -> String {
    file(&generate_str(text), "src/main.rs").to_string()
}

// --- The six examples -------------------------------------------------------

#[test]
fn hello_main_rs() {
    let krate = generate_path("examples/hello.vr");
    assert_eq!(paths(&krate), ["src/main.rs"]);
    insta::assert_snapshot!("hello_main_rs", file(&krate, "src/main.rs"));
}

#[test]
fn hello_cargo_toml() {
    let krate = generate_path("examples/hello.vr");
    insta::assert_snapshot!("hello_cargo_toml", krate.cargo_toml);
}

#[test]
fn functions_main_rs() {
    let krate = generate_path("examples/functions.vr");
    insta::assert_snapshot!("functions_main_rs", file(&krate, "src/main.rs"));
}

#[test]
fn structs_main_rs() {
    let krate = generate_path("examples/structs.vr");
    insta::assert_snapshot!("structs_main_rs", file(&krate, "src/main.rs"));
}

#[test]
fn borrowing_main_rs_matches_spec_section_5() {
    let krate = generate_path("examples/borrowing.vr");
    let main = file(&krate, "src/main.rs");
    let spec_body = "\
struct User {
    name: String,
}

fn rename(user: &mut User) {
    user.name = \"Bob\".to_string();
}

fn print_user(user: &User) {
    println!(\"{}\", user.name);
}

fn main() {
    let mut user = User { name: \"Alice\".to_string() };
    print_user(&user);
    rename(&mut user);
    print_user(&user);
}
";
    let expected = format!(
        "#![allow(dead_code, unused_variables, unused_mut, arithmetic_overflow, unconditional_panic)]\n\n{spec_body}"
    );
    assert_eq!(main, expected);
}

#[test]
fn modules_crate_has_two_files_and_a_mod_line() {
    let krate = generate_path("examples/modules/main.vr");
    assert_eq!(paths(&krate), ["src/main.rs", "src/math.rs"]);
    let main = file(&krate, "src/main.rs");
    assert!(main.contains("\nmod math;\n"), "{main}");
    insta::assert_snapshot!("modules_main_rs", main);
    insta::assert_snapshot!("modules_math_rs", file(&krate, "src/math.rs"));
}

#[test]
fn interop_crate_copies_greet_rs_verbatim() {
    let krate = generate_path("examples/interop/main.vr");
    assert_eq!(paths(&krate), ["src/main.rs", "src/greet.rs"]);
    let source = std::fs::read_to_string(workspace_root().join("examples/interop/greet.rs"))
        .expect("greet.rs should exist");
    assert_eq!(file(&krate, "src/greet.rs"), source);
    insta::assert_snapshot!("interop_main_rs", file(&krate, "src/main.rs"));
}

// --- Spec 4.3 table rows ----------------------------------------------------

#[test]
fn literal_let_that_stays_borrowed() {
    insta::assert_snapshot!(main_rs(
        "fn main() {\n    let s = \"x\";\n    println!(\"{}\", s);\n}\n"
    ));
}

#[test]
fn literal_let_that_becomes_owned() {
    insta::assert_snapshot!(main_rs(
        "struct Label {\n    text: string,\n}\n\nfn main() {\n    let s = \"x\";\n    let label = Label { text: s };\n    println!(\"{}\", label.text);\n}\n"
    ));
}

#[test]
fn literal_into_field_field_assignment_and_return() {
    insta::assert_snapshot!(main_rs(
        "struct Label {\n    text: string,\n}\n\nfn tail() -> string {\n    \"tail\"\n}\n\nfn early(flag: bool) -> string {\n    if flag {\n        return \"early\";\n    }\n    \"late\"\n}\n\nfn relabel(mut label: Label) {\n    label.text = \"b\";\n}\n\nfn main() {\n    let mut label = Label { text: \"a\" };\n    relabel(label);\n    label.text = \"c\";\n    println!(\"{} {} {}\", label.text, tail(), early(true));\n}\n"
    ));
}

#[test]
fn owned_local_into_field_return_and_imported_string_parameter() {
    let krate = generate_path("crates/varyk/tests/fixtures/codegen/owned_string/main.vr");
    insta::assert_snapshot!(file(&krate, "src/main.rs"));
}

#[test]
fn borrowed_local_and_literal_to_a_string_parameter() {
    insta::assert_snapshot!(main_rs(
        "fn show(s: string) {\n    println!(\"{}\", s);\n}\n\nfn forward(s: string) {\n    show(s);\n}\n\nfn main() {\n    let s = \"x\";\n    show(s);\n    show(\"y\");\n    forward(s);\n}\n"
    ));
}

#[test]
fn owned_local_to_a_string_parameter() {
    insta::assert_snapshot!(main_rs(
        "fn show(s: string) {\n    println!(\"{}\", s);\n}\n\nfn make() -> string {\n    \"made\"\n}\n\nfn main() {\n    let s = make();\n    show(s);\n    show(make());\n}\n"
    ));
}

#[test]
fn field_to_a_string_parameter() {
    insta::assert_snapshot!(main_rs(
        "struct User {\n    name: string,\n}\n\nfn show(s: string) {\n    println!(\"{}\", s);\n}\n\nfn show_user(user: User) {\n    show(user.name);\n}\n\nfn main() {\n    let user = User { name: \"Ann\" };\n    show(user.name);\n    show_user(user);\n}\n"
    ));
}

#[test]
fn owned_local_to_a_mut_string_parameter() {
    insta::assert_snapshot!(main_rs(
        "fn clear(mut s: string) {\n    s = \"\";\n}\n\nfn main() {\n    let mut s = \"x\";\n    clear(s);\n    clear(\"temp\");\n    println!(\"{}\", s);\n}\n"
    ));
}

#[test]
fn field_of_a_mutable_place_to_a_mut_string_parameter() {
    insta::assert_snapshot!(main_rs(
        "struct User {\n    name: string,\n}\n\nfn clear(mut s: string) {\n    s = \"\";\n}\n\nfn clear_user(mut user: User) {\n    clear(user.name);\n}\n\nfn main() {\n    let mut user = User { name: \"Ann\" };\n    clear(user.name);\n    clear_user(user);\n}\n"
    ));
}

#[test]
fn let_from_a_field() {
    insta::assert_snapshot!(main_rs(
        "struct User {\n    name: string,\n}\n\nfn show(user: User) {\n    let n = user.name;\n    println!(\"{}\", n);\n}\n\nfn main() {\n    let user = User { name: \"Ann\" };\n    let n = user.name;\n    println!(\"{}\", n);\n    show(user);\n}\n"
    ));
}

#[test]
fn assignment_through_a_mut_string_parameter() {
    insta::assert_snapshot!(main_rs(
        "fn set(mut name: string) {\n    name = \"x\";\n}\n\nfn forward(mut name: string) {\n    set(name);\n    println!(\"{}\", name);\n}\n\nfn main() {\n    let mut name = \"a\";\n    forward(name);\n}\n"
    ));
}

#[test]
fn string_comparison_between_a_field_and_a_call_result() {
    insta::assert_snapshot!(main_rs(
        "struct User {\n    name: string,\n}\n\nfn make() -> string {\n    \"Ann\"\n}\n\nfn same(user: User, other: string) -> bool {\n    user.name == make() && other != user.name && other == \"x\"\n}\n\nfn main() {\n    let user = User { name: \"Ann\" };\n    println!(\"{}\", same(user, \"Ann\"));\n}\n"
    ));
}

#[test]
fn deref_arithmetic_on_a_mut_i32_parameter() {
    insta::assert_snapshot!(main_rs(
        "fn bump(mut x: i32) {\n    x = x + 1;\n    println!(\"{}\", x);\n}\n\nfn main() {\n    let mut n = 1;\n    bump(n);\n    println!(\"{}\", n);\n}\n"
    ));
}

#[test]
fn let_whose_branches_mix_a_field_and_a_str_local() {
    insta::assert_snapshot!(main_rs(
        "struct User {\n    name: string,\n}\n\nfn main() {\n    let user = User { name: \"Ann\" };\n    let other = \"Bob\";\n    let n = if true {\n        user.name\n    } else {\n        other\n    };\n    println!(\"{}\", n);\n}\n"
    ));
}

#[test]
fn bare_local_statement_does_not_move() {
    insta::assert_snapshot!(main_rs(
        "struct User {\n    name: string,\n}\n\nfn main() {\n    let user = User { name: \"Ann\" };\n    user;\n    println!(\"{}\", user.name);\n}\n"
    ));
}

#[test]
fn imported_modes_on_copy_and_string_parameters() {
    let krate = generate_path("crates/varyk/tests/fixtures/interop/callable/main.vr");
    insta::assert_snapshot!(file(&krate, "src/main.rs"));
}

#[test]
fn imported_mut_reference_parameter_gets_a_mut_borrow() {
    let krate = generate_path("crates/varyk/tests/fixtures/interop/mut_i32/main.vr");
    let main = file(&krate, "src/main.rs");
    assert!(main.contains("crate::ext::bump(&mut n);"), "{main}");
    insta::assert_snapshot!(main);
}

// --- Visibility, literals, precedence ---------------------------------------

#[test]
fn pub_items_stay_pub_and_others_are_private() {
    let main = main_rs(
        "pub struct Open {\n    a: i32,\n}\n\nstruct Closed {\n    b: i32,\n}\n\npub fn open() {}\n\nfn closed() {}\n\nfn main() {}\n",
    );
    assert!(
        main.contains("\npub struct Open {\n    pub a: i32,\n}\n"),
        "{main}"
    );
    assert!(
        main.contains("\nstruct Closed {\n    b: i32,\n}\n"),
        "{main}"
    );
    assert!(main.contains("\npub fn open() {\n}\n"), "{main}");
    assert!(main.contains("\nfn closed() {\n}\n"), "{main}");
}

#[test]
fn string_literal_escapes_are_emitted_raw() {
    let main = main_rs("fn main() {\n    println!(\"{}\", \"a\\tb\\n\\\"c\\\"\\\\\");\n}\n");
    assert!(
        main.contains(r#"println!("{}", "a\tb\n\"c\"\\");"#),
        "{main}"
    );
}

#[test]
fn binary_operands_are_parenthesized() {
    let main = main_rs(
        "fn f(a: i32, b: i32, c: i32) -> i32 {\n    (a + b) * c\n}\n\nfn g(a: i32, b: i32) -> i32 {\n    -(a + b)\n}\n\nfn main() {}\n",
    );
    assert!(main.contains("    (a + b) * c\n"), "{main}");
    assert!(main.contains("    -(a + b)\n"), "{main}");
}

#[test]
fn non_default_literals_get_a_type_suffix() {
    insta::assert_snapshot!(main_rs(
        "fn main() {\n    let a: i64 = 10;\n    let b: u8 = 2;\n    let c = 3;\n    let d: f32 = 1.5;\n    let e = 2.5;\n    println!(\"{} {} {} {} {}\", a, b, c, d, e);\n}\n"
    ));
}

#[test]
fn control_flow() {
    insta::assert_snapshot!(main_rs(
        "fn count(limit: i32) -> i32 {\n    let mut i = 0;\n    while i < limit {\n        i = i + 1;\n        if i == 3 {\n            continue;\n        } else if i > 5 {\n            break;\n        }\n    }\n    i\n}\n\nfn main() {\n    println!(\"{}\", count(10));\n}\n"
    ));
}

#[test]
fn cross_module_struct_and_function_paths() {
    let krate = generate_path("crates/varyk/tests/fixtures/codegen/cross_module/main.vr");
    insta::assert_snapshot!("cross_module_main_rs", file(&krate, "src/main.rs"));
    insta::assert_snapshot!("cross_module_shapes_rs", file(&krate, "src/shapes.rs"));
}

#[test]
fn if_operand_of_arithmetic_is_parenthesized() {
    insta::assert_snapshot!(main_rs(
        "fn pick(c: bool) -> i32 {\n    if c { 1 } else { 2 } + 10\n}\n\nfn main() {\n    println!(\"{}\", pick(true));\n}\n"
    ));
}

#[test]
fn if_operand_of_string_comparison_is_parenthesized() {
    insta::assert_snapshot!(main_rs(
        "fn same(c: bool, a: string) -> bool {\n    (if c { a } else { \"b\" }) == \"x\"\n}\n\nfn main() {\n    println!(\"{}\", same(true, \"x\"));\n}\n"
    ));
}

#[test]
fn if_of_new_values_is_borrowed_as_a_whole() {
    insta::assert_snapshot!(main_rs(
        "fn make() -> string {\n    \"m\"\n}\n\nfn show(s: string) {\n    println!(\"{}\", s);\n}\n\nfn clear(mut s: string) {\n    s = \"\";\n}\n\nfn f(c: bool, name: string) -> bool {\n    show(if c { make() } else { \"lit\" });\n    clear(if c { make() } else { make() });\n    println!(\"{}\", if c { make() } else { \"y\" });\n    (if c { make() } else { \"y\" }) == name\n}\n\nfn main() {\n    println!(\"{}\", f(true, \"m\"));\n}\n"
    ));
}

#[test]
fn if_of_literals_to_a_mut_string_parameter_is_borrowed_as_a_whole() {
    insta::assert_snapshot!(main_rs(
        "fn show(s: string) {\n    println!(\"{}\", s);\n}\n\nfn clear(mut s: string) {\n    s = \"\";\n}\n\nfn main() {\n    let c = true;\n    show(if c { \"a\" } else { \"b\" });\n    clear(if c { \"a\" } else { \"b\" });\n}\n"
    ));
}

#[test]
fn discarded_if_of_new_struct_values_has_no_borrow() {
    insta::assert_snapshot!(main_rs(
        "struct Point {\n    x: i32,\n}\n\nfn make() -> Point {\n    Point { x: 1 }\n}\n\nfn main() {\n    let c = true;\n    if c { make() } else { make() };\n}\n"
    ));
}

#[test]
fn field_of_a_block_with_a_place_leaf_borrows_and_leaves_the_local_usable() {
    insta::assert_snapshot!(main_rs(
        "struct Point {\n    x: i32,\n}\n\nfn main() {\n    let p = Point { x: 1 };\n    let x = { p }.x;\n    println!(\"{} {}\", x, p.x);\n}\n"
    ));
}

#[test]
fn field_of_a_block_with_only_temporary_leaves_borrows_the_whole_value() {
    insta::assert_snapshot!(main_rs(
        "struct Point {\n    x: i32,\n}\n\nfn make() -> Point {\n    Point { x: 1 }\n}\n\nfn main() {\n    let x = { make() }.x;\n    println!(\"{}\", x);\n}\n"
    ));
}

#[test]
fn new_values_inside_blocks_are_borrowed_as_a_whole() {
    insta::assert_snapshot!(main_rs(
        "struct User {\n    name: string,\n}\n\nfn make() -> User {\n    User { name: \"Ann\" }\n}\n\nfn text() -> string {\n    \"t\"\n}\n\nfn show(s: string) {\n    println!(\"{}\", s);\n}\n\nfn show_user(user: User) {\n    println!(\"{}\", user.name);\n}\n\nfn main() {\n    let c = true;\n    let s = text();\n    show(if c { text() } else { make().name });\n    show({ let a = make(); a.name });\n    show({ let t = s; t });\n    show_user({ let u = make(); u });\n    println!(\"{}\", if c { make().name } else { \"lit\" });\n}\n"
    ));
}

#[test]
fn field_of_an_if_reborrows_and_is_reached_mutably_when_lent_mutably() {
    insta::assert_snapshot!(main_rs(
        "struct User {\n    name: string,\n}\n\nfn clear(mut s: string) {\n    s = \"\";\n}\n\nfn show(user: User) {\n    println!(\"{}\", user.name);\n}\n\nfn f(c: bool, mut user: User) {\n    let mut other = User { name: \"Bob\" };\n    println!(\"{}\", (if c { other } else { user }).name);\n    clear((if c { other } else { user }).name);\n    show(user);\n}\n\nfn main() {\n    f(true, User { name: \"Ann\" });\n}\n"
    ));
}

#[test]
fn changeable_binding_of_text_is_a_mut_string() {
    insta::assert_snapshot!(main_rs(
        "struct User {\n    name: string,\n}\n\nfn clear(mut s: string) {\n    s = \"\";\n}\n\nfn main() {\n    let c = false;\n    let mut user = User { name: \"Ann\" };\n    let mut n = if c { \"x\" } else { user.name };\n    clear(n);\n    n = \"Bob\";\n    println!(\"{}\", user.name);\n}\n"
    ));
}

#[test]
fn copy_if_lent_to_a_reference_parameter_is_copied_as_a_whole() {
    let krate = generate_path("crates/varyk/tests/fixtures/codegen/copy_to_reference/main.vr");
    insta::assert_snapshot!(file(&krate, "src/main.rs"));
}
