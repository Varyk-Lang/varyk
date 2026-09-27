//! End-to-end diagnostic snapshots: one erroneous program per
//! diagnostic code under `tests/fixtures/errors/<case>/main.vr`, checked
//! with the real `varyk check` binary. Each snapshot is the human
//! rendering on stderr; each case must produce exactly one diagnostic,
//! with exactly its code. V0002 is also covered by `tests/cli.rs`.

mod common;

use common::varyk;

/// Runs `varyk check` on the case's entry file and returns its stderr,
/// after asserting the case failed with exactly one diagnostic, headed
/// `error[<code>]`, with a `-->` location line, and (for fix-it cases)
/// a `help:` line.
fn check_case(case: &str, code: &str, has_fix_it: bool) -> String {
    check_entry(case, "main.vr", code, has_fix_it)
}

/// Runs `varyk check` on the case's `entry` file; see [`check_case`].
fn check_entry(case: &str, entry: &str, code: &str, has_fix_it: bool) -> String {
    let path = format!("crates/varyk/tests/fixtures/errors/{case}/{entry}");
    let output = varyk(&["check", &path]);

    assert_eq!(output.status.code(), Some(1), "{case}: {:?}", output.status);
    assert!(
        output.stdout.is_empty(),
        "{case}: expected no stdout, got: {:?}",
        String::from_utf8_lossy(&output.stdout)
    );
    let stderr = String::from_utf8(output.stderr).expect("stderr should be valid utf-8");

    assert!(
        stderr.starts_with(&format!("error[{code}]: ")),
        "{case}: expected an `error[{code}]` header, got:\n{stderr}"
    );
    assert_eq!(
        stderr.matches("error[").count(),
        1,
        "{case}: expected exactly one diagnostic, got:\n{stderr}"
    );
    assert!(
        stderr
            .lines()
            .any(|line| line.trim_start().starts_with("--> ")),
        "{case}: expected a `-->` line, got:\n{stderr}"
    );
    assert_eq!(
        stderr.lines().any(|line| line.starts_with("help: ")),
        has_fix_it,
        "{case}: `help:` line presence should be {has_fix_it}, got:\n{stderr}"
    );
    stderr
}

/// One test per case: `name`, the fixture directory, its code, and
/// whether it carries a fix-it.
macro_rules! error_case {
    ($name:ident, $code:literal, fix_it: $fix_it:literal) => {
        #[test]
        fn $name() {
            let stderr = check_case(stringify!($name), $code, $fix_it);
            insta::assert_snapshot!(stderr);
        }
    };
}

/// One test per package case (a `Cargo.toml` and its root file, M3 spec
/// 2.1, 2.2), checked through `$entry`, the root file.
macro_rules! package_error_case {
    ($name:ident, $code:literal, $entry:literal) => {
        #[test]
        fn $name() {
            let stderr = check_entry(stringify!($name), $entry, $code, false);
            insta::assert_snapshot!(stderr);
        }
    };
}

