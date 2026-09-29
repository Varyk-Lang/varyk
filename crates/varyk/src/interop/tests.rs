use super::*;

fn one(text: &str) -> ImportedFn {
    let mut imported = import_rust_module(text).expect("should parse").fns;
    assert_eq!(
        imported.len(),
        1,
        "expected exactly one imported fn from: {text}"
    );
    imported.remove(0)
}

fn local(name: &str) -> RustTy {
    RustTy::Named(RustPath::Local(name.to_string()))
}

fn none(text: &str) {
    let imported = import_rust_module(text).expect("should parse").fns;
    assert!(
        imported.is_empty(),
        "expected no imported fns from: {text}, got {imported:?}"
    );
}

// --- Import table (spec 4.5): parameter types -----------------------

#[test]
fn param_bool() {
    let f = one("pub fn f(x: bool) {}");
    assert_eq!(f.params, vec![RustTy::Bool]);
}

#[test]
fn param_integers() {
    let f = one("pub fn f(a: i8, b: i16, c: i32, d: i64, e: u8, g: u16, h: u32, i: u64) {}");
    assert_eq!(
        f.params,
        vec![
            RustTy::I8,
            RustTy::I16,
            RustTy::I32,
            RustTy::I64,
            RustTy::U8,
            RustTy::U16,
            RustTy::U32,
            RustTy::U64,
        ]
    );
}

#[test]
fn param_usize() {
    let f = one("pub fn f(n: usize, r: &usize) -> usize { n }");
    assert_eq!(
        f.params,
        vec![RustTy::Usize, RustTy::Ref(Box::new(RustTy::Usize))]
    );
    assert_eq!(f.ret, RustTy::Usize);
}

#[test]
fn param_floats() {
    let f = one("pub fn f(a: f32, b: f64) {}");
    assert_eq!(f.params, vec![RustTy::F32, RustTy::F64]);
}

#[test]
fn param_shared_ref_to_copy_primitive() {
    let f = one("pub fn f(x: &i32) {}");
    assert_eq!(f.params, vec![RustTy::Ref(Box::new(RustTy::I32))]);
}

#[test]
fn param_mut_ref_to_copy_primitive() {
    let f = one("pub fn f(x: &mut i32) {}");
    assert_eq!(f.params, vec![RustTy::RefMut(Box::new(RustTy::I32))]);
}

#[test]
fn param_str_ref() {
    let f = one("pub fn f(x: &str) {}");
    assert_eq!(f.params, vec![RustTy::Str]);
}

#[test]
fn param_mut_string_ref() {
    let f = one("pub fn f(x: &mut String) {}");
    assert_eq!(f.params, vec![RustTy::RefMutString]);
}

#[test]
fn param_owned_string() {
    let f = one("pub fn f(x: String) {}");
    assert_eq!(f.params, vec![RustTy::String]);
}

#[test]
fn param_shared_string_ref() {
    let f = one("pub fn f(x: &String) {}");
    assert_eq!(f.params, vec![RustTy::Opaque("& String".to_string())]);
}

// --- Return types ------------------------------------------------------

#[test]
fn return_string() {
    let f = one("pub fn f() -> String { String::new() }");
    assert_eq!(f.ret, RustTy::String);
}

#[test]
fn return_primitive() {
    let f = one("pub fn f() -> i32 { 0 }");
    assert_eq!(f.ret, RustTy::I32);
}

#[test]
fn return_missing_is_unit() {
    let f = one("pub fn f() {}");
    assert_eq!(f.ret, RustTy::Unit);
}

#[test]
fn return_explicit_unit() {
    let f = one("pub fn f() -> () {}");
    assert_eq!(f.ret, RustTy::Unit);
}

#[test]
fn return_reference_is_opaque() {
    let f = one("pub fn f() -> &'static str { \"\" }");
    assert!(matches!(f.ret, RustTy::Opaque(_)));
}

// --- Borrowed returns (M4 spec 2.12) ---------------------------------

fn method_of(text: &str) -> ImportedFn {
    let mut imported = import_rust_module(text).expect("should parse").structs;
    assert_eq!(imported.len(), 1, "expected one struct from: {text}");
    let mut methods = imported.remove(0).methods;
    assert_eq!(methods.len(), 1, "expected one method from: {text}");
    methods.remove(0)
}

fn method_sig(sig: &str) -> ImportedFn {
    method_of(&format!(
        "pub struct S {{ pub n: i32 }}\nimpl S {{ pub {sig} {{ todo!() }} }}"
    ))
}

#[test]
fn a_shared_self_method_returning_str_is_rooted_at_self() {
    let f = method_sig("fn name(&self, sep: &str) -> &str");
    assert_eq!(f.ret, RustTy::Str);
    assert_eq!(f.ret_root, Some(0));
}

#[test]
fn a_shared_self_method_returning_a_struct_reference_is_rooted_at_self() {
    let f = method_sig("fn me(&self) -> &Self");
    assert_eq!(f.ret, RustTy::Ref(Box::new(local("S"))));
    assert_eq!(f.ret_root, Some(0));
    let f = method_sig("fn me(&self, n: i32) -> &S");
    assert_eq!(f.ret, RustTy::Ref(Box::new(local("S"))));
    assert_eq!(f.ret_root, Some(0));
}

#[test]
fn a_free_fn_with_one_shared_reference_parameter_is_rooted_at_it() {
    let f = one("pub fn first_word(s: &str) -> &str { s }");
    assert_eq!(f.ret, RustTy::Str);
    assert_eq!(f.ret_root, Some(0));
    let f = one("pub fn after(n: i32, s: &str) -> &str { s }");
    assert_eq!(f.ret_root, Some(1));
}

#[test]
fn other_reference_returns_stay_opaque() {
    for text in [
        "pub fn f(a: &str, b: &str) -> &str { a }",
        "pub fn f(v: &mut Vec<String>) -> &str { \"\" }",
        "pub fn f(s: &mut String) -> &str { s }",
        "pub fn f(n: i32) -> &str { \"\" }",
        "pub fn f(s: &str) -> &mut String { todo!() }",
        "pub fn f(s: &str) -> Option<&str> { None }",
        "pub fn f(s: &str) -> &[i32] { &[] }",
        "pub fn f(s: &str) -> &String { todo!() }",
        "pub fn f(s: &str) -> impl Fn(i32) -> i32 { |x| x }",
        "pub fn f(s: &str) -> fn(i32) -> i32 { todo!() }",
        "pub fn f(s: &'static str) -> &str { s }",
        "pub fn f<'a>(s: &'a str) -> &'a str { s }",
    ] {
        let f = one(text);
        assert!(matches!(f.ret, RustTy::Opaque(_)), "{text}: {:?}", f.ret);
        assert_eq!(f.ret_root, None, "{text}");
    }
    for sig in [
        "fn pick<'a>(&self, other: &'a str) -> &'a str",
        "fn get(&mut self) -> &S",
        "fn get(&mut self) -> &str",
        "fn get(&'static self) -> &str",
        "fn get(&self) -> Option<&S>",
        "fn get(&self) -> &[i32]",
        "fn get(&self) -> &String",
        "fn get(&self) -> &mut S",
    ] {
        let f = method_sig(sig);
        assert!(matches!(f.ret, RustTy::Opaque(_)), "{sig}: {:?}", f.ret);
        assert_eq!(f.ret_root, None, "{sig}");
    }
}

// --- Opaque parameter cases, signature text preserved -------------------

