use prima_core::{Number, Value};
use prima_runtime::Evaluator;

fn eval(src: &str) -> Value {
    prima_stdlib::init();
    Evaluator::new().eval_value(src).expect("eval failed")
}

fn eval_fmt(src: &str) -> String {
    let mut ev = Evaluator::new();
    let v = ev.eval_value(src).expect("eval failed");
    ev.format_value(&v)
}

#[test]
fn mathdef_call() {
    assert_eq!(
        eval("let f(x) = x^2;\nf(3)"),
        Value::Number(Number::from(9))
    );
}

#[test]
fn mfn_broadcasts_over_array() {
    let v = eval("let f(x) = x^2;\nf([1, 2, 3])");
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
fn symbolic_substitution_collapses() {
    assert_eq!(
        eval("let f(x) = x^2 + 6;\nf(sqrt(2))"),
        Value::Number(Number::from(8))
    );
}

#[test]
fn rational_division_is_exact() {
    assert_eq!(eval_fmt("1/3"), "\\frac{1}{3}");
    assert_eq!(eval("1/3 + 1/3 + 1/3"), Value::Number(Number::from(1)));
}

#[test]
fn tex_literal_renders_latex() {
    assert_eq!(eval_fmt(r#"tex"\sqrt{2} + \pi""#), "\\sqrt{2} + \\pi");
}

#[test]
fn euler_identity_collapses_to_zero() {
    assert_eq!(
        eval(r#"simplify(tex"\e^{i\pi} + 1")"#),
        Value::Number(Number::from(0))
    );
}

#[test]
fn symbolic_power_renders() {
    assert_eq!(eval_fmt("x^2"), "x^{2}");
}

#[test]
fn add_is_commutative_canonical() {
    assert_eq!(eval_fmt("1 + x"), "x + 1");
    assert_eq!(eval_fmt("x + 1"), "x + 1");
}

#[test]
fn nested_add_cancels() {
    assert_eq!(eval_fmt("simplify(x^2 + 1 - 1)"), "x^{2}");
}

#[test]
fn array_binary_broadcast() {
    // `Array + scalar` is elementwise; `Array + Array` concatenates (v2.1, spec §11.3).
    assert_eq!(
        eval("[1, 2, 3] + 10"),
        Value::Array(vec![
            Value::Number(Number::from(11)),
            Value::Number(Number::from(12)),
            Value::Number(Number::from(13))
        ])
    );
    assert_eq!(
        eval("[1, 2, 3] + [10, 20, 30]"),
        Value::Array(vec![
            Value::Number(Number::from(1)),
            Value::Number(Number::from(2)),
            Value::Number(Number::from(3)),
            Value::Number(Number::from(10)),
            Value::Number(Number::from(20)),
            Value::Number(Number::from(30))
        ])
    );
    assert_eq!(
        eval("[1, 2, 3]^2"),
        Value::Array(vec![
            Value::Number(Number::from(1)),
            Value::Number(Number::from(4)),
            Value::Number(Number::from(9))
        ])
    );
}

#[test]
fn nested_array_allowed_as_data() {
    // v2.1: nested arrays are legal as data (broadcast still rejects them, spec §11.3/§11.4).
    assert_eq!(
        eval("[[1, 2], [3, 4]]"),
        Value::Array(vec![
            Value::Array(vec![
                Value::Number(Number::from(1)),
                Value::Number(Number::from(2))
            ]),
            Value::Array(vec![
                Value::Number(Number::from(3)),
                Value::Number(Number::from(4))
            ]),
        ])
    );
}

#[test]
fn sqrt_exact_perfect_squares() {
    assert_eq!(eval("sqrt(4)"), Value::Number(Number::from(2)));
    assert_eq!(
        eval("sqrt(9/4)"),
        Value::Number(Number::from(3) / Number::from(2))
    );
    assert_eq!(eval_fmt("sqrt(2)"), "\\sqrt{2}");
}

#[test]
fn trig_and_log_constants() {
    assert_eq!(eval("sin(0)"), Value::Number(Number::from(0)));
    assert_eq!(eval("cos(0)"), Value::Number(Number::from(1)));
    assert_eq!(eval("tan(0)"), Value::Number(Number::from(0)));
    assert_eq!(eval("ln(\\e)"), Value::Number(Number::from(1)));
    assert_eq!(eval("log(1)"), Value::Number(Number::from(0)));
    assert_eq!(eval("abs(-3)"), Value::Number(Number::from(3)));
    assert_eq!(eval("sin(\\pi)"), Value::Number(Number::from(0)));
}

#[test]
fn empty_array_broadcast_rejected() {
    assert!(
        Evaluator::new()
            .eval_value("let f(x) = x^2;\nf([])")
            .is_err()
    );
}

#[test]
fn division_by_zero_rejected() {
    assert!(Evaluator::new().eval_value("1/0").is_err());
}

#[test]
fn unbound_variable_is_symbolic() {
    assert_eq!(eval_fmt("x + 1"), "x + 1");
}

#[test]
fn closure_over_let() {
    assert_eq!(
        eval("let a = 5;\nlet f(x) = x + a;\nf(1)"),
        Value::Number(Number::from(6))
    );
}

#[test]
fn comparisons() {
    assert_eq!(eval("1 < 2"), Value::Bool(true));
    assert_eq!(eval("3 >= 3"), Value::Bool(true));
    assert_eq!(eval("1 == 1.0"), Value::Bool(true));
    assert_eq!(eval("1 != 2"), Value::Bool(true));
}

#[test]
fn index_read() {
    assert_eq!(
        eval("let v = [10, 20, 30];\nv[1]"),
        Value::Number(Number::from(20))
    );
    assert!(
        Evaluator::new()
            .eval_value("let v = [1, 2];\nv[5]")
            .is_err()
    );
}

#[test]
fn float_contagion_in_function() {
    let v = eval("let f(x) = x^2;\nf(3.0)");
    match v {
        Value::Number(Number::Real(prima_core::Real::F64(f))) => assert_eq!(f, 9.0),
        other => panic!("expected F64, got {other:?}"),
    }
}
