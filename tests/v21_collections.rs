//! v2.1 collection semantics self-check (spec §11.3/§11.6/§11.7, appendix B.1):
//! heterogeneous arrays, dicts/sets, comprehensions, and the collection convenience functions.
//!
//! These integration tests drive the public `Evaluator` + `eval_value` entry points only.

use prima_core::{Number, Value};
use prima_runtime::Evaluator;

fn eval(src: &str) -> Value {
    prima_stdlib::init();
    Evaluator::new()
        .eval_value(src)
        .unwrap_or_else(|e| panic!("eval failed for {src:?}: {e}"))
}

fn eval_ok(src: &str) {
    prima_stdlib::init();
    Evaluator::new()
        .eval_value(src)
        .unwrap_or_else(|e| panic!("eval failed for {src:?}: {e}"));
}

fn n(i: i64) -> Value {
    Value::Number(Number::from(i))
}

#[test]
fn array_push_and_concat() {
    // `Array + Array` concatenates (spec §11.3); `v.push(4)` mutates through the binding.
    assert_eq!(
        eval("let v = [1, 2, 3];\nv.push(4);\nv + [5]"),
        Value::Array(vec![n(1), n(2), n(3), n(4), n(5),])
    );
}

#[test]
fn dict_index_membership_and_keys() {
    assert_eq!(eval("let d = { \"a\": 1, \"b\": 2 };\nd[\"b\"]"), n(2));
    assert_eq!(
        eval("let d = { \"a\": 1, \"b\": 2 };\n\"x\" in d"),
        Value::Bool(false)
    );
    assert_eq!(
        eval("let d = { \"a\": 1, \"b\": 2 };\n\"a\" in d"),
        Value::Bool(true)
    );
    assert_eq!(
        eval("let d = { \"a\": 1, \"b\": 2 };\nd.keys()"),
        Value::Array(vec![Value::String("a".into()), Value::String("b".into()),])
    );
    assert_eq!(
        eval("let d = { \"a\": 1, \"b\": 2 };\nd.keys().len()"),
        n(2)
    );
    assert!(
        Evaluator::new()
            .eval_value("let d = { \"a\": 1 };\nd[\"x\"]")
            .is_err()
    );
}

#[test]
fn set_dedup_algebra_and_length() {
    assert_eq!(eval("let s = {1, 2, 3, 2};\ns.len()"), n(3));
    assert_eq!(
        eval("let s = {1, 2, 3, 2};\nlet u = s ∪ {5, 6};\nu.len()"),
        n(5)
    );
    assert_eq!(
        eval("let s = {1, 2, 3};\nlet i = s ∩ {2, 3, 9};\ni.len()"),
        n(2)
    );
    assert_eq!(eval("let s = {1, 2, 3};\nlet d = s \\ {3};\nd.len()"), n(2));
    assert_eq!(eval("let s = {1, 2, 3};\n2 in s"), Value::Bool(true));
}

#[test]
fn array_comprehension() {
    assert_eq!(
        eval("[x^2 for x in range(0, 5)]"),
        Value::Array(vec![n(0), n(1), n(4), n(9), n(16),])
    );
    assert_eq!(
        eval("[x for x in range(0, 6) if x % 2 == 0]"),
        Value::Array(vec![n(0), n(2), n(4),])
    );
}

#[test]
fn dict_and_set_comprehensions() {
    assert_eq!(
        eval("let t = {x: x^2 for x in range(0, 3)};\nt.len()"),
        n(3)
    );
    assert_eq!(eval("let t = {x: x^2 for x in range(0, 3)};\nt[2]"), n(4));
    assert_eq!(eval("let o = {x for x in range(0, 5)};\no.len()"), n(5));
}

#[test]
fn convenience_functions() {
    assert_eq!(eval("len(\"hello\")"), n(5));
    assert_eq!(eval("2 in [1, 2, 3]"), Value::Bool(true));
    assert_eq!(eval("sum([1, 2, 3])"), n(6));
    assert_eq!(eval("prod([1, 2, 3])"), n(6));
    assert_eq!(eval("min([3, 1, 2])"), n(1));
    assert_eq!(eval("max([3, 1, 2])"), n(3));
    assert_eq!(
        eval("sorted([3, 1, 2])"),
        Value::Array(vec![n(1), n(2), n(3)])
    );
    assert_eq!(eval("reversed([1, 2])"), Value::Array(vec![n(2), n(1)]));
    assert_eq!(eval("count([1, 2, 2], 2)"), n(2));
    assert_eq!(eval("index([3, 1, 2], 2)"), n(2));
    assert_eq!(eval("first([1, 2])"), Value::Option(Some(Box::new(n(1)))));
    assert_eq!(eval("last([1, 2])"), Value::Option(Some(Box::new(n(2)))));
    assert_eq!(
        eval("enumerate([\"a\", \"b\"])"),
        Value::Array(vec![
            Value::Tuple(vec![n(0), Value::String("a".into())]),
            Value::Tuple(vec![n(1), Value::String("b".into())]),
        ])
    );
    assert_eq!(
        eval("zip([1, 2], [\"a\", \"b\"])"),
        Value::Array(vec![
            Value::Tuple(vec![n(1), Value::String("a".into())]),
            Value::Tuple(vec![n(2), Value::String("b".into())]),
        ])
    );
    assert_eq!(
        eval("linspace(0, 10, 5)"),
        Value::Array(vec![
            Value::Number(Number::Real(prima_core::Real::F64(0.0))),
            Value::Number(Number::Real(prima_core::Real::F64(2.5))),
            Value::Number(Number::Real(prima_core::Real::F64(5.0))),
            Value::Number(Number::Real(prima_core::Real::F64(7.5))),
            Value::Number(Number::Real(prima_core::Real::F64(10.0))),
        ])
    );
    assert_eq!(eval("all([true, true])"), Value::Bool(true));
    assert_eq!(eval("any([false, true])"), Value::Bool(true));
}

