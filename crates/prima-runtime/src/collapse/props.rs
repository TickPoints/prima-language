//! Numeric predicates/accessors and shared collapse-argument helpers (spec §9).

use num_bigint::BigInt;
use num_integer::Integer;
use num_traits::Zero;
use prima_core::collapse::collapse_value;
use prima_core::{BuiltinSymbols, ExprPool, Number, Real, Value};

use crate::error::RuntimeError;

/// `Number` predicates/accessors (spec §9/§18.1), dispatched by method name. Real-domain predicates
/// reject complex values; `abs`/`sqrt` on complex yield the real magnitude/domain error.
pub(super) fn numeric_property(name: &str, n: &Number) -> Result<Value, RuntimeError> {
    use prima_core::Number as N;
    match name {
        "abs" => Ok(Value::Number(match n {
            N::Complex(c) => {
                let r = c.re.to_f64_lossy();
                let i = c.im.to_f64_lossy();
                N::Real(Real::F64((r * r + i * i).sqrt()))
            }
            other => other.abs(),
        })),
        "sqrt" => {
            ensure_real(name, n)?;
            if let Some(s) = n.sqrt() {
                Ok(Value::Number(s))
            } else {
                Ok(Value::Number(N::Real(Real::F64(n.to_f64_lossy().sqrt()))))
            }
        }
        "sign" => {
            ensure_real(name, n)?;
            let x = n.to_f64_lossy();
            Ok(Value::Number(N::Integer(Box::new(BigInt::from(
                if x > 0.0 {
                    1i8
                } else if x < 0.0 {
                    -1i8
                } else {
                    0i8
                },
            )))))
        }
        "is_positive" => {
            ensure_real(name, n)?;
            Ok(Value::Bool(n.to_f64_lossy() > 0.0))
        }
        "is_negative" => {
            ensure_real(name, n)?;
            Ok(Value::Bool(n.to_f64_lossy() < 0.0))
        }
        "is_zero" => Ok(Value::Bool(n.is_zero())),
        "is_integer" => Ok(Value::Bool(n.is_integer_value())),
        "is_rational" => Ok(Value::Bool(matches!(n, N::Integer(_) | N::Rational(_)))),
        "is_real" => Ok(Value::Bool(!n.is_complex())),
        "is_complex" => Ok(Value::Bool(n.is_complex())),
        "is_even" | "is_odd" => {
            let i = n.as_bigint().ok_or_else(|| {
                RuntimeError::Type(format!("`Number.{name}` requires an integer"))
            })?;
            let even = (&i % BigInt::from(2u8)).is_zero();
            Ok(Value::Bool(if name == "is_even" { even } else { !even }))
        }
        "is_finite" => Ok(Value::Bool(number_is_finite(n))),
        "is_nan" => Ok(Value::Bool(number_is_nan(n))),
        "floor" | "ceil" => {
            ensure_real(name, n)?;
            let out = match n {
                N::Integer(_) => n.clone(),
                N::Rational(r) => {
                    let a = r.numer().clone();
                    let b = r.denom().clone();
                    if name == "floor" {
                        N::Integer(Box::new(a.div_mod_floor(&b).0))
                    } else {
                        // ceil(a/b) = -floor(-a/b)
                        N::Integer(Box::new(-(-a).div_mod_floor(&b).0))
                    }
                }
                N::Real(Real::F64(f)) => N::Real(Real::F64(if name == "floor" {
                    f.floor()
                } else {
                    f.ceil()
                })),
                N::Real(Real::F32(f)) => N::Real(Real::F32(if name == "floor" {
                    f.floor()
                } else {
                    f.ceil()
                })),
                other => normalize_property(other),
            };
            Ok(Value::Number(out))
        }
        "round" => {
            ensure_real(name, n)?;
            // Delegate to the core method: it normalizes the fixed-width collapsed layer first, so
            // wide integers (e.g. `u128`) round exactly instead of saturating through an `as i64` cast.
            Ok(Value::Number(n.round()))
        }
        "numerator" | "denominator" => {
            let r = n.as_rational().ok_or_else(|| {
                RuntimeError::Type(format!("`Number.{name}` requires a rational number"))
            })?;
            let part = if name == "numerator" {
                r.numer().clone()
            } else {
                r.denom().clone()
            };
            Ok(Value::Number(N::Integer(Box::new(part))))
        }
        "real" | "imag" => {
            let part = match n {
                N::Complex(c) => {
                    if name == "real" {
                        c.re.clone()
                    } else {
                        c.im.clone()
                    }
                }
                other if name == "real" => other.clone(),
                _ => N::Integer(Box::new(BigInt::from(0))),
            };
            Ok(Value::Number(part))
        }
        "bit_length" => {
            let i = n.as_bigint().ok_or_else(|| {
                RuntimeError::Type("`Number.bit_length` requires an integer".into())
            })?;
            Ok(Value::Number(N::Integer(Box::new(BigInt::from(
                i.bits() as i64
            )))))
        }
        _ => Err(RuntimeError::UnknownNumericMethod {
            name: name.to_string(),
        }),
    }
}

