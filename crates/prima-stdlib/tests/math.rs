use prima_core::{Number, Value};
use prima_runtime::{Evaluator, RuntimeError};

/// Evaluate an in-memory program that imports Rust-hosted stdlib namespaces (spec §18).
fn eval(src: &str) -> Value {
    prima_stdlib::init();
    Evaluator::new().eval_value(src).expect("eval failed")
}

/// Evaluate a program expected to fail with a runtime error.
fn eval_err(src: &str) -> RuntimeError {
    prima_stdlib::init();
    Evaluator::new()
        .eval_value(src)
        .expect_err("expected a runtime error")
}

/// Extract an `Array<Integer>` result as `i64`s.
fn ints(v: &Value) -> Vec<i64> {
    match v {
        Value::Array(items) => items
            .iter()
            .map(|x| match x {
                Value::Number(n) => n.as_i64().expect("integer element"),
                other => panic!("expected integer element, got {other:?}"),
            })
            .collect(),
        other => panic!("expected Array, got {other:?}"),
    }
}

/// Extract an `Array<F64>` result as `f64`s.
fn floats(v: &Value) -> Vec<f64> {
    match v {
        Value::Array(items) => items
            .iter()
            .map(|x| match x {
                Value::Number(n) => n.to_f64_lossy(),
                other => panic!("expected number element, got {other:?}"),
            })
            .collect(),
        other => panic!("expected Array, got {other:?}"),
    }
}

/// The `(re, im)` parts of a complex result.
fn complex_parts(v: &Value) -> (f64, f64) {
    match v {
        Value::Number(n) => match n.as_complex_parts() {
            Some((re, im)) => (re.to_f64_lossy(), im.to_f64_lossy()),
            None => (n.to_f64_lossy(), 0.0),
        },
        other => panic!("expected Number, got {other:?}"),
    }
}

#[test]
fn math_gcd_lcm() {
    assert_eq!(
        eval("import math;\nmath::gcd(12, 18)"),
        Value::Number(Number::from(6))
    );
    assert_eq!(
        eval("import math;\nmath::gcd(0, 5)"),
        Value::Number(Number::from(5))
    );
    assert_eq!(
        eval("import math;\nmath::lcm(4, 6)"),
        Value::Number(Number::from(12))
    );
    assert_eq!(
        eval("import math;\nmath::lcm(0, 5)"),
        Value::Number(Number::from(0))
    );
}

#[test]
fn math_factor() {
    assert_eq!(
        ints(&eval("import math;\nmath::factor(360)")),
        vec![2, 2, 2, 3, 3, 5]
    );
    assert_eq!(ints(&eval("import math;\nmath::factor(97)")), vec![97]);
    assert_eq!(
        ints(&eval("import math;\nmath::factor(1)")),
        Vec::<i64>::new()
    );
}

#[test]
fn math_factor_rejects_non_positive() {
    assert_eq!(eval_err("import math;\nmath::factor(0)").kind(), "Message");
    assert_eq!(eval_err("import math;\nmath::factor(-7)").kind(), "Message");
}

#[test]
fn math_primes() {
    assert_eq!(
        ints(&eval("import math;\nmath::primes(30)")),
        vec![2, 3, 5, 7, 11, 13, 17, 19, 23, 29]
    );
    assert_eq!(ints(&eval("import math;\nmath::primes(2)")), vec![2]);
    assert_eq!(
        ints(&eval("import math;\nmath::primes(1)")),
        Vec::<i64>::new()
    );
    assert_eq!(
        ints(&eval("import math;\nmath::primes(0)")),
        Vec::<i64>::new()
    );
}

#[test]
fn math_primes_fallback_matches_native() {
    // `primes` is a layered `@builtin(O1)`: force the `.pra` fallback with `opt_level := O0`.
    let native = eval("import math;\nmath::primes(50)");
    let fallback = eval("config { opt_level := O0 }\nimport math;\nmath::primes(50)");
    assert_eq!(native, fallback);
}

#[test]
fn math_crt() {
    match eval("import math;\nmath::crt([2, 3, 2], [3, 5, 7])") {
        Value::Result(Ok(v)) => assert_eq!(*v, Value::Number(Number::from(23))),
        other => panic!("expected Result<Ok>, got {other:?}"),
    }
}

#[test]
fn math_crt_errors() {
    // Length mismatch.
    match eval("import math;\nmath::crt([1, 2], [3, 5, 7])") {
        Value::Result(Err(_)) => {}
        other => panic!("expected Result<Err>, got {other:?}"),
    }
    // Non-pairwise-coprime moduli.
    match eval("import math;\nmath::crt([1, 2], [4, 6])") {
        Value::Result(Err(_)) => {}
        other => panic!("expected Result<Err>, got {other:?}"),
    }
}

#[test]
fn math_mod_pow() {
    assert_eq!(
        eval("import math;\nmath::mod_pow(2, 10, 1000)"),
        Value::Number(Number::from(24))
    );
    assert_eq!(
        eval("import math;\nmath::mod_pow(3, 0, 7)"),
        Value::Number(Number::from(1))
    );
    assert_eq!(
        eval("import math;\nmath::mod_pow(5, 3, 100)"),
        Value::Number(Number::from(25))
    );
    // A negative base is reduced to its least non-negative residue.
    assert_eq!(
        eval("import math;\nmath::mod_pow(-2, 3, 5)"),
        Value::Number(Number::from(2))
    );
}

