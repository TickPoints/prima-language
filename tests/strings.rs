// String tests (spec §18.1): `format`, unicode escapes, and the `String` method family.
use prima_core::{Number, Value};
use prima_runtime::Evaluator;

fn eval(src: &str) -> Value {
    Evaluator::new().eval_value(src).expect("eval failed")
}

fn eval_str(src: &str) -> String {
    match eval(src) {
        Value::String(s) => s,
        other => panic!("expected String, got {other:?}"),
    }
}

#[test]
fn format_interpolates_args() {
    assert_eq!(eval_str("format(\"a is {}\", 42)"), "a is 42");
    assert_eq!(eval_str("let name = \"world\";\nformat(\"hello {}\", name)"), "hello world");
}

#[test]
fn format_multiple_args() {
    assert_eq!(eval_str("format(\"{} + {} = {}\", 1, 2, 3)"), "1 + 2 = 3");
}

#[test]
fn unicode_escapes_in_string_literals() {
    assert_eq!(eval_str("\"smile: \\u{1F600}\""), "smile: 😀");
    assert_eq!(eval_str("\"\\u{3C0}\""), "π");
}

#[test]
fn standard_escapes() {
    assert_eq!(eval_str("\"a\\nb\\t\\\"q\\\"\""), "a\nb\t\"q\"");
}

#[test]
fn string_len_is_char_count() {
    assert_eq!(eval("let s = \"hello\";\ns.len()"), Value::Number(Number::from(5)));
    // `len` counts characters, not bytes.
    assert_eq!(eval("let s = \"héllo\";\ns.len()"), Value::Number(Number::from(5)));
}

#[test]
fn case_conversion() {
    assert_eq!(eval("let s = \"aXb\";\ns.to_lower()"), Value::String("axb".into()));
    assert_eq!(eval("let s = \"aXb\";\ns.to_upper()"), Value::String("AXB".into()));
}

#[test]
fn push_appends() {
    assert_eq!(eval("let s = \"ab\";\ns.push(\"c\")"), Value::String("abc".into()));
}

#[test]
fn split_yields_tuple_of_strings() {
    assert_eq!(
        eval("let s = \"a,b,c\";\ns.split(\",\")"),
        Value::Tuple(vec![
            Value::String("a".into()),
            Value::String("b".into()),
            Value::String("c".into()),
        ])
    );
}

#[test]
fn contains_reports_substring() {
    assert_eq!(eval("let s = \"hello world\";\ns.contains(\"world\")"), Value::Bool(true));
    assert_eq!(eval("let s = \"hello world\";\ns.contains(\"xyz\")"), Value::Bool(false));
}

#[test]
fn trim_replace_repeat() {
    assert_eq!(eval("let s = \"  hi  \";\ns.trim()"), Value::String("hi".into()));
    assert_eq!(eval("let s = \"a-b\";\ns.replace(\"-\", \"+\")"), Value::String("a+b".into()));
    assert_eq!(eval("let s = \"ab\";\ns.repeat(3)"), Value::String("ababab".into()));
}

#[test]
fn insert_is_result_checked() {
    assert_eq!(
        eval("let s = \"hi\";\ns.insert(1, \"o\")"),
        Value::Result(Ok(Box::new(Value::String("hoi".into()))))
    );
    assert!(matches!(eval("let s = \"hi\";\ns.insert(9, \"o\")"), Value::Result(Err(_))));
}

#[test]
fn string_associated_new() {
    assert_eq!(eval("String::new()"), Value::String(String::new()));
}

#[test]
fn fixed_width_collapse_round_trips() {
    assert_eq!(eval("to_u8(255)"), Value::Number(Number::U8(255)));
    assert_eq!(eval("to_usize(42)"), Value::Number(Number::Usize(42)));
    assert_eq!(eval("to_i128(-7)"), Value::Number(Number::I128(-7)));
    assert_eq!(eval("to_i32(to_u8(255))"), Value::Number(Number::I32(255)));
    assert!(Evaluator::new().eval_value("to_u8(256)").is_err(), "u8 overflow must error");
}
