// Parallelism (spec §17): `@parallel` MFn broadcast and `parfor`.
use prima_core::Value;
use prima_runtime::Evaluator;

fn eval(src: &str) -> Value {
    Evaluator::new().eval_value(src).expect("eval failed")
}

fn eval_fmt(src: &str) -> String {
    let mut ev = Evaluator::new();
    let v = ev.eval_value(src).expect("eval failed");
    ev.format_value(&v)
}

fn eval_array(src: &str) -> Vec<prima_core::Number> {
    match eval(src) {
        Value::Array(a) => a
            .into_iter()
            .map(|v| match v {
                Value::Number(n) => n,
                other => panic!("expected a numeric array element, got {other:?}"),
            })
            .collect(),
        other => panic!("expected an array, got {other:?}"),
    }
}

#[test]
fn parfor_writes_index_slots() {
    assert_eq!(eval_array("let A = [0, 0, 0, 0, 0];\nparfor i in 0..5 {\n    A[i] = i * i;\n}\nA"), [0, 1, 4, 9, 16].into_iter().map(prima_core::Number::from).collect::<Vec<_>>());
}

#[test]
fn parfor_with_step() {
    assert_eq!(eval_array("let A = [0, 0, 0, 0, 0, 0];\nparfor i in 0..6 step 2 {\n    A[i] = i + 100;\n}\nA"), [100, 0, 102, 0, 104, 0].into_iter().map(prima_core::Number::from).collect::<Vec<_>>());
}

#[test]
fn parfor_reads_other_array() {
    // Reading `A[j]` while writing independent slots is allowed (read-only snapshots).
    assert_eq!(eval_array("let A = [1, 2, 3, 4];\nlet B = [0, 0, 0, 0];\nparfor i in 0..4 {\n    B[i] = A[i] * 2;\n}\nB"), [2, 4, 6, 8].into_iter().map(prima_core::Number::from).collect::<Vec<_>>());
}

#[test]
fn parfor_add_assign() {
    assert_eq!(eval_array("let A = [1, 1, 1];\nparfor i in 0..3 {\n    A[i] += 10;\n}\nA"), [11, 11, 11].into_iter().map(prima_core::Number::from).collect::<Vec<_>>());
}

#[test]
fn parfor_rejects_side_effects() {
    // `print` (impure) in the body is a compile-time error (E0082).
    let src = "let A = [0, 0];\nparfor i in 0..2 {\n    print(i);\n}";
    assert!(Evaluator::new().eval_src(src).is_err());
}

#[test]
fn parfor_out_of_bounds_errors() {
    let src = "let A = [0, 0];\nparfor i in 0..4 {\n    A[i] = i;\n}";
    assert!(Evaluator::new().eval_src(src).is_err());
}

#[test]
fn parallel_broadcast_small_array_sequential_equivalence() {
    // Under the threshold the sequential path runs; results match the `@parallel` contract.
    assert_eq!(eval_array("let f(x) @parallel = x^2 + 1;\nlet v = [1, 2, 3];\nf(v)"), [2, 5, 10].into_iter().map(prima_core::Number::from).collect::<Vec<_>>());
}

#[test]
fn parallel_broadcast_large_array() {
    // Above the threshold the rayon path runs; sample the endpoints.
    let src = "let f(x) @parallel = x^2 + 1;\nlet v = range(0, 5000);\nlet w = f(v);\n[w[0], w[4999]]";
    assert_eq!(eval_fmt(src), "[1, 24990002]");
}

#[test]
fn range_builtin() {
    assert_eq!(eval_array("range(0, 4)"), [0, 1, 2, 3].into_iter().map(prima_core::Number::from).collect::<Vec<_>>());
    assert_eq!(eval_array("range(2, 10, 3)"), [2, 5, 8].into_iter().map(prima_core::Number::from).collect::<Vec<_>>());
    assert_eq!(eval_array("range(5, 0, -2)"), [5, 3, 1].into_iter().map(prima_core::Number::from).collect::<Vec<_>>());
}
