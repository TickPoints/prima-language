use prima_runtime::stdlib::{get_impl, get_module_source};

/// The embedded stdlib signature modules and their `@builtin` implementations are registered by
/// `prima_stdlib::init()` (spec §18.4). This suite asserts the type surface is present on the
/// registry side: each module source exists, the declared signatures carry through verbatim, and
/// the fully-qualified impl keys resolve.
#[test]
fn embedded_signature_modules_are_registered() {
    prima_stdlib::init();

    // `linalg` source with the declared `determinant` signature (spec appendix B.2).
    let linalg = get_module_source("linalg").expect("linalg source registered");
    assert!(
        linalg.contains("determinant(M: Matrix<F64>) -> F64"),
        "linalg.pra must declare the determinant signature, got:\n{linalg}"
    );

    // Nested `sys::path` and `time` module sources resolve by their full module path.
    assert!(get_module_source("sys::path").is_some(), "sys::path source registered");
    assert!(get_module_source("time").is_some(), "time source registered");
}

#[test]
fn builtin_impls_are_registered() {
    prima_stdlib::init();

    // Impls are keyed by fully-qualified `module::name` (spec §18.4), including the flattened
    // `::`-joined item names.
    assert!(get_impl("linalg::determinant").is_some(), "linalg::determinant impl registered");
    assert!(get_impl("time::now").is_some(), "time::now impl registered");
    assert!(get_impl("linalg::Matrix::zeros").is_some(), "linalg::Matrix::zeros impl registered");
    assert!(get_impl("sys::path::join").is_some(), "sys::path::join impl registered");
}
