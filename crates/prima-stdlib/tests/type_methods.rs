//! Golden behavior table for the builtin-class method sets beyond `String` (spec §11.3/§11.6/
//! §16.3/§18.1, Phase 10): `Array`/`Dict`/`Set` follow Python `list`/`dict`/`set`, `Number` follows
//! the collapse family plus numeric predicates/accessors, and `Char`/`Tuple`/`Option`/`Result` have
//! their own small method sets.

use prima_core::{Number, Value};
use prima_runtime::Evaluator;

fn eval(src: &str) -> Value {
    prima_stdlib::init();
    Evaluator::new()
        .eval_value(src)
        .unwrap_or_else(|e| panic!("eval failed for {src:?}: {e}"))
}

fn eval_str(src: &str) -> String {
    match eval(src) {
        Value::String(s) => s,
        other => panic!("expected String for {src:?}, got {other:?}"),
    }
}

fn n(i: i64) -> Value {
    Value::Number(Number::from(i))
}

#[test]
fn array_methods() {
    assert_eq!(eval("let a = [1, 2, 3, 2];\na.len()"), n(4));
    assert!(matches!(
        eval("let a = [1, 2];\na.is_empty()"),
        Value::Bool(false)
    ));
    assert!(matches!(
        eval("let a = [];\na.is_empty()"),
        Value::Bool(true)
    ));
    assert_eq!(
        eval("let a = [1, 2, 3];\na.get(-1)"),
        Value::Option(Some(Box::new(n(3))))
    );
    assert_eq!(eval("let a = [1, 2];\na.get(9)"), Value::Option(None));
    assert!(matches!(
        eval("let a = [1, 2];\na.contains(2)"),
        Value::Bool(true)
    ));
    assert_eq!(eval("let a = [1, 2, 3];\na.index(3)"), n(2));
    assert_eq!(eval("let a = [1, 2, 1];\na.count(1)"), n(2));
    assert_eq!(
        eval("let a = [1, 2, 3];\na.first()"),
        Value::Option(Some(Box::new(n(1))))
    );
    assert_eq!(
        eval("let a = [1, 2, 3];\na.last()"),
        Value::Option(Some(Box::new(n(3))))
    );
    // `copy` is layered: native at O2, a comprehension at O0 (same semantics).
    assert_eq!(
        eval("let a = [1, 2, 3];\na.copy()"),
        Value::Array(vec![n(1), n(2), n(3)])
    );
    // Mutating methods write back through the binding.
    assert_eq!(
        eval("let a = [1];\na.push(2);\na"),
        Value::Array(vec![n(1), n(2)])
    );
    assert_eq!(
        eval("let a = [1, 2];\na.pop()"),
        Value::Option(Some(Box::new(n(2))))
    );
    assert_eq!(
        eval("let a = [3, 1, 2];\na.sort();\na"),
        Value::Array(vec![n(1), n(2), n(3)])
    );
    assert_eq!(
        eval("let a = [1, 2];\na.reverse();\na"),
        Value::Array(vec![n(2), n(1)])
    );
    assert_eq!(eval("let a = [1];\na.extend([2, 3]);\na.len()"), n(3));
    assert_eq!(
        eval("let a = [1, 2];\na.insert(0, 9);\na"),
        Value::Array(vec![n(9), n(1), n(2)])
    );
    // `remove` is index-based (negative counts from the end).
    assert_eq!(
        eval("let a = [7, 8, 9];\na.remove(1);\na"),
        Value::Array(vec![n(7), n(9)])
    );
    assert_eq!(eval("let a = [1, 2];\na.clear();\na.len()"), n(0));
}

