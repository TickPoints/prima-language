//! VM call dispatch and builtin invocation (spec §18.1/§19.5).

use std::rc::Rc;

use prima_core::{Number, Real, Value};

use super::ops::cache_callee;
use super::{Vm, VmOutcome, split_args};
use crate::builtins::Builtin;
use crate::error::RuntimeError;
use crate::eval::{EnvRef, Evaluator, Function};
use crate::vm::error::{VmError, VmLimit};
use crate::vm::op::Callee;

impl Evaluator {
    pub(crate) fn lookup_name_value(
        &mut self,
        env: &EnvRef,
        name: &str,
    ) -> Result<Value, RuntimeError> {
        let env_r = env.borrow();
        if let Some(v) = env_r.get_value(name) {
            return Ok(v);
        }
        if env_r.get_func(name).is_some() {
            return crate::error::type_err(format!("function `{name}` cannot be used as a value"));
        }
        Ok(Value::Expr(self.pool.symbol(self.symbols.intern(name))))
    }

    /// Resolve a call-by-name and apply it, in dispatch order:
    /// 1. a chunk in the running program's table → a new frame (with an arity guard, since a
    ///    mismatch would mis-consume the operand stack; the AST path reports the error);
    /// 2. a core builtin → direct dispatch (broadcast rules mirrored from `apply_function`);
    /// 3. a host `fn` → re-enter the VM through the per-definition chunk cache;
    /// 4. anything else (MFn, native, layered, `get`) → `apply_function`.
    pub(crate) fn vm_call_name(
        &mut self,
        vm: &mut Vm,
        env: &EnvRef,
        name: &str,
        argc: u16,
        site: u16,
    ) -> Result<(), VmError> {
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
                    return Err(VmLimit::ArityMismatch.into());
                }
                vm.push_frame(chunk);
                return Ok(());
            }
            _ => {}
        }
        if let Some(&idx) = vm.table.get(name) {
            let chunk = Rc::clone(&vm.functions[idx as usize]);
            if chunk.arity != argc {
                return Err(VmLimit::ArityMismatch.into());
            }
            // The chunk's leading `BindParams` consumes the already-pushed args.
            let frame = vm.frames.last().expect("unreachable");
            cache_callee(frame, site, epoch, Callee::ProgramFn(idx));
            vm.push_frame(chunk);
            return Ok(());
        }
        let Some(func) = env.borrow().get_func(name) else {
            return Err(RuntimeError::UnknownFunction {
                name: name.to_string(),
            }
            .into());
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
                // `Fallback` (outside the compiled subset) applies through the evaluator below.
                if let VmOutcome::Hit(v) = self.try_vm_named(env, name, args.clone())? {
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
    pub(crate) fn vm_call_builtin(
        &mut self,
        vm: &mut Vm,
        b: Builtin,
        args: Vec<Value>,
    ) -> Result<(), VmError> {
        let r = self.dispatch_builtin(b, args)?;
        vm.stack.push(r);
        Ok(())
    }

    /// Value-producing builtin dispatch shared by `CallName` and the register `to_f64` fast path:
    /// mirrors `apply_function`'s builtin path — pure builtins broadcast over array arguments
    /// (`R0009`), collection builtins take their array argument whole (spec appendix B.1).
    pub(crate) fn dispatch_builtin(
        &mut self,
        b: Builtin,
        args: Vec<Value>,
    ) -> Result<Value, VmError> {
        // `derivative`/`jit`/`map`/… receive *un-evaluated* argument expressions in `eval_call`
        // (spec §19.4, appendix B.1). The compiler rejects such calls before emitting argument
        // code, so this is an unreachable safety net that must not trigger a late AST re-run.
        if b.needs_unevaluated_args() {
            return Err(VmLimit::UnevaluatedBuiltinArgument.into());
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
                Ok(self.broadcast_call(&f, args, &positions)?)
            } else {
                Err(RuntimeError::BroadcastDisabled.into())
            }
        } else {
            Ok(self.call_builtin(b, args)?)
        }
    }

    /// Attach the current span to a runtime failure, leaving a capability limit (control flow,
    /// not a diagnostic) untouched.
    pub(crate) fn attach_vm_span(e: VmError, span: prima_syntax::Span) -> VmError {
        match e {
            VmError::Runtime(error) => VmError::Runtime(crate::error::attach_span(error, span)),
            limit => limit,
        }
    }

    /// Apply a function *value* directly (for compiled `Call` sites that put a closure/function on
    /// the stack). Falls back to the general function-value application.
    pub(crate) fn vm_apply_function_value(
        &mut self,
        vm: &mut Vm,
        callee: Value,
        args: Vec<Value>,
    ) -> Result<(), VmError> {
        // Only reachable if the compiler emits a function value; the current subset never does, so
        // treat it as a capability limit to be safe.
        let _ = (vm, callee, args);
        Err(VmLimit::FunctionValueCallUnsupported.into())
    }
}
