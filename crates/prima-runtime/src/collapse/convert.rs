//! Basic/try/checked/clamped/rounded/truncated collapse families (spec §9.2–9.6).

use prima_core::{BuiltinSymbols, ExprPool, Number, Real, Value};

use super::props::{arity, collapse_arg, ensure_real, int_failure_direction, int_range_error};
use crate::error::RuntimeError;

macro_rules! int_collapse_fns {
    // `to_`/`try_`/`checked_` (the ten fixed integer targets, spec §9.2–9.4).
    // `$wrap` builds the `Number`: the inline variants construct directly, the boxed
    // `I128`/`U128` variants box their payload (see `prima_core::number::Number`).
    ($to_fn:ident, $try_fn:ident, $checked_fn:ident, $wrap:expr, $as:ident, $ty:ty) => {
        pub(super) fn $to_fn(name: &str, n: &Number) -> Result<Value, RuntimeError> {
            ensure_real(name, n)?;
            match n.$as() {
                Some(v) => Ok(Value::Number(($wrap)(v))),
                None => Err(int_range_error(name, n, stringify!($ty), <$ty>::MIN as f64)),
            }
        }

        pub(super) fn $try_fn(name: &str, n: Option<Number>) -> Value {
            let Some(n) = n else {
                return Value::Result(Err(Box::new(format!(
                    "`{name}`: cannot collapse argument to a number"
                ))));
            };
            if n.is_complex() {
                return Value::Result(Err(Box::new(format!(
                    "`{name}`: cannot convert complex value {n} to {}",
                    stringify!($ty)
                ))));
            }
            match n.$as() {
                Some(v) => Value::Result(Ok(Box::new(Value::Number(($wrap)(v))))),
                None => Value::Result(Err(Box::new(format!(
                    "{}: `{name}`: {n} cannot be represented as {}",
                    int_failure_direction(&n, <$ty>::MIN as f64),
                    stringify!($ty)
                )))),
            }
        }

        pub(super) fn $checked_fn(name: &str, n: &Number) -> Result<Value, RuntimeError> {
            ensure_real(name, n)?;
            match n.$as() {
                Some(v) => Ok(Value::Result(Ok(Box::new(Value::Number(($wrap)(v)))))),
                None => Ok(Value::Result(Err(Box::new(format!(
                    "{}: `{name}`: {n} cannot be represented as {}",
                    int_failure_direction(n, <$ty>::MIN as f64),
                    stringify!($ty)
                ))))),
            }
        }
    };
    // `to_`/`try_` only (`isize`/`usize` have no `checked_` form, spec §9.4).
    ($to_fn:ident, $try_fn:ident, $wrap:expr, $as:ident, $ty:ty) => {
        pub(super) fn $to_fn(name: &str, n: &Number) -> Result<Value, RuntimeError> {
            ensure_real(name, n)?;
            match n.$as() {
                Some(v) => Ok(Value::Number(($wrap)(v))),
                None => Err(int_range_error(name, n, stringify!($ty), <$ty>::MIN as f64)),
            }
        }

        pub(super) fn $try_fn(name: &str, n: Option<Number>) -> Value {
            let Some(n) = n else {
                return Value::Result(Err(Box::new(format!(
                    "`{name}`: cannot collapse argument to a number"
                ))));
            };
            if n.is_complex() {
                return Value::Result(Err(Box::new(format!(
                    "`{name}`: cannot convert complex value {n} to {}",
                    stringify!($ty)
                ))));
            }
            match n.$as() {
                Some(v) => Value::Result(Ok(Box::new(Value::Number(($wrap)(v))))),
                None => Value::Result(Err(Box::new(format!(
                    "{}: `{name}`: {n} cannot be represented as {}",
                    int_failure_direction(&n, <$ty>::MIN as f64),
                    stringify!($ty)
                )))),
            }
        }
    };
}

