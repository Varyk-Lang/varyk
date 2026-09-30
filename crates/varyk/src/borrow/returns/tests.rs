use varyk_syntax::{FileId, SourceFile};

use super::{Classification, classify};
use crate::borrow::{Context, signatures};
use crate::hir::{HirProgram, LocalId};
use crate::resolve::resolve;
use crate::types::typecheck;

/// The program `text`, type-checked, and the classification of each of
/// its functions, by name.
fn classified(text: &str) -> (HirProgram, Vec<(String, Classification)>) {
    classified_at("dummy/test.vr", text)
}

/// [`classified`] for the program `text` at `path`, whose `mod` files are
/// read beside it.
fn classified_at(path: &str, text: &str) -> (HirProgram, Vec<(String, Classification)>) {
    let mut sources = Vec::new();
    let entry = SourceFile::new(FileId(0), path, text);
    let mut hir = match resolve(entry, &mut sources).and_then(|r| typecheck(r, &sources)) {
        Ok(hir) => hir,
        Err(diagnostics) => panic!("expected a checked program:\n{text}\n{diagnostics:#?}"),
    };
    let signatures = signatures(&hir);
    let cx = Context {
        signatures: &signatures,
        imported: &hir.imported,
        structs: &hir.structs,
        enums: &hir.enums,
        sources: &sources,
    };
    let results = classify(&cx, &mut hir.functions);
    let classes = hir
        .functions
        .iter()
        .zip(results)
        .map(|(f, r)| (f.name.clone(), r.class))
        .collect();
    (hir, classes)
}

/// The classification of function `name` in `text`.
fn class_of(text: &str, name: &str) -> Classification {
    let (_, classes) = classified(text);
    classes
        .into_iter()
        .find(|(n, _)| n == name)
        .map(|(_, c)| c)
        .unwrap_or_else(|| panic!("no function {name}"))
}

/// The classification of each function of `text`, a program with the
/// `.rs` module `r` of text `rs` beside it, by name.
fn classes_with_rs(label: &str, text: &str, rs: &str) -> Vec<(String, Classification)> {
    let dir =
        std::env::temp_dir().join(format!("varyk_returns_test_{label}_{}", std::process::id()));
    let written = std::fs::create_dir_all(&dir)
        .and_then(|()| std::fs::write(dir.join("r.rs"), rs))
        .and_then(|()| std::fs::write(dir.join("main.vr"), text));
    if let Err(error) = written {
        panic!("cannot write the test program: {error}");
    }
    let path = dir.join("main.vr");
    let (_, classes) = classified_at(&path.to_string_lossy(), text);
    let _ = std::fs::remove_dir_all(&dir);
    classes
}

/// The id of the local `name` of function `function` in `text`.
fn local(text: &str, function: &str, name: &str) -> LocalId {
    let (hir, _) = classified(text);
    let f = hir
        .functions
        .iter()
        .find(|f| f.name == function)
        .unwrap_or_else(|| panic!("no function {function}"));
    let index = f
        .locals
        .iter()
        .position(|l| l.name == name)
        .unwrap_or_else(|| panic!("no local {name}"));
    LocalId(index as u32)
}

const USER: &str = "struct User {\n    name: string,\n    nickname: string,\n    age: i32,\n}\nfn mk() -> User {\n    User { name: \"a\", nickname: \"b\", age: 1 }\n}\n";

fn with_user(body: &str) -> String {
    format!("{USER}{body}fn main() {{}}\n")
}

#[test]
fn every_return_new_is_new() {
    let text = with_user(
        "fn f(u: User, c: bool) -> string {\n    if c {\n        return \"x\";\n    }\n    if c { u.name.clone() } else { format!(\"{}\", u.age) }\n}\n",
    );
    assert_eq!(class_of(&text, "f"), Classification::New);
}

#[test]
fn a_field_of_self_and_an_if_of_fields_are_part_of_self() {
    let text = with_user(
        "impl User {\n    fn name_ref(self) -> string {\n        self.name\n    }\n    fn display(self) -> string {\n        if self.nickname.is_empty() { self.name } else { self.nickname }\n    }\n}\n",
    );
    let this = LocalId(0);
    assert_eq!(class_of(&text, "name_ref"), Classification::Part(this));
    assert_eq!(class_of(&text, "display"), Classification::Part(this));
}

