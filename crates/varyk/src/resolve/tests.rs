use super::*;
use crate::diagnostics::codes;
use crate::interop::import_rust_module;
use crate::types::{FloatKind, IntKind};
use varyk_syntax::PathStart;

/// Resolves a single-file program from an inline string, with a dummy
/// path (no `mod` declarations may appear).
fn resolve_str(text: &str) -> (Result<Resolved, Vec<Diagnostic>>, Vec<SourceFile>) {
    let mut sources = Vec::new();
    let entry = SourceFile::new(FileId(0), "dummy/test.vr", text);
    let result = resolve(entry, &mut sources);
    (result, sources)
}

fn errors_str(text: &str) -> Vec<Diagnostic> {
    match resolve_str(text).0 {
        Ok(_) => panic!("expected diagnostics for:\n{text}"),
        Err(diagnostics) => diagnostics,
    }
}

/// Resolves `<rel>` relative to the workspace root, the way the CLI
/// would.
fn resolve_file(rel: &str) -> (Result<Resolved, Vec<Diagnostic>>, Vec<SourceFile>) {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel);
    let text = std::fs::read_to_string(&path).expect("fixture should exist");
    let mut sources = Vec::new();
    let entry = SourceFile::new(FileId(0), path, text);
    let result = resolve(entry, &mut sources);
    (result, sources)
}

fn fixture(case: &str) -> (Vec<Diagnostic>, Vec<SourceFile>) {
    fixture_in("resolve", case)
}

/// [`fixture`] for a case of the end-to-end diagnostic fixtures,
/// `tests/fixtures/errors/<case>/`, shared rather than copied.
fn error_fixture(case: &str) -> (Vec<Diagnostic>, Vec<SourceFile>) {
    fixture_in("errors", case)
}

/// Resolves `tests/fixtures/<dir>/<case>/main.vr`, which must fail.
fn fixture_in(dir: &str, case: &str) -> (Vec<Diagnostic>, Vec<SourceFile>) {
    let (result, sources) =
        resolve_file(&format!("crates/varyk/tests/fixtures/{dir}/{case}/main.vr"));
    match result {
        Ok(_) => panic!("expected diagnostics for fixture {case}"),
        Err(diagnostics) => (diagnostics, sources),
    }
}

/// The span of the first occurrence of `needle` in `sources[file]`.
fn span_of(sources: &[SourceFile], file: u32, needle: &str) -> Span {
    let text = &sources[file as usize].text;
    let start = text
        .find(needle)
        .unwrap_or_else(|| panic!("{needle:?} not in file"));
    Span::new(FileId(file), start as u32, (start + needle.len()) as u32)
}

fn only(diagnostics: &[Diagnostic]) -> &Diagnostic {
    assert_eq!(
        diagnostics.len(),
        1,
        "expected one diagnostic, got {diagnostics:#?}"
    );
    &diagnostics[0]
}

/// A module path as written, `crate::shop::cart` or `super`, with dummy
/// spans, for driving the lookups directly.
fn p(text: &str) -> Path {
    let span = Span::new(FileId(0), 0, 0);
    let mut parts: Vec<&str> = text.split("::").collect();
    let leading = match parts[0] {
        "crate" => PathStart::Crate,
        "self" => PathStart::SelfMod,
        "super" => PathStart::Super,
        _ => PathStart::None,
    };
    if leading != PathStart::None {
        parts.remove(0);
    }
    let segments = parts
        .into_iter()
        .map(|name| varyk_syntax::Ident {
            name: name.to_string(),
            span,
        })
        .collect();
    Path {
        leading,
        segments,
        span,
    }
}

fn module_id(resolved: &Resolved, name: &str) -> ModuleId {
    resolved
        .modules
        .iter()
        .find(|m| m.name == name)
        .unwrap_or_else(|| panic!("no module {name}"))
        .id
}

// --- Module loading -------------------------------------------------

#[test]
fn mod_math_loads_math_vr() {
    let (result, sources) = resolve_file("examples/modules/main.vr");
    let resolved = result.expect("examples/modules should resolve");
    assert_eq!(sources.len(), 2);
    assert_eq!(resolved.modules.len(), 2);
    let math = &resolved.modules[module_id(&resolved, "math").0 as usize];
    assert!(matches!(math.kind, ModuleKind::Varyk(_)));
    assert_eq!(math.file, FileId(1));
    assert!(sources[1].path.ends_with("math.vr"));

    let callee = resolved
        .symbols
        .lookup_fn(resolved.entry, Some(&p("math")), "square")
        .expect("math::square should be visible");
    let Callee::Varyk(id) = callee else {
        panic!("expected a Varyk fn, got {callee:?}");
    };
    let sig = &resolved.symbols.fns[id.0 as usize];
    assert_eq!(sig.name, "square");
    assert!(sig.is_pub);
    assert_eq!(
        sig.params,
        vec![("x".to_string(), Ty::Int(IntKind::I32), ParamMode::Owned)]
    );
    assert_eq!(sig.ret, Ty::Int(IntKind::I32));
    assert_eq!(resolved.fn_decl(id).name.name, "square");
}

#[test]
fn mod_greet_loads_greet_rs_through_the_importer() {
    let (result, sources) = resolve_file("examples/interop/main.vr");
    let resolved = result.expect("examples/interop should resolve");
    assert_eq!(sources.len(), 2);
    let greet = &resolved.modules[module_id(&resolved, "greet").0 as usize];
    assert!(matches!(greet.kind, ModuleKind::Rust(_)));

    let callee = resolved
        .symbols
        .lookup_fn(resolved.entry, Some(&p("greet")), "hello")
        .expect("greet::hello should be visible");
    let Callee::Imported(id) = callee else {
        panic!("expected an imported fn, got {callee:?}");
    };
    let sig = &resolved.symbols.imported[id.0 as usize];
    assert!(sig.callable);
    assert_eq!(sig.params, vec![(Ty::String, ParamMode::SharedBorrow)]);
    assert_eq!(sig.ret, Ty::String);
    assert!(sig.signature.contains("hello"));
}

#[test]
fn both_module_files_present_is_v0104_naming_both() {
    let (diagnostics, sources) = fixture("both_files");
    let d = only(&diagnostics);
    assert_eq!(d.code, codes::V0104);
    assert_eq!(d.span, span_of(&sources, 0, "mod util;"));
    assert!(
        d.message.contains("util.vr") && d.message.contains("util.rs"),
        "{}",
        d.message
    );
}

#[test]
fn missing_module_file_is_v0104_at_the_mod() {
    let (diagnostics, sources) = fixture("missing_file");
    let d = only(&diagnostics);
    assert_eq!(d.code, codes::V0104);
    assert_eq!(d.span, span_of(&sources, 0, "mod nothing;"));
}

#[test]
fn unparseable_rs_module_is_v0104_at_the_mod() {
    let (diagnostics, sources) = fixture("bad_rs");
    let d = only(&diagnostics);
    assert_eq!(d.code, codes::V0104);
    assert_eq!(d.span, span_of(&sources, 0, "mod broken;"));
    // The file was still pushed so a diagnostic could cite it.
    assert_eq!(sources.len(), 2);
}

#[test]
fn mod_main_is_v0104_at_the_mod() {
    let (diagnostics, sources) = fixture("mod_main");
    let d = only(&diagnostics);
    assert_eq!(d.code, codes::V0104);
    assert_eq!(d.span, span_of(&sources, 0, "mod main;"));
}

#[test]
fn mod_lib_is_v0104_at_the_mod() {
    let (diagnostics, sources) = fixture("mod_lib");
    let d = only(&diagnostics);
    assert_eq!(d.code, codes::V0104);
    assert_eq!(d.span, span_of(&sources, 0, "mod lib;"));
}

#[test]
fn mod_bin_in_the_root_is_v0104_at_the_mod() {
    let d = errors_str("mod bin;\n\nfn main() {}\n");
    let d = only(&d);
    assert_eq!(d.code, codes::V0104);
    assert!(d.notes.iter().any(|n| n.contains("src/bin/")), "{d:#?}");
}

#[test]
fn reserved_module_names_are_v0104_in_any_capitalization() {
    for (decl, name) in [
        ("mod Main;", "Main"),
        ("mod LIB;", "LIB"),
        ("mod Bin;", "Bin"),
    ] {
        let d = errors_str(&format!("{decl}\n\nfn main() {{}}\n"));
        let d = only(&d);
        assert_eq!(d.code, codes::V0104, "{decl}");
        assert!(d.message.contains(&format!("`{name}`")), "{}", d.message);
        assert!(d.message.contains("any capitalization"), "{}", d.message);
        assert!(d.message.contains("use another name"), "{}", d.message);
        assert!(!d.notes.is_empty(), "{d:?}");
    }
}

#[test]
fn mod_bin_below_the_root_is_allowed() {
    let (result, _) = resolve_file("crates/varyk/tests/fixtures/resolve/nested_bin/main.vr");
    result.expect("a nested `mod bin;` should resolve");
}

#[test]
fn mod_inside_a_vr_module_without_its_file_is_v0104_naming_the_directory() {
    let (diagnostics, sources) = fixture("nested_mod");
    let d = only(&diagnostics);
    assert_eq!(d.code, codes::V0104);
    assert_eq!(d.span, span_of(&sources, 1, "mod inner;"));
    assert!(d.message.contains("`outer/`"), "{}", d.message);
}

// --- The module tree (spec 3.1) ------------------------------------------

fn tree() -> Resolved {
    let (result, _) = resolve_file("crates/varyk/tests/fixtures/resolve/tree/main.vr");
    result.unwrap_or_else(|d| panic!("tree should resolve: {d:#?}"))
}

#[test]
fn mod_in_a_non_entry_vr_file_loads_the_child_from_its_directory() {
    let (result, sources) = resolve_file("crates/varyk/tests/fixtures/resolve/tree/main.vr");
    let r = result.unwrap_or_else(|d| panic!("tree should resolve: {d:#?}"));
    let shop = module_id(&r, "shop");
    let cart = module_id(&r, "cart");
    let module = |id: ModuleId| &r.modules[id.0 as usize];
    assert_eq!(module(shop).parent, Some(r.entry));
    assert_eq!(module(cart).parent, Some(shop));
    assert!(!module(shop).is_pub && module(cart).is_pub);
    assert!(
        sources[module(cart).file.0 as usize]
            .path
            .ends_with("shop/cart.vr")
    );
    assert!(module(shop).dir.ends_with("tree/shop"));
    assert_eq!(
        r.symbols.ancestors(cart).collect::<Vec<_>>(),
        vec![cart, shop, r.entry]
    );
}

#[test]
fn shop_mod_vr_loads_when_shop_vr_is_absent() {
    let (result, sources) = resolve_file("crates/varyk/tests/fixtures/resolve/dir_module/main.vr");
    let r = result.unwrap_or_else(|d| panic!("dir_module should resolve: {d:#?}"));
    let shop = module_id(&r, "shop");
    let cart = module_id(&r, "cart");
    let module = |id: ModuleId| &r.modules[id.0 as usize];
    assert!(
        sources[module(shop).file.0 as usize]
            .path
            .ends_with("shop/mod.vr")
    );
    assert!(
        sources[module(cart).file.0 as usize]
            .path
            .ends_with("shop/cart.vr")
    );
    assert_eq!(module(cart).parent, Some(shop));
}

#[test]
fn shop_vr_beside_shop_mod_vr_is_v0104_naming_both() {
    let (diagnostics, sources) = error_fixture("v0104_mod_vr_twin");
    let d = only(&diagnostics);
    assert_eq!(d.code, codes::V0104);
    assert_eq!(d.span, span_of(&sources, 0, "mod shop;"));
    assert!(
        d.message.contains("shop.vr") && d.message.contains("shop/mod.vr"),
        "{}",
        d.message
    );
}

#[test]
fn vr_beside_rs_at_depth_is_v0104() {
    let (diagnostics, sources) = fixture("deep_twin");
    let d = only(&diagnostics);
    assert_eq!(d.code, codes::V0104);
    assert_eq!(d.span, span_of(&sources, 1, "mod util;"));
    assert!(
        d.message.contains("util.vr") && d.message.contains("util.rs"),
        "{}",
        d.message
    );
}

#[test]
fn super_from_the_crate_root_is_v0111() {
    let d = errors_str("fn f(x: super::Foo) {}\n\nfn main() {}\n");
    let d = only(&d);
    assert_eq!(d.code, codes::V0111);
    assert!(d.message.contains("super"), "{d:#?}");
    let r = tree();
    let err = super::resolve_path(&r.symbols, r.entry, &p("super")).expect_err("no parent");
    assert_eq!(err.code, codes::V0111);
}

#[test]
fn self_super_and_crate_paths_resolve() {
    let r = tree();
    let (root, shop, cart) = (r.entry, module_id(&r, "shop"), module_id(&r, "cart"));
    // Boxed: clippy flags a closure returning the large `Diagnostic`.
    let at = |from: ModuleId, text: &str| {
        super::resolve_path(&r.symbols, from, &p(text)).map_err(Box::new)
    };
    assert_eq!(at(cart, "self"), Ok(cart));
    assert_eq!(at(cart, "super"), Ok(shop));
    assert_eq!(at(cart, "crate"), Ok(root));
    assert_eq!(at(cart, "crate::shop::cart"), Ok(cart));
    assert_eq!(at(shop, "self::cart"), Ok(cart));
    assert_eq!(at(shop, "cart"), Ok(cart));
    assert_eq!(at(root, "shop::cart"), Ok(cart));
    // A bare leading name is a module declared in `from`, never a
    // sibling or a grandchild.
    let err = at(root, "cart").expect_err("cart is not declared in the root");
    assert!(err.message.contains("`cart`"), "{err:#?}");
    let err = at(root, "crate::shop::nope").expect_err("no such module");
    assert!(err.message.contains("crate::shop::nope"), "{err:#?}");
}

#[test]
fn a_three_segment_type_path_resolves_to_the_same_type_as_its_crate_form() {
    let r = tree();
    let symbols = &r.symbols;
    let cart = symbols
        .lookup_type(r.entry, Some(&p("shop::cart")), "Cart")
        .expect("shop::cart::Cart");
    let order = symbols
        .lookup_type(r.entry, Some(&p("shop")), "Order")
        .expect("shop::Order");
    assert_eq!(
        symbols.lookup_type(r.entry, Some(&p("crate::shop::cart")), "Cart"),
        Ok(cart)
    );
    let Ok(Callee::Varyk(take)) = symbols.lookup_fn(r.entry, None, "take") else {
        panic!("take")
    };
    let types: Vec<Ty> = symbols.fns[take.0 as usize]
        .params
        .iter()
        .map(|p| p.1.clone())
        .collect();
    assert_eq!(types, vec![cart.ty(), cart.ty(), order.ty()]);
    let UserType::Struct(cart_id) = cart else {
        panic!("Cart is a struct")
    };
    // `super::Order`-style and `self::Cart` resolve from inside `cart`.
    let cart_module = module_id(&r, "cart");
    let Ok(Callee::Varyk(make)) = symbols.lookup_fn(cart_module, None, "make") else {
        panic!("make")
    };
    assert_eq!(symbols.fns[make.0 as usize].ret, Ty::Struct(cart_id));
    assert_eq!(symbols.fns[make.0 as usize].params[0].1, order.ty());
}

#[test]
fn a_call_path_through_nested_modules_resolves() {
    let r = tree();
    let cart = module_id(&r, "cart");
    let Ok(Callee::Varyk(pick)) = r.symbols.lookup_fn(r.entry, Some(&p("shop::cart")), "pick")
    else {
        panic!("shop::cart::pick")
    };
    assert_eq!(r.symbols.fns[pick.0 as usize].module, cart);
    assert!(
        r.symbols
            .lookup_fn(cart, Some(&p("super")), "start")
            .is_ok()
    );
    assert!(
        r.symbols
            .lookup_fn(cart, Some(&p("crate::shop")), "start")
            .is_ok()
    );
    assert!(
        r.symbols
            .lookup_fn(r.entry, Some(&p("self")), "helper")
            .is_ok()
    );
}