#[test]
fn param_generic_type_is_opaque() {
    let f = one("pub fn f<T>(x: T) {}");
    assert_eq!(f.params, vec![RustTy::Opaque("T".to_string())]);
}

#[test]
fn param_vec_of_a_mapped_type_is_opaque_only_when_its_element_is() {
    let f = one("pub fn f(x: Vec<&str>) {}");
    match &f.params[0] {
        RustTy::Opaque(text) => assert_eq!(text, "Vec < & str >"),
        other => panic!("expected Opaque, got {other:?}"),
    }
}

#[test]
fn param_option_is_opaque() {
    let f = one("pub fn f<T>(x: Option<T>) {}");
    match &f.params[0] {
        RustTy::Opaque(text) => assert_eq!(text, "Option < T >"),
        other => panic!("expected Opaque, got {other:?}"),
    }
}

#[test]
fn param_lifetime_ref_is_opaque() {
    let f = one("pub fn f<'a>(x: &'a str) {}");
    match &f.params[0] {
        RustTy::Opaque(text) => assert_eq!(text, "& 'a str"),
        other => panic!("expected Opaque, got {other:?}"),
    }
}

#[test]
fn param_trait_object_is_opaque() {
    let f = one("pub fn f(x: &dyn Foo) {}");
    assert!(matches!(&f.params[0], RustTy::Opaque(_)));
}

#[test]
fn param_impl_trait_is_opaque() {
    let f = one("pub fn f(x: impl Foo) {}");
    assert!(matches!(&f.params[0], RustTy::Opaque(_)));
}

#[test]
fn param_tuple_is_opaque() {
    let f = one("pub fn f(x: (i32, i32)) {}");
    match &f.params[0] {
        RustTy::Opaque(text) => assert_eq!(text, "(i32 , i32)"),
        other => panic!("expected Opaque, got {other:?}"),
    }
}

#[test]
fn param_array_is_opaque() {
    let f = one("pub fn f(x: [i32; 4]) {}");
    assert!(matches!(&f.params[0], RustTy::Opaque(_)));
}

#[test]
fn param_ref_to_non_primitive_is_opaque() {
    let f = one("pub fn f(x: &Foo) {}");
    match &f.params[0] {
        RustTy::Opaque(text) => assert_eq!(text, "& Foo"),
        other => panic!("expected Opaque, got {other:?}"),
    }
}

#[test]
fn param_multi_segment_path_is_opaque() {
    let f = one("pub fn f(x: std::string::String) {}");
    assert!(matches!(&f.params[0], RustTy::Opaque(_)));
}

/// `&mut str` is not in the spec 4.5 table (which only has `&str`);
/// it must not be conflated with `&str`.
#[test]
fn param_mut_str_ref_is_opaque() {
    let f = one("pub fn f(x: &mut str) {}");
    assert!(matches!(&f.params[0], RustTy::Opaque(_)));
}

#[test]
fn generic_function_has_an_opaque_return() {
    let f = one("pub fn zero<T: Default>() -> i32 { 0 }");
    assert!(matches!(&f.ret, RustTy::Opaque(_)), "{f:?}");
    let f = one("pub fn zero<T>() -> i32 where T: Default { 0 }");
    assert!(matches!(&f.ret, RustTy::Opaque(_)), "{f:?}");
    let f = one("pub fn plain() -> i32 { 0 }");
    assert_eq!(f.ret, RustTy::I32);
}

// --- Shadowed type names: a local item can shadow a mapped name --------

/// A module-local `struct` named after a primitive shadows it: the
/// bare name no longer resolves to the primitive, so it must not be
/// mapped as one (or the generated crate fails to compile, since the
/// call site would pass a Varyk-primitive value where the local struct
/// type is expected).
#[test]
fn struct_shadowing_a_primitive_name_is_not_the_primitive() {
    let f = one("pub struct i32; pub fn f(_: i32) {}");
    assert_eq!(f.params, vec![local("i32")]);
}

/// Same shadowing hazard via a local `type` alias reusing `String`.
#[test]
fn type_alias_shadowing_string_is_not_string() {
    let f = one("type String = Vec<u8>; pub fn f(x: String) {}");
    assert_eq!(f.params, vec![local("String")]);
}

/// A `use` that renames an import onto a mapped name is the same
/// hazard: `i32` now names whatever was imported, not the primitive.
#[test]
fn use_rename_shadowing_a_primitive_name_is_not_the_primitive() {
    let f = one("use std::string::String as i32; pub fn f(x: i32) {}");
    assert_eq!(
        f.params,
        vec![RustTy::Named(RustPath::Used("i32".to_string()))]
    );
}

/// An ordinary module with no shadowing items maps names exactly as
/// before: an unrelated `use` and struct don't affect `i32`/`String`.
#[test]
fn ordinary_module_without_shadowing_is_unaffected() {
    let f = one("use std::collections::HashMap; \
         struct Point { x: i32, y: i32 } \
         pub fn f(a: i32, b: String) -> i32 { a }");
    assert_eq!(f.params, vec![RustTy::I32, RustTy::String]);
    assert_eq!(f.ret, RustTy::I32);
}

/// A glob `use` could bring in a same-named `String` from anywhere, so
/// every mapped name becomes opaque conservatively, making the function
/// uncallable (V0108 at the resolver).
#[test]
fn glob_use_makes_mapped_names_unmapped() {
    let f = one("use crate::types::*; pub fn f(_: String, _: Vec<i32>) {}");
    assert_eq!(
        f.params[0],
        RustTy::Named(RustPath::Glob("String".to_string()))
    );
    assert!(matches!(&f.params[1], RustTy::Opaque(_)), "{f:?}");
}

/// A name only a glob could have brought in is recorded as such, so the
/// resolver says "replace the glob" rather than naming a `use` line that
/// does not exist; a name a named `use` brings in stays `Used`.
#[test]
fn name_only_a_glob_could_bring_in_is_glob() {
    let f = one("use std::collections::*; pub fn two() -> i32 { 2 }");
    assert_eq!(f.ret, RustTy::Named(RustPath::Glob("i32".to_string())));
    let f = one("use std::collections::*; use crate::t::Thing; pub fn f(_: Thing) {}");
    assert_eq!(
        f.params[0],
        RustTy::Named(RustPath::Used("Thing".to_string()))
    );
}

/// A call of a macro whose definition in the file holds an item keyword
/// could define a same-named `String` item, so every mapped name is
/// conservatively unmapped, recorded as the macro's doing; the call (the
/// first such) is kept with its line and why for the note.
#[test]
fn item_macro_that_could_define_types_makes_mapped_names_unmapped() {
    let imported = import_rust_module(
        "macro_rules! ty { () => { pub struct String; } }\nty!();\n\
         pub fn f(_: String, v: Vec<i32>) -> i32 { 1 }",
    )
    .expect("should parse");
    let f = &imported.fns[0];
    assert_eq!(
        f.params,
        vec![
            RustTy::Named(RustPath::Macro("String".to_string())),
            RustTy::Named(RustPath::Macro("Vec".to_string())),
        ]
    );
    assert_eq!(f.ret, RustTy::Named(RustPath::Macro("i32".to_string())));
    assert_eq!(
        imported.expander,
        Some(Expander {
            shown: "ty!".to_string(),
            line: 2,
            risk: MacroRisk::Writes
        })
    );
    let imported = import_rust_module("fn g() {}\nmake! { struct S; }\npub fn f(_: String) {}")
        .expect("should parse");
    assert_eq!(
        imported.expander,
        Some(Expander {
            shown: "make!".to_string(),
            line: 2,
            risk: MacroRisk::Writes
        })
    );
}

