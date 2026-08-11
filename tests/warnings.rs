// Warning tests (spec §16.5): `W0001` (newline-separated statements), `W0002` (deprecated `|>` pipeline),
// and `W0005` (operator-overload use) with configurable `overload_policy`.
use prima_runtime::Evaluator;

fn warnings_of(src: &str) -> Vec<String> {
    let mut ev = Evaluator::new();
    ev.eval_value(src).expect("eval failed");
    ev.warnings().iter().map(|w| w.code.to_string()).collect()
}

#[test]
fn newline_separated_statements_emit_w0001() {
    let ws = warnings_of("let x = 1\nx + 1\n");
    assert!(ws.iter().any(|c| c == "W0001"), "warnings = {ws:?}");
}

#[test]
fn semicolon_terminated_statements_emit_no_warning() {
    let ws = warnings_of("let x = 1;\nx + 1;");
    assert!(!ws.iter().any(|c| c == "W0001"), "warnings = {ws:?}");
}

#[test]
fn pipeline_emits_w0002() {
    let ws = warnings_of("let x = 1;\nx |> to_f64");
    assert!(ws.iter().any(|c| c == "W0002"), "warnings = {ws:?}");
}

#[test]
fn pipeline_still_evaluates() {
    let mut ev = Evaluator::new();
    let v = ev.eval_value("let f(x) = x^2;\n3 |> f").expect("eval failed");
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
    ev.eval_value("let x = 1;\nx |> to_f64").expect("eval failed");
    assert!(ev.warnings().iter().any(|w| w.code == "W0002"));
    // A fresh parse resets the warning list (spec §16.5: collected since the last parse entry point).
    ev.eval_value("let y = 2;").expect("eval failed");
    assert!(!ev.warnings().iter().any(|w| w.code == "W0002"), "warnings = {:?}", ev.warnings());
}
