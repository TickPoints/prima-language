// Warning tests (spec §16.5): newline-separated statements and the `|>` pipeline were
// removed in v2.3 and are now hard parse errors (E0011/E0010); `W0006` (removed `format`
// function) and `W0005` (operator-overload use) remain configurable warnings.
use prima_runtime::Evaluator;

fn warnings_of(src: &str) -> Vec<String> {
    let mut ev = Evaluator::new();
    ev.eval_value(src).expect("eval failed");
    ev.warnings().iter().map(|w| w.code.to_string()).collect()
}

#[test]
fn format_call_emits_w0006() {
    // `format` was removed (spec §18.1): the parser records the W0006 deprecation warning even
    // though evaluation then fails with an unknown-function error.
    let mut ev = Evaluator::new();
    let _ = ev.eval_value(r#"let s = format("a is {}", 42);"#);
    let ws: Vec<String> = ev.warnings().iter().map(|w| w.code.to_string()).collect();
    assert!(ws.iter().any(|c| c == "W0006"), "warnings = {ws:?}");
}

#[test]
fn fstring_emits_no_w0006() {
    let ws = warnings_of(
        r#"let a = 42;
let s = f"a is {a}";"#,
    );
    assert!(!ws.iter().any(|c| c == "W0006"), "warnings = {ws:?}");
}

#[test]
fn newline_separated_statements_are_parse_error() {
    let err = Evaluator::new()
        .eval_value("let x = 1\nx + 1\n")
        .unwrap_err();
    assert!(err.to_string().contains("E0011"), "error = {err}");
}

#[test]
fn semicolon_terminated_statements_evaluate() {
    assert!(Evaluator::new().eval_value("let x = 1;\nx + 1;").is_ok());
}

#[test]
fn pipeline_is_parse_error() {
    let err = Evaluator::new()
        .eval_value("let x = 1;\nx |> to_f64")
        .unwrap_err();
    assert!(err.to_string().contains("E0010"), "error = {err}");
}

#[test]
fn direct_call_evaluates() {
    let v = Evaluator::new()
        .eval_value("let f(x) = x^2;\nf(3)")
        .expect("eval failed");
    assert_eq!(v, prima_core::Value::Number(prima_core::Number::from(9)));
}

#[test]
fn operator_overload_emits_w0005_by_default() {
    let src = "class Vec2 { pub x: F64, pub y: F64 }\nimpl ops::Add for Vec2 {\n    fn add(self, rhs) -> Vec2 { Vec2 { x: self.x + rhs.x, y: self.y + rhs.y } }\n}\nlet a = Vec2 { x: 1, y: 2 };\nlet b = Vec2 { x: 3, y: 4 };\na + b";
    let ws = warnings_of(src);
    assert!(ws.iter().any(|c| c == "W0005"), "warnings = {ws:?}");
}

#[test]
fn overload_policy_allow_silences_w0005() {
    let src = "class Vec2 { pub x: F64, pub y: F64 }\nimpl ops::Add for Vec2 {\n    fn add(self, rhs) -> Vec2 { Vec2 { x: self.x + rhs.x, y: self.y + rhs.y } }\n}\nlet a = Vec2 { x: 1, y: 2 };\nlet b = Vec2 { x: 3, y: 4 };\nwith config { overload_policy := allow } {\n    a + b\n}";
    let ws = warnings_of(src);
    assert!(!ws.iter().any(|c| c == "W0005"), "warnings = {ws:?}");
}

#[test]
fn overload_policy_deny_errors() {
    let src = "class V { pub x: F64 }\nimpl ops::Add for V {\n    fn add(self, rhs) -> V { V { x: self.x } }\n}\nwith config { overload_policy := deny } {\n    let a = V { x: 1 };\n    a + a\n}";
    assert!(Evaluator::new().eval_value(src).is_err());
}

#[test]
fn warnings_do_not_accumulate_across_eval_value() {
    let mut ev = Evaluator::new();
    let src = "class Vec2 { pub x: F64, pub y: F64 }\nimpl ops::Add for Vec2 {\n    fn add(self, rhs) -> Vec2 { Vec2 { x: self.x + rhs.x, y: self.y + rhs.y } }\n}\nlet a = Vec2 { x: 1, y: 2 };\nlet b = Vec2 { x: 3, y: 4 };\na + b";
    ev.eval_value(src).expect("eval failed");
    assert!(ev.warnings().iter().any(|w| w.code == "W0005"));
    // A fresh parse resets the warning list (spec §16.5: collected since the last parse entry point).
    ev.eval_value("let y = 2;").expect("eval failed");
    assert!(
        !ev.warnings().iter().any(|w| w.code == "W0005"),
        "warnings = {:?}",
        ev.warnings()
    );
}
