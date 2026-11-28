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
//! Runtime errors propagate to the caller without an AST re-run; only typed capability limits
//! (see [`VmLimit`]) fall back to the AST interpreter.

use std::rc::Rc;

use prima_core::{Number, Real, Value};
use prima_syntax::ast::BinOp as AstBinOp;

use super::error::{VmError, VmLimit};
use super::op::{ArithOp, Callee, Chunk, Const, Imm, Op, Program as VmProgram};
use crate::builtins::Builtin;
use crate::error::RuntimeError;
use crate::eval::{EnvRef, Evaluator};

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
    /// function and returns its result; with `entry = None` it runs the root chunk body. Returns
    /// [`VmError::Runtime`] for a program error, [`VmError::Limit`] when the compiled subset cannot
    /// execute a construct, and [`VmError::UnknownEntry`] for a missing entry name.
    pub fn run_vm(
        &mut self,
        env: &EnvRef,
        program: &VmProgram,
        entry: Option<(&str, Vec<Value>)>,
    ) -> Result<Value, VmError> {
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
                let idx = *vm
                    .table
                    .get(name)
                    .ok_or_else(|| VmError::UnknownEntry(name.to_string()))?;
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
                                    RuntimeError::ExpectedBoolean {
                                        context: crate::error::BooleanContext::Condition,
                                    },
                                    span,
                                )
                                .into());
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
                                            RuntimeError::ExpectedBoolean {
                                                context: crate::error::BooleanContext::Condition,
                                            },
                                            span,
                                        )
                                        .into());
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
                                            RuntimeError::ExpectedBoolean {
                                                context: crate::error::BooleanContext::Condition,
                                            },
                                            span,
                                        )
                                        .into());
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
                                        RuntimeError::InvalidIndex {
                                            problem: crate::error::IndexProblem::NotInteger {
                                                what: "array index",
                                            },
                                        },
                                        span,
                                    )
                                    .into());
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
                                        )
                                        .into());
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
                                            RuntimeError::DivisionByZero,
                                            span,
                                        )
                                        .into());
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
                                                RuntimeError::ModuloByZero,
                                                span,
                                            )
                                            .into());
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
                                            )
                                            .into());
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
                            return Err(VmLimit::InvalidLocalSlot.into());
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
                                    return Err(VmLimit::ArityMismatch.into());
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
                                        .map_err(|e| Self::attach_vm_span(e, span))?
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
                                        RuntimeError::UnknownFunction {
                                            name: "to_f64".into(),
                                        },
                                        span,
                                    )
                                    .into());
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
                        if frame.slots.get(arr as usize).is_none() {
                            return Err(VmLimit::InvalidLocalSlot.into());
                        }
                        if let Some(Value::Array(a)) = frame.slots.get_mut(arr as usize) {
                            a.with_mut(|items| items.push(v));
                        } else {
                            // A non-array receiver (e.g. `String::push`) keeps its authoritative
                            // behavior in the AST path; delegate in place so the body is not re-run.
                            let receiver =
                                frame.slots.get(arr as usize).cloned().unwrap_or(Value::Nil);
                            super::helpers::vm_method_value(self, env, receiver, "push", vec![v])?;
                        }
                    }
                    Op::RegPushImm { arr, imm } => {
                        let v = imm.to_value();
                        if frame.slots.get(arr as usize).is_none() {
                            return Err(VmLimit::InvalidLocalSlot.into());
                        }
                        if let Some(Value::Array(a)) = frame.slots.get_mut(arr as usize) {
                            a.with_mut(|items| items.push(v));
                        } else {
                            let receiver =
                                frame.slots.get(arr as usize).cloned().unwrap_or(Value::Nil);
                            super::helpers::vm_method_value(self, env, receiver, "push", vec![v])?;
                        }
                    }
                    Op::RegFill { arr, lo, hi, imm } => {
                        // Bulk extend for the fill idiom (spec §14). A non-integer bound reproduces
                        // the AST loop's error; a non-array target delegates one `push` per
                        // iteration, so a class/String receiver keeps its authoritative behavior.
                        let span = frame_span(frame);
                        let bound = |v: &Value| -> Result<i64, RuntimeError> {
                            match v {
                                Value::Number(n) => n.as_i64().ok_or_else(|| {
                                    RuntimeError::Type(format!(
                                        "loop range must be integers, got {n}"
                                    ))
                                }),
                                other => Err(RuntimeError::Type(format!(
                                    "loop range must be integers, got {other:?}"
                                ))),
                            }
                        };
                        let lo_v = frame
                            .slots
                            .get(lo as usize)
                            .cloned()
                            .ok_or(VmLimit::InvalidLocalSlot)?;
                        let hi_v = frame
                            .slots
                            .get(hi as usize)
                            .cloned()
                            .ok_or(VmLimit::InvalidLocalSlot)?;
                        let (lo_i, hi_i) = match (bound(&lo_v), bound(&hi_v)) {
                            (Ok(lo), Ok(hi)) => (lo, hi),
                            (Err(e), _) | (_, Err(e)) => {
                                return Err(crate::error::attach_span(e, span).into());
                            }
                        };
                        let count = if hi_i > lo_i {
                            hi_i as i128 - lo_i as i128
                        } else {
                            0
                        };
                        if count > crate::eval::MAX_RANGE_ELEMS {
                            return Err(crate::error::attach_span(
                                RuntimeError::LimitExceeded { what: "fill", detail: format!("materializes {count} elements, exceeding the {} element limit", crate::eval::MAX_RANGE_ELEMS) },
                                span
                            )
                            .into());
                        }
                        let v = imm.to_value();
                        if matches!(frame.slots.get(arr as usize), Some(Value::Array(_))) {
                            if let Some(Value::Array(a)) = frame.slots.get_mut(arr as usize) {
                                a.with_mut(|items| {
                                    items.extend(std::iter::repeat_n(v, count as usize))
                                });
                            }
                        } else {
                            if frame.slots.get(arr as usize).is_none() {
                                return Err(VmLimit::InvalidLocalSlot.into());
                            }
                            let receiver =
                                frame.slots.get(arr as usize).cloned().unwrap_or(Value::Nil);
                            for _ in 0..count {
                                super::helpers::vm_method_value(
                                    self,
                                    env,
                                    receiver.clone(),
                                    "push",
                                    vec![v.clone()],
                                )?;
                            }
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
                                            RuntimeError::ExpectedBoolean {
                                                context: crate::error::BooleanContext::Condition,
                                            },
                                            span,
                                        )
                                        .into());
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
                                    RuntimeError::ExpectedBoolean {
                                        context: crate::error::BooleanContext::Condition,
                                    },
                                    frame_span(frame),
                                )
                                .into());
                            }
                            None => match reg_index_read(self, frame, arr, idx)? {
                                Value::Bool(b) => b,
                                _ => {
                                    return Err(crate::error::attach_span(
                                        RuntimeError::ExpectedBoolean {
                                            context: crate::error::BooleanContext::Condition,
                                        },
                                        frame_span(frame),
                                    )
                                    .into());
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
mod admission;
mod calls;
mod numeric;
mod ops;
mod registers;
mod step;
mod vm_state;

pub(crate) use ops::VmOutcome;
use ops::*;
use registers::*;
