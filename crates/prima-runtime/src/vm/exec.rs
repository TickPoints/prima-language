//! Bytecode VM executor (spec §19.5).
//!
//! A call/return stack machine over `Value`-typed slots. Each `Frame` references a shared `Rc<Chunk>`
//! (which owns its constant pool), a program counter, and a slot array. The VM owns a frame stack
//! and the operand stack; numeric/comparison fast paths compute directly on `Number` (spec §6.1),
//! while every other value-producing command delegates to the `Evaluator` (arithmetic via
//! `eval_binary`, index reads/writes, method calls, and calls via `apply_function`), so VM results
//! equal AST-interpreter results by construction. Control flow and local slot management are
//! handled natively by the dispatch loop.
//!
//! The VM is entered through [`Evaluator::run_vm`]. When compilation rejects a construct, the
//! program runs on the AST interpreter (the authoritative path). Compiled chunks are cached per
//! function definition on the defining environment (spec §19.5), so repeated calls never recompile.
//! Runtime errors propagate to the caller without an AST re-run; only VM *capability limits*
//! (see [`vm_limit`]) fall back to the AST interpreter.

use std::rc::Rc;

use prima_core::{Number, Real, Value};
use prima_syntax::ast::BinOp as AstBinOp;

use super::op::{ArithOp, Callee, Chunk, Const, Imm, Op, Program as VmProgram};
use crate::builtins::Builtin;
use crate::error::RuntimeError;
use crate::eval::{EnvRef, Evaluator, Function};

/// Message prefix marking a VM capability limit (spec §19.5): the compiled subset cannot execute
/// the construct, so the caller falls back to the AST interpreter (the authoritative path) instead
/// of surfacing the error. Distinguished from genuine runtime errors, which propagate to the
/// caller directly (an AST re-run would duplicate side effects).
pub(crate) const VM_LIMIT_PREFIX: &str = "VM: ";

/// Build a VM capability-limit error (see [`VM_LIMIT_PREFIX`]).
pub(crate) fn vm_limit(msg: impl std::fmt::Display) -> RuntimeError {
    RuntimeError::Message(format!("{VM_LIMIT_PREFIX}{msg}"))
}

/// Whether an error is a VM capability limit (fall back to the AST) rather than a genuine runtime
/// error (propagate).
pub(crate) fn is_vm_limit(e: &RuntimeError) -> bool {
    matches!(e, RuntimeError::Message(m) if m.starts_with(VM_LIMIT_PREFIX))
}

/// A single active call frame: the chunk being executed, its program counter, and its slot array.
#[derive(Debug)]
pub struct Frame {
    chunk: Rc<Chunk>,
    ip: usize,
    slots: Vec<Value>,
}

/// The VM: a stack of frames plus the operand stack shared across frames. Chunks and the dispatch
/// table are borrowed from the `VmProgram` being executed, so entering the VM never copies bytecode.
pub struct Vm<'a> {
    frames: Vec<Frame>,
    stack: Vec<Value>,
    /// Function-name → chunk index dispatch table.
    table: &'a rustc_hash::FxHashMap<String, u32>,
    functions: &'a [Rc<Chunk>],
    /// `(epoch, is_builtin)` cache for the `to_f64` register fast path: re-resolved whenever the
    /// function-definition epoch changes, so a user `to_f64` shadow is honored (spec §19.5).
    to_f64_builtin: Option<(u64, bool)>,
}

fn pop(stack: &mut Vec<Value>) -> Value {
    stack.pop().unwrap_or(Value::Nil)
}

/// Split the top `n` values off the stack into an ordered `Vec` (top-most last).
fn split_args(stack: &mut Vec<Value>, n: usize) -> Vec<Value> {
    let start = stack.len().saturating_sub(n);
    stack.split_off(start)
}

