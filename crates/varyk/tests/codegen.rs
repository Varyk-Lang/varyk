//! Rust backend snapshot tests: the generated crate for fourteen of the
//! examples, and one focused program per row of the spec 4.3 string table
//! plus the other emission rules of spec 6.3 and 6.4.

use std::path::{Path, PathBuf};

use varyk::backend::{Backend, CrateInfo, GeneratedCrate, RustBackend};
use varyk_syntax::{FileId, SourceFile};

// --- Helpers ----------------------------------------------------------------

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn generate_source(entry: SourceFile) -> GeneratedCrate {
    let mut sources = Vec::new();
    let program = match varyk::check_file(entry, varyk::package::Kind::Binary, None, &mut sources) {
        Ok(program) => program,
        Err(diagnostics) => panic!("expected the program to check, got {diagnostics:#?}"),
    };
    RustBackend.generate(&program, &CrateInfo::single_file("test_pkg".to_string()))
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
        .find(|file| file.path == path)
        .map(|file| file.text.as_str())
        .unwrap_or_else(|| panic!("no file {path} in {:?}", paths(krate)))
}

fn paths(krate: &GeneratedCrate) -> Vec<&str> {
    krate.files.iter().map(|file| file.path.as_str()).collect()
}

fn copied_paths(krate: &GeneratedCrate) -> Vec<&str> {
    krate.copied.iter().map(|(path, _)| path.as_str()).collect()
}

/// The item-level attribute every generated item carries (spec 2.3), so a
/// test can strip it out and compare the rest to a spec body that
/// predates task 2's per-item attributes.
const ALLOW_ITEM: &str = "#[allow(warnings, arithmetic_overflow, unconditional_panic)]";

fn without_allow_lines(text: &str) -> String {
    text.lines()
        .filter(|line| *line != ALLOW_ITEM)
        .map(|line| format!("{line}\n"))
        .collect::<String>()
        // The specs show std macros bare; the backend writes their full
        // path, so that no `#[macro_export]` macro of a `.rs` module can
        // stand in for them.
        .replace("::std::", "")
}

fn main_rs(text: &str) -> String {
    file(&generate_str(text), "src/main.rs").to_string()
}

// --- The six milestone-1 examples ------------------------------------------

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
    // M4 spec 2.10 adds the one `#[derive(..)]` line to the spec's body.
    let spec_body = "\
#[derive(Clone, PartialEq)]
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
    // Every generated item now carries its own attribute line (spec 2.3)
    // instead of one crate-wide header; strip those out to compare
    // against the spec's plain body.
    assert_eq!(without_allow_lines(main), spec_body);
}

#[test]
fn modules_crate_has_two_files_and_a_mod_line() {
    let krate = generate_path("examples/modules/main.vr");
    assert_eq!(paths(&krate), ["src/main.rs", "src/math.rs"]);
    let main = file(&krate, "src/main.rs");
    assert!(main.starts_with("mod math;\n"), "{main}");
    insta::assert_snapshot!("modules_main_rs", main);
    insta::assert_snapshot!("modules_math_rs", file(&krate, "src/math.rs"));
}

#[test]
fn interop_crate_copies_greet_rs_verbatim() {
    let krate = generate_path("examples/interop/main.vr");
    assert_eq!(paths(&krate), ["src/main.rs"]);
    assert_eq!(copied_paths(&krate), ["src/greet.rs"]);
    let source = std::fs::read_to_string(workspace_root().join("examples/interop/greet.rs"))
        .expect("greet.rs should exist");
    let copied = std::fs::read_to_string(&krate.copied[0].1)
        .expect("the copied entry's path should point at a readable file");
    assert_eq!(copied, source);
    insta::assert_snapshot!("interop_main_rs", file(&krate, "src/main.rs"));
}

/// An imported struct has no definition in the generated crate; its
/// associated function is reached through it, and its methods' receivers
/// are borrowed as `&self` and `&mut self` say (M3 spec 4.1, 4.2).
#[test]
fn imported_struct_emits_no_definition_and_borrows_by_receiver() {
    let krate = generate_path("crates/varyk/tests/fixtures/interop/matcher/main.vr");
    assert_eq!(paths(&krate), ["src/main.rs"]);
    assert_eq!(copied_paths(&krate), ["src/matcher.rs"]);
    let main = file(&krate, "src/main.rs");
    assert!(!main.contains("struct Matcher"), "{main}");
    insta::assert_snapshot!("imported_struct_main_rs", main);
}

