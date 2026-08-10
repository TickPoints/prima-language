//! Collapse function family (spec §9.2–9.6): `to_/try_/checked_/clamped_/rounded_/truncated_` plus the `unwrap` family.
//!
//! The naming scheme encodes safety properties (spec §9.1): the basic forms raise a runtime error on failure (§9.2),
//! the try forms return a `Result` (§9.3), the checked forms check for overflow (§9.4), the clamped forms force a range (§9.5), and the rounding forms round in the specified way (§9.6).

use num_bigint::BigInt;
use prima_core::collapse::collapse_value;
use prima_core::{BuiltinSymbols, ExprPool, Number, Real, Value};

use crate::error::RuntimeError;

/// Invoke a collapse builtin function (spec §9.2–9.6).
///
/// An unknown name or wrong number of arguments returns `RuntimeError::Message`; numeric arguments are first collapsed
/// via `collapse_value` (spec §9); a value that cannot be collapsed reports `RuntimeError::Collapse` (spec §9.8: only collapse failure errors).
pub fn call(
    name: &str,
    args: &[Value],
    pool: &ExprPool,
    builtins: &BuiltinSymbols,
) -> Result<Value, RuntimeError> {
    match name {
        "to_i32" => { arity(name, args, 1)?; to_i32(name, &collapse_arg(pool, builtins, &args[0])?) }
        "to_i64" => { arity(name, args, 1)?; to_i64(name, &collapse_arg(pool, builtins, &args[0])?) }
        "to_f32" => { arity(name, args, 1)?; to_f32(name, &collapse_arg(pool, builtins, &args[0])?) }
        "to_f64" => { arity(name, args, 1)?; to_f64(name, &collapse_arg(pool, builtins, &args[0])?) }
        "to_bigint" => { arity(name, args, 1)?; to_bigint(name, &collapse_arg(pool, builtins, &args[0])?) }
        "to_rational" => { arity(name, args, 1)?; to_rational(name, &collapse_arg(pool, builtins, &args[0])?) }
        "to_bigfloat" => { arity(name, args, 1)?; to_bigfloat(&collapse_arg(pool, builtins, &args[0])?) }
        "to_complex" => { arity(name, args, 1)?; to_complex(&collapse_arg(pool, builtins, &args[0])?) }
        "try_i32" => { arity(name, args, 1)?; Ok(try_i32(name, collapse_value(pool, builtins, &args[0]))) }
        "try_i64" => { arity(name, args, 1)?; Ok(try_i64(name, collapse_value(pool, builtins, &args[0]))) }
        "try_f64" => { arity(name, args, 1)?; Ok(try_f64(name, collapse_value(pool, builtins, &args[0]))) }
        "try_bigint" => { arity(name, args, 1)?; Ok(try_bigint(name, collapse_value(pool, builtins, &args[0]))) }
        "try_rational" => { arity(name, args, 1)?; Ok(try_rational(name, collapse_value(pool, builtins, &args[0]))) }
        "try_complex" => { arity(name, args, 1)?; Ok(try_complex(name, collapse_value(pool, builtins, &args[0]))) }
        "checked_i32" => { arity(name, args, 1)?; checked_i32(name, &collapse_arg(pool, builtins, &args[0])?) }
        "checked_u64" => { arity(name, args, 1)?; checked_u64(name, &collapse_arg(pool, builtins, &args[0])?) }
        "checked_add" => {
            arity(name, args, 2)?;
            let a = collapse_arg(pool, builtins, &args[0])?;
            let b = collapse_arg(pool, builtins, &args[1])?;
            checked_binary(name, &a, &b, |x, y| x + y)
        }
        "checked_mul" => {
            arity(name, args, 2)?;
            let a = collapse_arg(pool, builtins, &args[0])?;
            let b = collapse_arg(pool, builtins, &args[1])?;
            checked_binary(name, &a, &b, |x, y| x * y)
        }
        "clamped_i32" => {
            arity(name, args, 3)?;
            let x = collapse_arg(pool, builtins, &args[0])?;
            let min = collapse_arg(pool, builtins, &args[1])?;
            let max = collapse_arg(pool, builtins, &args[2])?;
            clamped_i32(name, &x, &min, &max)
        }
        "clamped_u64" => { arity(name, args, 1)?; clamped_u64(name, &collapse_arg(pool, builtins, &args[0])?) }
        "clamped_f64" => {
            arity(name, args, 3)?;
            let x = collapse_arg(pool, builtins, &args[0])?;
            let min = collapse_arg(pool, builtins, &args[1])?;
            let max = collapse_arg(pool, builtins, &args[2])?;
            clamped_f64(name, &x, &min, &max)
        }
        "rounded_f64" => {
            arity(name, args, 2)?;
            let x = collapse_arg(pool, builtins, &args[0])?;
            let digits = collapse_arg(pool, builtins, &args[1])?;
            rounded_f64(name, &x, &digits)
        }
        "rounded_i32" => { arity(name, args, 1)?; rounded_i32(name, &collapse_arg(pool, builtins, &args[0])?) }
        "truncated_i32" => { arity(name, args, 1)?; truncated_i32(name, &collapse_arg(pool, builtins, &args[0])?) }
        "unwrap" => unwrap(name, args),
        "unwrap_or" => unwrap_or(name, args),
        "expect" => expect(name, args),
        _ => Err(RuntimeError::Message(format!("unknown collapse function `{name}`"))),
    }
}

