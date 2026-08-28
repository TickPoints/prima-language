use prima_core::{Number, Value, ValueKey};
use prima_runtime::Evaluator;

/// Evaluate an in-memory program that imports the Rust-hosted `io` namespace (spec §18).
fn eval(src: &str) -> Value {
    prima_stdlib::init();
    Evaluator::new().eval_value(src).expect("eval failed")
}

/// Unwrap a `Value::Result(Ok(v))`; panic on `Err`/non-Result.
fn ok_of(v: Value) -> Value {
    match v {
        Value::Result(Ok(inner)) => *inner,
        other => panic!("expected Result<Ok>, got {other:?}"),
    }
}

#[test]
fn json_parse_object_with_array() {
    let v = ok_of(eval(
        r#"import io;
io::json_parse("{\"a\": 1, \"b\": [true, null]}")"#,
    ));
    match v {
        Value::Dict(d) => {
            assert_eq!(
                d.get(&ValueKey::Str("a".into())),
                Some(&Value::Number(Number::from(1)))
            );
            assert_eq!(
                d.get(&ValueKey::Str("b".into())),
                Some(&Value::Array(vec![Value::Bool(true), Value::Nil]))
            );
        }
        other => panic!("expected Dict, got {other:?}"),
    }
}

#[test]
fn json_parse_scalars() {
    assert_eq!(
        ok_of(eval(
            r#"import io;
io::json_parse("3.5")"#
        )),
        Value::Number(Number::from(3.5_f64))
    );
    assert_eq!(
        ok_of(eval(
            r#"import io;
io::json_parse("\"hi\"")"#
        )),
        Value::String("hi".into())
    );
    assert_eq!(
        ok_of(eval(
            r#"import io;
io::json_parse("[]")"#
        )),
        Value::Array(vec![])
    );
    assert_eq!(
        ok_of(eval(
            r#"import io;
io::json_parse("null")"#
        )),
        Value::Nil
    );
}

#[test]
fn json_stringify_roundtrips_dict() {
    // `?` unwraps the `Result` (spec §16.3); the stringify/parse round-trip preserves the dict value.
    let v = eval(
        r#"import io;
let d = io::json_parse(io::json_stringify({ "a": 1 })?)?;
d["a"]"#,
    );
    assert_eq!(v, Value::Number(Number::from(1)));
}

#[test]
fn json_parse_invalid_string_is_err() {
    assert!(matches!(
        eval(
            r#"import io;
io::json_parse("{not json}")"#
        ),
        Value::Result(Err(_))
    ));
}

#[test]
fn csv_parse_handles_quoted_commas() {
    let v = ok_of(eval(
        "import io;\nio::csv_parse(\"a,b\\n1,\\\"x, y\\\"\\n\")",
    ));
    assert_eq!(
        v,
        Value::Array(vec![
            Value::Array(vec![Value::String("a".into()), Value::String("b".into())]),
            Value::Array(vec![
                Value::String("1".into()),
                Value::String("x, y".into())
            ]),
        ])
    );
}

#[test]
fn csv_parse_handles_escaped_quotes_and_newlines() {
    let v = ok_of(eval(
        "import io;\nio::csv_parse(\"h,\\\"q\\\"\\\"q\\\"\\n1,\\\"line1\\nline2\\\"\\n\")",
    ));
    assert_eq!(
        v,
        Value::Array(vec![
            Value::Array(vec![
                Value::String("h".into()),
                Value::String("q\"q".into())
            ]),
            Value::Array(vec![
                Value::String("1".into()),
                Value::String("line1\nline2".into())
            ]),
        ])
    );
}

#[test]
fn csv_stringify_quotes_fields_with_specials() {
    let v = ok_of(eval(
        r#"import io;
io::csv_stringify([["a", "b"], ["1", "x, y"]])"#,
    ));
    match v {
        Value::String(s) => {
            assert!(s.contains("\"x, y\""), "csv output: {s:?}");
            assert!(s.starts_with("a,b\n"), "csv output: {s:?}");
        }
        other => panic!("expected String, got {other:?}"),
    }
}

#[test]
fn csv_parse_stringify_roundtrip() {
    let v = eval(
        r#"import io;
let rows = io::csv_parse(io::csv_stringify([["a", "b"], ["1", "x, y"]])?)?;
rows[1][1]"#,
    );
    assert_eq!(v, Value::String("x, y".into()));
}

