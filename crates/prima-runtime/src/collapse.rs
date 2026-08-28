//! Collapse function family (spec §9.2–9.6): `to_/try_/checked_/clamped_/rounded_/truncated_` plus the `unwrap` family,
//! string helpers (`to_string`/`concat`, §18.1; `format` was removed in v2.2) and the `Option`/`Result` constructors (spec §4.4).
//!
//! The naming scheme encodes safety properties (spec §9.1): the basic forms raise a runtime error on failure (§9.2),
//! the try forms return a `Result` (§9.3), the checked forms check for overflow (§9.4), the clamped forms force a range (§9.5), and the rounding forms round in the specified way (§9.6).

use prima_core::collapse::collapse_value;
use prima_core::render::{render_latex, render_number};
use prima_core::{BuiltinSymbols, ExprPool, Number, Real, SymbolTable, Value, ValueKey};

use crate::error::RuntimeError;

/// Invoke a collapse builtin function (spec §9.2–9.6, §18.1, §4.4).
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
        // ---- basic collapse (spec §9.2): failure is a runtime error ----
        "to_f32" => {
            arity(name, args, 1)?;
            to_f32(name, &collapse_arg(pool, builtins, &args[0])?)
        }
        "to_f64" => {
            arity(name, args, 1)?;
            to_f64(name, &collapse_arg(pool, builtins, &args[0])?)
        }
        "to_bigint" => {
            arity(name, args, 1)?;
            to_bigint(name, &collapse_arg(pool, builtins, &args[0])?)
        }
        "to_rational" => {
            arity(name, args, 1)?;
            to_rational(name, &collapse_arg(pool, builtins, &args[0])?)
        }
        "to_bigfloat" => {
            arity(name, args, 1)?;
            to_bigfloat(&collapse_arg(pool, builtins, &args[0])?)
        }
        "to_complex" => {
            arity(name, args, 1)?;
            to_complex(&collapse_arg(pool, builtins, &args[0])?)
        }
        // `to_`/`try_`/`checked_` integer targets (spec §6.1/§9.2–9.4). `isize`/`usize` have no `checked_` form (spec §9.4).
        "to_i8" => {
            arity(name, args, 1)?;
            to_i8(name, &collapse_arg(pool, builtins, &args[0])?)
        }
        "try_i8" => {
            arity(name, args, 1)?;
            Ok(try_i8(name, try_arg(pool, builtins, &args[0])))
        }
        "checked_i8" => {
            arity(name, args, 1)?;
            checked_i8(name, &collapse_arg(pool, builtins, &args[0])?)
        }
        "to_i16" => {
            arity(name, args, 1)?;
            to_i16(name, &collapse_arg(pool, builtins, &args[0])?)
        }
        "try_i16" => {
            arity(name, args, 1)?;
            Ok(try_i16(name, try_arg(pool, builtins, &args[0])))
        }
        "checked_i16" => {
            arity(name, args, 1)?;
            checked_i16(name, &collapse_arg(pool, builtins, &args[0])?)
        }
        "to_i32" => {
            arity(name, args, 1)?;
            to_i32(name, &collapse_arg(pool, builtins, &args[0])?)
        }
        "try_i32" => {
            arity(name, args, 1)?;
            Ok(try_i32(name, try_arg(pool, builtins, &args[0])))
        }
        "checked_i32" => {
            arity(name, args, 1)?;
            checked_i32(name, &collapse_arg(pool, builtins, &args[0])?)
        }
        "to_i64" => {
            arity(name, args, 1)?;
            to_i64(name, &collapse_arg(pool, builtins, &args[0])?)
        }
        "try_i64" => {
            arity(name, args, 1)?;
            Ok(try_i64(name, try_arg(pool, builtins, &args[0])))
        }
        "checked_i64" => {
            arity(name, args, 1)?;
            checked_i64(name, &collapse_arg(pool, builtins, &args[0])?)
        }
        "to_i128" => {
            arity(name, args, 1)?;
            to_i128(name, &collapse_arg(pool, builtins, &args[0])?)
        }
        "try_i128" => {
            arity(name, args, 1)?;
            Ok(try_i128(name, try_arg(pool, builtins, &args[0])))
        }
        "checked_i128" => {
            arity(name, args, 1)?;
            checked_i128(name, &collapse_arg(pool, builtins, &args[0])?)
        }
        "to_u8" => {
            arity(name, args, 1)?;
            to_u8(name, &collapse_arg(pool, builtins, &args[0])?)
        }
        "try_u8" => {
            arity(name, args, 1)?;
            Ok(try_u8(name, try_arg(pool, builtins, &args[0])))
        }
        "checked_u8" => {
            arity(name, args, 1)?;
            checked_u8(name, &collapse_arg(pool, builtins, &args[0])?)
        }
        "to_u16" => {
            arity(name, args, 1)?;
            to_u16(name, &collapse_arg(pool, builtins, &args[0])?)
        }
        "try_u16" => {
            arity(name, args, 1)?;
            Ok(try_u16(name, try_arg(pool, builtins, &args[0])))
        }
        "checked_u16" => {
            arity(name, args, 1)?;
            checked_u16(name, &collapse_arg(pool, builtins, &args[0])?)
        }
        "to_u32" => {
            arity(name, args, 1)?;
            to_u32(name, &collapse_arg(pool, builtins, &args[0])?)
        }
        "try_u32" => {
            arity(name, args, 1)?;
            Ok(try_u32(name, try_arg(pool, builtins, &args[0])))
        }
        "checked_u32" => {
            arity(name, args, 1)?;
            checked_u32(name, &collapse_arg(pool, builtins, &args[0])?)
        }
        "to_u64" => {
            arity(name, args, 1)?;
            to_u64(name, &collapse_arg(pool, builtins, &args[0])?)
        }
        "try_u64" => {
            arity(name, args, 1)?;
            Ok(try_u64(name, try_arg(pool, builtins, &args[0])))
        }
        "checked_u64" => {
            arity(name, args, 1)?;
            checked_u64(name, &collapse_arg(pool, builtins, &args[0])?)
        }
        "to_u128" => {
            arity(name, args, 1)?;
            to_u128(name, &collapse_arg(pool, builtins, &args[0])?)
        }
        "try_u128" => {
            arity(name, args, 1)?;
            Ok(try_u128(name, try_arg(pool, builtins, &args[0])))
        }
        "checked_u128" => {
            arity(name, args, 1)?;
            checked_u128(name, &collapse_arg(pool, builtins, &args[0])?)
        }
        "to_isize" => {
            arity(name, args, 1)?;
            to_isize(name, &collapse_arg(pool, builtins, &args[0])?)
        }
        "try_isize" => {
            arity(name, args, 1)?;
            Ok(try_isize(name, try_arg(pool, builtins, &args[0])))
        }
        "to_usize" => {
            arity(name, args, 1)?;
            to_usize(name, &collapse_arg(pool, builtins, &args[0])?)
        }
        "try_usize" => {
            arity(name, args, 1)?;
            Ok(try_usize(name, try_arg(pool, builtins, &args[0])))
        }
        // ---- try collapse (spec §9.3): result wrapped in a `Result`, no runtime error is raised ----
        "try_f32" => {
            arity(name, args, 1)?;
            Ok(try_f32(name, try_arg(pool, builtins, &args[0])))
        }
        "try_f64" => {
            arity(name, args, 1)?;
            Ok(try_f64(name, try_arg(pool, builtins, &args[0])))
        }
        "try_bigint" => {
            arity(name, args, 1)?;
            Ok(try_bigint(name, try_arg(pool, builtins, &args[0])))
        }
        "try_rational" => {
            arity(name, args, 1)?;
            Ok(try_rational(name, try_arg(pool, builtins, &args[0])))
        }
        "try_complex" => {
            arity(name, args, 1)?;
            Ok(try_complex(name, try_arg(pool, builtins, &args[0])))
        }
        // ---- checked collapse (spec §9.4): overflow/range check, returns a `Result` ----
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
        // ---- clamped collapse (spec §9.5): force into range ----
        "clamped_i8" => clamped_3(name, args, pool, builtins, clamped_i8),
        "clamped_i16" => clamped_3(name, args, pool, builtins, clamped_i16),
        "clamped_i32" => clamped_3(name, args, pool, builtins, clamped_i32),
        "clamped_i64" => clamped_3(name, args, pool, builtins, clamped_i64),
        "clamped_i128" => clamped_3(name, args, pool, builtins, clamped_i128),
        "clamped_u8" => clamped_3(name, args, pool, builtins, clamped_u8),
        "clamped_u16" => clamped_3(name, args, pool, builtins, clamped_u16),
        "clamped_u32" => clamped_3(name, args, pool, builtins, clamped_u32),
        "clamped_u128" => clamped_3(name, args, pool, builtins, clamped_u128),
        "clamped_u64" => {
            arity(name, args, 1)?;
            clamped_u64(name, &collapse_arg(pool, builtins, &args[0])?)
        }
        "clamped_f32" => clamped_3(name, args, pool, builtins, clamped_f32),
        "clamped_f64" => clamped_3(name, args, pool, builtins, clamped_f64),
        // ---- rounding collapse (spec §9.6) ----
        "rounded_f64" => {
            arity(name, args, 2)?;
            let x = collapse_arg(pool, builtins, &args[0])?;
            let digits = collapse_arg(pool, builtins, &args[1])?;
            rounded_f64(name, &x, &digits)
        }
        "rounded_f32" => {
            arity(name, args, 2)?;
            let x = collapse_arg(pool, builtins, &args[0])?;
            let digits = collapse_arg(pool, builtins, &args[1])?;
            rounded_f32(name, &x, &digits)
        }
        "rounded_i32" => {
            arity(name, args, 1)?;
            rounded_i32(name, &collapse_arg(pool, builtins, &args[0])?)
        }
        "truncated_i32" => {
            arity(name, args, 1)?;
            truncated_i32(name, &collapse_arg(pool, builtins, &args[0])?)
        }
        // ---- `unwrap` family: unwrap the `Result`/`Option` of safe collapse (spec §9.3, §16.3) ----
        "unwrap" => unwrap(name, args),
        "unwrap_or" => unwrap_or(name, args),
        "expect" => expect(name, args),
        // ---- Option/Result constructors (spec §4.4) ----
        "Some" => some(args),
        "None" => none(args),
        "Ok" => ok(args),
        "Err" => err_builtin(args),
        // ---- string/format helpers (spec §18.1); `format` was removed in v2.2 (f-strings) ----
        "to_string" => to_string_call(args, pool),
        "concat" => concat_call(args, pool),
        _ => Err(RuntimeError::Message(format!(
            "unknown collapse function `{name}`"
        ))),
    }
}

