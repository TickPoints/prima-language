// v2.1 collection semantics (spec §11.3/§11.6/§11.7, §18.1): mutable Arrays, Dict/Set, comprehensions,
// convenience functions, and the completed String methods.
use prima_core::{Number, Value};
use prima_runtime::Evaluator;

fn eval(src: &str) -> Value {
    Evaluator::new().eval_value(src).expect("eval failed")
}

fn eval_err(src: &str) -> bool {
    Evaluator::new().eval_value(src).is_err()
}

fn eval_fmt(src: &str) -> String {
    let mut ev = Evaluator::new();
    let v = ev.eval_value(src).expect("eval failed");
    ev.format_value(&v)
}

fn arr(vals: Vec<Value>) -> Value {
    Value::Array(vals)
}

#[test]
fn array_heterogeneous_literal_and_concat() {
    assert_eq!(
        eval("[1, \"a\", true]"),
        arr(vec![Value::Number(Number::from(1)), Value::String("a".into()), Value::Bool(true)])
    );
    // `+` concatenates arrays (v2.1 §11.3); `+=` extends in place.
    assert_eq!(
        eval("let a = [1, 2];\nlet b = [3, 4];\nlet c = a + b;\nc"),
        arr(vec![
            Value::Number(Number::from(1)),
            Value::Number(Number::from(2)),
            Value::Number(Number::from(3)),
            Value::Number(Number::from(4))
        ])
    );
    assert_eq!(eval("let a = [1, 2];\na += [3];\na"), arr(vec![Value::Number(Number::from(1)), Value::Number(Number::from(2)), Value::Number(Number::from(3))]));
}

#[test]
fn array_mutation_methods() {
    let src = "let mut v = [1, 2, 3];\nv.push(4);\nv";
    assert_eq!(
        eval(src),
        arr(vec![Value::Number(Number::from(1)), Value::Number(Number::from(2)), Value::Number(Number::from(3)), Value::Number(Number::from(4))])
    );
    assert_eq!(eval("let v = [1, 2, 3];\nv.pop()"), Value::Option(Some(Box::new(Value::Number(Number::from(3))))));
    assert_eq!(
        eval("let v = [0];\nv.insert(0, 9);\nv"),
        arr(vec![Value::Number(Number::from(9)), Value::Number(Number::from(0))])
    );
    assert_eq!(eval("let v = [7, 8];\nv.remove(0)"), Value::Number(Number::from(7)));
    assert_eq!(eval("let v = [3, 1, 2];\nv.sort();\nv"), arr(vec![Value::Number(Number::from(1)), Value::Number(Number::from(2)), Value::Number(Number::from(3))]));
    assert_eq!(eval("let v = [1, 2];\nv.reverse();\nv"), arr(vec![Value::Number(Number::from(2)), Value::Number(Number::from(1))]));
    assert_eq!(eval("let v = [1, 2];\nv.clear();\nv"), arr(vec![]));
}

#[test]
fn array_negative_index_and_slices() {
    assert_eq!(eval("[10, 20, 30, 40][-1]"), Value::Number(Number::from(40)));
    assert_eq!(eval("[10, 20, 30, 40][-2]"), Value::Number(Number::from(30)));
    assert_eq!(
        eval("[10, 20, 30, 40][1..3]"),
        arr(vec![Value::Number(Number::from(20)), Value::Number(Number::from(30))])
    );
    assert_eq!(
        eval("[10, 20, 30, 40][..2]"),
        arr(vec![Value::Number(Number::from(10)), Value::Number(Number::from(20))])
    );
    assert_eq!(
        eval("[10, 20, 30, 40][-2..]"),
        arr(vec![Value::Number(Number::from(30)), Value::Number(Number::from(40))])
    );
    // Out of range (including negative beyond bounds) → R0003.
    assert!(eval_err("[1, 2, 3][10]"));
    assert!(eval_err("[1, 2, 3][-4]"));
}

#[test]
fn array_slice_assignment() {
    assert_eq!(
        eval("let mut v = [1, 2, 3, 4];\nv[1..3] = [20, 30];\nv"),
        arr(vec![Value::Number(Number::from(1)), Value::Number(Number::from(20)), Value::Number(Number::from(30)), Value::Number(Number::from(4))])
    );
    assert_eq!(
        eval("let mut v = [1, 2, 3, 4];\nv[0..1] = [];\nv"),
        arr(vec![Value::Number(Number::from(2)), Value::Number(Number::from(3)), Value::Number(Number::from(4))])
    );
}