// --- `use` (spec 3.3) ----------------------------------------------------

fn use_basic() -> (Resolved, Vec<SourceFile>) {
    let (result, sources) = resolve_file("crates/varyk/tests/fixtures/resolve/use_basic/main.vr");
    (
        result.unwrap_or_else(|d| panic!("use_basic should resolve: {d:#?}")),
        sources,
    )
}

#[test]
fn use_of_a_type_through_its_crate_path_resolves_as_a_bare_name() {
    let (r, _) = use_basic();
    let symbols = &r.symbols;
    let cart_module = module_id(&r, "cart");
    let cart = symbols
        .lookup_type(r.entry, None, "Cart")
        .expect("Cart should resolve through `use crate::shop::cart::Cart;`");
    let UserType::Struct(id) = cart else {
        panic!("Cart is a struct")
    };
    assert_eq!(symbols.structs[id.0 as usize].module, cart_module);
    assert_eq!(
        symbols.use_target_path(r.entry, "Cart").as_deref(),
        Some("crate::shop::cart::Cart")
    );
}

#[test]
fn use_of_a_module_is_followed_by_more_segments_as_if_it_had_been_named() {
    let (r, _) = use_basic();
    let symbols = &r.symbols;
    let cart_by_crate_path = symbols.lookup_type(r.entry, None, "Cart").expect("Cart");
    let cart_by_alias = symbols
        .lookup_type(r.entry, Some(&p("cart")), "Cart")
        .expect("cart::Cart should resolve through `use shop::cart;`");
    assert_eq!(cart_by_alias, cart_by_crate_path);
    assert_eq!(
        symbols.use_target_path(r.entry, "cart").as_deref(),
        Some("crate::shop::cart")
    );
}

#[test]
fn use_of_a_rust_function_resolves_as_a_bare_call() {
    let (r, _) = use_basic();
    let symbols = &r.symbols;
    let helper = symbols
        .lookup_fn(r.entry, None, "helper")
        .expect("helper should resolve through `use crate::util::helper;`");
    assert!(matches!(helper, Callee::Imported(_)), "{helper:?}");
    assert_eq!(
        symbols.use_target_path(r.entry, "helper").as_deref(),
        Some("crate::util::helper")
    );
}

#[test]
fn use_with_as_introduces_a_different_local_name() {
    let (r, _) = use_basic();
    let symbols = &r.symbols;
    let a_module = module_id(&r, "a");
    // `B` itself is not in scope under its own name at the root.
    assert_eq!(
        symbols.lookup_type(r.entry, None, "B"),
        Err(LookupError::Unknown)
    );
    let c = symbols
        .lookup_type(r.entry, None, "C")
        .expect("C should resolve to a::B through `use a::B as C;`");
    let UserType::Struct(id) = c else {
        panic!("B is a struct")
    };
    assert_eq!(symbols.structs[id.0 as usize].module, a_module);
    assert_eq!(symbols.structs[id.0 as usize].name, "B");
    assert_eq!(
        symbols.use_target_path(r.entry, "C").as_deref(),
        Some("crate::a::B")
    );
}

#[test]
fn use_basic_program_typechecks_with_every_alias_used_as_a_value_call_and_pattern() {
    let (result, sources) = resolve_file("crates/varyk/tests/fixtures/resolve/use_basic/main.vr");
    let resolved = result.expect("use_basic should resolve");
    crate::types::typecheck(resolved, &sources)
        .unwrap_or_else(|d| panic!("use_basic should typecheck: {d:#?}"));
}

#[test]
fn use_of_a_type_and_a_fn_sharing_a_name_coexist_in_their_own_namespaces() {
    // `use crate::a::Foo;` (a struct) and `use crate::b::Foo;` (a
    // function) introduce the same local name in different namespaces
    // (spec 2.1's shared type namespace does not include functions), so
    // neither should overwrite the other's alias.
    let (result, sources) =
        resolve_file("crates/varyk/tests/fixtures/resolve/use_two_namespaces/main.vr");
    let resolved = result.expect("use_two_namespaces should resolve");
    let symbols = &resolved.symbols;
    let struct_foo = symbols
        .lookup_type(resolved.entry, None, "Foo")
        .expect("the struct alias should still resolve");
    assert!(matches!(struct_foo, UserType::Struct(_)), "{struct_foo:?}");
    let fn_foo = symbols
        .lookup_fn(resolved.entry, None, "Foo")
        .expect("the function alias should still resolve");
    assert!(matches!(fn_foo, Callee::Varyk(_)), "{fn_foo:?}");
    crate::types::typecheck(resolved, &sources)
        .unwrap_or_else(|d| panic!("use_two_namespaces should typecheck: {d:#?}"));
}

#[test]
fn use_of_a_known_crate_leading_name_is_v0110_with_the_facade_note() {
    let d = errors_str("use std::collections::HashMap;\n\nfn main() {}\n");
    let d = only(&d);
    assert_eq!(d.code, codes::V0110);
    assert!(d.message.contains("std"), "{d:#?}");
    assert!(
        d.notes
            .iter()
            .any(|n| n.contains("does not use crates directly")),
        "{d:#?}"
    );

    for name in ["core", "alloc"] {
        let d = errors_str(&format!("use {name}::x;\n\nfn main() {{}}\n"));
        let d = only(&d);
        assert_eq!(d.code, codes::V0110, "{name}: {d:#?}");
    }
}

#[test]
fn use_of_an_unrecognized_leading_name_is_v0100_not_v0110() {
    // `regex` is not a module declared anywhere in this program and is
    // not one of the crate names this compiler recognizes by name alone:
    // it must not be misreported as a known crate.
    let d = errors_str("use regex::Regex;\n\nfn main() {}\n");
    let d = only(&d);
    assert_eq!(d.code, codes::V0100);
    assert!(!d.message.contains("crate"), "{d:#?}");
    assert!(d.message.contains("regex"), "{d:#?}");
    assert!(
        d.notes.iter().any(|n| n.contains("if `regex` is a crate")),
        "{d:#?}"
    );
}

#[test]
fn use_alias_reusing_a_reserved_name_is_v0103() {
    let d =
        errors_str("struct Cart {\n    n: i32,\n}\n\nuse crate::Cart as Vec;\n\nfn main() {}\n");
    let d = only(&d);
    assert_eq!(d.code, codes::V0103);
    assert!(d.message.contains("Vec"), "{d:#?}");
    assert!(d.message.contains("built-in"), "{d:#?}");
}

#[test]
fn use_of_a_bare_reserved_name_is_v0103_at_the_use_too() {
    // The struct's own declaration is already V0103 (a struct cannot be
    // named after a built-in type); the `use` line below, whose local
    // name is the same reserved word (no `as`), must be flagged on its
    // own account too, not silently registered because the declaration
    // already complained.
    let d = errors_str("struct Vec {\n    n: i32,\n}\n\nuse crate::Vec;\n\nfn main() {}\n");
    assert_eq!(d.len(), 2, "{d:#?}");
    assert!(d.iter().all(|x| x.code == codes::V0103), "{d:#?}");
}

#[test]
fn use_ending_at_an_enum_variant_is_v0111() {
    let d = errors_str("enum Shape {\n    Circle(f64),\n}\n\nuse Shape::Circle;\n\nfn main() {}\n");
    let d = only(&d);
    assert_eq!(d.code, codes::V0111);
    assert!(d.message.contains("Shape::Circle"), "{d:#?}");
}

#[test]
fn use_name_clashing_with_a_local_item_is_v0103() {
    let (diagnostics, sources) = error_fixture("v0103_use_name_clash");
    let d = only(&diagnostics);
    assert_eq!(d.code, codes::V0103);
    let span = span_of(&sources, 0, "use crate::shop::Cart;");
    assert_eq!(d.span, Span::new(span.file, span.end - 5, span.end - 1));
}

/// `use a::T;` imports every namespace `T` occupies in `a` (the emitted
/// Rust `use` does), so `a`'s struct `T` clashes with a local struct `T`
/// even though the `use` is only ever called as a function (rustc E0255).
#[test]
fn use_of_a_name_in_two_namespaces_clashes_with_a_local_struct() {
    let (diagnostics, sources) = fixture("use_both_namespaces_local");
    let d = only(&diagnostics);
    assert_eq!(d.code, codes::V0103);
    let span = span_of(&sources, 0, "use a::T;");
    assert_eq!(d.span, Span::new(span.file, span.end - 2, span.end - 1));
}

/// `use a::Foo; use b::Foo;` where `b` has a function and a struct `Foo`:
/// the two structs clash (rustc E0252).
#[test]
fn use_of_a_name_in_two_namespaces_clashes_with_another_use() {
    let (diagnostics, _) = fixture("use_both_namespaces_two");
    assert_eq!(only(&diagnostics).code, codes::V0103);
}

#[test]
fn use_of_an_associated_function_is_v0111_saying_to_call_it() {
    let (diagnostics, sources) = fixture("use_associated_fn");
    let d = only(&diagnostics);
    assert_eq!(d.code, codes::V0111);
    assert!(
        d.message.contains("write `use a::T;` and then `T::new()`"),
        "{d:#?}"
    );
    assert_eq!(d.span, span_of(&sources, 0, "T::new"));
}

/// A `use` path cannot start at a name another `use` made, whichever
/// comes first; the message spells the path meant.
#[test]
fn use_through_another_use_is_v0111_in_either_order() {
    for (dir, case) in [
        ("errors", "v0111_use_through_alias"),
        ("resolve", "use_through_alias_reversed"),
    ] {
        let (diagnostics, sources) = fixture_in(dir, case);
        let d = only(&diagnostics);
        assert_eq!(d.code, codes::V0111, "{case}");
        assert!(d.message.contains("write `use a::g as gg;`"), "{d:#?}");
        let written = span_of(&sources, 0, "aa::g");
        assert_eq!(
            d.span,
            Span::new(written.file, written.start, written.start + 2),
            "{case}"
        );
    }
}

/// A `use` through another `use` of a type goes on to a variant: the
/// advice is the variant's, not a joined path, which would name a
/// variant too (M3 round 5).
#[test]
fn use_through_a_type_alias_says_to_write_the_variant_through_the_type() {
    let (diagnostics, _) = fixture("use_through_type_alias");
    let d = only(&diagnostics);
    assert_eq!(d.code, codes::V0111);
    assert_eq!(
        d.message,
        "`use` cannot name a variant; `Shape` can already be used here, so write `Shape::Point`"
    );
}

/// A `use` through another `use` of a function suggests no path: there
/// is none to suggest.
#[test]
fn use_through_a_function_alias_suggests_no_path() {
    let (diagnostics, _) = fixture("use_through_fn_alias");
    let d = only(&diagnostics);
    assert_eq!(d.code, codes::V0111);
    assert!(
        d.message.contains("`g` is a name made by another `use`"),
        "{d:#?}"
    );
    assert!(!d.message.contains("write `use"), "{d:#?}");
}

/// A type a `use` of the file brings in, in either order, is in scope.
#[test]
fn use_of_a_variant_of_a_type_another_use_brings_in_says_it_is_in_scope() {
    let (diagnostics, _) = fixture("use_variant_of_used_type");
    let d = only(&diagnostics);
    assert_eq!(d.code, codes::V0111);
    assert_eq!(
        d.message,
        "`use` cannot name a variant; `Shape` can already be used here, so write `Shape::Point`"
    );
}

/// `self::` or, in the entry file, `crate::` before a name another `use`
/// made is the same mistake as the bare name.
#[test]
fn use_through_an_alias_after_self_or_crate_is_v0111() {
    for case in ["use_self_through_alias", "use_crate_through_alias"] {
        let (diagnostics, _) = fixture(case);
        let d = only(&diagnostics);
        assert_eq!(d.code, codes::V0111, "{case}");
        assert!(d.message.contains("write `use a::g;`"), "{case}: {d:#?}");
    }
}

#[test]
fn use_of_a_private_item_is_v0105() {
    let (diagnostics, _) = fixture("use_private_item");
    let d = only(&diagnostics);
    assert_eq!(d.code, codes::V0105);
}

#[test]
fn use_of_an_item_behind_a_private_module_is_v0105_naming_the_module() {
    let (diagnostics, _) = fixture("use_private_module");
    let d = only(&diagnostics);
    assert_eq!(d.code, codes::V0105);
    assert!(d.message.contains("`cart`"), "{d:#?}");
}

#[test]
fn use_aliasing_a_private_module_itself_is_v0105_naming_it() {
    let (diagnostics, _) = fixture("use_private_module_alias");
    let d = only(&diagnostics);
    assert_eq!(d.code, codes::V0105);
    assert!(d.message.contains("`cart`"), "{d:#?}");
}

#[test]
fn use_leading_name_declared_in_another_file_is_v0111_saying_to_write_crate() {
    let (diagnostics, sources) = fixture("use_leading_elsewhere");
    let d = only(&diagnostics);
    assert_eq!(d.code, codes::V0111);
    assert_eq!(d.span, span_of(&sources, 1, "util"));
    assert!(d.message.contains("crate"), "{d:#?}");
}

/// The suggested `use` keeps the rename the written one has.
#[test]
fn use_leading_name_declared_elsewhere_keeps_its_rename_in_the_fix() {
    let (diagnostics, sources) = fixture("use_leading_elsewhere_renamed");
    let d = only(&diagnostics);
    assert_eq!(d.code, codes::V0111);
    assert_eq!(d.span, span_of(&sources, 2, "shop"));
    assert!(
        d.notes
            .iter()
            .any(|n| n.contains("write `use crate::a::shop::Cart as C;`")),
        "{d:#?}"
    );
}

/// A bare `use shop;` of a module declared in another file is the same
/// V0111, with the `use` spelled from `crate::`.
#[test]
fn bare_use_of_a_module_declared_elsewhere_is_v0111_saying_to_write_crate() {
    let (diagnostics, sources) = fixture("use_bare_module_elsewhere");
    let d = only(&diagnostics);
    assert_eq!(d.code, codes::V0111);
    assert_eq!(d.span, span_of(&sources, 2, "shop"));
    assert!(
        d.notes
            .iter()
            .any(|n| n.contains("write `use crate::a::shop;`")),
        "{d:#?}"
    );
}

#[test]
fn module_defining_main_is_v0106() {
    let (diagnostics, sources) = fixture("module_main");
    let d = only(&diagnostics);
    assert_eq!(d.code, codes::V0106);
    assert_eq!(d.span.file, FileId(1));
    assert_eq!(d.span.start, span_of(&sources, 1, "fn main").start);
}

// --- Visibility ---------------------------------------------------------

#[test]
fn non_pub_struct_in_a_pub_signature_or_pub_struct_field_is_v0105() {
    let (result, sources) =
        resolve_file("crates/varyk/tests/fixtures/resolve/private_in_public/main.vr");
    let diagnostics = result.expect_err("private types in public items should fail");
    let spans: Vec<Span> = diagnostics.iter().map(|d| d.span).collect();
    let at = |needle: &str| {
        let span = span_of(&sources, 1, needle);
        Span::new(span.file, span.end - "Hidden".len() as u32, span.end)
    };
    assert_eq!(
        spans,
        vec![
            at("pub fn take(h: Hidden"),
            at("make() -> Hidden"),
            at("Shown {\n    pub h: Hidden")
        ]
    );
    for d in &diagnostics {
        assert_eq!(d.code, codes::V0105);
        assert_eq!(
            d.message,
            "`Hidden` is used by a public item but is not public; add `pub` to the struct"
        );
        assert_eq!(d.fix_it.as_ref().expect("fix-it").replacement, "pub ");
    }
}

