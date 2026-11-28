//! `unwrap` family, `Option`/`Result` helpers, and string rendering (spec §4.4/§9.3/§18.1).

use prima_core::render::{render_latex, render_number};
use prima_core::{ExprPool, SymbolTable, Value, ValueKey};

use super::props::arity;
use crate::error::RuntimeError;

pub(super) fn unwrap(name: &str, args: &[Value]) -> Result<Value, RuntimeError> {
    arity(name, args, 1)?;
    match &args[0] {
        Value::Result(Ok(v)) | Value::Option(Some(v)) => Ok((**v).clone()),
        Value::Result(Err(msg)) => Err(RuntimeError::CustomError {
            message: (**msg).clone(),
        }),
        Value::Option(None) => Err(RuntimeError::Type(format!(
            "called `{name}` on a `None` value"
        ))),
        other => Err(RuntimeError::Type(format!(
            "`{name}` expects a `Result` or `Option` value, got {other:?}"
        ))),
    }
}

pub(super) fn unwrap_or(name: &str, args: &[Value]) -> Result<Value, RuntimeError> {
    arity(name, args, 2)?;
    match &args[0] {
        Value::Result(Ok(v)) | Value::Option(Some(v)) => Ok((**v).clone()),
        Value::Result(Err(_)) | Value::Option(None) => Ok(args[1].clone()),
        other => Err(RuntimeError::Type(format!(
            "`{name}` expects a `Result` or `Option` value, got {other:?}"
        ))),
    }
}

/// `expect` (spec §9.3): the failure message format is `<user message>: <underlying error>`.
pub(super) fn expect(name: &str, args: &[Value]) -> Result<Value, RuntimeError> {
    arity(name, args, 2)?;
    let msg = arg_to_string(&args[1]);
    match &args[0] {
        Value::Result(Ok(v)) | Value::Option(Some(v)) => Ok((**v).clone()),
        Value::Result(Err(err)) => Err(RuntimeError::CustomError {
            message: format!("{msg}: {err}"),
        }),
        Value::Option(None) => Err(RuntimeError::CustomError { message: msg }),
        other => Err(RuntimeError::Type(format!(
            "`{name}` expects a `Result` or `Option` value, got {other:?}"
        ))),
    }
}

/// `is_some` (spec §18.1): `true` for `Option::Some`.
pub(super) fn is_some(args: &[Value]) -> Result<Value, RuntimeError> {
    arity("is_some", args, 1)?;
    Ok(Value::Bool(matches!(&args[0], Value::Option(Some(_)))))
}

/// `is_none` (spec §18.1): `true` for `Option::None`.
pub(super) fn is_none(args: &[Value]) -> Result<Value, RuntimeError> {
    arity("is_none", args, 1)?;
    Ok(Value::Bool(matches!(&args[0], Value::Option(None))))
}

/// `is_ok` (spec §18.1): `true` for `Result::Ok`.
pub(super) fn is_ok(args: &[Value]) -> Result<Value, RuntimeError> {
    arity("is_ok", args, 1)?;
    Ok(Value::Bool(matches!(&args[0], Value::Result(Ok(_)))))
}

/// `is_err` (spec §18.1): `true` for `Result::Err`.
pub(super) fn is_err(args: &[Value]) -> Result<Value, RuntimeError> {
    arity("is_err", args, 1)?;
    Ok(Value::Bool(matches!(&args[0], Value::Result(Err(_)))))
}

/// `value_or` (spec §16.3/§18.1): the `Ok` value or `default` on `Err` (non-panicking `unwrap_or`).
pub(super) fn value_or(args: &[Value]) -> Result<Value, RuntimeError> {
    arity("value_or", args, 2)?;
    match &args[0] {
        Value::Result(Ok(v)) => Ok((**v).clone()),
        Value::Result(Err(_)) => Ok(args[1].clone()),
        other => Err(RuntimeError::Type(format!(
            "`value_or` expects a `Result` value, got {other:?}"
        ))),
    }
}

// ---- Option/Result constructors (spec §4.4) ----