impl Evaluator {
    /// Enter the bytecode VM for `program`. With `entry = Some((name, args))` it calls that top-level
    /// function and returns its result; with `entry = None` it runs the root chunk body. Returns an
    /// `Err` for a missing entry or an executing-runtime error.
    pub fn run_vm(
        &mut self,
        env: &EnvRef,
        program: &VmProgram,
        entry: Option<(&str, Vec<Value>)>,
    ) -> Result<Value, crate::error::RuntimeError> {
        // Reuse a pooled frame/stack buffer (spec §19.5): repeated and nested calls avoid
        // reallocating; an early error return simply drops the buffers.
        let mut vm = Vm {
            frames: self.vm_frames_pool.pop().unwrap_or_default(),
            stack: self.vm_stack_pool.pop().unwrap_or_default(),
            table: &program.names,
            functions: &program.functions,
            to_f64_builtin: None,
        };
        vm.frames.clear();
        vm.stack.clear();
        match entry {
            Some((name, args)) => {
                let idx = *vm.table.get(name).ok_or_else(|| {
                    crate::error::RuntimeError::Message(format!("unknown VM entry `{name}`"))
                })?;
                let chunk = Rc::clone(&vm.functions[idx as usize]);
                vm.push_frame(chunk);
                for a in args {
                    vm.stack.push(a);
                }
            }
            None => {
                vm.push_frame(Rc::clone(&program.root));
            }
        }

        // Dispatch loop (no explicit return detection needed: `ReturnValue`/implicit end pops the
        // frame; the result is the last value each returning frame leaves on the stack). The hot
        // instruction set (local load/store, constants, jumps, `Small`-typed arithmetic and
        // comparisons, fused loop forms) is handled inline against the cached top frame: the
        // handlers touch neither the `Evaluator` nor the config/span machinery, so a numeric loop
        // runs at dispatch cost only (spec §19.5). Everything else delegates to `step_vm`, which
        // may switch frames — the outer loop then re-acquires the new top frame.
        'dispatch: while let Some(frame) = vm.frames.last_mut() {
            // Acquire the top frame; hot ops run against it without re-borrowing, and the fused
            // `other` arm re-enters the outer loop after any frame switch.
            loop {
                let op = match frame.chunk.code.get(frame.ip) {
                    // Running off the end without a `ReturnValue` (the root chunk): the frame's
                    // result is the last value left on the stack.
                    None => break 'dispatch,
                    Some(op) => *op,
                };
                frame.ip += 1;
                match op {
                    Op::LoadLocal(slot) => {
                        let v = frame
                            .slots
                            .get(slot as usize)
                            .cloned()
                            .unwrap_or(Value::Nil);
                        vm.stack.push(v);
                    }
                    Op::SetLocal(slot) => {
                        let v = pop(&mut vm.stack);
                        if let Some(s) = frame.slots.get_mut(slot as usize) {
                            *s = v.clone();
                        }
                        vm.stack.push(v);
                    }
                    Op::SetLocalNc(slot) => {
                        let v = pop(&mut vm.stack);
                        if let Some(s) = frame.slots.get_mut(slot as usize) {
                            *s = v;
                        }
                    }
                    Op::AddImmLocal { slot, imm } => {
                        // `slots[slot] += imm` with no stack traffic (spec §6.1): `Small` checked
                        // in place; any other slot value goes through the `Number` tower, and a
                        // non-number slot reproduces the AST's `eval_binary` diagnostics.
                        let span = frame_span(frame);
                        if let Some(s) = frame.slots.get_mut(slot as usize) {
                            if matches!(s, Value::Number(Number::Small(_))) {
                                if let Value::Number(Number::Small(x)) = s {
                                    match x.checked_add(imm) {
                                        Some(v) => *x = v,
                                        None => {
                                            let r = Number::Small(*x) + Number::from(imm);
                                            *s = Value::Number(r);
                                        }
                                    }
                                }
                            } else {
                                let old = std::mem::replace(s, Value::Nil);
                                match old {
                                    Value::Number(n) => {
                                        *s = Value::Number(n + Number::from(imm));
                                    }
                                    other => {
                                        let r = self
                                            .eval_binary(
                                                AstBinOp::Add,
                                                other,
                                                Value::Number(Number::from(imm)),
                                            )
                                            .map_err(|e| crate::error::attach_span(e, span))?;
                                        *s = r;
                                    }
                                }
                            }
                        }
                    }
                    Op::AddToSlot(slot) => {
                        // `slots[slot] = slots[slot] + <popped rhs>` (fused `x = x + expr` for a
                        // local target, spec §12.2). `Number + Number` covers the exact tower
                        // (including `Small` overflow widening); anything else keeps the full
                        // `eval_binary` semantics (overloads, `Undefined` strictness).
                        let rhs = pop(&mut vm.stack);
                        let span = frame_span(frame);
                        if let Some(s) = frame.slots.get_mut(slot as usize) {
                            let old = std::mem::replace(s, Value::Nil);
                            match (old, rhs) {
                                // `Small` accumulator fast path (spec §6.1): checked i64 addition
                                // with exact-tower widening on overflow.
                                (
                                    Value::Number(Number::Small(x)),
                                    Value::Number(Number::Small(y)),
                                ) => match x.checked_add(y) {
                                    Some(v) => {
                                        *s = Value::Number(Number::Small(v));
                                    }
                                    None => {
                                        *s = Value::Number(Number::Small(x) + Number::Small(y));
                                    }
                                },
                                // F64 accumulator fast path (spec §6.5): plain IEEE addition.
                                (
                                    Value::Number(Number::Real(Real::F64(x))),
                                    Value::Number(Number::Real(Real::F64(y))),
                                ) => {
                                    *s = Value::Number(Number::Real(Real::F64(x + y)));
                                }
                                (Value::Number(x), Value::Number(y)) => {
                                    *s = Value::Number(x + y);
                                }
                                (old, rhs) => {
                                    let r = self
                                        .eval_binary(AstBinOp::Add, old, rhs)
                                        .map_err(|e| crate::error::attach_span(e, span))?;
                                    *s = r;
                                }
                            }
                        } else {
                            vm.stack.push(rhs);
                        }
                    }
                    Op::Const(k) => {
                        let c = frame.chunk.constants.get(k as usize).cloned();
                        vm.stack.push(resolve_const(c));
                    }
                    Op::Pop => {
                        vm.stack.pop();
                    }
                    Op::Jump(off) => {
                        // Cancellation (host interruption) at the loop back-edge (spec §16), matching
                        // the AST interpreter's per-iteration check.
                        if off < 0 {
                            self.check_cancelled()?;
                        }
                        frame.ip = jump_target(frame.ip, off);
                    }
                    Op::JumpIfFalse(off) => {
                        let cond = pop(&mut vm.stack);
                        match cond {
                            // Conditions must be boolean (spec §12.1) — mirrors `eval_cond`.
                            Value::Bool(b) => {
                                if !b {
                                    frame.ip = jump_target(frame.ip, off);
                                }
                            }
                            _ => {
                                let span = frame_span(frame);
                                return Err(crate::error::attach_span(
                                    crate::error::RuntimeError::Message(
                                        "condition must be a boolean".into(),
                                    ),
                                    span,
                                ));
                            }
                        }
                    }
                    Op::BranchLocalLt { a, b, off } => {
                        // Fused `LoadLocal a; LoadLocal b; Lt; JumpIfFalse` (spec §14): jump out
                        // of the loop when NOT `a < b`. `Small`/`F64` compare by reference (no
                        // operand clones); other shapes keep the general comparison semantics.
                        let cont = match (frame.slots.get(a as usize), frame.slots.get(b as usize))
                        {
                            (
                                Some(Value::Number(Number::Small(x))),
                                Some(Value::Number(Number::Small(y))),
                            ) => *x < *y,
                            (
                                Some(Value::Number(Number::Real(Real::F64(x)))),
                                Some(Value::Number(Number::Real(Real::F64(y)))),
                            ) => *x < *y,
                            _ => {
                                let va = frame.slots.get(a as usize).cloned().unwrap_or(Value::Nil);
                                let vb = frame.slots.get(b as usize).cloned().unwrap_or(Value::Nil);
                                match self.eval_compare(AstBinOp::Lt, va, vb)? {
                                    Value::Bool(b) => b,
                                    _ => {
                                        let span = frame_span(frame);
                                        return Err(crate::error::attach_span(
                                            crate::error::RuntimeError::Message(
                                                "condition must be a boolean".into(),
                                            ),
                                            span,
                                        ));
                                    }
                                }
                            }
                        };
                        if !cont {
                            frame.ip = jump_target(frame.ip, off);
                        }
                    }
                    Op::BranchLocalLe { a, b, off } => {
                        // Fused `LoadLocal a; LoadLocal b; Le; JumpIfFalse` (spec §14); see
                        // `BranchLocalLt` for the fast/fallback split.
                        let cont = match (frame.slots.get(a as usize), frame.slots.get(b as usize))
                        {
                            (
                                Some(Value::Number(Number::Small(x))),
                                Some(Value::Number(Number::Small(y))),
                            ) => *x <= *y,
                            (
                                Some(Value::Number(Number::Real(Real::F64(x)))),
                                Some(Value::Number(Number::Real(Real::F64(y)))),
                            ) => *x <= *y,
                            _ => {
                                let va = frame.slots.get(a as usize).cloned().unwrap_or(Value::Nil);
                                let vb = frame.slots.get(b as usize).cloned().unwrap_or(Value::Nil);
                                match self.eval_compare(AstBinOp::Le, va, vb)? {
                                    Value::Bool(b) => b,
                                    _ => {
                                        let span = frame_span(frame);
                                        return Err(crate::error::attach_span(
                                            crate::error::RuntimeError::Message(
                                                "condition must be a boolean".into(),
                                            ),
                                            span,
                                        ));
                                    }
                                }
                            }
                        };
                        if !cont {
                            frame.ip = jump_target(frame.ip, off);
                        }
                    }
                    Op::Index => {
                        let idx = pop(&mut vm.stack);
                        let base = pop(&mut vm.stack);
                        match (base, idx) {
                            // Array + integer index fast path (spec §11.3): negative-index
                            // normalization and the out-of-range diagnostic mirror
                            // `vm_index`/`index_to_usize`; every other shape delegates.
                            (Value::Array(a), Value::Number(n))
                                if !n.is_complex()
                                    && matches!(n, Number::Small(_) | Number::Integer(_)) =>
                            {
                                let len = a.len();
                                let Some(i) = n.as_i64() else {
                                    let span = frame_span(frame);
                                    return Err(crate::error::attach_span(
                                        crate::error::RuntimeError::Message(
                                            "array index must be an integer".into(),
                                        ),
                                        span,
                                    ));
                                };
                                let resolved = if i < 0 {
                                    len.checked_sub(i.unsigned_abs() as usize)
                                } else {
                                    usize::try_from(i).ok()
                                };
                                match resolved.filter(|&r| r < len).and_then(|r| a.get(r)) {
                                    Some(v) => vm.stack.push(v),
                                    None => {
                                        let span = frame_span(frame);
                                        return Err(crate::error::attach_span(
                                            crate::error::RuntimeError::IndexOutOfBounds(format!(
                                                "index {i} (length {len})"
                                            )),
                                            span,
                                        ));
                                    }
                                }
                            }
                            (base, idx) => {
                                let v = super::helpers::vm_index(self, base, idx)?;
                                vm.stack.push(v);
                            }
                        }
                    }
                    Op::Add | Op::Sub | Op::Mul | Op::Div | Op::Rem => {
                        let b = pop(&mut vm.stack);
                        let a = pop(&mut vm.stack);
                        match (a, b) {
                            // `Small`-typed fast path (spec §6.1): checked i64 arithmetic entirely
                            // local to the loop — no `Evaluator`, config, or span traffic. Overflow
                            // widens through the `Number` tower (exact layer, spec §6.1).
                            (Value::Number(Number::Small(p)), Value::Number(Number::Small(q)))
                                if matches!(op, Op::Add | Op::Sub | Op::Mul) =>
                            {
                                let r = match op {
                                    Op::Add => p.checked_add(q),
                                    Op::Sub => p.checked_sub(q),
                                    _ => p.checked_mul(q),
                                };
                                match r {
                                    Some(v) => vm.stack.push(Value::Number(Number::Small(v))),
                                    None => {
                                        let r = match op {
                                            Op::Add => Number::Small(p) + Number::Small(q),
                                            Op::Sub => Number::Small(p) - Number::Small(q),
                                            _ => Number::Small(p) * Number::Small(q),
                                        };
                                        vm.stack.push(Value::Number(r));
                                    }
                                }
                            }
                            // F64 fast path (spec §6.5): IEEE arithmetic, policy-independent for
                            // `+ - *`; kept out of the exact-tower promotion machinery. Must be
                            // tried before the general `Number` arm.
                            (
                                Value::Number(Number::Real(Real::F64(x))),
                                Value::Number(Number::Real(Real::F64(y))),
                            ) if matches!(op, Op::Add | Op::Sub | Op::Mul) => {
                                let r = match op {
                                    Op::Add => x + y,
                                    Op::Sub => x - y,
                                    _ => x * y,
                                };
                                vm.stack.push(Value::Number(Number::Real(Real::F64(r))));
                            }
                            (Value::Number(x), Value::Number(y))
                                if matches!(op, Op::Add | Op::Sub | Op::Mul) =>
                            {
                                let r = match op {
                                    Op::Add => x + y,
                                    Op::Sub => x - y,
                                    _ => x * y,
                                };
                                vm.stack.push(Value::Number(r));
                            }
                            // Division keeps the `fraction` policy semantics (spec §13.3) and the
                            // exact-layer zero-divisor diagnostics (spec §6.2/§13.4).
                            // Mixed exact/F64 division promotes to F64 (spec §6.4) — the result
                            // is in the collapsed layer either way, so compute directly.
                            (
                                Value::Number(Number::Small(x)),
                                Value::Number(Number::Real(Real::F64(y))),
                            ) if op == Op::Div => {
                                vm.stack
                                    .push(Value::Number(Number::Real(Real::F64((x as f64) / y))));
                            }
                            (
                                Value::Number(Number::Real(Real::F64(x))),
                                Value::Number(Number::Small(y)),
                            ) if op == Op::Div => {
                                vm.stack
                                    .push(Value::Number(Number::Real(Real::F64(x / (y as f64)))));
                            }
                            (
                                Value::Number(Number::Real(Real::F64(x))),
                                Value::Number(Number::Real(Real::F64(y))),
                            ) if op == Op::Div => {
                                // F64 division is policy-independent: the result is already in
                                // the collapsed layer (IEEE, inf on zero divisor, spec §6.5).
                                vm.stack.push(Value::Number(Number::Real(Real::F64(x / y))));
                            }
                            (Value::Number(x), Value::Number(y)) if op == Op::Div => {
                                if y.is_zero() && !matches!(y, Number::Real(_)) {
                                    if x.is_zero()
                                        && let Some(v) = self.custom_zero_div()
                                    {
                                        vm.stack.push(v);
                                    } else {
                                        let span = frame_span(frame);
                                        return Err(crate::error::attach_span(
                                            crate::error::RuntimeError::Message(
                                                "division by zero".into(),
                                            ),
                                            span,
                                        ));
                                    }
                                } else {
                                    let r = x / y;
                                    vm.stack.push(if self.current_config().fraction {
                                        Value::Number(r)
                                    } else {
                                        Value::Number(Number::Real(Real::F64(r.to_f64_lossy())))
                                    });
                                }
                            }
                            // Remainder (spec §11.4 Mod): integer/`F64` pairs mirror
                            // `number_mod` (including its `modulo by zero` diagnostic).
                            (Value::Number(x), Value::Number(y)) if op == Op::Rem => {
                                match (&x, &y) {
                                    (Number::Small(p), Number::Small(q)) => {
                                        if *q == 0 {
                                            let span = frame_span(frame);
                                            return Err(crate::error::attach_span(
                                                crate::error::RuntimeError::Message(
                                                    "modulo by zero".into(),
                                                ),
                                                span,
                                            ));
                                        }
                                        vm.stack.push(Value::Number(Number::Small(p % q)));
                                    }
                                    (Number::Real(Real::F64(p)), Number::Real(Real::F64(q))) => {
                                        vm.stack
                                            .push(Value::Number(Number::Real(Real::F64(p % q))));
                                    }
                                    _ => {
                                        let span = frame_span(frame);
                                        let r = crate::eval::number_mod(&x, &y)
                                            .map_err(|e| crate::error::attach_span(e, span))?;
                                        vm.stack.push(Value::Number(r));
                                    }
                                }
                            }
                            (a, b) => {
                                let span = frame_span(frame);
                                let r = self
                                    .eval_binary(binop(&op), a, b)
                                    .map_err(|e| crate::error::attach_span(e, span))?;
                                vm.stack.push(r);
                            }
                        }
                    }

                    Op::Lt | Op::Le | Op::Gt | Op::Ge | Op::EqCmp | Op::NeCmp => {
                        let b = pop(&mut vm.stack);
                        let a = pop(&mut vm.stack);
                        match (a, b) {
                            // `Small`-typed comparison fast path (spec §6.4 promotion is unnecessary
                            // when both operands share the same exact integer type).
                            (Value::Number(Number::Small(x)), Value::Number(Number::Small(y))) => {
                                use std::cmp::Ordering;
                                let ord = x.cmp(&y);
                                vm.stack.push(Value::Bool(match op {
                                    Op::EqCmp => ord == Ordering::Equal,
                                    Op::NeCmp => ord != Ordering::Equal,
                                    Op::Lt => ord == Ordering::Less,
                                    Op::Le => ord != Ordering::Greater,
                                    Op::Gt => ord == Ordering::Greater,
                                    _ => ord != Ordering::Less,
                                }));
                            }
                            (Value::Number(x), Value::Number(y)) => {
                                let r = self.vm_compare_numbers(cmp_binop(&op), x, y)?;
                                vm.stack.push(r);
                            }
                            (a, b) => {
                                let span = frame_span(frame);
                                let r = self
                                    .eval_compare(cmp_binop(&op), a, b)
                                    .map_err(|e| crate::error::attach_span(e, span))?;
                                vm.stack.push(r);
                            }
                        }
                    }
                    Op::IndexStoreLocal(slot) => {
                        // Fused store (spec §11.3): an array slot with a `Small` integer index
                        // normalizes and writes in place (CoW when the handle is aliased); the
                        // assigned value stays on the stack. Everything else delegates.
                        let value = pop(&mut vm.stack);
                        let idx = pop(&mut vm.stack);
                        let span = frame_span(frame);
                        if let Some(s) = frame.slots.get_mut(slot as usize) {
                            match (s, idx) {
                                (Value::Array(a), Value::Number(Number::Small(i))) => {
                                    let len = a.len();
                                    let resolved = if i < 0 {
                                        len.checked_sub(i.unsigned_abs() as usize)
                                    } else {
                                        usize::try_from(i).ok()
                                    };
                                    match resolved {
                                        Some(r) if r < len => {
                                            a.with_mut(|items| items[r] = value.clone());
                                            vm.stack.push(value);
                                        }
                                        other => {
                                            // `index_to_usize` reports the original index when
                                            // normalization underflows.
                                            let shown = other.unwrap_or(i.unsigned_abs() as usize);
                                            let shown = if i < 0 { i } else { shown as i64 };
                                            return Err(crate::error::attach_span(
                                                crate::error::RuntimeError::IndexOutOfBounds(
                                                    format!("index {shown} (length {len})"),
                                                ),
                                                span,
                                            ));
                                        }
                                    }
                                }
                                (s, idx) => {
                                    super::helpers::vm_index_store(self, s, idx, value.clone())
                                        .map_err(|e| crate::error::attach_span(e, span))?;
                                    vm.stack.push(value);
                                }
                            }
                        } else {
                            return Err(vm_limit("invalid local slot"));
                        }
                    }
                    Op::MethodLocal {
                        name: k,
                        argc,
                        slot,
                    } => {
                        // Fused `push` on an array slot (spec §11.3): scalar arguments cannot
                        // alias the receiver buffer, so the element is pushed directly; array /
                        // class arguments keep the general path (self-reference snapshotting).
                        let name_ok = matches!(
                            frame.chunk.constants.get(k as usize),
                            Some(Const::Name(n)) if n == "push"
                        );
                        if name_ok && argc == 1 {
                            let arg = pop(&mut vm.stack);
                            let scalar = matches!(
                                arg,
                                Value::Number(_)
                                    | Value::Bool(_)
                                    | Value::Char(_)
                                    | Value::String(_)
                                    | Value::Nil
                                    | Value::Symbol(_)
                                    | Value::Expr(_)
                                    | Value::JitFunction(_)
                            );
                            let recv = frame.slots.get_mut(slot as usize);
                            match recv {
                                Some(Value::Array(a)) if scalar => {
                                    a.with_mut(|items| items.push(arg));
                                    vm.stack.push(Value::Nil);
                                }
                                _ => {
                                    vm.stack.push(arg);
                                    self.step_vm(
                                        &mut vm,
                                        Op::MethodLocal {
                                            name: k,
                                            argc,
                                            slot,
                                        },
                                        env,
                                    )?;
                                    if vm.frames.is_empty() {
                                        break 'dispatch;
                                    }
                                    continue 'dispatch;
                                }
                            }
                        } else {
                            self.step_vm(
                                &mut vm,
                                Op::MethodLocal {
                                    name: k,
                                    argc,
                                    slot,
                                },
                                env,
                            )?;
                            if vm.frames.is_empty() {
                                break 'dispatch;
                            }
                            continue 'dispatch;
                        }
                    }
                    Op::CallName { name: site, argc } => {
                        // Cached-callee fast path (spec §19.5): a hit needs neither the name
                        // constant materialized nor the environment walked. A miss falls back to
                        // `step_vm`, which resolves (and populates) the cache for next time.
                        let epoch = crate::eval::func_epoch();
                        let cached = {
                            let mut cache = frame.chunk.callee_cache.borrow_mut();
                            if cache.len() <= site as usize {
                                cache.resize(site as usize + 1, None);
                            }
                            cache[site as usize]
                        };
                        match cached {
                            Some((e, Callee::Builtin(b))) if e == epoch => {
                                // `to_f64(x)` numeric fast path on the cached hit too (spec §9.2): skips the
                                // builtin dispatch chain; other argument shapes go through `vm_call_builtin`.
                                if argc == 1 && matches!(b, Builtin::Collapse("to_f64")) {
                                    let arg = vm.stack.pop().unwrap_or(Value::Nil);
                                    match arg {
                                        Value::Number(ref n) if !n.is_complex() => {
                                            let v = n.to_f64_lossy();
                                            vm.stack
                                                .push(Value::Number(Number::Real(Real::F64(v))));
                                            continue 'dispatch;
                                        }
                                        _ => vm.stack.push(arg),
                                    }
                                }
                                let args = split_args(&mut vm.stack, argc as usize);
                                self.vm_call_builtin(&mut vm, b, args)?;
                                continue 'dispatch;
                            }
                            Some((e, Callee::ProgramFn(idx))) if e == epoch => {
                                let chunk = Rc::clone(&vm.functions[idx as usize]);
                                if chunk.arity != argc {
                                    let span = frame_span(frame);
                                    return Err(crate::error::attach_span(
                                        vm_limit("argument count mismatch"),
                                        span,
                                    ));
                                }
                                vm.push_frame(chunk);
                                continue 'dispatch;
                            }
                            _ => {
                                self.step_vm(&mut vm, Op::CallName { name: site, argc }, env)?;
                                if vm.frames.is_empty() {
                                    break 'dispatch;
                                }
                                continue 'dispatch;
                            }
                        }
                    }
                    // ————— register-form local ops (locals as registers) —————
                    // Each op matches slot references directly on the `Small`/`F64` fast paths, so
                    // no operand `Value` is cloned; only the uncommon shapes clone/fall back.
                    Op::RegMove { dst, a } => {
                        let v = frame.slots.get(a as usize).cloned().unwrap_or(Value::Nil);
                        if let Some(s) = frame.slots.get_mut(dst as usize) {
                            *s = v;
                        }
                    }
                    Op::RegNeg { dst, a } => {
                        let r = match frame.slots.get(a as usize) {
                            Some(Value::Number(Number::Small(x))) => {
                                Value::Number(Number::Small(-x))
                            }
                            Some(Value::Number(Number::Real(Real::F64(x)))) => {
                                Value::Number(Number::Real(Real::F64(-x)))
                            }
                            Some(Value::Number(n)) => Value::Number(-n.clone()),
                            _ => {
                                let span = frame_span(frame);
                                let v = frame.slots.get(a as usize).cloned().unwrap_or(Value::Nil);
                                super::helpers::vm_neg(self, v)
                                    .map_err(|e| crate::error::attach_span(e, span))?
                            }
                        };
                        if let Some(s) = frame.slots.get_mut(dst as usize) {
                            *s = r;
                        }
                    }
                    Op::RegToF64 { dst, a } => {
                        // The fast path is only valid while `to_f64` still resolves to the core
                        // builtin (a user shadow re-resolves, matching the AST interpreter).
                        let is_builtin = to_f64_builtin(&mut vm.to_f64_builtin, env);
                        let r = if is_builtin {
                            match frame.slots.get(a as usize) {
                                // Collapse fast path (spec §9.2): a numeric local converts directly.
                                Some(Value::Number(Number::Small(x))) => {
                                    Value::Number(Number::Real(Real::F64(*x as f64)))
                                }
                                Some(Value::Number(Number::Real(Real::F64(x)))) => {
                                    Value::Number(Number::Real(Real::F64(*x)))
                                }
                                Some(Value::Number(Number::Real(Real::F32(x)))) => {
                                    Value::Number(Number::Real(Real::F64(*x as f64)))
                                }
                                Some(Value::Number(n)) if !n.is_complex() => {
                                    Value::Number(Number::Real(Real::F64(n.to_f64_lossy())))
                                }
                                // Arrays broadcast, strings parse, … keep the authoritative path.
                                _ => {
                                    let span = frame_span(frame);
                                    let v =
                                        frame.slots.get(a as usize).cloned().unwrap_or(Value::Nil);
                                    self.dispatch_builtin(Builtin::Collapse("to_f64"), vec![v])
                                        .map_err(|e| crate::error::attach_span(e, span))?
                                }
                            }
                        } else {
                            let span = frame_span(frame);
                            let arg = frame.slots.get(a as usize).cloned().unwrap_or(Value::Nil);
                            match env.borrow().get_func("to_f64") {
                                Some(f) => self
                                    .apply_function(&f, vec![arg])
                                    .map_err(|e| crate::error::attach_span(e, span))?,
                                None => {
                                    return Err(crate::error::attach_span(
                                        RuntimeError::Message("unknown function `to_f64`".into()),
                                        span,
                                    ));
                                }
                            }
                        };
                        if let Some(s) = frame.slots.get_mut(dst as usize) {
                            *s = r;
                        }
                    }
                    Op::RegBin { op, dst, a, b } => {
                        let r = match (frame.slots.get(a as usize), frame.slots.get(b as usize)) {
                            (
                                Some(Value::Number(Number::Small(x))),
                                Some(Value::Number(Number::Small(y))),
                            ) => {
                                let (x, y) = (*x, *y);
                                let narrowed = match op {
                                    ArithOp::Add => x.checked_add(y).map(Number::Small),
                                    ArithOp::Sub => x.checked_sub(y).map(Number::Small),
                                    ArithOp::Mul => x.checked_mul(y).map(Number::Small),
                                    ArithOp::Rem => {
                                        if y != 0 {
                                            x.checked_rem(y).map(Number::Small)
                                        } else {
                                            None
                                        }
                                    }
                                    ArithOp::Div => None,
                                };
                                match narrowed {
                                    Some(n) => Value::Number(n),
                                    None => {
                                        let span = frame_span(frame);
                                        self.reg_arith(
                                            op,
                                            Value::Number(Number::Small(x)),
                                            Value::Number(Number::Small(y)),
                                            span,
                                        )?
                                    }
                                }
                            }
                            (
                                Some(Value::Number(Number::Real(Real::F64(x)))),
                                Some(Value::Number(Number::Real(Real::F64(y)))),
                            ) => {
                                let (x, y) = (*x, *y);
                                let z = match op {
                                    ArithOp::Add => x + y,
                                    ArithOp::Sub => x - y,
                                    ArithOp::Mul => x * y,
                                    ArithOp::Div => x / y,
                                    ArithOp::Rem => x % y,
                                };
                                Value::Number(Number::Real(Real::F64(z)))
                            }
                            _ => {
                                let span = frame_span(frame);
                                let va = frame.slots.get(a as usize).cloned().unwrap_or(Value::Nil);
                                let vb = frame.slots.get(b as usize).cloned().unwrap_or(Value::Nil);
                                self.reg_arith(op, va, vb, span)?
                            }
                        };
                        if let Some(s) = frame.slots.get_mut(dst as usize) {
                            *s = r;
                        }
                    }
                    Op::RegBinImm { op, dst, a, imm } => {
                        let r = match (frame.slots.get(a as usize), imm) {
                            (Some(Value::Number(Number::Small(x))), Imm::Int(y)) => {
                                let (x, y) = (*x, y);
                                let narrowed = match op {
                                    ArithOp::Add => x.checked_add(y).map(Number::Small),
                                    ArithOp::Sub => x.checked_sub(y).map(Number::Small),
                                    ArithOp::Mul => x.checked_mul(y).map(Number::Small),
                                    ArithOp::Rem => {
                                        if y != 0 {
                                            x.checked_rem(y).map(Number::Small)
                                        } else {
                                            None
                                        }
                                    }
                                    ArithOp::Div => None,
                                };
                                match narrowed {
                                    Some(n) => Value::Number(n),
                                    None => {
                                        let span = frame_span(frame);
                                        self.reg_arith(
                                            op,
                                            Value::Number(Number::Small(x)),
                                            imm.to_value(),
                                            span,
                                        )?
                                    }
                                }
                            }
                            (Some(Value::Number(Number::Real(Real::F64(x)))), Imm::Float(bits)) => {
                                let (x, y) = (*x, f64::from_bits(bits));
                                let z = match op {
                                    ArithOp::Add => x + y,
                                    ArithOp::Sub => x - y,
                                    ArithOp::Mul => x * y,
                                    ArithOp::Div => x / y,
                                    ArithOp::Rem => x % y,
                                };
                                Value::Number(Number::Real(Real::F64(z)))
                            }
                            _ => {
                                let span = frame_span(frame);
                                let va = frame.slots.get(a as usize).cloned().unwrap_or(Value::Nil);
                                self.reg_arith(op, va, imm.to_value(), span)?
                            }
                        };
                        if let Some(s) = frame.slots.get_mut(dst as usize) {
                            *s = r;
                        }
                    }
                    Op::RegMulAdd { dst, a, b } => {
                        let r = match (
                            frame.slots.get(dst as usize),
                            frame.slots.get(a as usize),
                            frame.slots.get(b as usize),
                        ) {
                            (
                                Some(Value::Number(Number::Small(s))),
                                Some(Value::Number(Number::Small(x))),
                                Some(Value::Number(Number::Small(y))),
                            ) => {
                                let (s, x, y) = (*s, *x, *y);
                                match x.checked_mul(y).and_then(|p| s.checked_add(p)) {
                                    Some(v) => Value::Number(Number::Small(v)),
                                    None => {
                                        let span = frame_span(frame);
                                        self.reg_mul_add(
                                            Value::Number(Number::Small(s)),
                                            Value::Number(Number::Small(x)),
                                            Value::Number(Number::Small(y)),
                                            span,
                                        )?
                                    }
                                }
                            }
                            (
                                Some(Value::Number(Number::Real(Real::F64(s)))),
                                Some(Value::Number(Number::Real(Real::F64(x)))),
                                Some(Value::Number(Number::Real(Real::F64(y)))),
                            ) => Value::Number(Number::Real(Real::F64(*s + *x * *y))),
                            _ => {
                                let span = frame_span(frame);
                                let acc =
                                    frame.slots.get(dst as usize).cloned().unwrap_or(Value::Nil);
                                let va = frame.slots.get(a as usize).cloned().unwrap_or(Value::Nil);
                                let vb = frame.slots.get(b as usize).cloned().unwrap_or(Value::Nil);
                                self.reg_mul_add(acc, va, vb, span)?
                            }
                        };
                        if let Some(s) = frame.slots.get_mut(dst as usize) {
                            *s = r;
                        }
                    }
                    Op::RegIndex { dst, arr, idx } => {
                        let v = reg_index_read(self, frame, arr, idx)?;
                        if let Some(s) = frame.slots.get_mut(dst as usize) {
                            *s = v;
                        }
                    }
                    Op::RegIndexPush { arr, idx } => {
                        let v = reg_index_read(self, frame, arr, idx)?;
                        vm.stack.push(v);
                    }
                    // Void register pushes: emitted only for the statement form `local.push(arg)`,
                    // so no result is left on the operand stack (an expression-position `push`
                    // stays on the stack `MethodLocal` path, which yields `Nil`).
                    Op::RegPush { arr, src } => {
                        let v = frame.slots.get(src as usize).cloned().unwrap_or(Value::Nil);
                        match frame.slots.get_mut(arr as usize) {
                            Some(Value::Array(a)) => a.with_mut(|items| items.push(v)),
                            _ => return Err(vm_limit("`push` receiver must be an array binding")),
                        }
                    }
                    Op::RegPushImm { arr, imm } => {
                        let v = imm.to_value();
                        match frame.slots.get_mut(arr as usize) {
                            Some(Value::Array(a)) => a.with_mut(|items| items.push(v)),
                            _ => return Err(vm_limit("`push` receiver must be an array binding")),
                        }
                    }
                    Op::RegFill { arr, lo, hi, imm } => {
                        // Bulk extend for the fill idiom (spec §14). Bounds are integers (checked
                        // below); a non-integer bound falls back to the AST interpreter.
                        let span = frame_span(frame);
                        let lo_i = match frame.slots.get(lo as usize) {
                            Some(Value::Number(n)) => n.as_i64(),
                            _ => None,
                        };
                        let hi_i = match frame.slots.get(hi as usize) {
                            Some(Value::Number(n)) => n.as_i64(),
                            _ => None,
                        };
                        let (Some(lo_i), Some(hi_i)) = (lo_i, hi_i) else {
                            return Err(crate::error::attach_span(
                                vm_limit("fill loop bounds must be integers"),
                                span,
                            ));
                        };
                        let count = if hi_i > lo_i {
                            hi_i as i128 - lo_i as i128
                        } else {
                            0
                        };
                        if count > crate::eval::MAX_RANGE_ELEMS {
                            return Err(crate::error::attach_span(
                                RuntimeError::Message(format!(
                                    "fill materializes {count} elements, exceeding the {} element limit",
                                    crate::eval::MAX_RANGE_ELEMS
                                )),
                                span,
                            ));
                        }
                        let v = imm.to_value();
                        match frame.slots.get_mut(arr as usize) {
                            Some(Value::Array(a)) => a.with_mut(|items| {
                                items.extend(std::iter::repeat_n(v, count as usize))
                            }),
                            _ => return Err(vm_limit("fill target must be an array binding")),
                        }
                    }
                    Op::RegIndexStore { arr, idx, src } => {
                        let v = frame.slots.get(src as usize).cloned().unwrap_or(Value::Nil);
                        if let Some(Value::Number(Number::Small(i))) = frame.slots.get(idx as usize)
                        {
                            let i = *i;
                            reg_index_store_small(self, frame, arr, i, v)?;
                        } else {
                            reg_index_store(self, frame, arr, idx, v)?;
                        }
                    }
                    Op::RegIndexStoreImm { arr, idx, imm } => {
                        if let Some(Value::Number(Number::Small(i))) = frame.slots.get(idx as usize)
                        {
                            let i = *i;
                            reg_index_store_small(self, frame, arr, i, imm.to_value())?;
                        } else {
                            reg_index_store(self, frame, arr, idx, imm.to_value())?;
                        }
                    }
                    Op::BranchLocalCmpSum { le, a, b, imm, off } => {
                        // Fused `slots[a] <[=] slots[b] + imm`: `Small`/`Small` computes inline;
                        // any other shape falls back to the general add + compare.
                        let fast = match (frame.slots.get(a as usize), frame.slots.get(b as usize))
                        {
                            (
                                Some(Value::Number(Number::Small(x))),
                                Some(Value::Number(Number::Small(y))),
                            ) => y
                                .checked_add(imm)
                                .map(|rhs| if le { *x <= rhs } else { *x < rhs }),
                            _ => None,
                        };
                        let cont = match fast {
                            Some(c) => c,
                            None => {
                                let span = frame_span(frame);
                                let va = frame.slots.get(a as usize).cloned().unwrap_or(Value::Nil);
                                let vb = frame.slots.get(b as usize).cloned().unwrap_or(Value::Nil);
                                let rhs = self.reg_arith(
                                    ArithOp::Add,
                                    vb,
                                    Imm::Int(imm).to_value(),
                                    span,
                                )?;
                                match self.eval_compare(
                                    if le { AstBinOp::Le } else { AstBinOp::Lt },
                                    va,
                                    rhs,
                                )? {
                                    Value::Bool(b) => b,
                                    _ => {
                                        return Err(crate::error::attach_span(
                                            crate::error::RuntimeError::Message(
                                                "condition must be a boolean".into(),
                                            ),
                                            span,
                                        ));
                                    }
                                }
                            }
                        };
                        if !cont {
                            frame.ip = jump_target(frame.ip, off);
                        }
                    }
                    Op::RegIndexBranchFalse { arr, idx, off } => {
                        // `if slots[arr][slots[idx]]`: read the element and jump when false; a
                        // non-boolean element is the usual condition error (spec §12.1). The
                        // array+`Small`-index case reads the bool without cloning the element.
                        let fast =
                            match (frame.slots.get(arr as usize), frame.slots.get(idx as usize)) {
                                (Some(Value::Array(a)), Some(Value::Number(Number::Small(i)))) => {
                                    let i = *i;
                                    let len = a.len();
                                    resolve_small_index(len, i).map(|r| {
                                        a.with(|items| match items.get(r) {
                                            Some(Value::Bool(b)) => Ok(*b),
                                            _ => Err(()),
                                        })
                                    })
                                }
                                _ => None,
                            };
                        let cont = match fast {
                            Some(Ok(b)) => b,
                            Some(Err(())) => {
                                return Err(crate::error::attach_span(
                                    RuntimeError::Message("condition must be a boolean".into()),
                                    frame_span(frame),
                                ));
                            }
                            None => match reg_index_read(self, frame, arr, idx)? {
                                Value::Bool(b) => b,
                                _ => {
                                    return Err(crate::error::attach_span(
                                        RuntimeError::Message("condition must be a boolean".into()),
                                        frame_span(frame),
                                    ));
                                }
                            },
                        };
                        if !cont {
                            frame.ip = jump_target(frame.ip, off);
                        }
                    }
                    other => {
                        self.step_vm(&mut vm, other, env)?;
                        if vm.frames.is_empty() {
                            break 'dispatch;
                        }
                        continue 'dispatch;
                    }
                }
            }
        }
        let result = vm.stack.pop().unwrap_or(Value::Nil);
        self.vm_frames_pool.push(vm.frames);
        self.vm_stack_pool.push(vm.stack);
        Ok(result)
    }
}
/// Array+`Small` index read for the register-form index ops (spec §11.3): negative-index
/// normalization and the out-of-range diagnostic mirror the stack `Index` fast path, and the
/// receiver handle is never cloned. Every other base/index shape delegates to `vm_index`.
#[inline]
fn reg_index_read(
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
    super::helpers::vm_index(ev, base, index)
        .map_err(|e| crate::error::attach_span(e, frame_span(frame)))
}

