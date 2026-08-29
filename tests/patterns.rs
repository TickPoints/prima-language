// Pattern tests (spec §4.4): destructuring, `if let`/`while let`, full `match` patterns, and the `?` operator (spec §16.3).
use std::cell::RefCell;
use std::rc::Rc;

use prima_core::{Number, Value};
use prima_runtime::Evaluator;

fn eval(src: &str) -> Value {
    prima_stdlib::init();
    Evaluator::new().eval_value(src).expect("eval failed")
}

fn run_src(src: &str) -> String {
    prima_stdlib::init();
    let out = Rc::new(RefCell::new(String::new()));
    let out_c = Rc::clone(&out);
    let mut ev = Evaluator::with_sink(move |s| out_c.borrow_mut().push_str(&s));
    ev.eval_src(src).expect("eval failed");
    out.borrow().clone()
}

#[test]
fn tuple_destructuring() {
    assert_eq!(
        eval("let (a, b) = (1, 2);\na + b"),
        Value::Number(Number::from(3))
    );
}

#[test]
fn if_let_binds_some() {
    let out = run_src(
        "let v = [10];\nif let Some(x) = v.get(0) {\n    println(x);\n} else {\n    println(-1);\n}",
    );
    assert_eq!(out, "10\n");
}

#[test]
fn if_let_falls_to_else_on_none() {
    let out = run_src(
        "let v = [1];\nif let Some(x) = v.get(5) {\n    println(x);\n} else {\n    println(-1);\n}",
    );
    assert_eq!(out, "-1\n");
}

#[test]
fn while_let_consumes_iterator() {
    let out = run_src(
        "let v = [1, 2, 3];\nlet i = 0;\nwhile let Some(x) = v.get(i) {\n    println(x);\n    i = i + 1;\n}",
    );
    assert_eq!(out, "1\n2\n3\n");
}

#[test]
fn match_literal_or_range_guard_wildcard() {
    assert_eq!(
        eval("match 0 { 0 => \"zero\", _ => \"other\" }"),
        Value::String("zero".into())
    );
    assert_eq!(
        eval("match 2 { 1 | 2 => \"small\", _ => \"other\" }"),
        Value::String("small".into())
    );
    assert_eq!(
        eval("match 7 { 3..=9 => \"medium\", _ => \"other\" }"),
        Value::String("medium".into())
    );
    assert_eq!(
        eval("match 200 { n if n > 100 => \"large\", _ => \"other\" }"),
        Value::String("large".into())
    );
    assert_eq!(
        eval(
            "match -4 { 0 => \"zero\", 1 | 2 => \"small\", 3..=9 => \"medium\", n if n > 100 => \"large\", _ => \"other\" }"
        ),
        Value::String("other".into())
    );
}

#[test]
fn match_binding_and_expression_position() {
    let src = "let r = match 5 {\n    0 => \"zero\",\n    1 | 2 => \"small\",\n    3..=9 => \"medium\",\n    n if n > 100 => \"large\",\n    _ => \"other\"\n};\nr";
    assert_eq!(eval(src), Value::String("medium".into()));
}

#[test]
fn match_on_result() {
    let src = "match try_i32(7) {\n    Ok(n) => n,\n    Err(e) => -1\n}";
    assert_eq!(eval(src), Value::Number(Number::I32(7)));
    let src = "match try_i32(1e20) {\n    Ok(n) => n,\n    Err(e) => -1\n}";
    assert_eq!(eval(src), Value::Number(Number::from(-1)));
}

#[test]
fn match_on_option() {
    assert_eq!(
        eval("match Some(42) { Some(x) => x, None => 0 }"),
        Value::Number(Number::from(42))
    );
    assert_eq!(
        eval("match get([1], 5) { Some(x) => x, None => 0 }"),
        Value::Number(Number::from(0))
    );
}

#[test]
fn match_non_exhaustive_is_error() {
    assert!(
        Evaluator::new()
            .eval_value("match 1 {\n    2 => 0\n}")
            .is_err()
    );
}

#[test]
fn try_operator_unwraps_ok() {
    let v = eval(
        "fn f(x) -> Result<Integer, Error> {\n    let v = try_i32(x)?;\n    return Ok(v);\n}\nf(7)",
    );
    assert_eq!(
        v,
        Value::Result(Ok(Box::new(Value::Number(Number::I32(7)))))
    );
}

#[test]
fn try_operator_propagates_none() {
    let err = Evaluator::new()
        .eval_value(
            "fn g() -> Option<Integer> {\n    let x = get([1], 5)?;\n    return Some(x);\n}\ng()",
        )
        .unwrap_err();
    assert!(err.to_string().contains("None"), "unexpected error: {err}");
}

#[test]
fn struct_pattern_destructuring() {
    let src = "class P { pub x: Integer, pub y: Integer }\nlet p = P { x: 3, y: 4 };\nlet P { x, y } = p;\nx + y";
    assert_eq!(eval(src), Value::Number(Number::from(7)));
}

#[test]
fn array_destructuring() {
    assert_eq!(
        eval("let [a, b] = [1, 2];\na + b"),
        Value::Number(Number::from(3))
    );
}

#[test]
fn let_pattern_mismatch_errors() {
    assert!(
        Evaluator::new()
            .eval_value("let (a, b) = (1, 2, 3);\na")
            .is_err()
    );
}

#[test]
fn match_on_string() {
    assert_eq!(
        eval("match \"a\" { \"a\" => 1, \"b\" => 2, _ => 0 }"),
        Value::Number(Number::from(1))
    );
    assert_eq!(
        eval("match \"z\" { \"a\" => 1, \"b\" => 2, _ => 0 }"),
        Value::Number(Number::from(0))
    );
}

#[test]
fn variant_patterns_for_some_ok_err() {
    assert_eq!(
        eval("match Some(1) { Some(x) => x, None => -1 }"),
        Value::Number(Number::from(1))
    );
    let v = eval("match try_i32(5) { Ok(x) => x, Err(e) => -1 }");
    assert_eq!(v, Value::Number(Number::I32(5)));
}