fn arity(name: &str, args: &[Value], n: usize) -> Result<(), RuntimeError> {
    if args.len() == n {
        Ok(())
    } else {
        Err(RuntimeError::Message(format!("`{name}` expects {n} argument(s), got {}", args.len())))
    }
}

fn collapse_arg(pool: &ExprPool, builtins: &BuiltinSymbols, v: &Value) -> Result<Number, RuntimeError> {
    collapse_value(pool, builtins, v)
        .ok_or_else(|| RuntimeError::Collapse(format!("cannot collapse {v:?} to a number")))
}

/// A complex number cannot collapse to the real numeric domain (spec §9.2 basic collapse operates on reals).
fn ensure_real(name: &str, n: &Number) -> Result<(), RuntimeError> {
    if n.is_complex() {
        Err(RuntimeError::Domain(format!("`{name}` does not accept complex values, got {n}")))
    } else {
        Ok(())
    }
}

// ---- Basic collapse (spec §9.2): failure is a runtime error ----

fn to_i32(name: &str, n: &Number) -> Result<Value, RuntimeError> {
    ensure_real(name, n)?;
    match n.as_i32() {
        Some(v) => Ok(Value::Number(Number::from(v))),
        None => Err(RuntimeError::Overflow(format!("`{name}`: {n} cannot be represented as i32"))),
    }
}

fn to_i64(name: &str, n: &Number) -> Result<Value, RuntimeError> {
    ensure_real(name, n)?;
    match n.as_i64() {
        Some(v) => Ok(Value::Number(Number::from(v))),
        None => Err(RuntimeError::Overflow(format!("`{name}`: {n} cannot be represented as i64"))),
    }
}

fn to_f32(name: &str, n: &Number) -> Result<Value, RuntimeError> {
    ensure_real(name, n)?;
    Ok(Value::Number(Number::Real(Real::F32(n.to_f64_lossy() as f32))))
}

fn to_f64(name: &str, n: &Number) -> Result<Value, RuntimeError> {
    ensure_real(name, n)?;
    Ok(Value::Number(Number::Real(Real::F64(n.to_f64_lossy()))))
}

fn to_bigint(name: &str, n: &Number) -> Result<Value, RuntimeError> {
    match n.as_bigint() {
        Some(b) => Ok(Value::Number(Number::Integer(b))),
        None => Err(RuntimeError::Collapse(format!("`{name}`: {n} is not an integer"))),
    }
}

fn to_rational(name: &str, n: &Number) -> Result<Value, RuntimeError> {
    match n.as_rational() {
        Some(r) => Ok(Value::Number(Number::Rational(r))),
        None => Err(RuntimeError::Collapse(format!("`{name}`: {n} cannot be represented as a rational"))),
    }
}

/// `to_bigfloat` (spec §9.2) is a degenerate implementation that preserves precision: the number is returned unchanged (exact values are kept, `Real` stays f64).
/// Arbitrary-precision floats (BigFloat) are deferred to a later phase.
fn to_bigfloat(n: &Number) -> Result<Value, RuntimeError> {
    Ok(Value::Number(n.clone()))
}

fn to_complex(n: &Number) -> Result<Value, RuntimeError> {
    if n.is_complex() {
        return Ok(Value::Number(n.clone()));
    }
    Ok(Value::Number(Number::Complex {
        re: Box::new(n.clone()),
        im: Box::new(Number::from(0)),
    }))
}