fn arity(name: &str, args: &[Value], n: usize) -> Result<(), RuntimeError> {
    if args.len() == n {
        Ok(())
    } else {
        Err(RuntimeError::Message(format!(
            "`{name}` expects {n} argument(s), got {}",
            args.len()
        )))
    }
}

fn collapse_arg(
    pool: &ExprPool,
    builtins: &BuiltinSymbols,
    v: &Value,
) -> Result<Number, RuntimeError> {
    collapse_value(pool, builtins, v)
        .or_else(|| numeric_string(v))
        .ok_or_else(|| RuntimeError::Collapse(format!("cannot collapse {v:?} to a number")))
}

/// Parse a numeric string argument (`try_f64("3.14")`, spec §16.3): a `Value::String` is parsed as a number.
fn numeric_string(v: &Value) -> Option<Number> {
    match v {
        Value::String(s) => s.trim().parse::<f64>().ok().map(Number::from),
        _ => None,
    }
}

/// Argument to the `try_*` family: `None` means the argument could not be collapsed to a number
/// (reported as a `Result::Err` rather than a runtime error, spec §9.3).
fn try_arg(pool: &ExprPool, builtins: &BuiltinSymbols, v: &Value) -> Option<Number> {
    collapse_value(pool, builtins, v).or_else(|| numeric_string(v))
}