#[test]
fn dict_methods() {
    assert_eq!(eval("let d = { \"a\": 1, \"b\": 2 };\nd.len()"), n(2));
    assert_eq!(
        eval("let d = { \"a\": 1 };\nd.get(\"a\")"),
        Value::Option(Some(Box::new(n(1))))
    );
    assert_eq!(
        eval("let d = { \"a\": 1 };\nd.get(\"x\")"),
        Value::Option(None)
    );
    assert!(matches!(
        eval("let d = { \"a\": 1 };\nd.contains(\"a\")"),
        Value::Bool(true)
    ));
    assert!(matches!(
        eval("let d = { \"a\": 1 };\nd.contains(\"x\")"),
        Value::Bool(false)
    ));
    assert_eq!(
        eval_str("let d = { \"b\": 2, \"a\": 1 };\njoin(d.keys(), \",\")"),
        "a,b"
    );
    // `copy` is layered (dict comprehension at O0).
    assert_eq!(eval("let d = { \"a\": 1 };\nd.copy().len()"), n(1));
    assert_eq!(eval("let d = { \"a\": 1 };\nd.setdefault(\"b\", 2)"), n(2));
    assert_eq!(eval("let d = { \"a\": 1 };\nd.setdefault(\"a\", 9)"), n(1));
    assert_eq!(
        eval("let d = { \"a\": 1 };\nd.setdefault(\"b\", 2);\nd.len()"),
        n(2)
    );
    assert_eq!(
        eval("let d = { \"a\": 1 };\nd.popitem()"),
        Value::Tuple(vec![Value::String("a".into()), n(1)])
    );
    assert_eq!(
        eval("let d = { \"a\": 1, \"b\": 2 };\nd.remove(\"a\")"),
        Value::Option(Some(Box::new(n(1))))
    );
    assert_eq!(
        eval("let d = { \"a\": 1 };\nd.update({ \"b\": 2 }).len()"),
        n(2)
    );
}

#[test]
fn set_methods() {
    assert_eq!(eval("let s = {1, 2, 3};\ns.len()"), n(3));
    assert!(matches!(
        eval("let s = {1, 2};\ns.contains(2)"),
        Value::Bool(true)
    ));
    assert_eq!(
        eval("let s = {1, 2};\ns.union({2, 3})"),
        eval("let s = {1, 2, 3};\ns.copy()")
    );
    // `symmetric_difference` is layered (set-algebra expression at O0).
    assert_eq!(
        eval("let s = {1, 2, 3};\ns.symmetric_difference({3, 4})"),
        eval("let s = {1, 2, 4};\ns.copy()")
    );
    assert!(matches!(
        eval("let s = {1, 2};\ns.issubset({1, 2, 3})"),
        Value::Bool(true)
    ));
    assert!(matches!(
        eval("let s = {1, 2, 3};\ns.issubset({1, 2})"),
        Value::Bool(false)
    ));
    assert!(matches!(
        eval("let s = {1, 2, 3};\ns.issuperset({1, 2})"),
        Value::Bool(true)
    ));
    assert!(matches!(
        eval("let s = {1, 2};\ns.isdisjoint({3})"),
        Value::Bool(true)
    ));
    assert!(matches!(
        eval("let s = {1, 2};\ns.isdisjoint({2})"),
        Value::Bool(false)
    ));
    assert_eq!(eval("let s = {1, 2};\ns.add(3);\ns.len()"), n(3));
    assert_eq!(eval("let s = {1, 2};\ns.discard(9);\ns.len()"), n(2));
    assert!(matches!(
        eval("let s = {1, 2};\ns.pop()"),
        Value::Option(Some(_))
    ));
    assert_eq!(eval("let s = {1, 2};\ns.pop();\ns.len()"), n(1));
    assert_eq!(eval("let s = {1, 2};\ns.update([3, 4]);\ns.len()"), n(4));
    assert_eq!(eval("let s = {1};\ns.clear();\ns.len()"), n(0));
}

