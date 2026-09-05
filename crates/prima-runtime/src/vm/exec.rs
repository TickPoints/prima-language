//! Bytecode VM executor (spec §19.5, Milestone B).
//!
//! A call/return stack machine over `Value`-typed slots. Each `Frame` references a compiled `Chunk`
//! (which owns its constant pool), a program counter, and a slot array. The VM owns a frame stack
//! and the operand stack; every value-producing command delegates to the `Evaluator` (arithmetic via
//! `eval_binary`, comparisons via `eval_compare`, index reads/writes, method calls, and calls via
//! `apply_function`), so VM results equal AST-interpreter results by construction. Control flow and
//! local slot management are handled natively by the dispatch loop.
//!
//! The VM is entered through [`Evaluator::run_vm`] behind the `vm` config policy. When
//! `compile_program` rejects a program, the whole program runs on the AST interpreter (the
//! authoritative path).

use prima_core::Value;
use prima_syntax::ast::BinOp as AstBinOp;
use prima_syntax::ast::{Block, Param};

use super::op::{Const, Op, Program as VmProgram};
use crate::error::RuntimeError;
use crate::eval::{EnvRef, Evaluator, Function};

/// A single active call frame: the chunk being executed, its program counter, and its slot array.
#[derive(Debug)]
pub struct Frame {
    chunk: super::op::Chunk,
    ip: usize,
    slots: Vec<Value>,
}

impl Frame {
    /// Resolve the current instruction, advancing the program counter.
    fn next(&mut self) -> Option<Op> {
        let op = self.chunk.code.get(self.ip).cloned();
        self.ip += 1;
        op
    }
}

/// The VM: a stack of frames plus the operand stack shared across frames.
pub struct Vm {
    frames: Vec<Frame>,
    stack: Vec<Value>,
    /// Function-name → chunk index dispatch table.
    table: std::collections::HashMap<String, u32>,
    functions: Vec<super::op::Chunk>,
}

fn pop(stack: &mut Vec<Value>) -> Value {
    stack.pop().unwrap_or(Value::Nil)
}

/// Split the top `n` values off the stack into an ordered `Vec` (top-most last).
fn split_args(stack: &mut Vec<Value>, n: usize) -> Vec<Value> {
    let start = stack.len().saturating_sub(n);
    stack.split_off(start)
}

/// Whether a `Value` is falsey (spec §12.1): `false`, `Nil`, numeric zero, or `""`.
fn is_falsey(v: &Value) -> bool {
    match v {
        Value::Bool(b) => !b,
        Value::Nil => true,
        Value::Number(n) => n.is_zero(),
        Value::String(s) => s.is_empty(),
        _ => false,
    }
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
        let mut vm = Vm {
            frames: Vec::new(),
            stack: Vec::new(),
            table: program.names.clone(),
            functions: program.functions.clone(),
        };
        match entry {
            Some((name, args)) => {
                let idx = *vm.table.get(name).ok_or_else(|| {
                    crate::error::RuntimeError::Message(format!("unknown VM entry `{name}`"))
                })?;
                let chunk = vm.functions[idx as usize].clone();
                vm.push_frame(chunk);
                for a in args {
                    vm.stack.push(a);
                }
            }
            None => {
                vm.push_frame(program.root.clone());
            }
        }

        // Dispatch loop (no explicit return detection needed: `ReturnValue`/implicit end pops the
        // frame; the result is the last value each returning frame leaves on the stack).
        loop {
            let op = {
                let f = vm.frames.last_mut();
                match f.and_then(|f| f.next()) {
                    Some(op) => op,
                    None => break,
                }
            };
            self.step_vm(&mut vm, op, env)?;
            if vm.frames.is_empty() {
                break;
            }
        }
        Ok(vm.stack.pop().unwrap_or(Value::Nil))
    }
}
impl Vm {
    /// Push a frame for `chunk`, reserving its slot array.
    fn push_frame(&mut self, chunk: super::op::Chunk) {
        let slots = vec![Value::Nil; chunk.slot_count as usize];
        self.frames.push(Frame {
            chunk,
            ip: 0,
            slots,
        });
    }