int_collapse_fns!(to_i8, try_i8, checked_i8, Number::I8, as_i8, i8);
int_collapse_fns!(to_i16, try_i16, checked_i16, Number::I16, as_i16, i16);
int_collapse_fns!(to_i32, try_i32, checked_i32, Number::I32, as_i32, i32);
int_collapse_fns!(to_i64, try_i64, checked_i64, Number::I64, as_i64, i64);
int_collapse_fns!(
    to_i128,
    try_i128,
    checked_i128,
    |v: i128| Number::I128(Box::new(v)),
    as_i128,
    i128
);
int_collapse_fns!(to_u8, try_u8, checked_u8, Number::U8, as_u8, u8);
int_collapse_fns!(to_u16, try_u16, checked_u16, Number::U16, as_u16, u16);
int_collapse_fns!(to_u32, try_u32, checked_u32, Number::U32, as_u32, u32);
int_collapse_fns!(to_u64, try_u64, checked_u64, Number::U64, as_u64, u64);
int_collapse_fns!(
    to_u128,
    try_u128,
    checked_u128,
    |v: u128| Number::U128(Box::new(v)),
    as_u128,
    u128
);
int_collapse_fns!(to_isize, try_isize, Number::Isize, as_isize, isize);
int_collapse_fns!(to_usize, try_usize, Number::Usize, as_usize, usize);

// ---- Basic collapse (spec §9.2): failure is a runtime error ----

pub(super) fn to_f32(name: &str, n: &Number) -> Result<Value, RuntimeError> {
    ensure_real(name, n)?;
    Ok(Value::Number(Number::Real(Real::F32(
        n.to_f64_lossy() as f32
    ))))
}

pub(super) fn to_f64(name: &str, n: &Number) -> Result<Value, RuntimeError> {
    ensure_real(name, n)?;
    Ok(Value::Number(Number::Real(Real::F64(n.to_f64_lossy()))))
}

pub(super) fn to_bigint(name: &str, n: &Number) -> Result<Value, RuntimeError> {
    match n.as_bigint() {
        Some(b) => Ok(Value::Number(Number::Integer(Box::new(b)))),
        None => Err(RuntimeError::Collapse(format!(
            "`{name}`: {n} is not an integer"
        ))),
    }
}

pub(super) fn to_rational(name: &str, n: &Number) -> Result<Value, RuntimeError> {
    match n.as_rational() {
        Some(r) => Ok(Value::Number(Number::Rational(Box::new(r)))),
        None => Err(RuntimeError::Collapse(format!(
            "`{name}`: {n} cannot be represented as a rational"
        ))),
    }
}

/// `to_bigfloat` (spec §9.2) is a degenerate implementation that preserves precision: the number is returned unchanged (exact values are kept, `Real` stays f64).
/// Arbitrary-precision floats (BigFloat) are deferred to a later release.
pub(super) fn to_bigfloat(n: &Number) -> Result<Value, RuntimeError> {
    Ok(Value::Number(n.clone()))
}

pub(super) fn to_complex(n: &Number) -> Result<Value, RuntimeError> {
    if n.is_complex() {
        return Ok(Value::Number(n.clone()));
    }
    Ok(Value::Number(Number::from_complex(
        n.clone(),
        Number::from(0),
    )))
}

// ---- Try collapse (spec §9.3): result wrapped in a `Result`, no runtime error is raised ----

pub(super) fn try_f32(name: &str, n: Option<Number>) -> Value {
    let Some(n) = n else {
        return Value::Result(Err(Box::new(format!(
            "`{name}`: cannot collapse argument to a number"
        ))));
    };
    if n.is_complex() {
        return Value::Result(Err(Box::new(format!(
            "`{name}`: cannot convert complex value {n} to f32"
        ))));
    }
    Value::Result(Ok(Box::new(Value::Number(Number::Real(Real::F32(
        n.to_f64_lossy() as f32,
    ))))))
}