#[test]
fn lookup_enforces_pub_across_modules() {
    let (result, sources) = resolve_file("crates/varyk/tests/fixtures/resolve/visibility/main.vr");
    let resolved = result.expect("visibility fixture should resolve");
    let symbols = &resolved.symbols;
    let math = module_id(&resolved, "math");

    let hidden = symbols.lookup_fn(resolved.entry, Some(&p("math")), "hidden");
    let expected = span_of(&sources, 1, "fn hidden() {}");
    assert_eq!(
        hidden,
        Err(LookupError::NotVisible {
            decl: expected,
            keyword: "fn"
        })
    );

    let shown = symbols
        .lookup_fn(resolved.entry, Some(&p("math")), "shown")
        .expect("pub fn should be visible");
    let Callee::Varyk(id) = shown else { panic!() };
    assert_eq!(
        symbols.fns[id.0 as usize].params,
        vec![
            (
                "n".to_string(),
                Ty::Int(IntKind::I32),
                ParamMode::MutableBorrow
            ),
            ("s".to_string(), Ty::String, ParamMode::SharedBorrow),
        ]
    );

    // Within its own module, a non-`pub` item is visible.
    assert!(symbols.lookup_fn(math, None, "hidden").is_ok());
    let holder = symbols.lookup_struct(math, "Holder").unwrap();
    let secret = symbols.lookup_struct(math, "Secret").unwrap();
    let fields = &symbols.structs[holder.0 as usize].fields;
    assert_eq!(fields.len(), 1);
    assert_eq!(fields[0].name, "s");
    assert_eq!(fields[0].ty, Ty::Struct(secret));
    assert!(!fields[0].is_pub);

    assert_eq!(
        symbols.lookup_fn(resolved.entry, Some(&p("nope")), "shown"),
        Err(LookupError::Unknown)
    );
    assert_eq!(
        symbols.lookup_fn(resolved.entry, Some(&p("math")), "nope"),
        Err(LookupError::Unknown)
    );
    assert_eq!(
        symbols.lookup_fn(resolved.entry, None, "shown"),
        Err(LookupError::Unknown)
    );
    assert!(symbols.lookup_fn(resolved.entry, None, "main").is_ok());
}

// --- Entry `main` -------------------------------------------------------

#[test]
fn entry_without_main_is_v0106_at_the_start_of_the_file() {
    let d = errors_str("fn helper() {}\n");
    let d = only(&d);
    assert_eq!(d.code, codes::V0106);
    assert_eq!(d.span, Span::new(FileId(0), 0, 0));
}

#[test]
fn main_with_parameters_is_v0106() {
    let d = errors_str("fn main(x: i32) {}\n");
    let d = only(&d);
    assert_eq!(d.code, codes::V0106);
    assert_eq!(d.span.start, 0);
}

#[test]
fn main_returning_a_value_is_v0106() {
    let d = errors_str("fn main() -> i32 { 0 }\n");
    assert_eq!(only(&d).code, codes::V0106);
}

// --- Types and duplicates -----------------------------------------------

#[test]
fn unknown_type_in_signature_or_field_is_v0101() {
    let text = "struct S {\n    a: Nope,\n}\n\nfn f(x: Missing) -> Gone {}\n\nfn main() {}\n";
    let (result, sources) = resolve_str(text);
    let diagnostics = result.expect_err("should fail");
    assert_eq!(diagnostics.len(), 3, "{diagnostics:#?}");
    for (d, name) in diagnostics.iter().zip(["Nope", "Missing", "Gone"]) {
        assert_eq!(d.code, codes::V0101);
        assert_eq!(d.span, span_of(&sources, 0, name));
    }
}

#[test]
fn an_unknown_module_in_a_type_path_is_v0101_naming_the_whole_path() {
    let d = errors_str("fn f(x: a::b::Cart, y: crate::Foo) {}\n\nfn main() {}\n");
    assert_eq!(d.len(), 2, "{d:#?}");
    assert!(d.iter().all(|d| d.code == codes::V0101), "{d:#?}");
    assert!(d[0].message.contains("`a::b::Cart`"), "{d:#?}");
    assert!(d[1].message.contains("`crate::Foo`"), "{d:#?}");
}

#[test]
fn unknown_type_matching_another_modules_type_is_v0101_with_a_did_you_mean_note() {
    let (diagnostics, sources) = fixture("unknown_type_other_module");
    let d = only(&diagnostics);
    assert_eq!(d.code, codes::V0101);
    let at = span_of(&sources, 0, "Item)");
    assert_eq!(d.span, Span::new(at.file, at.start, at.start + 4));
    assert!(
        d.notes.iter().any(|n| n == "did you mean `m::Item`?"),
        "{d:#?}"
    );
}

#[test]
fn duplicate_function_is_v0103_labelling_the_first() {
    let text = "fn f() {}\nfn f() {}\nfn main() {}\n";
    let diagnostics = errors_str(text);
    let d = only(&diagnostics);
    assert_eq!(d.code, codes::V0103);
    assert_eq!(d.span, Span::new(FileId(0), 13, 14));
    assert_eq!(d.labels.len(), 1);
    assert_eq!(d.labels[0].span, Span::new(FileId(0), 3, 4));
}

#[test]
fn enum_function_named_after_a_variant_is_v0103() {
    let text = "enum E {\n    Make(i32),\n    Nope,\n}\nimpl E {\n    fn Make(x: i32) -> E {\n        E::Nope\n    }\n}\nfn main() {}\n";
    let diagnostics = errors_str(text);
    let d = only(&diagnostics);
    assert_eq!(d.code, codes::V0103);
    assert!(d.message.contains("a variant of `E`"), "{}", d.message);
    assert_eq!(&text[d.span.start as usize..d.span.end as usize], "Make");
}

#[test]
fn duplicate_struct_is_v0103() {
    let d = errors_str("struct S {}\nstruct S {}\nfn main() {}\n");
    assert_eq!(only(&d).code, codes::V0103);
}

#[test]
fn struct_named_like_a_module_is_v0103_labelling_the_mod() {
    let (diagnostics, sources) = fixture("mod_struct_clash");
    let d = only(&diagnostics);
    assert_eq!(d.code, codes::V0103);
    let at_struct = span_of(&sources, 0, "struct foo");
    assert_eq!(
        d.span,
        Span::new(FileId(0), at_struct.start + 7, at_struct.end)
    );
    assert_eq!(d.labels.len(), 1);
    let at_mod = span_of(&sources, 0, "mod foo");
    assert_eq!(
        d.labels[0].span,
        Span::new(FileId(0), at_mod.start + 4, at_mod.end)
    );
}

#[test]
fn indirectly_recursive_structs_are_one_v0109_at_the_closing_field() {
    let text = "struct A {\n    b: B,\n}\nstruct B {\n    a: A,\n}\nfn main() {}\n";
    let (result, sources) = resolve_str(text);
    let diagnostics = result.expect_err("should fail");
    let d = only(&diagnostics);
    assert_eq!(d.code, codes::V0109);
    assert_eq!(d.span, span_of(&sources, 0, "a: A"));
    assert_eq!(
        d.notes,
        vec![
            "the cycle is A -> B -> A".to_string(),
            ELSEWHERE_NOTE.to_string()
        ]
    );
}

#[test]
fn a_struct_field_of_another_struct_is_not_recursive() {
    let text = "struct A {\n    b: B,\n}\nstruct B {\n    x: i32,\n}\nfn main() {}\n";
    assert!(resolve_str(text).0.is_ok());
}

#[test]
fn capital_string_and_str_are_v0107_with_a_fix_it_to_string() {
    let text = "fn f(a: String, b: str) {}\nfn main() {}\n";
    let (result, sources) = resolve_str(text);
    let diagnostics = result.expect_err("should fail");
    assert_eq!(diagnostics.len(), 2, "{diagnostics:#?}");
    for (d, name) in diagnostics.iter().zip(["String", "str"]) {
        assert_eq!(d.code, codes::V0107);
        let span = span_of(&sources, 0, &format!(": {name}"));
        let span = Span::new(span.file, span.start + 2, span.end);
        assert_eq!(d.span, span);
        let fix = d.fix_it.as_ref().expect("fix-it");
        assert_eq!(fix.span, span);
        assert_eq!(fix.replacement, "string");
    }
}

#[test]
fn param_modes_follow_copy_and_mut() {
    let text =
        "struct U {\n    n: i32,\n}\nfn f(a: bool, b: U, mut c: U, mut d: f64) {}\nfn main() {}\n";
    let (result, _) = resolve_str(text);
    let resolved = result.expect("should resolve");
    let Ok(Callee::Varyk(id)) = resolved.symbols.lookup_fn(resolved.entry, None, "f") else {
        panic!()
    };
    let modes: Vec<ParamMode> = resolved.symbols.fns[id.0 as usize]
        .params
        .iter()
        .map(|p| p.2)
        .collect();
    assert_eq!(
        modes,
        vec![
            ParamMode::Owned,
            ParamMode::SharedBorrow,
            ParamMode::MutableBorrow,
            ParamMode::MutableBorrow,
        ]
    );
}

#[test]
fn syntax_errors_stop_resolution() {
    let d = errors_str("fn main( {}\n");
    assert!(d.iter().all(|d| d.code != codes::V0106), "{d:#?}");
    assert!(!d.is_empty());
}

// --- Enums, impl blocks, and generic types ------------------------------

fn resolved(text: &str) -> Resolved {
    match resolve_str(text).0 {
        Ok(resolved) => resolved,
        Err(diagnostics) => panic!("expected {text:?} to resolve, got {diagnostics:#?}"),
    }
}

fn member<'a>(resolved: &'a Resolved, owner: UserType, name: &str) -> (FnId, &'a FnSig) {
    let found = resolved
        .symbols
        .lookup_member(resolved.entry, owner, name)
        .unwrap_or_else(|e| panic!("no member {name}: {e:?}"));
    let Callee::Varyk(id) = found else {
        panic!("{name} is not a Varyk member: {found:?}");
    };
    (id, &resolved.symbols.fns[id.0 as usize])
}

const I32: Ty = Ty::Int(IntKind::I32);
const F64: Ty = Ty::Float(FloatKind::F64);

#[test]
fn enum_and_impl_register() {
    let r = resolved(
        "enum Shape {\n    Circle(f64),\n    Rect(f64, string),\n    Point,\n}\n\nstruct Counter {\n    count: i32,\n}\n\nimpl Counter {\n    fn new() -> Counter {\n        Counter { count: 0 }\n    }\n\n    fn add(mut self, by: i32) {\n        self.count = self.count + by;\n    }\n\n    fn value(self) -> i32 {\n        self.count\n    }\n}\n\nfn main() {}\n",
    );
    let symbols = &r.symbols;
    let Ok(UserType::Enum(shape)) = symbols.lookup_type(r.entry, None, "Shape") else {
        panic!("Shape should be an enum");
    };
    let def = &symbols.enums[shape.0 as usize];
    assert_eq!(
        def.variants,
        vec![
            VariantDef {
                name: "Circle".to_string(),
                fields: VariantFieldsDef::Tuple(vec![F64]),
                rename: None,
            },
            VariantDef {
                name: "Rect".to_string(),
                fields: VariantFieldsDef::Tuple(vec![F64, Ty::String]),
                rename: None,
            },
            VariantDef {
                name: "Point".to_string(),
                fields: VariantFieldsDef::Tuple(vec![]),
                rename: None,
            },
        ]
    );
    assert_eq!(def.variant("Point"), Some(2));
    assert_eq!(def.variant("Square"), None);

    let counter = symbols
        .lookup_type(r.entry, None, "Counter")
        .expect("Counter");
    let (_, new) = member(&r, counter, "new");
    assert_eq!(
        (new.owner, new.self_mode, &new.ret),
        (Some(counter), None, &counter.ty())
    );
    let (add_id, add) = member(&r, counter, "add");
    assert_eq!(add.self_mode, Some(ParamMode::MutableBorrow));
    assert_eq!(add.params, vec![("by".to_string(), I32, ParamMode::Owned)]);
    assert_eq!(r.fn_decl(add_id).name.name, "add");
    let (_, value) = member(&r, counter, "value");
    assert_eq!(
        (value.self_mode, &value.ret),
        (Some(ParamMode::SharedBorrow), &I32)
    );

    // Methods and associated functions are not free functions.
    assert_eq!(
        symbols.lookup_fn(r.entry, None, "new"),
        Err(LookupError::Unknown)
    );
    assert_eq!(
        symbols.lookup_member(r.entry, counter, "nope"),
        Err(LookupError::Unknown)
    );
}

#[test]
fn two_impl_blocks_for_one_type_merge() {
    let r = resolved(
        "struct C {\n    n: i32,\n}\nimpl C {\n    fn a(self) {}\n}\nimpl C {\n    fn b(self) {}\n}\nfn main() {}\n",
    );
    let c = r.symbols.lookup_type(r.entry, None, "C").expect("C");
    assert_eq!(member(&r, c, "a").1.owner, Some(c));
    assert_eq!(member(&r, c, "b").1.owner, Some(c));
}

#[test]
fn a_method_defined_twice_across_impl_blocks_is_v0103() {
    let text = "struct C {\n    n: i32,\n}\nimpl C {\n    fn a(self) {}\n}\nimpl C {\n    fn a() {}\n}\nfn main() {}\n";
    let (result, sources) = resolve_str(text);
    let diagnostics = result.expect_err("should fail");
    let d = only(&diagnostics);
    assert_eq!(d.code, codes::V0103);
    let second = span_of(&sources, 0, "a() {}");
    assert_eq!(d.span, Span::new(FileId(0), second.start, second.start + 1));
    let first = span_of(&sources, 0, "a(self)");
    assert_eq!(
        d.labels[0].span,
        Span::new(FileId(0), first.start, first.start + 1)
    );
}

#[test]
fn an_impl_on_an_enum_registers() {
    let r = resolved(
        "enum Shape {\n    Point,\n}\nimpl Shape {\n    fn name(self) -> string {\n        \"point\"\n    }\n}\nfn main() {}\n",
    );
    let shape = r
        .symbols
        .lookup_type(r.entry, None, "Shape")
        .expect("Shape");
    assert!(matches!(shape, UserType::Enum(_)));
    let (_, name) = member(&r, shape, "name");
    assert_eq!(
        (name.owner, name.self_mode, &name.ret),
        (Some(shape), Some(ParamMode::SharedBorrow), &Ty::String)
    );
}

#[test]
fn an_impl_for_a_type_not_in_the_file_is_v0001() {
    let text = "impl Nope {\n    fn f() {}\n}\nfn main() {}\n";
    let (result, sources) = resolve_str(text);
    let diagnostics = result.expect_err("should fail");
    let d = only(&diagnostics);
    assert_eq!(d.code, codes::V0001);
    assert_eq!(d.span, span_of(&sources, 0, "Nope"));

    let (diagnostics, sources) = fixture("impl_other_module");
    let d = only(&diagnostics);
    assert_eq!(d.code, codes::V0001);
    assert_eq!(d.span, span_of(&sources, 0, "Item"));
}

#[test]
fn an_enum_containing_itself_through_option_is_v0109_and_through_vec_is_not() {
    let text = "enum L {\n    C(Option<L>),\n    End,\n}\nfn main() {}\n";
    let (result, sources) = resolve_str(text);
    let diagnostics = result.expect_err("should fail");
    let d = only(&diagnostics);
    assert_eq!(d.code, codes::V0109);
    assert_eq!(d.span, span_of(&sources, 0, "Option<L>"));
    assert_eq!(d.message, "an enum cannot contain itself");
    assert_eq!(
        d.notes,
        vec![
            "the cycle is L -> L".to_string(),
            ELSEWHERE_NOTE.to_string()
        ]
    );

    resolved("enum T {\n    C(Vec<T>),\n    Leaf,\n}\nfn main() {}\n");
}

/// The note every V0109 carries after the cycle (M4 spec 2.7).
const ELSEWHERE_NOTE: &str = "a `Vec` or `HashMap` holds its elements elsewhere, so holding \
                              the value in one breaks the cycle";

#[test]
fn a_named_field_variant_containing_itself_is_v0109() {
    let text = "enum L {\n    C { next: Option<L> },\n    End,\n}\nfn main() {}\n";
    let (result, sources) = resolve_str(text);
    let diagnostics = result.expect_err("should fail");
    let d = only(&diagnostics);
    assert_eq!(d.code, codes::V0109);
    assert_eq!(d.span, span_of(&sources, 0, "Option<L>"));
    assert_eq!(
        d.notes,
        vec![
            "the cycle is L -> L".to_string(),
            ELSEWHERE_NOTE.to_string()
        ]
    );
}