/// A complex number cannot collapse to the real numeric domain (spec §9.2 basic collapse operates on reals).
fn ensure_real(name: &str, n: &Number) -> Result<(), RuntimeError> {
    if n.is_complex() {
        Err(RuntimeError::Domain(format!(
            "`{name}` does not accept complex values, got {n}"
        )))
    } else {
        Ok(())
    }
}

// ---- Table-driven integer collapse (spec §6.1/§9.2–9.4) ----
//
// The twelve fixed-width integer targets (`i8`…`usize`, spec §6.1) share the same shapes: `to_*` raises
// `RuntimeError::Overflow` on a range/integrality failure, `try_*` returns a `Value::Result` instead, and
// `checked_*` (the ten non-isize/usize targets, spec §9.4) also returns a `Value::Result` whose `Err` names the
// overflow. Success always produces the matching fixed-width `Number` variant so the collapsed type identity survives
// (spec §6.1: collapsed types exist only after explicit collapse).

macro_rules! int_collapse_fns {
    // `to_`/`try_`/`checked_` (the ten fixed integer targets, spec §9.2–9.4).
    ($to_fn:ident, $try_fn:ident, $checked_fn:ident, $variant:ident, $as:ident, $ty:ty) => {
        fn $to_fn(name: &str, n: &Number) -> Result<Value, RuntimeError> {
            ensure_real(name, n)?;
            match n.$as() {
                Some(v) => Ok(Value::Number(Number::$variant(v))),
                None => Err(RuntimeError::Overflow(format!(
                    "`{name}`: {n} cannot be represented as {}",
                    stringify!($ty)
                ))),
            }
        }

        fn $try_fn(name: &str, n: Option<Number>) -> Value {
            let Some(n) = n else {
                return Value::Result(Err(format!(
                    "`{name}`: cannot collapse argument to a number"
                )));
            };
            if n.is_complex() {
                return Value::Result(Err(format!(
                    "`{name}`: cannot convert complex value {n} to {}",
                    stringify!($ty)
                )));
            }
            match n.$as() {
                Some(v) => Value::Result(Ok(Box::new(Value::Number(Number::$variant(v))))),
                None => Value::Result(Err(format!(
                    "`{name}`: {n} cannot be represented as {}",
                    stringify!($ty)
                ))),
            }
        }

        fn $checked_fn(name: &str, n: &Number) -> Result<Value, RuntimeError> {
            ensure_real(name, n)?;
            match n.$as() {
                Some(v) => Ok(Value::Result(Ok(Box::new(Value::Number(
                    Number::$variant(v),
                ))))),
                None => Ok(Value::Result(Err(format!(
                    "overflow: `{name}`: {n} cannot be represented as {}",
                    stringify!($ty)
                )))),
            }
        }
    };
    // `to_`/`try_` only (`isize`/`usize` have no `checked_` form, spec §9.4).
    ($to_fn:ident, $try_fn:ident, $variant:ident, $as:ident, $ty:ty) => {
        fn $to_fn(name: &str, n: &Number) -> Result<Value, RuntimeError> {
            ensure_real(name, n)?;
            match n.$as() {
                Some(v) => Ok(Value::Number(Number::$variant(v))),
                None => Err(RuntimeError::Overflow(format!(
                    "`{name}`: {n} cannot be represented as {}",
                    stringify!($ty)
                ))),
            }
        }

        fn $try_fn(name: &str, n: Option<Number>) -> Value {
            let Some(n) = n else {
                return Value::Result(Err(format!(
                    "`{name}`: cannot collapse argument to a number"
                )));
            };
            if n.is_complex() {
                return Value::Result(Err(format!(
                    "`{name}`: cannot convert complex value {n} to {}",
                    stringify!($ty)
                )));
            }
            match n.$as() {
                Some(v) => Value::Result(Ok(Box::new(Value::Number(Number::$variant(v))))),
                None => Value::Result(Err(format!(
                    "`{name}`: {n} cannot be represented as {}",
                    stringify!($ty)
                ))),
            }
        }
    };
}