/// A call resolves to a `macro_rules!` at any depth: one defined in an
/// inline module under `#[macro_use]` still decides by its text.
#[test]
fn a_macro_defined_in_an_inline_module_is_read() {
    let imported = import_rust_module(
        "#[macro_use]\nmod inner {\n    macro_rules! mk { () => { pub type String = i32; } }\n}\n\
         mk!();\npub fn f() -> String { 1 }",
    )
    .expect("should parse");
    assert_eq!(
        imported.fns[0].ret,
        RustTy::Named(RustPath::Macro("String".to_string()))
    );
    assert_eq!(imported.expander.map(|e| e.risk), Some(MacroRisk::Writes));
}

/// A local macro whose text calls another macro relays to whatever that
/// one writes, which may be defined in another file.
#[test]
fn a_local_macro_calling_another_macro_could_define_types() {
    let imported = import_rust_module(
        "macro_rules! a { () => { crate::mk_string!(); } }\na!();\npub fn f() -> String { \
         String::new() }",
    )
    .expect("should parse");
    assert_eq!(
        imported.fns[0].ret,
        RustTy::Named(RustPath::Macro("String".to_string()))
    );
    assert_eq!(imported.expander.map(|e| e.risk), Some(MacroRisk::Relays));
}

/// A local macro that pastes its tokens as they are relays to any macro
/// its call names.
#[test]
fn a_local_macro_passing_through_a_macro_call_could_define_types() {
    let imported = import_rust_module(
        "macro_rules! fwd { ($($t:tt)*) => { $($t)* } }\nfwd! { crate::mk!(); }\n\
         pub fn f() -> String { String::new() }",
    )
    .expect("should parse");
    assert_eq!(
        imported.fns[0].ret,
        RustTy::Named(RustPath::Macro("String".to_string()))
    );
    assert_eq!(imported.expander.map(|e| e.risk), Some(MacroRisk::Relays));
}

/// `thread_local!`, a local macro whose text has no item keyword and
/// calls no macro, and a definition never called as an item cannot
/// define a type name, so they shadow nothing.
#[test]
fn item_macro_without_item_keywords_does_not_shadow_mapped_names() {
    let imported = import_rust_module(
        "thread_local! { static N: std::cell::Cell<i32> = std::cell::Cell::new(0); }\n\
         macro_rules! twice { ($e:expr) => { $e * 2 } }\n\
         macro_rules! nothing { () => {} }\nnothing!();\n\
         macro_rules! unused { () => { pub struct String; } }\n\
         pub fn f(s: String, v: Vec<i32>) -> i32 { 1 }",
    )
    .expect("should parse");
    let f = &imported.fns[0];
    assert_eq!(
        f.params,
        vec![RustTy::String, RustTy::Vec(Box::new(RustTy::I32))]
    );
    assert_eq!(f.ret, RustTy::I32);
    assert_eq!(imported.expander, None);
}

/// A macro defined in another file, reached by a path or a `use`, may
/// write items its invocation's tokens never spell.
#[test]
fn item_macro_defined_elsewhere_could_define_types() {
    for text in [
        "crate::make!();\npub fn f(_: String) {}",
        "use crate::make;\nmake!();\npub fn f(_: String) {}",
        "#[macro_use] extern crate std as s;\nmake!();\npub fn f(_: String) {}",
        "make!();\npub fn f(_: String) {}",
    ] {
        let imported = import_rust_module(text).expect("should parse");
        assert_eq!(
            imported.fns[0].params,
            vec![RustTy::Named(RustPath::Macro("String".to_string()))],
            "{text}"
        );
    }
}

/// `include!` splices in a file that is never copied next to the
/// module, so the import is rejected rather than left to fail in rustc.
#[test]
fn include_macro_is_rejected() {
    let err = import_rust_module("include!(\"defs.rs\"); pub fn f() {}").unwrap_err();
    assert_eq!(
        err,
        ImportError::Include {
            name: "include".to_string(),
            span: 0..7
        }
    );
}

/// The same for `include_str!`/`include_bytes!` nested anywhere: in a
/// constant or inside a function body.
#[test]
fn nested_include_str_and_include_bytes_are_rejected() {
    let err = import_rust_module("pub fn f() -> &'static str { include_str!(\"a.txt\") }")
        .unwrap_err()
        .to_string();
    assert!(err.contains("`include_str!`"), "{err}");
    let err = import_rust_module("const B: &[u8] = include_bytes!(\"a.bin\"); pub fn f() {}")
        .unwrap_err()
        .to_string();
    assert!(err.contains("`include_bytes!`"), "{err}");
}

/// Raw spellings name the same things: `r#include_str!` is rejected
/// like `include_str!`, and `struct r#String` shadows `String`.
#[test]
fn raw_identifiers_are_normalized() {
    let err = import_rust_module("pub fn f() -> &'static str { r#include_str!(\"a.txt\") }")
        .unwrap_err()
        .to_string();
    assert!(err.contains("`include_str!`"), "{err}");
    let f = one("pub struct r#String; pub fn f(_: String) {}");
    assert_eq!(f.params, vec![local("String")]);
    let f = one("pub fn r#f(_: r#i32) {}");
    assert_eq!((f.name.as_str(), &f.params), ("f", &vec![RustTy::I32]));
}

/// An identifier merely named `include` (a function, say) is not a
/// macro invocation and is left alone.
#[test]
fn plain_include_identifier_is_not_rejected() {
    let f = one("fn include() {} pub fn f() { include() }");
    assert_eq!(f.name, "f");
}

/// A file with no glob `use` is unaffected: an ordinary named `use`
/// still maps `String` normally.
#[test]
fn non_glob_use_does_not_shadow_mapped_names() {
    let f = one("use std::collections::HashMap; pub fn f(_: String) {}");
    assert_eq!(f.params, vec![RustTy::String]);
}

// --- Ignore list ---------------------------------------------------

#[test]
fn ignores_pub_crate() {
    none("pub(crate) fn f() {}");
}

#[test]
fn ignores_pub_super() {
    none("pub(super) fn f() {}");
}

#[test]
fn ignores_method_in_impl() {
    none("struct S; impl S { pub fn f(&self) {} }");
}

#[test]
fn ignores_unsafe_fn() {
    none("pub unsafe fn f() {}");
}

#[test]
fn ignores_async_fn() {
    none("pub async fn f() {}");
}

#[test]
fn ignores_const_fn() {
    none("pub const fn f() {}");
}