error_case!(v0001_unsupported_construct, "V0001", fix_it: false);
error_case!(v0001_use_grouped_import, "V0001", fix_it: false);
error_case!(v0001_use_glob_import, "V0001", fix_it: false);
error_case!(v0001_pub_paren_crate_fn, "V0001", fix_it: false);
error_case!(v0001_pub_paren_super_fn, "V0001", fix_it: false);
error_case!(v0001_super_super, "V0001", fix_it: false);
error_case!(v0001_pub_paren_field, "V0001", fix_it: false);
error_case!(v0001_pub_use, "V0001", fix_it: false);
error_case!(v0002_break_outside_loop, "V0002", fix_it: false);
error_case!(v0003_unterminated_string, "V0003", fix_it: false);
error_case!(v0010_ampersand_at_call, "V0010", fix_it: true);
error_case!(v0011_reference_parameter_type, "V0011", fix_it: true);
error_case!(v0012_lifetime_parameter, "V0012", fix_it: true);
error_case!(v0100_unknown_name, "V0100", fix_it: false);
error_case!(v0100_opaque_enum_variant, "V0100", fix_it: false);
error_case!(v0100_cfg_rust_enum_variant, "V0100", fix_it: false);
error_case!(v0100_skipped_rust_method, "V0100", fix_it: false);
error_case!(v0100_restricted_rust_method, "V0100", fix_it: false);
error_case!(v0100_cfg_rust_function, "V0100", fix_it: false);
error_case!(v0100_const_rust_function, "V0100", fix_it: false);
error_case!(v0100_rust_enum_method, "V0100", fix_it: false);
error_case!(v0100_rust_inline_module, "V0100", fix_it: false);
error_case!(v0101_unknown_type, "V0101", fix_it: false);
error_case!(v0101_generic_rust_struct, "V0101", fix_it: false);
error_case!(v0101_restricted_rust_struct, "V0101", fix_it: false);
error_case!(v0101_use_generic_rust_struct, "V0101", fix_it: false);
error_case!(v0101_unit_rust_struct_value, "V0101", fix_it: false);
error_case!(v0101_tuple_rust_struct_call, "V0101", fix_it: false);
error_case!(v0101_generic_rust_enum_pattern, "V0101", fix_it: false);
error_case!(v0101_packed_rust_struct, "V0101", fix_it: false);
error_case!(v0101_unsized_rust_struct, "V0101", fix_it: false);
error_case!(v0101_cfg_rust_struct, "V0101", fix_it: false);
error_case!(v0101_pub_use_rust_type, "V0101", fix_it: false);
error_case!(v0102_unknown_field, "V0102", fix_it: false);
error_case!(v0103_binding_named_after_variant, "V0103", fix_it: false);
error_case!(v0103_duplicate_definition, "V0103", fix_it: false);
error_case!(v0103_struct_named_after_builtin_type, "V0103", fix_it: false);
error_case!(v0104_module_file_missing, "V0104", fix_it: false);
error_case!(v0104_rust_module_with_submodule, "V0104", fix_it: false);
error_case!(v0104_rust_module_with_include, "V0104", fix_it: false);
error_case!(v0104_rust_module_with_extern_crate, "V0104", fix_it: false);
error_case!(v0104_mod_vr_twin, "V0104", fix_it: false);
error_case!(v0104_mod_bin_in_root, "V0104", fix_it: false);
error_case!(v0104_mod_capitalised_main, "V0104", fix_it: false);
error_case!(v0105_item_not_visible, "V0105", fix_it: true);
error_case!(v0105_private_struct_in_public_signature, "V0105", fix_it: true);
error_case!(v0105_private_module, "V0105", fix_it: true);
error_case!(v0105_private_module_in_public_signature, "V0105", fix_it: true);
error_case!(v0105_private_field, "V0105", fix_it: true);
error_case!(v0105_private_field_in_literal, "V0105", fix_it: true);
error_case!(v0105_rust_struct_literal_hidden_field, "V0105", fix_it: false);
error_case!(v0105_rust_fn_pub_crate, "V0105", fix_it: false);
package_error_case!(v0104_rust_crate_not_a_dependency, "V0104", "src/main.vr");
package_error_case!(v0104_rust_crate_dev_dependency, "V0104", "src/main.vr");
package_error_case!(v0106_main_in_library, "V0106", "src/lib.vr");
error_case!(v0106_missing_main, "V0106", fix_it: false);
error_case!(v0107_rust_string_type, "V0107", fix_it: true);
error_case!(v0108_unsupported_rust_signature, "V0108", fix_it: false);
error_case!(v0108_unusable_rust_field, "V0108", fix_it: false);
error_case!(v0108_rust_glob_use, "V0108", fix_it: false);
error_case!(v0108_rust_macro_defines_names, "V0108", fix_it: false);
error_case!(v0108_rust_type_through_use, "V0108", fix_it: false);
error_case!(v0108_rust_type_behind_private_module, "V0108", fix_it: false);
error_case!(v0108_unit_inside_rust_type, "V0108", fix_it: false);
error_case!(v0108_rust_alias_of_a_mapped_name, "V0108", fix_it: false);
error_case!(v0109_recursive_struct, "V0109", fix_it: false);
error_case!(v0110_use_crate_std, "V0110", fix_it: false);
error_case!(v0111_super_in_root, "V0111", fix_it: false);
error_case!(v0111_use_variant, "V0111", fix_it: false);
error_case!(v0111_use_variant_in_module, "V0111", fix_it: false);
error_case!(v0111_use_through_alias, "V0111", fix_it: false);
error_case!(v0111_use_module_declared_elsewhere, "V0111", fix_it: false);
error_case!(v0103_use_name_clash, "V0103", fix_it: false);
error_case!(v0200_type_mismatch, "V0200", fix_it: false);
error_case!(v0201_wrong_argument_count, "V0201", fix_it: false);
error_case!(v0202_placeholder_count, "V0202", fix_it: false);
error_case!(v0203_print_struct, "V0203", fix_it: false);
error_case!(v0204_missing_variant, "V0204", fix_it: false);
error_case!(v0205_wrong_arity, "V0205", fix_it: false);
error_case!(v0206_question_outside_result, "V0206", fix_it: false);
error_case!(v0207_none_needs_type, "V0207", fix_it: false);
error_case!(v0207_vec_new_needs_type, "V0207", fix_it: false);
error_case!(v0300_change_through_param, "V0300", fix_it: true);
error_case!(v0300_push_on_borrowed, "V0300", fix_it: true);
error_case!(v0301_assign_immutable_let, "V0301", fix_it: true);
error_case!(v0302_immutable_let_to_mut_param, "V0302", fix_it: true);
error_case!(v0303_param_to_mut_param, "V0303", fix_it: true);
error_case!(v0303_drop_payload_to_mut_param, "V0303", fix_it: true);
error_case!(v0303_drop_unsure_from_macro, "V0303", fix_it: true);
error_case!(v0303_match_binding_to_mut_param, "V0303", fix_it: true);
error_case!(v0304_param_into_struct, "V0304", fix_it: true);
error_case!(v0304_drop_binding_kept_by_match, "V0304", fix_it: true);
error_case!(v0305_use_after_move, "V0305", fix_it: false);
error_case!(v0306_same_value_twice, "V0306", fix_it: false);
error_case!(v0306_used_while_lent, "V0306", fix_it: false);
error_case!(v0307_alias_changed, "V0307", fix_it: false);
error_case!(v0307_push_in_for, "V0307", fix_it: false);
package_error_case!(v0400_edition_2021, "V0400", "src/main.vr");
package_error_case!(v0401_workspace_true, "V0401", "src/main.vr");
package_error_case!(v0401_target_dependencies, "V0401", "src/main.vr");
package_error_case!(v0402_target_path, "V0402", "src/main.vr");
package_error_case!(v0403_unreadable_manifest, "V0403", "src/main.vr");
package_error_case!(v0403_invalid_toml, "V0403", "src/main.vr");
package_error_case!(v0403_missing_name, "V0403", "src/main.vr");
package_error_case!(v0403_both_roots, "V0403", "src/main.vr");
package_error_case!(v0403_invalid_name, "V0403", "src/main.vr");
package_error_case!(v0403_program_name, "V0403", "src/main.vr");
