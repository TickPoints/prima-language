//! Layered `@builtin(O2)` method consistency (spec §18.4/§18.1, Phase 10): for each layered builtin
//! method the Rust fast path (used at `opt_level >= O2`) and the `.pra` fallback body (used at O0)
//! must produce identical results — the `.pra` body is the semantic authority.

use prima_core::Value;
use prima_runtime::Evaluator;

fn eval_at(opt_level: &str, src: &str) -> Value {
    prima_stdlib::init();
    let program = format!("config {{ opt_level := {opt_level} }}\n{src}");
    Evaluator::new()
        .eval_value(&program)
        .unwrap_or_else(|e| panic!("eval failed at {opt_level} for {src:?}: {e}"))
}

/// Assert the `.pra` fallback (O0) and the native fast path (O2) agree for the given program.
fn consistent(src: &str) {
    let o0 = eval_at("O0", src);
    let o2 = eval_at("O2", src);
    assert_eq!(o0, o2, "O0/O2 diverge for {src:?}");
}

#[test]
fn string_layered_methods() {
    consistent("let s = \"a,b,,c\";\ns.split(\",\")");
    consistent("let s = \"aé,b\";\ns.split(\"\")");
    consistent("let s = \"a-b\";\ns.split(\"-\")");
    consistent("let s = \"\";\ns.split(\",\")");
    consistent("let s = \"ababa\";\ns.replace(\"aba\", \"X\")");
    consistent("let s = \"hello\";\ns.replace(\"\", \"-\")");
    consistent("let s = \"hello\";\ns.replace(\"x\", \"y\")");
    consistent("let s = \"  xxy  \";\ns.strip(\" x\")");
    consistent("let s = \"abc\";\ns.strip(\"z\")");
    consistent("let s = \"hello\";\ns.find(\"ll\")");
    consistent("let s = \"hello\";\ns.find(\"zz\")");
    consistent("let s = \"hello\";\ns.find(\"\")");
    consistent("let s = \"aéa\";\ns.find(\"é\")");
    consistent("\"-\".join([\"x\", \"y\", \"z\"])");
    consistent("\"\".join([])");
    consistent("\"\".join([\"a\", \"b\"])");
}

#[test]
fn collection_layered_methods() {
    consistent("let a = [1, 2, 3];\na.copy()");
    consistent("let a = [];\na.copy()");
    consistent("let d = { \"a\": 1, \"b\": 2 };\nd.copy()");
    consistent("let s = {1, 2, 3};\ns.symmetric_difference({3, 4})");
}

#[test]
fn char_layered_method() {
    consistent("'5'.is_digit()");
    consistent("'a'.is_digit()");
    consistent("'\\u{3c0}'.is_digit()");
}
