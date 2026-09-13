//! Cranelift engine (spec §19.2): compiles `Bytecode` into a native `extern "C"` function
//! `fn(*const f64, usize) -> f64` (argument buffer + arity). The compiled machine code lives in
//! process memory owned by a process-wide `JITModule` and never leaves the process (ExprId handles
//! are process-local, implementation plan §6).
//!
//! Transcendental and math operations (`Abs`, `Sqrt`, `Exp`, `Ln`, `Log10`, `Sin`, `Cos`, `Tan`,
//! `Pow`, `Rem`) are lowered to direct calls of registered Rust trampolines (`pj_*`). Each
//! trampoline is declared as a `Linkage::Import` function in the `JITModule` and its address is
//! registered in the builder's symbol table, so the generated code resolves the call without
//! relying on the platform's libm symbol names.

use std::mem;
use std::sync::{Arc, Mutex, OnceLock};

use cranelift_codegen::ir::immediates::Offset32;
use cranelift_codegen::ir::{AbiParam, InstBuilder, MemFlagsData, Signature, Type, Value, types};
use cranelift_codegen::isa::TargetIsa;
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{FuncId, Linkage, Module, default_libcall_names};

use crate::bytecode::{Bytecode, MAX_PARAMS, Op};
use crate::ir::ScalarType;

/// A compiled numeric scalar function: reads `arity` f64 arguments from the buffer and returns the
/// result. The `entry` pointer is executable machine code owned by the engine; call it from any
/// thread (the generated code has no shared mutable state).
pub struct CompiledScalar {
    /// Number of f64 arguments the entry point reads from the buffer.
    pub arity: usize,
    entry: unsafe extern "C" fn(*const f64, usize) -> f64,
}

impl CompiledScalar {
    /// Call the compiled function with exactly `arity` arguments (asserted).
    pub fn call(&self, args: &[f64]) -> f64 {
        assert_eq!(args.len(), self.arity, "compiled function arity mismatch");
        // SAFETY: the entry point is generated machine code that only reads `args.len()` f64 values
        // from the buffer and performs no unsafe memory access.
        unsafe { (self.entry)(args.as_ptr(), args.len()) }
    }
}

// ————————————————————— Rust trampolines —————————————————————

// The generated code calls these directly; keeping them in Rust (instead of relying on cranelift's
// libcall name resolution) makes the JIT output portable across platforms. They are exported with
// their exact symbol names so the `JITModule` symbol table and the process dlsym fallback agree.

#[unsafe(no_mangle)]
pub extern "C" fn pj_sin(x: f64) -> f64 {
    x.sin()
}

#[unsafe(no_mangle)]
pub extern "C" fn pj_cos(x: f64) -> f64 {
    x.cos()
}

#[unsafe(no_mangle)]
pub extern "C" fn pj_tan(x: f64) -> f64 {
    x.tan()
}

#[unsafe(no_mangle)]
pub extern "C" fn pj_exp(x: f64) -> f64 {
    x.exp()
}

#[unsafe(no_mangle)]
pub extern "C" fn pj_ln(x: f64) -> f64 {
    x.ln()
}

#[unsafe(no_mangle)]
pub extern "C" fn pj_log10(x: f64) -> f64 {
    x.log10()
}

#[unsafe(no_mangle)]
pub extern "C" fn pj_sqrt(x: f64) -> f64 {
    x.sqrt()
}

#[unsafe(no_mangle)]
pub extern "C" fn pj_abs(x: f64) -> f64 {
    x.abs()
}

/// `f64` remainder (`a % b`); cranelift 0.135 has no `frem` instruction, only integer `srem`/`urem`.
#[unsafe(no_mangle)]
pub extern "C" fn pj_rem(a: f64, b: f64) -> f64 {
    a % b
}

#[unsafe(no_mangle)]
pub extern "C" fn pj_pow(a: f64, b: f64) -> f64 {
    a.powf(b)
}

// ————————————————————— engine —————————————————————

