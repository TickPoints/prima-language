// JIT integration tests (spec §19.2/§19.4): auto hot-path compilation of MFn bodies, the `@jit`
// annotation, the `jit(...)` builtin, and reverse-mode `jit(grad(f))`. Native compilation comes from
// `prima-jit`; these tests assert the exact values the JIT path must produce, and the interpreted
// fallback must agree whenever compilation is unavailable.
use prima_core::Value;
use prima_runtime::Evaluator;

fn eval(src: &str) -> Value {
    prima_stdlib::init();
    Evaluator::new().eval_value(src).expect("eval failed")
}

fn as_f64(v: &Value) -> f64 {
    match v {
        Value::Number(n) => n.to_f64_lossy(),
        _ => panic!("expected a number, got {v:?}"),
    }
}

#[test]
fn hot_path_stays_correct_past_threshold() {
    // More than `JIT_CALL_THRESHOLD` (100) numeric calls to the same MFn: the hot path (native or the
    // cached fallback) must stay correct.
    let src =
        "let f(x) = x^2 + 1;\nlet r = to_f64(0);\nfor i in 0..150 {\n    r = f(to_f64(3));\n}\nr";
    let v = eval(src);
    assert!((as_f64(&v) - 10.0).abs() < 1e-9, "expected 10.0, got {v:?}");
}

#[test]
fn jit_annotation_forces_compile_attempt() {
    // `@jit` compiles on the first numeric call (spec §19.2); with a failed compilation it falls back
    // to the interpreter and must still return the exact value.
    assert!((as_f64(&eval("let g(x) @jit = sin(x);\ng(to_f64(0))")) - 0.0).abs() < 1e-12);
    assert!((as_f64(&eval("let g(x) @jit = sin(x);\ng(to_f64(1))")) - 1.0_f64.sin()).abs() < 1e-9);
    assert!((as_f64(&eval("let g(x) @jit = x^2 + x;\ng(to_f64(3))")) - 12.0).abs() < 1e-9);
}

#[test]
fn jit_builtin_on_symbolic_expression() {
    let v = eval("let h = jit(x^2 + 1);\nh(to_f64(3))");
    assert!((as_f64(&v) - 10.0).abs() < 1e-9, "expected 10.0, got {v:?}");
}

#[test]
fn jit_builtin_on_mfn_name() {
    let v = eval("let f(x) = x^2 + 1;\nlet h = jit(f);\nh(to_f64(3))");
    assert!((as_f64(&v) - 10.0).abs() < 1e-9, "expected 10.0, got {v:?}");
}

#[test]
fn jit_grad_single_var() {
    // jit(grad(f)) with f = x^2 + 1 → derivative 2x → 6 at x = 3 (spec §19.4 stage 3).
    let v = eval("let f(x) = x^2 + 1;\nlet df = jit(grad(f));\ndf(to_f64(3))");
    assert!((as_f64(&v) - 6.0).abs() < 1e-9, "expected 6.0, got {v:?}");
}

#[test]
fn jit_grad_multi_var_returns_array() {
    // g(x, y) = x^2*y + y^3 → grad = [2xy, x^2 + 3y^2] → [4, 13] at (1, 2).
    let v = eval("let g(x, y) = x^2*y + y^3;\nlet dg = jit(grad(g));\ndg(to_f64(1), to_f64(2))");
    let Value::Array(items) = v else {
        panic!("expected an array, got {v:?}");
    };
    assert_eq!(items.len(), 2);
    assert!(
        (as_f64(&items[0]) - 4.0).abs() < 1e-9,
        "∂x = 4, got {}",
        as_f64(&items[0])
    );
    assert!(
        (as_f64(&items[1]) - 13.0).abs() < 1e-9,
        "∂y = 13, got {}",
        as_f64(&items[1])
    );
}

#[test]
fn jit_grad_on_symbolic_tuple() {
    // `grad(x^2 + 1)` is a symbolic tuple (spec §19.4): jit-ing it must yield the same derivative.
    let v = eval("let dh = jit(grad(x^2 + 1));\ndh(to_f64(3))");
    assert!((as_f64(&v) - 6.0).abs() < 1e-9, "expected 6.0, got {v:?}");
}

#[test]
fn jit_value_is_a_callable_handle() {
    // The `jit` result is a value handle, not a named function (spec §19.2).
    let mut ev = Evaluator::new();
    let v = ev.eval_value("jit(x^2 + 1)").expect("jit failed");
    assert!(
        matches!(v, Value::JitFunction(_)),
        "expected a JitFunction handle, got {v:?}"
    );
}

#[test]
fn jit_rejects_non_function_argument() {
    let err = Evaluator::new().eval_value("jit(42)").unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("`jit` argument must be a function"),
        "got: {msg}"
    );
}

#[test]
fn jit_function_requires_numeric_args() {
    // A JitFunction only accepts numeric (non-complex) arguments.
    let err = Evaluator::new()
        .eval_value("let h = jit(x^2 + 1);\nh(\"nope\")")
        .unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("numeric"), "got: {msg}");
}
