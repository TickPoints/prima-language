use proptest::collection::vec;
use proptest::prelude::*;

use prima_core::Value;
use prima_runtime::Evaluator;

fn arb_string() -> impl Strategy<Value = String> {
    vec(any::<char>(), 0..128).prop_map(|v| v.into_iter().collect())
}

/// Evaluate a generated program with a fresh interpreter.
fn eval(src: &str) -> Value {
    prima_stdlib::init();
    Evaluator::new().eval_value(src).expect("generated program failed")
}

fn ints(values: &[i64]) -> String {
    values
        .iter()
        .map(i64::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

fn as_i64(v: &Value) -> i64 {
    match v {
        Value::Number(n) => n.as_i64().expect("integer"),
        other => panic!("expected Integer, got {other:?}"),
    }
}

proptest! {
    #[test]
    fn lexer_never_panics(s in arb_string()) {
        let _ = prima_syntax::lexer::lex(&s);
    }

    #[test]
    fn parser_never_panics(s in arb_string()) {
        let _ = prima_syntax::parse(&s);
    }

    #[test]
    fn array_concat_length(
        a in vec(-100i64..100, 0..12),
        b in vec(-100i64..100, 0..12),
    ) {
        let src = format!("let a = [{}];\nlet b = [{}];\nlen(a + b)", ints(&a), ints(&b));
        prop_assert_eq!(as_i64(&eval(&src)), (a.len() + b.len()) as i64);
    }

    #[test]
    fn broadcast_preserves_shape(values in vec(-50i64..50, 1..12)) {
        let src = format!(
            "let f(x) = x * 2 + 1;\nlet a = [{}];\nlen(f(a))",
            ints(&values)
        );
        prop_assert_eq!(as_i64(&eval(&src)), values.len() as i64);
    }

    #[test]
    fn insert_remove_round_trip(
        values in vec(-50i64..50, 1..10),
        slot in 0usize..10,
    ) {
        let index = slot % values.len();
        let src = format!(
            "let mut a = [{}];\na.insert({index}, 99);\na.remove({index});\na",
            ints(&values)
        );
        let expected = Value::Array(
            values
                .iter()
                .map(|v| Value::Number(prima_core::Number::from(*v)))
                .collect::<Vec<_>>()
                .into(),
        );
        prop_assert_eq!(eval(&src), expected);
    }

    #[test]
    fn set_dedup_length(values in vec(0i64..8, 0..20)) {
        let unique = values.iter().collect::<std::collections::HashSet<_>>().len();
        let src = format!("len({{{}}})", ints(&values));
        prop_assert_eq!(as_i64(&eval(&src)), unique as i64);
    }
}
