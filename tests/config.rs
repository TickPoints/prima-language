use prima_core::{Number, Real, Value};
use prima_runtime::Evaluator;

fn eval(src: &str) -> Value {
    Evaluator::new().eval_value(src).expect("eval failed")
}

#[test]
fn fraction_false_yields_f64() {
    let v = eval("config { fraction := false }\n1/3");
    match v {
        Value::Number(Number::Real(Real::F64(x))) => assert!((x - 1.0 / 3.0).abs() < 1e-9, "got {x}"),
        other => panic!("expected F64, got {other:?}"),
    }
}

#[test]
fn fraction_default_is_exact() {
    let mut ev = Evaluator::new();
    let v = ev.eval_value("1/3").expect("eval failed");
    assert_eq!(ev.format_value(&v), "\\frac{1}{3}");
}

#[test]
fn broadcast_disabled_rejects_implicit() {
    let err = Evaluator::new().eval_value("config { broadcast := false }\nlet f(x) = x^2;\nf([1, 2, 3])");
    assert!(err.is_err(), "implicit broadcast should error when disabled");
}

#[test]
fn broadcast_op_works_when_disabled() {
    let v = eval("config { broadcast := false }\nlet f(x) = x^2;\nlet v = [1, 2, 3];\nv @. f");
    assert_eq!(
        v,
        Value::Array(vec![
            Value::Number(Number::from(1)),
            Value::Number(Number::from(4)),
            Value::Number(Number::from(9))
        ])
    );
}

#[test]
fn negative_base_fractional_pow_domain() {
    let v = eval("(-1)^0.5");
    match v {
        Value::Number(Number::Complex { re, im }) => {
            assert_eq!(*re, Number::from(0));
            assert_eq!(im.to_f64_lossy(), 1.0);
        }
        other => panic!("expected Complex, got {other:?}"),
    }
    assert!(Evaluator::new().eval_value("config { domain := real }\n(-1)^0.5").is_err());
}

#[test]
fn with_config_scopes_local_policy() {
    // Module-level complex; switching to real locally makes sqrt of a negative an error.
    let src = "config { domain := complex }\nwith config { domain := real } {\n    (-1)^0.5\n}";
    assert!(Evaluator::new().eval_value(src).is_err());
}

#[test]
fn division_by_zero_errors_by_default() {
    assert!(Evaluator::new().eval_value("0/0").is_err());
    assert!(Evaluator::new().eval_value("1/0").is_err());
}

#[test]
fn custom_zero_div_black_magic() {
    let v = eval("config { undefined_handling := custom { 0/0 := 1 } }\n0/0");
    assert_eq!(v, Value::Number(Number::from(1)));
}

#[test]
fn unknown_config_key_rejected() {
    assert!(Evaluator::new().eval_value("config { nonsense := true }\n1").is_err());
}