#[test]
fn number_methods() {
    // collapse-family conversions
    assert_eq!(eval("to_f64(7).to_i64()"), Value::Number(Number::I64(7)));
    assert_eq!(
        eval("to_f64(7).rounded(1)"),
        Value::Number(Number::Real(prima_core::Real::F64(7.0)))
    );
    // predicates and accessors
    assert!(matches!(eval("3.7.is_integer()"), Value::Bool(false)));
    assert!(matches!(eval("4.is_integer()"), Value::Bool(true)));
    assert!(matches!(eval("(7/2).is_rational()"), Value::Bool(true)));
    assert!(matches!(eval("to_f64(2.0).is_real()"), Value::Bool(true)));
    assert!(matches!(
        eval("to_complex(1).is_complex()"),
        Value::Bool(true)
    ));
    assert!(matches!(eval("5.is_positive()"), Value::Bool(true)));
    assert!(matches!(eval("(-5).is_negative()"), Value::Bool(true)));
    assert!(matches!(eval("0.is_zero()"), Value::Bool(true)));
    assert!(matches!(eval("4.is_even()"), Value::Bool(true)));
    assert!(matches!(eval("3.is_odd()"), Value::Bool(true)));
    assert!(matches!(eval("to_f64(1.0).is_finite()"), Value::Bool(true)));
    assert!(matches!(eval("to_f64(0.0).is_nan()"), Value::Bool(false)));
    assert_eq!(eval("(-5).abs()"), n(5));
    assert_eq!(eval("(-5).sign()"), n(-1));
    assert_eq!(eval("(3).sign()"), n(1));
    assert_eq!(eval("3.7.floor()"), eval("to_f64(3.0)"));
    assert_eq!(eval("3.7.ceil()"), eval("to_f64(4.0)"));
    assert_eq!(eval("(-3.7).floor()"), eval("to_f64(-4.0)"));
    assert_eq!(eval("3.7.round()"), eval("to_f64(4.0)"));
    assert_eq!(eval("(7/2).floor()"), n(3));
    assert_eq!(eval("(7/2).ceil()"), n(4));
    assert_eq!(eval("(7/2).numerator()"), n(7));
    assert_eq!(eval("(7/2).denominator()"), n(2));
    assert_eq!(eval("to_f64(4).sqrt()"), eval("to_f64(2.0)"));
    assert_eq!(eval("10.bit_length()"), n(4));
    assert_eq!(eval("to_complex(3).real()"), n(3));
    assert_eq!(eval("to_complex(3).imag()"), n(0));
}

#[test]
fn char_tuple_option_result() {
    // Char
    assert!(matches!(eval("'5'.is_digit()"), Value::Bool(true)));
    assert!(matches!(eval("'a'.is_digit()"), Value::Bool(false)));
    assert!(matches!(eval("'a'.is_alpha()"), Value::Bool(true)));
    assert!(matches!(eval("'A'.is_upper()"), Value::Bool(true)));
    assert!(matches!(eval("'a'.is_lower()"), Value::Bool(true)));
    assert!(matches!(eval("' '.is_space()"), Value::Bool(true)));
    assert!(matches!(eval("'a'.is_ascii()"), Value::Bool(true)));
    assert_eq!(eval("'A'.to_lower()"), Value::Char('a'));
    assert_eq!(eval("'a'.to_upper()"), Value::Char('A'));
    assert_eq!(eval_str("'x'.to_string()"), "x");
    assert_eq!(eval("'\\u{3c0}'.code()"), n(960));
    // Tuple
    assert_eq!(eval("(1, \"a\", 1).len()"), n(3));
    assert_eq!(
        eval("(1, \"a\", 1).get(1)"),
        Value::Option(Some(Box::new(Value::String("a".into()))))
    );
    assert_eq!(eval("(1, \"a\", 1).count(1)"), n(2));
    assert_eq!(eval("(1, \"a\").index(\"a\")"), n(1));
    assert_eq!(
        eval("(1, \"a\").first()"),
        Value::Option(Some(Box::new(n(1))))
    );
    assert_eq!(
        eval("(1, \"a\").last()"),
        Value::Option(Some(Box::new(Value::String("a".into()))))
    );
    // Option
    assert!(matches!(eval("Some(5).is_some()"), Value::Bool(true)));
    assert!(matches!(eval("None().is_none()"), Value::Bool(true)));
    assert_eq!(eval("Some(5).unwrap()"), n(5));
    assert_eq!(eval("None().unwrap_or(0)"), n(0));
    // Result
    assert!(matches!(eval("Ok(1).is_ok()"), Value::Bool(true)));
    assert!(matches!(eval("Err(\"x\").is_err()"), Value::Bool(true)));
    assert_eq!(eval("Ok(9).unwrap()"), n(9));
    assert_eq!(eval("Ok(9).value_or(0)"), n(9));
    assert_eq!(eval("Err(\"x\").value_or(0)"), n(0));
}
