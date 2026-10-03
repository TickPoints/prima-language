//! Runtime diagnostic codes (spec appendix C.2) and `did you mean` suggestions (spec §16.4).
//!
//! Exercises the `RuntimeError::code()` surface end-to-end through the evaluator, plus the
//! evaluator's `did you mean` help for an unknown function call.

use prima_runtime::Evaluator;

/// Evaluate `src`, expecting an error, and return it.
fn err_of(src: &str) -> prima_runtime::RuntimeError {
    Evaluator::new()
        .eval_value(src)
        .expect_err("expected a runtime error")
}

#[test]
fn runtime_codes_are_structured() {
    assert_eq!(err_of("sum([])").code(), "R0014"); // empty_collection
    assert_eq!(err_of("d = {1: 2}; d[3]").code(), "R0012"); // key_not_found
    assert_eq!(err_of("[1, 2][5]").code(), "R0003"); // index_out_of_bounds
    assert_eq!(err_of("[1, 2] - [1]").code(), "R0004"); // dimension_mismatch
}

#[test]
fn unknown_function_offers_did_you_mean() {
    let err = err_of("sqr(2)");
    let help = err.help().unwrap_or_default();
    assert!(
        help.contains("sqrt"),
        "expected a `sqrt` suggestion for `sqr`, got: {help:?} (error: {err})"
    );
}