// ---- Try collapse (spec §9.3): result wrapped in a `Result`, no runtime error is raised ----

fn try_i32(name: &str, n: Option<Number>) -> Value {
    let Some(n) = n else {
        return Value::Result(Err(format!("`{name}`: cannot collapse argument to a number")));
    };
    if n.is_complex() {
        return Value::Result(Err(format!("`{name}`: cannot convert complex value {n} to i32")));
    }
    match n.as_i32() {
        Some(v) => Value::Result(Ok(Box::new(Value::Number(Number::from(v))))),
        None => Value::Result(Err(format!("`{name}`: {n} cannot be represented as i32"))),
    }
}

fn try_i64(name: &str, n: Option<Number>) -> Value {
    let Some(n) = n else {
        return Value::Result(Err(format!("`{name}`: cannot collapse argument to a number")));
    };
    if n.is_complex() {
        return Value::Result(Err(format!("`{name}`: cannot convert complex value {n} to i64")));
    }
    match n.as_i64() {
        Some(v) => Value::Result(Ok(Box::new(Value::Number(Number::from(v))))),
        None => Value::Result(Err(format!("`{name}`: {n} cannot be represented as i64"))),
    }
}

fn try_f64(name: &str, n: Option<Number>) -> Value {
    let Some(n) = n else {
        return Value::Result(Err(format!("`{name}`: cannot collapse argument to a number")));
    };
    if n.is_complex() {
        return Value::Result(Err(format!("`{name}`: cannot convert complex value {n} to f64")));
    }
    Value::Result(Ok(Box::new(Value::Number(Number::Real(Real::F64(n.to_f64_lossy()))))))
}

fn try_bigint(name: &str, n: Option<Number>) -> Value {
    let Some(n) = n else {
        return Value::Result(Err(format!("`{name}`: cannot collapse argument to a number")));
    };
    match n.as_bigint() {
        Some(b) => Value::Result(Ok(Box::new(Value::Number(Number::Integer(b))))),
        None => Value::Result(Err(format!("`{name}`: {n} is not an integer"))),
    }
}

fn try_rational(name: &str, n: Option<Number>) -> Value {
    let Some(n) = n else {
        return Value::Result(Err(format!("`{name}`: cannot collapse argument to a number")));
    };
    match n.as_rational() {
        Some(r) => Value::Result(Ok(Box::new(Value::Number(Number::Rational(r))))),
        None => Value::Result(Err(format!("`{name}`: {n} cannot be represented as a rational"))),
    }
}

fn try_complex(name: &str, n: Option<Number>) -> Value {
    let Some(n) = n else {
        return Value::Result(Err(format!("`{name}`: cannot collapse argument to a number")));
    };
    if n.is_complex() {
        return Value::Result(Ok(Box::new(Value::Number(n))));
    }
    Value::Result(Ok(Box::new(Value::Number(Number::Complex {
        re: Box::new(n),
        im: Box::new(Number::from(0)),
    }))))
}

// ---- Checked collapse (spec §9.4): check for overflow, return a `Result` ----

fn checked_i32(name: &str, n: &Number) -> Result<Value, RuntimeError> {
    ensure_real(name, n)?;
    match n.as_i32() {
        Some(v) => Ok(Value::Result(Ok(Box::new(Value::Number(Number::from(v)))))),
        None => Ok(Value::Result(Err(format!("overflow: `{name}`: {n} cannot be represented as i32")))),
    }
}

fn checked_u64(name: &str, n: &Number) -> Result<Value, RuntimeError> {
    ensure_real(name, n)?;
    match n.as_u64() {
        Some(v) => Ok(Value::Result(Ok(Box::new(Value::Number(Number::Integer(BigInt::from(v))))))),
        None => Ok(Value::Result(Err(format!("overflow: `{name}`: {n} is not a non-negative integer in u64 range")))),
    }
}

/// `checked_add`/`checked_mul` (spec §9.4): evaluate via f64, then verify the result can be exactly represented back in i64, otherwise `Err`.
fn checked_binary(name: &str, a: &Number, b: &Number, op: fn(f64, f64) -> f64) -> Result<Value, RuntimeError> {
    ensure_real(name, a)?;
    ensure_real(name, b)?;
    let v = op(a.to_f64_lossy(), b.to_f64_lossy());
    let vi = v as i64;
    if v.is_nan() || v.is_infinite() || vi as f64 != v {
        return Ok(Value::Result(Err(format!("overflow: `{name}`: result {v} cannot be represented as i64"))));
    }
    Ok(Value::Result(Ok(Box::new(Value::Number(Number::from(vi))))))
}