#[test]
fn array_readonly_helpers_and_membership() {
    assert_eq!(eval("let v = [3, 1, 2, 1];\nv.len()"), Value::Number(Number::from(4)));
    assert_eq!(eval("let v = [1, 2];\nv.get(1)"), Value::Option(Some(Box::new(Value::Number(Number::from(2))))));
    assert_eq!(eval("let v = [1, 2];\nv.get(9)"), Value::Option(None));
    assert_eq!(eval("let v = [3, 1, 2];\nv.contains(2)"), Value::Bool(true));
    assert_eq!(eval("let v = [3, 1, 2];\nv.index(2)"), Value::Number(Number::from(2)));
    assert!(eval_err("let v = [3, 1, 2];\nv.index(9)"));
    assert_eq!(eval("let v = [1, 2, 1];\nv.count(1)"), Value::Number(Number::from(2)));
    assert_eq!(eval("let v = [3, 1, 2];\nv.first()"), Value::Option(Some(Box::new(Value::Number(Number::from(3))))));
    assert_eq!(eval("let v = [3, 1, 2];\nv.last()"), Value::Option(Some(Box::new(Value::Number(Number::from(2))))));
    assert_eq!(eval("let v = [];\nv.first()"), Value::Option(None));
    assert_eq!(eval("2 in [1, 2, 3]"), Value::Bool(true));
    assert_eq!(eval("5 in [1, 2, 3]"), Value::Bool(false));
}

#[test]
fn dict_literal_index_and_membership() {
    let v = eval("let d = { \"a\": 1, \"b\": 2 };\nd");
    let Value::Dict(d) = v else { panic!("expected dict") };
    assert_eq!(d.len(), 2);
    assert_eq!(eval("let d = { \"a\": 1 };\nd[\"a\"]"), Value::Number(Number::from(1)));
    assert!(eval_err("let d = { \"a\": 1 };\nd[\"x\"]"));
    assert_eq!(eval("let d = { \"a\": 1 };\nd.get(\"a\")"), Value::Option(Some(Box::new(Value::Number(Number::from(1))))));
    assert_eq!(eval("let d = { \"a\": 1 };\nd.get(\"x\")"), Value::Option(None));
    assert_eq!(eval("let d = { \"a\": 1 };\n\"a\" in d"), Value::Bool(true));
    assert_eq!(eval("let d = { \"a\": 1 };\n\"z\" in d"), Value::Bool(false));
    assert_eq!(eval("let d = { \"a\": 1 };\nd.len()"), Value::Number(Number::from(1)));
}

#[test]
fn dict_mutation_and_views() {
    assert_eq!(eval("let d = { \"a\": 1 };\nd[\"b\"] = 2;\nd[\"b\"]"), Value::Number(Number::from(2)));
    assert_eq!(eval("let d = { \"a\": 1 };\nd.insert(\"b\", 2);\nd[\"b\"]"), Value::Number(Number::from(2)));
    assert_eq!(eval("let d = { \"a\": 1 };\nd.remove(\"a\")"), Value::Option(Some(Box::new(Value::Number(Number::from(1))))));
    assert_eq!(eval("let d = { \"a\": 1 };\nd.remove(\"x\")"), Value::Option(None));
    assert_eq!(eval("let d = { \"a\": 1, \"b\": 2 };\nd.keys()"), arr(vec![Value::String("a".into()), Value::String("b".into())]));
    assert_eq!(
        eval("let d = { \"a\": 1, \"b\": 2 };\nd.values()"),
        arr(vec![Value::Number(Number::from(1)), Value::Number(Number::from(2))])
    );
    let items = eval("let d = { \"a\": 1 };\nd.items()");
    assert_eq!(
        items,
        arr(vec![Value::Tuple(vec![Value::String("a".into()), Value::Number(Number::from(1))])])
    );
    // `update` returns the merged dict (spec §11.6).
    assert_eq!(
        eval("let d = { \"a\": 1 };\nlet dd = d.update({ \"x\": 9 });\ndd.len()"),
        Value::Number(Number::from(2))
    );
    assert_eq!(eval("let d = { \"a\": 1 };\nd.clear();\nd.len()"), Value::Number(Number::from(0)));
}