#[test]
fn a_hash_map_payload_breaks_a_cycle_as_a_vec_does() {
    resolved("struct Node {\n    children: HashMap<string, Node>,\n}\nfn main() {}\n");
    resolved("enum T {\n    C(HashMap<i32, T>),\n    Leaf,\n}\nfn main() {}\n");
}

#[test]
fn hash_map_resolves_with_an_integer_bool_or_string_key() {
    let resolved = resolved(
        "fn f(a: HashMap<string, i32>, b: HashMap<u8, Vec<string>>, c: HashMap<bool, HashMap<i64, f64>>) {}\nfn main() {}\n",
    );
    let (_, sig) = resolved
        .symbols
        .fns
        .iter()
        .enumerate()
        .find(|(_, sig)| sig.name == "f")
        .expect("f");
    let boxed = |ty: Ty| Box::new(ty);
    assert_eq!(
        sig.params[0].1,
        Ty::HashMap(boxed(Ty::String), boxed(Ty::Int(IntKind::I32)))
    );
    assert_eq!(
        sig.params[1].1,
        Ty::HashMap(
            boxed(Ty::Int(IntKind::U8)),
            boxed(Ty::Vec(boxed(Ty::String)))
        )
    );
    assert_eq!(
        sig.params[2].1,
        Ty::HashMap(
            boxed(Ty::Bool),
            boxed(Ty::HashMap(
                boxed(Ty::Int(IntKind::I64)),
                boxed(Ty::Float(FloatKind::F64))
            ))
        )
    );
}

#[test]
fn hash_map_with_a_wrong_arity_or_key_is_v0101() {
    for (ty, words) in [
        (
            "HashMap<string>",
            "`HashMap` takes two types, written `HashMap<K, V>`",
        ),
        ("HashMap<string, i32, i32>", "`HashMap` takes two types"),
        ("HashMap", "`HashMap` takes two types"),
        ("HashMap<f64, i32>", "an integer type, `bool`, or `string`"),
        (
            "HashMap<Vec<i32>, i32>",
            "an integer type, `bool`, or `string`",
        ),
        ("HashMap<P, i32>", "an integer type, `bool`, or `string`"),
    ] {
        let text = format!("struct P {{\n    n: i32,\n}}\nfn f(m: {ty}) {{}}\nfn main() {{}}\n");
        let (result, sources) = resolve_str(&text);
        let diagnostics = result.expect_err("should fail");
        let d = only(&diagnostics);
        assert_eq!(d.code, codes::V0101, "{ty}");
        assert!(d.message.contains(words), "{ty}: {d:#?}");
        if ty == "HashMap<f64, i32>" {
            assert_eq!(d.span, span_of(&sources, 0, "f64"));
        }
    }
}

#[test]
fn hash_map_declared_as_a_name_is_v0103() {
    for text in [
        "struct HashMap {}\nfn main() {}\n",
        "enum HashMap {\n    A,\n}\nfn main() {}\n",
        "fn HashMap() {}\nfn main() {}\n",
    ] {
        let d = errors_str(text);
        let d = only(&d);
        assert_eq!(d.code, codes::V0103, "{text}");
        assert_eq!(
            d.message, "the name `HashMap` is already taken by a built-in type",
            "{text}"
        );
    }
}

#[test]
fn a_struct_and_an_enum_containing_each_other_through_result_are_one_v0109() {
    let text = "struct S {\n    r: Result<i32, E>,\n}\nenum E {\n    Wrap(S),\n}\nfn main() {}\n";
    let (result, sources) = resolve_str(text);
    let diagnostics = result.expect_err("should fail");
    let d = only(&diagnostics);
    assert_eq!(d.code, codes::V0109);
    let payload = span_of(&sources, 0, "S),");
    assert_eq!(
        d.span,
        Span::new(payload.file, payload.start, payload.start + 1)
    );
    assert_eq!(
        d.notes,
        vec![
            "the cycle is S -> E -> S".to_string(),
            ELSEWHERE_NOTE.to_string()
        ]
    );
}

#[test]
fn reserved_names_in_each_declaration_position_are_v0103() {
    let text = "mod String;\nfn Some() {}\nenum Option {\n    A,\n}\nenum E {\n    Ok,\n}\nstruct None {}\nstruct F {\n    Vec: i32,\n}\nimpl F {\n    fn Err(self) {}\n}\nfn main() {}\n";
    let (result, sources) = resolve_str(text);
    let diagnostics = result.expect_err("should fail");
    let mut spans: Vec<Span> = diagnostics.iter().map(|d| d.span).collect();
    spans.sort_by_key(|span| span.start);
    let expected: Vec<Span> = ["String", "Some", "Option", "Ok", "None", "Vec", "Err"]
        .iter()
        .map(|name| span_of(&sources, 0, name))
        .collect();
    assert_eq!(spans, expected, "{diagnostics:#?}");
    assert!(diagnostics.iter().all(|d| d.code == codes::V0103));
    let some = diagnostics
        .iter()
        .find(|d| d.span == expected[1])
        .expect("Some");
    assert_eq!(some.message, "the name `Some` is already taken by `Option`");
    let option = diagnostics
        .iter()
        .find(|d| d.span == expected[2])
        .expect("Option");
    assert_eq!(
        option.message,
        "the name `Option` is already taken by a built-in type"
    );
}

#[test]
fn option_result_and_vec_resolve_with_the_right_number_of_type_arguments() {
    let r = resolved(
        "struct U {\n    n: i32,\n}\nfn f(x: Vec<Option<Result<U, string>>>, n: usize) -> Option<usize> {\n    None\n}\nfn main() {}\n",
    );
    let Ok(Callee::Varyk(id)) = r.symbols.lookup_fn(r.entry, None, "f") else {
        panic!()
    };
    let sig = &r.symbols.fns[id.0 as usize];
    let u = Ty::Struct(r.symbols.lookup_struct(r.entry, "U").expect("U"));
    let usize = Ty::Int(IntKind::Usize);
    let x = Ty::Vec(Box::new(Ty::Option(Box::new(Ty::Result(
        Box::new(u),
        Box::new(Ty::String),
    )))));
    assert_eq!(
        sig.params,
        vec![
            ("x".to_string(), x, ParamMode::SharedBorrow),
            ("n".to_string(), usize.clone(), ParamMode::Owned),
        ]
    );
    assert_eq!(sig.ret, Ty::Option(Box::new(usize)));
}

#[test]
fn option_result_or_vec_with_the_wrong_number_of_type_arguments_is_v0101() {
    let text = "fn f(a: Result<i32>, b: Option<i32, i32>, c: Vec) {}\nfn main() {}\n";
    let (result, sources) = resolve_str(text);
    let diagnostics = result.expect_err("should fail");
    assert_eq!(diagnostics.len(), 3, "{diagnostics:#?}");
    let expected = [
        (
            "Result<i32>",
            "`Result` takes two types, written `Result<T, E>`",
        ),
        (
            "Option<i32, i32>",
            "`Option` takes one type, written `Option<T>`",
        ),
        ("Vec)", "`Vec` takes one type, written `Vec<T>`"),
    ];
    for (d, (needle, message)) in diagnostics.iter().zip(expected) {
        assert_eq!(d.code, codes::V0101);
        let span = span_of(&sources, 0, needle);
        let len = needle.trim_end_matches(')').len() as u32;
        assert_eq!(d.span, Span::new(span.file, span.start, span.start + len));
        assert_eq!(d.message, message);
    }
}

#[test]
fn a_module_type_in_a_signature_resolves_and_needs_pub() {
    let (result, _) = resolve_file("crates/varyk/tests/fixtures/resolve/module_types/main.vr");
    let r = result.expect("module_types should resolve");
    let symbols = &r.symbols;
    let task = symbols
        .lookup_type(r.entry, Some(&p("m")), "Item")
        .expect("m::Item");
    let shape = symbols
        .lookup_type(r.entry, Some(&p("m")), "Shape")
        .expect("m::Shape");
    let Ok(Callee::Varyk(take)) = symbols.lookup_fn(r.entry, None, "take") else {
        panic!()
    };
    let types: Vec<Ty> = symbols.fns[take.0 as usize]
        .params
        .iter()
        .map(|p| p.1.clone())
        .collect();
    assert_eq!(types, vec![task.ty(), shape.ty()]);
    let UserType::Enum(shape_id) = shape else {
        panic!("m::Shape is an enum")
    };
    assert_eq!(
        symbols.enums[shape_id.0 as usize].variant("Circle"),
        Some(1)
    );

    // `m::Item::new` is `pub`; `secret` is not.
    let (_, new) = member(&r, task, "new");
    assert_eq!(new.name, "new");
    assert!(matches!(
        symbols.lookup_member(r.entry, task, "secret"),
        Err(LookupError::NotVisible { keyword: "fn", .. })
    ));
    let m = module_id(&r, "m");
    assert!(symbols.lookup_member(m, task, "secret").is_ok());
    assert!(matches!(
        symbols.lookup_type(r.entry, Some(&p("m")), "Hidden"),
        Err(LookupError::NotVisible {
            keyword: "struct",
            ..
        })
    ));

    let (diagnostics, sources) = fixture("module_type_private");
    let d = only(&diagnostics);
    assert_eq!(d.code, codes::V0105);
    assert_eq!(d.span, span_of(&sources, 0, "m::Hidden"));
    let fix = d.fix_it.as_ref().expect("fix-it");
    assert_eq!(fix.span.file, FileId(1));
    assert_eq!(fix.span.start, span_of(&sources, 1, "struct Hidden").start);
    assert_eq!(fix.replacement, "pub ");
}

#[test]
fn a_private_type_inside_a_public_enum_or_generic_type_is_v0105() {
    let (result, sources) =
        resolve_file("crates/varyk/tests/fixtures/resolve/private_in_public_generic/main.vr");
    let diagnostics = result.expect_err("should fail");
    let spans: Vec<Span> = diagnostics.iter().map(|d| d.span).collect();
    let hidden = span_of(&sources, 1, "Hidden)");
    let hidden = Span::new(hidden.file, hidden.start, hidden.start + 6);
    assert_eq!(spans, vec![hidden, span_of(&sources, 1, "Option<Hidden>")]);
    assert!(diagnostics.iter().all(|d| d.code == codes::V0105));
}

#[test]
fn a_pub_item_may_name_a_private_type_its_users_can_all_see() {
    // A private type of the crate root is visible everywhere.
    resolved(
        "struct Hidden {}\npub enum E {\n    A(Hidden),\n}\npub fn f(x: Option<Hidden>) {}\nfn main() {}\n",
    );
    for case in [
        "reach_root_type",
        "reach_parent_type",
        "reach_private_owner",
    ] {
        let (result, _) = resolve_file(&format!(
            "crates/varyk/tests/fixtures/resolve/{case}/main.vr"
        ));
        if let Err(d) = result {
            panic!("{case} should resolve: {d:#?}");
        }
    }
}

#[test]
fn usize_imports_and_resolves() {
    let fns = import_rust_module("pub fn f(n: usize, r: &usize, m: &mut usize) -> usize { n }")
        .expect("parses");
    let symbols = Symbols::default();
    let sig = signatures::Mapper::new(&symbols, ModuleId(1))
        .sig(None, fns.fns.into_iter().next().expect("one fn"));
    let usize = Ty::Int(IntKind::Usize);
    assert!(sig.callable);
    assert_eq!(
        sig.params,
        vec![
            (usize.clone(), ParamMode::Owned),
            (usize.clone(), ParamMode::SharedBorrow),
            (usize.clone(), ParamMode::MutableBorrow),
        ]
    );
    assert_eq!(sig.ret, usize);
}

// --- `varyk_std::Error` in a signature (milestone 5b3 spec 2.4) -------

/// The one function of `.rs` text `text`, mapped in a package whose only
/// dependency is `varyk-std`.
fn mapped(text: &str) -> ImportedSig {
    let crates = ["varyk-std".to_string()];
    let imported = crate::interop::import_rust_module_in(text, Some(&crates)).expect("parses");
    let symbols = Symbols::default();
    signatures::Mapper::new(&symbols, ModuleId(1))
        .sig(None, imported.fns.into_iter().next().expect("one fn"))
}

#[test]
fn a_result_with_varyk_std_error_is_a_result_of_error() {
    let sig = mapped("pub fn load(id: i64) -> Result<i64, varyk_std::Error> { Ok(id) }");
    assert!(sig.callable, "{:?}", sig.note);
    assert_eq!(
        sig.ret,
        Ty::Result(Box::new(Ty::Int(IntKind::I64)), Box::new(Ty::Error))
    );
    assert!(sig.names_std);
    let sig = mapped("pub fn all() -> Result<Vec<String>, varyk_std::Error> { Ok(Vec::new()) }");
    assert!(sig.callable, "{:?}", sig.note);
    assert_eq!(
        sig.ret,
        Ty::Result(Box::new(Ty::Vec(Box::new(Ty::String))), Box::new(Ty::Error))
    );
    assert!(sig.names_std);
}

#[test]
fn a_signature_without_varyk_std_does_not_name_it() {
    let sig = mapped("pub fn load(id: i64) -> Result<i64, String> { Ok(id) }");
    assert!(sig.callable);
    assert!(!sig.names_std);
}

#[test]
fn varyk_std_error_anywhere_else_is_not_callable_and_says_where_it_can_be() {
    for text in [
        "pub fn f(e: varyk_std::Error) -> bool { true }",
        "pub fn f(e: &varyk_std::Error) -> bool { true }",
        "pub fn f(e: Option<varyk_std::Error>) -> bool { true }",
        "pub fn f() -> varyk_std::Error { todo!() }",
        "pub fn f() -> Option<varyk_std::Error> { None }",
        "pub fn f() -> Result<varyk_std::Error, String> { todo!() }",
        "pub fn f() -> Result<i64, Vec<varyk_std::Error>> { Ok(1) }",
        "pub fn f(r: Result<i64, varyk_std::Error>) -> bool { true }",
        "pub fn f() -> Option<Result<i64, varyk_std::Error>> { None }",
        "pub fn f() -> Result<Result<i64, varyk_std::Error>, String> { Ok(Ok(1)) }",
    ] {
        let sig = mapped(text);
        assert!(!sig.callable, "{text}");
        let note = sig.note.unwrap_or_default();
        assert!(
            note.contains("`varyk_std::Error` can only be the error of a returned `Result`"),
            "{text}: {note}"
        );
    }
}

#[test]
fn a_static_str_parameter_takes_only_literal_text() {
    let sig = mapped("pub fn one(query: &'static str, id: i64) -> i64 { id }");
    assert!(sig.callable, "{:?}", sig.note);
    assert_eq!(
        sig.params,
        vec![
            (Ty::String, ParamMode::SharedBorrow),
            (Ty::Int(IntKind::I64), ParamMode::Owned)
        ]
    );
    assert_eq!(sig.literal, vec![true, false]);
    assert!(!sig.names_std);
    let sig = mapped("pub fn show(text: &str) {}");
    assert_eq!(sig.literal, vec![false]);
}

#[test]
fn a_static_str_anywhere_else_is_not_callable_and_says_where_it_can_be() {
    for text in [
        "pub fn f() -> &'static str { \"\" }",
        "pub fn f() -> Option<&'static str> { None }",
        "pub fn f(q: Option<&'static str>) {}",
        "pub fn f(q: Vec<&'static str>) {}",
    ] {
        let sig = mapped(text);
        assert!(!sig.callable, "{text}");
        assert!(sig.literal.is_empty(), "{text}");
        let note = sig.note.unwrap_or_default();
        assert!(
            note.contains("`&'static str` can only be a parameter"),
            "{text}: {note}"
        );
    }
}