// ---- Clamped collapse (spec §9.5): force into range ----

fn clamped_i32(name: &str, x: &Number, min: &Number, max: &Number) -> Result<Value, RuntimeError> {
    ensure_real(name, x)?;
    ensure_real(name, min)?;
    ensure_real(name, max)?;
    let c = x.to_f64_lossy().clamp(min.to_f64_lossy(), max.to_f64_lossy());
    Ok(Value::Number(Number::from(c as i64)))
}

/// `clamped_u64` (spec §9.5): single argument, clamped to `[0, u64::MAX]` (negative → 0, out of range → `u64::MAX`).
fn clamped_u64(name: &str, n: &Number) -> Result<Value, RuntimeError> {
    ensure_real(name, n)?;
    let c = n.to_f64_lossy().clamp(0.0, u64::MAX as f64);
    Ok(Value::Number(Number::Integer(BigInt::from(c as u64))))
}

fn clamped_f64(name: &str, x: &Number, min: &Number, max: &Number) -> Result<Value, RuntimeError> {
    ensure_real(name, x)?;
    ensure_real(name, min)?;
    ensure_real(name, max)?;
    Ok(Value::Number(x.clamped_f64(min.to_f64_lossy(), max.to_f64_lossy())))
}

// ---- Rounding collapse (spec §9.6) ----

fn rounded_f64(name: &str, x: &Number, digits: &Number) -> Result<Value, RuntimeError> {
    ensure_real(name, x)?;
    let d = digits
        .as_i64()
        .ok_or_else(|| RuntimeError::Type(format!("`{name}` expects an integer digit count, got {digits}")))?;
    Ok(Value::Number(x.rounded_digits(d)))
}

fn rounded_i32(name: &str, n: &Number) -> Result<Value, RuntimeError> {
    ensure_real(name, n)?;
    Ok(Value::Number(n.round()))
}

fn truncated_i32(name: &str, n: &Number) -> Result<Value, RuntimeError> {
    ensure_real(name, n)?;
    Ok(Value::Number(n.truncate()))
}

// ---- `unwrap` family: unwrap the `Result` of safe collapse (spec §9.3) ----

fn unwrap(name: &str, args: &[Value]) -> Result<Value, RuntimeError> {
    arity(name, args, 1)?;
    match &args[0] {
        Value::Result(Ok(v)) => Ok((**v).clone()),
        Value::Result(Err(msg)) => Err(RuntimeError::Message(msg.clone())),
        other => Err(RuntimeError::Type(format!("`{name}` expects a `Result` value, got {other:?}"))),
    }
}

fn unwrap_or(name: &str, args: &[Value]) -> Result<Value, RuntimeError> {
    arity(name, args, 2)?;
    match &args[0] {
        Value::Result(Ok(v)) => Ok((**v).clone()),
        Value::Result(Err(_)) => Ok(args[1].clone()),
        other => Err(RuntimeError::Type(format!("`{name}` expects a `Result` value, got {other:?}"))),
    }
}

/// `expect` (spec §9.3): the failure message format is `<user message>: <underlying error>`.
fn expect(name: &str, args: &[Value]) -> Result<Value, RuntimeError> {
    arity(name, args, 2)?;
    let msg = arg_to_string(&args[1]);
    match &args[0] {
        Value::Result(Ok(v)) => Ok((**v).clone()),
        Value::Result(Err(err)) => Err(RuntimeError::Message(format!("{msg}: {err}"))),
        other => Err(RuntimeError::Type(format!("`{name}` expects a `Result` value, got {other:?}"))),
    }
}

