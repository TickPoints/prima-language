//! Collapse function family (spec §9.2–9.6): `to_/try_/checked_/clamped_/rounded_/truncated_` plus the `unwrap` family,
//! string helpers (`to_string`/`concat`, §18.1; `format` was removed in v2.2) and the `Option`/`Result` constructors (spec §4.4).
//!
//! The naming scheme encodes safety properties (spec §9.1): the basic forms raise a runtime error on failure (§9.2),
//! the try forms return a `Result` (§9.3), the checked forms check for overflow (§9.4), the clamped forms force a range (§9.5), and the rounding forms round in the specified way (§9.6).

use prima_core::{BuiltinSymbols, ExprPool, Value};

use crate::error::RuntimeError;

mod convert;
mod props;
mod wrappers;

use convert::*;
use props::*;
use wrappers::*;

/// Invoke a collapse builtin function (spec §9.2–9.6, §18.1, §4.4).
///
/// An unknown name returns `RuntimeError::UnknownCollapseFunction`; wrong arity returns
/// `RuntimeError::Arity`; numeric arguments are first collapsed
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
        // ---- `Option`/`Result` predicates and accessors (spec §16.3/§18.1) ----
        "is_some" => is_some(args),
        "is_none" => is_none(args),
        "is_ok" => is_ok(args),
        "is_err" => is_err(args),
        "value_or" => value_or(args),
        // ---- Option/Result constructors (spec §4.4) ----
        "Some" => some(args),
        "None" => none(args),
        "Ok" => ok(args),
        "Err" => err_builtin(args),
        // ---- string/format helpers (spec §18.1); `format` was removed in v2.2 (f-strings) ----
        "to_string" => to_string_call(args, pool),
        "concat" => concat_call(args, pool),
        // ---- `Number` predicates and accessors (spec §9/§18.1): `x.is_integer()`, `x.abs()`, ... ----
        "abs" | "sign" | "floor" | "ceil" | "round" | "sqrt" | "numerator" | "denominator"
        | "real" | "imag" | "bit_length" | "is_integer" | "is_rational" | "is_real"
        | "is_complex" | "is_positive" | "is_negative" | "is_zero" | "is_even" | "is_odd"
        | "is_finite" | "is_nan" => {
            arity(name, args, 1)?;
            numeric_property(name, &collapse_arg(pool, builtins, &args[0])?)
        }
        _ => Err(RuntimeError::UnknownCollapseFunction {
            name: name.to_string(),
        }),
    }
}

#[cfg(test)]
mod tests;