#[test]
fn a_last_vec_of_values_makes_the_signature_variadic() {
    let sig = mapped("pub fn run(query: &'static str, values: Vec<varyk_std::Value>) -> i64 { 1 }");
    assert!(sig.callable, "{:?}", sig.note);
    assert!(sig.variadic);
    assert_eq!(sig.params, vec![(Ty::String, ParamMode::SharedBorrow)]);
    assert_eq!(sig.literal, vec![true]);
    assert!(sig.names_std);
    let sig = mapped("pub fn run(values: Vec<varyk_std::Value>) {}");
    assert!(sig.callable, "{:?}", sig.note);
    assert!(sig.variadic);
    assert!(sig.params.is_empty());
    let sig = mapped("pub fn run(id: i64) {}");
    assert!(!sig.variadic);
    assert!(!sig.names_std);
}

#[test]
fn values_anywhere_else_is_not_callable_and_says_where_they_can_be() {
    for text in [
        "pub fn f(values: Vec<varyk_std::Value>, id: i64) {}",
        "pub fn f(value: varyk_std::Value) {}",
        "pub fn f(value: &varyk_std::Value) {}",
        "pub fn f(value: Option<varyk_std::Value>) {}",
        "pub fn f() -> Vec<varyk_std::Value> { Vec::new() }",
        "pub fn f() -> varyk_std::Value { todo!() }",
    ] {
        let sig = mapped(text);
        assert!(!sig.callable, "{text}");
        assert!(!sig.variadic, "{text}");
        // Uncallable, but the module still needs `varyk-std` to compile.
        assert!(sig.names_std, "{text}");
        let note = sig.note.unwrap_or_default();
        assert!(
            note.contains("`Vec<varyk_std::Value>` can only be the last parameter"),
            "{text}: {note}"
        );
    }
}

#[test]
fn a_unit_result_with_varyk_std_error_stays_unsupported() {
    let sig = mapped("pub fn save() -> Result<(), varyk_std::Error> { Ok(()) }");
    assert!(!sig.callable);
    let note = sig.note.unwrap_or_default();
    assert!(note.contains("`()` inside another type"), "{note}");
}

#[test]
fn a_bare_error_from_a_use_says_to_write_the_full_path() {
    let sig = mapped("use varyk_std::Error;\npub fn f() -> Result<i64, Error> { Ok(1) }");
    assert!(!sig.callable);
    let note = sig.note.unwrap_or_default();
    assert!(note.contains("write the full path"), "{note}");
}

// --- Private modules (spec 3.2) -----------------------------------------

fn private_module() -> Resolved {
    let (result, _) = resolve_file("crates/varyk/tests/fixtures/resolve/private_module/main.vr");
    result.unwrap_or_else(|d| panic!("private_module should resolve: {d:#?}"))
}

#[test]
fn a_pub_item_in_a_private_module_used_from_the_root_is_not_visible_at_the_module() {
    let r = private_module();
    let cart = module_id(&r, "cart");
    let other = module_id(&r, "other");
    let module = LookupError::PrivateModule { module: cart };
    assert_eq!(
        r.symbols
            .lookup_fn(r.entry, Some(&p("shop::cart")), "count"),
        Err(module)
    );
    assert_eq!(
        r.symbols
            .lookup_type(r.entry, Some(&p("crate::shop::cart")), "Cart")
            .map(|_| ()),
        Err(module)
    );
    assert_eq!(
        r.symbols
            .lookup_fn(other, Some(&p("crate::shop::cart")), "count"),
        Err(module)
    );
    // An imported Rust function is `pub`, but its module is not.
    let fast = module_id(&r, "fast");
    assert_eq!(
        r.symbols.lookup_fn(r.entry, Some(&p("shop::fast")), "go"),
        Err(LookupError::PrivateModule { module: fast })
    );
    let shop = module_id(&r, "shop");
    assert!(r.symbols.lookup_fn(shop, Some(&p("fast")), "go").is_ok());
}

#[test]
fn a_private_module_and_a_private_item_are_visible_below_their_module() {
    let r = private_module();
    let shop = module_id(&r, "shop");
    let cart = module_id(&r, "cart");
    let open = module_id(&r, "open");
    // `cart` from its parent, and from a sibling inside that parent.
    assert!(r.symbols.lookup_fn(shop, Some(&p("cart")), "count").is_ok());
    assert!(
        r.symbols
            .lookup_fn(open, Some(&p("super::cart")), "count")
            .is_ok()
    );
    // A private item of `shop` from its child.
    assert!(
        r.symbols
            .lookup_fn(cart, Some(&p("super")), "secret")
            .is_ok()
    );
    assert!(
        r.symbols
            .lookup_fn(cart, Some(&p("crate::shop")), "secret")
            .is_ok()
    );
}

#[test]
fn a_private_item_used_from_a_sibling_module_is_not_visible() {
    let r = private_module();
    let other = module_id(&r, "other");
    assert!(matches!(
        r.symbols
            .lookup_fn(other, Some(&p("crate::shop")), "secret"),
        Err(LookupError::NotVisible { keyword: "fn", .. })
    ));
}

#[test]
fn a_pub_mod_chain_is_visible_from_anywhere() {
    let r = private_module();
    let other = module_id(&r, "other");
    assert!(
        r.symbols
            .lookup_fn(r.entry, Some(&p("shop::open")), "hello")
            .is_ok()
    );
    assert!(
        r.symbols
            .lookup_fn(other, Some(&p("crate::shop::open")), "hello")
            .is_ok()
    );
}

#[test]
fn a_pub_method_of_a_type_in_a_private_module_is_not_visible_at_the_module() {
    let r = private_module();
    let cart = module_id(&r, "cart");
    let shop = module_id(&r, "shop");
    let owner = r
        .symbols
        .lookup_type(shop, Some(&p("cart")), "Cart")
        .expect("visible from shop");
    assert_eq!(
        r.symbols.lookup_member(r.entry, owner, "new"),
        Err(LookupError::PrivateModule { module: cart })
    );
    assert!(r.symbols.lookup_member(shop, owner, "new").is_ok());
}

#[test]
fn a_type_behind_a_private_module_is_v0105_with_a_pub_mod_fix_it() {
    let (diagnostics, sources) = fixture("private_module_type");
    let d = only(&diagnostics);
    assert_eq!(d.code, codes::V0105);
    assert_eq!(d.span, span_of(&sources, 0, "shop::cart::Cart"));
    assert_eq!(
        d.message,
        "module `cart` is private to `shop`, so type `shop::cart::Cart` cannot be used here; \
         write `pub mod cart;`"
    );
    let fix = d.fix_it.as_ref().expect("fix-it");
    assert_eq!(fix.span, Span::new(FileId(1), 0, 0));
    assert_eq!(fix.replacement, "pub ");
    assert_eq!(d.labels[0].span, span_of(&sources, 1, "mod"));
}

#[test]
fn a_pub_fn_returning_a_type_its_callers_cannot_name_is_v0105_at_the_module() {
    let (diagnostics, sources) = fixture("private_interface");
    let d = only(&diagnostics);
    assert_eq!(d.code, codes::V0105);
    assert_eq!(d.span, span_of(&sources, 1, "Vec<cart::Cart>"));
    assert_eq!(
        d.message,
        "`Cart` is in module `cart`, which only `shop` can see, but `carts` can be used from \
         outside `shop`; write `pub mod cart;` (or make `carts` private)"
    );
    let fix = d.fix_it.as_ref().expect("fix-it");
    assert_eq!(fix.span, Span::new(FileId(1), 0, 0));
    assert_eq!(fix.replacement, "pub ");
    assert!(
        d.notes.iter().any(|n| n.contains("private_interfaces")),
        "{d:#?}"
    );
}

// --- Imported structs and methods (M3 spec 4.1, 4.2, 4.4) ----------------

fn rust_structs() -> Resolved {
    let (result, _) = resolve_file("crates/varyk/tests/fixtures/resolve/rust_structs/main.vr");
    result.unwrap_or_else(|d| panic!("rust_structs should resolve: {d:#?}"))
}

/// The imported struct `name` of module `module` (by its `mod` name).
fn imported_struct<'a>(
    resolved: &'a Resolved,
    module: &str,
    name: &str,
) -> (StructId, &'a StructDef) {
    let at = p(module);
    let Ok(UserType::Struct(id)) = resolved
        .symbols
        .lookup_type(resolved.entry, Some(&at), name)
    else {
        panic!("no struct {module}::{name}");
    };
    (id, &resolved.symbols.structs[id.0 as usize])
}

fn imported_member<'a>(resolved: &'a Resolved, id: StructId, name: &str) -> &'a ImportedSig {
    match resolved
        .symbols
        .lookup_member(resolved.entry, UserType::Struct(id), name)
    {
        Ok(Callee::Imported(found)) => &resolved.symbols.imported[found.0 as usize],
        other => panic!("{name} is not an imported member: {other:?}"),
    }
}

fn struct_field<'a>(def: &'a StructDef, name: &str) -> &'a FieldDef {
    def.fields
        .iter()
        .find(|f| f.name == name)
        .unwrap_or_else(|| panic!("no field {name}"))
}

#[test]
fn imported_struct_is_in_the_struct_table_with_field_visibility() {
    let resolved = rust_structs();
    let (_, def) = imported_struct(&resolved, "ext", "Matcher");
    assert!(def.imported && def.is_pub);
    let label = struct_field(def, "label");
    assert_eq!(
        (&label.ty, label.is_pub, &label.unusable),
        (&Ty::String, true, &None)
    );
    let words = struct_field(def, "words");
    assert_eq!(
        (&words.ty, words.is_pub),
        (&Ty::Vec(Box::new(Ty::String)), false)
    );
    assert!(!struct_field(def, "hits").is_pub);
    let counts = struct_field(def, "counts");
    assert!(counts.is_pub);
    let unusable = counts.unusable.as_ref().expect("HashMap does not map");
    assert_eq!(unusable.rust_ty, "HashMap < String , i32 >");
    let (_, local) = imported_struct(&resolved, "crate", "Local");
    assert!(!local.imported);
}

#[test]
fn imported_methods_and_associated_functions_are_members() {
    let resolved = rust_structs();
    let (id, _) = imported_struct(&resolved, "ext", "Matcher");
    let new = imported_member(&resolved, id, "new");
    assert!(new.callable);
    assert_eq!(
        (new.owner, new.self_mode),
        (Some(UserType::Struct(id)), None)
    );
    assert_eq!(new.params, vec![(Ty::String, ParamMode::SharedBorrow)]);
    assert_eq!(new.ret, Ty::Struct(id));
    let is_match = imported_member(&resolved, id, "is_match");
    assert_eq!(
        is_match.modes(),
        vec![ParamMode::SharedBorrow, ParamMode::SharedBorrow]
    );
    assert_eq!(is_match.ret, Ty::Bool);
    let bump = imported_member(&resolved, id, "bump");
    assert_eq!(bump.modes(), vec![ParamMode::MutableBorrow]);
    // Methods are not free functions of the module.
    assert!(
        resolved
            .symbols
            .lookup_fn(resolved.entry, Some(&p("ext")), "is_match")
            .is_err()
    );
}

#[test]
fn opaque_methods_are_uncallable_and_say_what_to_change() {
    let resolved = rust_structs();
    let (id, _) = imported_struct(&resolved, "ext", "Matcher");
    for (name, says) in [
        ("into_inner", "`&self` or `&mut self`"),
        ("name", "return an owned value"),
        ("pick", "generic"),
    ] {
        let sig = imported_member(&resolved, id, name);
        assert!(!sig.callable, "{name}");
        let note = sig.note.as_deref().unwrap_or_default();
        assert!(note.contains(says), "{name}: {note}");
    }
}

#[test]
fn imported_types_in_fields_and_signatures() {
    let resolved = rust_structs();
    let (matcher, _) = imported_struct(&resolved, "ext", "Matcher");
    let (thing, _) = imported_struct(&resolved, "other", "Thing");
    let (_, holder) = imported_struct(&resolved, "ext", "Holder");
    assert_eq!(struct_field(holder, "full").ty, Ty::Struct(thing));
    assert_eq!(
        struct_field(holder, "list").ty,
        Ty::Vec(Box::new(Ty::Struct(matcher)))
    );
    let note = |field: &str| {
        let unusable = struct_field(holder, field)
            .unusable
            .as_ref()
            .unwrap_or_else(|| panic!("{field} should be unusable"));
        unusable.note.clone().unwrap_or_default()
    };
    assert!(
        note("short").contains("write the full path"),
        "{}",
        note("short")
    );
    assert!(
        note("local").contains("declared in Varyk"),
        "{}",
        note("local")
    );
    let wrapped = struct_field(holder, "wrapped").unusable.as_ref();
    assert_eq!(wrapped.map(|u| u.rust_ty.as_str()), Some("Wrap < i32 >"));

    let Ok(Callee::Imported(pair)) =
        resolved
            .symbols
            .lookup_fn(resolved.entry, Some(&p("ext")), "pair")
    else {
        panic!("pair should be imported");
    };
    let pair = &resolved.symbols.imported[pair.0 as usize];
    assert!(pair.callable);
    assert_eq!(
        pair.params,
        vec![
            (Ty::Struct(matcher), ParamMode::SharedBorrow),
            (Ty::Struct(matcher), ParamMode::MutableBorrow),
            (Ty::Struct(thing), ParamMode::Owned),
        ]
    );
    assert_eq!(pair.ret, Ty::Option(Box::new(Ty::Struct(matcher))));
}

#[test]
fn skipped_structs_are_not_types_and_say_why() {
    let resolved = rust_structs();
    let symbols = &resolved.symbols;
    let ext = p("ext");
    assert!(
        symbols
            .lookup_type(resolved.entry, Some(&ext), "Wrap")
            .is_err()
    );
    assert!(
        symbols
            .lookup_type(resolved.entry, Some(&ext), "Meters")
            .is_err()
    );
    let span = Span::new(FileId(0), 0, 0);
    let diagnostic = symbols
        .skipped_item(resolved.entry, Some(&ext), "Meters", "ext::Meters", span)
        .expect("Meters was skipped");
    assert_eq!(diagnostic.code, codes::V0101);
    assert!(
        diagnostic.notes[0].contains("tuple struct"),
        "{diagnostic:?}"
    );
    assert!(
        symbols
            .skipped_item(resolved.entry, Some(&ext), "Matcher", "ext::Matcher", span)
            .is_none()
    );
}

/// `use` of a `.rs` struct Varyk did not import is V0101 saying why (M3
/// spec 4.1), not "cannot find".
#[test]
fn use_of_a_skipped_rust_struct_is_v0101_with_the_reason() {
    let (result, sources) =
        resolve_file("crates/varyk/tests/fixtures/errors/v0101_use_generic_rust_struct/main.vr");
    let diagnostics = result.expect_err("should fail");
    let d = only(&diagnostics);
    assert_eq!(d.code, codes::V0101);
    assert_eq!(d.span, span_of(&sources, 0, "ext::Wrap"));
    assert!(
        d.notes
            .iter()
            .any(|n| n.contains("type or lifetime parameters")),
        "{d:#?}"
    );
}

// --- Imported enums (M3 spec 4.3, 4.4) ------------------------------------

fn rust_enums() -> Resolved {
    let (result, _) = resolve_file("crates/varyk/tests/fixtures/resolve/rust_enums/main.vr");
    result.unwrap_or_else(|d| panic!("rust_enums should resolve: {d:#?}"))
}

/// The imported enum `name` of module `module` (by its `mod` name).
fn imported_enum<'a>(resolved: &'a Resolved, module: &str, name: &str) -> (EnumId, &'a EnumDef) {
    let at = p(module);
    let Ok(UserType::Enum(id)) = resolved
        .symbols
        .lookup_type(resolved.entry, Some(&at), name)
    else {
        panic!("no enum {module}::{name}");
    };
    (id, &resolved.symbols.enums[id.0 as usize])
}

/// The types the variant `name` of `def` holds.
fn enum_variant(def: &EnumDef, name: &str) -> Vec<Ty> {
    def.variants
        .iter()
        .find(|variant| variant.name == name)
        .unwrap_or_else(|| panic!("no variant {name}"))
        .types()
        .into_iter()
        .cloned()
        .collect()
}