/// The process-wide JIT engine: owns the `JITModule` so compiled code stays alive for the whole
/// process (addresses are stored in `CompiledScalar.entry`). Compilation is serialized under a
/// `Mutex` (cranelift `JITModule` compilation is not thread-safe); `CompiledScalar::call` is
/// lock-free. `None` means engine initialization failed — the JIT is then permanently unavailable
/// for the process and every compilation degrades to the interpreter fallback.
static ENGINE: OnceLock<Option<Mutex<JitEngine>>> = OnceLock::new();

/// Declared `FuncId`s of the trampoline imports, resolved once per engine lifetime.
#[derive(Clone, Copy)]
struct Trampolines {
    rem: FuncId,
    pow: FuncId,
    abs: FuncId,
    sqrt: FuncId,
    exp: FuncId,
    ln: FuncId,
    log10: FuncId,
    sin: FuncId,
    cos: FuncId,
    tan: FuncId,
}

/// Trampolines used by whole-function JIT code (arena + dense-array support, see `rt.rs`).
#[derive(Clone, Copy)]
pub(crate) struct FuncTrampolines {
    pub arena_new: FuncId,
    pub arena_alloc: FuncId,
    pub arena_free: FuncId,
    pub memcpy: FuncId,
    pub fill_u8: FuncId,
    pub fill_i64: FuncId,
    pub fill_f64: FuncId,
    pub i64_rem: FuncId,
    pub f64_rem: FuncId,
    pub cancel: FuncId,
}

pub(crate) struct JitEngine {
    pub(crate) module: JITModule,
    tramps: Trampolines,
    pub(crate) func: FuncTrampolines,
}

/// A mutable per-call context handed to JIT code. Generated code sets `error` and returns a default
/// when it hits an exact-arithmetic overflow or an out-of-range index; the caller then re-runs the
/// call on the interpreter (the JIT is pure, so re-running cannot duplicate side effects).
#[derive(Default)]
pub struct JitContext {
    pub error: u32,
}

/// The raw entry point of a compiled function: `(ctx, args, len) -> result`, with every argument
/// and the result passed as 64-bit words (i64, or f64 bits, or bool as 0/1).
pub type JitEntry = unsafe extern "C" fn(*mut JitContext, *const u64, usize) -> u64;

/// A compiled whole function.
pub struct CompiledFunction {
    pub arity: usize,
    pub params: Vec<ScalarType>,
    pub ret: ScalarType,
    pub(crate) entry: JitEntry,
}

impl CompiledFunction {
    /// Call the compiled function. `args` must hold one word per parameter (i64 / f64 bits / bool),
    /// in order; the result is returned as a word (interpret per [`Self::ret`]).
    pub fn call_raw(&self, ctx: &mut JitContext, args: &[u64]) -> u64 {
        debug_assert_eq!(args.len(), self.arity);
        // SAFETY: the entry point is generated machine code with the declared `(ctx, args, len)`
        // ABI; it only reads `args.len()` words from the buffer.
        unsafe { (self.entry)(ctx, args.as_ptr(), args.len()) }
    }
}

/// Run `f` with the locked process-wide JIT engine, or `None` when the engine is unavailable.
pub(crate) fn with_engine<R>(f: impl FnOnce(&mut JitEngine) -> Option<R>) -> Option<R> {
    let engine = ENGINE.get_or_init(|| JitEngine::init().map(Mutex::new));
    let mut guard = engine.as_ref()?.lock().ok()?;
    f(&mut guard)
}

fn unary_signature(isa: &dyn TargetIsa) -> Signature {
    let mut sig = Signature::new(isa.default_call_conv());
    sig.params.push(AbiParam::new(types::F64));
    sig.returns.push(AbiParam::new(types::F64));
    sig
}

fn binary_signature(isa: &dyn TargetIsa) -> Signature {
    let mut sig = Signature::new(isa.default_call_conv());
    sig.params.push(AbiParam::new(types::F64));
    sig.params.push(AbiParam::new(types::F64));
    sig.returns.push(AbiParam::new(types::F64));
    sig
}

/// Cast an `extern "C"` trampoline to a raw byte address for the builder's symbol table.
fn unary_addr(f: extern "C" fn(f64) -> f64) -> *const u8 {
    f as *const u8
}

/// Cast a binary trampoline to a raw byte address for the builder's symbol table.
fn binary_addr(f: extern "C" fn(f64, f64) -> f64) -> *const u8 {
    f as *const u8
}