pub(super) fn try_f64(name: &str, n: Option<Number>) -> Value {
    let Some(n) = n else {
        return Value::Result(Err(Box::new(format!(
            "`{name}`: cannot collapse argument to a number"
        ))));
    };
    if n.is_complex() {
        return Value::Result(Err(Box::new(format!(
            "`{name}`: cannot convert complex value {n} to f64"
        ))));
    }
    Value::Result(Ok(Box::new(Value::Number(Number::Real(Real::F64(
        n.to_f64_lossy(),
    ))))))
}

pub(super) fn try_bigint(name: &str, n: Option<Number>) -> Value {
    let Some(n) = n else {
        return Value::Result(Err(Box::new(format!(
            "`{name}`: cannot collapse argument to a number"
        ))));
    };
    match n.as_bigint() {
        Some(b) => Value::Result(Ok(Box::new(Value::Number(Number::Integer(Box::new(b)))))),
        None => Value::Result(Err(Box::new(format!("`{name}`: {n} is not an integer")))),
    }
}

pub(super) fn try_rational(name: &str, n: Option<Number>) -> Value {
    let Some(n) = n else {
        return Value::Result(Err(Box::new(format!(
            "`{name}`: cannot collapse argument to a number"
        ))));
    };
    match n.as_rational() {
        Some(r) => Value::Result(Ok(Box::new(Value::Number(Number::Rational(Box::new(r)))))),
        None => Value::Result(Err(Box::new(format!(
            "`{name}`: {n} cannot be represented as a rational"
        )))),
    }
}

pub(super) fn try_complex(name: &str, n: Option<Number>) -> Value {
    let Some(n) = n else {
        return Value::Result(Err(Box::new(format!(
            "`{name}`: cannot collapse argument to a number"
        ))));
    };
    if n.is_complex() {
        return Value::Result(Ok(Box::new(Value::Number(n))));
    }
    Value::Result(Ok(Box::new(Value::Number(Number::from_complex(
        n,
        Number::from(0),
    )))))
}

// ---- Checked collapse (spec §9.4): check for overflow, return a `Result` ----

/// `checked_add`/`checked_mul` (spec §9.4): evaluate via f64, then verify the result can be exactly represented back in i64, otherwise `Err`.
pub(super) fn checked_binary(
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
        let direction = if v < i64::MIN as f64 {
            "underflow"
        } else {
            "overflow"
        };
        return Ok(Value::Result(Err(Box::new(format!(
            "{direction}: `{name}`: result {v} cannot be represented as i64"
        )))));
    }
    Ok(Value::Result(Ok(Box::new(Value::Number(Number::from(vi))))))
}

// ---- Clamped collapse (spec §9.5): force into range ----

/// Shared 3-argument `clamped_<ty>(x, min, max)` dispatch: collapse all three args to numbers, then clamp.
pub(super) fn clamped_3(
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
    // `$wrap` builds the `Number`: inline variants construct directly, boxed `I128`/`U128` box
    // their payload (see `prima_core::number::Number`).
    ($fn:ident, $wrap:expr, $ty:ty) => {
        pub(super) fn $fn(
            name: &str,
            x: &Number,
            min: &Number,
            max: &Number,
        ) -> Result<Value, RuntimeError> {
            ensure_real(name, x)?;
            ensure_real(name, min)?;
            ensure_real(name, max)?;
            let c = x
                .to_f64_lossy()
                .clamp(min.to_f64_lossy(), max.to_f64_lossy());
            Ok(Value::Number(($wrap)(c as $ty)))
        }
    };
}

// The unsigned 3-argument forms clamp to `[min, max]` (spec §9.5 `clamped_u8(x, min, max)`); `clamped_u64` keeps the
// single-argument `[0, u64::MAX]` form from the earlier implementation (spec §9.5 lists both).
int_clamped_fns!(clamped_i8, Number::I8, i8);
int_clamped_fns!(clamped_i16, Number::I16, i16);
int_clamped_fns!(clamped_i32, Number::I32, i32);
int_clamped_fns!(clamped_i64, Number::I64, i64);
int_clamped_fns!(clamped_i128, |v: i128| Number::I128(Box::new(v)), i128);
int_clamped_fns!(clamped_u8, Number::U8, u8);
int_clamped_fns!(clamped_u16, Number::U16, u16);
int_clamped_fns!(clamped_u32, Number::U32, u32);
int_clamped_fns!(clamped_u128, |v: u128| Number::U128(Box::new(v)), u128);