int_collapse_fns!(to_i8, try_i8, checked_i8, I8, as_i8, i8);
int_collapse_fns!(to_i16, try_i16, checked_i16, I16, as_i16, i16);
int_collapse_fns!(to_i32, try_i32, checked_i32, I32, as_i32, i32);
int_collapse_fns!(to_i64, try_i64, checked_i64, I64, as_i64, i64);
int_collapse_fns!(to_i128, try_i128, checked_i128, I128, as_i128, i128);
int_collapse_fns!(to_u8, try_u8, checked_u8, U8, as_u8, u8);
int_collapse_fns!(to_u16, try_u16, checked_u16, U16, as_u16, u16);
int_collapse_fns!(to_u32, try_u32, checked_u32, U32, as_u32, u32);
int_collapse_fns!(to_u64, try_u64, checked_u64, U64, as_u64, u64);
int_collapse_fns!(to_u128, try_u128, checked_u128, U128, as_u128, u128);
int_collapse_fns!(to_isize, try_isize, Isize, as_isize, isize);
int_collapse_fns!(to_usize, try_usize, Usize, as_usize, usize);

// ---- Basic collapse (spec §9.2): failure is a runtime error ----

fn to_f32(name: &str, n: &Number) -> Result<Value, RuntimeError> {
    ensure_real(name, n)?;
    Ok(Value::Number(Number::Real(Real::F32(
        n.to_f64_lossy() as f32
    ))))
}

fn to_f64(name: &str, n: &Number) -> Result<Value, RuntimeError> {
    ensure_real(name, n)?;
    Ok(Value::Number(Number::Real(Real::F64(n.to_f64_lossy()))))
}

fn to_bigint(name: &str, n: &Number) -> Result<Value, RuntimeError> {
    match n.as_bigint() {
        Some(b) => Ok(Value::Number(Number::Integer(b))),
        None => Err(RuntimeError::Collapse(format!(
            "`{name}`: {n} is not an integer"
        ))),
    }
}

fn to_rational(name: &str, n: &Number) -> Result<Value, RuntimeError> {
    match n.as_rational() {
        Some(r) => Ok(Value::Number(Number::Rational(r))),
        None => Err(RuntimeError::Collapse(format!(
            "`{name}`: {n} cannot be represented as a rational"
        ))),
    }
}

/// `to_bigfloat` (spec §9.2) is a degenerate implementation that preserves precision: the number is returned unchanged (exact values are kept, `Real` stays f64).
/// Arbitrary-precision floats (BigFloat) are deferred to a later release.
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

fn try_f32(name: &str, n: Option<Number>) -> Value {
    let Some(n) = n else {
        return Value::Result(Err(format!(
            "`{name}`: cannot collapse argument to a number"
        )));
    };
    if n.is_complex() {
        return Value::Result(Err(format!(
            "`{name}`: cannot convert complex value {n} to f32"
        )));
    }
    Value::Result(Ok(Box::new(Value::Number(Number::Real(Real::F32(
        n.to_f64_lossy() as f32,
    ))))))
}

fn try_f64(name: &str, n: Option<Number>) -> Value {
    let Some(n) = n else {
        return Value::Result(Err(format!(
            "`{name}`: cannot collapse argument to a number"
        )));
    };
    if n.is_complex() {
        return Value::Result(Err(format!(
            "`{name}`: cannot convert complex value {n} to f64"
        )));
    }
    Value::Result(Ok(Box::new(Value::Number(Number::Real(Real::F64(
        n.to_f64_lossy(),
    ))))))
}

fn try_bigint(name: &str, n: Option<Number>) -> Value {
    let Some(n) = n else {
        return Value::Result(Err(format!(
            "`{name}`: cannot collapse argument to a number"
        )));
    };
    match n.as_bigint() {
        Some(b) => Value::Result(Ok(Box::new(Value::Number(Number::Integer(b))))),
        None => Value::Result(Err(format!("`{name}`: {n} is not an integer"))),
    }
}

fn try_rational(name: &str, n: Option<Number>) -> Value {
    let Some(n) = n else {
        return Value::Result(Err(format!(
            "`{name}`: cannot collapse argument to a number"
        )));
    };
    match n.as_rational() {
        Some(r) => Value::Result(Ok(Box::new(Value::Number(Number::Rational(r))))),
        None => Value::Result(Err(format!(
            "`{name}`: {n} cannot be represented as a rational"
        ))),
    }
}