/// Build a cranelift signature from parameter/return types.
fn build_sig(isa: &dyn TargetIsa, params: &[Type], ret: Option<Type>) -> Signature {
    let mut s = Signature::new(isa.default_call_conv());
    for p in params {
        s.params.push(AbiParam::new(*p));
    }
    if let Some(r) = ret {
        s.returns.push(AbiParam::new(r));
    }
    s
}

impl JitEngine {
    /// Build the engine, or `None` when cranelift cannot initialize (no panic across the API —
    /// the JIT is optional and callers degrade to the interpreter).
    fn init() -> Option<JitEngine> {
        let mut builder = JITBuilder::new(default_libcall_names()).ok()?;
        // Register the trampoline addresses so the imports resolve deterministically from the
        // builder's own symbol table (the fallback is a platform dlsym search of the process).
        builder
            .symbol("pj_sin", unary_addr(pj_sin))
            .symbol("pj_cos", unary_addr(pj_cos))
            .symbol("pj_tan", unary_addr(pj_tan))
            .symbol("pj_exp", unary_addr(pj_exp))
            .symbol("pj_ln", unary_addr(pj_ln))
            .symbol("pj_log10", unary_addr(pj_log10))
            .symbol("pj_sqrt", unary_addr(pj_sqrt))
            .symbol("pj_abs", unary_addr(pj_abs))
            .symbol("pj_rem", binary_addr(pj_rem))
            .symbol("pj_pow", binary_addr(pj_pow))
            .symbol(
                "pj_arena_new",
                crate::rt::pj_arena_new as *const () as *const u8,
            )
            .symbol(
                "pj_arena_alloc",
                crate::rt::pj_arena_alloc as *const () as *const u8,
            )
            .symbol(
                "pj_arena_free",
                crate::rt::pj_arena_free as *const () as *const u8,
            )
            .symbol("pj_memcpy", crate::rt::pj_memcpy as *const () as *const u8)
            .symbol(
                "pj_fill_u8",
                crate::rt::pj_fill_u8 as *const () as *const u8,
            )
            .symbol(
                "pj_fill_i64",
                crate::rt::pj_fill_i64 as *const () as *const u8,
            )
            .symbol(
                "pj_fill_f64",
                crate::rt::pj_fill_f64 as *const () as *const u8,
            )
            .symbol(
                "pj_i64_rem",
                crate::rt::pj_i64_rem as *const () as *const u8,
            )
            .symbol(
                "pj_check_cancel",
                crate::rt::pj_check_cancel as *const () as *const u8,
            );
        let mut module = JITModule::new(builder);
        let unary = unary_signature(module.isa());
        let binary = binary_signature(module.isa());
        let declare = |module: &mut JITModule, name: &str, sig: &Signature| -> Option<FuncId> {
            module.declare_function(name, Linkage::Import, sig).ok()
        };
        let sin = declare(&mut module, "pj_sin", &unary)?;
        let cos = declare(&mut module, "pj_cos", &unary)?;
        let tan = declare(&mut module, "pj_tan", &unary)?;
        let exp = declare(&mut module, "pj_exp", &unary)?;
        let ln = declare(&mut module, "pj_ln", &unary)?;
        let log10 = declare(&mut module, "pj_log10", &unary)?;
        let sqrt = declare(&mut module, "pj_sqrt", &unary)?;
        let abs = declare(&mut module, "pj_abs", &unary)?;
        let rem = declare(&mut module, "pj_rem", &binary)?;
        let pow = declare(&mut module, "pj_pow", &binary)?;

        // Whole-function trampolines (arena + dense arrays).
        let ptr = module.isa().pointer_type();
        let s_arena_new = build_sig(module.isa(), &[], Some(ptr));
        let s_arena_alloc = build_sig(module.isa(), &[ptr, types::I64], Some(ptr));
        let s_arena_free = build_sig(module.isa(), &[ptr], None);
        let s_memcpy = build_sig(module.isa(), &[ptr, ptr, types::I64], None);
        let s_fill_u8 = build_sig(module.isa(), &[ptr, types::I64, types::I8], None);
        let s_fill_i64 = build_sig(module.isa(), &[ptr, types::I64, types::I64], None);
        let s_fill_f64 = build_sig(module.isa(), &[ptr, types::I64, types::F64], None);
        let s_i64_rem = build_sig(module.isa(), &[types::I64, types::I64], Some(types::I64));
        let s_cancel = build_sig(module.isa(), &[], Some(types::I8));
        let func = FuncTrampolines {
            arena_new: declare(&mut module, "pj_arena_new", &s_arena_new)?,
            arena_alloc: declare(&mut module, "pj_arena_alloc", &s_arena_alloc)?,
            arena_free: declare(&mut module, "pj_arena_free", &s_arena_free)?,
            memcpy: declare(&mut module, "pj_memcpy", &s_memcpy)?,
            fill_u8: declare(&mut module, "pj_fill_u8", &s_fill_u8)?,
            fill_i64: declare(&mut module, "pj_fill_i64", &s_fill_i64)?,
            fill_f64: declare(&mut module, "pj_fill_f64", &s_fill_f64)?,
            i64_rem: declare(&mut module, "pj_i64_rem", &s_i64_rem)?,
            f64_rem: rem,
            cancel: declare(&mut module, "pj_check_cancel", &s_cancel)?,
        };
        Some(JitEngine {
            module,
            tramps: Trampolines {
                rem,
                pow,
                abs,
                sqrt,
                exp,
                ln,
                log10,
                sin,
                cos,
                tan,
            },
            func,
        })
    }
}