#[test]
fn set_literal_dedups_and_methods() {
    assert_eq!(eval("let s = {1, 2, 3, 2};\ns.len()"), Value::Number(Number::from(3)));
    assert_eq!(eval("let s = {1, 2};\ns.contains(1)"), Value::Bool(true));
    assert_eq!(eval("let s = {1, 2};\n1 in s"), Value::Bool(true));
    assert_eq!(eval("let s = {1, 2};\n3 in s"), Value::Bool(false));
    assert_eq!(eval("let s = {1, 2};\ns.add(3);\ns.len()"), Value::Number(Number::from(3)));
    assert_eq!(eval("let s = {1, 2};\ns.remove(2);\ns.len()"), Value::Number(Number::from(1)));
    assert!(eval_err("let s = {1, 2};\ns.remove(9)"));
    assert_eq!(eval("let s = {1, 2};\ns.discard(9);\ns.len()"), Value::Number(Number::from(2)));
}

#[test]
fn set_algebra_operators() {
    assert_eq!(eval("let s = {1, 2, 3};\n(s ∪ {5, 6}).len()"), Value::Number(Number::from(5)));
    assert_eq!(eval("let s = {1, 2, 3};\n(s ∩ {2, 3}).len()"), Value::Number(Number::from(2)));
    assert_eq!(eval("let s = {1, 2, 3};\n(s \\ {3}).len()"), Value::Number(Number::from(2)));
    assert_eq!(eval("let s = {1, 2};\ns.union({3}).len()"), Value::Number(Number::from(3)));
    assert_eq!(eval("let s = {1, 2, 3};\ns.intersection({2}).len()"), Value::Number(Number::from(1)));
    assert_eq!(eval("let s = {1, 2, 3};\ns.difference({3}).len()"), Value::Number(Number::from(2)));
}

#[test]
fn array_comprehension() {
    assert_eq!(
        eval("[x^2 for x in range(0, 5)]"),
        arr(vec![
            Value::Number(Number::from(0)),
            Value::Number(Number::from(1)),
            Value::Number(Number::from(4)),
            Value::Number(Number::from(9)),
            Value::Number(Number::from(16))
        ])
    );
    assert_eq!(
        eval("[x for x in range(0, 10) if x % 2 == 0]"),
        arr(vec![
            Value::Number(Number::from(0)),
            Value::Number(Number::from(2)),
            Value::Number(Number::from(4)),
            Value::Number(Number::from(6)),
            Value::Number(Number::from(8))
        ])
    );
    assert_eq!(eval("[(x, y) for x in range(0, 2) for y in range(0, 2)]"),
        arr(vec![
            Value::Tuple(vec![Value::Number(Number::from(0)), Value::Number(Number::from(0))]),
            Value::Tuple(vec![Value::Number(Number::from(0)), Value::Number(Number::from(1))]),
            Value::Tuple(vec![Value::Number(Number::from(1)), Value::Number(Number::from(0))]),
            Value::Tuple(vec![Value::Number(Number::from(1)), Value::Number(Number::from(1))]),
        ]));
}

#[test]
fn dict_and_set_comprehension() {
    let v = eval("{x: x^2 for x in range(0, 5)}");
    let Value::Dict(d) = v else { panic!("expected dict") };
    assert_eq!(d.len(), 5);
    assert_eq!(d[&prima_core::ValueKey::Int(3)], Value::Number(Number::from(9)));
    assert_eq!(eval("{x for x in range(0, 10) if x % 2 == 1}.len()"), Value::Number(Number::from(5)));
    assert_eq!(eval("((x, x+1) for x in range(0, 3))"),
        Value::Tuple(vec![
            Value::Tuple(vec![Value::Number(Number::from(0)), Value::Number(Number::from(1))]),
            Value::Tuple(vec![Value::Number(Number::from(1)), Value::Number(Number::from(2))]),
            Value::Tuple(vec![Value::Number(Number::from(2)), Value::Number(Number::from(3))]),
        ]));
}

#[test]
fn comprehension_iterates_string_and_dict() {
    assert_eq!(
        eval("[c for c in \"ab\"]"),
        arr(vec![Value::Char('a'), Value::Char('b')])
    );
    assert_eq!(eval("[k for k in { \"x\": 1, \"y\": 2 }]"),
        arr(vec![Value::String("x".into()), Value::String("y".into())]));
}

