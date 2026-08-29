// String tests (spec §18.1): f-strings, raw/single-quoted literals, unicode escapes, and the `String` method family.
use prima_core::{Number, Value};
use prima_runtime::Evaluator;

fn eval(src: &str) -> Value {
    prima_stdlib::init();
    Evaluator::new().eval_value(src).expect("eval failed")
}

fn eval_str(src: &str) -> String {
    match eval(src) {
        Value::String(s) => s,
        other => panic!("expected String, got {other:?}"),
    }
}

#[test]
fn fstring_interpolates_expressions() {
    assert_eq!(eval_str("let a = 42;\nf\"a is {a}\""), "a is 42");
    assert_eq!(
        eval_str("let name = \"world\";\nf\"hello {name}\""),
        "hello world"
    );
    assert_eq!(
        eval_str("let x = 1;\nlet y = 2;\nf\"{x} + {y} = {x + y}\""),
        "1 + 2 = 3"
    );
}

#[test]
fn fstring_escaped_braces() {
    assert_eq!(eval_str("f\"{{literal braces}}\""), "{literal braces}");
    assert_eq!(eval_str("f\"{1} and {{2}}\""), "1 and {2}");
}

#[test]
fn fstring_float_precision_spec() {
    assert_eq!(eval_str(r#"f"{to_f64(pi):0.2}""#), "3.14");
    assert_eq!(eval_str("f\"{3.14159:.3}\""), "3.142");
}

#[test]
fn fstring_width_and_alignment() {
    assert_eq!(eval_str(r#"f"|{5:>5}|""#), "|    5|");
    assert_eq!(eval_str(r#"f"|{5:<5}|""#), "|5    |");
    assert_eq!(eval_str(r#"f"|{5:^5}|""#), "|  5  |");
    assert_eq!(eval_str(r#"f"|{5:05}|""#), "|00005|");
    // Numbers right-align by default (Python `format` semantics); strings left-align.
    assert_eq!(eval_str(r#"f"|{5:5}|""#), "|    5|");
    assert_eq!(eval_str(r#"f"|{"a":5}|""#), "|a    |");
}

#[test]
fn fstring_renders_values() {
    assert_eq!(eval_str(r#"f"x = {true}""#), "x = true");
    assert_eq!(eval_str("f\"s = {\"a,b\"}\""), "s = a,b");
    assert_eq!(eval_str(r#"f"arr = {[1, 2, 3]}""#), "arr = [1, 2, 3]");
}

#[test]
fn fstring_interpolation_can_be_any_expression() {
    assert_eq!(eval_str("f\"{sqrt(9)}\""), "3");
    assert_eq!(eval_str(r#"f"{ "a".to_upper() }""#), "A");
}

#[test]
fn single_quote_strings_are_equivalent() {
    assert_eq!(eval_str("'hello'"), "hello");
    assert_eq!(
        eval("let s = 'ab';\ns.len()"),
        Value::Number(Number::from(2))
    );
    // A single character is a `Char` (spec appendix A `char`).
    assert_eq!(eval("'a'"), Value::Char('a'));
}

#[test]
fn raw_strings_do_not_escape() {
    assert_eq!(eval_str(r#"r"a\nb""#), "a\\nb");
    assert_eq!(eval_str(r"r'\t'"), "\\t");
}

#[test]
fn raw_fstring_keeps_literals_raw_but_interpolates() {
    let v = eval(
        r#"let x = 5;
rf"a\nb{x}""#,
    );
    assert_eq!(v, Value::String("a\\nb5".into()));
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
    assert_eq!(
        eval("let s = \"hello\";\ns.len()"),
        Value::Number(Number::from(5))
    );
    // `len` counts characters, not bytes.
    assert_eq!(
        eval("let s = \"héllo\";\ns.len()"),
        Value::Number(Number::from(5))
    );
}

#[test]
fn case_conversion() {
    assert_eq!(
        eval("let s = \"aXb\";\ns.to_lower()"),
        Value::String("axb".into())
    );
    assert_eq!(
        eval("let s = \"aXb\";\ns.to_upper()"),
        Value::String("AXB".into())
    );
}

#[test]
fn push_appends() {
    assert_eq!(
        eval("let s = \"ab\";\ns.push(\"c\")"),
        Value::String("abc".into())
    );
}

#[test]
fn split_yields_array_of_strings() {
    // v2.1 (spec §18.1): `String.split` returns `Array<String>`.
    assert_eq!(
        eval("let s = \"a,b,c\";\ns.split(\",\")"),
        Value::Array(vec![
            Value::String("a".into()),
            Value::String("b".into()),
            Value::String("c".into()),
        ])
    );
}

#[test]
fn contains_reports_substring() {
    assert_eq!(
        eval("let s = \"hello world\";\ns.contains(\"world\")"),
        Value::Bool(true)
    );
    assert_eq!(
        eval("let s = \"hello world\";\ns.contains(\"xyz\")"),
        Value::Bool(false)
    );
}

#[test]
fn trim_replace_repeat() {
    assert_eq!(
        eval("let s = \"  hi  \";\ns.trim()"),
        Value::String("hi".into())
    );
    assert_eq!(
        eval("let s = \"a-b\";\ns.replace(\"-\", \"+\")"),
        Value::String("a+b".into())
    );
    assert_eq!(
        eval("let s = \"ab\";\ns.repeat(3)"),
        Value::String("ababab".into())
    );
}

#[test]
fn insert_is_result_checked() {
    assert_eq!(
        eval("let s = \"hi\";\ns.insert(1, \"o\")"),
        Value::Result(Ok(Box::new(Value::String("hoi".into()))))
    );
    assert!(matches!(
        eval("let s = \"hi\";\ns.insert(9, \"o\")"),
        Value::Result(Err(_))
    ));
}

#[test]
fn string_associated_new() {
    assert_eq!(eval("String::new()"), Value::String(String::new()));
}

#[test]
fn format_call_is_removed() {
    // `format` was removed in v2.2 (spec §18.1): it is no longer a builtin.
    let err = Evaluator::new()
        .eval_value("format(\"a is {}\", 42)")
        .unwrap_err();
    assert!(
        err.to_string().contains("unknown function `format`"),
        "error = {err}"
    );
}

#[test]
fn fixed_width_collapse_round_trips() {
    assert_eq!(eval("to_u8(255)"), Value::Number(Number::U8(255)));
    assert_eq!(eval("to_usize(42)"), Value::Number(Number::Usize(42)));
    assert_eq!(eval("to_i128(-7)"), Value::Number(Number::I128(-7)));
    assert_eq!(eval("to_i32(to_u8(255))"), Value::Number(Number::I32(255)));
    assert!(
        Evaluator::new().eval_value("to_u8(256)").is_err(),
        "u8 overflow must error"
    );
}

#[test]
fn string_method_basics() {
    // Moved from the runtime crate's unit tests: the `String` class lives in the standard library.
    assert_eq!(
        eval("let s = \"hello\";\ns.len()"),
        Value::Number(Number::from(5))
    );
    assert_eq!(
        eval("let s = \"aXb\";\ns.to_lower()"),
        Value::String("axb".into())
    );
    assert_eq!(
        eval("let s = \"ab\";\ns.push(\"c\")"),
        Value::String("abc".into())
    );
    assert_eq!(
        eval("let s = \"hello world\";\ns.contains(\"world\")"),
        Value::Bool(true)
    );
    assert_eq!(
        eval("let s = \"a,b,c\";\ns.split(\",\")"),
        Value::Array(vec![
            Value::String("a".into()),
            Value::String("b".into()),
            Value::String("c".into()),
        ])
    );
    assert_eq!(
        eval("let s = \"hi\";\ns.insert(1, \"o\")"),
        Value::Result(Ok(Box::new(Value::String("hoi".into()))))
    );
    assert!(matches!(
        eval("let s = \"hi\";\ns.insert(9, \"o\")"),
        Value::Result(Err(_))
    ));
    assert_eq!(eval("String::new()"), Value::String(String::new()));
}

#[test]
fn unknown_string_method_attaches_registry_note_and_help() {
    // Moved from the runtime crate's unit tests: the doc registry is seeded from `core/string.pra`.
    prima_stdlib::init();
    let err = Evaluator::new()
        .eval_value("let s = \"hi\";\ns.toupper()")
        .expect_err("expected an unknown-method error");
    assert!(
        err.to_string()
            .contains("unknown `String` method `toupper`"),
        "unexpected error: {err}"
    );
    assert_eq!(err.help().as_deref(), Some("did you mean `to_upper`?"));
    let notes = err.notes();
    // The note points at the suggested method's definition (its doc, spec §16.4).
    assert!(
        notes.iter().any(|n| n.contains("core/string.pra")),
        "notes: {notes:?}"
    );
    assert!(
        notes.iter().any(|n| n.contains("uppercase")),
        "notes: {notes:?}"
    );
}