pub(super) fn some(args: &[Value]) -> Result<Value, RuntimeError> {
    arity("Some", args, 1)?;
    Ok(Value::Option(Some(Box::new(args[0].clone()))))
}

pub(super) fn none(args: &[Value]) -> Result<Value, RuntimeError> {
    arity("None", args, 0)?;
    Ok(Value::Option(None))
}

pub(super) fn ok(args: &[Value]) -> Result<Value, RuntimeError> {
    arity("Ok", args, 1)?;
    Ok(Value::Result(Ok(Box::new(args[0].clone()))))
}

pub(super) fn err_builtin(args: &[Value]) -> Result<Value, RuntimeError> {
    arity("Err", args, 1)?;
    Ok(Value::Result(Err(Box::new(arg_to_string(&args[0])))))
}

// ---- String/format helpers (spec §18.1) ----
// `format` was removed in v2.2 (f-strings replace it, spec §18.1); `to_string`/`concat` remain.

pub(super) fn to_string_call(args: &[Value], pool: &ExprPool) -> Result<Value, RuntimeError> {
    arity("to_string", args, 1)?;
    Ok(Value::String(value_to_string(pool, &args[0]).into()))
}

pub(super) fn concat_call(args: &[Value], pool: &ExprPool) -> Result<Value, RuntimeError> {
    arity("concat", args, 2)?;
    let a = value_to_string(pool, &args[0]);
    let b = value_to_string(pool, &args[1]);
    Ok(Value::String((a + &b).into()))
}

/// Render any value to its display string (spec §16.1 `format_value`): mirrors the evaluator's rendering so
/// `to_string`/`concat` agree with `print`. `Expr` renders through the LaTeX view (spec §8.3).
pub(super) fn value_to_string(pool: &ExprPool, v: &Value) -> String {
    match v {
        Value::Nil => "nil".into(),
        Value::Number(n) => render_number(n),
        Value::Bool(b) => b.to_string(),
        Value::Char(c) => c.to_string(),
        Value::String(s) => s.to_string(),
        Value::Array(elems) => {
            let inner: Vec<String> =
                elems.with(|items| items.iter().map(|e| value_to_string(pool, e)).collect());
            format!("[{}]", inner.join(", "))
        }
        Value::Dict(d) => {
            let mut keys: Vec<ValueKey> = d.keys().cloned().collect();
            keys.sort_by_key(|a| value_to_string(pool, &a.to_value()));
            let inner: Vec<String> = keys
                .iter()
                .map(|k| {
                    format!(
                        "{}: {}",
                        value_to_string(pool, &k.to_value()),
                        value_to_string(pool, &d[k])
                    )
                })
                .collect();
            format!("{{{}}}", inner.join(", "))
        }
        Value::Set(s) => {
            let mut elems: Vec<Value> = s.iter().map(|k| k.to_value()).collect();
            elems.sort_by_key(|a| value_to_string(pool, a));
            let inner: Vec<String> = elems.iter().map(|e| value_to_string(pool, e)).collect();
            format!("{{{}}}", inner.join(", "))
        }
        Value::Expr(id) => render_latex(pool, SymbolTable::global(), *id),
        Value::Symbol(_) => "symbol".into(),
        Value::Indeterminate(_) => "indeterminate".into(),
        Value::Undefined => "undefined".into(),
        Value::Error(msg) => format!("error: {msg}"),
        Value::Class(id) => format!("class {id}"),
        Value::Tuple(items) => {
            let inner: Vec<String> = items.iter().map(|it| value_to_string(pool, it)).collect();
            format!("({})", inner.join(", "))
        }
        Value::Option(Some(v)) => value_to_string(pool, v),
        Value::Option(None) => "none".into(),
        Value::Result(Ok(v)) => value_to_string(pool, v),
        Value::Result(Err(msg)) => format!("err: {msg}"),
        Value::JitFunction(id) => format!("jit function {id}"),
    }
}

pub(super) fn arg_to_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.to_string(),
        other => format!("{other:?}"),
    }
}