fn arg_to_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => format!("{other:?}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> (ExprPool, &'static BuiltinSymbols) {
        (ExprPool::new(), BuiltinSymbols::global())
    }

    #[test]
    fn to_f64_of_symbolic_sqrt2() {
        let (pool, builtins) = setup();
        let f = pool.symbol(builtins.sqrt);
        let app = pool.apply(f, &[pool.integer(2)]);
        let v = call("to_f64", &[Value::Expr(app)], &pool, builtins).unwrap();
        match v {
            Value::Number(Number::Real(Real::F64(x))) => assert!((x - std::f64::consts::SQRT_2).abs() < 1e-9),
            other => panic!("expected F64, got {other:?}"),
        }
    }

    #[test]
    fn to_i32_overflow_errors() {
        let (pool, builtins) = setup();
        let big = Number::Integer(BigInt::from(2_147_483_648i64));
        let err = call("to_i32", &[Value::Number(big)], &pool, builtins).unwrap_err();
        assert!(matches!(err, RuntimeError::Overflow(_)));
    }

    #[test]
    fn to_f64_rejects_complex() {
        let (pool, builtins) = setup();
        let c = Number::complex(3, 4);
        let err = call("to_f64", &[Value::Number(c)], &pool, builtins).unwrap_err();
        assert!(matches!(err, RuntimeError::Domain(_)));
    }

    #[test]
    fn try_i32_result_conversions() {
        let (pool, builtins) = setup();
        let in_range = call("try_i32", &[Value::Number(Number::from(42))], &pool, builtins).unwrap();
        assert_eq!(in_range, Value::Result(Ok(Box::new(Value::Number(Number::from(42))))));

        let out = call(
            "try_i32",
            &[Value::Number(Number::Integer(BigInt::from(2_147_483_648i64)))],
            &pool,
            builtins,
        )
        .unwrap();
        assert!(matches!(out, Value::Result(Err(_))), "expected Err result, got {out:?}");

        let nonnum = call("try_i32", &[Value::Bool(true)], &pool, builtins).unwrap();
        assert!(matches!(nonnum, Value::Result(Err(_))), "expected Err result, got {nonnum:?}");
    }

    #[test]
    fn checked_i32_and_checked_arithmetic() {
        let (pool, builtins) = setup();
        let ok = call("checked_i32", &[Value::Number(Number::from(5))], &pool, builtins).unwrap();
        assert_eq!(ok, Value::Result(Ok(Box::new(Value::Number(Number::from(5))))));

        let out = call(
            "checked_i32",
            &[Value::Number(Number::Integer(BigInt::from(2_147_483_648i64)))],
            &pool,
            builtins,
        )
        .unwrap();
        assert!(matches!(out, Value::Result(Err(_))), "expected Err result, got {out:?}");

        let sum = call(
            "checked_add",
            &[Value::Number(Number::from(5)), Value::Number(Number::from(7))],
            &pool,
            builtins,
        )
        .unwrap();
        assert_eq!(sum, Value::Result(Ok(Box::new(Value::Number(Number::from(12))))));

        let prod = call(
            "checked_mul",
            &[Value::Number(Number::from(1_000_000_000)), Value::Number(Number::from(1_000_000_000))],
            &pool,
            builtins,
        )
        .unwrap();
        assert_eq!(
            prod,
            Value::Result(Ok(Box::new(Value::Number(Number::from(1_000_000_000_000_000_000i64)))))
        );

        let overflow = call(
            "checked_mul",
            &[
                Value::Number(Number::from(10_000_000_000i64)),
                Value::Number(Number::from(10_000_000_000i64)),
            ],
            &pool,
            builtins,
        )
        .unwrap();
        assert!(matches!(overflow, Value::Result(Err(_))), "expected overflow, got {overflow:?}");
    }

    #[test]
    fn clamped_i32_clamps_to_bounds() {
        let (pool, builtins) = setup();
        let args = |x: i64| {
            vec![
                Value::Number(Number::from(x)),
                Value::Number(Number::from(0)),
                Value::Number(Number::from(255)),
            ]
        };
        assert_eq!(
            call("clamped_i32", &args(1000), &pool, builtins).unwrap(),
            Value::Number(Number::from(255))
        );
        assert_eq!(
            call("clamped_i32", &args(-5), &pool, builtins).unwrap(),
            Value::Number(Number::from(0))
        );
        assert_eq!(
            call("clamped_i32", &args(128), &pool, builtins).unwrap(),
            Value::Number(Number::from(128))
        );
    }

    #[test]
    fn clamped_u64_bounds() {
        let (pool, builtins) = setup();
        let neg = call("clamped_u64", &[Value::Number(Number::from(-5))], &pool, builtins).unwrap();
        assert_eq!(neg, Value::Number(Number::Integer(BigInt::from(0))));

        let huge = Number::Real(Real::F64(1e30));
        let v = call("clamped_u64", &[Value::Number(huge)], &pool, builtins).unwrap();
        assert_eq!(v, Value::Number(Number::Integer(BigInt::from(u64::MAX))));
    }

    #[test]
    fn rounded_f64_digits() {
        let (pool, builtins) = setup();
        let pi = pool.symbol(builtins.pi);
        let v = call("rounded_f64", &[Value::Expr(pi), Value::Number(Number::from(3))], &pool, builtins).unwrap();
        let expected = (std::f64::consts::PI * 1000.0).round() / 1000.0;
        match v {
            Value::Number(Number::Real(Real::F64(x))) => assert_eq!(x, expected),
            other => panic!("expected F64, got {other:?}"),
        }
    }

    #[test]
    fn rounding_family_on_rational() {
        let (pool, builtins) = setup();
        let r = Number::from(22) / Number::from(7);
        assert!(matches!(r, Number::Rational(_)));

        let v = call("rounded_i32", &[Value::Number(r.clone())], &pool, builtins).unwrap();
        assert_eq!(v, Value::Number(Number::from(3)));

        let v = call("truncated_i32", &[Value::Number(r)], &pool, builtins).unwrap();
        assert_eq!(v, Value::Number(Number::from(3)));

        let neg = Number::from(-22) / Number::from(7);
        let v = call("truncated_i32", &[Value::Number(neg)], &pool, builtins).unwrap();
        assert_eq!(v, Value::Number(Number::from(-3)));
    }

    #[test]
    fn to_bigint_and_to_rational() {
        let (pool, builtins) = setup();
        let v = call("to_bigint", &[Value::Number(Number::from(7))], &pool, builtins).unwrap();
        assert_eq!(v, Value::Number(Number::Integer(BigInt::from(7))));

        let frac = Number::from(7) / Number::from(2);
        let err = call("to_bigint", &[Value::Number(frac.clone())], &pool, builtins).unwrap_err();
        assert!(matches!(err, RuntimeError::Collapse(_)));

        let v = call("to_rational", &[Value::Number(Number::from(3))], &pool, builtins).unwrap();
        assert!(matches!(v, Value::Number(Number::Rational(_))));

        let v = call("to_rational", &[Value::Number(frac)], &pool, builtins).unwrap();
        assert_eq!(v, Value::Number(Number::from(7) / Number::from(2)));
    }

    #[test]
    fn to_complex_wraps_real() {
        let (pool, builtins) = setup();
        let v = call("to_complex", &[Value::Number(Number::from(3))], &pool, builtins).unwrap();
        assert_eq!(
            v,
            Value::Number(Number::Complex {
                re: Box::new(Number::from(3)),
                im: Box::new(Number::from(0)),
            })
        );
    }

    #[test]
    fn unwrap_family() {
        let (pool, builtins) = setup();
        let ok = Value::Result(Ok(Box::new(Value::Number(Number::from(7)))));
        let err = Value::Result(Err("boom".to_string()));

        assert_eq!(call("unwrap", std::slice::from_ref(&ok), &pool, builtins).unwrap(), Value::Number(Number::from(7)));
        assert!(matches!(
            call("unwrap", std::slice::from_ref(&err), &pool, builtins),
            Err(RuntimeError::Message(m)) if m == "boom"
        ));

        assert_eq!(
            call("unwrap_or", &[err.clone(), Value::Number(Number::from(0))], &pool, builtins).unwrap(),
            Value::Number(Number::from(0))
        );
        assert_eq!(
            call("unwrap_or", &[ok.clone(), Value::Number(Number::from(0))], &pool, builtins).unwrap(),
            Value::Number(Number::from(7))
        );

        assert_eq!(
            call("expect", &[ok.clone(), Value::String("failed".into())], &pool, builtins).unwrap(),
            Value::Number(Number::from(7))
        );
        match call("expect", &[err, Value::String("failed".into())], &pool, builtins) {
            Err(RuntimeError::Message(m)) => assert_eq!(m, "failed: boom"),
            other => panic!("expected `failed: boom`, got {other:?}"),
        }
    }

    #[test]
    fn unknown_name_and_arity_are_messages() {
        let (pool, builtins) = setup();
        assert!(matches!(call("to_i33", &[], &pool, builtins), Err(RuntimeError::Message(_))));
        assert!(matches!(call("to_f64", &[], &pool, builtins), Err(RuntimeError::Message(_))));
        assert!(matches!(
            call("clamped_f64", &[Value::Number(Number::from(1))], &pool, builtins),
            Err(RuntimeError::Message(_))
        ));
    }
}

