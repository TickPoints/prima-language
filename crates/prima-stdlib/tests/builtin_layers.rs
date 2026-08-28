use prima_core::Value;
use prima_runtime::Evaluator;

/// Evaluate an in-memory program importing `num` (spec §B.5), with the given `opt_level` config
/// prepended so the layered `@builtin(O1)` path (`num::fibonacci`) can be exercised at each tier.
fn eval_at(opt_level: &str, src: &str) -> Value {
    prima_stdlib::init();
    let program = format!("config {{ opt_level := {opt_level} }}\n{src}");
    Evaluator::new().eval_value(&program).expect("eval failed")
}

/// The `n`-th Fibonacci number as a `BigInt`-backed integer, recovered from a numeric `Value`.
fn fib_int(v: &Value) -> i64 {
    match v {
        Value::Number(n) => n.as_i64().expect("expected an integer"),
        other => panic!("expected Number, got {other:?}"),
    }
}

#[test]
fn layered_fibonacci_native_matches_pra() {
    let src = "import num;\nnum::fibonacci(0);\nnum::fibonacci(1);\nnum::fibonacci(10);\nnum::fibonacci(20)";
    // `opt_level := O0` falls back to the `.pra` body; `opt_level := O2` (default) uses the Rust
    // implementation (spec §18.4). Both must agree — the `.pra` body is the semantic authority.
    let o0 = eval_at("O0", src);
    let o2 = eval_at("O2", src);
    assert_eq!(fib_int(&o0), 6765);
    assert_eq!(fib_int(&o2), 6765);
    assert_eq!(o0, o2);
}

#[test]
fn layered_fibonacci_small_values() {
    assert_eq!(fib_int(&eval_at("O0", "import num;\nnum::fibonacci(0)")), 0);
    assert_eq!(fib_int(&eval_at("O1", "import num;\nnum::fibonacci(1)")), 1);
    assert_eq!(fib_int(&eval_at("O1", "import num;\nnum::fibonacci(6)")), 8);
    assert_eq!(
        fib_int(&eval_at("O3", "import num;\nnum::fibonacci(15)")),
        610
    );
}

#[test]
fn builtin_o0_signature_only_requires_registered_impl() {
    // A bare `@builtin` (O0) at an unregistered name errors with E0055 (spec §18.4).
    prima_stdlib::init();
    let err = Evaluator::new()
        .eval_value("@builtin\npub fn not_a_real_builtin() -> Integer;")
        .unwrap_err();
    assert!(err.to_string().contains("E0055"), "got: {err}");
}
