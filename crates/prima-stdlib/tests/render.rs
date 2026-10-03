#![cfg(feature = "render")]
//! `render` module tests (spec §18.6): symbolic/LaTeX → SVG and Unicode terminal text.

use prima_core::Value;
use prima_runtime::Evaluator;

fn eval(src: &str) -> Value {
    prima_stdlib::init();
    Evaluator::new().eval_value(src).expect("eval failed")
}

fn string_of(v: Value) -> String {
    match v {
        Value::String(s) => s.to_string(),
        other => panic!("expected String, got {other:?}"),
    }
}

#[test]
fn terminal_renders_symbolic_expression() {
    // An undefined name becomes a symbol, so `x^2 + 1` is a symbolic expression.
    let s = string_of(eval("import render;\nrender::to_terminal(x^2 + 1)"));
    assert!(s.contains('x'), "{s}");
    assert!(s.contains('²'), "expected a superscript two: {s}");
}

#[test]
fn terminal_renders_latex_string() {
    let s = string_of(eval(
        "import render;\nrender::to_terminal(\"\\\\frac{1}{2}\")",
    ));
    assert_eq!(s, "(1)/(2)");
    let s = string_of(eval("import render;\nrender::to_terminal(\"\\\\sqrt{2}\")"));
    assert_eq!(s, "√2");
}

#[test]
fn svg_is_self_contained() {
    let v = eval("import render;\nrender::to_svg(x^2 + 1)");
    let Value::Result(Ok(inner)) = v else {
        panic!("expected Result<Ok>, got {v:?}");
    };
    let svg = string_of(*inner);
    assert!(svg.contains("<svg"), "not an SVG document: {svg:.80}");
    // The `embed-fonts` build emits glyph outlines, not a bare `<text>`-only document.
    assert!(svg.contains("</svg>"));
}

#[test]
fn invalid_latex_is_an_error_not_a_panic() {
    let v = eval("import render;\nrender::to_svg(\"\\\\frac{1}\")");
    assert!(matches!(v, Value::Result(Err(_))), "got {v:?}");
}