#[test]
fn a_literal_beside_a_part_is_mixed() {
    let text =
        with_user("fn f(u: User, c: bool) -> string {\n    if c { \"x\" } else { u.name }\n}\n");
    assert!(matches!(class_of(&text, "f"), Classification::Mixed(..)));
}

#[test]
fn parts_of_two_parameters_are_two_roots() {
    let text = with_user(
        "fn f(a: User, b: User, c: bool) -> string {\n    if c {\n        return a.name;\n    }\n    b.name\n}\nfn g(a: User, b: User, c: bool) -> string {\n    if c { a.name } else { b.name }\n}\n",
    );
    let (a, b) = (LocalId(0), LocalId(1));
    assert_eq!(class_of(&text, "f"), Classification::TwoRoots(a, b));
    assert_eq!(class_of(&text, "g"), Classification::TwoRoots(a, b));
}

#[test]
fn part_of_a_local_is_of_local() {
    let text = with_user("fn f() -> string {\n    let u = mk();\n    u.name\n}\n");
    assert_eq!(
        class_of(&text, "f"),
        Classification::OfLocal(local(&text, "f", "u"))
    );
}

#[test]
fn part_of_a_mut_parameter_is_of_mut_param() {
    let text = with_user(
        "impl User {\n    fn name_mut(mut self) -> string {\n        self.name\n    }\n}\nfn f(mut v: Vec<string>) -> string {\n    v[0]\n}\n",
    );
    assert_eq!(
        class_of(&text, "name_mut"),
        Classification::OfMutParam(LocalId(0))
    );
    assert_eq!(class_of(&text, "f"), Classification::OfMutParam(LocalId(0)));
}

#[test]
fn a_copy_field_is_new() {
    let text = with_user("fn f(u: User) -> i32 {\n    u.age\n}\n");
    assert_eq!(class_of(&text, "f"), Classification::New);
}

#[test]
fn returning_another_borrowed_return_is_part_of_its_argument() {
    let text = with_user(
        "impl User {\n    fn name_ref(self) -> string {\n        self.name\n    }\n}\nfn first(users: Vec<User>) -> User {\n    users[0]\n}\nfn lead_name(users: Vec<User>) -> string {\n    let u = first(users);\n    u.name_ref()\n}\nfn g(s: string) -> string {\n    s.trim()\n}\n",
    );
    assert_eq!(class_of(&text, "first"), Classification::Part(LocalId(0)));
    assert_eq!(
        class_of(&text, "lead_name"),
        Classification::Part(LocalId(0))
    );
    assert_eq!(class_of(&text, "g"), Classification::Part(LocalId(0)));
}

#[test]
fn a_recursive_group_is_never_a_borrowed_return() {
    let text = with_user(
        "fn a(u: User) -> string {\n    b(u)\n}\nfn b(u: User) -> string {\n    let x = a(u);\n    u.name\n}\nfn c(u: User, n: i32) -> string {\n    if n > 0 {\n        let x = c(u, n - 1);\n    }\n    u.name\n}\n",
    );
    // `a`'s call of `b` is inside the group, so it counts as new.
    assert_eq!(class_of(&text, "a"), Classification::New);
    assert!(matches!(class_of(&text, "b"), Classification::Recursive(_)));
    assert!(matches!(class_of(&text, "c"), Classification::Recursive(_)));
}

#[test]
fn a_recursive_call_beside_a_part_is_recursive_not_mixed() {
    let text = with_user(
        "fn d(u: User, n: i32) -> string {\n    if n == 0 {\n        u.name\n    } else {\n        d(u, n - 1)\n    }\n}\n",
    );
    assert!(matches!(class_of(&text, "d"), Classification::Recursive(_)));
}

#[test]
fn a_let_assigned_parts_in_a_loop_is_part_of_what_it_loops_over() {
    let text = "fn longest(words: Vec<string>) -> string {\n    let mut best = \"\";\n    for w in words {\n        if w.len() > best.len() {\n            best = w;\n        }\n    }\n    best\n}\nfn main() {}\n";
    assert_eq!(class_of(text, "longest"), Classification::Part(LocalId(0)));
}

#[test]
fn a_borrowed_return_on_a_literal_is_rootless() {
    let text = "fn first_word(s: string) -> string {\n    s.trim()\n}\nfn f() -> string {\n    first_word(\"a b\")\n}\nfn g() -> string {\n    let w = first_word(\"a b\");\n    w\n}\nfn main() {}\n";
    assert!(matches!(class_of(text, "f"), Classification::Rootless(_)));
    assert!(matches!(class_of(text, "g"), Classification::Rootless(_)));
}