fn try_complex(name: &str, n: Option<Number>) -> Value {
    let Some(n) = n else {
        return Value::Result(Err(format!(
            "`{name}`: cannot collapse argument to a number"
        )));
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

/// `checked_add`/`checked_mul` (spec §9.4): evaluate via f64, then verify the result can be exactly represented back in i64, otherwise `Err`.
fn checked_binary(
    name: &str,
    a: &Number,
    b: &Number,
    op: fn(f64, f64) -> f64,
) -> Result<Value, RuntimeError> {
    ensure_real(name, a)?;
    ensure_real(name, b)?;
    let v = op(a.to_f64_lossy(), b.to_f64_lossy());
    let vi = v as i64;
    if v.is_nan() || v.is_infinite() || vi as f64 != v {
        return Ok(Value::Result(Err(format!(
            "overflow: `{name}`: result {v} cannot be represented as i64"
        ))));
    }
    Ok(Value::Result(Ok(Box::new(Value::Number(Number::from(vi))))))
}

// ---- Clamped collapse (spec §9.5): force into range ----

/// Shared 3-argument `clamped_<ty>(x, min, max)` dispatch: collapse all three args to numbers, then clamp.
fn clamped_3(
    name: &str,
    args: &[Value],
    pool: &ExprPool,
    builtins: &BuiltinSymbols,
    f: fn(&str, &Number, &Number, &Number) -> Result<Value, RuntimeError>,
) -> Result<Value, RuntimeError> {
    arity(name, args, 3)?;
    let x = collapse_arg(pool, builtins, &args[0])?;
    let min = collapse_arg(pool, builtins, &args[1])?;
    let max = collapse_arg(pool, builtins, &args[2])?;
    f(name, &x, &min, &max)
}

macro_rules! int_clamped_fns {
    ($fn:ident, $variant:ident, $ty:ty) => {
        fn $fn(name: &str, x: &Number, min: &Number, max: &Number) -> Result<Value, RuntimeError> {
            ensure_real(name, x)?;
            ensure_real(name, min)?;
            ensure_real(name, max)?;
            let c = x
                .to_f64_lossy()
                .clamp(min.to_f64_lossy(), max.to_f64_lossy());
            Ok(Value::Number(Number::$variant(c as $ty)))
        }
    };
}

// The unsigned 3-argument forms clamp to `[min, max]` (spec §9.5 `clamped_u8(x, min, max)`); `clamped_u64` keeps the
// single-argument `[0, u64::MAX]` form from the earlier implementation (spec §9.5 lists both).
int_clamped_fns!(clamped_i8, I8, i8);
int_clamped_fns!(clamped_i16, I16, i16);
int_clamped_fns!(clamped_i32, I32, i32);
int_clamped_fns!(clamped_i64, I64, i64);
int_clamped_fns!(clamped_i128, I128, i128);
int_clamped_fns!(clamped_u8, U8, u8);
int_clamped_fns!(clamped_u16, U16, u16);
int_clamped_fns!(clamped_u32, U32, u32);
int_clamped_fns!(clamped_u128, U128, u128);

/// `clamped_u64` (spec §9.5): single argument, clamped to `[0, u64::MAX]` (negative → 0, out of range → `u64::MAX`).
fn clamped_u64(name: &str, n: &Number) -> Result<Value, RuntimeError> {
    ensure_real(name, n)?;
    let c = n.to_f64_lossy().clamp(0.0, u64::MAX as f64);
    Ok(Value::Number(Number::U64(c as u64)))
}

fn clamped_f32(name: &str, x: &Number, min: &Number, max: &Number) -> Result<Value, RuntimeError> {
    ensure_real(name, x)?;
    ensure_real(name, min)?;
    ensure_real(name, max)?;
    let v = x
        .to_f64_lossy()
        .clamp(min.to_f64_lossy(), max.to_f64_lossy());
    Ok(Value::Number(Number::Real(Real::F32(v as f32))))
}

fn clamped_f64(name: &str, x: &Number, min: &Number, max: &Number) -> Result<Value, RuntimeError> {
    ensure_real(name, x)?;
    ensure_real(name, min)?;
    ensure_real(name, max)?;
    Ok(Value::Number(
        x.clamped_f64(min.to_f64_lossy(), max.to_f64_lossy()),
    ))
}

// ---- Rounding collapse (spec §9.6) ----

fn rounded_f64(name: &str, x: &Number, digits: &Number) -> Result<Value, RuntimeError> {
    ensure_real(name, x)?;
    let d = digits.as_i64().ok_or_else(|| {
        RuntimeError::Type(format!(
            "`{name}` expects an integer digit count, got {digits}"
        ))
    })?;
    Ok(Value::Number(x.rounded_digits(d)))
}

fn rounded_f32(name: &str, x: &Number, digits: &Number) -> Result<Value, RuntimeError> {
    ensure_real(name, x)?;
    let d = digits.as_i64().ok_or_else(|| {
        RuntimeError::Type(format!(
            "`{name}` expects an integer digit count, got {digits}"
        ))
    })?;
    let mult = 10f64.powi(d as i32);
    let v = (x.to_f64_lossy() * mult).round() / mult;
    Ok(Value::Number(Number::Real(Real::F32(v as f32))))
}

fn rounded_i32(name: &str, n: &Number) -> Result<Value, RuntimeError> {
    ensure_real(name, n)?;
    Ok(Value::Number(n.round()))
}

fn truncated_i32(name: &str, n: &Number) -> Result<Value, RuntimeError> {
    ensure_real(name, n)?;
    Ok(Value::Number(n.truncate()))
}

// ---- `unwrap` family: unwrap the `Result`/`Option` of safe collapse (spec §9.3, §16.3) ----

fn unwrap(name: &str, args: &[Value]) -> Result<Value, RuntimeError> {
    arity(name, args, 1)?;
    match &args[0] {
        Value::Result(Ok(v)) | Value::Option(Some(v)) => Ok((**v).clone()),
        Value::Result(Err(msg)) => Err(RuntimeError::Message(msg.clone())),
        Value::Option(None) => Err(RuntimeError::Message(format!(
            "called `{name}` on a `None` value"
        ))),
        other => Err(RuntimeError::Type(format!(
            "`{name}` expects a `Result` or `Option` value, got {other:?}"
        ))),
    }
}

fn unwrap_or(name: &str, args: &[Value]) -> Result<Value, RuntimeError> {
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
fn expect(name: &str, args: &[Value]) -> Result<Value, RuntimeError> {
    arity(name, args, 2)?;
    let msg = arg_to_string(&args[1]);
    match &args[0] {
        Value::Result(Ok(v)) | Value::Option(Some(v)) => Ok((**v).clone()),
        Value::Result(Err(err)) => Err(RuntimeError::Message(format!("{msg}: {err}"))),
        Value::Option(None) => Err(RuntimeError::Message(msg)),
        other => Err(RuntimeError::Type(format!(
            "`{name}` expects a `Result` or `Option` value, got {other:?}"
        ))),
    }
}

// ---- Option/Result constructors (spec §4.4) ----

fn some(args: &[Value]) -> Result<Value, RuntimeError> {
    arity("Some", args, 1)?;
    Ok(Value::Option(Some(Box::new(args[0].clone()))))
}

fn none(args: &[Value]) -> Result<Value, RuntimeError> {
    arity("None", args, 0)?;
    Ok(Value::Option(None))
}

fn ok(args: &[Value]) -> Result<Value, RuntimeError> {
    arity("Ok", args, 1)?;
    Ok(Value::Result(Ok(Box::new(args[0].clone()))))
}

fn err_builtin(args: &[Value]) -> Result<Value, RuntimeError> {
    arity("Err", args, 1)?;
    Ok(Value::Result(Err(arg_to_string(&args[0]))))
}

// ---- String/format helpers (spec §18.1) ----
// `format` was removed in v2.2 (f-strings replace it, spec §18.1); `to_string`/`concat` remain.

fn to_string_call(args: &[Value], pool: &ExprPool) -> Result<Value, RuntimeError> {
    arity("to_string", args, 1)?;
    Ok(Value::String(value_to_string(pool, &args[0])))
}

fn concat_call(args: &[Value], pool: &ExprPool) -> Result<Value, RuntimeError> {
    arity("concat", args, 2)?;
    let a = value_to_string(pool, &args[0]);
    let b = value_to_string(pool, &args[1]);
    Ok(Value::String(a + &b))
}

/// Render any value to its display string (spec §16.1 `format_value`): mirrors the evaluator's rendering so
/// `to_string`/`concat` agree with `print`. `Expr` renders through the LaTeX view (spec §8.3).
fn value_to_string(pool: &ExprPool, v: &Value) -> String {
    match v {
        Value::Nil => "nil".into(),
        Value::Number(n) => render_number(n),
        Value::Bool(b) => b.to_string(),
        Value::Char(c) => c.to_string(),
        Value::String(s) => s.clone(),
        Value::Array(elems) => {
            let inner: Vec<String> = elems.iter().map(|e| value_to_string(pool, e)).collect();
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

fn arg_to_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => format!("{other:?}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use num_bigint::BigInt;

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
            Value::Number(Number::Real(Real::F64(x))) => {
                assert!((x - std::f64::consts::SQRT_2).abs() < 1e-9)
            }
            other => panic!("expected F64, got {other:?}"),
        }
    }

    #[test]
    fn to_integer_family_produces_fixed_width_variants() {
        let (pool, builtins) = setup();
        assert_eq!(
            call("to_i8", &[Value::Number(Number::from(7))], &pool, builtins).unwrap(),
            Value::Number(Number::I8(7))
        );
        assert_eq!(
            call(
                "to_i16",
                &[Value::Number(Number::from(300))],
                &pool,
                builtins
            )
            .unwrap(),
            Value::Number(Number::I16(300))
        );
        assert_eq!(
            call(
                "to_i64",
                &[Value::Number(Number::from(42))],
                &pool,
                builtins
            )
            .unwrap(),
            Value::Number(Number::I64(42))
        );
        assert_eq!(
            call(
                "to_i128",
                &[Value::Number(Number::from(-7))],
                &pool,
                builtins
            )
            .unwrap(),
            Value::Number(Number::I128(-7))
        );
        assert_eq!(
            call(
                "to_u64",
                &[Value::Number(Number::from(42))],
                &pool,
                builtins
            )
            .unwrap(),
            Value::Number(Number::U64(42))
        );
        assert_eq!(
            call(
                "to_usize",
                &[Value::Number(Number::from(42))],
                &pool,
                builtins
            )
            .unwrap(),
            Value::Number(Number::Usize(42))
        );
        // Rational with integral value also collapses.
        let r = Number::from(10) / Number::from(2);
        assert_eq!(
            call("to_i32", &[Value::Number(r)], &pool, builtins).unwrap(),
            Value::Number(Number::I32(5))
        );
    }

    #[test]
    fn to_integer_overflow_errors() {
        let (pool, builtins) = setup();
        let big = Value::Number(Number::Integer(BigInt::from(2_147_483_648i64)));
        let err = call("to_i32", std::slice::from_ref(&big), &pool, builtins).unwrap_err();
        assert!(matches!(err, RuntimeError::Overflow(_)));

        let err = call(
            "to_i8",
            &[Value::Number(Number::from(200))],
            &pool,
            builtins,
        )
        .unwrap_err();
        assert!(matches!(err, RuntimeError::Overflow(_)));

        let err = call(
            "to_u64",
            &[Value::Number(Number::from(-1))],
            &pool,
            builtins,
        )
        .unwrap_err();
        assert!(matches!(err, RuntimeError::Overflow(_)));

        // Non-integral values also fail with overflow (spec §9.8: only collapse failure errors).
        let frac = Number::from(7) / Number::from(2);
        let err = call("to_i8", &[Value::Number(frac)], &pool, builtins).unwrap_err();
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
    fn try_family_returns_result_values() {
        let (pool, builtins) = setup();
        let in_range = call(
            "try_i8",
            &[Value::Number(Number::from(42))],
            &pool,
            builtins,
        )
        .unwrap();
        assert_eq!(
            in_range,
            Value::Result(Ok(Box::new(Value::Number(Number::I8(42)))))
        );

        let out = call(
            "try_i8",
            &[Value::Number(Number::Integer(BigInt::from(200)))],
            &pool,
            builtins,
        )
        .unwrap();
        assert!(
            matches!(out, Value::Result(Err(_))),
            "expected Err result, got {out:?}"
        );

        let nonnum = call("try_i8", &[Value::Bool(true)], &pool, builtins).unwrap();
        assert!(
            matches!(nonnum, Value::Result(Err(_))),
            "expected Err result, got {nonnum:?}"
        );

        let ok_f64 = call(
            "try_f64",
            &[Value::Number(Number::from(3))],
            &pool,
            builtins,
        )
        .unwrap();
        assert!(
            matches!(ok_f64, Value::Result(Ok(v)) if matches!(*v, Value::Number(Number::Real(Real::F64(_)))))
        );

        let ok_usize = call(
            "try_usize",
            &[Value::Number(Number::from(3))],
            &pool,
            builtins,
        )
        .unwrap();
        assert_eq!(
            ok_usize,
            Value::Result(Ok(Box::new(Value::Number(Number::Usize(3)))))
        );
    }

    #[test]
    fn checked_family_and_arithmetic() {
        let (pool, builtins) = setup();
        let ok = call(
            "checked_i32",
            &[Value::Number(Number::from(5))],
            &pool,
            builtins,
        )
        .unwrap();
        assert_eq!(
            ok,
            Value::Result(Ok(Box::new(Value::Number(Number::I32(5)))))
        );

        let out = call(
            "checked_i32",
            &[Value::Number(Number::Integer(BigInt::from(
                2_147_483_648i64,
            )))],
            &pool,
            builtins,
        )
        .unwrap();
        assert!(
            matches!(out, Value::Result(Err(_))),
            "expected Err result, got {out:?}"
        );

        let ok_u128 = call(
            "checked_u128",
            &[Value::Number(Number::Integer(BigInt::from(u128::MAX)))],
            &pool,
            builtins,
        )
        .unwrap();
        assert_eq!(
            ok_u128,
            Value::Result(Ok(Box::new(Value::Number(Number::U128(u128::MAX)))))
        );

        let out_of_range = call(
            "checked_u128",
            &[Value::Number(Number::Integer(BigInt::from(-1)))],
            &pool,
            builtins,
        )
        .unwrap();
        assert!(matches!(out_of_range, Value::Result(Err(_))));

        let sum = call(
            "checked_add",
            &[
                Value::Number(Number::from(5)),
                Value::Number(Number::from(7)),
            ],
            &pool,
            builtins,
        )
        .unwrap();
        assert_eq!(
            sum,
            Value::Result(Ok(Box::new(Value::Number(Number::from(12)))))
        );

        let prod = call(
            "checked_mul",
            &[
                Value::Number(Number::from(1_000_000_000)),
                Value::Number(Number::from(1_000_000_000)),
            ],
            &pool,
            builtins,
        )
        .unwrap();
        assert_eq!(
            prod,
            Value::Result(Ok(Box::new(Value::Number(Number::from(
                1_000_000_000_000_000_000i64
            )))))
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
        assert!(
            matches!(overflow, Value::Result(Err(_))),
            "expected overflow, got {overflow:?}"
        );
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
            Value::Number(Number::I32(255))
        );
        assert_eq!(
            call("clamped_i32", &args(-5), &pool, builtins).unwrap(),
            Value::Number(Number::I32(0))
        );
        assert_eq!(
            call("clamped_i32", &args(128), &pool, builtins).unwrap(),
            Value::Number(Number::I32(128))
        );
        // Out-of-i8-range bounds saturate to the i8 range on the cast back.
        assert_eq!(
            call("clamped_i8", &args(200), &pool, builtins).unwrap(),
            Value::Number(Number::I8(127))
        );
    }

    #[test]
    fn clamped_u64_bounds() {
        let (pool, builtins) = setup();
        let neg = call(
            "clamped_u64",
            &[Value::Number(Number::from(-5))],
            &pool,
            builtins,
        )
        .unwrap();
        assert_eq!(neg, Value::Number(Number::U64(0)));

        let huge = Number::Real(Real::F64(1e30));
        let v = call("clamped_u64", &[Value::Number(huge)], &pool, builtins).unwrap();
        assert_eq!(v, Value::Number(Number::U64(u64::MAX)));
    }

    #[test]
    fn rounded_f64_digits() {
        let (pool, builtins) = setup();
        let pi = pool.symbol(builtins.pi);
        let v = call(
            "rounded_f64",
            &[Value::Expr(pi), Value::Number(Number::from(3))],
            &pool,
            builtins,
        )
        .unwrap();
        let expected = (std::f64::consts::PI * 1000.0).round() / 1000.0;
        match v {
            Value::Number(Number::Real(Real::F64(x))) => assert_eq!(x, expected),
            other => panic!("expected F64, got {other:?}"),
        }
    }

    #[test]
    fn rounded_f32_digits() {
        let (pool, builtins) = setup();
        let v = call(
            "rounded_f32",
            &[
                Value::Number(Number::from(std::f32::consts::PI as f64)),
                Value::Number(Number::from(2)),
            ],
            &pool,
            builtins,
        )
        .unwrap();
        match v {
            Value::Number(Number::Real(Real::F32(x))) => {
                let expected = (std::f32::consts::PI * 100.0).round() / 100.0;
                assert_eq!(x, expected);
            }
            other => panic!("expected F32, got {other:?}"),
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
        let v = call(
            "to_bigint",
            &[Value::Number(Number::from(7))],
            &pool,
            builtins,
        )
        .unwrap();
        assert_eq!(v, Value::Number(Number::Integer(BigInt::from(7))));

        let frac = Number::from(7) / Number::from(2);
        let err = call("to_bigint", &[Value::Number(frac.clone())], &pool, builtins).unwrap_err();
        assert!(matches!(err, RuntimeError::Collapse(_)));

        let v = call(
            "to_rational",
            &[Value::Number(Number::from(3))],
            &pool,
            builtins,
        )
        .unwrap();
        assert!(matches!(v, Value::Number(Number::Rational(_))));

        let v = call("to_rational", &[Value::Number(frac)], &pool, builtins).unwrap();
        assert_eq!(v, Value::Number(Number::from(7) / Number::from(2)));
    }

    #[test]
    fn to_complex_wraps_real() {
        let (pool, builtins) = setup();
        let v = call(
            "to_complex",
            &[Value::Number(Number::from(3))],
            &pool,
            builtins,
        )
        .unwrap();
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

        assert_eq!(
            call("unwrap", std::slice::from_ref(&ok), &pool, builtins).unwrap(),
            Value::Number(Number::from(7))
        );
        assert!(matches!(
            call("unwrap", std::slice::from_ref(&err), &pool, builtins),
            Err(RuntimeError::Message(m)) if m == "boom"
        ));

        assert_eq!(
            call(
                "unwrap_or",
                &[err.clone(), Value::Number(Number::from(0))],
                &pool,
                builtins
            )
            .unwrap(),
            Value::Number(Number::from(0))
        );
        assert_eq!(
            call(
                "unwrap_or",
                &[ok.clone(), Value::Number(Number::from(0))],
                &pool,
                builtins
            )
            .unwrap(),
            Value::Number(Number::from(7))
        );

        assert_eq!(
            call(
                "expect",
                &[ok.clone(), Value::String("failed".into())],
                &pool,
                builtins
            )
            .unwrap(),
            Value::Number(Number::from(7))
        );
        match call(
            "expect",
            &[err, Value::String("failed".into())],
            &pool,
            builtins,
        ) {
            Err(RuntimeError::Message(m)) => assert_eq!(m, "failed: boom"),
            other => panic!("expected `failed: boom`, got {other:?}"),
        }
    }

    #[test]
    fn unwrap_family_accepts_option() {
        let (pool, builtins) = setup();
        let some = Value::Option(Some(Box::new(Value::Number(Number::from(3)))));
        let none = Value::Option(None);

        assert_eq!(
            call("unwrap", std::slice::from_ref(&some), &pool, builtins).unwrap(),
            Value::Number(Number::from(3))
        );
        assert!(matches!(
            call("unwrap", std::slice::from_ref(&none), &pool, builtins),
            Err(RuntimeError::Message(m)) if m.contains("None")
        ));

        assert_eq!(
            call(
                "unwrap_or",
                &[none.clone(), Value::Number(Number::from(0))],
                &pool,
                builtins
            )
            .unwrap(),
            Value::Number(Number::from(0))
        );
        assert_eq!(
            call(
                "unwrap_or",
                &[some.clone(), Value::Number(Number::from(0))],
                &pool,
                builtins
            )
            .unwrap(),
            Value::Number(Number::from(3))
        );
        assert_eq!(
            call(
                "expect",
                &[some, Value::String("wanted".into())],
                &pool,
                builtins
            )
            .unwrap(),
            Value::Number(Number::from(3))
        );
    }

    #[test]
    fn option_result_constructors() {
        let (pool, builtins) = setup();
        assert_eq!(
            call("Some", &[Value::Number(Number::from(3))], &pool, builtins).unwrap(),
            Value::Option(Some(Box::new(Value::Number(Number::from(3)))))
        );
        assert_eq!(
            call("None", &[], &pool, builtins).unwrap(),
            Value::Option(None)
        );
        assert_eq!(
            call("Ok", &[Value::Number(Number::from(3))], &pool, builtins).unwrap(),
            Value::Result(Ok(Box::new(Value::Number(Number::from(3)))))
        );
        assert_eq!(
            call("Err", &[Value::String("boom".into())], &pool, builtins).unwrap(),
            Value::Result(Err("boom".into()))
        );
    }

    #[test]
    fn to_string_and_concat() {
        let (pool, builtins) = setup();
        let s = call(
            "to_string",
            &[Value::Number(Number::from(42))],
            &pool,
            builtins,
        )
        .unwrap();
        assert_eq!(s, Value::String("42".into()));

        let s = call(
            "concat",
            &[Value::String("a".into()), Value::String("b".into())],
            &pool,
            builtins,
        )
        .unwrap();
        assert_eq!(s, Value::String("ab".into()));

        let s = call(
            "concat",
            &[Value::Number(Number::from(1)), Value::String("x".into())],
            &pool,
            builtins,
        )
        .unwrap();
        assert_eq!(s, Value::String("1x".into()));
    }

    #[test]
    fn unknown_name_and_arity_are_messages() {
        let (pool, builtins) = setup();
        assert!(matches!(
            call("to_i33", &[], &pool, builtins),
            Err(RuntimeError::Message(_))
        ));
        assert!(matches!(
            call("to_f64", &[], &pool, builtins),
            Err(RuntimeError::Message(_))
        ));
        assert!(matches!(
            call(
                "clamped_f64",
                &[Value::Number(Number::from(1))],
                &pool,
                builtins
            ),
            Err(RuntimeError::Message(_))
        ));
        assert!(matches!(
            call("Some", &[], &pool, builtins),
            Err(RuntimeError::Message(_))
        ));
        assert!(matches!(
            call("None", &[Value::Number(Number::from(1))], &pool, builtins),
            Err(RuntimeError::Message(_))
        ));
    }
}
