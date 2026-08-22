use prima_core::{Number, Real, Value};
use prima_runtime::Evaluator;

fn eval(src: &str) -> Value {
    Evaluator::new().eval_value(src).expect("eval failed")
}

fn eval_fmt(src: &str) -> String {
    let mut ev = Evaluator::new();
    let v = ev.eval_value(src).expect("eval failed");
    ev.format_value(&v)
}

#[test]
fn to_f64_of_symbolic_sqrt2() {
    match eval("to_f64(sqrt(2))") {
        Value::Number(Number::Real(Real::F64(x))) => assert!((x - std::f64::consts::SQRT_2).abs() < 1e-9),
        other => panic!("expected F64, got {other:?}"),
    }
}

#[test]
fn to_f64_of_builtin_const() {
    match eval("to_f64(\\pi)") {
        Value::Number(Number::Real(Real::F64(x))) => assert!((x - std::f64::consts::PI).abs() < 1e-9),
        other => panic!("expected F64, got {other:?}"),
    }
}

#[test]
fn to_i32_exact() {
    assert_eq!(eval("to_i32(42)"), Value::Number(Number::I32(42)));
}

#[test]
fn to_i32_overflow_is_runtime_error() {
    assert!(Evaluator::new().eval_value("to_i32(1e20)").is_err());
}

#[test]
fn try_i32_returns_result() {
    assert!(matches!(eval("try_i32(1e20)"), Value::Result(Err(_))));
    assert_eq!(eval("try_i32(7)"), Value::Result(Ok(Box::new(Value::Number(Number::I32(7))))));
}

#[test]
fn unwrap_or_direct_call() {
    assert_eq!(eval("unwrap_or(try_i32(1e20), 0)"), Value::Number(Number::from(0)));
    assert_eq!(eval("unwrap_or(try_i32(7), 0)"), Value::Number(Number::I32(7)));
}

#[test]
fn match_on_result() {
    assert_eq!(
        eval("match try_i32(7) {\n    Ok(n) => n\n    Err(e) => -1\n}"),
        Value::Number(Number::I32(7))
    );
    assert_eq!(
        eval("match try_i32(1e20) {\n    Ok(n) => n\n    Err(e) => -1\n}"),
        Value::Number(Number::from(-1))
    );
}

#[test]
fn checked_and_clamped_collapse() {
    assert_eq!(eval("checked_i32(5)"), Value::Result(Ok(Box::new(Value::Number(Number::I32(5))))));
    assert_eq!(eval("clamped_i32(1000, 0, 255)"), Value::Number(Number::I32(255)));
    assert_eq!(eval("clamped_f64(0.5, 0.0, 1.0)"), Value::Number(Number::Real(Real::F64(0.5))));
    assert_eq!(eval("truncated_i32(7/2)"), Value::Number(Number::from(3)));
    assert_eq!(eval_fmt("rounded_f64(\\pi, 3)"), "3.142");
}

#[test]
fn to_rational_preserves_exact() {
    assert_eq!(eval_fmt("to_rational(1/3)"), "\\frac{1}{3}");
}

#[test]
fn to_complex_wraps() {
    match eval("to_complex(3)") {
        Value::Number(Number::Complex { re, im }) => {
            assert_eq!(*re, Number::from(3));
            assert_eq!(*im, Number::from(0));
        }
        other => panic!("expected Complex, got {other:?}"),
    }
}