#[test]
fn ignores_extern_fn() {
    none(r#"pub extern "C" fn f() {}"#);
}

/// A plain-`pub` function or `pub use` name left out for a reason a
/// caller could not guess is recorded with why, and a `#[cfg]`-gated
/// struct or enum by name, unless another item of the file takes it.
#[test]
fn skipped_free_functions_re_exports_and_cfg_types_are_recorded() {
    let module = import_rust_module(
        "#[cfg(test)] pub fn a() {}\n\
         pub fn b(#[cfg(test)] x: i32) {}\n\
         pub const fn c() {}\n\
         pub unsafe fn d() {}\n\
         pub async fn e() {}\n\
         pub use std::cmp::{max, min as least};\n\
         #[cfg(test)] fn private() {}\n\
         #[cfg(unix)] pub fn both() {}\n\
         #[cfg(not(unix))] pub fn both() {}\n\
         #[cfg(test)] pub fn kept() {}\n\
         pub fn kept() {}\n\
         #[cfg(test)] pub struct S { pub n: i32 }\n\
         #[cfg(test)] pub enum E { A }\n\
         #[cfg(test)] pub struct T { pub n: i32 }\n\
         pub struct T { pub n: i32 }\n",
    )
    .expect("should parse");
    let fns: Vec<&str> = module.fns.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(fns, ["kept"]);
    assert_eq!(
        module.skipped_fns,
        [
            ("a".to_string(), "function", "behind `#[cfg]`"),
            ("b".to_string(), "function", "a parameter behind `#[cfg]`"),
            ("c".to_string(), "function", "`const`"),
            ("d".to_string(), "function", "`unsafe`"),
            ("e".to_string(), "function", "`async`"),
            (
                "least".to_string(),
                "item",
                "brought in from elsewhere by `pub use`"
            ),
            (
                "max".to_string(),
                "item",
                "brought in from elsewhere by `pub use`"
            ),
            ("both".to_string(), "function", "behind `#[cfg]`"),
            ("both".to_string(), "function", "behind `#[cfg]`"),
        ]
    );
    assert_eq!(
        module.cfg_types,
        [("struct", "S".to_string()), ("enum", "E".to_string())]
    );
}

#[test]
fn ignores_cfg_gated() {
    none("#[cfg(test)] pub fn f() {}");
}

#[test]
fn file_level_cfg_imports_nothing() {
    none("#![cfg(test)] pub fn f() {}");
    none("#![cfg_attr(test, allow(dead_code))] pub fn f() {}");
}

/// A raw-spelled attribute is the same attribute.
#[test]
fn ignores_raw_cfg_gated() {
    none("#[r#cfg(test)] pub fn f() {}");
    none("#![r#cfg(test)] pub fn f() {}");
}

/// A free function with a restricted `pub(...)` is not imported, but its
/// name and visibility are kept, so a use of it can say why.
#[test]
fn restricted_pub_fns_are_recorded_not_imported() {
    let module = import_rust_module(
        "pub(crate) fn two() -> i32 { 2 } pub(super) fn three() {} pub(in crate::g) fn four() {}",
    )
    .expect("parses");
    assert!(module.fns.is_empty());
    assert_eq!(
        module.restricted_fns,
        vec![
            ("two".to_string(), "pub(crate)".to_string()),
            ("three".to_string(), "pub(super)".to_string()),
            ("four".to_string(), "pub(in crate::g)".to_string())
        ]
    );
}

/// `extern crate` names a dependency the generated crate does not
/// have, so the import is rejected; the built-in crates are fine.
#[test]
fn extern_crate_is_rejected_except_builtin_crates() {
    let err = import_rust_module("extern crate serde; pub fn f() {}")
        .unwrap_err()
        .to_string();
    assert!(err.contains("`serde`"), "{err}");
    let f = one(
        "extern crate std; extern crate core; extern crate alloc; extern crate proc_macro; \
         use alloc::vec::Vec as V; pub fn f() {}",
    );
    assert_eq!(f.name, "f");
}

/// `alloc` is not in the extern prelude: without `extern crate alloc;`
/// a `use alloc::..` fails under rustc (E0433), so it is rejected too.
#[test]
fn use_of_alloc_without_extern_crate_is_rejected() {
    let err = import_rust_module("use alloc::vec::Vec; pub fn f() {}")
        .unwrap_err()
        .to_string();
    assert!(err.contains("`alloc`"), "{err}");
}

/// A `use` resolves its first segment against the module it sits in, not
/// the parent (rustc E0432 for the first case).
#[test]
fn use_roots_are_scoped_per_module() {
    let err = import_rust_module("mod m {} mod n { use m::X; } pub fn f() {}")
        .unwrap_err()
        .to_string();
    assert!(err.contains("`m`"), "{err}");
    let f = one("mod n { mod m { pub struct X; } use m::X; } pub fn f() {}");
    assert_eq!(f.name, "f");
    let f = one("extern crate self as me; use me::f as g; pub fn f() {}");
    assert_eq!(f.name, "f");
    let err = import_rust_module("extern crate std as s; mod n { use s::fmt; } pub fn f() {}")
        .unwrap_err()
        .to_string();
    assert!(err.contains("`s`"), "{err}");
}

/// The include rule is blunt on purpose: an `include_str!` in an attribute
/// is a file include like any other.
#[test]
fn include_in_an_attribute_is_rejected() {
    let err = import_rust_module("#[doc = include_str!(\"x.md\")] pub fn f() {}")
        .unwrap_err()
        .to_string();
    assert!(err.contains("include_str"), "{err}");
}

/// A leading `::` names a crate, never a local module (rustc E0432).
#[test]
fn use_with_a_leading_colon_does_not_see_local_modules() {
    let err = import_rust_module("mod m { pub struct X; } use ::m::X; pub fn f() {}")
        .unwrap_err()
        .to_string();
    assert!(err.contains("`m`"), "{err}");
    let f = one("use ::std::fmt; pub fn f() {}");
    assert_eq!(f.name, "f");
    // A name `extern crate` brings in is a crate name, so `::` reaches it.
    let f = one(
        "extern crate std as s; extern crate alloc; use ::s::mem; use ::alloc::vec; pub fn f() {}",
    );
    assert_eq!(f.name, "f");
}

/// A `use` rooted at a crate the generated crate does not depend on is
/// rejected like `extern crate`; local modules and built-in crates pass.
#[test]
fn use_of_external_crate_is_rejected() {
    let err = import_rust_module("use serde::Serialize; pub fn f() {}")
        .unwrap_err()
        .to_string();
    assert!(err.contains("`serde`"), "{err}");
    let err = import_rust_module("mod m { use ::serde::Serialize; } pub fn f() {}")
        .unwrap_err()
        .to_string();
    assert!(err.contains("`serde`"), "{err}");
    let f = one(
        "mod types { pub struct S; } use types::S; use std::fmt; use crate::types::S as T; use self::{types::S as U}; pub fn f() {}",
    );
    assert_eq!(f.name, "f");
}

/// `extern crate` is checked against crates only: an item of the file with
/// the same name does not make it a dependency (rustc E0463).
#[test]
fn extern_crate_named_like_a_local_item_is_still_rejected() {
    let text = "extern crate foo; pub fn foo() {}";
    let foo = ImportError::Crate {
        name: "foo".to_string(),
        span: 13..16,
    };
    assert_eq!(&text[13..16], "foo");
    assert_eq!(import_rust_module(text).unwrap_err(), foo);
    let deps = ["dep".to_string()];
    assert_eq!(import_rust_module_in(text, Some(&deps)).unwrap_err(), foo);
}

/// In a package, `extern crate` and `use` of a declared dependency pass
/// (a `-` in its manifest name is `_` in Rust); an undeclared crate is
/// rejected, naming the package's `[dependencies]`.
#[test]
fn a_package_module_may_name_its_dependencies_only() {
    let deps = ["dep".to_string(), "regex-lite".to_string()];
    let module = import_rust_module_in(
        "extern crate dep; use regex_lite::Regex; use dep::x; pub fn f() {}",
        Some(&deps),
    )
    .expect("declared dependencies pass");
    assert_eq!(module.fns[0].name, "f");
    let err = import_rust_module_in("use other::x; pub fn f() {}", Some(&deps)).unwrap_err();
    assert_eq!(
        err,
        ImportError::Crate {
            name: "other".to_string(),
            span: 4..9
        }
    );
}

/// An out-of-line `mod inner;` is rejected inside an inline module too.
#[test]
fn nested_out_of_line_module_is_rejected() {
    let err = import_rust_module("mod outer { mod inner; } pub fn f() {}").unwrap_err();
    assert_eq!(
        err,
        ImportError::NestedModule {
            name: "inner".to_string(),
            span: 16..21
        }
    );
}

/// `use m::String::{self}` binds `String`, and an inline `mod String`
/// takes the name too; both make the mapped name opaque.
#[test]
fn grouped_self_import_and_inline_module_shadow_mapped_names() {
    let f =
        one("mod types { pub struct String; } use types::String::{self}; pub fn f(_: String) {}");
    assert_eq!(
        f.params[0],
        RustTy::Named(RustPath::Used("String".to_string()))
    );
    let f = one("mod String {} pub fn f(_: String) {}");
    assert!(matches!(&f.params[0], RustTy::Opaque(_)), "{f:?}");
}

#[test]
fn ignores_cfg_attr_gated() {
    none("#[cfg_attr(not(test), cfg(test))] pub fn f() {}");
}

#[test]
fn ignores_test_fn() {
    none("#[test] pub fn f() {}");
}

#[test]
fn ignores_private_fn() {
    none("fn f() {}");
}

#[test]
fn ignores_macro_rules() {
    none("macro_rules! f { () => {} }");
}

// --- Parse failure ---------------------------------------------------

#[test]
fn parse_failure_returns_named_error() {
    let err = import_rust_module("pub fn f(").expect_err("should fail to parse");
    assert!(
        matches!(&err, ImportError::Parse(message) if !message.is_empty()),
        "{err:?}"
    );
}

// --- Nested modules --------------------------------------------------

/// An out-of-line `mod inner;` cannot be honored, because the backend
/// copies only this one file into the generated crate; the whole import
/// fails rather than silently dropping the declaration.
#[test]
fn out_of_line_mod_declaration_is_an_error() {
    let err = import_rust_module("mod inner;\npub fn f() {}")
        .expect_err("should fail: nested module cannot be copied");
    assert!(err.to_string().contains("inner"), "{err}");
}

/// An inline `mod inner { ... }` has nothing missing from the copied
/// file, so it is simply ignored, same as any other non-`fn` item.
#[test]
fn inline_mod_is_ignored() {
    let f = one("mod inner { pub fn helper() {} }\npub fn f() {}");
    assert_eq!(f.name, "f");
}

// --- The one real input in the repository ---------------------------

#[test]
fn imports_greet_rs() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/interop/greet.rs"
    );
    let text = std::fs::read_to_string(path).expect("examples/interop/greet.rs should exist");
    let fns = import_rust_module(&text).expect("should parse").fns;
    assert_eq!(fns.len(), 2, "{fns:?}");
    assert_eq!(fns[0].name, "hello");
    assert_eq!(fns[0].params, vec![RustTy::Str]);
    assert_eq!(fns[0].ret, RustTy::String);
    // `first_word`, the imported borrowed return of M4 spec 4.
    assert_eq!(fns[1].name, "first_word");
    assert_eq!(fns[1].params, vec![RustTy::Str]);
    assert_eq!(fns[1].ret, RustTy::Str);
    assert_eq!(fns[1].ret_root, Some(0));
}

