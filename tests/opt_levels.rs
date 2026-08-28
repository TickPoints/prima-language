use std::cell::RefCell;
use std::rc::Rc;

use prima_core::Value;
use prima_runtime::Evaluator;

/// Evaluate an in-memory program with a given `opt_level` prepended (spec §10.2), returning the
/// last value. The default (no config) is `O2`.
fn eval_at(opt_level: &str, src: &str) -> Value {
    let program = format!("config {{ opt_level := {opt_level} }}\n{src}");
    Evaluator::new().eval_value(&program).expect("eval failed")
}

/// Run a program, capturing everything written to the console sink (spec §18.1b).
fn run_src(src: &str) -> String {
    let out = Rc::new(RefCell::new(String::new()));
    let out_c = Rc::clone(&out);
    let mut ev = Evaluator::with_sink(move |s| out_c.borrow_mut().push_str(&s));
    ev.eval_src(src).expect("eval failed");
    out.borrow().clone()
}

#[test]
fn opt_level_does_not_change_array_results() {
    // Dense F64 array elementwise ops (spec §11.4): `O3` uses the SIMD kernel (spec §10.2) while
    // `O0`/`O2` use the scalar loop. IEEE lane arithmetic is bit-identical, so results agree.
    let src = "let a = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0];\na * 2.0";
    let o0 = eval_at("O0", src);
    let o2 = eval_at("O2", src);
    let o3 = eval_at("O3", src);
    assert_eq!(o3, o0);
    assert_eq!(o3, o2);

    let src_left = "let a = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0];\n2.0 - a";
    assert_eq!(eval_at("O0", src_left), eval_at("O3", src_left));

    let src_aa = "let a = [10.0, 20.0, 30.0, 40.0];\nlet b = [2.0, 4.0, 5.0, 8.0];\na / b";
    assert_eq!(eval_at("O0", src_aa), eval_at("O3", src_aa));
}

#[test]
fn opt_level_does_not_change_loop_results() {
    // The arithmetic-series closed form (spec §10.1) is gated at `opt_level >= O1` (spec §10.2);
    // at `O0` the loop runs natively, but the observable result is identical.
    let src = "let s = 0;\nfor i in 0..100 { s += i }\ns";
    assert_eq!(eval_at("O0", src), eval_at("O2", src));
}

#[test]
fn default_opt_level_is_o2() {
    // With no `config`, `opt_level` defaults to `O2` (spec §13.2), which must match an explicit `O2`.
    let src = "let s = 0;\nfor i in 1..5 { s += i }\ns";
    let default_v = Evaluator::new().eval_value(src).expect("eval failed");
    assert_eq!(default_v, eval_at("O2", src));
}

#[test]
fn simplify_level_controls_symbolic_depth() {
    // `simplify_level` (spec §8.3) is independent of `opt_level`: at level 2 `\sin(0)` folds to `0`,
    // at level 0 the builtin constant-folding rule does not fire and the call is preserved.
    let level2 = run_src("config { simplify_level := 2 }\nprintln(tex\"\\sin(0)\");\n");
    let level0 = run_src("config { simplify_level := 0 }\nprintln(tex\"\\sin(0)\");\n");
    assert_eq!(level2.trim(), "0");
    assert!(
        level0.contains("\\sin"),
        "expected unreduced call, got: {level0:?}"
    );
}

#[test]
fn explicit_simplify_is_full_regardless_of_level() {
    // The explicit `simplify(...)` builtin always applies the full rule set (spec §8.3 level 3),
    // even under a reduced `simplify_level` config.
    let out = run_src("config { simplify_level := 0 }\nprintln(simplify(tex\"\\sin(0)\"));\n");
    assert_eq!(out.trim(), "0");
}