/// An imported enum has no definition in the generated crate; a `match`
/// on it uses its variants' full path from the crate root (M3 spec 4.3).
#[test]
fn imported_enum_emits_no_definition_and_matches_by_variant_path() {
    let krate = generate_path("crates/varyk/tests/fixtures/interop/kind/main.vr");
    assert_eq!(paths(&krate), ["src/main.rs"]);
    assert_eq!(copied_paths(&krate), ["src/kind.rs"]);
    let main = file(&krate, "src/main.rs");
    assert!(!main.contains("enum Kind"), "{main}");
    assert!(main.contains("match k {"), "{main}");
    assert!(main.contains("kind::Kind::Word(n)"), "{main}");
    assert!(main.contains("kind::Kind::Number(n)"), "{main}");
    assert!(main.contains("kind::Kind::Unit"), "{main}");
    insta::assert_snapshot!("imported_enum_main_rs", main);
}

// --- The six milestone-2 examples ------------------------------------------

#[test]
fn enums_main_rs() {
    let krate = generate_path("examples/enums.vr");
    insta::assert_snapshot!("enums_main_rs", file(&krate, "src/main.rs"));
}

#[test]
fn methods_example_main_rs() {
    let krate = generate_path("examples/methods.vr");
    insta::assert_snapshot!("methods_example_main_rs", file(&krate, "src/main.rs"));
}

#[test]
fn collections_main_rs() {
    let krate = generate_path("examples/collections.vr");
    insta::assert_snapshot!("collections_main_rs", file(&krate, "src/main.rs"));
}

#[test]
fn errors_main_rs() {
    let krate = generate_path("examples/errors.vr");
    insta::assert_snapshot!("errors_main_rs", file(&krate, "src/main.rs"));
}

