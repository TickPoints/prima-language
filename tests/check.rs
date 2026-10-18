use prima_runtime::check::{check_path_checked, check_src, check_src_checked};
use tempfile::TempDir;

#[test]
fn type_mismatch_f64_expr() {
    let errs = check_src("let x: F64 = sqrt(2)");
    assert_eq!(errs.len(), 1);
    assert!(
        errs[0].message.contains("F64"),
        "message = {}",
        errs[0].message
    );
    assert!(
        errs[0].message.contains("Expr"),
        "message = {}",
        errs[0].message
    );
}

#[test]
fn type_mismatch_integer_from_float() {
    let errs = check_src("let z: Integer = 3.14");
    assert_eq!(errs.len(), 1);
    assert!(
        errs[0].message.contains("Integer"),
        "message = {}",
        errs[0].message
    );
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

// ————— module/config diagnostics (spec appendix C.1) —————

#[test]
fn e0022_unknown_config_key_is_static_and_suggests() {
    let (errors, _) = check_src_checked("config { fractoin := true }\n1;");
    assert_eq!(errors.len(), 1, "errors: {errors:?}");
    assert_eq!(errors[0].code, "E0022");
    assert!(
        errors[0]
            .help
            .as_deref()
            .unwrap_or_default()
            .contains("fraction"),
        "expected a `fraction` suggestion: {:?}",
        errors[0].help
    );
}

#[test]
fn e0022_unknown_key_inside_with_config() {
    let (errors, _) = check_src_checked("with config { nonsense := true } {\n    1;\n}");
    assert_eq!(errors.len(), 1, "errors: {errors:?}");
    assert_eq!(errors[0].code, "E0022");
}

#[test]
fn e0030_missing_module_is_reported() {
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("main.pra");
    let src = "import missing;\n1;";
    std::fs::write(&file, src).unwrap();
    let (errors, _) = check_path_checked(&file, src);
    assert_eq!(errors.len(), 1, "errors: {errors:?}");
    assert_eq!(errors[0].code, "E0030");
    assert!(errors[0].message.contains("missing"));
}

#[test]
fn e0032_from_import_of_private_item() {
    let dir = TempDir::new().unwrap();
    let module = dir.path().join("m.pra");
    std::fs::write(
        &module,
        "pub let square(x) = x * x;\nlet helper(x) = x + 1;\n",
    )
    .unwrap();
    let file = dir.path().join("main.pra");
    let src = "from m import helper;\n1;";
    std::fs::write(&file, src).unwrap();
    let (errors, _) = check_path_checked(&file, src);
    assert_eq!(errors.len(), 1, "errors: {errors:?}");
    assert_eq!(errors[0].code, "E0032");
    assert!(errors[0].message.contains("helper"));
}

#[test]
fn e0032_qualified_access_of_private_item() {
    let dir = TempDir::new().unwrap();
    let module = dir.path().join("m.pra");
    std::fs::write(
        &module,
        "pub let square(x) = x * x;\nlet helper(x) = x + 1;\n",
    )
    .unwrap();
    let file = dir.path().join("main.pra");
    let src = "import m;\nm::helper(1);";
    std::fs::write(&file, src).unwrap();
    let (errors, _) = check_path_checked(&file, src);
    assert_eq!(errors.len(), 1, "errors: {errors:?}");
    assert_eq!(errors[0].code, "E0032");
}

#[test]
fn e0040_from_import_of_missing_item() {
    let dir = TempDir::new().unwrap();
    let module = dir.path().join("m.pra");
    std::fs::write(&module, "pub let square(x) = x * x;\n").unwrap();
    let file = dir.path().join("main.pra");
    let src = "from m import squar;\n1;";
    std::fs::write(&file, src).unwrap();
    let (errors, _) = check_path_checked(&file, src);
    assert_eq!(errors.len(), 1, "errors: {errors:?}");
    assert_eq!(errors[0].code, "E0040");
    assert!(
        errors[0]
            .help
            .as_deref()
            .unwrap_or_default()
            .contains("square"),
        "expected a `square` suggestion: {:?}",
        errors[0].help
    );
}