#[test]
fn csv_parse_unterminated_quote_is_err() {
    assert!(matches!(
        eval("import io;\nio::csv_parse(\"a,\\\"unterminated\\n\")"),
        Value::Result(Err(_))
    ));
}

#[test]
fn write_read_file_roundtrip() {
    let dir = std::env::temp_dir();
    let path = dir.join(format!("prima_io_test_{}.txt", std::process::id()));
    let path_str = path.to_string_lossy().to_string();
    // `write_file` returns a `Result`; unwrap it to a plain success so the test asserts Ok.
    assert!(matches!(
        eval(&format!(
            "import io;\nio::write_file(\"{path_str}\", \"hello\")"
        )),
        Value::Result(Ok(_))
    ));
    assert_eq!(
        ok_of(eval(&format!("import io;\nio::read_file(\"{path_str}\")"))),
        Value::String("hello".into())
    );
    let _ = std::fs::remove_file(&path);
}

#[test]
fn exists_on_file_and_missing() {
    let dir = std::env::temp_dir();
    let path = dir.join(format!("prima_io_exists_{}.txt", std::process::id()));
    let path_str = path.to_string_lossy().to_string();
    std::fs::write(&path, "x").expect("write temp file");
    assert_eq!(
        eval(&format!("import io;\nio::exists(\"{path_str}\")")),
        Value::Bool(true)
    );
    assert_eq!(
        eval(
            r#"import io;
io::exists("/nonexistent/prima_xyz")"#
        ),
        Value::Bool(false)
    );
    let _ = std::fs::remove_file(&path);
}

#[test]
fn read_lines_splits_content() {
    let dir = std::env::temp_dir();
    let path = dir.join(format!("prima_io_lines_{}.txt", std::process::id()));
    let path_str = path.to_string_lossy().to_string();
    std::fs::write(&path, "a\nb\nc\n").expect("write temp file");
    assert_eq!(
        ok_of(eval(&format!("import io;\nio::read_lines(\"{path_str}\")"))),
        Value::Array(vec![
            Value::String("a".into()),
            Value::String("b".into()),
            Value::String("c".into()),
        ])
    );
    let _ = std::fs::remove_file(&path);
}

#[test]
fn read_file_nonexistent_is_err() {
    assert!(matches!(
        eval(
            r#"import io;
io::read_file("/nonexistent/prima_xyz")"#
        ),
        Value::Result(Err(_))
    ));
}

#[test]
fn read_write_json_roundtrip() {
    let dir = std::env::temp_dir();
    let path = dir.join(format!("prima_io_{}.json", std::process::id()));
    let path_str = path.to_string_lossy().to_string();
    assert!(matches!(
        eval(&format!(
            "import io;\nio::write_json(\"{path_str}\", {{ \"k\": [1, 2] }})"
        )),
        Value::Result(Ok(_))
    ));
    let v = ok_of(eval(&format!("import io;\nio::read_json(\"{path_str}\")")));
    match v {
        Value::Dict(d) => assert_eq!(
            d.get(&ValueKey::Str("k".into())),
            Some(&Value::Array(vec![
                Value::Number(Number::from(1)),
                Value::Number(Number::from(2)),
            ]))
        ),
        other => panic!("expected Dict, got {other:?}"),
    }
    let _ = std::fs::remove_file(&path);
}

#[test]
fn read_write_csv_roundtrip() {
    let dir = std::env::temp_dir();
    let path = dir.join(format!("prima_io_{}.csv", std::process::id()));
    let path_str = path.to_string_lossy().to_string();
    assert!(matches!(
        eval(&format!(
            "import io;\nio::write_csv(\"{path_str}\", [[\"a\", \"b\"], [\"1\", \"x, y\"]])"
        )),
        Value::Result(Ok(_))
    ));
    let v = ok_of(eval(&format!("import io;\nio::read_csv(\"{path_str}\")")));
    assert_eq!(
        v,
        Value::Array(vec![
            Value::Array(vec![Value::String("a".into()), Value::String("b".into())]),
            Value::Array(vec![
                Value::String("1".into()),
                Value::String("x, y".into())
            ]),
        ])
    );
    let _ = std::fs::remove_file(&path);
}
