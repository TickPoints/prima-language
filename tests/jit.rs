// JIT integration tests (spec §19.2/§19.4): auto hot-path compilation of MFn bodies, the `@jit`
// annotation, the `jit(...)` builtin, and reverse-mode `jit(grad(f))`. Native compilation comes from
// `prima-jit`; these tests assert the exact values the JIT path must produce, and the interpreted
// fallback must agree whenever compilation is unavailable.
use prima_core::{Number, Value};
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
fn mfn_auto_jit_keeps_integer_arguments_exact() {
    // Auto hot-path compilation is gated on all-`Real` arguments (spec §19.2): Integer/Rational
    // calls keep exact arithmetic even after the threshold, so a result never silently becomes F64.
    let mut ev = Evaluator::new();
    let v = ev
        .eval_value(
            "let f(x) = x^2 + 1;\nlet r = 0;\nfor i in 0..150 { r = f(i); }\n(1 / f(2), f(2))",
        )
        .expect("eval failed");
    let Value::Tuple(items) = &v else {
        panic!("expected a tuple, got {v:?}");
    };
    assert_eq!(items[1], Value::Number(Number::from(5)));
    match &items[0] {
        Value::Number(n @ Number::Rational(_)) => {
            assert!((n.to_f64_lossy() - 0.2).abs() < 1e-12, "1/5, got {n:?}");
        }
        other => panic!("expected the exact rational 1/5, got {other:?}"),
    }
    assert_eq!(
        ev.path_stats().mfn_jit_hits,
        0,
        "auto JIT must not serve Integer calls"
    );
}

#[test]
fn mfn_auto_jit_runs_for_real_arguments_and_keeps_exact_calls() {
    // Real arguments still trigger auto compilation, but the compiled artifact is only reused for
    // Real calls: a later Integer call must stay on the exact path (spec §19.2).
    let mut ev = Evaluator::new();
    let v = ev
        .eval_value(
            "let f(x) = x^2 + 1;\nlet r = 0.0;\nfor i in 0..120 { r = f(to_f64(i)); }\n(1 / f(2))",
        )
        .expect("eval failed");
    assert!(
        ev.path_stats().mfn_jit_hits > 0,
        "auto JIT must serve Real calls"
    );
    assert!(
        matches!(v, Value::Number(Number::Rational(_))),
        "Integer call after a Real-triggered compilation must stay exact, got {v:?}"
    );
}

#[test]
fn explicit_jit_accepts_integer_arguments() {
    // `@jit` is an explicit opt-in to F64 numeric execution (spec §19.2), so Integer arguments are
    // accepted and the result is Real.
    let mut ev = Evaluator::new();
    let v = ev
        .eval_value("let f(x) @jit = x^2 + 1;\nf(2)")
        .expect("eval failed");
    assert!(
        matches!(v, Value::Number(Number::Real(_))),
        "@jit opts into F64 execution, got {v:?}"
    );
    assert!(ev.path_stats().mfn_jit_hits > 0);
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
    let items = items.to_vec();
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