/// Fast array index store for the register-form ops (spec §11.3): the array+`Small`-index case
/// normalizes and writes in place (copy-on-write when the handle is aliased); every other shape
/// delegates to `vm_index_store`; a non-array receiver is a VM capability limit (AST fallback).
#[inline]
fn reg_index_store(
    ev: &mut Evaluator,
    frame: &mut Frame,
    arr: u16,
    idx: u16,
    value: Value,
) -> Result<(), RuntimeError> {
    let span = frame_span(frame);
    let index = frame.slots.get(idx as usize).cloned().unwrap_or(Value::Nil);
    match (frame.slots.get_mut(arr as usize), index) {
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
                )),
            }
        }
        (Some(slot @ Value::Array(_)), index) => {
            super::helpers::vm_index_store(ev, slot, index, value)
                .map_err(|e| crate::error::attach_span(e, span))
        }
        _ => Err(vm_limit("index assignment requires an array binding")),
    }
}

/// Array store with an already-extracted small integer index: avoids cloning the index `Value`
/// (the common `A[j] = v` shape, spec §11.3).
#[inline]
fn reg_index_store_small(
    ev: &mut Evaluator,
    frame: &mut Frame,
    arr: u16,
    i: i64,
    value: Value,
) -> Result<(), RuntimeError> {
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
                )),
            }
        }
        // A `Dict` binding (or any other mutable indexed value) keeps the general path.
        Some(other) => {
            super::helpers::vm_index_store(ev, other, Value::Number(Number::Small(i)), value)
                .map_err(|e| crate::error::attach_span(e, span))
        }
        None => Err(vm_limit("invalid local slot")),
    }
}