/// Check that the bytecode is a well-formed straight-line stack program for `arity` parameters:
/// the arity is representable (`Op::Param` indexes with a `u8`), every `Param` index is in range,
/// the stack never underflows, and the program leaves one result.
fn validate_bytecode(bc: &Bytecode, arity: usize) -> Option<()> {
    if arity > MAX_PARAMS {
        return None;
    }
    let mut height = 0usize;
    for op in &bc.0 {
        match op {
            Op::Const(_) => height += 1,
            Op::Param(i) => {
                if (*i as usize) >= arity {
                    return None;
                }
                height += 1;
            }
            Op::Neg
            | Op::Abs
            | Op::Sqrt
            | Op::Exp
            | Op::Ln
            | Op::Log10
            | Op::Sin
            | Op::Cos
            | Op::Tan => {
                if height == 0 {
                    return None;
                }
            }
            Op::Add | Op::Sub | Op::Mul | Op::Div | Op::Rem | Op::Pow => {
                if height < 2 {
                    return None;
                }
                height -= 1;
            }
        }
    }
    (height == 1).then_some(())
}

/// Compile a bytecode program into a `CompiledScalar`, or `None` if cranelift fails to lower it.
pub fn compile_bytecode(bc: &Bytecode, arity: usize) -> Option<Arc<CompiledScalar>> {
    validate_bytecode(bc, arity)?;

    let engine = ENGINE.get_or_init(|| JitEngine::init().map(Mutex::new));
    // A `None` engine means initialization failed earlier; the JIT stays permanently unavailable.
    let engine = engine.as_ref()?;
    let mut engine = engine.lock().ok()?;
    let JitEngine { module, tramps, .. } = &mut *engine;
    let tramps = *tramps;

    let isa = module.isa();
    let ptr: Type = isa.pointer_type();
    let frontend_config = isa.frontend_config();

    let mut sig = Signature::new(isa.default_call_conv());
    sig.params.push(AbiParam::new(ptr));
    sig.params.push(AbiParam::new(ptr));
    sig.returns.push(AbiParam::new(types::F64));

    let func_id = module.declare_anonymous_function(&sig).ok()?;

    let mut ctx = module.make_context();
    ctx.func.signature = sig;
    let mut func_ctx = FunctionBuilderContext::new();
    let mut builder = FunctionBuilder::new(&mut ctx.func, &mut func_ctx);

    let entry_block = builder.create_block();
    builder.switch_to_block(entry_block);
    builder.append_block_params_for_function_params(entry_block);
    // Function params are `(ptr, len)`; only the pointer is used (arity is fixed at compile time).
    let buffer = builder.func.dfg.block_params(entry_block)[0];

    let mut stack: Vec<Value> = Vec::new();
    for op in &bc.0 {
        let v = emit_op(&mut builder, module, tramps, *op, buffer, &mut stack);
        stack.push(v);
    }
    let result = stack.pop().expect("validated: stack holds one result");
    builder.ins().return_(&[result]);
    builder.seal_block(entry_block);
    builder.finalize(frontend_config);

    module.define_function(func_id, &mut ctx).ok()?;
    module.finalize_definitions().ok()?;

    let code = module.get_finalized_function(func_id);
    // SAFETY: `code` points at executable machine code with ABI `(ptr, len) -> f64` matching the
    // declared signature; `CompiledScalar::call` upholds the buffer contract.
    let entry: unsafe extern "C" fn(*const f64, usize) -> f64 = unsafe { mem::transmute(code) };
    Some(Arc::new(CompiledScalar { arity, entry }))
}

