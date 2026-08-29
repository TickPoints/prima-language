//! Golden behavior table for the `String` method set (spec §18.1, Phase 10): each case's expected
//! value follows Python 3 `str` semantics (the method set's reference), adapted where Prima deviates
//! (`split("")` → single characters, `casefold` → simplified lowercase, `is_digit` → Unicode Numeric).

use prima_core::{Number, Value};
use prima_runtime::Evaluator;

fn eval(src: &str) -> Value {
    prima_stdlib::init();
    Evaluator::new()
        .eval_value(src)
        .unwrap_or_else(|e| panic!("eval failed for {src:?}: {e}"))
}

fn eval_str(src: &str) -> String {
    match eval(src) {
        Value::String(s) => s,
        other => panic!("expected String for {src:?}, got {other:?}"),
    }
}

fn eval_arr(src: &str) -> Vec<Value> {
    match eval(src) {
        Value::Array(a) => a,
        other => panic!("expected Array for {src:?}, got {other:?}"),
    }
}

fn eval_bool(src: &str) -> bool {
    match eval(src) {
        Value::Bool(b) => b,
        other => panic!("expected Bool for {src:?}, got {other:?}"),
    }
}

fn s(v: &str) -> Value {
    Value::String(v.into())
}

#[test]
fn case_conversion() {
    assert_eq!(eval_str(r#""héllo".to_upper()"#), "HÉLLO");
    assert_eq!(eval_str(r#""HÉLLO".to_lower()"#), "héllo");
    assert_eq!(eval_str(r#""aXb".swapcase()"#), "AxB");
    assert_eq!(eval_str(r#""hello WORLD".swapcase()"#), "HELLO world");
    assert_eq!(eval_str(r#""hÉllo".capitalize()"#), "Héllo");
    assert_eq!(eval_str(r#""hello".capitalize()"#), "Hello");
    assert_eq!(eval_str(r#""".capitalize()"#), "");
    assert_eq!(eval_str(r#""hello world".title()"#), "Hello World");
    assert_eq!(eval_str(r#""he'S A Guy".title()"#), "He'S A Guy");
    assert_eq!(eval_str(r#""Straße".casefold()"#), "straße");
}

#[test]
fn predicates() {
    assert!(eval_bool(r#""ABC123".is_upper()"#));
    assert!(!eval_bool(r#""AbC".is_upper()"#));
    assert!(!eval_bool(r#""123".is_upper()"#));
    assert!(eval_bool(r#""abc".is_lower()"#));
    assert!(!eval_bool(r#""aBc".is_lower()"#));
    assert!(!eval_bool(r#""123".is_lower()"#));
    assert!(eval_bool(r#""abc".is_alpha()"#));
    assert!(!eval_bool(r#""abc1".is_alpha()"#));
    assert!(!eval_bool(r#""".is_alpha()"#));
    assert!(eval_bool(r#""123".is_digit()"#));
    assert!(eval_bool(r#""١٢٣".is_digit()"#));
    assert!(!eval_bool(r#""12a".is_digit()"#));
    assert!(!eval_bool(r#""".is_digit()"#));
    assert!(eval_bool(r#""abc123".is_alnum()"#));
    assert!(!eval_bool(r#""abc_".is_alnum()"#));
    assert!(eval_bool(r#"" \t".is_space()"#));
    assert!(!eval_bool(r#""a ".is_space()"#));
    assert!(eval_bool(r#""".is_ascii()"#));
    assert!(eval_bool(r#""abc".is_ascii()"#));
    assert!(!eval_bool(r#""π".is_ascii()"#));
    assert!(eval_bool(r#""hello".is_empty() == false"#));
    assert!(eval_bool(r#""".is_empty()"#));
}

#[test]
fn search_and_replace() {
    assert_eq!(
        eval("let s = \"hello\";\ns.find(\"ll\")"),
        Value::Option(Some(Box::new(Value::Number(Number::from(2)))))
    );
    assert_eq!(
        eval("let s = \"hello\";\ns.find(\"zz\")"),
        Value::Option(None)
    );
    assert_eq!(
        eval("let s = \"hello\";\ns.find(\"\")"),
        Value::Option(Some(Box::new(Value::Number(Number::from(0)))))
    );
    assert_eq!(
        eval("let s = \"hello\";\ns.rfind(\"l\")"),
        Value::Option(Some(Box::new(Value::Number(Number::from(3)))))
    );
    assert_eq!(
        eval("let s = \"hello\";\ns.rfind(\"\")"),
        Value::Option(Some(Box::new(Value::Number(Number::from(5)))))
    );
    assert_eq!(
        eval("let s = \"banana\";\ns.count(\"an\")"),
        Value::Number(Number::from(2))
    );
    assert_eq!(
        eval("let s = \"hello\";\ns.count(\"\")"),
        Value::Number(Number::from(6))
    );
    assert_eq!(eval_str(r#""ababa".replace("aba", "X")"#), "Xba");
    assert_eq!(eval_str(r#""ababa".replace("", "-")"#), "ababa");
    assert!(eval_bool(r#""hello world".contains("world")"#));
    assert!(!eval_bool(r#""hello".contains("xyz")"#));
    assert!(eval_bool(r#""hello".starts_with("he")"#));
    assert!(!eval_bool(r#""hello".starts_with("el")"#));
    assert!(eval_bool(r#""hello".ends_with("llo")"#));
    assert!(!eval_bool(r#""hello".ends_with("ell")"#));
}

#[test]
fn split_join() {
    assert_eq!(
        eval_arr(r#""a,b,,c".split(",")"#),
        vec![s("a"), s("b"), s(""), s("c")]
    );
    // `split("")` yields the single characters (spec §18.1, like Python `list(s)`).
    assert_eq!(
        eval_arr(r#""aé,b".split("")"#),
        vec![s("a"), s("é"), s(","), s("b")]
    );
    assert_eq!(
        eval_arr(r#""a-b-c".split("-")"#),
        vec![s("a"), s("b"), s("c")]
    );
    assert_eq!(eval_str(r#""-".join(["x", "y", "z"])"#), "x-y-z");
    assert_eq!(eval_str(r#""".join(["a", "b"])"#), "ab");
    assert_eq!(eval_str(r#""-".join([])"#), "");
    assert_eq!(
        eval_arr(r#""a\nb\nc\n".splitlines()"#),
        vec![s("a"), s("b"), s("c")]
    );
    assert_eq!(eval_arr(r#""a\r\nb".splitlines()"#), vec![s("a"), s("b")]);
    assert_eq!(eval_arr(r#""".splitlines()"#), Vec::<Value>::new());
    assert_eq!(eval_arr(r#""\n".splitlines()"#), vec![s("")]);
}

#[test]
fn strip_and_padding() {
    assert_eq!(eval_str(r#""  hi  ".trim()"#), "hi");
    assert_eq!(eval_str(r#""  xxy  ".strip(" x")"#), "y");
    assert_eq!(eval_str(r#""  xx".lstrip(" x")"#), "");
    assert_eq!(eval_str(r#""xx  ".rstrip(" x")"#), "");
    assert_eq!(eval_str(r#""hi".center(7, "-")"#), "--hi---");
    assert_eq!(eval_str(r#""hi".ljust(5, ".")"#), "hi...");
    assert_eq!(eval_str(r#""hi".rjust(5, ".")"#), "...hi");
    assert_eq!(eval_str(r#""42".zfill(5)"#), "00042");
    assert_eq!(eval_str(r#""-42".zfill(6)"#), "-00042");
    assert_eq!(eval_str(r#""+7".zfill(4)"#), "+007");
    assert_eq!(eval_str(r#""a\tb".expandtabs(4)"#), "a   b");
    assert_eq!(eval_str(r#""a\tb".expandtabs(0)"#), "a\tb");
}

#[test]
fn prefix_suffix_and_partition() {
    assert_eq!(eval_str(r#""hello".removeprefix("he")"#), "llo");
    assert_eq!(eval_str(r#""hello".removeprefix("xyz")"#), "hello");
    assert_eq!(eval_str(r#""hello".removesuffix("lo")"#), "hel");
    assert_eq!(eval_str(r#""hello".removesuffix("zz")"#), "hello");
    assert_eq!(
        eval(r#""hello world".partition(" ")"#),
        Value::Tuple(vec![s("hello"), s(" "), s("world")])
    );
    assert_eq!(
        eval(r#""hello".partition("x")"#),
        Value::Tuple(vec![s("hello"), s(""), s("")])
    );
    assert_eq!(
        eval(r#""hello world".rpartition("o")"#),
        Value::Tuple(vec![s("hello w"), s("o"), s("rld")])
    );
    assert_eq!(
        eval(r#""hello".rpartition("x")"#),
        Value::Tuple(vec![s(""), s(""), s("hello")])
    );
}

#[test]
fn accessors() {
    assert_eq!(eval(r#""hello".len()"#), Value::Number(Number::from(5)));
    assert_eq!(eval(r#""héllo".len()"#), Value::Number(Number::from(5)));
    assert_eq!(
        eval(r#""hello".char_at(1)"#),
        Value::Option(Some(Box::new(Value::Char('e'))))
    );
    assert_eq!(eval(r#""hello".char_at(99)"#), Value::Option(None));
    assert_eq!(eval_str(r#""hello".substring(1, 3)"#), "el");
    assert_eq!(eval_str(r#""ab".push("c")"#), "abc");
    assert_eq!(eval_str(r#""ab".repeat(3)"#), "ababab");
    assert_eq!(
        eval(r#""hi".insert(1, "o")"#),
        Value::Result(Ok(Box::new(Value::String("hoi".into()))))
    );
    assert!(matches!(
        eval(r#""hi".insert(9, "o")"#),
        Value::Result(Err(_))
    ));
}

#[test]
fn unicode_aware() {
    // Indices are Unicode scalar values, not bytes.
    assert_eq!(eval_str(r#""é".repeat(2)"#), "éé");
    assert_eq!(eval(r#""aé".len()"#), Value::Number(Number::from(2)));
    assert_eq!(
        eval(r#""aéa".find("é")"#),
        Value::Option(Some(Box::new(Value::Number(Number::from(1)))))
    );
    assert_eq!(eval_str(r#""aéa".replace("é", "b")"#), "aba");
    assert_eq!(eval_str(r#""héllo".to_upper()"#), "HÉLLO");
}