#[test]
fn imported_enum_is_in_the_enum_table_with_its_variants() {
    let resolved = rust_enums();
    let (_, def) = imported_enum(&resolved, "ext", "Kind");
    assert!(def.imported && def.is_pub);
    assert_eq!(def.opaque, None);
    assert_eq!(enum_variant(def, "Word"), Vec::<Ty>::new());
    assert_eq!(enum_variant(def, "Number"), vec![Ty::Int(IntKind::I32)]);
    let (_, local) = imported_enum(&resolved, "crate", "Local");
    assert!(!local.imported);
}

#[test]
fn a_named_field_variant_makes_the_enum_opaque() {
    let resolved = rust_enums();
    let (_, def) = imported_enum(&resolved, "ext", "Shape");
    assert!(def.imported);
    let reason = def.opaque.as_deref().expect("should be opaque");
    assert!(reason.contains("Circle"), "{reason}");
}

#[test]
fn an_unmappable_payload_makes_the_enum_opaque() {
    let resolved = rust_enums();
    let (_, def) = imported_enum(&resolved, "ext", "Counted");
    let reason = def.opaque.as_deref().expect("should be opaque");
    assert!(reason.contains("Tally"), "{reason}");
    let (_, def) = imported_enum(&resolved, "ext", "ViaUse");
    let reason = def.opaque.as_deref().expect("should be opaque");
    assert!(reason.contains("write the full path"), "{reason}");
}

/// An imported enum is a valid `Ty::Enum` in fields, a `crate::` payload,
/// and a `.rs` function's return type (spec 4.4), even though it is
/// mapped only once every module's imports are known.
#[test]
fn imported_enum_types_in_fields_and_signatures() {
    let resolved = rust_enums();
    let (kind, _) = imported_enum(&resolved, "ext", "Kind");
    let (thing, _) = imported_enum(&resolved, "ext", "Holds");
    let thing_struct =
        match resolved
            .symbols
            .lookup_type(resolved.entry, Some(&p("other")), "Thing")
        {
            Ok(UserType::Struct(id)) => id,
            other => panic!("Thing should be an imported struct: {other:?}"),
        };
    assert_eq!(
        enum_variant(&resolved.symbols.enums[thing.0 as usize], "Full"),
        vec![Ty::Struct(thing_struct)]
    );
    let (_, container) = imported_struct(&resolved, "ext", "Container");
    assert_eq!(struct_field(container, "kind").ty, Ty::Enum(kind));

    let Ok(Callee::Imported(make_kind)) =
        resolved
            .symbols
            .lookup_fn(resolved.entry, Some(&p("ext")), "make_kind")
    else {
        panic!("make_kind should be imported");
    };
    let sig = &resolved.symbols.imported[make_kind.0 as usize];
    assert!(sig.callable);
    assert_eq!(sig.ret, Ty::Enum(kind));
}

#[test]
fn skipped_enums_are_not_types_and_say_why() {
    let resolved = rust_enums();
    let symbols = &resolved.symbols;
    let ext = p("ext");
    assert!(
        symbols
            .lookup_type(resolved.entry, Some(&ext), "Wrap")
            .is_err()
    );
    let span = Span::new(FileId(0), 0, 0);
    let diagnostic = symbols
        .skipped_item(resolved.entry, Some(&ext), "Wrap", "ext::Wrap", span)
        .expect("Wrap was skipped");
    assert_eq!(diagnostic.code, codes::V0101);
    assert!(diagnostic.message.contains("enum"), "{diagnostic:?}");
    assert!(
        diagnostic.notes[0].contains("type or lifetime parameters"),
        "{diagnostic:?}"
    );
    assert!(
        symbols
            .skipped_item(resolved.entry, Some(&ext), "Kind", "ext::Kind", span)
            .is_none()
    );
}

/// A Varyk struct field or enum payload naming an imported enum resolves
/// to `Ty::Enum` without panicking (regression: the recursive-type
/// check's per-item table used to be sized to every Varyk-declared enum
/// only, one short of every imported enum too, so an imported `EnumId`
/// indexed past its end).
#[test]
fn a_varyk_struct_field_and_enum_payload_naming_an_imported_enum_resolve() {
    let resolved = rust_enums();
    let (kind, _) = imported_enum(&resolved, "ext", "Kind");
    let with_kind = match resolved
        .symbols
        .lookup_type(resolved.entry, None, "WithKind")
    {
        Ok(UserType::Struct(id)) => id,
        other => panic!("WithKind should be a Varyk struct: {other:?}"),
    };
    let with_kind = &resolved.symbols.structs[with_kind.0 as usize];
    assert_eq!(struct_field(with_kind, "kind").ty, Ty::Enum(kind));

    let wraps_kind = match resolved
        .symbols
        .lookup_type(resolved.entry, None, "WrapsKind")
    {
        Ok(UserType::Enum(id)) => id,
        other => panic!("WrapsKind should be a Varyk enum: {other:?}"),
    };
    let wraps_kind = &resolved.symbols.enums[wraps_kind.0 as usize];
    assert!(!wraps_kind.imported);
    assert_eq!(enum_variant(wraps_kind, "Held"), vec![Ty::Enum(kind)]);
}

/// Resolves `text` as a package's entry of `kind` depending on `crates`.
fn resolve_root_str(text: &str, kind: Kind, crates: &[&str]) -> Result<Resolved, Vec<Diagnostic>> {
    let mut sources = Vec::new();
    let entry = SourceFile::new(FileId(0), "dummy/src/lib.vr", text);
    let crates: Vec<String> = crates.iter().map(|name| name.to_string()).collect();
    let dependencies = Dependencies {
        crates: &crates,
        dev: &[],
        optional: &[],
    };
    resolve_root(
        entry,
        kind,
        Some(dependencies),
        Packages::default(),
        &mut sources,
    )
}

#[test]
fn a_library_needs_no_main() {
    let resolved = resolve_root_str("pub fn greet() {}\n", Kind::Library, &[])
        .unwrap_or_else(|d| panic!("{d:#?}"));
    assert_eq!(resolved.kind, Kind::Library);
}

#[test]
fn main_in_a_library_is_v0106() {
    let d = resolve_root_str("fn main() {}\n", Kind::Library, &[]).unwrap_err();
    let d = only(&d);
    assert_eq!(d.code, codes::V0106);
    assert!(d.message.contains("library"), "{d:#?}");
}

#[test]
fn use_of_a_dependency_is_v0110_with_the_facade_note() {
    let d = resolve_root_str(
        "use regex::Regex;\n\nfn main() {}\n",
        Kind::Binary,
        &["regex"],
    )
    .unwrap_err();
    let d = only(&d);
    assert_eq!(d.code, codes::V0110, "{d:#?}");
    assert!(d.message.contains("regex"), "{d:#?}");
}

#[test]
fn use_of_a_hyphenated_dependency_names_it_with_an_underscore() {
    let d = resolve_root_str(
        "use my_dep::Thing;\n\nfn main() {}\n",
        Kind::Binary,
        &["my-dep"],
    )
    .unwrap_err();
    assert_eq!(only(&d).code, codes::V0110, "{d:#?}");
}

// --- An imported item naming a type behind a private module (M3 4.2) ------

#[test]
fn an_imported_fn_and_field_naming_a_hidden_type_are_usable_only_inside_its_module() {
    for case in ["hidden_type_inside", "hidden_type_outside"] {
        let (result, _) = resolve_file(&format!(
            "crates/varyk/tests/fixtures/interop/{case}/main.vr"
        ));
        let resolved = result.expect("resolves; the call site decides");
        let symbols = &resolved.symbols;
        let shop = module_id(&resolved, "shop");
        let sig = symbols
            .imported
            .iter()
            .find(|sig| sig.name == "make")
            .expect("make is imported");
        // The signature keeps its types and is callable inside `shop`.
        assert!(sig.callable);
        assert_eq!(sig.within, Some(shop));
        assert!(matches!(sig.ret, Ty::Struct(_)));
        assert!(symbols.usable_from(sig.within, shop));
        assert!(!symbols.usable_from(sig.within, resolved.entry));
        let boxed = symbols
            .structs
            .iter()
            .find(|s| s.name == "Boxed")
            .expect("Boxed is imported");
        let unusable = boxed.fields[0].unusable.as_ref().expect("marked");
        assert_eq!(unusable.within, Some(shop));
        assert!(matches!(boxed.fields[0].ty, Ty::Struct(_)));
    }
}

#[test]
fn a_use_of_a_variant_by_crate_path_of_a_type_in_scope_says_only_to_write_the_path() {
    let diagnostics =
        errors_str("enum Shape {\n    Circle,\n}\n\nuse crate::Shape::Circle;\n\nfn main() {}\n");
    let d = only(&diagnostics);
    assert_eq!(d.code, codes::V0111);
    assert_eq!(
        d.message,
        "`use` cannot name a variant; `Shape` can already be used here, so write `Shape::Circle`"
    );
}

#[test]
fn a_rs_struct_named_error_is_v0113_at_its_name() {
    let (diagnostics, sources) = error_fixture("v0113_rs_struct_named_error");
    let d = only(&diagnostics);
    assert_eq!(d.code, codes::V0113, "{d:#?}");
    assert_eq!(d.span, span_of(&sources, 1, "Error"));
}

#[test]
fn a_use_alias_named_error_is_v0113() {
    let d = errors_str("struct Problem {\n    n: i32,\n}\nuse Problem as Error;\nfn main() {}\n");
    assert_eq!(only(&d).code, codes::V0113, "{d:#?}");
}

// --- Attributes and reserved names (M5a spec 2.2, 2.7, 2.10) ---------------

/// The code and headline of every diagnostic for `text`.
fn codes_and_messages(text: &str) -> Vec<(&'static str, String)> {
    errors_str(text)
        .into_iter()
        .map(|d| (d.code, d.message))
        .collect()
}

fn resolved_str(text: &str) -> Resolved {
    match resolve_str(text).0 {
        Ok(resolved) => resolved,
        Err(diagnostics) => panic!("unexpected diagnostics for:\n{text}\n{diagnostics:#?}"),
    }
}

#[test]
fn derive_on_a_struct_is_v0112_naming_the_four_with_the_derive_note() {
    let diagnostics = errors_str("#[derive(Clone)]\nstruct P { x: i32 }\nfn main() {}");
    let d = only(&diagnostics);
    assert_eq!(d.code, codes::V0112);
    assert_eq!(d.message, "there is no attribute named `derive`");
    assert_eq!((d.span.start, d.span.end), (0, 16));
    assert_eq!(
        d.notes,
        vec![
            "the attributes are `#[rename(\"key\")]`, `#[default(value)]`, `#[skip]`, and \
             `#[test]`"
                .to_string(),
            "`.clone()`, `==`, and JSON work without a derive: Varyk gives a type each of them \
             when it can"
                .to_string(),
        ]
    );
}

#[test]
fn an_attribute_on_a_struct_enum_impl_mod_or_use_is_v0112() {
    let cases = [
        (
            "#[skip] struct P { x: i32 }",
            "`#[skip]` cannot go on a struct",
        ),
        (
            "#[rename(\"e\")] enum E { A }",
            "`#[rename]` cannot go on an enum",
        ),
        (
            "struct P { x: i32 }\n#[test] impl P { }",
            "`#[test]` cannot go on an `impl` block",
        ),
        // No file for `m`: V0104 as well.
        ("#[skip] mod m;", "`#[skip]` cannot go on a `mod` line"),
        (
            "#[skip] use crate::f as g;\nfn f() {}",
            "`#[skip]` cannot go on a `use` line",
        ),
    ];
    for (source, message) in cases {
        let text = format!("{source}\nfn main() {{}}");
        let found: Vec<(&str, String)> = codes_and_messages(&text)
            .into_iter()
            .filter(|(code, _)| *code != codes::V0104)
            .collect();
        assert_eq!(found, vec![(codes::V0112, message.to_string())], "{text}");
    }
}

#[test]
fn attributes_in_the_wrong_place_are_v0112_saying_where_they_go() {
    let cases = [
        (
            "struct P { #[inline] x: i32 }",
            "there is no attribute named `inline`",
            None,
        ),
        (
            "struct P { #[test] x: i32 }",
            "`#[test]` cannot go on a struct field",
            Some("`#[test]` goes before a top-level function"),
        ),
        (
            "enum E { #[skip] A }",
            "`#[skip]` cannot go on a variant",
            Some("`#[skip]` goes before a struct field"),
        ),
        (
            "enum E { #[rename(\"c\")] C(i32) }",
            "`#[rename]` cannot go on a variant that carries data",
            Some("`#[rename]` goes before a struct field or a variant that carries no data"),
        ),
        (
            "enum E { C { #[rename(\"x\")] x: i32 } }",
            "`#[rename]` cannot go on a field of a variant",
            Some("`#[rename]` goes before a struct field or a variant that carries no data"),
        ),
        (
            "struct P { x: i32 }\nimpl P { #[test] fn t() {} }",
            "`#[test]` cannot go on a method",
            Some("`#[test]` goes before a top-level function"),
        ),
        (
            "struct P { x: i32 }\nimpl P { #[skip] fn t(self) {} }",
            "`#[skip]` cannot go on a method",
            Some("`#[skip]` goes before a struct field"),
        ),
        (
            "#[default(1)] fn f() {}",
            "`#[default]` cannot go on a function",
            Some("`#[default]` goes before a struct field"),
        ),
    ];
    for (source, message, note) in cases {
        let text = format!("{source}\nfn main() {{}}");
        let diagnostics = errors_str(&text);
        let d = only(&diagnostics);
        assert_eq!(
            (d.code, d.message.as_str()),
            (codes::V0112, message),
            "{text}"
        );
        if let Some(note) = note {
            assert_eq!(d.notes, vec![note.to_string()], "{text}");
        }
    }
}

#[test]
fn the_same_attribute_twice_is_v0112_at_the_second() {
    let text = "struct P { #[skip] #[skip] x: Option<i32> }\nfn main() {}";
    let (result, sources) = resolve_str(text);
    let Err(diagnostics) = result else {
        panic!("expected diagnostics");
    };
    let d = only(&diagnostics);
    assert_eq!(d.code, codes::V0112);
    assert_eq!(d.message, "`#[skip]` is written twice here");
    let second = text.rfind("#[skip]").expect("in text") as u32;
    assert_eq!(d.span.start, second);
    assert_eq!(d.labels[0].span, span_of(&sources, 0, "#[skip]"));
}

#[test]
fn a_missing_or_unexpected_argument_is_v0112() {
    let cases = [
        (
            "struct P { #[rename] x: i32 }",
            "`#[rename]` needs a value, as in `#[rename(\"userName\")]`",
        ),
        (
            "struct P { #[default] x: i32 }",
            "`#[default]` needs a value, as in `#[default(8080)]`",
        ),
        (
            "struct P { #[skip(1)] x: Option<i32> }",
            "`#[skip]` takes no value",
        ),
        ("#[test(1)] fn t() {}", "`#[test]` takes no value"),
    ];
    for (source, message) in cases {
        let text = format!("{source}\nfn main() {{}}");
        assert_eq!(
            codes_and_messages(&text),
            vec![(codes::V0112, message.to_string())],
            "{text}"
        );
    }
}