// --- Structs and methods (M3 spec 4.1, 4.2, 4.4) ----------------------

/// The one struct `text` imports.
fn one_struct(text: &str) -> ImportedStruct {
    let mut module = import_rust_module(text).expect("should parse");
    assert_eq!(module.structs.len(), 1, "{module:?}");
    module.structs.remove(0)
}

/// `#[derive(..)]` lists are read for `Clone` and `PartialEq` and nothing
/// else, across several attributes; a hand-written `impl` is not seen
/// (M4 spec 2.12).
#[test]
fn derive_lists_are_read_for_clone_and_partial_eq() {
    let both = Derives {
        clone: true,
        eq: true,
    };
    let clone = Derives {
        clone: true,
        eq: false,
    };
    let eq = Derives {
        clone: false,
        eq: true,
    };
    for (text, derives) in [
        (
            "#[derive(Clone, PartialEq)] pub struct S { pub a: i32 }",
            both,
        ),
        ("#[derive(Debug, Clone)] pub struct S { pub a: i32 }", clone),
        (
            "#[derive(Debug)]\n#[derive(PartialEq, Eq, Hash)] pub struct S { pub a: i32 }",
            eq,
        ),
        (
            "#[derive(Debug, Default)] pub struct S { pub a: i32 }",
            Derives::default(),
        ),
        (
            "#[derive(other::Clone, PartialEq)] pub struct S { pub a: i32 }",
            eq,
        ),
        (
            "pub struct S { pub a: i32 }\nimpl Clone for S { fn clone(&self) -> S { S { a: self.a } } }",
            Derives::default(),
        ),
    ] {
        assert_eq!(one_struct(text).derives, derives, "{text}");
    }
    for (text, derives) in [
        (
            "#[derive(Clone, Copy, PartialEq)] pub enum K { A, B(i32) }",
            both,
        ),
        ("#[derive(PartialEq)] pub enum K { A }", eq),
        ("pub enum K { A }", Derives::default()),
    ] {
        assert_eq!(one_enum(text).derives, derives, "{text}");
    }
}

/// A `#[repr(packed)]` struct, whose fields cannot be borrowed, and one
/// with no fixed size are listed as unfit, with why.
#[test]
fn packed_and_unsized_structs_are_unfit() {
    for text in [
        "#[repr(packed)] pub struct S { pub a: u8, pub x: i32 }",
        "#[repr(C, packed(2))] pub struct S { pub a: u8, pub x: i32 }",
    ] {
        let s = one_struct(text);
        assert!(s.unfit.is_some_and(|why| why.contains("packed")), "{text}");
    }
    for text in [
        "pub struct S { pub n: i32, pub bytes: [u8] }",
        "pub struct S { pub text: str }",
        "pub struct S { pub n: i32, pub f: dyn std::fmt::Debug }",
    ] {
        let s = one_struct(text);
        assert!(
            s.unfit.is_some_and(|why| why.contains("no fixed size")),
            "{text}"
        );
    }
    for text in [
        "#[repr(C)] pub struct S { pub a: u8 }",
        "pub struct S { pub bytes: Vec<u8>, pub s: String }",
    ] {
        assert_eq!(one_struct(text).unfit, None, "{text}");
    }
}

#[test]
fn a_skipped_method_is_recorded_with_why() {
    let s = one_struct(
        "pub struct A { pub n: i32 }\n\
         impl A {\n\
             pub fn ok(&self) -> i32 { 1 }\n\
             pub unsafe fn u(&self) {}\n\
             pub const fn c() -> i32 { 1 }\n\
             pub async fn a(&self) {}\n\
             #[cfg(test)] pub fn t(&self) {}\n\
             fn private(&self) {}\n\
         }\n\
         #[cfg(unix)] impl A { pub fn gated(&self) {} }\n\
         impl Clone for A { fn clone(&self) -> A { A { n: self.n } } }\n\
         impl std::fmt::Display for crate::ext::A {\n\
             fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result { Ok(()) }\n\
         }\n",
    );
    let names: Vec<&str> = s.methods.iter().map(|m| m.name.as_str()).collect();
    assert_eq!(names, ["ok"]);
    assert_eq!(
        s.skipped_methods,
        [
            ("u".to_string(), "`unsafe`"),
            ("c".to_string(), "`const`"),
            ("a".to_string(), "`async`"),
            ("t".to_string(), "behind `#[cfg]`"),
            ("gated".to_string(), "behind `#[cfg]`"),
            ("clone".to_string(), "a trait method"),
            ("fmt".to_string(), "a trait method"),
        ]
    );
}

