//! Single-instruction execution and evaluator-backed VM operations (spec §19.5).

use prima_core::{Number, Value};
use prima_syntax::ast::BinOp as AstBinOp;

use super::ops::*;
use super::registers::*;
use super::{Vm, pop, split_args};
use crate::error::RuntimeError;
use crate::eval::{EnvRef, Evaluator};
use crate::vm::error::{VmError, VmLimit};
use crate::vm::op::Op;

impl Evaluator {
    /// Execute one `Op`, delegating value semantics to the evaluator. `Op::Jump*` operate on the
    /// program counter; `Op::CallName`/`Op::Method`/`Op::Return*` manage frames.
    pub(super) fn step_vm(&mut self, vm: &mut Vm, op: Op, env: &EnvRef) -> Result<(), VmError> {
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
                let v = crate::vm::helpers::current_self_value(self)?;
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
                // Mutating collection methods are compiled to `MethodLocal`/`MethodName` (the
                // compiler rejects other receiver shapes up front), so this path never needs a
                // whole-function fallback after the arguments have been evaluated.
                let receiver = pop(&mut vm.stack);
                let r = crate::vm::helpers::vm_method_value(self, env, receiver, &name, args)?;
                vm.stack.push(r);
                Ok(())
            }
            Op::MethodLocal { name, argc, slot } => {
                let name = resolve_name(vm.active_chunk(), name)?.to_string();
                let args = split_args(&mut vm.stack, argc as usize);
                // Mutating `Array`/`Dict`/`Set` methods on a local slot mutate the slot's storage in
                // place (CoW when the handle is aliased, spec §11.3/§11.6). Any other receiver
                // (e.g. `String::push`) keeps its authoritative AST behavior; delegate in place
                // instead of re-running the body after its side effects (spec §19.5).
                let is_collection = vm
                    .frames
                    .last()
                    .and_then(|f| f.slots.get(slot as usize))
                    .map(|value| matches!(value, Value::Array(_) | Value::Dict(_) | Value::Set(_)));
                let Some(is_collection) = is_collection else {
                    return Err(VmLimit::InvalidLocalSlot.into());
                };
                let r = if is_collection {
                    let frame = vm.frames.last_mut().expect("unreachable");
                    let s = frame.slots.get_mut(slot as usize).expect("checked above");
                    match s {
                        Value::Array(a) => self.mutate_array_handle(a, &name, args)?,
                        Value::Dict(d) => self.mutate_dict_handle(d, &name, args)?,
                        Value::Set(set) => self.mutate_set_handle(set, &name, args)?,
                        _ => unreachable!("checked above"),
                    }
                } else {
                    let receiver = vm
                        .frames
                        .last()
                        .and_then(|f| f.slots.get(slot as usize))
                        .cloned()
                        .unwrap_or(Value::Nil);
                    crate::vm::helpers::vm_method_value(self, env, receiver, &name, args)?
                };
                vm.stack.push(r);
                Ok(())
            }
            Op::MethodName {
                receiver,
                name: k,
                argc,
            } => {
                let name = resolve_name(vm.active_chunk(), k)?.to_string();
                let binding = resolve_name(vm.active_chunk(), receiver)?.to_string();
                let args = split_args(&mut vm.stack, argc as usize);
                // Mutating collection methods write the receiver value back along the scope chain
                // (spec §11.3/§11.6). A missing or non-collection binding is authoritative in the
                // AST path; delegate the call in place rather than re-running the function body.
                if matches!(
                    env.borrow().get_value(&binding),
                    Some(Value::Array(_) | Value::Dict(_) | Value::Set(_))
                ) {
                    let mut out: Option<Result<Value, RuntimeError>> = None;
                    env.borrow_mut().update_value(&binding, |slot| {
                        out = Some(match slot {
                            Value::Array(a) => self.mutate_array_handle(a, &name, args),
                            Value::Dict(d) => self.mutate_dict_handle(d, &name, args),
                            Value::Set(set) => self.mutate_set_handle(set, &name, args),
                            _ => Err(RuntimeError::Internal {
                                problem: crate::error::InternalProblem::BindingChanged,
                            }),
                        });
                    });
                    let Some(result) = out else {
                        return Err(VmLimit::BindingDisappeared.into());
                    };
                    vm.stack.push(result?);
                } else {
                    let receiver = self.lookup_name_value(env, &binding)?;
                    let r = crate::vm::helpers::vm_method_value(self, env, receiver, &name, args)?;
                    vm.stack.push(r);
                }
                Ok(())
            }
            Op::MakeArray(n) => {
                let items = split_args(&mut vm.stack, n as usize);
                vm.stack.push(Value::Array(items.into()));
                Ok(())
            }
            Op::MakeTuple(n) => {
                let items = split_args(&mut vm.stack, n as usize);
                vm.stack.push(Value::Tuple(items.into()));
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
            Op::Field { name } => {
                let name = resolve_name(vm.active_chunk(), name)?.to_string();
                let receiver = pop(&mut vm.stack);
                let v = self.field_value(receiver, &name)?;
                vm.stack.push(v);
                Ok(())
            }
            Op::EvalStmt { stmt } => {
                let stmt = vm
                    .active_chunk()
                    .fallback_stmts
                    .get(stmt as usize)
                    .cloned()
                    .ok_or(VmLimit::InvalidFallbackStatement)?;
                match self.eval_stmt(env, &stmt)? {
                    crate::eval::Flow::Continue => Ok(()),
                    crate::eval::Flow::Return(_) => Err(VmLimit::FallbackStatementReturned.into()),
                }
            }
            Op::Index => {
                let idx = pop(&mut vm.stack);
                let base = pop(&mut vm.stack);
                let v = crate::vm::helpers::vm_index(self, base, idx)?;
                vm.stack.push(v);
                Ok(())
            }
            Op::IndexMulti { spec } => {
                let shape = vm
                    .active_chunk()
                    .index_specs
                    .get(spec as usize)
                    .cloned()
                    .ok_or(VmLimit::InvalidMultiIndexShape)?;
                let mut raw = Vec::with_capacity(shape.len());
                for is_slice in shape.iter().rev() {
                    if *is_slice {
                        let hi = pop(&mut vm.stack);
                        let lo = pop(&mut vm.stack);
                        raw.push((true, lo, hi));
                    } else {
                        raw.push((false, pop(&mut vm.stack), Value::Nil));
                    }
                }
                raw.reverse();
                let base = pop(&mut vm.stack);
                let bound = |v: Value| -> Result<Option<i64>, RuntimeError> {
                    match v {
                        Value::Nil => Ok(None),
                        Value::Number(n) => {
                            n.as_i64()
                                .map(Some)
                                .ok_or_else(|| RuntimeError::InvalidIndex {
                                    problem: crate::error::IndexProblem::NotInteger {
                                        what: "slice bound",
                                    },
                                })
                        }
                        _ => Err(RuntimeError::InvalidIndex {
                            problem: crate::error::IndexProblem::NotInteger {
                                what: "slice bound",
                            },
                        }),
                    }
                };
                let mut selections = Vec::with_capacity(raw.len());
                for (is_slice, a, b) in raw {
                    if is_slice {
                        selections.push(crate::eval::IndexSelection::Slice(bound(a)?, bound(b)?));
                    } else {
                        let idx = match a {
                            Value::Number(n) => n.as_i64(),
                            _ => None,
                        }
                        .ok_or_else(|| RuntimeError::InvalidIndex {
                            problem: crate::error::IndexProblem::NotInteger {
                                what: "array index",
                            },
                        })?;
                        selections.push(crate::eval::IndexSelection::Elem(idx));
                    }
                }
                let v = self.index_nd(base, &selections)?;
                vm.stack.push(v);
                Ok(())
            }
            Op::IndexStoreLocal(slot) => {
                let value = pop(&mut vm.stack);
                let idx = pop(&mut vm.stack);
                let span = vm.current_span();
                // Mutate the slot's storage in place (CoW when the handle is aliased, spec §11.3):
                // the write lands in the slot itself, so later loads observe it. `vm_index_store`
                // handles arrays and dicts and produces the authoritative error for other targets.
                let frame = vm.frames.last_mut().expect("unreachable");
                let Some(s) = frame.slots.get_mut(slot as usize) else {
                    return Err(VmLimit::InvalidLocalSlot.into());
                };
                crate::vm::helpers::vm_index_store(self, s, idx, value.clone())
                    .map_err(|e| crate::error::attach_span(e, span))?;
                vm.stack.push(value);
                Ok(())
            }
            Op::IndexStoreName(k) => {
                let value = pop(&mut vm.stack);
                let idx = pop(&mut vm.stack);
                let name = resolve_name(vm.active_chunk(), k)?.to_string();
                // Mutate the binding's storage in place along the chain (spec §11.3/§12.2);
                // `vm_index_store` handles arrays and dicts and errors authoritatively otherwise.
                let mut res: Option<Result<(), RuntimeError>> = None;
                env.borrow_mut().update_value(&name, |slot| {
                    res = Some(crate::vm::helpers::vm_index_store(
                        self,
                        slot,
                        idx.clone(),
                        value.clone(),
                    ));
                });
                match res {
                    Some(Ok(())) => {}
                    Some(Err(e)) => return Err(e.into()),
                    None => {
                        // The binding disappeared after becoming visible: surface the AST's
                        // undefined-name error instead of re-running the body.
                        self.lookup_name_value(env, &name)?;
                        return Err(VmLimit::BindingDisappeared.into());
                    }
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
                    other => crate::vm::helpers::vm_neg(self, other)?,
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
                            RuntimeError::ExpectedBoolean {
                                context: crate::error::BooleanContext::Not,
                            },
                            span,
                        )
                        .into())
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
                            RuntimeError::ExpectedBoolean {
                                context: crate::error::BooleanContext::Condition,
                            },
                            span,
                        )
                        .into())
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
                            RuntimeError::ExpectedBoolean {
                                context: crate::error::BooleanContext::Condition,
                            },
                            span,
                        )
                        .into())
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
                            RuntimeError::ExpectedBoolean {
                                context: crate::error::BooleanContext::Condition,
                            },
                            span,
                        )
                        .into());
                    }
                };
                if !cont && let Some(f) = vm.frames.last_mut() {
                    f.ip = jump_target(f.ip, off);
                }
                Ok(())
            }
            Op::LoadUpvalue(_) | Op::SetUpvalue(_) => {
                Err(VmLimit::UpvalueOutsideCompiledSubset.into())
            }
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
                            RuntimeError::ExpectedBoolean {
                                context: crate::error::BooleanContext::Condition,
                            },
                            span,
                        )
                        .into());
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
                    return Err(VmLimit::InvalidLocalSlot.into());
                };
                slice_store_value(target, lo, hi, rhs)?;
                Ok(())
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
                    Some(r) => Ok(r?),
                    None => {
                        // Surface the AST's undefined-name error instead of re-running the body.
                        self.lookup_name_value(env, &name)?;
                        Err(VmLimit::BindingDisappeared.into())
                    }
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
            | Op::RegIndexBranchFalse { .. } => Err(VmLimit::RegisterOpOutsideDispatchLoop.into()),
        }
    }
}
