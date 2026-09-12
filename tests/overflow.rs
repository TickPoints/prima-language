//! Integer overflow behavior (spec §16.1 R0001): loop index stepping (`for`/`parfor`/`range`)
//! uses checked arithmetic and reports `overflow` instead of silently wrapping (which previously
//! panicked in debug builds or looped/wrapped silently in release), and the arithmetic-sum closed
//! form is exact (`BigInt`), matching the real loop's exact accumulation.

use prima_core::{Number, Value};
use prima_runtime::Evaluator;

fn eval(src: &str) -> Result<Value, prima_runtime::error::RuntimeError> {
    prima_stdlib::init();
    Evaluator::new().eval_value(src)
}

const I64_MIN: &str = "-9223372036854775808";
const I64_MAX: &str = "9223372036854775807";

#[test]
fn for_loop_step_overflow_is_an_error() {
    // `i += step` overflows after the first iteration: an error, not a debug panic / wraparound.
    let src = format!("for i in -5..-100 step {I64_MIN} {{\n    let x = i;\n}}");
    let e = eval(&src).expect_err("expected an overflow error");
    assert!(
        e.to_string().to_lowercase().contains("overflow"),
        "unexpected error: {e}"
    );
}

#[test]
fn for_loop_step_overflow_positive_direction() {
    let src = format!("for i in 1..3 step {I64_MAX} {{\n    let x = i;\n}}");
    let e = eval(&src).expect_err("expected an overflow error");
    assert!(
        e.to_string().to_lowercase().contains("overflow"),
        "unexpected error: {e}"
    );
}

#[test]
fn range_builtin_step_overflow_is_an_error() {
    // `range(1, 3, i64::MAX)`: the second index `1 + i64::MAX` overflows.
    let src = format!("range(1, 3, {I64_MAX})");
    let e = eval(&src).expect_err("expected an overflow error");
    assert!(
        e.to_string().to_lowercase().contains("overflow"),
        "unexpected error: {e}"
    );
}

#[test]
fn parfor_iteration_count_overflow_is_an_error() {
    // The full i64 range has more than i64::MAX iterations: the closed-form count must not wrap
    // into a negative `as usize` allocation.
    let src = format!("parfor i in {I64_MIN}..{I64_MAX} {{\n}}");
    let e = eval(&src).expect_err("expected an overflow error");
    assert!(
        e.to_string().to_lowercase().contains("overflow"),
        "unexpected error: {e}"
    );
}

#[test]
fn closed_form_sum_is_exact_beyond_i64() {
    // `for i in 0..5_000_000_000 { s += i }` — the closed form `n(n-1)/2` overflows i64, so the
    // result must be computed exactly (5e9 * (5e9-1) / 2 = 12499999997500000000 > i64::MAX).
    let v = eval("let s = 0;\nfor i in 0..5000000000 { s += i }\ns").expect("eval failed");
    assert_eq!(
        v,
        Value::Number(Number::Integer(Box::new("12499999997500000000".parse().unwrap())))
    );
}

#[test]
fn closed_form_sum_matches_the_real_loop() {
    // For values where the exact sum still fits i64, the closed form and the real loop agree.
    let closed = eval("let s = 0;\nfor i in 0..4000000 { s += i }\ns").expect("eval failed");
    assert_eq!(closed, Value::Number(Number::from(7999998000000i64)));
    let looped = eval("config { loop_optimization := false }\nlet s = 0;\nfor i in 0..4000000 { s += i }\ns")
        .expect("eval failed");
    assert_eq!(closed, looped);
}

#[test]
fn closed_form_start_one_sum_is_exact_beyond_i64() {
    // `1..n` closed form is n(n+1)/2; at n = 5e9 it exceeds i64::MAX.
    let v = eval("let s = 0;\nfor i in 1..5000000000 { s += i }\ns").expect("eval failed");
    assert_eq!(
        v,
        Value::Number(Number::Integer(Box::new("12500000002500000000".parse().unwrap())))
    );
}
