//! Foundation for explicit symbolic → numeric collapse (spec §9).
//!
//! Numeric evaluation only lowers determinate nodes to `Number`: built-in constants (`\e`/`\pi`),
//! exact nodes, unary numeric operators, and the four arithmetic operations plus powers. It returns `None`
//! for unresolved symbols or unsupported operators; the caller (the collapse function family, spec §9.2)
//! decides whether to raise an error or keep the symbolic form.

use crate::builtins::BuiltinSymbols;
use crate::expr_pool::{ExprData, ExprId, ExprPool};
use crate::number::{Number, Real};
use crate::value::Value;

/// Collapse a `Value` to a number (spec §9): `Number` is returned as-is, `Expr` is evaluated numerically,
/// everything else (including `Undefined`/`Indeterminate`/arrays) yields `None`.
pub fn collapse_value(pool: &ExprPool, builtins: &BuiltinSymbols, v: &Value) -> Option<Number> {
    match v {
        Value::Number(n) => Some(n.clone()),
        Value::Expr(id) => numeric_value(pool, builtins, *id),
        _ => None,
    }
}

/// ExprDAG → number (spec §9).
pub fn numeric_value(pool: &ExprPool, builtins: &BuiltinSymbols, id: ExprId) -> Option<Number> {
    match pool.get(id)? {
        ExprData::Symbol(s) => symbol_value(builtins, s),
        ExprData::Integer(_) | ExprData::Rational(_) | ExprData::Real(_) => pool.const_number(id),
        ExprData::Add(items) => fold(
            items.iter().copied(),
            Number::from(0),
            |a, b| a + b,
            pool,
            builtins,
        ),
        ExprData::Mul(items) => fold(
            items.iter().copied(),
            Number::from(1),
            |a, b| a * b,
            pool,
            builtins,
        ),
        ExprData::Pow { base, exp } => {
            let b = numeric_value(pool, builtins, base)?;
            let e = numeric_value(pool, builtins, exp)?;
            match b.pow(&e) {
                Some(r) => Some(r),
                None => Some(Number::Real(Real::F64(
                    b.to_f64_lossy().powf(e.to_f64_lossy()),
                ))),
            }
        }
        ExprData::Apply { f, args } => apply_value(pool, builtins, f, &args),
        ExprData::Indeterminate(_) => None,
    }
}

fn fold(
    items: impl Iterator<Item = ExprId>,
    init: Number,
    f: impl Fn(Number, Number) -> Number,
    pool: &ExprPool,
    builtins: &BuiltinSymbols,
) -> Option<Number> {
    let mut acc = init;
    for it in items {
        acc = f(acc, numeric_value(pool, builtins, it)?);
    }
    Some(acc)
}

fn symbol_value(builtins: &BuiltinSymbols, s: crate::symbol::SymbolId) -> Option<Number> {
    if s == builtins.e {
        Some(Number::Real(Real::F64(std::f64::consts::E)))
    } else if s == builtins.pi {
        Some(Number::Real(Real::F64(std::f64::consts::PI)))
    } else {
        None
    }
}

fn apply_value(
    pool: &ExprPool,
    builtins: &BuiltinSymbols,
    f: ExprId,
    args: &[ExprId],
) -> Option<Number> {
    if args.len() != 1 {
        return None;
    }
    let arg = numeric_value(pool, builtins, args[0])?;
    if f == pool.symbol(builtins.sqrt) {
        return match arg.sqrt() {
            Some(r) => Some(r),
            None => Some(Number::Real(Real::F64(arg.to_f64_lossy().sqrt()))),
        };
    }
    if f == pool.symbol(builtins.abs) {
        return Some(arg.abs());
    }
    let x = arg.to_f64_lossy();
    let v = if f == pool.symbol(builtins.exp) {
        Some(x.exp())
    } else if f == pool.symbol(builtins.log) || f == pool.symbol(builtins.ln) {
        Some(x.ln())
    } else if f == pool.symbol(builtins.sin) {
        Some(x.sin())
    } else if f == pool.symbol(builtins.cos) {
        Some(x.cos())
    } else if f == pool.symbol(builtins.tan) {
        Some(x.tan())
    } else {
        None
    }?;
    Some(Number::Real(Real::F64(v)))
}
