//! Semantic boundaries (spec §12.3/§16.3/§11.4): class-instance sharing, `?` propagation, and
//! broadcast policy edges. These are the cases where the evaluator's observable behavior is easy to
//! break while refactoring.

use prima_core::{Number, Value};
use prima_runtime::Evaluator;

fn eval(src: &str) -> Value {
    prima_stdlib::init();
    Evaluator::new().eval_value(src).expect("eval failed")
}

fn eval_err(src: &str) -> prima_runtime::RuntimeError {
    prima_stdlib::init();
    Evaluator::new()
        .eval_value(src)
        .expect_err("expected a runtime error")
}

fn int(v: &Value) -> i64 {
    match v {
        Value::Number(n) => n.as_i64().expect("integer"),
        other => panic!("expected Integer, got {other:?}"),
    }
}

/// `Counter` with a constructor, a getter, an identity method, and a method returning a new instance.
const COUNTER: &str = "\
class Counter {
    value: Integer,
    pub fn new(v: Integer) -> Self { Counter { value: v } },
    pub fn get(self) -> Integer { self.value },
    pub fn me(self) -> Counter { self },
    pub fn bump(self) -> Counter { Counter { value: self.value + 1 } }
}
";

// ————— class instance ownership (spec §12.3) —————

/// Instance handles of every element of a tuple result.
fn class_ids(v: &Value) -> Vec<u32> {
    let Value::Tuple(items) = v else {
        panic!("expected a tuple, got {v:?}");
    };
    items
        .iter()
        .map(|item| match item {
            Value::Class(id) => *id,
            other => panic!("expected a class instance, got {other:?}"),
        })
        .collect()
}

#[test]
fn assignment_shares_the_instance_handle() {
    let src = format!("{COUNTER}\nlet a = Counter::new(1);\nlet b = a;\n(a, b)");
    let ids = class_ids(&eval(&src));
    assert_eq!(ids[0], ids[1], "`let b = a` must share the handle");
}

#[test]
fn returning_a_class_instance_keeps_sharing() {
    let src = format!("{COUNTER}\nlet a = Counter::new(1);\n(a, a.me())");
    let ids = class_ids(&eval(&src));
    assert_eq!(ids[0], ids[1], "returning `self` must share the handle");
}

#[test]
fn struct_literal_creates_a_distinct_instance() {
    let src = format!("{COUNTER}\n(Counter::new(1), Counter::new(1))");
    let ids = class_ids(&eval(&src));
    assert_ne!(ids[0], ids[1], "constructors must create new instances");

    let src = format!("{COUNTER}\nlet a = Counter::new(1);\n(a, a.bump())");
    let ids = class_ids(&eval(&src));
    assert_ne!(ids[0], ids[1], "`bump` returns a new instance");
}

#[test]
fn returning_a_basic_field_yields_an_independent_value() {
    let src = format!("{COUNTER}\nlet a = Counter::new(4);\nlet v = a.get();\nv + 1");
    assert_eq!(int(&eval(&src)), 5);
}

// ————— `?` propagation (spec §16.3) —————

#[test]
fn try_operator_propagates_err_from_result_function() {
    let src = "\
fn parse(x: String) -> Result<Integer, Error> {
    let v = try_i32(x)?;
    return Ok(v);
}
match parse(\"nope\") {
    Ok(v) => -1,
    Err(e) => -2
}";
    assert_eq!(int(&eval(src)), -2);
}

#[test]
fn try_operator_propagates_ok_value() {
    let src = "\
fn parse(x: String) -> Result<Integer, Error> {
    let v = try_i32(x)?;
    return Ok(v);
}
match parse(\"7\") {
    Ok(v) => v,
    Err(e) => -1
}";
    assert_eq!(int(&eval(src)), 7);
}

#[test]
fn try_operator_propagates_none_from_option_function() {
    let src = "\
fn first(xs: Array<Value>) -> Option<Integer> {
    let x = xs.get(0)?;
    return Some(x);
}
fn missing(xs: Array<Value>) -> Option<Integer> {
    let x = xs.get(5)?;
    return Some(x);
}
(first([7]), missing([7]))";
    let Value::Tuple(items) = eval(src) else {
        panic!("expected a tuple");
    };
    assert_eq!(
        items.as_ref(),
        [
            Value::Option(Some(Box::new(Value::Number(Number::from(7))))),
            Value::Option(None)
        ]
    );
}

#[test]
fn try_operator_propagates_from_method_body() {
    let src = "\
class Probe {
    pub fn at(self, xs: Array<Value>) -> Option<Integer> {
        let x = xs.get(5)?;
        Some(x)
    }
}
let p = Probe { };
p.at([1, 2])";
    assert_eq!(eval(src), Value::Option(None));
}

// ————— broadcast policy edges (spec §11.4) —————

#[test]
fn disabled_broadcast_rejects_nested_array_argument() {
    assert!(eval_err(
        "config { broadcast := false }\nlet f(x) = x^2;\nf([[1, 2], [3, 4]])"
    )
    .to_string()
    .contains("broadcast"));
}

#[test]
fn explicit_broadcast_still_works_when_implicit_is_disabled() {
    let v = eval("config { broadcast := false }\nlet f(x) = x^2;\n[1, 2, 3] @. f");
    let Value::Array(items) = v else {
        panic!("expected an array");
    };
    items.with(|values| {
        assert_eq!(
            values,
            [
                Value::Number(Number::from(1)),
                Value::Number(Number::from(4)),
                Value::Number(Number::from(9))
            ]
        );
    });
}
