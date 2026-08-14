// Symbolic differentiation (spec §19.4, v2.1): `derivative`/`partial`/`grad`/`limit`.
use prima_core::Value;
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
fn derivative_of_mfn() {
    // `derivative(f, x)` lowers the MFn body with `x` as a symbol (spec §19.4).
    assert_eq!(eval_fmt("let f(x) = x^2 + sin(x);\nderivative(f, x)"), "2 x + \\cos\\left(x\\right)");
}

#[test]
fn derivative_of_expression() {
    // Expression form works too.
    assert_eq!(eval_fmt("derivative(x^3, x)"), "3 \\left(x^{2}\\right)");
    assert_eq!(eval_fmt("derivative(5, x)"), "0");
}

#[test]
fn second_derivative() {
    assert_eq!(eval_fmt("derivative(derivative(x^3, x), x)"), "6 x");
}

#[test]
fn partial_derivatives() {
    assert_eq!(eval_fmt("let g(x, y) = x^2*y + y^3;\npartial(g, x)"), "2 x y");
    assert_eq!(eval_fmt("let g(x, y) = x^2*y + y^3;\npartial(g, y)"), "x^{2} + 3 \\left(y^{2}\\right)");
}

#[test]
fn grad_returns_tuple_of_partials() {
    // Free-variable gradient; represented as a `Tuple` of `Expr` values until `Array` is generalized.
    let v = eval("grad(x^2 + y^2)");
    let Value::Tuple(items) = v else {
        panic!("expected a tuple, got {v:?}");
    };
    assert_eq!(items.len(), 2);
}

#[test]
fn derivative_with_string_variable() {
    // The variable may also be given by name (spec §19.4).
    assert_eq!(eval_fmt("derivative(x^2, \"x\")"), "2 x");
}

#[test]
fn limit_via_lhopital() {
    assert_eq!(eval("limit(sin(x)/x, x, 0)"), Value::Number(1.into()));
}

#[test]
fn limit_direct_substitution() {
    assert_eq!(eval("limit(x^2, x, 3)"), Value::Number(9.into()));
}

#[test]
fn limit_returns_symbolic_when_indeterminate() {
    // A limit that cannot be resolved numerically stays a symbolic expression.
    let v = eval("limit(x^2, x, x)"); // substituting x→x is a no-op; not a number
    assert!(matches!(v, Value::Expr(_)), "got {v:?}");
}