/// Normalize a fixed-width collapsed number back to the exact tower for floor/ceil (spec §6.1).
pub(super) fn normalize_property(n: &Number) -> Number {
    match n {
        Number::I8(i) => Number::Integer(Box::new(BigInt::from(*i))),
        Number::I16(i) => Number::Integer(Box::new(BigInt::from(*i))),
        Number::I32(i) => Number::Integer(Box::new(BigInt::from(*i))),
        Number::I64(i) => Number::Integer(Box::new(BigInt::from(*i))),
        Number::I128(i) => Number::Integer(Box::new(BigInt::from(**i))),
        Number::Isize(i) => Number::Integer(Box::new(BigInt::from(*i))),
        Number::U8(u) => Number::Integer(Box::new(BigInt::from(*u))),
        Number::U16(u) => Number::Integer(Box::new(BigInt::from(*u))),
        Number::U32(u) => Number::Integer(Box::new(BigInt::from(*u))),
        Number::U64(u) => Number::Integer(Box::new(BigInt::from(*u))),
        Number::U128(u) => Number::Integer(Box::new(BigInt::from(**u))),
        Number::Usize(u) => Number::Integer(Box::new(BigInt::from(*u))),
        Number::BigFloat(f) => Number::Real(Real::F64(*f)),
        other => other.clone(),
    }
}

/// Whether a number is finite: real values check their float, everything else is finite.
pub(super) fn number_is_finite(n: &Number) -> bool {
    match n {
        Number::Real(Real::F64(f)) => f.is_finite(),
        Number::Real(Real::F32(f)) => f.is_finite(),
        Number::BigFloat(f) => f.is_finite(),
        Number::Complex(c) => number_is_finite(&c.re) && number_is_finite(&c.im),
        _ => true,
    }
}

/// Whether a number is `NaN`: real values check their float, everything else is not.
pub(super) fn number_is_nan(n: &Number) -> bool {
    match n {
        Number::Real(Real::F64(f)) => f.is_nan(),
        Number::Real(Real::F32(f)) => f.is_nan(),
        Number::BigFloat(f) => f.is_nan(),
        Number::Complex(c) => number_is_nan(&c.re) || number_is_nan(&c.im),
        _ => false,
    }
}

pub(super) fn arity(name: &str, args: &[Value], n: usize) -> Result<(), RuntimeError> {
    crate::args::Args::new(name, args).exact(n)
}

pub(super) fn collapse_arg(
    pool: &ExprPool,
    builtins: &BuiltinSymbols,
    v: &Value,
) -> Result<Number, RuntimeError> {
    collapse_value(pool, builtins, v)
        .or_else(|| numeric_string(v))
        .ok_or_else(|| RuntimeError::Collapse(format!("cannot collapse {v:?} to a number")))
}

/// Parse a numeric string argument (`try_f64("3.14")`, spec §16.3): a `Value::String` is parsed as a number.
pub(super) fn numeric_string(v: &Value) -> Option<Number> {
    match v {
        Value::String(s) => s.trim().parse::<f64>().ok().map(Number::from),
        _ => None,
    }
}

/// Argument to the `try_*` family: `None` means the argument could not be collapsed to a number
/// (reported as a `Result::Err` rather than a runtime error, spec §9.3).
pub(super) fn try_arg(pool: &ExprPool, builtins: &BuiltinSymbols, v: &Value) -> Option<Number> {
    collapse_value(pool, builtins, v).or_else(|| numeric_string(v))
}

/// A complex number cannot collapse to the real numeric domain (spec §9.2 basic collapse operates on reals).
pub(super) fn ensure_real(name: &str, n: &Number) -> Result<(), RuntimeError> {
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
// a range/cast error on failure (`R0002` underflow below the target minimum, else `R0001` overflow),
// `try_*` returns a `Value::Result` instead, and `checked_*` (the ten non-isize/usize targets, spec §9.4)
// also returns a `Value::Result` whose `Err` names the direction. Success always produces the matching
// fixed-width `Number` variant so the collapsed type identity survives (spec §6.1: collapsed types exist
// only after explicit collapse).

/// Direction of an integer range failure (spec §9.2/§9.4): a value below the target's minimum is an
/// underflow (`R0002`), anything else is an overflow (`R0001`).
pub(super) fn int_failure_direction(n: &Number, min: f64) -> &'static str {
    if n.to_f64_lossy() < min {
        "underflow"
    } else {
        "overflow"
    }
}

/// The `RuntimeError` for a `to_*` integer range failure (spec §9.2): below the target minimum is
/// `R0002 underflow`, everything else `R0001 overflow`.
pub(super) fn int_range_error(name: &str, n: &Number, ty: &str, min: f64) -> RuntimeError {
    let message = format!("`{name}`: {n} cannot be represented as {ty}");
    if int_failure_direction(n, min) == "underflow" {
        RuntimeError::Underflow(message)
    } else {
        RuntimeError::Overflow(message)
    }
}
