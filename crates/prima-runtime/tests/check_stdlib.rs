//! `prima check` stdlib call-site validation (spec §18.4, §16.2 E0050): arity and argument-type
//! errors are caught at compile time by checking calls against signatures harvested from embedded
//! `.pra` signature modules. No impl registration is needed — the checker only reads sources.
//!
//! The `checktest` module path is unique to this test binary and its source is static, so
//! registering it is safe and idempotent even with tests running in parallel threads.

use prima_runtime::check::check_src;
use prima_runtime::stdlib::register_module_source;

/// Embedded signature module exercising scalar, string and `::`-joined item names.
const CHECKTEST_SRC: &str = "\
@builtin pub fn add(a: F64, b: F64) -> F64;\n\
@builtin pub fn greet(name: String) -> String;\n\
@builtin pub fn Matrix::zeros(rows: Integer, cols: Integer) -> Matrix<F64>;\n\
";

fn register_checktest() {
    register_module_source("checktest", CHECKTEST_SRC);
}

fn messages(src: &str) -> Vec<String> {
    check_src(src).into_iter().map(|e| e.message).collect()
}

#[test]
fn qualified_call_with_matching_arguments_passes() {
    register_checktest();
    assert!(check_src("import checktest; let x = checktest::add(1.0, 2.0);").is_empty());
}

#[test]
fn too_many_arguments_is_e0050() {
    register_checktest();
    let errs = messages("import checktest; let x = checktest::add(1.0, 2.0, 3.0);");
    assert_eq!(errs.len(), 1, "expected one arity error, got {errs:?}");
    assert!(errs[0].contains("expects 2 argument"), "got: {errs:?}");
}

#[test]
fn fewer_arguments_are_allowed_for_optional_trailing_params() {
    register_checktest();
    assert!(check_src("import checktest; let x = checktest::greet();").is_empty());
}

#[test]
fn integer_argument_rejected_for_string_param() {
    register_checktest();
    let errs = messages("import checktest; let x = checktest::greet(42);");
    assert_eq!(errs.len(), 1, "expected one type error, got {errs:?}");
    assert!(errs[0].contains("expects String"), "got: {errs:?}");
}

#[test]
fn string_argument_matches_string_param() {
    register_checktest();
    assert!(check_src("import checktest; let x = checktest::greet(\"hi\");").is_empty());
}

#[test]
fn matrix_zeros_qualified_call_passes() {
    register_checktest();
    assert!(check_src("import checktest; let m = checktest::Matrix::zeros(2, 2);").is_empty());
}

#[test]
fn matrix_zeros_type_mismatch_is_e0050() {
    register_checktest();
    let errs = messages("import checktest; let m = checktest::Matrix::zeros(1.5, 2);");
    assert_eq!(errs.len(), 1, "expected one type error, got {errs:?}");
    assert!(errs[0].contains("expects Integer"), "got: {errs:?}");
}

#[test]
fn from_import_exposes_bare_name_signature() {
    register_checktest();
    // Integer is assignable to F64 (implicit promotion, spec §6.3).
    assert!(check_src("from checktest import add; let y = add(1, 2);").is_empty());
}

#[test]
fn from_import_alias_resolves_to_signature() {
    register_checktest();
    assert!(check_src("from checktest import add as my_add; let z = my_add(1, 2);").is_empty());
}

#[test]
fn namespace_import_does_not_expose_bare_name() {
    // `import checktest` binds only `checktest::…`; a bare `add(...)` stays unresolved and is not
    // checked against the harvested signature (it would fail at runtime, not at compile time).
    register_checktest();
    assert!(check_src("import checktest; let x = add(1, 2, 3);").is_empty());
}

#[test]
fn unimported_module_is_not_checked() {
    // No `checktest` import at all: the call is not resolvable to a signature, so no error.
    register_checktest();
    assert!(check_src("let x = checktest::add(1.0, 2.0);").is_empty());
}
