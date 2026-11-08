#![cfg(feature = "advanced")]
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
fn math_taylor_polynomials() {
    let src = "import math; math::taylor(sin(x), x, 0, 5)";
    let expected = "x - x^3/6 + x^5/120";
    assert_eq!(eval(src), eval(expected));
    assert_eq!(
        eval("import math; math::taylor(x^2, x, 1, 0)"),
        Value::Number(Number::from(1))
    );
    assert_eq!(
        eval("import math; math::taylor(x^2, x, 1, 2)"),
        eval("1 + 2*(x-1) + (x-1)^2")
    );
    assert_eq!(
        eval("import math; let f(x) = x^2; math::taylor(f, x, 0, 2)"),
        eval("x^2")
    );
    assert_eq!(
        eval("import math; math::taylor(|x| x^2, x, 0, 2)"),
        eval("x^2")
    );
    assert_eq!(
        eval("from math import taylor as series; series(x^2, x, 0, 2)"),
        eval("x^2")
    );
    assert_eq!(
        eval("import math; fn f() { return math::taylor(x^2, x, 0, 2); } f()"),
        eval("x^2")
    );
    assert_eq!(
        eval(
            "from math import taylor as series; fn f() { let v = series(x^2, x, 0, 2); return v; } f()"
        ),
        eval("x^2")
    );
    assert_eq!(
        eval(
            "from math import taylor as series; fn f() { let v = series(1, \"x\", 0, 1); return v; } f()"
        ),
        Value::Number(Number::from(1))
    );
}

#[test]
fn math_taylor_invalid_arguments() {
    for call in [
        "math::taylor(x, x, 0, -1)",
        "math::taylor(x, x, 0, 1.5)",
        "math::taylor(x, x, 0)",
        "math::taylor(x, 1, 0, 1)",
    ] {
        eval_err(&format!("import math; {call}"));
    }
}

#[test]
fn math_taylor_function_parameter_substitution() {
    assert_eq!(
        eval("import math; let f(t) = t^2 + t; math::taylor(f, x, 1, 2)"),
        eval("1 + 1 + 3*(x-1) + (x-1)^2")
    );
    assert_eq!(
        eval("import math; math::taylor(|t| t^2 + t, x, 1, 2)"),
        eval("2 + 3*(x-1) + (x-1)^2")
    );
    assert_eq!(
        eval("import math; math::taylor(x^2, x, 0, 1000000)"),
        eval("x^2")
    );
    assert_eq!(
        eval("import math; math::taylor(sin(x), x, 0, 11)"),
        eval("x - x^3/6 + x^5/120 - x^7/5040 + x^9/362880 - x^11/39916800")
    );
    for call in [
        "math::taylor(x, x, 0, 999999999999999999999999999)",
        "math::taylor(|a, b| a + b, x, 0, 2)",
    ] {
        let error = eval_err(&format!("import math; {call}"));
        assert!(error.to_string().contains("math::taylor"), "{error}");
    }
}

#[test]
fn math_taylor_rejects_unsupported_derivatives() {
    use prima_core::{ExprPool, SymbolTable};
    use prima_runtime::{Env, Function, NamespaceItem};

    fn unknown(_: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
        let Value::Expr(arg) = args[0] else {
            panic!("expected symbolic argument")
        };
        let pool = ExprPool::global();
        let f = pool.symbol(SymbolTable::global().intern("unknown_function"));
        Ok(Value::Expr(pool.apply(f, &[arg])))
    }

    prima_stdlib::init();
    prima_runtime::stdlib::register_namespace(
        "taylor_test",
        [(
            "unknown".to_string(),
            NamespaceItem::Func(Function::Native {
                name: "unknown",
                call: unknown,
            }),
        )]
        .into_iter()
        .collect(),
    );
    let mut ev = Evaluator::new();
    for source in [
        "import math; import taylor_test; math::taylor(taylor_test::unknown(x), x, 0, 1)",
        "import math; import taylor_test; let f(t) = taylor_test::unknown(t); math::taylor(f, x, 0, 1)",
        "import math; import taylor_test; math::taylor(|t| taylor_test::unknown(t), x, 0, 1)",
    ] {
        let env = Env::new().into_ref();
        let error = ev
            .eval_value_keep_env(&env, source)
            .expect_err("unsupported derivative");
        assert!(
            error
                .to_string()
                .contains("cannot differentiate an unsupported function"),
            "{error}"
        );
    }
    assert!(
        ev.eval_value_keep_env(
            &Env::new().into_ref(),
            "import math; import taylor_test; math::taylor(taylor_test::unknown(x), x, 0, 0)"
        )
        .is_ok()
    );
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
fn math_factor_large_semiprime_uses_pollard_rho() {
    // Trial division to `sqrt(n)` would need ~10^9 iterations; Pollard rho finds both factors.
    let p = 1_000_000_007i64;
    let q = 1_000_000_009i64;
    let n = (p as i128) * (q as i128);
    assert_eq!(
        ints(&eval(&format!("import math;\nmath::factor({n})"))),
        vec![p, q]
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
