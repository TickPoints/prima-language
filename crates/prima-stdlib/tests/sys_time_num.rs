use prima_core::{Number, Value};
use prima_runtime::Evaluator;

/// Evaluate an in-memory program that imports Rust-hosted stdlib namespaces (spec §18).
fn eval(src: &str) -> Value {
    prima_stdlib::init();
    Evaluator::new().eval_value(src).expect("eval failed")
}

#[test]
fn sys_path_join_uses_separator() {
    let v = eval("import sys::path;\nsys::path::join(\"a\", \"b\")");
    match v {
        Value::String(s) => assert!(s.contains(std::path::MAIN_SEPARATOR), "join output: {s:?}"),
        other => panic!("expected String, got {other:?}"),
    }
}

#[test]
fn sys_path_file_name_and_parent() {
    let v = eval("import sys::path;\nsys::path::file_name(\"a/b/c.txt\")");
    assert_eq!(
        v,
        Value::Option(Some(Box::new(Value::String("c.txt".into()))))
    );
    // `a/..` terminates in a parent traversal: no file name (spec §18.2).
    let v = eval("import sys::path;\nsys::path::file_name(\"a/..\")");
    assert_eq!(v, Value::Option(None));
    let v = eval("import sys::path;\nsys::path::parent(\"a/b\")");
    assert_eq!(v, Value::Option(Some(Box::new(Value::String("a".into())))));
}

#[test]
fn sys_os_name_is_non_empty() {
    let v = eval("import sys::os;\nsys::os::name()");
    match v {
        Value::String(s) => assert!(!s.is_empty()),
        other => panic!("expected String, got {other:?}"),
    }
}

#[test]
fn sys_env_get_returns_option() {
    // The variable is very likely unset, so `None` is the deterministic assertion.
    let v = eval("import sys::env;\nsys::env::get(\"PRIMA_STDLIB_TEST_UNSET_XXX\")");
    assert_eq!(v, Value::Option(None));
}

#[test]
fn time_unix_timestamp_of_now_is_positive() {
    let v = eval("import time;\ntime::unix_timestamp(time::now())");
    match v {
        Value::Number(n) => assert!(n.as_i64().is_some_and(|s| s > 0), "timestamp: {n}"),
        other => panic!("expected Number, got {other:?}"),
    }
}

#[test]
fn time_format_epoch_year() {
    assert_eq!(
        eval("import time;\ntime::format(0, \"%Y\")"),
        Value::String("1970".into())
    );
}

#[test]
fn time_format_and_parse_roundtrip() {
    let v = eval(
        r#"import time;
time::parse("2024-01-15", "%Y-%m-%d")"#,
    );
    match v {
        Value::Result(Ok(n)) => {
            let secs = match *n {
                Value::Number(Number::I64(s)) => s,
                other => panic!("expected I64 seconds, got {other:?}"),
            };
            let v = eval(&format!("import time;\ntime::format({secs}, \"%Y-%m-%d\")"));
            assert_eq!(v, Value::String("2024-01-15".into()));
        }
        other => panic!("expected Result<Ok>, got {other:?}"),
    }
}

#[test]
fn num_gcd_lcm() {
    assert_eq!(
        eval("import num;\nnum::gcd(12, 18)"),
        Value::Number(Number::from(6))
    );
    assert_eq!(
        eval("import num;\nnum::gcd(0, 5)"),
        Value::Number(Number::from(5))
    );
    assert_eq!(
        eval("import num;\nnum::lcm(4, 6)"),
        Value::Number(Number::from(12))
    );
}

#[test]
fn num_primality() {
    assert_eq!(eval("import num;\nnum::is_prime(7)"), Value::Bool(true));
    assert_eq!(eval("import num;\nnum::is_prime(1)"), Value::Bool(false));
    assert_eq!(eval("import num;\nnum::is_prime(100)"), Value::Bool(false));
}

#[test]
fn num_next_prime() {
    assert_eq!(
        eval("import num;\nnum::next_prime(10)"),
        Value::Number(Number::from(11))
    );
    assert_eq!(
        eval("import num;\nnum::next_prime(13)"),
        Value::Number(Number::from(13))
    );
}

#[test]
fn num_base_conversion() {
    assert_eq!(
        eval("import num;\nnum::to_base(255, 16)"),
        Value::String("ff".into())
    );
    assert_eq!(
        eval("import num;\nnum::to_base(8, 2)"),
        Value::String("1000".into())
    );
    match eval("import num;\nnum::from_base(\"ff\", 16)") {
        Value::Result(Ok(n)) => assert_eq!(*n, Value::Number(Number::from(255))),
        other => panic!("expected Result<Ok>, got {other:?}"),
    }
    match eval("import num;\nnum::from_base(\"2\", 2)") {
        Value::Result(Err(_)) => {}
        other => panic!("expected Result<Err>, got {other:?}"),
    }
}

#[test]
fn num_random_integer_in_range() {
    let v = eval("import num;\nnum::random_integer(3, 7)");
    match v {
        Value::Number(n) => {
            let r = n.as_i64().expect("random_integer must be integral");
            assert!((3..=7).contains(&r), "random value out of range: {r}");
        }
        other => panic!("expected Number, got {other:?}"),
    }
}

#[test]
fn duration_from_secs_is_seconds() {
    let v = eval("import time;\ntime::Duration::from_secs(5)");
    assert_eq!(v, Value::Number(Number::from(5)));
}