#[test]
fn math_mod_pow_rejects_invalid() {
    assert_eq!(
        eval_err("import math;\nmath::mod_pow(2, -1, 5)").kind(),
        "Message"
    );
    assert_eq!(
        eval_err("import math;\nmath::mod_pow(2, 3, 0)").kind(),
        "Message"
    );
}

#[test]
fn math_poly_eval() {
    // 1 + 2x + 3x^2 at x = 2 -> 17.
    assert_eq!(
        floats(&eval(
            "import math;\n[math::poly_eval([1.0, 2.0, 3.0], 2.0)]"
        )),
        vec![17.0]
    );
    assert_eq!(
        floats(&eval("import math;\n[math::poly_eval([], 5.0)]")),
        vec![0.0]
    );
}

#[test]
fn math_poly_add() {
    assert_eq!(
        floats(&eval(
            "import math;\nmath::poly_add([1.0, 2.0], [3.0, 4.0, 5.0])"
        )),
        vec![4.0, 6.0, 5.0]
    );
}

#[test]
fn math_poly_mul() {
    assert_eq!(
        floats(&eval(
            "import math;\nmath::poly_mul([1.0, 1.0], [1.0, 1.0])"
        )),
        vec![1.0, 2.0, 1.0]
    );
    assert_eq!(
        ints(&eval("import math;\n[len(math::poly_mul([], [1.0]))]")),
        vec![0]
    );
}

#[test]
fn math_poly_derivative() {
    assert_eq!(
        floats(&eval(
            "import math;\nmath::poly_derivative([1.0, 2.0, 3.0])"
        )),
        vec![2.0, 6.0]
    );
    assert_eq!(
        ints(&eval("import math;\n[len(math::poly_derivative([5.0]))]")),
        vec![0]
    );
}

#[test]
fn math_poly_roots_real() {
    // x^2 - 1 = [-1, 0, 1] -> roots -1 and 1 (real-axis noise snapped).
    let v = eval("import math;\nmath::poly_roots([-1.0, 0.0, 1.0])");
    let Value::Array(items) = v else {
        panic!("expected Array");
    };
    assert_eq!(items.len(), 2);
    let roots: Vec<(f64, f64)> = items.iter().map(|v| complex_parts(&v)).collect();
    assert!(
        (roots[0].0 + 1.0).abs() < 1e-9 && roots[0].1.abs() < 1e-9,
        "got {roots:?}"
    );
    assert!(
        (roots[1].0 - 1.0).abs() < 1e-9 && roots[1].1.abs() < 1e-9,
        "got {roots:?}"
    );
}

#[test]
fn math_poly_roots_complex() {
    // x^2 + 1 = [1, 0, 1] -> roots -i and +i (sorted by real then imaginary).
    let v = eval("import math;\nmath::poly_roots([1.0, 0.0, 1.0])");
    let Value::Array(items) = v else {
        panic!("expected Array");
    };
    let roots: Vec<(f64, f64)> = items.iter().map(|v| complex_parts(&v)).collect();
    assert_eq!(roots.len(), 2);
    assert!(
        roots[0].0.abs() < 1e-9 && (roots[0].1 + 1.0).abs() < 1e-9,
        "got {roots:?}"
    );
    assert!(
        roots[1].0.abs() < 1e-9 && (roots[1].1 - 1.0).abs() < 1e-9,
        "got {roots:?}"
    );
}

#[test]
fn math_poly_roots_cubic() {
    // x^3 - 6x^2 + 11x - 6 -> roots 1, 2, 3 (lowest-degree-first coefficients).
    let v = eval("import math;\nmath::poly_roots([-6.0, 11.0, -6.0, 1.0])");
    let Value::Array(items) = v else {
        panic!("expected Array");
    };
    let mut roots: Vec<f64> = items.iter().map(|v| complex_parts(&v).0).collect();
    roots.sort_by(|a, b| a.partial_cmp(b).unwrap());
    assert_eq!(roots.len(), 3);
    for (got, want) in roots.iter().zip([1.0, 2.0, 3.0]) {
        assert!((got - want).abs() < 1e-7, "got {roots:?}");
    }
    assert!(items.iter().all(|v| complex_parts(&v).1.abs() < 1e-7));
}

#[test]
fn math_poly_roots_degree_zero_is_empty() {
    assert_eq!(
        ints(&eval("import math;\n[len(math::poly_roots([5.0]))]")),
        vec![0]
    );
    // All-zero coefficients strip down to the zero polynomial: still no roots.
    assert_eq!(
        ints(&eval(
            "import math;\n[len(math::poly_roots([0.0, 0.0, 0.0]))]"
        )),
        vec![0]
    );
}

#[test]
fn math_continued_fraction() {
    // pi ~= [3; 7, 15, 1].
    assert_eq!(
        ints(&eval("import math;\nmath::continued_fraction(3.14159, 4)")),
        vec![3, 7, 15, 1]
    );
    // A terminating rational stops early: 1.5 = [1; 2].
    assert_eq!(
        ints(&eval("import math;\nmath::continued_fraction(1.5, 5)")),
        vec![1, 2]
    );
    assert_eq!(
        ints(&eval("import math;\nmath::continued_fraction(0.0, 3)")),
        vec![0]
    );
}

#[test]
fn math_continued_fraction_rejects_invalid_n() {
    assert_eq!(
        eval_err("import math;\nmath::continued_fraction(1.5, 0)").kind(),
        "Message"
    );
}
