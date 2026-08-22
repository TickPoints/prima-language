use std::cell::RefCell;
use std::rc::Rc;

use prima_core::Number;
use prima_runtime::Evaluator;

fn eval(src: &str) -> prima_core::Value {
    Evaluator::new().eval_value(src).expect("eval failed")
}

fn eval_fmt(src: &str) -> String {
    let mut ev = Evaluator::new();
    let v = ev.eval_value(src).expect("eval failed");
    ev.format_value(&v)
}

/// Run `src` and capture `print` output into a string.
fn run_src(src: &str) -> String {
    let out = Rc::new(RefCell::new(String::new()));
    let out_c = Rc::clone(&out);
    let mut ev = Evaluator::with_sink(move |s| out_c.borrow_mut().push_str(&s));
    ev.eval_src(src).expect("eval failed");
    out.borrow().clone()
}

#[test]
fn fn_with_if_else_return() {
    let v = eval("fn classify(x: Integer) -> String {\n    if x > 0 {\n        return \"positive\"\n    } else if x < 0 {\n        return \"negative\"\n    } else {\n        return \"zero\"\n    }\n}\nclassify(-3)");
    assert_eq!(v, prima_core::Value::String("negative".into()));
}

#[test]
fn while_loop_updates_outer_variable() {
    assert_eq!(run_src("let i = 0;\nwhile i < 3 {\n    println(i);\n    i += 1\n}"), "0\n1\n2\n");
}

#[test]
fn for_loop_with_step() {
    assert_eq!(run_src("for i in 0..10 step 2 {\n    println(i)\n}"), "0\n2\n4\n6\n8\n");
}

#[test]
fn for_loop_accumulates_when_optimization_off() {
    let v = eval("config { loop_optimization := false }\nlet s = 0;\nfor i in 0..5 {\n    s += i\n}\ns");
    assert_eq!(v, prima_core::Value::Number(Number::from(10)));
}

#[test]
fn return_propagates_from_while_body() {
    let v = eval("fn first(x: Integer) -> Integer {\n    while x > 0 {\n        return x\n    }\n    return 0\n}\nfirst(7)");
    assert_eq!(v, prima_core::Value::Number(Number::from(7)));
}

#[test]
fn match_on_error_result_binds_error() {
    // v2.0 (spec §16.3): no `try`/`catch` — `match` on the `Result` of a `try_*` collapse.
    let out = run_src("let x = try_i32(1e20);\nmatch x {\n    Ok(v) => println(v),\n    Err(e) => println(\"caught\", e)\n}");
    assert!(out.contains("caught"), "out = {out:?}");
}

#[test]
fn error_result_arm_handles_overflow() {
    let out = run_src("let x = try_i32(1e20);\nmatch x {\n    Ok(v) => println(\"wrong\", v),\n    Err(e) => println(\"overflow\")\n}");
    assert!(out.contains("overflow"), "out = {out:?}");
}

#[test]
fn success_result_skips_err_arm() {
    assert_eq!(run_src("let x = try_i32(7);\nmatch x {\n    Ok(v) => println(v),\n    Err(e) => println(\"never\")\n}"), "7\n");
}

#[test]
fn fn_returns_nil_without_return() {
    assert_eq!(eval_fmt("fn f(x: Integer) -> Integer {\n    let y = x\n}\nf(3)"), "nil");
}

#[test]
fn tail_call_optimization_avoids_stack_overflow() {
    // TCO (spec §10.2): a tail-recursive `fn` is trampolined, so a 100k-deep recursion runs in
    // constant stack space instead of overflowing.
    let v = eval("fn sum(n: Integer, acc: Integer) -> Integer {\n    if n == 0 {\n        return acc\n    }\n    return sum(n - 1, acc + n)\n}\nsum(100000, 0)");
    assert_eq!(v, prima_core::Value::Number(Number::from(5000050000_i64)));
}

#[test]
fn tail_call_early_return_in_prefix_still_works() {
    // An early `return` inside the effect-free prefix of a tail-call body must exit, not tail-jump.
    let v = eval("fn f(n: Integer) -> Integer {\n    if n == 0 {\n        return 42\n    }\n    return f(n - 1)\n}\nf(5)");
    assert_eq!(v, prima_core::Value::Number(Number::from(42)));
}
