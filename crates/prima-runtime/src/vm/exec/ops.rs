//! Pure helpers for the VM dispatch loop: jumps, constants, names, operators, and admission.

use std::rc::Rc;

use prima_core::Value;
use prima_syntax::ast::BinOp as AstBinOp;

use super::Frame;
use crate::error::RuntimeError;
use crate::vm::error::{VmError, VmLimit};
use crate::vm::op::{ArithOp, Callee, Chunk, Const, Op, Program as VmProgram};

pub(super) fn jump_target(ip: usize, off: i32) -> usize {
    ((ip as i64) - 1 + off as i64).max(0) as usize
}

pub(super) fn resolve_const(c: Option<Const>) -> Value {
    match c {
        Some(Const::Value(v)) => v,
        Some(Const::Str(s)) => Value::String(s.into()),
        Some(Const::Name(_)) => Value::Nil, // name constants only used by LoadName/CallName/Method
        None => Value::Nil,
    }
}

pub(super) fn resolve_name(chunk: &Chunk, k: u16) -> Result<&str, VmError> {
    match chunk.constants.get(k as usize) {
        Some(Const::Name(s)) => Ok(s),
        _ => Err(VmLimit::InvalidNameConstant.into()),
    }
}

pub(super) fn binop(op: &Op) -> AstBinOp {
    match op {
        Op::Add => AstBinOp::Add,
        Op::Sub => AstBinOp::Sub,
        Op::Mul => AstBinOp::Mul,
        Op::Div => AstBinOp::Div,
        Op::Rem => AstBinOp::Mod,
        Op::Pow => AstBinOp::Pow,
        _ => unreachable!(),
    }
}

/// The `ArithOp` for a stack arithmetic instruction (`Op::Add`…`Op::Rem`).
pub(super) fn arith_of(op: &Op) -> ArithOp {
    match op {
        Op::Add => ArithOp::Add,
        Op::Sub => ArithOp::Sub,
        Op::Mul => ArithOp::Mul,
        Op::Div => ArithOp::Div,
        Op::Rem => ArithOp::Rem,
        _ => unreachable!("arith_of called on a non-arithmetic op"),
    }
}

/// The AST binary operator for a register arithmetic op (the `eval_binary` fallback path).
pub(super) fn arith_binop(op: ArithOp) -> AstBinOp {
    match op {
        ArithOp::Add => AstBinOp::Add,
        ArithOp::Sub => AstBinOp::Sub,
        ArithOp::Mul => AstBinOp::Mul,
        ArithOp::Div => AstBinOp::Div,
        ArithOp::Rem => AstBinOp::Mod,
    }
}

pub(super) fn cmp_binop(op: &Op) -> AstBinOp {
    match op {
        Op::EqCmp => AstBinOp::Eq,
        Op::NeCmp => AstBinOp::Ne,
        Op::Lt => AstBinOp::Lt,
        Op::Le => AstBinOp::Le,
        Op::Gt => AstBinOp::Gt,
        Op::Ge => AstBinOp::Ge,
        _ => unreachable!(),
    }
}

pub(super) fn bool_binop(op: AstBinOp, a: Value, b: Value) -> Result<Value, RuntimeError> {
    match (a, b) {
        (Value::Bool(x), Value::Bool(y)) => Ok(Value::Bool(match op {
            AstBinOp::And => x && y,
            AstBinOp::Or => x || y,
            _ => unreachable!(),
        })),
        _ => Err(RuntimeError::ExpectedBoolean {
            context: crate::error::BooleanContext::LogicalOperands,
        }),
    }
}

/// Cache a resolved call-site callee on the executing chunk (spec §19.5); validated by epoch on
/// every subsequent execution.
pub(super) fn cache_callee(frame: &Frame, site: u16, epoch: u64, callee: Callee) {
    let chunk = &frame.chunk;
    let mut cache = chunk.callee_cache.borrow_mut();
    if cache.len() <= site as usize {
        cache.resize(site as usize + 1, None);
    }
    cache[site as usize] = Some((epoch, callee));
}

/// Wrap a compiled function chunk into a single-entry VM program (spec §19.5): the chunk is the
/// only function, dispatched under the reserved `__entry` name. Recursive self-calls inside the
/// body resolve through the same table, so the whole program is shared across re-entrant calls.
pub(super) fn single_entry_program(chunk: Chunk) -> VmProgram {
    let mut names = rustc_hash::FxHashMap::default();
    names.insert("__entry".to_string(), 0u32);
    VmProgram {
        root: Rc::new(Chunk::new()),
        functions: vec![Rc::new(chunk)],
        names,
    }
}

/// Outcome of an attempted bytecode-VM call (spec §19.5). `Fallback` covers both "body outside the
/// compiled subset" and a capability-limit fallback to the AST, so a caller (or a test) can never
/// mistake a silent interpretation for a VM execution.
pub(crate) enum VmOutcome {
    Hit(Value),
    Fallback,
}
