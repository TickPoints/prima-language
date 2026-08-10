use proptest::collection::vec;
use proptest::prelude::*;

fn arb_string() -> impl Strategy<Value = String> {
    vec(any::<char>(), 0..128).prop_map(|v| v.into_iter().collect())
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
}