#[test]
fn negative_index_and_slices() {
    assert_eq!(eval("[10, 20, 30, 40][-1]"), n(40));
    assert_eq!(eval("[10, 20, 30, 40][-3]"), n(20));
    assert_eq!(
        eval("[10, 20, 30, 40][1..3]"),
        Value::Array(vec![n(20), n(30)])
    );
    assert_eq!(
        eval("[10, 20, 30, 40][..2]"),
        Value::Array(vec![n(10), n(20)])
    );
    assert_eq!(
        eval("[10, 20, 30, 40][-2..]"),
        Value::Array(vec![n(30), n(40)])
    );
    assert!(Evaluator::new().eval_value("[10, 20][5]").is_err());
    assert!(Evaluator::new().eval_value("[10, 20][-7]").is_err());
}

#[test]
fn slice_assignment() {
    assert_eq!(
        eval("let a = [1, 2, 3, 4];\na[1..3] = [20, 30];\na"),
        Value::Array(vec![n(1), n(20), n(30), n(4),])
    );
    assert_eq!(
        eval("let a = [1, 2, 3, 4];\na[0..1] = [];\na"),
        Value::Array(vec![n(2), n(3), n(4),])
    );
}

#[test]
fn array_methods() {
    assert_eq!(eval("let v = [1, 2, 3];\nv.len()"), n(3));
    assert_eq!(eval("let v = [1, 2, 3];\nv.contains(2)"), Value::Bool(true));
    assert_eq!(eval("let v = [3, 1, 2];\nv.index(2)"), n(2));
    assert_eq!(eval("let v = [1, 2, 2];\nv.count(2)"), n(2));
    assert_eq!(
        eval("let v = [1, 2, 3];\nv.first()"),
        Value::Option(Some(Box::new(n(1))))
    );
    assert_eq!(
        eval("let v = [1, 2, 3];\nv.last()"),
        Value::Option(Some(Box::new(n(3))))
    );
    assert_eq!(
        eval("let v = [1, 2, 3];\nv.get(-1)"),
        Value::Option(Some(Box::new(n(3))))
    );
    assert_eq!(
        eval("let v = [1, 2, 3];\nv.pop()"),
        Value::Option(Some(Box::new(n(3))))
    );
    assert_eq!(
        eval("let v = [3, 1, 2];\nv.sort();\nv"),
        Value::Array(vec![n(1), n(2), n(3)])
    );
    assert_eq!(
        eval("let v = [1, 2, 3];\nv.reverse();\nv"),
        Value::Array(vec![n(3), n(2), n(1)])
    );
}

#[test]
fn map_filter_reduce() {
    eval_ok("let f(x) = x^2;");
    assert_eq!(
        eval("let f(x) = x^2;\nmap(f, [1, 2, 3])"),
        Value::Array(vec![n(1), n(4), n(9)])
    );
    assert_eq!(
        eval("let p(x) = x % 2 == 0;\nfilter(p, [1, 2, 3, 4])"),
        Value::Array(vec![n(2), n(4)])
    );
    assert_eq!(eval("let g(a, b) = a + b;\nreduce(g, [1, 2, 3], 0)"), n(6));
}

#[test]
fn string_join_strip_find() {
    assert_eq!(
        eval("\"-\".join([\"a\", \"b\", \"c\"])"),
        Value::String("a-b-c".into())
    );
    assert_eq!(
        eval("join([\"a\", \"b\", \"c\"], \"-\")"),
        Value::String("a-b-c".into())
    );
    assert_eq!(
        eval("let s = \"  hi  \";\ns.strip(\" \")"),
        Value::String("hi".into())
    );
    assert_eq!(
        eval("let s = \"hello\";\ns.find(\"ll\")"),
        Value::Option(Some(Box::new(n(2))))
    );
    assert_eq!(
        eval("let s = \"hello\";\ns.find(\"x\")"),
        Value::Option(None)
    );
}
