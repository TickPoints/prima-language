use prima_runtime::check::check_src;

#[test]
fn type_mismatch_f64_expr() {
    let errs = check_src("let x: F64 = sqrt(2)");
    assert_eq!(errs.len(), 1);
    assert!(errs[0].message.contains("F64"), "message = {}", errs[0].message);
    assert!(errs[0].message.contains("Expr"), "message = {}", errs[0].message);
}

#[test]
fn type_mismatch_integer_from_float() {
    let errs = check_src("let z: Integer = 3.14");
    assert_eq!(errs.len(), 1);
    assert!(errs[0].message.contains("Integer"), "message = {}", errs[0].message);
}

#[test]
fn type_mismatch_string_from_number() {
    let errs = check_src("let s: String = 5");
    assert_eq!(errs.len(), 1);
}

#[test]
fn collapse_return_type_satisfies_annotation() {
    assert!(check_src("let y: F64 = to_f64(sqrt(2))").is_empty());
}

#[test]
fn numeric_promotion_allowed() {
    assert!(check_src("let n: Integer = 7;\nlet r: F64 = 1;\nlet q: Rational = 1/2").is_empty());
}

#[test]
fn collect_multiple_errors_in_order() {
    let errs = check_src("let a: F64 = sqrt(2);\nlet b: String = 3;\nlet c: Bool = 1");
    assert_eq!(errs.len(), 3);
    assert!(errs[0].line <= errs[1].line && errs[1].line <= errs[2].line);
}

#[test]
fn syntax_error_surfaces_as_check_error() {
    let errs = check_src("let x = 1 +");
    assert!(!errs.is_empty());
}

#[test]
fn const_annotations_checked() {
    let errs = check_src("const c: F64 = \"hi\"");
    assert_eq!(errs.len(), 1);
}
