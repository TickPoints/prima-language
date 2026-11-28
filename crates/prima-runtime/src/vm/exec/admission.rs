//! VM admission: compile-once caching and `Hit`/`Fallback` outcomes (spec §19.5).

use std::rc::Rc;

use prima_core::Value;

use super::VmOutcome;
use super::ops::single_entry_program;
use crate::error::RuntimeError;
use crate::eval::{Env, EnvRef, Evaluator, Function};
use crate::vm::error::VmError;

impl Evaluator {
    /// Run a named function through the bytecode VM, compiling its body once per definition
    /// (spec §19.5): the compiled chunk is cached on the function value itself (an `OnceLock`,
    /// mirroring the JIT `HotState`); a redefinition binds a fresh function with a fresh cache.
    /// Compile failures are cached too, so a body outside the compiled subset is never
    /// retried. Returns `Fallback` when the name does not resolve to a host `fn`, the arity
    /// mismatches, or the body is outside the compiled subset; genuine runtime errors propagate.
    pub(crate) fn try_vm_named(
        &mut self,
        env: &EnvRef,
        name: &str,
        args: Vec<Value>,
    ) -> Result<VmOutcome, RuntimeError> {
        let Some(func) = env.borrow().get_func(name) else {
            self.path_stats.vm_fallbacks += 1;
            return Ok(VmOutcome::Fallback);
        };
        self.try_vm_call(&func, args)
    }

    /// Attempt to run a block-bodied function (`fn`) through the bytecode VM, compiling the body
    /// once per definition through `cache` (spec §19.5; compile failures are cached as `None`).
    /// Returns `Hit` when the body compiles and runs, `Fallback` when it is outside the compiled
    /// subset (caller falls back to the AST), or a runtime error, which propagates to the caller
    /// (an AST re-run would duplicate side effects, spec §19.5).
    pub(crate) fn try_vm_single(
        &mut self,
        params: &[prima_syntax::ast::Param],
        body: &prima_syntax::ast::Block,
        f_env: &EnvRef,
        cache: &crate::eval::VmChunkCache,
        args: Vec<Value>,
    ) -> Result<VmOutcome, RuntimeError> {
        if args.len() != params.len() {
            // Arity errors are reported by the authoritative AST path (spec §11).
            self.path_stats.vm_fallbacks += 1;
            return Ok(VmOutcome::Fallback);
        }
        let program = cache.get_or_init(|| {
            crate::vm::comp::compile_function_body(params, body)
                .ok()
                .map(single_entry_program)
                .map(Rc::new)
        });
        let Some(program) = program else {
            self.path_stats.vm_fallbacks += 1;
            return Ok(VmOutcome::Fallback); // outside the compiled subset → AST fallback
        };
        // Parameters bind into slots by the chunk's leading `BindParams`, which consumes the
        // argument values from the operand stack (and uses the closure env for free names). A fresh
        // call environment mirrors `apply_host`'s `Env::child`: names created by the body (including
        // nested `fn` bindings) are call-local instead of leaking into the module environment.
        let call_env = Env::child(f_env);
        match self.run_vm(&call_env, program, Some(("__entry", args))) {
            Ok(v) => {
                self.path_stats.vm_hits += 1;
                Ok(VmOutcome::Hit(v))
            }
            // A typed capability limit falls back to the AST interpreter; a genuine runtime error
            // propagates (re-running on the AST would duplicate side effects, spec §19.5).
            Err(VmError::Limit(_)) => {
                self.path_stats.vm_fallbacks += 1;
                Ok(VmOutcome::Fallback)
            }
            Err(VmError::Runtime(error)) => Err(error),
            Err(VmError::UnknownEntry(name)) => Err(RuntimeError::UnknownVmEntry {
                name: name.to_string(),
            }),
        }
    }

    /// Attempt to run a resolved function value through the bytecode VM (spec §19.5): like
    /// [`Self::try_vm_single`] but taking the function value, so `vm_call_function` (benchmarks,
    /// C-ABI) shares the per-definition chunk cache. Returns `Fallback` for anything but a host
    /// `fn` inside the compiled subset; genuine runtime errors propagate.
    pub(crate) fn try_vm_call(
        &mut self,
        func: &Function,
        args: Vec<Value>,
    ) -> Result<VmOutcome, RuntimeError> {
        match func {
            Function::Host {
                params,
                body,
                env: f_env,
                vm,
                ..
            } => self.try_vm_single(params, body, f_env, vm, args),
            _ => {
                self.path_stats.vm_fallbacks += 1;
                Ok(VmOutcome::Fallback)
            }
        }
    }
}