#[test]
fn restricted_methods_and_types_are_recorded_with_their_visibility() {
    let module = import_rust_module(
        "pub struct A { pub n: i32 }\n\
         impl A {\n\
             pub(crate) fn c(&self) {}\n\
             pub(super) fn s() -> A { A { n: 1 } }\n\
             pub(in crate::x) fn i(&self) {}\n\
             pub(self) fn own(&self) {}\n\
             fn private(&self) {}\n\
         }\n\
         pub(crate) struct B { pub n: i32 }\n\
         pub(super) enum C { X }\n\
         struct D;\n",
    )
    .expect("should parse");
    assert_eq!(
        module.structs[0].restricted_methods,
        [
            ("c".to_string(), "pub(crate)".to_string()),
            ("s".to_string(), "pub(super)".to_string()),
            ("i".to_string(), "pub(in crate::x)".to_string()),
            ("own".to_string(), "pub(self)".to_string()),
        ]
    );
    assert_eq!(
        module.restricted_types,
        [
            ("struct", "B".to_string(), "pub(crate)".to_string()),
            ("enum", "C".to_string(), "pub(super)".to_string()),
        ]
    );
}

fn field<'a>(s: &'a ImportedStruct, name: &str) -> &'a ImportedField {
    s.fields
        .iter()
        .find(|f| f.name == name)
        .unwrap_or_else(|| panic!("no field {name} in {s:?}"))
}

fn method<'a>(s: &'a ImportedStruct, name: &str) -> &'a ImportedFn {
    s.methods
        .iter()
        .find(|f| f.name == name)
        .unwrap_or_else(|| panic!("no method {name} in {s:?}"))
}

const MATCHER: &str = "
use std::collections::HashMap;

pub struct Matcher {
    pub label: String,
    words: Vec<String>,
    pub counts: HashMap<String, i32>,
    pub(crate) hits: u32,
}

impl Matcher {
    pub fn new(pattern: &str) -> Self {
        Matcher { label: pattern.to_string(), words: Vec::new(), counts: HashMap::new(), hits: 0 }
    }
    pub fn is_match(&self, s: &str) -> bool {
        self.words.iter().any(|w| w == s)
    }
    pub fn bump(&mut self) {
        self.hits += 1;
    }
    pub fn into_inner(self) -> String {
        self.label
    }
    pub fn name(&self) -> &String {
        &self.label
    }
    pub fn pick<T>(&self, value: T) -> T {
        value
    }
    fn private_helper(&self) {}
}
";

#[test]
fn struct_fields_keep_visibility_and_mapped_types() {
    let s = one_struct(MATCHER);
    assert_eq!(s.name, "Matcher");
    assert_eq!(s.kind, StructKind::Named);
    assert!(!s.generic);
    let label = field(&s, "label");
    assert_eq!((label.vis, &label.ty), (FieldVis::Pub, &RustTy::String));
    assert_eq!(&MATCHER[label.span.clone()], "label");
    let words = field(&s, "words");
    assert_eq!(
        (words.vis, &words.ty),
        (FieldVis::Hidden, &RustTy::Vec(Box::new(RustTy::String)))
    );
    let counts = field(&s, "counts");
    assert_eq!(counts.vis, FieldVis::Pub);
    assert_eq!(
        counts.ty,
        RustTy::Opaque("HashMap < String , i32 >".to_string())
    );
    assert_eq!(field(&s, "hits").vis, FieldVis::Hidden);
    assert_eq!(&MATCHER[s.span.clone()], "Matcher");
}

#[test]
fn inherent_methods_map_receivers_and_resolve_self() {
    let s = one_struct(MATCHER);
    let names: Vec<&str> = s.methods.iter().map(|m| m.name.as_str()).collect();
    assert_eq!(
        names,
        ["new", "is_match", "bump", "into_inner", "name", "pick"]
    );
    let new = method(&s, "new");
    assert_eq!(
        (new.receiver, &new.params, &new.ret),
        (None, &vec![RustTy::Str], &local("Matcher"))
    );
    assert_eq!(new.owner.as_deref(), Some("Matcher"));
    let is_match = method(&s, "is_match");
    assert_eq!(
        (is_match.receiver, &is_match.params, &is_match.ret),
        (Some(SelfMode::Shared), &vec![RustTy::Str], &RustTy::Bool)
    );
    let bump = method(&s, "bump");
    assert_eq!(
        (bump.receiver, &bump.params, &bump.ret),
        (Some(SelfMode::Mutable), &vec![], &RustTy::Unit)
    );
}

/// Owned `self`, a borrowed return, and a generic method stay opaque
/// (spec 4.2).
#[test]
fn owned_self_borrowed_return_and_generic_methods_are_opaque() {
    let s = one_struct(MATCHER);
    let into_inner = method(&s, "into_inner");
    assert_eq!(into_inner.receiver, None);
    assert_eq!(into_inner.params, vec![RustTy::Opaque("self".to_string())]);
    let name = method(&s, "name");
    assert_eq!(name.receiver, Some(SelfMode::Shared));
    assert!(matches!(&name.ret, RustTy::Opaque(_)), "{name:?}");
    let pick = method(&s, "pick");
    assert!(matches!(&pick.ret, RustTy::Opaque(_)), "{pick:?}");
    assert!(pick.signature.contains("pick < T >"), "{}", pick.signature);
}

#[test]
fn methods_are_not_free_functions() {
    let module = import_rust_module(MATCHER).expect("should parse");
    assert!(module.fns.is_empty(), "{:?}", module.fns);
}

#[test]
fn two_impl_blocks_merge() {
    let s = one_struct(
        "pub struct S { pub n: i32 }
         impl S { pub fn a(&self) -> i32 { self.n } }
         impl Clone for S { fn clone(&self) -> S { S { n: self.n } } }
         impl S { pub fn b(&mut self, other: &S, owned: S) -> Vec<S> { vec![owned] } }",
    );
    let names: Vec<&str> = s.methods.iter().map(|m| m.name.as_str()).collect();
    assert_eq!(names, ["a", "b"]);
    let b = method(&s, "b");
    assert_eq!(
        b.params,
        vec![RustTy::Ref(Box::new(local("S"))), local("S")]
    );
    assert_eq!(b.ret, RustTy::Vec(Box::new(local("S"))));
}

#[test]
fn generic_struct_is_listed_as_generic() {
    let s =
        one_struct("pub struct Wrap<T> { pub value: T } impl<T> Wrap<T> { pub fn get(&self) {} }");
    assert!(s.generic);
    assert!(s.methods.is_empty());
    let s = one_struct("pub struct Borrow<'a> { pub text: &'a str }");
    assert!(s.generic);
}

#[test]
fn tuple_and_unit_structs_are_listed_by_kind() {
    let s = one_struct("pub struct Meters(pub f64);");
    assert_eq!((s.kind, s.fields.len()), (StructKind::Tuple, 0));
    let s = one_struct("pub struct Marker;");
    assert_eq!(s.kind, StructKind::Unit);
}

/// A `#[cfg]`-gated field may be there or not, so Varyk treats it as
/// hidden: a literal can neither name it nor leave it out.
#[test]
fn cfg_gated_field_is_hidden() {
    let s = one_struct("pub struct S { pub a: i32, #[cfg(test)] pub b: i32 }");
    assert_eq!(field(&s, "a").vis, FieldVis::Pub);
    assert_eq!(field(&s, "b").vis, FieldVis::Hidden);
}