#[test]
fn strings_main_rs_matches_milestone_2_spec_section_4() {
    let krate = generate_path("examples/strings.vr");
    let main = file(&krate, "src/main.rs");
    // M4 spec 2.10 adds the one `#[derive(..)]` line to the spec's body.
    let spec_body = "\
#[derive(Clone, PartialEq)]
struct User {
    name: String,
}

fn name_of(user: &User) -> String {
    user.name.clone()
}

fn main() {
    let user = User { name: \"Alice\".to_string() };
    let name = name_of(&user);
    println!(\"{}\", name);
    let copy = name.clone();
    println!(\"{}\", copy);
    println!(\"{}\", format!(\"Hello, {}!\", name));
    println!(\"{}\", name.len());
    println!(\"{}\", name == \"Alice\");
}
";
    // Every generated item now carries its own attribute line (spec 2.3)
    // instead of one crate-wide header; strip those out to compare
    // against the spec's plain body.
    assert_eq!(without_allow_lines(main), spec_body);
    insta::assert_snapshot!("strings_main_rs", main);
}

#[test]
fn todo_crate_has_two_files_and_a_mod_line() {
    let krate = generate_path("examples/todo/main.vr");
    assert_eq!(paths(&krate), ["src/main.rs", "src/task.rs"]);
    let main = file(&krate, "src/main.rs");
    assert!(main.starts_with("mod task;\n"), "{main}");
    insta::assert_snapshot!("todo_main_rs", main);
    insta::assert_snapshot!("todo_task_rs", file(&krate, "src/task.rs"));
}

/// A three-level tree (spec 3.1): `shop/mod.vr` declares `pub mod cart`
/// and a private `.rs` module; `cart.vr` declares `pub mod item`. Every
/// generated file lands at its mirrored path, no `mod` line carries the
/// allow attribute (it would reach the `.rs` modules below), and
/// cross-module references are full `crate::` paths.
#[test]
fn nested_tree_mirrors_the_modules() {
    let krate = generate_path("crates/varyk/tests/fixtures/codegen/tree/main.vr");
    assert_eq!(
        paths(&krate),
        [
            "src/main.rs",
            "src/shop/mod.rs",
            "src/shop/cart.rs",
            "src/shop/cart/item.rs"
        ]
    );
    assert_eq!(copied_paths(&krate), ["src/shop/util.rs"]);
    let shop = file(&krate, "src/shop/mod.rs");
    assert!(shop.starts_with("pub mod cart;\nmod util;\n"), "{shop}");
    insta::assert_snapshot!("tree_main_rs", file(&krate, "src/main.rs"));
    insta::assert_snapshot!("tree_shop_mod_rs", shop);
    insta::assert_snapshot!("tree_shop_cart_rs", file(&krate, "src/shop/cart.rs"));
    insta::assert_snapshot!(
        "tree_shop_cart_item_rs",
        file(&krate, "src/shop/cart/item.rs")
    );
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
fn imported_borrowed_returns_call_as_written() {
    let krate = generate_path("crates/varyk/tests/fixtures/interop/first_word/main.vr");
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
    // A field's own `pub` (spec 3.4), not its struct's, decides whether
    // it is emitted `pub`: `Open` is a `pub` struct with one `pub` field
    // and one private field, and both keep their own flag.
    let main = main_rs(
        "pub struct Open {\n    a: i32,\n    pub b: i32,\n}\n\nstruct Closed {\n    c: i32,\n}\n\npub fn open() {}\n\nfn closed() {}\n\nfn main() {}\n",
    );
    assert!(
        main.contains("\npub struct Open {\n    a: i32,\n    pub b: i32,\n}\n"),
        "{main}"
    );
    assert!(
        main.contains("\nstruct Closed {\n    c: i32,\n}\n"),
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

/// The four `use` forms (spec 3.3) each emit as a canonical `crate::`
/// path, with `ALLOW_ITEM`: a type reached with the `crate::` keyword, a
/// module alias, a `.rs` function reached with `crate::`, and an aliased
/// type (`as`).
#[test]
fn use_forms_emit_canonical_crate_paths() {
    let krate = generate_path("crates/varyk/tests/fixtures/codegen/use_forms/main.vr");
    insta::assert_snapshot!("use_forms_main_rs", file(&krate, "src/main.rs"));
}

/// Two `use` declarations of one module sharing a local name in different
/// namespaces (spec 2.1: `use crate::a::Foo;` a struct, `use
/// crate::b::Foo;` a function) must each emit their own canonical path,
/// not the same one twice (which rustc would reject, E0252).
#[test]
fn use_declarations_sharing_a_local_name_across_namespaces_emit_their_own_paths() {
    let krate = generate_path("crates/varyk/tests/fixtures/codegen/use_namespaces/main.vr");
    let main_rs = file(&krate, "src/main.rs");
    assert!(
        main_rs.contains("use crate::a::Foo;"),
        "missing the struct's own line:\n{main_rs}"
    );
    assert!(
        main_rs.contains("use crate::b::Foo;"),
        "missing the function's own line:\n{main_rs}"
    );
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

// --- Milestone 2: enums, impl blocks, and generic types -----------------------

#[test]
fn enum_declaration_and_impl_blocks_with_self_receivers() {
    insta::assert_snapshot!(main_rs(
        "enum Shape {\n    Circle(f64),\n    Named(string, Option<string>),\n    Point,\n}\n\nstruct Counter {\n    count: i32,\n}\n\nimpl Counter {\n    fn new() -> Counter {\n        Counter { count: 0 }\n    }\n\n    fn add(mut self, by: i32) {\n        self.count = self.count + by;\n    }\n}\n\nimpl Shape {\n    fn sides(self) -> usize {\n        0\n    }\n}\n\nimpl Counter {\n    pub fn value(self) -> i32 {\n        self.count\n    }\n}\n\nfn tally(v: Vec<usize>, r: Result<Shape, string>, mut c: Counter) -> Option<Vec<string>> {\n    c.count = 1;\n    tally(v, r, c)\n}\n\nfn main() {\n    let mut counter = Counter { count: 1 };\n    counter.count = 2;\n    println!(\"{}\", counter.count);\n}\n"
    ));
}

#[test]
fn values_and_constructors() {
    let krate = generate_path("crates/varyk/tests/fixtures/codegen/values/main.vr");
    let main = file(&krate, "src/main.rs");
    assert!(main.contains("crate::geo::Shape::Point"), "{main}");
    assert!(main.contains("crate::geo::Shape::Circle(0.5)"), "{main}");
    assert!(
        main.contains("crate::geo::User { name: \"Ann\".to_string() }"),
        "{main}"
    );
    // A type hole filled by the `let`'s written type keeps it in Rust.
    assert!(
        main.contains("let empty: Vec<i32> = ::std::vec![];"),
        "{main}"
    );
    assert!(main.contains("let maybe: Option<i32> = None;"), "{main}");
    insta::assert_snapshot!("values_main_rs", main);
}

#[test]
fn methods_associated_functions_the_built_in_table_and_indexing() {
    let krate = generate_path("crates/varyk/tests/fixtures/codegen/methods/main.vr");
    let main = file(&krate, "src/main.rs");
    assert!(main.contains("crate::tally::Tally::new()"), "{main}");
    assert!(
        main.contains("let mut tasks: Vec<Task> = Vec::new();"),
        "{main}"
    );
    assert!(main.contains("Task::new(\"write\")"), "{main}");
    assert!(
        main.contains("Task::complete(&mut tasks[0usize]);"),
        "{main}"
    );
    assert!(main.contains("Task::is_done(&tasks[i])"), "{main}");
    assert!(main.contains("let first = &tasks[0usize];"), "{main}");
    // `clone` on a `&str` is spelled `.to_string()` (spec 3.5).
    assert!(main.contains("lit.to_string()"), "{main}");
    assert!(main.contains("self.label.clone()"), "{main}");
    insta::assert_snapshot!("methods_main_rs", main);
}

#[test]
fn match_on_places_and_temporaries() {
    let krate = generate_path("crates/varyk/tests/fixtures/codegen/matching/main.vr");
    let main = file(&krate, "src/main.rs");
    // A place is matched through a shared reference of the right shape.
    assert!(main.contains("match &self.status {"), "{main}");
    assert!(main.contains("match status {"), "{main}");
    assert!(main.contains("match &*status {"), "{main}");
    assert!(main.contains("match &tasks[0usize].status {"), "{main}");
    assert!(main.contains("match &maybe {"), "{main}");
    assert!(main.contains("match &picked {"), "{main}");
    // A temporary is matched as it is.
    assert!(main.contains("match parse(\"one\") {"), "{main}");
    // A Copy value bound through a reference is copied first.
    assert!(main.contains("let n = *n;"), "{main}");
    assert!(main.contains("let k = *k;"), "{main}");
    assert!(main.contains("crate::geo::Shape::Circle(r) => {"), "{main}");
    insta::assert_snapshot!("matching_main_rs", main);
}

#[test]
fn for_over_ranges_places_and_temporaries() {
    let krate = generate_path("crates/varyk/tests/fixtures/codegen/loops/main.vr");
    let main = file(&krate, "src/main.rs");
    assert!(main.contains("for i in 0usize..tasks.len() {"), "{main}");
    assert!(main.contains("for i in 0..3 {"), "{main}");
    // A place is looped over through a shared reference of the right shape.
    assert!(main.contains("for t in &self.tasks {"), "{main}");
    assert!(main.contains("for t in tasks {"), "{main}");
    assert!(main.contains("for t in &*tasks {"), "{main}");
    assert!(main.contains("for n in &numbers {"), "{main}");
    assert!(main.contains("for name in &names {"), "{main}");
    // A temporary is looped over as it is.
    assert!(main.contains("for word in words() {"), "{main}");
    assert!(main.contains("for t in make_tasks() {"), "{main}");
    // A Copy element reached through a reference is copied first.
    assert!(main.contains("let n = *n;"), "{main}");
    assert!(main.contains("let value = *value;"), "{main}");
    insta::assert_snapshot!("loops_main_rs", main);
}

#[test]
fn question_emits_as_written() {
    let krate = generate_path("crates/varyk/tests/fixtures/codegen/question/main.vr");
    let main = file(&krate, "src/main.rs");
    assert!(main.contains("let name = name_of(id)?;"), "{main}");
    assert!(main.contains("digit(text)?;"), "{main}");
    assert!(main.contains("total = total + digit(word)?;"), "{main}");
    // An owned local is moved into `?`.
    assert!(main.contains("let user = found?;"), "{main}");
    insta::assert_snapshot!("question_main_rs", main);
}

#[test]
fn nested_literal_and_string_patterns_if_let_and_while_let() {
    let krate = generate_path("crates/varyk/tests/fixtures/codegen/patterns/main.vr");
    let main = file(&krate, "src/main.rs");
    // A named-field variant, declared, made, and matched at depth on a
    // place, its Copy bindings copied at any depth.
    assert!(main.contains("Click { x: i32, y: i32 },"), "{main}");
    assert!(main.contains("Event::Click { y: 2, x: 0 }"), "{main}");
    assert!(main.contains("match &events[0usize] {"), "{main}");
    assert!(
        main.contains("Some(Event::Click { x: 0, y: y }) => {"),
        "{main}"
    );
    assert!(main.contains("let y = *y;"), "{main}");
    // Literal and range patterns as their values.
    assert!(main.contains("90..=100 =>"), "{main}");
    assert!(main.contains("-5..=-1 =>"), "{main}");
    // A string head is exactly a `&str` (review focus 3).
    assert!(main.contains("match owned.as_str() {"), "{main}");
    assert!(main.contains("other => other.to_string(),"), "{main}");
    // `if let`, `else if let`, and `while let`.
    assert!(
        main.contains("if let Some(Shape::Circle(r)) = shape {"),
        "{main}"
    );
    assert!(main.contains("} else if let None = shape {"), "{main}");
    assert!(
        main.contains("while let Some(top) = stack.pop() {"),
        "{main}"
    );
    insta::assert_snapshot!("patterns_main_rs", main);
}

#[test]
fn casts_inclusive_ranges_and_question_on_constructors() {
    let krate = generate_path("crates/varyk/tests/fixtures/codegen/expressions/main.vr");
    let main = file(&krate, "src/main.rs");
    // A cast is parenthesized, so a following `<` is not read as generics.
    assert!(main.contains("(a as i64) < big"), "{main}");
    assert!(main.contains("((a + 1i32) as i64)"), "{main}");
    assert!(main.contains("(a as i32) * 2"), "{main}");
    // A literal operand always carries its type suffix.
    assert!(main.contains("(-1i32 as u8)"), "{main}");
    // Under a cast every literal is suffixed, through blocks and `-`.
    assert!(main.contains("300i32"), "{main}");
    assert!(main.contains("(--1i32 as u8)"), "{main}");
    assert!(main.contains("\n        1i32\n"), "{main}");
    assert!(main.contains("for i in 1..=3 {"), "{main}");
    assert!(main.contains("Ok::<i32, String>(x)?"), "{main}");
    assert!(main.contains("let n = found?;"), "{main}");
    insta::assert_snapshot!("expressions_main_rs", main);
}

#[test]
fn table_rows_hash_maps_and_parse() {
    let krate = generate_path("crates/varyk/tests/fixtures/codegen/tables/main.vr");
    let main = file(&krate, "src/main.rs");
    // `HashMap` is written in full, in a type and in `HashMap::new()`, and
    // a `let` of one writes its type out.
    assert!(
        main.contains(
            "let mut counts: ::std::collections::HashMap<String, i32> = ::std::collections::HashMap::new();"
        ),
        "{main}"
    );
    assert!(main.contains("counts.contains_key(\"apple\")"), "{main}");
    // `parse` in each expected-type position.
    assert!(main.contains("text.parse::<i32>().ok()?"), "{main}");
    assert!(main.contains("text.parse::<bool>().ok()"), "{main}");
    assert!(main.contains("\"1.5\".parse::<f64>().ok()"), "{main}");
    assert!(main.contains("\"7\".parse::<u8>().ok()"), "{main}");
    // `contains` on a `Vec<string>` evaluates both sides before its own
    // names are in scope, so the user's `e` is not shadowed.
    assert!(
        main.contains(
            "(match (&names, e) { (haystack, needle) => haystack.iter().any(|e| e == needle) })"
        ),
        "{main}"
    );
    // A read `T` argument is lent; a `usize` index is passed by value.
    assert!(main.contains("numbers.contains(&2)"), "{main}");
    assert!(main.contains("numbers.insert(0usize, 9);"), "{main}");
    insta::assert_snapshot!("tables_main_rs", main);
}

#[test]
fn looked_into_and_taking_rows() {
    let krate = generate_path("crates/varyk/tests/fixtures/codegen/lookups/main.vr");
    let main = file(&krate, "src/main.rs");
    // A Copy payload is copied out of the map.
    assert!(
        main.contains("counts.get(word.as_str()).copied().unwrap_or(0)"),
        "{main}"
    );
    // A looked-into head is written as it is, already a reference.
    assert!(
        main.contains("if let Some(line) = lines.get(1usize) {"),
        "{main}"
    );
    // A Copy binding at depth is copied out of the reference.
    assert!(main.contains("match shapes.get(0usize) {"), "{main}");
    assert!(main.contains("let r = *r;"), "{main}");
    // A stored `Option<i32>` is copied out by `unwrap_or`.
    assert!(main.contains("o.unwrap_or(0)"), "{main}");
    insta::assert_snapshot!("lookups_main_rs", main);
}

/// The generated Rust of `examples/getters.vr` is the listing of M4 spec
/// 4, read from the spec itself, but for the lint attributes and the
/// `::std::` paths the listing leaves out.
#[test]
fn getters_main_rs_matches_the_spec_listing() {
    let spec = std::fs::read_to_string(
        workspace_root().join("docs/specs/2026-09-29-milestone-4-design.md"),
    )
    .expect("read the milestone-4 spec");
    let marker = "```rust\n#[derive(Clone, PartialEq)]\nstruct User {";
    let start = spec.find(marker).expect("the spec has the getters listing") + "```rust\n".len();
    let end = start + spec[start..].find("```").expect("a closing fence");
    let krate = generate_path("examples/getters.vr");
    assert_eq!(
        without_allow_lines(file(&krate, "src/main.rs")),
        &spec[start..end]
    );
}

/// The `readings.vr` listing of M4 spec 4: `Unit` and `Reading` derive
/// both traits, an enum is compared in place, and a borrowed struct is
/// cloned.
#[test]
fn readings_main_rs() {
    let krate = generate_path("examples/readings.vr");
    let main = file(&krate, "src/main.rs");
    assert!(
        main.contains(&format!(
            "{ALLOW_ITEM}\n#[derive(Clone, PartialEq)]\nenum Unit {{"
        )),
        "{main}"
    );
    assert!(
        main.contains(&format!(
            "{ALLOW_ITEM}\n#[derive(Clone, PartialEq)]\nstruct Reading {{"
        )),
        "{main}"
    );
    assert!(
        main.contains("if reading.unit == Unit::Fahrenheit {"),
        "{main}"
    );
    assert!(main.contains("reading.clone()"), "{main}");
    insta::assert_snapshot!("readings_main_rs", main);
}

/// A struct holding a Rust type that derives `Clone` only derives
/// `Clone`; one holding a Rust type that derives neither has no
/// `#[derive]` line.
#[test]
fn derives_follow_the_rust_fields() {
    let krate = generate_path("crates/varyk/tests/fixtures/codegen/clone_only/main.vr");
    let main = file(&krate, "src/main.rs");
    assert!(
        main.contains(&format!(
            "{ALLOW_ITEM}\n#[derive(Clone)]\nstruct Session {{"
        )),
        "{main}"
    );
    assert!(
        main.contains(&format!("{ALLOW_ITEM}\nstruct Server {{")),
        "{main}"
    );
    assert!(main.contains("let copy = session.clone();"), "{main}");
    insta::assert_snapshot!("clone_only_main_rs", main);
}

/// `==` on structs and containers: both operands at one reference depth
/// when either is a reference, else compared as they are (M4 spec 5).
#[test]
fn equality_brings_both_operands_to_one_reference_depth() {
    let main = main_rs(
        "struct User {\n    name: string,\n}\n\nfn same(u: User, param: User, v: Vec<User>) -> bool {\n    let own = User { name: \"a\" };\n    let list: Vec<User> = Vec::new();\n    own == param && param == own && list == v && own == User { name: \"b\" } && u == param\n}\n\nfn main() {\n    let a = User { name: \"a\" };\n    println!(\"{}\", same(a, a, Vec::new()));\n}\n",
    );
    assert!(
        main.contains("&own == param && param == &own && &list == v"),
        "{main}"
    );
    assert!(
        main.contains("own == User { name: \"b\".to_string() }"),
        "{main}"
    );
    assert!(main.contains("&& u == param"), "{main}");
}

#[test]
fn borrowed_returns_with_elision_and_one_written_lifetime() {
    // The `getters.vr` listing of M4 spec 4.
    let krate = generate_path("examples/getters.vr");
    let main = file(&krate, "src/main.rs");
    // Rooted at `self`, a string parameter, and a `Vec` parameter: elided.
    assert!(main.contains("fn display_name(&self) -> &str {"), "{main}");
    assert!(main.contains("fn trimmed(text: &str) -> &str {"), "{main}");
    assert!(
        main.contains("fn first(users: &Vec<User>) -> &User {"),
        "{main}"
    );
    // Rooted at one of two reference parameters: one lifetime.
    assert!(
        main.contains("fn name_unless<'a>(user: &'a User, hidden: &str) -> &'a str {"),
        "{main}"
    );
    // Each return is a reference to its part; a borrowed result is passed on
    // as it is.
    assert!(main.contains("&self.name"), "{main}");
    assert!(main.contains("text.trim()"), "{main}");
    assert!(main.contains("&users[0usize]"), "{main}");
    assert!(main.contains("let leader = first(&users);"), "{main}");
    assert!(main.contains("name_unless(leader, \"Alice\")"), "{main}");
    insta::assert_snapshot!("getters_main_rs", main);
}

#[test]
fn trim_returned_through_a_let() {
    let main = main_rs(
        "fn clean(text: string) -> string {\n    let t = text.trim();\n    t\n}\n\nfn main() {\n    let s = clean(\"  a  \");\n    println!(\"[{}]\", s);\n}\n",
    );
    assert!(main.contains("fn clean(text: &str) -> &str {"), "{main}");
    assert!(main.contains("let t = text.trim();"), "{main}");
    assert!(main.contains("let s = clean(\"  a  \");"), "{main}");
    insta::assert_snapshot!("trim_through_let_main_rs", main);
}

#[test]
fn option_map_writes_its_closure_as_written() {
    let main = main_rs(
        "fn main() {\n    let doubled = Some(4).map(|n| n * 2).is_some();\n    println!(\"{}\", doubled);\n}\n",
    );
    assert!(
        main.contains("let doubled = (Some(4)).map(|n| n * 2).is_some();"),
        "{main}"
    );
    insta::assert_snapshot!("option_map_closure_main_rs", main);
}

#[test]
fn a_map_err_closure_borrows_the_parameter_it_captures() {
    let main = main_rs(
        "fn parse_age(text: string) -> Result<i32, string> {\n    if text == \"17\" {\n        Ok(17)\n    } else {\n        Err(\"not a number\")\n    }\n}\n\nfn age_of(text: string, field: string) -> Result<i32, string> {\n    parse_age(text).map_err(|e| format!(\"{}: {}\", field, e))\n}\n\nfn width(text: string) -> usize {\n    text.len()\n}\n\nfn main() {\n    let label = \"age\".to_uppercase();\n    let checked = parse_age(\"x\").map_err(|e| {\n        let shown = label;\n        format!(\"{} {} {}\", shown, e, width(label))\n    });\n    println!(\"{}\", age_of(\"17\", label).is_ok());\n}\n",
    );
    assert!(
        main.contains("parse_age(text).map_err(|e| ::std::format!(\"{}: {}\", field, e))"),
        "{main}"
    );
    assert!(main.contains("let shown = &label;"), "{main}");
    assert!(main.contains("width(&label)"), "{main}");
    assert!(main.contains("age_of(\"17\", &label)"), "{main}");
    insta::assert_snapshot!("map_err_closure_main_rs", main);
}

// --- Chains (M4 spec 2.3, 3.3) ----------------------------------------------

/// The `iterators.vr` listing of M4 spec 4.
const ITERATORS: &str = "fn main() {
    let numbers = vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10];
    println!(\"{}\", numbers.iter().sum());
    println!(\"{}\", numbers.iter().filter(|n| n % 3 == 0).count());
    println!(\"{}\", numbers.iter().any(|n| n > 9));
    let doubled: Vec<string> = numbers.iter().filter(|n| n < 4).map(|n| format!(\"{}\", n * 2)).collect();
    println!(\"{}\", doubled.join(\" \"));
    let names = vec![\"cherry\", \"apple\", \"banana\"];
    let mut sorted: Vec<string> = names.iter().map(|n| n.clone()).collect();
    sorted.sort();
    println!(\"{}\", sorted.join(\", \"));
    let text = \"one two three\";
    println!(\"{}\", text.split(\" \").count());
}
";

#[test]
fn chains_of_the_iterators_listing() {
    let main = main_rs(ITERATORS);
    assert!(
        main.contains("numbers.iter().copied().sum::<i32>()"),
        "{main}"
    );
    assert!(
        main.contains(".filter(|n| {\n        let n = *n;\n        n % 3 == 0\n    })"),
        "{main}"
    );
    assert!(main.contains(".any(|n| n > 9)"), "{main}");
    assert!(main.contains(".collect::<Vec<_>>()"), "{main}");
    assert!(main.contains("names.iter().map(|n| n.clone())"), "{main}");
    assert!(main.contains("text.split(\" \").count()"), "{main}");
    insta::assert_snapshot!("iterators_main_rs", main);
}

#[test]
fn a_find_on_borrowed_items_is_looked_into_where_it_is_made() {
    let main = main_rs(
        "struct User {\n    name: string,\n    age: i32,\n}\n\nfn greet(name: string) {\n    println!(\"hi {}\", name);\n}\n\nfn main() {\n    let users = vec![User { name: \"ann\", age: 30 }, User { name: \"bo\", age: 4 }];\n    if let Some(u) = users.iter().find(|u| u.age > 18) {\n        greet(u.name);\n    }\n    match users.iter().map(|u| u.name).find(|n| n.len() < 3) {\n        Some(n) => greet(n),\n        None => {}\n    }\n    let ages = vec![3, 40];\n    let adult = ages.iter().find(|a| a > 18).unwrap_or(0);\n    println!(\"{}\", adult);\n}\n",
    );
    assert!(
        main.contains("if let Some(u) = users.iter().find(|u| {"),
        "{main}"
    );
    assert!(main.contains(".map(|u| u.name.as_str())"), "{main}");
    assert!(main.contains("ages.iter().copied().find(|a| {"), "{main}");
    assert!(!main.contains("}).copied()"), "{main}");
    insta::assert_snapshot!("find_looked_into_main_rs", main);
}

#[test]
fn a_map_to_trimmed_lines_hands_all_a_str() {
    let main = main_rs(
        "fn main() {\n    let lines = vec![\"  a  \", \" b\"];\n    let tidy = lines.iter().map(|line| line.trim()).all(|line| line.len() == 1);\n    println!(\"{}\", tidy);\n}\n",
    );
    assert!(
        main.contains("lines.iter().map(|line| line.trim()).all(|line| line.len() == 1usize)"),
        "{main}"
    );
    insta::assert_snapshot!("map_trim_all_main_rs", main);
}

// --- `for` over a chain (M4 spec 2.3, 3.3) ----------------------------------

/// The `words.vr` listing of M4 spec 4.
const WORDS: &str = "fn main() {
    let text = \"the cat saw the dog and the cat ran\";
    let mut counts: HashMap<string, i32> = HashMap::new();
    for word in text.split(\" \") {
        let n = counts.get(word).unwrap_or(0) + 1;
        counts.insert(word.clone(), n);
    }
    let mut words: Vec<string> = counts.keys().map(|w| w.clone()).collect();
    words.sort();
    for word in words {
        if let Some(n) = counts.get(word) {
            println!(\"{} {}\", word, n);
        }
    }
    println!(\"{} distinct words\", counts.len());
}
";

#[test]
fn a_for_over_split_in_the_words_listing() {
    let main = main_rs(WORDS);
    assert!(main.contains("for word in text.split(\" \") {"), "{main}");
    insta::assert_snapshot!("words_main_rs", main);
}

#[test]
fn a_for_over_a_chain_is_written_as_it_is() {
    let main = main_rs(
        "fn greet(name: string) {\n    println!(\"hi {}\", name);\n}\n\nfn main() {\n    let text = \"ann bo\";\n    let sep = \" \";\n    for w in text.split(sep) {\n        greet(w);\n    }\n    let numbers = vec![1, 5, 9];\n    let limit = 4;\n    for n in numbers.iter().filter(|n| n > limit) {\n        println!(\"{}\", n);\n    }\n    let names = vec![\"ann\", \"bo\"];\n    let mut best = \"\";\n    for w in names.iter().filter(|w| w.len() > 2) {\n        best = w;\n    }\n    greet(best);\n}\n",
    );
    assert!(main.contains("for w in text.split(sep) {"), "{main}");
    assert!(
        main.contains("for n in numbers.iter().copied().filter(|n| {"),
        "{main}"
    );
    assert!(
        main.contains("for w in names.iter().filter(|w| {"),
        "{main}"
    );
    insta::assert_snapshot!("for_over_chain_main_rs", main);
}

// --- The source map (M3 §5) over the milestone-4 nodes ----------------------

/// Whether a generated line carries no code of its own: blank, only
/// braces and punctuation, a lint or `#[derive]` attribute, a `mod` line,
/// or an `impl X {` header.
fn is_structural(line: &str) -> bool {
    let line = line.trim();
    line.chars().all(|c| "{}()[];,".contains(c))
        || line == ALLOW_ITEM
        || line.starts_with("#[derive(")
        || line.starts_with("mod ")
        || (line.starts_with("impl ") && line.ends_with('{'))
}

/// Every generated line of `text.vr` and `patterns.vr` that is not
/// structural maps back to a Varyk span, so rustc's message for any of
/// them lands on Varyk source (M4 spec 7).
#[test]
fn every_line_of_the_new_examples_maps_to_a_span() {
    for example in ["examples/text.vr", "examples/patterns.vr"] {
        let krate = generate_path(example);
        let main = krate
            .files
            .iter()
            .find(|file| file.path == "src/main.rs")
            .unwrap_or_else(|| panic!("no src/main.rs for {example}"));
        let unmapped: Vec<(usize, &str)> = main
            .text
            .lines()
            .enumerate()
            .filter(|(i, line)| {
                !is_structural(line) && main.lines.get(*i).copied().flatten().is_none()
            })
            .collect();
        assert_eq!(unmapped, Vec::new(), "{example}");
    }
}