/// `clamped_u64` (spec §9.5): single argument, clamped to `[0, u64::MAX]` (negative → 0, out of range → `u64::MAX`).
pub(super) fn clamped_u64(name: &str, n: &Number) -> Result<Value, RuntimeError> {
    ensure_real(name, n)?;
    let c = n.to_f64_lossy().clamp(0.0, u64::MAX as f64);
    Ok(Value::Number(Number::U64(c as u64)))
}

pub(super) fn clamped_f32(
    name: &str,
    x: &Number,
    min: &Number,
    max: &Number,
) -> Result<Value, RuntimeError> {
    ensure_real(name, x)?;
    ensure_real(name, min)?;
    ensure_real(name, max)?;
    let v = x
        .to_f64_lossy()
        .clamp(min.to_f64_lossy(), max.to_f64_lossy());
    Ok(Value::Number(Number::Real(Real::F32(v as f32))))
}

pub(super) fn clamped_f64(
    name: &str,
    x: &Number,
    min: &Number,
    max: &Number,
) -> Result<Value, RuntimeError> {
    ensure_real(name, x)?;
    ensure_real(name, min)?;
    ensure_real(name, max)?;
    Ok(Value::Number(
        x.clamped_f64(min.to_f64_lossy(), max.to_f64_lossy()),
    ))
}

// ---- Rounding collapse (spec §9.6) ----

/// Validate the digit count of the `rounded_*` family (spec §9.6): it must be an integer that fits
/// `i32`, the exponent domain of `10f64.powi`. An out-of-range count is rejected with a clear
/// `Overflow` error instead of silently wrapping through an `as i32` cast to a bogus exponent.
pub(super) fn rounded_digit_count(name: &str, digits: &Number) -> Result<i32, RuntimeError> {
    let d = digits.as_i64().ok_or_else(|| {
        RuntimeError::Type(format!(
            "`{name}` expects an integer digit count, got {digits}"
        ))
    })?;
    i32::try_from(d).map_err(|_| {
        RuntimeError::Overflow(format!(
            "`{name}`: digit count {d} is out of the supported range"
        ))
    })
}

pub(super) fn rounded_f64(name: &str, x: &Number, digits: &Number) -> Result<Value, RuntimeError> {
    ensure_real(name, x)?;
    let d = rounded_digit_count(name, digits)?;
    x.rounded_digits(i64::from(d))
        .map(Value::Number)
        .ok_or_else(|| {
            RuntimeError::Overflow(format!("`{name}`: cannot round {x} to {d} decimal places"))
        })
}

pub(super) fn rounded_f32(name: &str, x: &Number, digits: &Number) -> Result<Value, RuntimeError> {
    ensure_real(name, x)?;
    let d = rounded_digit_count(name, digits)?;
    let n = x.rounded_digits(i64::from(d)).ok_or_else(|| {
        RuntimeError::Overflow(format!("`{name}`: cannot round {x} to {d} decimal places"))
    })?;
    Ok(Value::Number(Number::Real(Real::F32(
        n.to_f64_lossy() as f32
    ))))
}

pub(super) fn rounded_i32(name: &str, n: &Number) -> Result<Value, RuntimeError> {
    ensure_real(name, n)?;
    Ok(Value::Number(n.round()))
}

pub(super) fn truncated_i32(name: &str, n: &Number) -> Result<Value, RuntimeError> {
    ensure_real(name, n)?;
    Ok(Value::Number(n.truncate()))
}

// ---- `unwrap` family: unwrap the `Result`/`Option` of safe collapse (spec §9.3, §16.3) ----
