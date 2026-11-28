//! Register-form index/slice helpers shared by the dispatch loop and `step_vm`.

use prima_core::{Number, Value};

use super::Frame;
use crate::builtins::Builtin;
use crate::error::RuntimeError;
use crate::eval::{EnvRef, Evaluator, Function};
use crate::vm::error::VmError;
use crate::vm::error::VmLimit;

pub(super) fn reg_index_read(
    ev: &mut Evaluator,
    frame: &Frame,
    arr: u16,
    idx: u16,
) -> Result<Value, RuntimeError> {
    if let (Some(Value::Array(a)), Some(Value::Number(Number::Small(i)))) =
        (frame.slots.get(arr as usize), frame.slots.get(idx as usize))
    {
        let i = *i;
        let len = a.len();
        return match resolve_small_index(len, i).and_then(|r| a.get(r)) {
            Some(v) => Ok(v),
            None => Err(crate::error::attach_span(
                RuntimeError::IndexOutOfBounds(format!("index {i} (length {len})")),
                frame_span(frame),
            )),
        };
    }
    let base = frame.slots.get(arr as usize).cloned().unwrap_or(Value::Nil);
    let index = frame.slots.get(idx as usize).cloned().unwrap_or(Value::Nil);
    crate::vm::helpers::vm_index(ev, base, index)
        .map_err(|e| crate::error::attach_span(e, frame_span(frame)))
}

/// Fast array index store for the register-form ops (spec §11.3): the array+`Small`-index case
/// normalizes and writes in place (copy-on-write when the handle is aliased); every other shape
/// delegates to `vm_index_store`; a non-array receiver is a VM capability limit (AST fallback).
#[inline]
pub(super) fn reg_index_store(
    ev: &mut Evaluator,
    frame: &mut Frame,
    arr: u16,
    idx: u16,
    value: Value,
) -> Result<(), VmError> {
    let span = frame_span(frame);
    let index = frame.slots.get(idx as usize).cloned().unwrap_or(Value::Nil);
    match (frame.slots.get_mut(arr as usize), index) {
        (None, _) => Err(VmLimit::InvalidLocalSlot.into()),
        (Some(Value::Array(a)), Value::Number(Number::Small(i))) => {
            let len = a.len();
            match resolve_small_index(len, i) {
                Some(r) => {
                    a.with_mut(|items| items[r] = value);
                    Ok(())
                }
                None => Err(crate::error::attach_span(
                    RuntimeError::IndexOutOfBounds(format!("index {i} (length {len})")),
                    span,
                )
                .into()),
            }
        }
        (Some(slot), index) => {
            crate::vm::helpers::vm_index_store(ev, slot, index, value)
                .map_err(|e| crate::error::attach_span(e, span))?;
            Ok(())
        }
    }
}

/// Array store with an already-extracted small integer index: avoids cloning the index `Value`
/// (the common `A[j] = v` shape, spec §11.3).
#[inline]
pub(super) fn reg_index_store_small(
    ev: &mut Evaluator,
    frame: &mut Frame,
    arr: u16,
    i: i64,
    value: Value,
) -> Result<(), VmError> {
    let span = frame_span(frame);
    match frame.slots.get_mut(arr as usize) {
        Some(Value::Array(a)) => {
            let len = a.len();
            match resolve_small_index(len, i) {
                Some(r) => {
                    a.with_mut(|items| items[r] = value);
                    Ok(())
                }
                None => Err(crate::error::attach_span(
                    RuntimeError::IndexOutOfBounds(format!("index {i} (length {len})")),
                    span,
                )
                .into()),
            }
        }
        // A `Dict` binding (or any other mutable indexed value) keeps the general path.
        Some(other) => {
            crate::vm::helpers::vm_index_store(ev, other, Value::Number(Number::Small(i)), value)
                .map_err(|e| crate::error::attach_span(e, span))?;
            Ok(())
        }
        None => Err(VmLimit::InvalidLocalSlot.into()),
    }
}

/// Splice `rhs` into `target[lo..hi]` (spec §11.3): a `Nil` bound means omitted (0 / length); the
/// array is mutated in place (copy-on-write when its handle is aliased). Mirrors the AST's
/// `slice_bounds` clamping.
pub(super) fn slice_store_value(
    target: &mut Value,
    lo: Value,
    hi: Value,
    rhs: Value,
) -> Result<(), RuntimeError> {
    let Value::Array(a) = target else {
        return Err(RuntimeError::InvalidIndex {
            problem: crate::error::IndexProblem::NotAnArray {
                got: crate::eval::value_type_name(target),
            },
        });
    };
    let len = a.len();
    let len_i = len as i64;
    let bound = |v: &Value, default: i64| -> Result<i64, RuntimeError> {
        match v {
            Value::Nil => Ok(default),
            Value::Number(n) => n.as_i64().ok_or_else(|| RuntimeError::InvalidIndex {
                problem: crate::error::IndexProblem::NotInteger {
                    what: "slice bound",
                },
            }),
            _ => Err(RuntimeError::InvalidIndex {
                problem: crate::error::IndexProblem::NotInteger {
                    what: "slice bound",
                },
            }),
        }
    };
    let raw_lo = bound(&lo, 0)?;
    let raw_hi = bound(&hi, len_i)?;
    let lo = if raw_lo < 0 {
        (len_i + raw_lo).max(0)
    } else {
        raw_lo.min(len_i)
    };
    let hi = if raw_hi < 0 {
        (len_i + raw_hi).max(0)
    } else {
        raw_hi.min(len_i)
    };
    if lo > hi {
        return Err(RuntimeError::InvalidIndex {
            problem: crate::error::IndexProblem::OutOfRange {
                start: lo,
                end: hi,
                length: len,
            },
        });
    }
    let Value::Array(rhs) = rhs else {
        return Err(RuntimeError::InvalidIndex {
            problem: crate::error::IndexProblem::SliceRhsNotArray,
        });
    };
    let items = rhs.to_vec();
    a.with_mut(|buf| {
        buf.splice(lo as usize..hi as usize, items);
    });
    Ok(())
}

/// Normalize a small integer index into `[0, len)`, or `None` when out of range (spec §11.3
/// negative-index normalization).
#[inline]
pub(super) fn resolve_small_index(len: usize, i: i64) -> Option<usize> {
    let r = if i < 0 {
        len.checked_sub(i.unsigned_abs() as usize)
    } else {
        usize::try_from(i).ok()
    };
    r.filter(|&r| r < len)
}

/// Whether `to_f64` currently resolves to the core builtin, cached against the process-wide
/// function-definition epoch: a user `fn to_f64` shadows the builtin, so the register fast path
/// must fall back to calling the user's function (spec §19.5).
pub(super) fn to_f64_builtin(cache: &mut Option<(u64, bool)>, env: &EnvRef) -> bool {
    let epoch = crate::eval::func_epoch();
    if let Some((e, v)) = *cache
        && e == epoch
    {
        return v;
    }
    let is_builtin = matches!(
        env.borrow().get_func("to_f64").as_deref(),
        Some(Function::Builtin(Builtin::Collapse("to_f64")))
    );
    *cache = Some((epoch, is_builtin));
    is_builtin
}

/// Current instruction's source span for diagnostics, read from the frame directly (usable while
/// the frame is mutably borrowed by the dispatch loop).
pub(super) fn frame_span(f: &Frame) -> prima_syntax::Span {
    let l = f
        .chunk
        .lines
        .get(f.ip.saturating_sub(1))
        .copied()
        .unwrap_or(0);
    prima_syntax::Span::new(l, l)
}
