//! Translation of a numeric scalar `ExprDAG` into `Bytecode` (spec §19.2).
//!
//! `dag_to_bytecode` returns `None` when the expression is not numeric-scalar compilable: complex
//! numbers, symbols that are neither parameters nor built-in constants, applications of non-math
//! builtins, or any unknown node. Free symbols are matched against `params` by name; built-in
//! constants (`\pi`, `\e`, …) are folded to their `f64` value via `BuiltinSymbols`.

use prima_core::expr_pool::{ExprData, ExprId};
use prima_core::symbol::SymbolTable;
use prima_core::{BuiltinSymbols, ExprPool};

use crate::bytecode::{Bytecode, Op};
use crate::engine::CompiledScalar;
use std::sync::Arc;

/// Translate a numeric scalar expression DAG into bytecode, or `None` if it cannot be compiled
/// (non-numeric node, unknown symbol, non-math application, …).
pub fn dag_to_bytecode(
    pool: &ExprPool,
    builtins: &BuiltinSymbols,
    expr: ExprId,
    params: &[String],
) -> Option<Bytecode> {
    let mut ops = Vec::new();
    if emit_expr(pool, builtins, expr, params, &mut ops) {
        Some(Bytecode(ops))
    } else {
        None
    }
}

/// Recursively lower `expr` into `ops`, returning `false` on the first node that is not
/// numeric-scalar compilable.
fn emit_expr(
    pool: &ExprPool,
    builtins: &BuiltinSymbols,
    expr: ExprId,
    params: &[String],
    ops: &mut Vec<Op>,
) -> bool {
    match pool.get(expr) {
        Some(ExprData::Integer(_) | ExprData::Rational(_) | ExprData::Real(_)) => {
            match pool.const_number(expr) {
                Some(n) => {
                    ops.push(Op::Const(n.to_f64_lossy()));
                    true
                }
                None => false,
            }
        }
        Some(ExprData::Symbol(s)) => emit_symbol(s, builtins, params, ops),
        Some(ExprData::Add(items)) => emit_nary(pool, builtins, &items, params, ops, Op::Add),
        Some(ExprData::Mul(items)) => emit_nary(pool, builtins, &items, params, ops, Op::Mul),
        Some(ExprData::Pow { base, exp }) => {
            emit_expr(pool, builtins, base, params, ops)
                && emit_expr(pool, builtins, exp, params, ops)
                && {
                    ops.push(Op::Pow);
                    true
                }
        }
        Some(ExprData::Apply { f, args }) => emit_apply(pool, builtins, f, &args, params, ops),
        Some(ExprData::Indeterminate(_)) => false,
        None => false,
    }
}

/// Lower an n-ary `Add`/`Mul` node: push every item, then fold them left-to-right with `op`
/// (the top of the stack after pushing item k is the fold of items 0..=k).
fn emit_nary(
    pool: &ExprPool,
    builtins: &BuiltinSymbols,
    items: &[ExprId],
    params: &[String],
    ops: &mut Vec<Op>,
    op: Op,
) -> bool {
    if items.is_empty() {
        return false;
    }
    for &it in items {
        if !emit_expr(pool, builtins, it, params, ops) {
            return false;
        }
    }
    for _ in 1..items.len() {
        ops.push(op);
    }
    true
}

/// A `Symbol` compiles to a parameter read (matched by name) or a built-in constant fold;
/// any other free symbol is not compilable.
fn emit_symbol(
    s: prima_core::symbol::SymbolId,
    builtins: &BuiltinSymbols,
    params: &[String],
    ops: &mut Vec<Op>,
) -> bool {
    let name = SymbolTable::global().name(s);
    if let Some(name) = name
        && let Some(idx) = params.iter().position(|p| *p == name)
    {
        ops.push(Op::Param(idx as u8));
        return true;
    }
    let c = if s == builtins.e {
        Some(std::f64::consts::E)
    } else if s == builtins.pi {
        Some(std::f64::consts::PI)
    } else if s == builtins.tau {
        Some(std::f64::consts::TAU)
    } else if s == builtins.inf {
        Some(f64::INFINITY)
    } else if s == builtins.gamma {
        Some(0.577_215_664_901_532_9)
    } else if s == builtins.phi {
        Some(1.618_033_988_749_895)
    } else {
        None
    };
    match c {
        Some(x) => {
            ops.push(Op::Const(x));
            true
        }
        None => false,
    }
}

/// An `Apply` compiles only when it is a unary application of a math builtin; the argument is
/// lowered first and the op is pushed afterwards.
fn emit_apply(
    pool: &ExprPool,
    builtins: &BuiltinSymbols,
    f: ExprId,
    args: &[ExprId],
    params: &[String],
    ops: &mut Vec<Op>,
) -> bool {
    if args.len() != 1 {
        return false;
    }
    let f_sym = match pool.get(f) {
        Some(ExprData::Symbol(s)) => s,
        _ => return false,
    };
    let op = if f_sym == builtins.sin {
        Some(Op::Sin)
    } else if f_sym == builtins.cos {
        Some(Op::Cos)
    } else if f_sym == builtins.tan {
        Some(Op::Tan)
    } else if f_sym == builtins.exp {
        Some(Op::Exp)
    } else if f_sym == builtins.ln || f_sym == builtins.log {
        // `log` and `ln` are distinct symbol ids that both mean the natural logarithm.
        Some(Op::Ln)
    } else if f_sym == builtins.sqrt {
        Some(Op::Sqrt)
    } else if f_sym == builtins.abs {
        Some(Op::Abs)
    } else {
        None
    };
    match op {
        Some(op) => {
            emit_expr(pool, builtins, args[0], params, ops) && {
                ops.push(op);
                true
            }
        }
        None => false,
    }
}

/// Convenience: `dag_to_bytecode` + `engine::compile_bytecode`. Returns `None` when the expression
/// is not compilable or cranelift fails.
pub fn compile_scalar(
    pool: &ExprPool,
    builtins: &BuiltinSymbols,
    expr: ExprId,
    params: &[String],
) -> Option<Arc<CompiledScalar>> {
    let bc = dag_to_bytecode(pool, builtins, expr, params)?;
    crate::engine::compile_bytecode(&bc, params.len())
}