#[test]
fn private_and_cfg_structs_are_not_imported() {
    let module = import_rust_module(
        "struct Hidden { pub n: i32 } pub(crate) struct Crate { pub n: i32 } \
         #[cfg(test)] pub struct Test { pub n: i32 }",
    )
    .expect("should parse");
    assert!(module.structs.is_empty(), "{:?}", module.structs);
}

#[test]
fn full_crate_path_field_type_is_named() {
    let s = one_struct(
        "pub struct Holder { pub thing: crate::other::Thing, pub list: Vec<crate::other::Thing> }",
    );
    let path = RustPath::Crate(vec!["other".to_string(), "Thing".to_string()]);
    assert_eq!(field(&s, "thing").ty, RustTy::Named(path.clone()));
    assert_eq!(
        field(&s, "list").ty,
        RustTy::Vec(Box::new(RustTy::Named(path)))
    );
}

/// A type reached through a `use` line is recorded as such, so the
/// resolver can say "write the full path" (spec 4.4).
#[test]
fn field_type_reached_through_use_is_used() {
    let s = one_struct("use crate::other::Thing; pub struct Holder { pub thing: Thing }");
    assert_eq!(
        field(&s, "thing").ty,
        RustTy::Named(RustPath::Used("Thing".to_string()))
    );
}

// --- Enums (M3 spec 4.3, 4.4) ------------------------------------------

fn one_enum(text: &str) -> ImportedEnum {
    let mut module = import_rust_module(text).expect("should parse");
    assert_eq!(module.enums.len(), 1, "{module:?}");
    module.enums.remove(0)
}

fn variant<'a>(e: &'a ImportedEnum, name: &str) -> &'a ImportedVariant {
    e.variants
        .iter()
        .find(|v| v.name == name)
        .unwrap_or_else(|| panic!("no variant {name} in {e:?}"))
}

#[test]
fn unit_and_tuple_variants_import() {
    let e = one_enum("pub enum Kind { Word, Number(i32) }");
    assert_eq!(e.name, "Kind");
    assert!(!e.generic);
    assert_eq!(e.opaque, None);
    assert_eq!(variant(&e, "Word").payload, Vec::<RustTy>::new());
    assert_eq!(variant(&e, "Number").payload, vec![RustTy::I32]);
}

/// A variant with named fields makes the whole enum opaque, with a
/// reason naming it (spec 4.3).
#[test]
fn named_field_variant_is_opaque() {
    let e = one_enum("pub enum Shape { Circle { radius: f64 }, Point }");
    let reason = e.opaque.as_deref().expect("should be opaque");
    assert!(reason.contains("Circle"), "{reason}");
    // The other, unit variant is still listed: naming it is also blocked
    // once the enum is opaque (the checker's job, not the importer's).
    assert_eq!(variant(&e, "Point").payload, Vec::<RustTy>::new());
}

/// A `#[cfg]`- or `#[cfg_attr]`-gated variant may not exist, so the enum
/// is opaque, with a reason naming the variant.
#[test]
fn cfg_gated_variant_is_opaque() {
    for text in [
        "pub enum E { A, #[cfg(test)] B }",
        "pub enum E { A, #[cfg_attr(test, allow(dead_code))] B(i32) }",
    ] {
        let e = one_enum(text);
        let reason = e.opaque.as_deref().expect("should be opaque");
        assert!(reason.contains("`B`, behind `#[cfg]`"), "{reason}");
    }
}

/// A `#[cfg]` on a tuple variant's field may remove the field, so the
/// payload's shape is unknown and the enum is opaque.
#[test]
fn cfg_gated_variant_field_is_opaque() {
    for text in [
        "pub enum E { A(#[cfg(any())] String, i32), B }",
        "pub enum E { A(#[cfg_attr(test, cfg(test))] i32), B }",
    ] {
        let e = one_enum(text);
        let reason = e.opaque.as_deref().expect("should be opaque");
        assert!(
            reason.contains("`A`, with a field behind `#[cfg]`"),
            "{reason}"
        );
    }
}

/// A `#[cfg]` on a parameter or the receiver may remove it, so the
/// function's parameters are unknown: a free function is not imported,
/// and a method is recorded as skipped.
#[test]
fn cfg_gated_parameter_or_receiver_skips_the_function() {
    none("pub fn f(#[cfg(test)] x: i32, y: i32) {}");
    none("pub fn f(#[cfg_attr(test, cfg(test))] x: i32) {}");
    let s = one_struct(
        "pub struct A { pub n: i32 }\n\
         impl A {\n\
             pub fn ok(&self) -> i32 { 1 }\n\
             pub fn f(#[cfg(any())] &self, y: i32) {}\n\
             pub fn g(&self, #[cfg(test)] y: i32) {}\n\
         }\n",
    );
    let names: Vec<&str> = s.methods.iter().map(|m| m.name.as_str()).collect();
    assert_eq!(names, ["ok"]);
    assert_eq!(
        s.skipped_methods,
        [
            ("f".to_string(), "a parameter behind `#[cfg]`"),
            ("g".to_string(), "a parameter behind `#[cfg]`"),
        ]
    );
}

/// A payload type that is unmappable by its shape alone (a `HashMap`) is
/// already known to be opaque at import time, with no symbols needed.
#[test]
fn unmapped_payload_type_is_opaque() {
    let e =
        one_enum("use std::collections::HashMap; pub enum Counted { Tally(HashMap<String, i32>) }");
    let reason = e.opaque.as_deref().expect("should be opaque");
    assert!(reason.contains("Tally"), "{reason}");
    assert!(matches!(
        &variant(&e, "Tally").payload[0],
        RustTy::Opaque(_)
    ));
}

/// A payload type reached through a `use` line is opaque for the same
/// reason a field is (spec 4.4): Varyk does not follow it.
#[test]
fn payload_reached_through_use_is_opaque() {
    let e = one_enum("use crate::other::Thing; pub enum Holder { Full(Thing) }");
    let reason = e.opaque.as_deref().expect("should be opaque");
    assert!(reason.contains("Full"), "{reason}");
}

#[test]
fn generic_enum_is_listed_as_generic() {
    let e = one_enum("pub enum Wrap<T> { Value(T) }");
    assert!(e.generic);
    let e = one_enum("pub enum Borrowed<'a> { Ref(&'a str) }");
    assert!(e.generic);
}

#[test]
fn private_and_cfg_enums_are_not_imported() {
    let module = import_rust_module(
        "enum Hidden { A } pub(crate) enum Crate { A } #[cfg(test)] pub enum Test { A }",
    )
    .expect("should parse");
    assert!(module.enums.is_empty(), "{:?}", module.enums);
}

#[test]
fn full_crate_path_and_local_payload_types_are_named() {
    let e =
        one_enum("pub struct Thing; pub enum Holder { Local(Thing), Remote(crate::other::Thing) }");
    assert_eq!(variant(&e, "Local").payload, vec![local("Thing")]);
    let path = RustPath::Crate(vec!["other".to_string(), "Thing".to_string()]);
    assert_eq!(variant(&e, "Remote").payload, vec![RustTy::Named(path)]);
    assert_eq!(e.opaque, None);
}