    /// The active (top) frame's chunk.
    fn active_chunk(&self) -> &super::op::Chunk {
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
                let v = self.lookup_name_value(env, &name);
                vm.stack.push(v);
                Ok(())
            }
            Op::CallName { name, argc } => {
                let name = resolve_name(vm.active_chunk(), name)?.to_string();
                self.vm_call_name(vm, env, &name, argc)
            }
            Op::Call { argc } => {
                let args = split_args(&mut vm.stack, argc as usize);
                let callee = pop(&mut vm.stack);
                self.vm_apply_function_value(vm, callee, args)
            }
            Op::Method { name, argc } => {
                let name = resolve_name(vm.active_chunk(), name)?.to_string();
                let args = split_args(&mut vm.stack, argc as usize);
                let receiver = pop(&mut vm.stack);
                let r = super::helpers::vm_method_value(self, env, receiver, &name, args)?;
                vm.stack.push(r);
                Ok(())
            }
            Op::MakeArray(n) => {
                let items = split_args(&mut vm.stack, n as usize);
                vm.stack.push(Value::Array(items));
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
                vm.stack.push(Value::Set(set));
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
                vm.stack.push(Value::Dict(dict));
                Ok(())
            }
            Op::Index => {
                let idx = pop(&mut vm.stack);
                let base = pop(&mut vm.stack);
                let v = super::helpers::vm_index(self, base, idx)?;
                vm.stack.push(v);
                Ok(())
            }
            Op::IndexStore => {
                let value = pop(&mut vm.stack);
                let idx = pop(&mut vm.stack);
                let base = pop(&mut vm.stack);
                super::helpers::vm_index_store(self, base, idx, value.clone())?;
                vm.stack.push(value);
                Ok(())
            }
            Op::Add | Op::Sub | Op::Mul | Op::Div | Op::Rem | Op::Pow => {
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
                let r = self
                    .eval_compare(cmp_binop(&op), a, b)
                    .map_err(|e| crate::error::attach_span(e, span))?;
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
                let r = super::helpers::vm_neg(self, a)?;
                vm.stack.push(r);
                Ok(())
            }
            Op::Not => {
                let a = pop(&mut vm.stack);
                vm.stack.push(Value::Bool(is_falsey(&a)));
                Ok(())
            }
            Op::Jump(off) => {
                if let Some(f) = vm.frames.last_mut() {
                    f.ip = jump_target(f.ip, off);
                }
                Ok(())
            }
            Op::JumpIfFalse(off) => {
                let cond = vm.stack.last().cloned().unwrap_or(Value::Nil);
                vm.stack.pop(); // the test always consumes the condition
                if is_falsey(&cond)
                    && let Some(f) = vm.frames.last_mut()
                {
                    f.ip = jump_target(f.ip, off);
                }
                Ok(())
            }
            Op::JumpIfTrue(off) => {
                let cond = vm.stack.last().cloned().unwrap_or(Value::Nil);
                if !is_falsey(&cond)
                    && let Some(f) = vm.frames.last_mut()
                {
                    f.ip = jump_target(f.ip, off);
                }
                Ok(())
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
            Op::LoadUpvalue(_) | Op::SetUpvalue(_) => Err(crate::error::RuntimeError::Message(
                "VM: upvalues not yet supported in this milestone".into(),
            )),
        }
    }

    /// Resolve a name to a value: an env value, or a fresh symbol for an unbound/global name (mirrors
    /// the interpreter's `Path` resolution).
    fn lookup_name_value(&mut self, env: &EnvRef, name: &str) -> Value {
        let env_r = env.borrow();
        if let Some(v) = env_r.get_value(name) {
            return v;
        }
        Value::Expr(self.pool.symbol(self.symbols.intern(name)))
    }

    /// Resolve a call-by-name and apply it. If the name maps to a compiled function chunk, recurse
    /// into it via a new frame; otherwise bind the arguments to the function's parameters through the
    /// AST path (which also handles builtins and collection convenience functions).
    fn vm_call_name(
        &mut self,
        vm: &mut Vm,
        env: &EnvRef,
        name: &str,
        argc: u16,
    ) -> Result<(), crate::error::RuntimeError> {
        if let Some(&idx) = vm.table.get(name) {
            let chunk = vm.functions[idx as usize].clone();
            // The chunk's leading `SetLocal`s consume the already-pushed args.
            vm.push_frame(chunk);
            return Ok(());
        }
        let func = env.borrow().get_func(name).ok_or_else(|| {
            crate::error::RuntimeError::Message(format!("unknown function `{name}`"))
        })?;
        let args = split_args(&mut vm.stack, argc as usize);
        let r = self.apply_function(&func, args)?;
        vm.stack.push(r);
        Ok(())
    }

    /// Apply a function *value* directly (for compiled `Call` sites that put a closure/function on
    /// the stack). Falls back to the general function-value application.
    fn vm_apply_function_value(
        &mut self,
        vm: &mut Vm,
        callee: Value,
        args: Vec<Value>,
    ) -> Result<(), crate::error::RuntimeError> {
        // Only reachable if the compiler emits a function value; the current subset never does, so
        // treat it as a runtime error to be safe.
        let _ = (vm, callee, args);
        Err(crate::error::RuntimeError::Message(
            "VM: function-value calls are not supported in this milestone".into(),
        ))
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

fn resolve_name(chunk: &super::op::Chunk, k: u16) -> Result<&str, crate::error::RuntimeError> {
    match chunk.constants.get(k as usize) {
        Some(Const::Name(s)) => Ok(s),
        _ => Err(crate::error::RuntimeError::Message(
            "invalid name constant".into(),
        )),
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

fn bool_binop(op: AstBinOp, a: Value, b: Value) -> Result<Value, crate::error::RuntimeError> {
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

impl Evaluator {
    /// VM-opt-in entry: attempt to run a function through the bytecode VM directly.
    pub(crate) fn try_vm_call(
        &mut self,
        func: &Function,
        args: Vec<Value>,
    ) -> Result<Option<Value>, RuntimeError> {
        let (params, body, f_env) = match func {
            Function::Host {
                params, body, env, ..
            } => (params, body, env),
            _ => return Ok(None),
        };
        self.try_vm_single(params, body, f_env, args)
    }

    /// Attempt to run a block-bodied function (`fn`) through the bytecode VM. Compiles the body into
    /// a chunk; returns `Ok(Some(value))` when the body compiles and runs, `Ok(None)` when the body is
    /// outside the compiled subset (caller falls back to the AST), or an error on a runtime failure.
    pub(crate) fn try_vm_single(
        &mut self,
        params: &[Param],
        body: &Block,
        f_env: &EnvRef,
        args: Vec<Value>,
    ) -> Result<Option<Value>, RuntimeError> {
        let chunk = match crate::vm::comp::compile_function_body(params, body) {
            Ok(c) => c,
            Err(_) => return Ok(None), // outside the compiled subset → AST fallback
        };
        let mut program = VmProgram {
            root: super::op::Chunk::new(),
            functions: vec![chunk],
            names: std::collections::HashMap::new(),
        };
        program.names.insert("__entry".to_string(), 0);
        // Parameters bind into slots by the chunk's leading `SetLocal`s, which consume the argument
        // values from the operand stack, so pass them there (and use the closure env for free names).
        match self.run_vm(f_env, &program, Some(("__entry", args))) {
            Ok(v) => Ok(Some(v)),
            // The VM is an optimization: any construct it cannot yet execute faithfully (e.g. a
            // mutating collection method that must write back to a binding) surfaces as an error in
            // the VM path, in which case we fall back to the authoritative AST interpreter.
            Err(_) => Ok(None),
        }
    }
}