#[test]
fn convenience_functions() {
    assert_eq!(eval("len([1, 2, 3])"), Value::Number(Number::from(3)));
    assert_eq!(eval("len(\"hello\")"), Value::Number(Number::from(5)));
    assert_eq!(eval("len({ \"a\": 1 })"), Value::Number(Number::from(1)));
    assert_eq!(eval("len({1, 2})"), Value::Number(Number::from(2)));
    assert_eq!(eval("len((1, 2, 3))"), Value::Number(Number::from(3)));
    assert_eq!(eval("enumerate([\"a\", \"b\"])"),
        arr(vec![
            Value::Tuple(vec![Value::Number(Number::from(0)), Value::String("a".into())]),
            Value::Tuple(vec![Value::Number(Number::from(1)), Value::String("b".into())]),
        ]));
    assert_eq!(eval("zip([1, 2], [\"a\", \"b\"])"),
        arr(vec![
            Value::Tuple(vec![Value::Number(Number::from(1)), Value::String("a".into())]),
            Value::Tuple(vec![Value::Number(Number::from(2)), Value::String("b".into())]),
        ]));
    assert_eq!(eval("sorted([3, 1, 2])"), arr(vec![Value::Number(Number::from(1)), Value::Number(Number::from(2)), Value::Number(Number::from(3))]));
    assert_eq!(eval("reversed([1, 2])"), arr(vec![Value::Number(Number::from(2)), Value::Number(Number::from(1))]));
    assert_eq!(eval("sum([1, 2, 3])"), Value::Number(Number::from(6)));
    assert_eq!(eval("prod([1, 2, 3])"), Value::Number(Number::from(6)));
    assert_eq!(eval("min([3, 1, 2])"), Value::Number(Number::from(1)));
    assert_eq!(eval("max([3, 1, 2])"), Value::Number(Number::from(3)));
    assert!(eval_err("sum([])"));
    assert!(eval_err("min([])"));
    assert_eq!(eval("all([true, true])"), Value::Bool(true));
    assert_eq!(eval("all([true, false])"), Value::Bool(false));
    assert_eq!(eval("any([false, true])"), Value::Bool(true));
    assert_eq!(eval("join([\"a\", \"b\"], \"-\")"), Value::String("a-b".into()));
    assert_eq!(eval("count([1, 2, 1], 1)"), Value::Number(Number::from(2)));
    assert_eq!(eval("index([1, 2, 3], 2)"), Value::Number(Number::from(1)));
    assert!(eval_err("index([1, 2], 9)"));
    assert_eq!(eval("first([1, 2])"), Value::Option(Some(Box::new(Value::Number(Number::from(1))))));
    assert_eq!(eval("last([1, 2])"), Value::Option(Some(Box::new(Value::Number(Number::from(2))))));
    assert_eq!(eval_fmt("linspace(0.0, 1.0, 3)"), "[0, 0.5, 1]");
}

#[test]
fn map_filter_reduce() {
    assert_eq!(
        eval("let f(x) = x^2;\nmap(f, [1, 2, 3])"),
        arr(vec![Value::Number(Number::from(1)), Value::Number(Number::from(4)), Value::Number(Number::from(9))])
    );
    assert_eq!(
        eval("let is_even(x) = x % 2 == 0;\nfilter(is_even, [1, 2, 3, 4])"),
        arr(vec![Value::Number(Number::from(2)), Value::Number(Number::from(4))])
    );
    assert_eq!(
        eval("reduce(|a, b| a + b, [1, 2, 3, 4], 0)"),
        Value::Number(Number::from(10))
    );
}

#[test]
fn string_split_returns_array() {
    assert_eq!(
        eval("let s = \"a,b,c\";\ns.split(\",\")"),
        arr(vec![Value::String("a".into()), Value::String("b".into()), Value::String("c".into())])
    );
}

#[test]
fn string_strip_join_find() {
    assert_eq!(eval("let s = \"  a  \";\ns.strip(\" \")"), Value::String("a".into()));
    assert_eq!(eval("let s = \"-\";\ns.join([\"x\", \"y\"])"), Value::String("x-y".into()));
    assert_eq!(eval("let s = \"hello world\";\ns.find(\"wor\")"), Value::Option(Some(Box::new(Value::Number(Number::from(6))))));
    assert_eq!(eval("let s = \"hello\";\ns.find(\"zzz\")"), Value::Option(None));
}

#[test]
fn empty_array_broadcast_and_empty_reduce_errors() {
    assert!(eval_err("let f(x) = x^2;\nf([])"));
    assert!(eval_err("let f(x) = x;\nf([\"a\"])"));
}
