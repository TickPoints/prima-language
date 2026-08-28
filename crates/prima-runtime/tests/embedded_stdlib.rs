//! Integration tests for embedded stdlib signature modules (spec §18.4): `@builtin pub fn`
//! declarations inside compiled-in `.pra` sources bind to Rust implementations registered under
//! fully-qualified keys, and resolve through module-qualified calls.
//!
//! The registries are process-global `OnceLock`s and the tests here are the only ones registering
//! the `testembed*` sources/impls, so unique names suffice; registration is idempotent anyway.

use prima_core::{Number, Value};
use prima_runtime::stdlib::{get_impl, register_impl, register_module_source};
use prima_runtime::{Evaluator, RuntimeError};

/// Extract the single integer argument of a test `@builtin` impl.
fn int_arg(args: &[Value], fname: &str) -> Result<i64, RuntimeError> {
    match args.first() {
        Some(Value::Number(n)) => n.as_i64().ok_or_else(|| {
            RuntimeError::Message(format!("`{fname}` expects an integer argument, got {n:?}"))
        }),
        other => Err(RuntimeError::Message(format!(
            "`{fname}` expects an integer argument, got {other:?}"
        ))),
    }
}

fn eval(src: &str) -> Value {
    Evaluator::new()
        .eval_value(src)
        .unwrap_or_else(|e| panic!("eval failed: {e}"))
}

#[test]
fn embedded_builtin_module_qualified_call() {
    register_module_source(
        "testembed",
        "@builtin pub fn triple(x: Integer) -> Integer;",
    );
    register_impl("testembed::triple", |_ev, args| {
        let x = int_arg(args, "testembed::triple")?;
        Ok(Value::Number(Number::from(3 * x)))
    });

    assert_eq!(
        eval("import testembed;\ntestembed::triple(4)"),
        Value::Number(Number::from(12))
    );
    assert!(get_impl("testembed::triple").is_some());
}

#[test]
fn embedded_builtin_from_import() {
    register_module_source(
        "testembed",
        "@builtin pub fn triple(x: Integer) -> Integer;",
    );
    register_impl("testembed::triple", |_ev, args| {
        let x = int_arg(args, "testembed::triple")?;
        Ok(Value::Number(Number::from(3 * x)))
    });

    assert_eq!(
        eval("import testembed;\nfrom testembed import triple;\ntriple(2)"),
        Value::Number(Number::from(6))
    );
}

#[test]
fn embedded_builtin_path_name() {
    // A `::`-joined `@builtin` name is exported under the joined key, so `testembed2::Util::twice`
    // resolves via the flattened module-item lookup (spec §18.4 / §18.3).
    register_module_source(
        "testembed2",
        "@builtin pub fn Util::twice(x: Integer) -> Integer;",
    );
    register_impl("testembed2::Util::twice", |_ev, args| {
        let x = int_arg(args, "testembed2::Util::twice")?;
        Ok(Value::Number(Number::from(2 * x)))
    });

    assert_eq!(
        eval("import testembed2;\ntestembed2::Util::twice(5)"),
        Value::Number(Number::from(10))
    );
}

#[test]
fn unregistered_embedded_builtin_errors_e0055() {
    // Source declares an `@builtin` with no matching impl: evaluating the import must fail with E0055.
    register_module_source("testembed3", "@builtin pub fn nope(x: Integer) -> Integer;");

    let err = Evaluator::new()
        .eval_value("import testembed3;")
        .unwrap_err();
    let msg = format!("{err}");
    assert!(msg.contains("E0055"), "expected E0055, got: {msg}");
    assert!(
        msg.contains("nope"),
        "expected the builtin name in the error, got: {msg}"
    );
}