#[test]
fn std_generics_map_in_enum_payloads() {
    let e = one_enum("pub enum Holder { Many(Vec<i32>), Maybe(Option<String>) }");
    assert_eq!(
        variant(&e, "Many").payload,
        vec![RustTy::Vec(Box::new(RustTy::I32))]
    );
    assert_eq!(
        variant(&e, "Maybe").payload,
        vec![RustTy::Option(Box::new(RustTy::String))]
    );
    assert_eq!(e.opaque, None);
}

#[test]
fn std_generics_map_in_params_and_returns() {
    let f = one(
        "pub fn f(a: Vec<i32>, b: &Vec<String>, c: &mut Option<u8>, d: Result<bool, String>) -> Option<Vec<i32>> { None }",
    );
    assert_eq!(
        f.params,
        vec![
            RustTy::Vec(Box::new(RustTy::I32)),
            RustTy::Ref(Box::new(RustTy::Vec(Box::new(RustTy::String)))),
            RustTy::RefMut(Box::new(RustTy::Option(Box::new(RustTy::U8)))),
            RustTy::Result(Box::new(RustTy::Bool), Box::new(RustTy::String)),
        ]
    );
    assert_eq!(
        f.ret,
        RustTy::Option(Box::new(RustTy::Vec(Box::new(RustTy::I32))))
    );
}

// --- `impl Drop` (a `match` on a temporary must not move out) ----------

#[test]
fn impl_drop_records_the_type_bare_or_by_path() {
    let imported = import_rust_module(
        "pub enum Ev { Quit }\nimpl Drop for Ev { fn drop(&mut self) {} }\n\
         impl std::ops::Drop for crate::other::Other { fn drop(&mut self) {} }\n\
         impl Clone for Ev { fn clone(&self) -> Ev { Ev::Quit } }\n",
    )
    .expect("should parse");
    assert_eq!(imported.drops.targets, vec!["Ev", "Other"]);
    assert!(imported.drops.unknown.is_none());
}

#[test]
fn impl_drop_is_found_in_inline_modules_fn_bodies_and_const_blocks() {
    let imported = import_rust_module(
        "mod inner { impl Drop for super::A { fn drop(&mut self) {} } }\n\
         fn f() { impl Drop for B { fn drop(&mut self) {} } }\n\
         const _: () = { impl core::ops::Drop for C { fn drop(&mut self) {} } };\n\
         use std::ops::Drop as D;\nimpl D for E { fn drop(&mut self) {} }\n",
    )
    .expect("should parse");
    assert_eq!(imported.drops.targets, vec!["A", "B", "C", "E"]);
    assert_eq!(imported.drops.aliases, vec!["D"]);
    // A rename of `Drop` could be implemented from another file.
    assert!(imported.drops.unknown.is_some());
}

#[test]
fn a_file_that_renames_reexports_or_globs_drop_marks_every_enum() {
    for text in [
        "pub use std::ops::Drop as D;\n",
        "use core::ops::{Add, Drop as D};\n",
        "pub use std::ops::Drop;\n",
        "pub(crate) use std::ops::{Drop};\n",
        "use std::ops::*;\n",
        "mod m { pub use core::ops::*; }\n",
    ] {
        let imported = import_rust_module(text).expect("should parse");
        assert!(imported.drops.unknown.is_some(), "{text}");
    }
    for text in ["use std::ops::Drop;\n", "use std::collections::*;\n"] {
        let imported = import_rust_module(text).expect("should parse");
        assert!(imported.drops.unknown.is_none(), "{text}");
    }
}

#[test]
fn impl_drop_scan_records_aliases_structs_and_macros() {
    let imported = import_rust_module(
        "use crate::ext::G2 as Other;\ntype G = crate::ext::G2;\n\
         pub struct S;\nmod m { union U { a: u8 } }\n\
         macro_rules! make { ($t:ty) => { impl Drop for $t { fn drop(&mut self) {} } } }\n",
    )
    .expect("should parse");
    assert_eq!(imported.drops.aliases, vec!["Other", "G"]);
    assert_eq!(imported.drops.structs, vec!["S", "U"]);
    assert!(imported.drops.unknown.is_some());
    let unreadable =
        import_rust_module("impl Drop for (A) { fn drop(&mut self) {} }\n").expect("should parse");
    assert!(unreadable.drops.unknown.is_some());
}

#[test]
fn a_macro_spelling_drop_in_any_position_marks_every_enum() {
    for text in [
        "fn f() { println!(\"{}\", { impl Drop for E { fn drop(&mut self) {} } 0 }); }\n",
        "fn f() -> String { format!(\"{}\", { impl Drop for E { fn drop(&mut self) {} } 0 }) }\n",
        "fn f() { let _ = vec![{ impl Drop for E { fn drop(&mut self) {} } 0 }]; }\n",
        "fn f() { assert!({ impl Drop for E { fn drop(&mut self) {} } true }); }\n",
        "const _: () = { m!(E, Drop); };\n",
        "m!(E, Drop);\n",
        // A macro defined elsewhere, called as an item.
        "dep::droppable!(E);\n",
        "use dep::droppable;\ndroppable!(E);\n",
        "#[macro_use] extern crate std as s;\ndroppable!(E);\n",
        // A local macro relaying to one defined elsewhere, as an item,
        // at any depth.
        "macro_rules! a { () => { dep::droppable!(E); } }\na!();\n",
        "macro_rules! a { () => { dep::droppable!(E); } }\nmod m { a!(); }\n",
        "fn f() { dep::droppable!(E); }\nmacro_rules! b { () => {} }\nb!();\ndroppable!(E);\n",
        // A local macro passing its tokens through to one defined
        // elsewhere.
        "macro_rules! fwd { ($($t:tt)*) => { $($t)* } }\nfwd! { dep::droppable!(E); }\n",
        // A macro defined elsewhere, in a statement or an expression.
        "const _: () = { dep::droppable!(E); };\n",
        "fn f() { dep::droppable!(E); }\n",
        "fn f() { let _ = droppable!(E); }\n",
        // A `std` macro whose tokens call one defined elsewhere.
        "fn f() { println!(\"{}\", dep::droppable!(E)); }\n",
        "fn f() { std::assert!(matches!(dep::droppable!(E), 0)); }\n",
        "macro_rules! id { ($($t:tt)*) => { $($t)* } }\n\
         fn f() { println!(\"{}\", id!(dep::droppable!(E))); }\n",
    ] {
        let imported =
            super::import_rust_module_in(text, Some(&["dep".to_string()])).expect("should parse");
        assert!(imported.drops.unknown.is_some(), "{text}");
    }
    // A macro whose tokens never spell `Drop` cannot write an `impl Drop`.
    for text in [
        "fn f() { println!(\"{}\", 1); let _ = vec![1]; }\n",
        "macro_rules! make { () => {} }\nthread_local! { static N: u8 = 0; }\nmake!();\n",
        // The known `std` macros, nested in each other and by path, and a
        // local expression macro, in a function body.
        "fn f() -> String { assert!(matches!(Some(1), Some(_))); \
         println!(\"{}\", format!(\"{}\", vec![1].len())); ::std::format!(\"x\") }\n",
        "macro_rules! sq { ($e:expr) => { $e * $e } }\nfn f() -> i32 { sq!(2) }\n",
        "fn f() -> i32 { macro_rules! sq { ($e:expr) => { $e * $e } } sq!(2) }\n",
        "macro_rules! sq { ($e:expr) => { $e * $e } }\nfn f() { println!(\"{}\", sq!(2)); }\n",
    ] {
        let imported = import_rust_module(text).expect("should parse");
        assert!(imported.drops.unknown.is_none(), "{text}");
    }
}