#[test]
fn a_literal_that_does_not_fit_is_v0209() {
    let cases = [
        (
            "struct P { #[rename(1)] x: i32 }",
            "`#[rename]` needs a name in quotes, as in `#[rename(\"userName\")]`",
        ),
        (
            "struct P { #[rename(userName)] x: i32 }",
            "`#[rename]` needs a name in quotes, as in `#[rename(\"userName\")]`",
        ),
        (
            "enum E { #[rename(true)] A }",
            "`#[rename]` needs a name in quotes, as in `#[rename(\"userName\")]`",
        ),
        (
            "struct P { #[rename(\"\")] x: i32 }",
            "`#[rename(\"\")]` gives an empty name",
        ),
        (
            "struct P { #[default(\"x\")] x: i32 }",
            "the default `\"x\"` does not fit the type `i32`",
        ),
        (
            "struct P { #[default(300)] x: u8 }",
            "the default `300` does not fit in `u8` (0 to 255)",
        ),
        (
            "struct P { #[default(-1)] x: u8 }",
            "the default `-1` does not fit in `u8` (0 to 255)",
        ),
        (
            "struct P { #[default(1)] x: f64 }",
            "the default `1` does not fit the type `f64`",
        ),
        (
            "struct P { #[default(1.5)] x: i32 }",
            "the default `1.5` does not fit the type `i32`",
        ),
        (
            "struct P { #[default(0.00000000000000000000000000000000000000000000001)] x: f32 }",
            "the default `0.00000000000000000000000000000000000000000000001` does not fit in `f32`",
        ),
        (
            "struct P { #[default(1)] x: bool }",
            "the default `1` does not fit the type `bool`",
        ),
        (
            "struct P { #[default(Role::Admin)] x: i32 }",
            "`#[default]` needs a number, a string in quotes, `true`, or `false`",
        ),
        (
            "struct P { #[default(1)] x: Option<i32> }",
            "`#[default]` cannot go on an `Option` field",
        ),
        (
            "enum Role { A }\nstruct P { #[default(1)] x: Role }",
            "`#[default]` cannot go on a field of type `Role`",
        ),
        (
            "struct Q { y: i32 }\nstruct P { #[default(1)] x: Q }",
            "`#[default]` cannot go on a field of type `Q`",
        ),
        (
            "struct P { #[default(1)] x: Vec<i32> }",
            "`#[default]` cannot go on a field of type `Vec<i32>`",
        ),
    ];
    for (source, message) in cases {
        let text = format!("{source}\nfn main() {{}}");
        assert_eq!(
            codes_and_messages(&text),
            vec![(codes::V0209, message.to_string())],
            "{text}"
        );
    }
}

#[test]
fn an_integer_default_on_a_float_says_to_write_a_fraction() {
    let diagnostics = errors_str("struct P { #[default(-2)] x: f32 }\nfn main() {}");
    assert_eq!(
        only(&diagnostics).notes,
        vec!["write `-2.0` for a number with a fractional part".to_string()]
    );
}

#[test]
fn well_placed_attributes_resolve_into_the_definitions() {
    let resolved = resolved_str(
        "enum Role { #[rename(\"admin\")] Admin, Member, Guest(i32) }\n\
         struct User {\n\
             #[rename(\"userName\")] user_name: string,\n\
             #[skip] #[default(-3)] a: i8,\n\
             #[default(1_000)] b: u64,\n\
             #[default(-1.5)] c: f64,\n\
             #[default(2.5)] d: f32,\n\
             #[default(\"hi\\n\")] e: string,\n\
             #[default(true)] f: bool,\n\
             #[skip] g: Option<i32>,\n\
             h: Role,\n\
         }\n\
         #[test]\n\
         fn checks() {}\n\
         fn main() {}",
    );
    let user = &resolved.symbols.structs[0];
    let attrs: Vec<&FieldAttrs> = user.fields.iter().map(|f| &f.attrs).collect();
    assert_eq!(attrs[0].rename.as_deref(), Some("userName"));
    assert_eq!(attrs[0].default, None);
    assert!(!attrs[0].skip);
    assert!(attrs[1].skip);
    assert_eq!(attrs[1].default, Some(HirDefault::Int(-3)));
    assert_eq!(attrs[2].default, Some(HirDefault::Int(1000)));
    assert_eq!(attrs[3].default, Some(HirDefault::Float(-1.5)));
    assert_eq!(attrs[4].default, Some(HirDefault::Float(2.5)));
    assert_eq!(attrs[5].default, Some(HirDefault::Str("hi\\n".to_string())));
    assert_eq!(attrs[6].default, Some(HirDefault::Bool(true)));
    assert!(attrs[7].skip);
    assert_eq!(attrs[8], &FieldAttrs::default());
    let role = &resolved.symbols.enums[0];
    let renames: Vec<Option<&str>> = role.variants.iter().map(|v| v.rename.as_deref()).collect();
    assert_eq!(renames, vec![Some("admin"), None, None]);
    let tests: Vec<(&str, bool)> = resolved
        .symbols
        .fns
        .iter()
        .map(|f| (f.name.as_str(), f.is_test))
        .collect();
    assert_eq!(tests, vec![("checks", true), ("main", false)]);
}

#[test]
fn a_test_with_parameters_or_a_return_type_is_v0114() {
    for source in ["#[test] fn t(x: i32) {}", "#[test] fn t() -> i32 { 1 }"] {
        let text = format!("{source}\nfn main() {{}}");
        let diagnostics = errors_str(&text);
        let d = only(&diagnostics);
        assert_eq!(d.code, codes::V0114, "{text}");
        assert_eq!(
            d.message,
            "a test must take no parameters and return nothing"
        );
        assert_eq!(&text[d.span.start as usize..d.span.end as usize], "fn t");
    }
}

#[test]
fn the_standard_module_names_are_v0113_for_a_module() {
    for name in ["json", "env", "log", "time"] {
        let text = format!("mod {name};\nfn main() {{}}");
        assert_eq!(
            codes_and_messages(&text),
            vec![(
                codes::V0113,
                format!("the name `{name}` is already taken by the standard `{name}` module")
            )],
            "{text}"
        );
    }
    // So may not a struct or an enum, whose associated functions would
    // otherwise be taken for the module's.
    for name in ["json", "env", "log", "time"] {
        for text in [
            format!("struct {name} {{ x: i32 }}\nfn main() {{}}"),
            format!("enum {name} {{ A }}\nfn main() {{}}"),
        ] {
            assert_eq!(
                codes_and_messages(&text),
                vec![(
                    codes::V0113,
                    format!("the name `{name}` is already taken by the standard `{name}` module")
                )],
                "{text}"
            );
        }
    }
    // A local, a field, and a function may use the names.
    resolved_str("struct S { log: i32 }\nfn env() {}\nfn main() { let json = 1; }");
}

#[test]
fn assert_and_assert_eq_are_v0113_for_a_function() {
    for name in ["assert", "assert_eq"] {
        let text = format!("fn {name}() {{}}\nfn main() {{}}");
        assert_eq!(
            codes_and_messages(&text),
            vec![(
                codes::V0113,
                format!("the name `{name}` is already taken by the standard check `{name}`")
            )],
            "{text}"
        );
    }
}

#[test]
fn a_use_of_a_standard_module_is_v0113() {
    for path in [
        "json",
        "json::parse",
        "log::info",
        "env",
        "time",
        "time::sleep",
    ] {
        let text = format!("use {path};\nfn main() {{}}");
        let first = path.split("::").next().unwrap_or(path);
        assert_eq!(
            codes_and_messages(&text),
            vec![(
                codes::V0113,
                format!("`{first}` is a standard module and cannot be brought in with `use`")
            )],
            "{text}"
        );
    }
}

#[test]
fn the_varyk_prefix_is_v0113_on_every_item() {
    let cases = [
        "fn varyk_f() {}",
        "struct varyk_S { x: i32 }",
        "enum varyk_E { A }",
        "mod varyk_m;",
        "struct P { x: i32 }\nimpl P { fn varyk_m(self) {} }",
        "fn f() {}\nuse crate::f as varyk_f;",
    ];
    for source in cases {
        let text = format!("{source}\nfn main() {{}}");
        let diagnostics = errors_str(&text);
        let d = only(&diagnostics);
        assert_eq!(d.code, codes::V0113, "{text}");
        assert!(
            d.message.starts_with("the name `varyk_")
                && d.message.ends_with("` starts with `varyk_`"),
            "{text}: {}",
            d.message
        );
    }
}

#[test]
fn a_use_of_a_test_function_is_v0114() {
    let (diagnostics, _) = error_fixture("v0114_test_used");
    let d = only(&diagnostics);
    assert_eq!(d.code, codes::V0114);
    assert_eq!(d.message, "`m::checks` is a test and cannot be imported");
}

#[test]
fn a_test_named_main_in_the_entry_file_is_v0114() {
    let text = "#[test]\nfn main() {}";
    let diagnostics = errors_str(text);
    let d = only(&diagnostics);
    assert_eq!(d.code, codes::V0114);
    assert_eq!(
        d.message,
        "`main` is where the program starts and cannot be a test"
    );
    assert_eq!(&text[d.span.start as usize..d.span.end as usize], "fn main");
}

#[test]
fn a_negative_default_on_an_unsigned_field_is_v0209_even_zero() {
    assert_eq!(
        codes_and_messages("struct P { #[default(-0)] x: u16 }\nfn main() {}"),
        vec![(
            codes::V0209,
            "the default `-0` does not fit in `u16` (0 to 65535)".to_string()
        )]
    );
}

// --- `pub use` (milestone 5b3 spec 2.5) --------------------------------------

fn reexports() -> (Resolved, Vec<SourceFile>) {
    let (result, sources) = resolve_file("crates/varyk/tests/fixtures/resolve/reexports/main.vr");
    (
        result.unwrap_or_else(|d| panic!("reexports should resolve: {d:#?}")),
        sources,
    )
}

#[test]
fn pub_use_names_vr_and_rs_items_from_the_reexporting_module() {
    let (r, _) = reexports();
    let symbols = &r.symbols;
    let user = module_id(&r, "user");
    let krate = p("crate");
    let fn_at = |path: &str, name: &str| symbols.lookup_fn(user, Some(&p(path)), name);
    let type_at = |path: &str, name: &str| symbols.lookup_type(user, Some(&p(path)), name);
    // A `.vr` function (a bare path), struct (`self::`), and enum
    // (`crate::`), and a `.rs` function and struct: each the item itself,
    // still reachable at its own path.
    for name in ["connect", "item"] {
        let short = symbols.lookup_fn(user, Some(&krate), name);
        assert!(matches!(short, Ok(Callee::Varyk(_))), "{name}: {short:?}");
        assert_eq!(short, fn_at("crate::db", name), "{name}");
    }
    let ping = fn_at("crate", "ping");
    assert!(matches!(ping, Ok(Callee::Imported(_))), "{ping:?}");
    assert_eq!(ping, fn_at("crate::ext", "ping"));
    for name in ["Pool", "Mode", "Handle"] {
        let short = type_at("crate", name);
        assert!(short.is_ok(), "{name}: {short:?}");
    }
    assert_eq!(type_at("crate", "Pool"), type_at("crate::db", "Pool"));
    assert!(matches!(type_at("crate", "Mode"), Ok(UserType::Enum(_))));
    assert_eq!(type_at("crate", "Mode"), type_at("crate::db", "Mode"));
    assert_eq!(type_at("crate", "Handle"), type_at("crate::ext", "Handle"));
    // A `pub use` of a `pub use` is the original item.
    assert_eq!(fn_at("crate::facade", "item"), fn_at("crate::db", "item"));
    // The re-exporting module's own file names it bare.
    assert_eq!(
        symbols.lookup_fn(r.entry, None, "connect"),
        fn_at("crate::db", "connect")
    );
}

#[test]
fn pub_use_program_typechecks() {
    let (r, sources) = reexports();
    crate::types::typecheck(r, &sources)
        .unwrap_or_else(|d| panic!("reexports should typecheck: {d:#?}"));
}

#[test]
fn pub_use_of_a_module_is_v0001() {
    let (d, _) = error_fixture("v0001_pub_use_module");
    let d = only(&d);
    assert_eq!(d.code, codes::V0001);
    assert!(d.message.contains("module"), "{d:#?}");
    assert!(d.message.contains("not supported yet"), "{d:#?}");
}

#[test]
fn pub_use_clashing_with_a_declared_or_imported_name_is_v0103() {
    let (d, _) = error_fixture("v0103_pub_use_clash");
    assert_eq!(only(&d).code, codes::V0103);
    let (d, _) = fixture("reexport_clash_use");
    assert_eq!(only(&d).code, codes::V0103);
}

#[test]
fn pub_use_of_a_private_item_or_through_a_private_module_is_v0105() {
    let (d, sources) = error_fixture("v0105_pub_use_private");
    let d = only(&d);
    assert_eq!(d.code, codes::V0105);
    assert!(d.message.contains("pub mod db;"), "{d:#?}");
    assert_eq!(d.span, span_of(&sources, 0, "db::connect"));
    let (d, _) = fixture("reexport_private_item");
    assert_eq!(only(&d).code, codes::V0105);
}

// --- Varyk packages (M5b2 spec 2.1 to 2.4) ----------------------------------

mod packages {
    use super::*;
    use crate::test_packages::{Build, Dep, units_and_route};

    fn callee(resolved: &Resolved, path: &str, name: &str) -> Result<Callee, LookupError> {
        resolved
            .symbols
            .lookup_fn(resolved.entry, Some(&p(path)), name)
    }

