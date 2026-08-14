use std::cell::RefCell;
use std::rc::Rc;

use prima_core::Number;
use prima_runtime::Evaluator;

fn eval(src: &str) -> prima_core::Value {
    Evaluator::new().eval_value(src).expect("eval failed")
}

fn run_src(src: &str) -> String {
    let out = Rc::new(RefCell::new(String::new()));
    let out_c = Rc::clone(&out);
    let mut ev = Evaluator::with_sink(move |s| out_c.borrow_mut().push_str(&s));
    ev.eval_src(src).expect("eval failed");
    out.borrow().clone()
}

#[test]
fn sum_1_to_100_closed_form() {
    assert_eq!(eval("let s = 0\nfor i in 1..100 { s += i }\ns"), prima_core::Value::Number(Number::from(5050)));
}

#[test]
fn sum_0_to_n_closed_form() {
    assert_eq!(eval("let s = 0\nfor i in 0..6 { s += i }\ns"), prima_core::Value::Number(Number::from(15)));
}

#[test]
fn non_accumulating_body_not_optimized() {
    assert_eq!(run_src("for i in 0..3 {\n    println(i)\n}"), "0\n1\n2\n");
}

#[test]
fn optimization_off_falls_back_to_loop() {
    assert_eq!(eval("config { loop_optimization := false }\nlet s = 0\nfor i in 0..5 { s += i }\ns"), prima_core::Value::Number(Number::from(10)));
}