/// Splice `rhs` into `target[lo..hi]` (spec §11.3): a `Nil` bound means omitted (0 / length); the
/// array is mutated in place (copy-on-write when its handle is aliased). Mirrors the AST's
/// `slice_bounds` clamping.
fn slice_store_value(
    target: &mut Value,
    lo: Value,
    hi: Value,
    rhs: Value,
) -> Result<(), RuntimeError> {
    let Value::Array(a) = target else {
        return Err(vm_limit("slice assignment requires an array binding"));
    };
    let len = a.len();
    let len_i = len as i64;
    let bound = |v: &Value, default: i64| -> Result<i64, RuntimeError> {
        match v {
            Value::Nil => Ok(default),
            Value::Number(n) => n
                .as_i64()
                .ok_or_else(|| RuntimeError::Message("slice bound must be an integer".into())),
            _ => Err(RuntimeError::Message(
                "slice bound must be an integer".into(),
            )),
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
        return Err(RuntimeError::Message(format!(
            "invalid slice range {lo}..{hi} (length {len})"
        )));
    }
    let Value::Array(rhs) = rhs else {
        return Err(RuntimeError::Message(
            "slice assignment right-hand side must be an array".into(),
        ));
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
fn resolve_small_index(len: usize, i: i64) -> Option<usize> {
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
fn to_f64_builtin(cache: &mut Option<(u64, bool)>, env: &EnvRef) -> bool {
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
fn frame_span(f: &Frame) -> prima_syntax::Span {
    let l = f
        .chunk
        .lines
        .get(f.ip.saturating_sub(1))
        .copied()
        .unwrap_or(0);
    prima_syntax::Span::new(l, l)
}

impl<'a> Vm<'a> {
    /// Push a frame for `chunk`, reserving its slot array.
    fn push_frame(&mut self, chunk: Rc<Chunk>) {
        let slots = vec![Value::Nil; chunk.slot_count as usize];
        self.frames.push(Frame {
            chunk,
            ip: 0,
            slots,
        });
    }

    /// The active (top) frame's chunk.
    fn active_chunk(&self) -> &Chunk {
        &self.frames.last().expect("unreachable").chunk
    }

    fn pop_frame(&mut self) {
        self.frames.pop();
    }

    /// Current instruction's source line for diagnostics.
    fn current_line(&self) -> u32 {
        let f = self.frames.last().expect("unreachable");
        f.chunk
            .lines
            .get(f.ip.saturating_sub(1))
            .copied()
            .unwrap_or(0)
    }

    /// Current instruction's source span (a degenerate line span) for diagnostics.
    fn current_span(&self) -> prima_syntax::Span {
        let l = self.current_line();
        prima_syntax::Span::new(l, l)
    }
}

impl Evaluator {
    /// Execute one `Op`, delegating value semantics to the evaluator. `Op::Jump*` operate on the
    /// program counter; `Op::CallName`/`Op::Method`/`Op::Return*` manage frames.
    fn step_vm(
        &mut self,
        vm: &mut Vm,
        op: Op,
        env: &EnvRef,
    ) -> Result<(), crate::error::RuntimeError> {
        match op {
            Op::Const(k) => {
                let c = vm.active_chunk().constants.get(k as usize).cloned();
                vm.stack.push(resolve_const(c));
                Ok(())
            }
            Op::LoadLocal(slot) => {
                let v = vm
                    .frames
                    .last()
                    .expect("unreachable")
                    .slots
                    .get(slot as usize)
                    .cloned()
                    .unwrap_or(Value::Nil);
                vm.stack.push(v);
                Ok(())
            }
            Op::SetLocal(slot) => {
                let v = pop(&mut vm.stack);
                if let Some(f) = vm.frames.last_mut()
                    && let Some(s) = f.slots.get_mut(slot as usize)
                {
                    *s = v.clone();
                }
                vm.stack.push(v);
                Ok(())
            }
            Op::BindParams(n) => {
                // Distribute the call arguments (pushed in order) to parameter slots `0..n`
                // and remove them from the operand stack (spec §11).
                let args = split_args(&mut vm.stack, n as usize);
                if let Some(f) = vm.frames.last_mut() {
                    for (i, a) in args.into_iter().enumerate() {
                        if let Some(s) = f.slots.get_mut(i) {
                            *s = a;
                        }
                    }
                }
                Ok(())
            }
            Op::Pop => {
                vm.stack.pop();
                Ok(())
            }
            Op::Dup => {
                let v = vm.stack.last().cloned().unwrap_or(Value::Nil);
                vm.stack.push(v);
                Ok(())
            }
            Op::LoadSelf => {
                let v = super::helpers::current_self_value(self)?;
                vm.stack.push(v);
                Ok(())
            }
            Op::LoadName(k) => {
                let name = resolve_name(vm.active_chunk(), k)?.to_string();
                let v = self.lookup_name_value(env, &name)?;
                vm.stack.push(v);
                Ok(())
            }
            Op::CallName { name, argc } => {
                let site = name;
                let name = resolve_name(vm.active_chunk(), name)?.to_string();
                self.vm_call_name(vm, env, &name, argc, site)
            }
            Op::Call { argc } => {
                let args = split_args(&mut vm.stack, argc as usize);
                let callee = pop(&mut vm.stack);
                self.vm_apply_function_value(vm, callee, args)
            }
            Op::Method { name, argc } => {
                let name = resolve_name(vm.active_chunk(), name)?.to_string();
                let args = split_args(&mut vm.stack, argc as usize);
                // Mutating `Dict`/`Set` methods write the whole receiver value back to its
                // binding (spec §11.6) — a write-back the compiled subset does not model — so
                // they fall back to the AST interpreter.
                if super::helpers::is_dict_mutating(&name) || super::helpers::is_set_mutating(&name)
                {
                    return Err(vm_limit("mutating dict/set methods are unsupported"));
                }
                let receiver = pop(&mut vm.stack);
                let r = super::helpers::vm_method_value(self, env, receiver, &name, args)?;
                vm.stack.push(r);
                Ok(())
            }
            Op::MethodLocal { name, argc, slot } => {
                let name = resolve_name(vm.active_chunk(), name)?.to_string();
                let args = split_args(&mut vm.stack, argc as usize);
                // Mutating `Array` method on a local slot (spec §11.3): the slot's array is
                // mutated in place (copy-on-write when its handle is aliased), so the mutation
                // is visible to later loads and aliasing bindings keep the old contents. A
                // non-array receiver is outside the compiled subset (e.g. a `String` method) —
                // the AST interpreter produces the authoritative behavior.
                let r = {
                    let frame = vm.frames.last_mut().expect("unreachable");
                    let Some(s) = frame.slots.get_mut(slot as usize) else {
                        return Err(vm_limit("invalid local slot"));
                    };
                    match s {
                        Value::Array(a) => self.mutate_array_handle(a, &name, args)?,
                        _ => return Err(vm_limit("expected an array binding")),
                    }
                };
                vm.stack.push(r);
                Ok(())
            }
            Op::MethodName { name: k, argc } => {
                let name = resolve_name(vm.active_chunk(), k)?.to_string();
                let args = split_args(&mut vm.stack, argc as usize);
                // Mutating `Array` method on an environment binding (spec §11.3): the binding's
                // array is mutated in place along the scope chain (CoW when aliased), mirroring
                // the AST's `mutate_array`. A missing or non-array receiver falls back to the
                // AST interpreter.
                if !matches!(env.borrow().get_value(&name), Some(Value::Array(_))) {
                    return Err(vm_limit("expected an array binding"));
                }
                let mut out: Option<Result<Value, RuntimeError>> = None;
                env.borrow_mut().update_value(&name, |slot| {
                    if let Value::Array(a) = slot {
                        out = Some(self.mutate_array_handle(a, &name, args));
                    }
                });
                let r = out.ok_or_else(|| vm_limit("expected an array binding"))??;
                vm.stack.push(r);
                Ok(())
            }
            Op::MakeArray(n) => {
                let items = split_args(&mut vm.stack, n as usize);
                vm.stack.push(Value::Array(items.into()));
                Ok(())
            }
            Op::MakeTuple(n) => {
                let items = split_args(&mut vm.stack, n as usize);
                vm.stack.push(Value::Tuple(items));
                Ok(())
            }
            Op::MakeSet(n) => {
                let items = split_args(&mut vm.stack, n as usize);
                let mut set = std::collections::HashSet::new();
                for it in items {
                    if let Some(k) = prima_core::ValueKey::from_value(&it) {
                        set.insert(k);
                    }
                }
                vm.stack.push(Value::Set(Box::new(set)));
                Ok(())
            }
            Op::MakeDict(n) => {
                // Dict literal lowers as alternating key/value pairs: `n` is the pair count.
                let pairs = split_args(&mut vm.stack, n as usize * 2);
                let mut dict = std::collections::HashMap::new();
                for pair in pairs.as_chunks::<2>().0 {
                    if let Some(k) = prima_core::ValueKey::from_value(&pair[0]) {
                        dict.insert(k, pair[1].clone());
                    }
                }
                vm.stack.push(Value::Dict(Box::new(dict)));
                Ok(())
            }
            Op::Index => {
                let idx = pop(&mut vm.stack);
                let base = pop(&mut vm.stack);
                let v = super::helpers::vm_index(self, base, idx)?;
                vm.stack.push(v);
                Ok(())
            }
            Op::IndexStoreLocal(slot) => {
                let value = pop(&mut vm.stack);
                let idx = pop(&mut vm.stack);
                let span = vm.current_span();
                // Mutate the slot's array in place (CoW when the handle is aliased, spec §11.3):
                // the write lands in the slot itself, so later loads observe it. A non-array slot
                // (e.g. a dict target with whole-value write-back, spec §11.6) is outside the
                // compiled subset.
                let frame = vm.frames.last_mut().expect("unreachable");
                let Some(s) = frame.slots.get_mut(slot as usize) else {
                    return Err(vm_limit("invalid local slot"));
                };
                if !matches!(s, Value::Array(_)) {
                    return Err(vm_limit("index assignment requires an array binding"));
                }
                super::helpers::vm_index_store(self, s, idx, value.clone())
                    .map_err(|e| crate::error::attach_span(e, span))?;
                vm.stack.push(value);
                Ok(())
            }
            Op::IndexStoreName(k) => {
                let value = pop(&mut vm.stack);
                let idx = pop(&mut vm.stack);
                let name = resolve_name(vm.active_chunk(), k)?.to_string();
                // Mutate the binding's array in place along the chain (spec §11.3/§12.2); a
                // missing or non-array binding falls back to the AST interpreter (dict
                // write-back and the authoritative diagnostics live there).
                if !matches!(env.borrow().get_value(&name), Some(Value::Array(_))) {
                    return Err(vm_limit("index assignment requires an array binding"));
                }
                let mut res: Option<Result<(), RuntimeError>> = None;
                env.borrow_mut().update_value(&name, |slot| {
                    res = Some(super::helpers::vm_index_store(
                        self,
                        slot,
                        idx.clone(),
                        value.clone(),
                    ));
                });
                match res {
                    Some(Ok(())) => {}
                    Some(Err(e)) => return Err(e),
                    None => return Err(vm_limit("index assignment requires an array binding")),
                }
                vm.stack.push(value);
                Ok(())
            }
            Op::Add | Op::Sub | Op::Mul | Op::Div | Op::Rem => {
                let b = pop(&mut vm.stack);
                let a = pop(&mut vm.stack);
                let span = vm.current_span();
                let r = match (a, b) {
                    // Numeric fast path (spec §6.1): compute directly on the Number tower (the
                    // Small i64 fast paths and exact overflow handling live inside `Number`);
                    // division keeps the `fraction` policy semantics.
                    (Value::Number(x), Value::Number(y)) => self
                        .vm_number_binary(arith_of(&op), x, y)
                        .map_err(|e| crate::error::attach_span(e, span))?,
                    // Symbolic/class/array operands keep the full `eval_binary` path: operator
                    // overloads (spec §18.5), elementwise broadcast (spec §11.4), DAG lowering,
                    // and `Undefined` strictness (spec §6.2).
                    (a, b) => self
                        .eval_binary(binop(&op), a, b)
                        .map_err(|e| crate::error::attach_span(e, span))?,
                };
                vm.stack.push(r);
                Ok(())
            }
            // `Pow` keeps the full delegation: its semantics (domain policy, symbolic fallback,
            // spec §6.5/§9.9) are not part of the local numeric fast path.
            Op::Pow => {
                let b = pop(&mut vm.stack);
                let a = pop(&mut vm.stack);
                let span = vm.current_span();
                let r = self
                    .eval_binary(binop(&op), a, b)
                    .map_err(|e| crate::error::attach_span(e, span))?;
                vm.stack.push(r);
                Ok(())
            }
            Op::Lt | Op::Le | Op::Gt | Op::Ge | Op::EqCmp | Op::NeCmp => {
                let b = pop(&mut vm.stack);
                let a = pop(&mut vm.stack);
                let span = vm.current_span();
                let r = match (a, b) {
                    // Numeric fast path: promotion + ordering, mirroring `eval_compare`'s numeric
                    // arm (spec §6.4 promotion, so `1 == 1.0` holds).
                    (Value::Number(x), Value::Number(y)) => self
                        .vm_compare_numbers(cmp_binop(&op), x, y)
                        .map_err(|e| crate::error::attach_span(e, span))?,
                    (a, b) => self
                        .eval_compare(cmp_binop(&op), a, b)
                        .map_err(|e| crate::error::attach_span(e, span))?,
                };
                vm.stack.push(r);
                Ok(())
            }
            Op::And => {
                let b = pop(&mut vm.stack);
                let a = pop(&mut vm.stack);
                vm.stack.push(bool_binop(AstBinOp::And, a, b)?);
                Ok(())
            }
            Op::Or => {
                let b = pop(&mut vm.stack);
                let a = pop(&mut vm.stack);
                vm.stack.push(bool_binop(AstBinOp::Or, a, b)?);
                Ok(())
            }
            Op::Neg => {
                let a = pop(&mut vm.stack);
                let r = match a {
                    // Numeric fast path (spec §6.1).
                    Value::Number(n) => Value::Number(-n),
                    // Class overloads (spec §18.5), arrays, symbolic values and `Undefined`
                    // strictness stay on the `eval_unary` path.
                    other => super::helpers::vm_neg(self, other)?,
                };
                vm.stack.push(r);
                Ok(())
            }
            Op::Not => {
                let a = pop(&mut vm.stack);
                match a {
                    Value::Bool(b) => {
                        vm.stack.push(Value::Bool(!b));
                        Ok(())
                    }
                    // Mirrors `eval_unary` (`!` requires a boolean, spec §12.1).
                    _ => {
                        let span = vm.current_span();
                        Err(crate::error::attach_span(
                            crate::error::RuntimeError::Message("`!` requires a boolean".into()),
                            span,
                        ))
                    }
                }
            }
            Op::Jump(off) => {
                // Cancellation (host interruption) at the loop back-edge (spec §16), matching the
                // AST interpreter's per-iteration check.
                if off < 0 {
                    self.check_cancelled()?;
                }
                if let Some(f) = vm.frames.last_mut() {
                    f.ip = jump_target(f.ip, off);
                }
                Ok(())
            }
            Op::JumpIfFalse(off) => {
                let cond = pop(&mut vm.stack); // the test always consumes the condition
                match cond {
                    // Conditions must be boolean (spec §12.1) — mirrors `eval_cond`.
                    Value::Bool(b) => {
                        if !b && let Some(f) = vm.frames.last_mut() {
                            f.ip = jump_target(f.ip, off);
                        }
                        Ok(())
                    }
                    _ => {
                        let span = vm.current_span();
                        Err(crate::error::attach_span(
                            crate::error::RuntimeError::Message(
                                "condition must be a boolean".into(),
                            ),
                            span,
                        ))
                    }
                }
            }
            Op::JumpIfTrue(off) => {
                let cond = pop(&mut vm.stack);
                match cond {
                    Value::Bool(b) => {
                        if b && let Some(f) = vm.frames.last_mut() {
                            f.ip = jump_target(f.ip, off);
                        }
                        Ok(())
                    }
                    _ => {
                        let span = vm.current_span();
                        Err(crate::error::attach_span(
                            crate::error::RuntimeError::Message(
                                "condition must be a boolean".into(),
                            ),
                            span,
                        ))
                    }
                }
            }
            Op::Return => {
                let v = pop(&mut vm.stack);
                vm.pop_frame();
                vm.stack.push(v);
                Ok(())
            }
            Op::ReturnValue => {
                let v = pop(&mut vm.stack);
                vm.pop_frame();
                vm.stack.push(v);
                Ok(())
            }
            // The fused forms are handled inline by the dispatch loop; `step_vm` only sees them
            // if the executor structure changes, in which case the AST path stays authoritative.
            Op::SetLocalNc(slot) => {
                let v = pop(&mut vm.stack);
                if let Some(f) = vm.frames.last_mut()
                    && let Some(s) = f.slots.get_mut(slot as usize)
                {
                    *s = v;
                }
                Ok(())
            }
            Op::AddImmLocal { slot, imm } => {
                if let Some(f) = vm.frames.last_mut()
                    && let Some(Value::Number(n)) = f.slots.get_mut(slot as usize)
                {
                    let r = n.clone() + Number::from(imm);
                    if let Some(Value::Number(s2)) = f.slots.get_mut(slot as usize) {
                        *s2 = r;
                    }
                }
                Ok(())
            }
            Op::AddToSlot(slot) => {
                let rhs = pop(&mut vm.stack);
                if let Some(f) = vm.frames.last_mut()
                    && let Some(s) = f.slots.get_mut(slot as usize)
                {
                    let old = std::mem::replace(s, Value::Nil);
                    let r = self.eval_binary(AstBinOp::Add, old, rhs)?;
                    if let Some(s2) = f.slots.get_mut(slot as usize) {
                        *s2 = r;
                    }
                } else {
                    vm.stack.push(rhs);
                }
                Ok(())
            }
            Op::BranchLocalLt { a, b, off } => {
                let va = vm
                    .frames
                    .last()
                    .and_then(|f| f.slots.get(a as usize))
                    .cloned()
                    .unwrap_or(Value::Nil);
                let vb = vm
                    .frames
                    .last()
                    .and_then(|f| f.slots.get(b as usize))
                    .cloned()
                    .unwrap_or(Value::Nil);
                let span = vm.current_span();
                let cont = match self.eval_compare(AstBinOp::Lt, va, vb)? {
                    Value::Bool(b) => b,
                    _ => {
                        return Err(crate::error::attach_span(
                            crate::error::RuntimeError::Message(
                                "condition must be a boolean".into(),
                            ),
                            span,
                        ));
                    }
                };
                if !cont && let Some(f) = vm.frames.last_mut() {
                    f.ip = jump_target(f.ip, off);
                }
                Ok(())
            }
            Op::LoadUpvalue(_) | Op::SetUpvalue(_) => Err(vm_limit(
                "upvalues are not supported in the compiled subset",
            )),
            Op::BranchLocalLe { a, b, off } => {
                let va = vm
                    .frames
                    .last()
                    .and_then(|f| f.slots.get(a as usize))
                    .cloned()
                    .unwrap_or(Value::Nil);
                let vb = vm
                    .frames
                    .last()
                    .and_then(|f| f.slots.get(b as usize))
                    .cloned()
                    .unwrap_or(Value::Nil);
                let span = vm.current_span();
                let cont = match self.eval_compare(AstBinOp::Le, va, vb)? {
                    Value::Bool(b) => b,
                    _ => {
                        return Err(crate::error::attach_span(
                            crate::error::RuntimeError::Message(
                                "condition must be a boolean".into(),
                            ),
                            span,
                        ));
                    }
                };
                if !cont && let Some(f) = vm.frames.last_mut() {
                    f.ip = jump_target(f.ip, off);
                }
                Ok(())
            }
            Op::SliceStoreLocal { slot } => {
                let rhs = pop(&mut vm.stack);
                let hi = pop(&mut vm.stack);
                let lo = pop(&mut vm.stack);
                let frame = vm.frames.last_mut().expect("unreachable");
                let Some(target) = frame.slots.get_mut(slot as usize) else {
                    return Err(vm_limit("invalid local slot"));
                };
                slice_store_value(target, lo, hi, rhs)
            }
            Op::SliceStoreName { name } => {
                let rhs = pop(&mut vm.stack);
                let hi = pop(&mut vm.stack);
                let lo = pop(&mut vm.stack);
                let name = resolve_name(vm.active_chunk(), name)?.to_string();
                let mut res: Option<Result<(), RuntimeError>> = None;
                env.borrow_mut().update_value(&name, |slot| {
                    res = Some(slice_store_value(slot, lo.clone(), hi.clone(), rhs.clone()));
                });
                match res {
                    Some(r) => r,
                    None => Err(vm_limit("slice assignment requires an array binding")),
                }
            }
            // Register-form ops are executed inline by the dispatch loop (`run_vm`); reaching
            // `step_vm` means the executor structure changed, so treat them as unsupported.
            Op::RegMove { .. }
            | Op::RegNeg { .. }
            | Op::RegToF64 { .. }
            | Op::RegBin { .. }
            | Op::RegBinImm { .. }
            | Op::RegMulAdd { .. }
            | Op::RegIndex { .. }
            | Op::RegIndexPush { .. }
            | Op::RegPush { .. }
            | Op::RegPushImm { .. }
            | Op::RegIndexStore { .. }
            | Op::RegIndexStoreImm { .. }
            | Op::BranchLocalCmpSum { .. }
            | Op::RegFill { .. }
            | Op::RegIndexBranchFalse { .. } => {
                Err(vm_limit("register op outside the dispatch loop"))
            }
        }
    }

    /// Numeric binary fast path (spec §6.1): `+ - * %` compute on the `Number` tower directly (the
    /// Small i64 checked fast paths and exact BigInt overflow handling live inside `Number`).
    /// Division replicates `eval_number_binary`: exact-layer zero divisors error (with the
    /// `undefined_handling := custom` black magic, spec §13.4) and `fraction := false` drops the
    /// result to F64 (spec §13.3).
    fn vm_number_binary(
        &mut self,
        op: ArithOp,
        x: prima_core::Number,
        y: prima_core::Number,
    ) -> Result<Value, RuntimeError> {
        use prima_core::{Number, Real};
        Ok(match op {
            ArithOp::Add => Value::Number(x + y),
            ArithOp::Sub => Value::Number(x - y),
            ArithOp::Mul => Value::Number(x * y),
            ArithOp::Div => {
                // Exact-layer division by zero: `0/0` is evaluated by black magic under the custom
                // policy (spec §13.4), otherwise the numeric layer errors (spec §6.2).
                if y.is_zero() && !matches!(y, Number::Real(_)) {
                    if x.is_zero()
                        && let Some(v) = self.custom_zero_div()
                    {
                        return Ok(v);
                    }
                    return crate::error::err("division by zero");
                }
                let r = x / y;
                // `fraction := false` (spec §13.3): the division result drops to F64.
                if self.current_config().fraction {
                    Value::Number(r)
                } else {
                    Value::Number(Number::Real(Real::F64(r.to_f64_lossy())))
                }
            }
            ArithOp::Rem => Value::Number(crate::eval::number_mod(&x, &y)?),
        })
    }

    /// Register-form arithmetic (spec §6.1/§12.2): `Small`/`F64` operands use the same inline fast
    /// paths as the stack instructions (`F64` division/remainder are already collapsed, so they
    /// are policy-independent); the exact tower handles everything else, and non-numbers keep the
    /// full `eval_binary` semantics (operator overloads, array concatenation, `Undefined`
    /// strictness), with the instruction's span attached.
    #[inline]
    fn reg_arith(
        &mut self,
        op: ArithOp,
        x: Value,
        y: Value,
        span: prima_syntax::Span,
    ) -> Result<Value, RuntimeError> {
        match (x, y) {
            (Value::Number(Number::Small(a)), Value::Number(Number::Small(b))) => {
                let narrowed = match op {
                    ArithOp::Add => a.checked_add(b).map(Number::Small),
                    ArithOp::Sub => a.checked_sub(b).map(Number::Small),
                    ArithOp::Mul => a.checked_mul(b).map(Number::Small),
                    // Integer remainder is exact and allocation-free; a zero divisor keeps the
                    // exact-tower diagnostic path below. Division keeps the policy path.
                    ArithOp::Rem => {
                        if b != 0 {
                            a.checked_rem(b).map(Number::Small)
                        } else {
                            None
                        }
                    }
                    ArithOp::Div => None,
                };
                match narrowed {
                    Some(n) => Ok(Value::Number(n)),
                    None => self
                        .vm_number_binary(op, Number::Small(a), Number::Small(b))
                        .map_err(|e| crate::error::attach_span(e, span)),
                }
            }
            (
                Value::Number(Number::Real(Real::F64(a))),
                Value::Number(Number::Real(Real::F64(b))),
            ) => {
                let r = match op {
                    ArithOp::Add => a + b,
                    ArithOp::Sub => a - b,
                    ArithOp::Mul => a * b,
                    ArithOp::Div => a / b,
                    ArithOp::Rem => a % b,
                };
                Ok(Value::Number(Number::Real(Real::F64(r))))
            }
            (Value::Number(a), Value::Number(b)) => self
                .vm_number_binary(op, a, b)
                .map_err(|e| crate::error::attach_span(e, span)),
            (a, b) => self
                .eval_binary(arith_binop(op), a, b)
                .map_err(|e| crate::error::attach_span(e, span)),
        }
    }

    /// Fused multiply-accumulate `acc + a*b` (spec §10): `Small`/`F64` inline fast paths with the
    /// exact-tower widening fallback.
    fn reg_mul_add(
        &mut self,
        acc: Value,
        a: Value,
        b: Value,
        span: prima_syntax::Span,
    ) -> Result<Value, RuntimeError> {
        use prima_core::{Number, Real};
        if let (
            Value::Number(Number::Small(s)),
            Value::Number(Number::Small(x)),
            Value::Number(Number::Small(y)),
        ) = (&acc, &a, &b)
        {
            if let Some(p) = x.checked_mul(*y)
                && let Some(r) = s.checked_add(p)
            {
                return Ok(Value::Number(Number::Small(r)));
            }
        } else if let (
            Value::Number(Number::Real(Real::F64(s))),
            Value::Number(Number::Real(Real::F64(x))),
            Value::Number(Number::Real(Real::F64(y))),
        ) = (&acc, &a, &b)
        {
            return Ok(Value::Number(Number::Real(Real::F64(s + x * y))));
        }
        let prod = self.reg_arith(ArithOp::Mul, a, b, span)?;
        self.reg_arith(ArithOp::Add, acc, prod, span)
    }

    /// Numeric comparison fast path: promote to a common type before comparing (spec §6.4, so
    /// `1 == 1.0` holds), mirroring `eval_compare`'s numeric arm.
    fn vm_compare_numbers(
        &mut self,
        op: AstBinOp,
        x: prima_core::Number,
        y: prima_core::Number,
    ) -> Result<Value, RuntimeError> {
        use prima_core::{Number, Real, number::promote};
        use std::cmp::Ordering;
        let ord = if let (Number::Small(a), Number::Small(b)) = (&x, &y) {
            Some(a.cmp(b))
        } else {
            let (px, py) = promote(&x, &y);
            match (&px, &py) {
                (Number::Small(a), Number::Small(b)) => Some(a.cmp(b)),
                (Number::Integer(a), Number::Integer(b)) => Some(a.cmp(b)),
                (Number::Rational(a), Number::Rational(b)) => Some(a.cmp(b)),
                (Number::Real(Real::F32(a)), Number::Real(Real::F32(b))) => a.partial_cmp(b),
                (Number::Real(Real::F64(a)), Number::Real(Real::F64(b))) => a.partial_cmp(b),
                _ => None,
            }
        }
        .ok_or_else(|| RuntimeError::Message("cannot compare these numbers".into()))?;
        Ok(Value::Bool(match op {
            AstBinOp::Eq => ord == Ordering::Equal,
            AstBinOp::Ne => ord != Ordering::Equal,
            AstBinOp::Lt => ord == Ordering::Less,
            AstBinOp::Le => ord != Ordering::Greater,
            AstBinOp::Gt => ord == Ordering::Greater,
            AstBinOp::Ge => ord != Ordering::Less,
            _ => unreachable!("vm_compare_numbers only handles comparison ops"),
        }))
    }

    /// Resolve a name to a value: an env value, or a fresh symbol for an unbound/global name (mirrors
    /// the interpreter's `Path` resolution). A bound function is not a value (spec §11).
    fn lookup_name_value(&mut self, env: &EnvRef, name: &str) -> Result<Value, RuntimeError> {
        let env_r = env.borrow();
        if let Some(v) = env_r.get_value(name) {
            return Ok(v);
        }
        if env_r.get_func(name).is_some() {
            return crate::error::err(format!("function `{name}` cannot be used as a value"));
        }
        Ok(Value::Expr(self.pool.symbol(self.symbols.intern(name))))
    }

    /// Resolve a call-by-name and apply it, in dispatch order:
    /// 1. a chunk in the running program's table → a new frame (with an arity guard, since a
    ///    mismatch would mis-consume the operand stack; the AST path reports the error);
    /// 2. a core builtin → direct dispatch (broadcast rules mirrored from `apply_function`);
    /// 3. a host `fn` → re-enter the VM through the per-definition chunk cache;
    /// 4. anything else (MFn, native, layered, `get`) → `apply_function`.
    fn vm_call_name(
        &mut self,
        vm: &mut Vm,
        env: &EnvRef,
        name: &str,
        argc: u16,
        site: u16,
    ) -> Result<(), RuntimeError> {
        // Per-call-site callee cache (spec §19.5): builtin and program-function callees resolve
        // once per function-definition epoch; anything environment-dependent re-resolves.
        let epoch = crate::eval::func_epoch();
        let cached = {
            let frame = vm.frames.last().expect("unreachable");
            let mut cache = frame.chunk.callee_cache.borrow_mut();
            if cache.len() <= site as usize {
                cache.resize(site as usize + 1, None);
            }
            cache[site as usize]
        };
        match cached {
            Some((e, Callee::Builtin(b))) if e == epoch => {
                let args = split_args(&mut vm.stack, argc as usize);
                return self.vm_call_builtin(vm, b, args);
            }
            Some((e, Callee::ProgramFn(idx))) if e == epoch => {
                let chunk = Rc::clone(&vm.functions[idx as usize]);
                if chunk.arity != argc {
                    return Err(vm_limit("argument count mismatch"));
                }
                vm.push_frame(chunk);
                return Ok(());
            }
            _ => {}
        }
        if let Some(&idx) = vm.table.get(name) {
            let chunk = Rc::clone(&vm.functions[idx as usize]);
            if chunk.arity != argc {
                return Err(vm_limit("argument count mismatch"));
            }
            // The chunk's leading `BindParams` consumes the already-pushed args.
            let frame = vm.frames.last().expect("unreachable");
            cache_callee(frame, site, epoch, Callee::ProgramFn(idx));
            vm.push_frame(chunk);
            return Ok(());
        }
        let Some(func) = env.borrow().get_func(name) else {
            return Err(crate::error::RuntimeError::Message(format!(
                "unknown function `{name}`"
            )));
        };
        match func.as_ref() {
            Function::Builtin(b) => {
                // Builtin callees are cacheable (spec §19.5): shadowing re-bumps the epoch.
                let frame = vm.frames.last().expect("unreachable");
                cache_callee(frame, site, epoch, Callee::Builtin(*b));
                // `to_f64(x)` fast path (spec §9.2 collapse family): a numeric argument converts
                // directly (`ensure_real` semantics: complex stays on the general error path),
                // skipping the builtin dispatch chain. Every other argument shape (broadcast over
                // arrays, numeric strings, …) keeps the general path.
                if argc == 1 && matches!(b, Builtin::Collapse("to_f64")) {
                    let arg = vm.stack.pop().unwrap_or(Value::Nil);
                    match arg {
                        Value::Number(ref n) if !n.is_complex() => {
                            let v = n.to_f64_lossy();
                            vm.stack.push(Value::Number(Number::Real(Real::F64(v))));
                            return Ok(());
                        }
                        _ => vm.stack.push(arg),
                    }
                }
                let args = split_args(&mut vm.stack, argc as usize);
                self.vm_call_builtin(vm, *b, args)
            }
            Function::Host { .. } => {
                // Tail-recursive bodies stay on the AST trampoline (spec §10.2 item 6): the
                // compiled subset has no constant-stack recursion, so running them here would
                // overflow where the AST path does not (see `apply_function`).
                let tco = match func.as_ref() {
                    Function::Host { body, .. } => {
                        self.current_config().opt_level >= crate::config::OptLevel::O2
                            && crate::opt::tail_call_of(body).is_some()
                    }
                    _ => false,
                };
                if tco {
                    let args = split_args(&mut vm.stack, argc as usize);
                    let r = self.apply_function(&func, args)?;
                    vm.stack.push(r);
                    return Ok(());
                }
                let args = split_args(&mut vm.stack, argc as usize);
                // Re-enter the VM through the per-definition chunk cache (spec §19.5);
                // `Ok(None)` (outside the compiled subset) applies through the evaluator below.
                if let Some(v) = self.try_vm_named(env, name, args.clone())? {
                    vm.stack.push(v);
                    return Ok(());
                }
                let r = self.apply_function(&func, args)?;
                vm.stack.push(r);
                Ok(())
            }
            _ => {
                let args = split_args(&mut vm.stack, argc as usize);
                let r = self.apply_function(&func, args)?;
                vm.stack.push(r);
                Ok(())
            }
        }
    }

    /// Direct builtin dispatch for `CallName` (spec §18.1/§11.4): mirrors `apply_function`'s
    /// builtin path — pure builtins broadcast over array arguments (`R0009`), collection
    /// builtins take their array argument whole (spec appendix B.1) — without the function-value
    /// indirection.
    fn vm_call_builtin(
        &mut self,
        vm: &mut Vm,
        b: Builtin,
        args: Vec<Value>,
    ) -> Result<(), RuntimeError> {
        let r = self.dispatch_builtin(b, args)?;
        vm.stack.push(r);
        Ok(())
    }

    /// Value-producing builtin dispatch shared by `CallName` and the register `to_f64` fast path:
    /// mirrors `apply_function`'s builtin path — pure builtins broadcast over array arguments
    /// (`R0009`), collection builtins take their array argument whole (spec appendix B.1).
    fn dispatch_builtin(&mut self, b: Builtin, args: Vec<Value>) -> Result<Value, RuntimeError> {
        // `derivative`/`jit`/`map`/… receive *un-evaluated* argument expressions in `eval_call`
        // (spec §19.4, appendix B.1); the VM has already evaluated its arguments, so these fall
        // back to the AST interpreter.
        if matches!(
            b,
            Builtin::Derivative
                | Builtin::Partial
                | Builtin::Grad
                | Builtin::Limit
                | Builtin::Jit
                | Builtin::Map
                | Builtin::Filter
                | Builtin::Reduce
        ) {
            return Err(vm_limit("builtin receives un-evaluated arguments"));
        }
        let takes_array_whole = b.is_collection();
        let positions: Vec<usize> = if takes_array_whole {
            Vec::new()
        } else {
            args.iter()
                .enumerate()
                .filter(|(_, v)| matches!(v, Value::Array(_)))
                .map(|(i, _)| i)
                .collect()
        };
        if !positions.is_empty() && b.is_pure() {
            if self.current_config().broadcast {
                let f = Function::Builtin(b);
                self.broadcast_call(&f, args, &positions)
            } else {
                Err(crate::error::RuntimeError::Message(
                    "implicit broadcast is disabled (`broadcast := false`); use `@.`".into(),
                ))
            }
        } else {
            self.call_builtin(b, args)
        }
    }

    /// Apply a function *value* directly (for compiled `Call` sites that put a closure/function on
    /// the stack). Falls back to the general function-value application.
    fn vm_apply_function_value(
        &mut self,
        vm: &mut Vm,
        callee: Value,
        args: Vec<Value>,
    ) -> Result<(), RuntimeError> {
        // Only reachable if the compiler emits a function value; the current subset never does, so
        // treat it as a capability limit to be safe.
        let _ = (vm, callee, args);
        Err(vm_limit("function-value calls are not supported"))
    }
}

/// Compute a jump target from a program counter and a signed offset. `ip` is the post-increment pc
/// (one past the jump instruction), so the offset is measured from `ip - 1`.
fn jump_target(ip: usize, off: i32) -> usize {
    ((ip as i64) - 1 + off as i64).max(0) as usize
}

fn resolve_const(c: Option<Const>) -> Value {
    match c {
        Some(Const::Value(v)) => v,
        Some(Const::Str(s)) => Value::String(s),
        Some(Const::Name(_)) => Value::Nil, // name constants only used by LoadName/CallName/Method
        None => Value::Nil,
    }
}

fn resolve_name(chunk: &Chunk, k: u16) -> Result<&str, RuntimeError> {
    match chunk.constants.get(k as usize) {
        Some(Const::Name(s)) => Ok(s),
        _ => Err(vm_limit("invalid name constant")),
    }
}

fn binop(op: &Op) -> AstBinOp {
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
fn arith_of(op: &Op) -> ArithOp {
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
fn arith_binop(op: ArithOp) -> AstBinOp {
    match op {
        ArithOp::Add => AstBinOp::Add,
        ArithOp::Sub => AstBinOp::Sub,
        ArithOp::Mul => AstBinOp::Mul,
        ArithOp::Div => AstBinOp::Div,
        ArithOp::Rem => AstBinOp::Mod,
    }
}

fn cmp_binop(op: &Op) -> AstBinOp {
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

fn bool_binop(op: AstBinOp, a: Value, b: Value) -> Result<Value, RuntimeError> {
    match (a, b) {
        (Value::Bool(x), Value::Bool(y)) => Ok(Value::Bool(match op {
            AstBinOp::And => x && y,
            AstBinOp::Or => x || y,
            _ => unreachable!(),
        })),
        _ => Err(crate::error::RuntimeError::Message(
            "`&&`/`||` require boolean operands".into(),
        )),
    }
}

/// Cache a resolved call-site callee on the executing chunk (spec §19.5); validated by epoch on
/// every subsequent execution.
fn cache_callee(frame: &Frame, site: u16, epoch: u64, callee: Callee) {
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
fn single_entry_program(chunk: Chunk) -> VmProgram {
    let mut names = rustc_hash::FxHashMap::default();
    names.insert("__entry".to_string(), 0u32);
    VmProgram {
        root: Rc::new(Chunk::new()),
        functions: vec![Rc::new(chunk)],
        names,
    }
}

impl Evaluator {
    /// Run a named function through the bytecode VM, compiling its body once per definition
    /// (spec §19.5): the compiled chunk is cached on the function value itself (an `OnceLock`,
    /// mirroring the JIT `HotState`); a redefinition binds a fresh function with a fresh cache.
    /// Compile failures are cached too, so a body outside the compiled subset is never
    /// retried. Returns `Ok(None)` when the name does not resolve to a host `fn`, the arity
    /// mismatches, or the body is outside the compiled subset; genuine runtime errors propagate.
    pub(crate) fn try_vm_named(
        &mut self,
        env: &EnvRef,
        name: &str,
        args: Vec<Value>,
    ) -> Result<Option<Value>, RuntimeError> {
        let Some(func) = env.borrow().get_func(name) else {
            return Ok(None);
        };
        self.try_vm_call(&func, args)
    }

    /// Attempt to run a block-bodied function (`fn`) through the bytecode VM, compiling the body
    /// once per definition through `cache` (spec §19.5; compile failures are cached as `None`).
    /// Returns `Ok(Some(value))` when the body compiles and runs, `Ok(None)` when the body
    /// is outside the compiled subset (caller falls back to the AST), or a runtime error, which
    /// propagates to the caller (an AST re-run would duplicate side effects, spec §19.5).
    pub(crate) fn try_vm_single(
        &mut self,
        params: &[prima_syntax::ast::Param],
        body: &prima_syntax::ast::Block,
        f_env: &EnvRef,
        cache: &crate::eval::VmChunkCache,
        args: Vec<Value>,
    ) -> Result<Option<Value>, RuntimeError> {
        if args.len() != params.len() {
            // Arity errors are reported by the authoritative AST path (spec §11).
            return Ok(None);
        }
        let program = cache.get_or_init(|| {
            crate::vm::comp::compile_function_body(params, body)
                .ok()
                .map(single_entry_program)
                .map(Rc::new)
        });
        let Some(program) = program else {
            return Ok(None); // outside the compiled subset → AST fallback
        };
        // Parameters bind into slots by the chunk's leading `BindParams`, which consumes the
        // argument values from the operand stack (and uses the closure env for free names).
        match self.run_vm(f_env, program, Some(("__entry", args))) {
            Ok(v) => Ok(Some(v)),
            // A VM capability limit falls back to the AST interpreter; a genuine runtime error
            // propagates (re-running on the AST would duplicate side effects, spec §19.5).
            Err(e) if is_vm_limit(&e) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Attempt to run a resolved function value through the bytecode VM (spec §19.5): like
    /// [`Self::try_vm_single`] but taking the function value, so `vm_call_function` (benchmarks,
    /// C-ABI) shares the per-definition chunk cache. Returns `Ok(None)` for anything but a host
    /// `fn` inside the compiled subset; genuine runtime errors propagate.
    pub(crate) fn try_vm_call(
        &mut self,
        func: &Function,
        args: Vec<Value>,
    ) -> Result<Option<Value>, RuntimeError> {
        match func {
            Function::Host {
                params,
                body,
                env: f_env,
                vm,
                ..
            } => self.try_vm_single(params, body, f_env, vm, args),
            _ => Ok(None),
        }
    }
}