    fn sig<'a>(resolved: &'a Resolved, path: &str, name: &str) -> &'a ImportedSig {
        match callee(resolved, path, name) {
            Ok(Callee::Imported(id)) => &resolved.symbols.imported[id.0 as usize],
            other => panic!("`{path}::{name}` should be a package's function: {other:?}"),
        }
    }

    fn units() -> Build {
        let mut build = Build::default();
        build.add("units", &[]);
        build
    }

    fn resolve_errors(build: &mut Build, text: &str, deps: &[(&str, Dep<'_>)]) -> Vec<Diagnostic> {
        match build.resolve(text, deps) {
            Ok(_) => panic!("expected diagnostics for:\n{text}"),
            Err(diagnostics) => diagnostics,
        }
    }

    #[test]
    fn a_dependency_names_its_package_after_local_modules_and_aliases() {
        let mut build = units();
        let resolved = build
            .resolve("fn main() {}\n", &[("units", Dep::Package("units"))])
            .unwrap_or_else(|d| panic!("{d:#?}"));
        let one = sig(&resolved, "units", "one");
        assert_eq!(one.ret, I32);
        assert_eq!(
            one.package,
            Some(PackageItem {
                package: PackageId(0),
                name: "units".to_string(),
                path: Vec::new(),
            })
        );
        let add = sig(&resolved, "units::length", "add");
        assert_eq!(
            add.package.as_ref().map(|item| item.path.clone()),
            Some(vec!["length".to_string()])
        );

        // A module declared here comes first.
        let resolved = build
            .resolve_fixture("app_local", &[("units", Dep::Package("units"))])
            .unwrap_or_else(|d| panic!("{d:#?}"));
        assert!(matches!(
            callee(&resolved, "units", "one"),
            Ok(Callee::Varyk(_))
        ));

        // A `use` alias comes first too.
        let resolved = build
            .resolve_fixture("app_alias", &[("units", Dep::Package("units"))])
            .unwrap_or_else(|d| panic!("{d:#?}"));
        assert!(matches!(
            callee(&resolved, "units", "one"),
            Ok(Callee::Varyk(_))
        ));
    }

    #[test]
    fn a_dependency_key_is_named_with_its_dash_read_as_an_underscore() {
        let mut build = units();
        let resolved = build
            .resolve("fn main() {}\n", &[("unit-kit", Dep::Package("units"))])
            .unwrap_or_else(|d| panic!("{d:#?}"));
        assert_eq!(sig(&resolved, "unit_kit", "one").ret, I32);
        assert_eq!(callee(&resolved, "units", "one"), Err(LookupError::Unknown));
    }

    #[test]
    fn a_standard_name_comes_before_a_dependency_and_its_v0113_shows_the_rename() {
        let mut build = units();
        let d = resolve_errors(
            &mut build,
            "use json::length;\nfn main() {}\n",
            &[("json", Dep::Package("units"))],
        );
        let d = only(&d);
        assert_eq!(d.code, codes::V0113);
        assert!(
            d.notes
                .iter()
                .any(|note| note.contains("package = \"units\"")),
            "{d:#?}"
        );
    }

    #[test]
    fn crate_inside_a_package_means_that_package() {
        let mut build = units();
        let resolved = build
            .resolve("fn main() {}\n", &[("units", Dep::Package("units"))])
            .unwrap_or_else(|d| panic!("{d:#?}"));
        assert_eq!(sig(&resolved, "units", "zero_value").ret, I32);
    }

    #[test]
    fn an_item_of_a_package_without_pub_is_v0105() {
        let mut build = units();
        let resolved = build
            .resolve("fn main() {}\n", &[("units", Dep::Package("units"))])
            .unwrap_or_else(|d| panic!("{d:#?}"));
        assert!(matches!(
            callee(&resolved, "units", "secret"),
            Err(LookupError::NotVisible { .. })
        ));
        assert!(matches!(
            callee(&resolved, "units::hidden", "deep"),
            Err(LookupError::PrivateModule { .. })
        ));
        let d = resolve_errors(
            &mut build,
            "use units::secret;\nfn main() {}\n",
            &[("units", Dep::Package("units"))],
        );
        assert_eq!(only(&d).code, codes::V0105);
    }

    #[test]
    fn a_rust_crate_in_a_path_or_a_use_is_v0110() {
        let mut build = units();
        let deps = [("regex", Dep::Rust)];
        let d = resolve_errors(&mut build, "use regex::Regex;\nfn main() {}\n", &deps);
        assert_eq!(only(&d).code, codes::V0110);
        let d = resolve_errors(
            &mut build,
            "fn f(r: regex::Regex) {}\nfn main() {}\n",
            &deps,
        );
        let d = only(&d);
        assert_eq!(d.code, codes::V0110);
        assert!(
            d.message.contains("`regex` is a library written in Rust"),
            "{d:#?}"
        );
    }

    #[test]
    fn naming_an_optional_dependency_is_v0401() {
        let mut build = units();
        let deps = [("units", Dep::Optional("units"))];
        let d = resolve_errors(&mut build, "use units::one;\nfn main() {}\n", &deps);
        let d = only(&d);
        assert_eq!(d.code, codes::V0401);
        assert!(d.message.contains("optional"), "{d:#?}");
    }

    #[test]
    fn use_of_a_package_alone_is_v0111() {
        let mut build = units();
        let d = resolve_errors(
            &mut build,
            "use units;\nfn main() {}\n",
            &[("units", Dep::Package("units"))],
        );
        assert_eq!(only(&d).code, codes::V0111);
    }

    #[test]
    fn a_package_s_tests_are_not_visible() {
        let mut build = units();
        let resolved = build
            .resolve("fn main() {}\n", &[("units", Dep::Package("units"))])
            .unwrap_or_else(|d| panic!("{d:#?}"));
        assert_eq!(
            callee(&resolved, "units", "one_is_one"),
            Err(LookupError::Unknown)
        );
    }

    #[test]
    fn v0100_shows_the_rename_when_the_name_is_a_dependency_it_cannot_reach() {
        let mut build = units();
        let d = resolve_errors(
            &mut build,
            "fn f(m: varyk_units::length::Meters) {}\nfn main() {}\n",
            &[("varyk-units", Dep::Package("units"))],
        );
        let d = only(&d);
        assert_eq!(d.code, codes::V0101);
        // A type is V0101; a call is V0100, checked in the type checker.
        let mut build = units();
        let resolved = build.resolve(
            "fn main() {\n    let n = varyk_units::one();\n}\n",
            &[("varyk-units", Dep::Package("units"))],
        );
        let resolved = resolved.unwrap_or_else(|d| panic!("{d:#?}"));
        assert_eq!(
            callee(&resolved, "varyk_units", "one"),
            Err(LookupError::Unknown)
        );
    }

    #[test]
    fn v0111_for_a_module_declared_elsewhere_comes_first_and_shows_the_rename() {
        let mut build = units();
        let d = match build.resolve_fixture("app_elsewhere", &[("units", Dep::Package("units"))]) {
            Ok(_) => panic!("`use units::one;` should be V0111"),
            Err(d) => d,
        };
        let d = only(&d);
        assert_eq!(d.code, codes::V0111);
        assert!(
            d.notes
                .iter()
                .any(|note| note.contains("is also a dependency")),
            "{d:#?}"
        );
    }

    #[test]
    fn a_single_name_is_never_a_package() {
        let mut build = units();
        let d = resolve_errors(
            &mut build,
            "fn f(u: units) {}\nfn main() {}\n",
            &[("units", Dep::Package("units"))],
        );
        assert_eq!(only(&d).code, codes::V0101);
    }

    #[test]
    fn an_impl_of_a_package_s_type_is_v0001() {
        let mut build = units();
        let d = resolve_errors(
            &mut build,
            "use units::length::Meters;\nimpl Meters {\n    fn f(self) {}\n}\nfn main() {}\n",
            &[("units", Dep::Package("units"))],
        );
        assert_eq!(only(&d).code, codes::V0001);
    }

    #[test]
    fn a_rust_dependency_never_gets_the_rename_note() {
        let mut build = units();
        let d = resolve_errors(
            &mut build,
            "use json::parse;\nfn main() {}\n",
            &[("json", Dep::Rust)],
        );
        let d = only(&d);
        assert_eq!(d.code, codes::V0113);
        assert!(
            !d.notes
                .iter()
                .any(|note| note.contains("also a dependency")),
            "{d:#?}"
        );
    }

    #[test]
    fn a_standard_name_comes_before_a_rust_dependency_of_that_name() {
        let mut build = units();
        let d = resolve_errors(
            &mut build,
            "fn f(t: time::Clock) {}\nfn main() {}\n",
            &[("time", Dep::Rust)],
        );
        let d = only(&d);
        assert_eq!(d.code, codes::V0101, "{d:#?}");
    }

    #[test]
    fn a_package_keyed_with_a_standard_name_gets_the_right_reason() {
        let mut build = units();
        let d = resolve_errors(
            &mut build,
            "use time::one;\nfn main() {}\n",
            &[("time", Dep::Package("units"))],
        );
        let d = only(&d);
        assert_eq!(d.code, codes::V0113);
        assert!(
            d.notes
                .iter()
                .any(|note| note.contains("`time` is a standard module, which comes first")),
            "{d:#?}"
        );
    }

    #[test]
    fn a_dependency_s_module_present_as_vr_and_rs_loads_from_the_vr() {
        let mut build = Build::default();
        build.add("twin", &[]);
        let resolved = build
            .resolve("fn main() {}\n", &[("twin", Dep::Package("twin"))])
            .unwrap_or_else(|d| panic!("{d:#?}"));
        assert_eq!(sig(&resolved, "twin::part", "which").ret, I32);
        assert_eq!(
            callee(&resolved, "twin::part", "other"),
            Err(LookupError::Unknown)
        );
    }

    #[test]
    fn an_imported_function_keeps_its_modes_async_ness_and_borrowed_return() {
        let mut build = units_and_route();
        let resolved = build
            .resolve(
                "fn main() {}\n",
                &[
                    ("units", Dep::Package("units")),
                    ("route", Dep::Package("route")),
                ],
            )
            .unwrap_or_else(|d| panic!("{d:#?}"));
        let grow = sig(&resolved, "units::length", "grow");
        assert_eq!(grow.params[0].1, ParamMode::MutableBorrow);
        let add = sig(&resolved, "units::length", "add");
        assert_eq!(add.params[0].1, ParamMode::SharedBorrow);
        assert!(sig(&resolved, "units", "ready").is_async);
        assert!(!add.is_async);
        assert_eq!(sig(&resolved, "route", "longest").ret_root, Some(0));
        assert_eq!(add.ret_root, None);
        let meters = resolved
            .symbols
            .lookup_type(resolved.entry, Some(&p("units::length")), "Meters")
            .expect("Meters");
        let doubled = resolved
            .symbols
            .lookup_member(resolved.entry, meters, "doubled")
            .expect("doubled");
        let Callee::Imported(doubled) = doubled else {
            panic!("a package's method is imported: {doubled:?}");
        };
        let doubled = &resolved.symbols.imported[doubled.0 as usize];
        assert_eq!(
            (doubled.owner, doubled.self_mode, doubled.ret_root),
            (Some(meters), Some(ParamMode::SharedBorrow), None)
        );
    }

    #[test]
    fn a_package_reached_twice_gives_its_types_one_identity() {
        let mut build = units_and_route();
        let resolved = build
            .resolve(
                "fn main() {}\n",
                &[
                    ("units", Dep::Package("units")),
                    ("route", Dep::Package("route")),
                ],
            )
            .unwrap_or_else(|d| panic!("{d:#?}"));
        let meters: Vec<StructId> = (0..resolved.symbols.structs.len())
            .filter(|&id| resolved.symbols.structs[id].name == "Meters")
            .map(|id| StructId(id as u32))
            .collect();
        assert_eq!(meters.len(), 1, "one `Meters`");
        assert_eq!(sig(&resolved, "route", "total").ret, Ty::Struct(meters[0]));
        // A package the program does not list is imported, not named.
        let resolved = build
            .resolve("fn main() {}\n", &[("route", Dep::Package("route"))])
            .unwrap_or_else(|d| panic!("{d:#?}"));
        assert_eq!(
            callee(&resolved, "units::length", "add"),
            Err(LookupError::Unknown)
        );
    }

    #[test]
    fn a_pub_use_of_another_packages_item_is_v0001() {
        let mut build = units();
        let d = resolve_errors(
            &mut build,
            "pub use units::one;\nfn main() {}\n",
            &[("units", Dep::Package("units"))],
        );
        let d = only(&d);
        assert_eq!(d.code, codes::V0001);
        assert!(d.message.contains("another package"), "{d:#?}");
    }

    #[test]
    fn a_packages_pub_use_names_its_item_from_the_reexporting_module() {
        let mut build = Build::default();
        build.add("relib", &[]);
        let deps = [("relib", Dep::Package("relib"))];
        build
            .check(
                "fn main() {\n    let thing: relib::Thing = relib::item();\n    println!(\"{}\", thing.size);\n}\n",
                &deps,
            )
            .unwrap_or_else(|d| panic!("{d:#?}"));
        let resolved = build
            .resolve("fn main() {}\n", &deps)
            .unwrap_or_else(|d| panic!("{d:#?}"));
        // The short path and the item's own path name one function and
        // one struct, declared in `relib::db`.
        let short = callee(&resolved, "relib", "item");
        assert!(matches!(short, Ok(Callee::Imported(_))), "{short:?}");
        assert_eq!(short, callee(&resolved, "relib::db", "item"));
        let symbols = &resolved.symbols;
        let thing = symbols.lookup_type(resolved.entry, Some(&p("relib")), "Thing");
        assert!(matches!(thing, Ok(UserType::Struct(_))), "{thing:?}");
        assert_eq!(
            thing,
            symbols.lookup_type(resolved.entry, Some(&p("relib::db")), "Thing")
        );
        assert_eq!(
            sig(&resolved, "relib", "item")
                .package
                .as_ref()
                .map(|item| item.path.clone()),
            Some(vec!["db".to_string()])
        );
    }
}

/// A library's `pub` items in `pub` modules are used by other packages,
/// so a type they name must be one those packages can name too (M5b2 spec
/// 2.3): a `.vr` field, and a `.rs` field and method. The same files as a
/// program are fine.
#[test]
fn a_library_s_items_other_packages_use_must_name_types_they_can_see() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/resolve/library_hidden_types/src");
    let entry = || {
        let path = dir.join("lib.vr");
        let text = std::fs::read_to_string(&path).expect("fixture should exist");
        SourceFile::new(FileId(0), path, text)
    };
    let mut sources = Vec::new();
    let diagnostics = resolve_root(
        entry(),
        Kind::Library,
        None,
        Packages::default(),
        &mut sources,
    )
    .err()
    .unwrap_or_default();
    let mut found: Vec<(&str, String)> = diagnostics
        .iter()
        .map(|d| {
            let file = sources[d.span.file.0 as usize]
                .path
                .file_name()
                .map_or_else(String::new, |name| name.to_string_lossy().into_owned());
            (d.code, format!("{file}: {}", d.message))
        })
        .collect();
    found.sort();
    assert_eq!(
        found,
        [
            (
                codes::V0105,
                "api.rs: `Square` is in module `parts`, which other packages cannot see, but \
                 `Shelf` can be used from other packages; write `pub mod parts;`"
                    .to_string()
            ),
            (
                codes::V0105,
                "api.rs: `Square` is in module `parts`, which other packages cannot see, but \
                 `top` can be used from other packages; write `pub mod parts;`"
                    .to_string()
            ),
            (
                codes::V0105,
                "lib.vr: `Note` is in module `notes`, which other packages cannot see, but \
                 `Order` can be used from other packages; write `pub mod notes;` (or make \
                 `Order` private)"
                    .to_string()
            ),
        ],
        "{diagnostics:#?}"
    );

    // As a program, nothing outside uses it: no `main`, and nothing else.
    let mut sources = Vec::new();
    let diagnostics = resolve_root(
        entry(),
        Kind::Binary,
        None,
        Packages::default(),
        &mut sources,
    )
    .err()
    .unwrap_or_default();
    assert!(
        diagnostics.iter().all(|d| d.code != codes::V0105),
        "{diagnostics:#?}"
    );
}

// --- A type parameter chosen from the result (milestone 5b3 spec 2.1) ----

#[test]
fn a_deserialize_owned_result_has_a_hole_of_its_shape() {
    for (text, shape) in [
        (
            "pub fn one<T: serde::de::DeserializeOwned>(q: &'static str) -> Result<T, varyk_std::Error> { todo!() }",
            ResultShape::Plain,
        ),
        (
            "pub fn first<T: varyk_std::serde::de::DeserializeOwned>() -> Result<Option<T>, varyk_std::Error> { todo!() }",
            ResultShape::Option,
        ),
        (
            "pub async fn all<T: serde::de::DeserializeOwned>(id: i64) -> Result<Vec<T>, varyk_std::Error> { todo!() }",
            ResultShape::Vec,
        ),
    ] {
        let sig = mapped(text);
        assert!(sig.callable, "{text}: {:?}", sig.note);
        assert_eq!(sig.result_hole, Some(shape), "{text}");
        // The call writes `T`'s serde derive, whichever path the bound has.
        assert!(sig.names_std, "{text}");
    }
    let sig = mapped("pub fn load(id: i64) -> Result<i64, varyk_std::Error> { Ok(id) }");
    assert_eq!(sig.result_hole, None);
}

#[test]
fn another_generic_shape_is_not_callable_and_says_which() {
    for (text, reason) in [
        (
            "pub fn f<T: serde::de::DeserializeOwned, U: Clone>() -> Result<T, varyk_std::Error> { todo!() }",
            "two or more type parameters",
        ),
        (
            "pub fn f<T>() -> Result<T, varyk_std::Error> where T: serde::de::DeserializeOwned { todo!() }",
            "a `where` clause",
        ),
        (
            "pub fn f<T: Default>() -> Result<T, varyk_std::Error> { todo!() }",
            "no bound, or a bound other than `serde::de::DeserializeOwned`",
        ),
        (
            "pub fn f<'a, T: serde::de::DeserializeOwned>(s: &'a str) -> Result<T, varyk_std::Error> { todo!() }",
            "a lifetime parameter",
        ),
        (
            "pub fn f<T: serde::de::DeserializeOwned>(t: T) -> Result<T, varyk_std::Error> { todo!() }",
            "the type parameter in a parameter",
        ),
        (
            "pub fn f<T: serde::de::DeserializeOwned>() -> Result<(T, i64), varyk_std::Error> { todo!() }",
            "a return that is not one of those three shapes",
        ),
    ] {
        let sig = mapped(text);
        assert!(!sig.callable, "{text}");
        assert_eq!(sig.result_hole, None, "{text}");
        let note = sig.note.unwrap_or_default();
        assert!(
            note.contains("only with one type parameter `T: serde::de::DeserializeOwned`"),
            "{text}: {note}"
        );
        assert!(
            note.ends_with(&format!("this one has {reason}")),
            "{text}: {note}"
        );
    }
}
