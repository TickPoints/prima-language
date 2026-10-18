//! `mem::Arc` behavior (spec §12.3/§12.4): explicit counting, payload access, and the rule that
//! ordinary class-instance copies (`let b = a`) do not change the explicit count.

#![cfg(feature = "core")]

use prima_core::{Number, Value};
use prima_runtime::Evaluator;

fn eval(src: &str) -> Value {
    prima_stdlib::init();
    Evaluator::new().eval_value(src).expect("eval failed")
}

fn int(v: &Value) -> i64 {
    match v {
        Value::Number(n) => n.as_i64().expect("integer"),
        other => panic!("expected Integer, got {other:?}"),
    }
}

#[test]
fn new_starts_at_one() {
    let v = eval("import mem;\nlet a = mem::Arc::new(42);\na.strong_count()");
    assert_eq!(int(&v), 1);
}

#[test]
fn clone_increments_and_returns_the_payload() {
    let v = eval(
        "import mem;\nlet a = mem::Arc::new(42);\nlet b = a.clone();\nlet c = a.clone();\na.strong_count()",
    );
    assert_eq!(int(&v), 3);

    let payload = eval("import mem;\nlet a = mem::Arc::new(42);\nlet b = a.clone();\nb.get()");
    assert_eq!(payload, Value::Number(Number::from(42)));
}

#[test]
fn ordinary_copy_does_not_increment() {
    // `let d = a` shares the class-instance handle (spec §12.3); only `clone()` counts.
    let v = eval(
        "import mem;\nlet a = mem::Arc::new(1);\nlet b = a.clone();\nlet d = a;\na.strong_count()",
    );
    assert_eq!(int(&v), 2);
}

#[test]
fn get_returns_a_class_payload_as_a_shared_handle() {
    let v = eval(
        "import mem;\nclass Box { pub x: Integer, pub fn new(x: Integer) -> Self { Box { x } } }\n\
         let a = mem::Arc::new(Box::new(7));\na.get().x",
    );
    assert_eq!(int(&v), 7);
}