/// Emit one bytecode op: pop its operands from `stack` and return the result value to push.
fn emit_op(
    builder: &mut FunctionBuilder,
    module: &mut JITModule,
    tramps: Trampolines,
    op: Op,
    buffer: Value,
    stack: &mut Vec<Value>,
) -> Value {
    match op {
        Op::Const(x) => builder.ins().f64const(x),
        Op::Param(i) => {
            // Byte offset of slot `i` in the f64 argument buffer; computed in `i32` so the multiply
            // never overflows a narrower domain (a `u8` arithmetic would wrap at i >= 32).
            let offset = i32::from(i) * 8;
            builder.ins().load(
                types::F64,
                MemFlagsData::new(),
                buffer,
                Offset32::new(offset),
            )
        }
        Op::Neg => {
            let x = pop1(stack);
            builder.ins().fneg(x)
        }
        Op::Add => binop(|a, b| builder.ins().fadd(a, b), stack),
        Op::Sub => binop(|a, b| builder.ins().fsub(a, b), stack),
        Op::Mul => binop(|a, b| builder.ins().fmul(a, b), stack),
        Op::Div => binop(|a, b| builder.ins().fdiv(a, b), stack),
        Op::Rem => {
            let (a, b) = pop2(stack);
            call_tramp(builder, module, tramps.rem, &[a, b])
        }
        Op::Pow => {
            let (a, b) = pop2(stack);
            call_tramp(builder, module, tramps.pow, &[a, b])
        }
        Op::Abs => call_unary(builder, module, tramps.abs, pop1(stack)),
        Op::Sqrt => call_unary(builder, module, tramps.sqrt, pop1(stack)),
        Op::Exp => call_unary(builder, module, tramps.exp, pop1(stack)),
        Op::Ln => call_unary(builder, module, tramps.ln, pop1(stack)),
        Op::Log10 => call_unary(builder, module, tramps.log10, pop1(stack)),
        Op::Sin => call_unary(builder, module, tramps.sin, pop1(stack)),
        Op::Cos => call_unary(builder, module, tramps.cos, pop1(stack)),
        Op::Tan => call_unary(builder, module, tramps.tan, pop1(stack)),
    }
}

fn pop1(stack: &mut Vec<Value>) -> Value {
    stack.pop().expect("validated: stack has an operand")
}

fn pop2(stack: &mut Vec<Value>) -> (Value, Value) {
    let b = stack.pop().expect("validated: stack has two operands");
    let a = stack.pop().expect("validated: stack has two operands");
    (a, b)
}

/// Fold `f(a, b)` where `a` was pushed before `b` (top of stack is `b`).
fn binop(f: impl FnOnce(Value, Value) -> Value, stack: &mut Vec<Value>) -> Value {
    let (a, b) = pop2(stack);
    f(a, b)
}

/// Emit a direct call to an imported trampoline and return its single f64 result.
fn call_tramp(
    builder: &mut FunctionBuilder,
    module: &mut JITModule,
    func_id: FuncId,
    args: &[Value],
) -> Value {
    let func_ref = module.declare_func_in_func(func_id, builder.func);
    let inst = builder.ins().call(func_ref, args);
    builder.func.dfg.first_result(inst)
}

fn call_unary(
    builder: &mut FunctionBuilder,
    module: &mut JITModule,
    func_id: FuncId,
    x: Value,
) -> Value {
    call_tramp(builder, module, func_id, &[x])
}