#[test]
fn calls_of_a_borrowed_return_are_rooted_and_its_root_recorded() {
    use crate::hir::{HirExprKind, HirStmt};
    let text = with_user(
        "fn first(users: Vec<User>) -> User {\n    users[0]\n}\nfn f(users: Vec<User>) {\n    let u = first(users);\n}\n",
    );
    let (hir, _) = classified(&text);
    let first = hir.functions.iter().find(|f| f.name == "first");
    assert_eq!(first.and_then(|f| f.ret_root), Some(LocalId(0)));
    let f = hir.functions.iter().find(|f| f.name == "f");
    let rooted = f.and_then(|f| match &f.body.stmts[0] {
        HirStmt::Let { value, .. } => match &value.kind {
            HirExprKind::Call { rooted, .. } => *rooted,
            _ => None,
        },
        _ => None,
    });
    assert_eq!(rooted, Some(0));
}

#[test]
fn a_question_mark_beside_a_part_is_mixed() {
    // A `?` returns early with a new `None` or `Err`.
    let text = "struct User {\n    nick: Option<string>,\n    port: Option<i32>,\n}\nfn g(u: User, m: HashMap<string, i32>) -> Option<string> {\n    let n = m.get(\"k\")?;\n    u.nick\n}\nfn check(x: i32) -> Result<i32, string> {\n    Ok(x)\n}\nfn pick(r: Result<i32, string>, x: i32) -> Result<i32, string> {\n    let y = check(x)?;\n    r\n}\nimpl User {\n    fn nick_or(self, text: string) -> Option<string> {\n        let r: Result<i32, Error> = text.parse();\n        let n: i32 = r.ok()?;\n        self.nick\n    }\n    fn port_of(self, text: string) -> Option<i32> {\n        let r: Result<i32, Error> = text.parse();\n        let n: i32 = r.ok()?;\n        self.port\n    }\n}\nfn main() {}\n";
    // `port_of` returns an `Option` of a Copy payload, still a part.
    for name in ["g", "pick", "nick_or", "port_of"] {
        assert!(
            matches!(class_of(text, name), Classification::Mixed(..)),
            "{name}"
        );
    }
}

#[test]
fn part_of_a_number_parameter_is_of_local() {
    // Rust passes a number by value, so a borrowed return from it is part
    // of a value that ends when the function returns (spec 3.1).
    let rs = "pub fn refnum(x: &i32) -> &str { if *x > 0 { \"pos\" } else { \"neg\" } }\n";
    let text = "mod r;\nfn sign(n: i32) -> string {\n    r::refnum(n)\n}\nfn sign_of(s: string, n: i32) -> string {\n    r::refnum(n)\n}\nfn main() {}\n";
    let classes = classes_with_rs("copy_param", text, rs);
    let class = |name: &str| classes.iter().find(|(n, _)| n == name).map(|(_, c)| *c);
    assert_eq!(class("sign"), Some(Classification::OfLocal(LocalId(0))));
    assert_eq!(class("sign_of"), Some(Classification::OfLocal(LocalId(1))));
}

#[test]
fn part_of_a_copy_for_variable_is_of_local() {
    // Over a `Vec` of numbers, the variable is a copy of each element
    // (spec 3.1): a borrowed return from it is part of a value that ends
    // with the function, not part of the `Vec`.
    let rs = "pub fn refnum(x: &i32) -> &str { if *x > 0 { \"pos\" } else { \"neg\" } }\n";
    let text = "mod r;\nstruct Ns {\n    ns: Vec<i32>,\n}\nimpl Ns {\n    fn sign(self) -> string {\n        for n in self.ns {\n            return r::refnum(n);\n        }\n        r::refnum(self.ns[0])\n    }\n}\nfn f(v: Vec<i32>) -> string {\n    for n in v {\n        if n < 0 {\n            return r::refnum(n);\n        }\n    }\n    r::refnum(v[0])\n}\nfn main() {}\n";
    let classes = classes_with_rs("copy_for", text, rs);
    let class = |name: &str| classes.iter().find(|(n, _)| n == name).map(|(_, c)| *c);
    assert_eq!(class("f"), Some(Classification::OfLocal(LocalId(1))));
    assert_eq!(class("sign"), Some(Classification::OfLocal(LocalId(1))));
}
