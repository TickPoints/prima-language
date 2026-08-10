use prima_syntax::parse;

#[test]
fn duplicate_config_rejected() {
    let errs = parse("config {}\nconfig {}").unwrap_err();
    assert!(errs[0].message.contains("config"));
}

#[test]
fn config_after_import_rejected() {
    let errs = parse("import foo\nconfig {}").unwrap_err();
    assert!(errs[0].message.contains("config"));
}

#[test]
fn import_after_statement_rejected() {
    let errs = parse("let x = 1\nimport foo").unwrap_err();
    assert!(errs[0].message.contains("import"));
}

#[test]
fn missing_rhs_rejected() {
    let errs = parse("let x =").unwrap_err();
    assert!(errs[0].message.contains("expression"));
}

#[test]
fn unmatched_paren_rejected() {
    let errs = parse("let x = (1 + 2").unwrap_err();
    assert!(errs[0].message.contains("`"));
}

#[test]
fn missing_catch_rejected() {
    let errs = parse("try { let x = 1 }").unwrap_err();
    assert!(errs[0].message.contains("catch"));
}

#[test]
fn unknown_annotation_rejected() {
    let errs = parse("let f(x) @wat = x").unwrap_err();
    assert!(errs[0].message.contains("annotation"));
}

#[test]
fn unclosed_block_rejected() {
    let errs = parse("fn f() -> F64 { let x = 1").unwrap_err();
    assert!(errs[0].message.contains("expression"));
}

#[test]
fn unterminated_string_is_error() {
    let errs = parse("let s = \"abc").unwrap_err();
    assert!(errs[0].message.contains("string"));
}
