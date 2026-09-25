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
    let path = format!("crates/varyk/tests/fixtures/errors/{case}/main.vr");
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

error_case!(v0001_unsupported_construct, "V0001", fix_it: false);
error_case!(v0002_break_outside_loop, "V0002", fix_it: false);
error_case!(v0003_unterminated_string, "V0003", fix_it: false);
error_case!(v0010_ampersand_at_call, "V0010", fix_it: true);
error_case!(v0011_reference_parameter_type, "V0011", fix_it: true);
error_case!(v0012_lifetime_parameter, "V0012", fix_it: true);
error_case!(v0100_unknown_name, "V0100", fix_it: false);
error_case!(v0101_unknown_type, "V0101", fix_it: false);
error_case!(v0102_unknown_field, "V0102", fix_it: false);
error_case!(v0103_binding_named_after_variant, "V0103", fix_it: false);
error_case!(v0103_duplicate_definition, "V0103", fix_it: false);
error_case!(v0103_struct_named_after_builtin_type, "V0103", fix_it: false);
error_case!(v0104_module_file_missing, "V0104", fix_it: false);
error_case!(v0104_rust_module_with_submodule, "V0104", fix_it: false);
error_case!(v0104_rust_module_with_include, "V0104", fix_it: false);
error_case!(v0104_rust_module_with_extern_crate, "V0104", fix_it: false);
error_case!(v0105_item_not_visible, "V0105", fix_it: true);
error_case!(v0105_private_struct_in_public_signature, "V0105", fix_it: true);
error_case!(v0106_missing_main, "V0106", fix_it: false);
error_case!(v0107_rust_string_type, "V0107", fix_it: true);
error_case!(v0108_unsupported_rust_signature, "V0108", fix_it: false);
error_case!(v0109_recursive_struct, "V0109", fix_it: false);
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
error_case!(v0304_param_into_struct, "V0304", fix_it: true);
error_case!(v0305_use_after_move, "V0305", fix_it: false);
error_case!(v0306_same_value_twice, "V0306", fix_it: false);
error_case!(v0306_used_while_lent, "V0306", fix_it: false);
error_case!(v0307_alias_changed, "V0307", fix_it: false);
error_case!(v0307_push_in_for, "V0307", fix_it: false);
