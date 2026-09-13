//! Whole-function JIT: lower a typed [`IrFunction`] to cranelift machine code (spec §19.2).
//!
//! The generated function has ABI `(ctx, args, len) -> u64`: scalars cross the boundary as raw
//! 64-bit words (i64, f64 bits, or bool as 0/1). Integer arithmetic is checked and array accesses
//! are bounds-checked; on an overflow or out-of-range index the generated code sets `ctx.error`
//! and returns, and the caller re-runs the call on the interpreter (the JIT is pure).
//!
//! Dense arrays are `{ data, len, cap }` headers allocated from a per-call [`crate::rt::Arena`]
//! that is freed at the single return epilogue.

use std::collections::HashMap;
use std::sync::Arc;

use cranelift_codegen::ir::condcodes::{FloatCC, IntCC};
use cranelift_codegen::ir::immediates::Offset32;
use cranelift_codegen::ir::{
    AbiParam, Block, InstBuilder, MemFlagsData, Signature, Type, Value, types,
};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext, Variable};
use cranelift_jit::JITModule;
use cranelift_module::{FuncId, Module};

use crate::engine::{CompiledFunction, FuncTrampolines, JitEngine, JitEntry};
use crate::ir::{CmpOp, ElemType, FloatOp, IntOp, IrFunction, IrOp, ScalarType, SlotType};

/// Array header field offsets.
const HDR_DATA: i32 = 0;
const HDR_LEN: i32 = 8;
const HDR_CAP: i32 = 16;
const HDR_BYTES: i64 = 24;

/// Compile an IR function, or `None` when cranelift fails to lower it.
pub fn compile_ir(ir: &IrFunction) -> Option<Arc<CompiledFunction>> {
    crate::engine::with_engine(|engine| compile_inner(engine, ir))
}

fn scalar_ty(t: ScalarType) -> Type {
    match t {
        ScalarType::I64 => types::I64,
        ScalarType::F64 => types::F64,
        ScalarType::Bool => types::I8,
    }
}

fn elem_ty(e: ElemType) -> Type {
    match e {
        ElemType::I64 => types::I64,
        ElemType::F64 => types::F64,
        ElemType::Bool => types::I8,
    }
}

fn elem_size(e: ElemType) -> i64 {
    match e {
        ElemType::Bool => 1,
        ElemType::I64 | ElemType::F64 => 8,
    }
}

/// Compiler state shared by the op emitters. `module` and the `FunctionBuilder` borrow disjoint
/// objects (the `JITModule` vs. the local cranelift `Context`), so both can be held mutably.
struct Ctx<'a> {
    module: &'a mut JITModule,
    slots: &'a [SlotType],
    vars: Vec<Variable>,
    ret_var: Variable,
    ctx_var: Variable,
    arena_var: Variable,
    epilogue: Block,
    error_block: Block,
    blocks: HashMap<usize, Block>,
    all_blocks: Vec<Block>,
    func: FuncTrampolines,
    ret: ScalarType,
}

impl Ctx<'_> {
    fn block(&mut self, b: &mut FunctionBuilder) -> Block {
        let blk = b.create_block();
        self.all_blocks.push(blk);
        blk
    }
}

fn compile_inner(engine: &mut JitEngine, ir: &IrFunction) -> Option<Arc<CompiledFunction>> {
    let isa = engine.module.isa();
    let ptr = isa.pointer_type();
    let frontend_config = isa.frontend_config();

    let mut sig = Signature::new(isa.default_call_conv());
    sig.params.push(AbiParam::new(ptr)); // ctx
    sig.params.push(AbiParam::new(ptr)); // args
    sig.params.push(AbiParam::new(types::I64)); // len
    sig.returns.push(AbiParam::new(types::I64)); // raw result word

    let func_id = engine.module.declare_anonymous_function(&sig).ok()?;
    let mut context = engine.module.make_context();
    context.func.signature = sig;
    let mut func_ctx = FunctionBuilderContext::new();
    let mut b = FunctionBuilder::new(&mut context.func, &mut func_ctx);

    let leaders = compute_leaders(&ir.ops);
    let entry = b.create_block();
    let epilogue = b.create_block();
    let error_block = b.create_block();
    let mut blocks: HashMap<usize, Block> = HashMap::new();
    let mut all_blocks = vec![entry, epilogue, error_block];
    for (i, &l) in leaders.iter().enumerate() {
        let blk = if i == 0 { entry } else { b.create_block() };
        blocks.insert(l, blk);
        if i != 0 {
            all_blocks.push(blk);
        }
    }

    // ————— entry prologue (variables are declared in the entry block) —————
    b.switch_to_block(entry);
    b.append_block_params_for_function_params(entry);
    let (ctx_ptr, args_ptr) = {
        let p = b.func.dfg.block_params(entry);
        (p[0], p[1])
    };
    let ctx_var = b.declare_var(ptr);
    let ret_var = b.declare_var(types::I64);
    let arena_var = b.declare_var(ptr);
    let vars: Vec<Variable> = ir
        .slots
        .iter()
        .map(|st| {
            let ty = match st {
                SlotType::Scalar(s) => scalar_ty(*s),
                SlotType::Array(_) => ptr,
            };
            b.declare_var(ty)
        })
        .collect();
    b.def_var(ctx_var, ctx_ptr);

    let mut c = Ctx {
        module: &mut engine.module,
        slots: &ir.slots,
        vars,
        ret_var,
        ctx_var,
        arena_var,
        epilogue,
        error_block,
        blocks,
        all_blocks,
        func: engine.func,
        ret: ir.ret,
    };

    let arena = call_ret(&mut b, c.module, c.func.arena_new, &[]);
    b.def_var(c.arena_var, arena);
    for (i, pty) in ir.params.iter().enumerate() {
        let off = (i as i32) * 8;
        let v = b.ins().load(
            scalar_ty(*pty),
            MemFlagsData::new(),
            args_ptr,
            Offset32::new(off),
        );
        b.def_var(c.vars[i], v);
    }

    // ————— emit leader ranges —————
    for (k, &l) in leaders.iter().enumerate() {
        let blk = c.blocks[&l];
        if k != 0 {
            b.switch_to_block(blk);
        }
        let end = leaders.get(k + 1).copied().unwrap_or(ir.ops.len());
        let mut terminated = false;
        for (offset, op) in ir.ops[l..end].iter().enumerate() {
            emit_op(&mut b, &mut c, op, l + offset);
            if is_terminator(op) {
                terminated = true;
                break;
            }
        }
        if !terminated {
            let target = leaders
                .get(k + 1)
                .copied()
                .map(|n| c.blocks[&n])
                .unwrap_or(c.epilogue);
            b.ins().jump(target, &[]);
        }
    }

    // ————— epilogue —————
    b.switch_to_block(epilogue);
    let arena = b.use_var(c.arena_var);
    call_void(&mut b, c.module, c.func.arena_free, &[arena]);
    let ret = b.use_var(c.ret_var);
    b.ins().return_(&[ret]);

    // ————— error —————
    b.switch_to_block(error_block);
    let ctx_ptr = b.use_var(c.ctx_var);
    let one = b.ins().iconst(types::I32, 1);
    b.ins()
        .store(MemFlagsData::new(), one, ctx_ptr, Offset32::new(0));
    let arena = b.use_var(c.arena_var);
    call_void(&mut b, c.module, c.func.arena_free, &[arena]);
    let zero = b.ins().iconst(types::I64, 0);
    b.ins().return_(&[zero]);

    for blk in c.all_blocks {
        b.seal_block(blk);
    }
    b.finalize(frontend_config);

    engine.module.define_function(func_id, &mut context).ok()?;
    engine.module.finalize_definitions().ok()?;
    let code = engine.module.get_finalized_function(func_id);
    // SAFETY: `code` is executable machine code with the declared `(ptr, ptr, i64) -> i64` ABI.
    let entry: JitEntry = unsafe { std::mem::transmute(code) };
    Some(Arc::new(CompiledFunction {
        arity: ir.arity(),
        params: ir.params.clone(),
        ret: ir.ret,
        entry,
    }))
}

fn compute_leaders(ops: &[IrOp]) -> Vec<usize> {
    let mut set = std::collections::BTreeSet::new();
    set.insert(0usize);
    for (i, op) in ops.iter().enumerate() {
        match op {
            IrOp::Jump { target } => {
                set.insert(*target as usize);
                set.insert(i + 1);
            }
            IrOp::BranchBool { target, .. } => {
                set.insert(*target as usize);
                set.insert(i + 1);
            }
            _ => {}
        }
    }
    set.into_iter().filter(|&x| x < ops.len()).collect()
}

fn is_terminator(op: &IrOp) -> bool {
    matches!(
        op,
        IrOp::Jump { .. } | IrOp::BranchBool { .. } | IrOp::Return { .. }
    )
}

fn get(b: &mut FunctionBuilder, c: &Ctx, slot: u16) -> Value {
    b.use_var(c.vars[slot as usize])
}

fn set(b: &mut FunctionBuilder, c: &Ctx, slot: u16, v: Value) {
    b.def_var(c.vars[slot as usize], v);
}

fn call_ret(b: &mut FunctionBuilder, module: &mut JITModule, id: FuncId, args: &[Value]) -> Value {
    let fref = module.declare_func_in_func(id, b.func);
    let inst = b.ins().call(fref, args);
    b.func.dfg.inst_results(inst)[0]
}

fn call_void(b: &mut FunctionBuilder, module: &mut JITModule, id: FuncId, args: &[Value]) {
    let fref = module.declare_func_in_func(id, b.func);
    b.ins().call(fref, args);
}

/// Branch to the error block when `cond` is true, continuing in a fresh block.
fn guard(b: &mut FunctionBuilder, c: &mut Ctx, cond: Value) {
    let cont = c.block(b);
    b.ins().brif(cond, c.error_block, &[], cont, &[]);
    b.switch_to_block(cont);
}

fn emit_op(b: &mut FunctionBuilder, c: &mut Ctx, op: &IrOp, index: usize) {
    match op {
        IrOp::ConstI64 { dst, v } => {
            let x = b.ins().iconst(types::I64, *v);
            set(b, c, *dst, x);
        }
        IrOp::ConstF64 { dst, v } => {
            let x = b.ins().f64const(*v);
            set(b, c, *dst, x);
        }
        IrOp::ConstBool { dst, v } => {
            let x = b.ins().iconst(types::I8, i64::from(*v));
            set(b, c, *dst, x);
        }
        IrOp::Copy { dst, src } => {
            let x = get(b, c, *src);
            set(b, c, *dst, x);
        }
        IrOp::IntBin { dst, a, b: rhs, op } => {
            let x = get(b, c, *a);
            let y = get(b, c, *rhs);
            let r = int_bin(b, c, x, y, *op);
            set(b, c, *dst, r);
        }
        IrOp::IntBinImm { dst, a, v, op } => {
            let x = get(b, c, *a);
            let y = b.ins().iconst(types::I64, *v);
            let r = int_bin(b, c, x, y, *op);
            set(b, c, *dst, r);
        }
        IrOp::FloatBin { dst, a, b: rhs, op } => {
            let x = get(b, c, *a);
            let y = get(b, c, *rhs);
            let r = float_bin(b, c, x, y, *op);
            set(b, c, *dst, r);
        }
        IrOp::FloatBinImm { dst, a, v, op } => {
            let x = get(b, c, *a);
            let y = b.ins().f64const(*v);
            let r = float_bin(b, c, x, y, *op);
            set(b, c, *dst, r);
        }
        IrOp::IntNeg { dst, src } => {
            let x = get(b, c, *src);
            let r = b.ins().ineg(x);
            set(b, c, *dst, r);
        }
        IrOp::FloatNeg { dst, src } => {
            let x = get(b, c, *src);
            let r = b.ins().fneg(x);
            set(b, c, *dst, r);
        }
        IrOp::I64ToF64 { dst, src } => {
            let x = get(b, c, *src);
            let r = b.ins().fcvt_from_sint(types::F64, x);
            set(b, c, *dst, r);
        }
        IrOp::IntCmp { dst, a, b: rhs, op } => {
            let x = get(b, c, *a);
            let y = get(b, c, *rhs);
            let r = b.ins().icmp(int_cc(*op), x, y);
            set(b, c, *dst, r);
        }
        IrOp::FloatCmp { dst, a, b: rhs, op } => {
            let x = get(b, c, *a);
            let y = get(b, c, *rhs);
            // NaN comparisons are an error in the interpreter (`partial_cmp` is `None`).
            let unordered = b.ins().fcmp(FloatCC::Unordered, x, y);
            guard(b, c, unordered);
            let r = b.ins().fcmp(float_cc(*op), x, y);
            set(b, c, *dst, r);
        }
        IrOp::BoolNot { dst, src } => {
            let x = get(b, c, *src);
            let zero = b.ins().iconst(types::I8, 0);
            let r = b.ins().icmp(IntCC::Equal, x, zero);
            set(b, c, *dst, r);
        }
        IrOp::Jump { target } => {
            // Poll host cancellation at loop back-edges (spec §16), matching the interpreter's
            // per-iteration check; a request deopts so the interpreter reports "interrupted".
            if (*target as usize) <= index {
                let flag = call_ret(b, c.module, c.func.cancel, &[]);
                let zero = b.ins().iconst(types::I8, 0);
                let cancelled = b.ins().icmp(IntCC::NotEqual, flag, zero);
                guard(b, c, cancelled);
            }
            let blk = c.blocks[&(*target as usize)];
            b.ins().jump(blk, &[]);
        }
        IrOp::BranchBool {
            cond,
            negate,
            target,
        } => {
            let x = get(b, c, *cond);
            let target_blk = c.blocks[&(*target as usize)];
            // The fall-through is the next instruction, which the leader analysis always makes a
            // block (or the epilogue at the very end).
            let fallthrough = c.blocks.get(&(index + 1)).copied().unwrap_or(c.epilogue);
            if *negate {
                b.ins().brif(x, fallthrough, &[], target_blk, &[]);
            } else {
                b.ins().brif(x, target_blk, &[], fallthrough, &[]);
            }
            // The block is terminated; the main loop switches to the next leader (the fall-through).
        }
        IrOp::Return { src } => {
            let v = get(b, c, *src);
            let raw = match c.ret {
                ScalarType::I64 => v,
                ScalarType::F64 => b.ins().bitcast(types::I64, MemFlagsData::new(), v),
                ScalarType::Bool => b.ins().uextend(types::I64, v),
            };
            b.def_var(c.ret_var, raw);
            b.ins().jump(c.epilogue, &[]);
        }
        IrOp::NewArray { dst, .. } => {
            let size = b.ins().iconst(types::I64, HDR_BYTES);
            let arena = b.use_var(c.arena_var);
            let hdr = call_ret(b, c.module, c.func.arena_alloc, &[arena, size]);
            let zero = b.ins().iconst(types::I64, 0);
            b.ins()
                .store(MemFlagsData::new(), zero, hdr, Offset32::new(HDR_DATA));
            b.ins()
                .store(MemFlagsData::new(), zero, hdr, Offset32::new(HDR_LEN));
            b.ins()
                .store(MemFlagsData::new(), zero, hdr, Offset32::new(HDR_CAP));
            set(b, c, *dst, hdr);
        }
        IrOp::ArrayLen { dst, arr } => {
            let hdr = get(b, c, *arr);
            let len = b
                .ins()
                .load(types::I64, MemFlagsData::new(), hdr, Offset32::new(HDR_LEN));
            set(b, c, *dst, len);
        }
        IrOp::ArrayPush { arr, src } => {
            let hdr = get(b, c, *arr);
            let v = get(b, c, *src);
            let elem = elem_of(c, *arr);
            array_push(b, c, hdr, v, elem);
        }
        IrOp::ArrayPushI64Imm { arr, v } => {
            let hdr = get(b, c, *arr);
            let x = b.ins().iconst(types::I64, *v);
            array_push(b, c, hdr, x, ElemType::I64);
        }
        IrOp::ArrayPushF64Imm { arr, v } => {
            let hdr = get(b, c, *arr);
            let x = b.ins().f64const(*v);
            array_push(b, c, hdr, x, ElemType::F64);
        }
        IrOp::ArrayPushBoolImm { arr, v } => {
            let hdr = get(b, c, *arr);
            let x = b.ins().iconst(types::I8, i64::from(*v));
            array_push(b, c, hdr, x, ElemType::Bool);
        }
        IrOp::ArrayFillI64 { arr, count, v } => {
            let hdr = get(b, c, *arr);
            let n = get(b, c, *count);
            let x = b.ins().iconst(types::I64, *v);
            array_fill(b, c, hdr, n, x, ElemType::I64);
        }
        IrOp::ArrayFillF64 { arr, count, v } => {
            let hdr = get(b, c, *arr);
            let n = get(b, c, *count);
            let x = b.ins().f64const(*v);
            array_fill(b, c, hdr, n, x, ElemType::F64);
        }
        IrOp::ArrayFillBool { arr, count, v } => {
            let hdr = get(b, c, *arr);
            let n = get(b, c, *count);
            let x = b.ins().iconst(types::I8, i64::from(*v));
            array_fill(b, c, hdr, n, x, ElemType::Bool);
        }
        IrOp::ArrayGet { dst, arr, idx } => {
            let hdr = get(b, c, *arr);
            let i = get(b, c, *idx);
            let elem = elem_of(c, *arr);
            let addr = array_addr(b, c, hdr, i, elem);
            let v = b
                .ins()
                .load(elem_ty(elem), MemFlagsData::new(), addr, Offset32::new(0));
            set(b, c, *dst, v);
        }
        IrOp::ArraySet { arr, idx, src } => {
            let hdr = get(b, c, *arr);
            let i = get(b, c, *idx);
            let v = get(b, c, *src);
            let elem = elem_of(c, *arr);
            let addr = array_addr(b, c, hdr, i, elem);
            b.ins()
                .store(MemFlagsData::new(), v, addr, Offset32::new(0));
        }
        IrOp::ArraySetI64Imm { arr, idx, v } => {
            let hdr = get(b, c, *arr);
            let i = get(b, c, *idx);
            let x = b.ins().iconst(types::I64, *v);
            let addr = array_addr(b, c, hdr, i, ElemType::I64);
            b.ins()
                .store(MemFlagsData::new(), x, addr, Offset32::new(0));
        }
        IrOp::ArraySetF64Imm { arr, idx, v } => {
            let hdr = get(b, c, *arr);
            let i = get(b, c, *idx);
            let x = b.ins().f64const(*v);
            let addr = array_addr(b, c, hdr, i, ElemType::F64);
            b.ins()
                .store(MemFlagsData::new(), x, addr, Offset32::new(0));
        }
        IrOp::ArraySetBoolImm { arr, idx, v } => {
            let hdr = get(b, c, *arr);
            let i = get(b, c, *idx);
            let x = b.ins().iconst(types::I8, i64::from(*v));
            let addr = array_addr(b, c, hdr, i, ElemType::Bool);
            b.ins()
                .store(MemFlagsData::new(), x, addr, Offset32::new(0));
        }
    }
}

/// The element type of array slot `arr` (recorded in the IR slot table).
fn elem_of(c: &Ctx, arr: u16) -> ElemType {
    match c.slots[arr as usize] {
        SlotType::Array(e) => e,
        SlotType::Scalar(_) => unreachable!("array op on a scalar slot"),
    }
}

/// Checked integer arithmetic.
fn int_bin(b: &mut FunctionBuilder, c: &mut Ctx, x: Value, y: Value, op: IntOp) -> Value {
    match op {
        IntOp::Add | IntOp::Sub | IntOp::Mul => {
            let (sum, of) = match op {
                IntOp::Add => b.ins().sadd_overflow(x, y),
                IntOp::Sub => b.ins().ssub_overflow(x, y),
                _ => b.ins().smul_overflow(x, y),
            };
            let zero = b.ins().iconst(types::I8, 0);
            let overflow = b.ins().icmp(IntCC::NotEqual, of, zero);
            guard(b, c, overflow);
            sum
        }
        IntOp::Rem => {
            let zero = b.ins().iconst(types::I64, 0);
            let is_zero = b.ins().icmp(IntCC::Equal, y, zero);
            guard(b, c, is_zero);
            call_ret(b, c.module, c.func.i64_rem, &[x, y])
        }
    }
}

/// Float arithmetic (IEEE; division by zero yields inf, as the interpreter does for `F64`).
fn float_bin(b: &mut FunctionBuilder, c: &mut Ctx, x: Value, y: Value, op: FloatOp) -> Value {
    match op {
        FloatOp::Add => b.ins().fadd(x, y),
        FloatOp::Sub => b.ins().fsub(x, y),
        FloatOp::Mul => b.ins().fmul(x, y),
        FloatOp::Div => b.ins().fdiv(x, y),
        FloatOp::Rem => call_ret(b, c.module, c.func.f64_rem, &[x, y]),
    }
}

fn int_cc(op: CmpOp) -> IntCC {
    match op {
        CmpOp::Lt => IntCC::SignedLessThan,
        CmpOp::Le => IntCC::SignedLessThanOrEqual,
        CmpOp::Gt => IntCC::SignedGreaterThan,
        CmpOp::Ge => IntCC::SignedGreaterThanOrEqual,
        CmpOp::Eq => IntCC::Equal,
        CmpOp::Ne => IntCC::NotEqual,
    }
}

fn float_cc(op: CmpOp) -> FloatCC {
    match op {
        CmpOp::Lt => FloatCC::LessThan,
        CmpOp::Le => FloatCC::LessThanOrEqual,
        CmpOp::Gt => FloatCC::GreaterThan,
        CmpOp::Ge => FloatCC::GreaterThanOrEqual,
        CmpOp::Eq => FloatCC::Equal,
        CmpOp::Ne => FloatCC::NotEqual,
    }
}

fn array_push(b: &mut FunctionBuilder, c: &mut Ctx, hdr: Value, v: Value, elem: ElemType) {
    let esize = elem_size(elem);
    let len = b
        .ins()
        .load(types::I64, MemFlagsData::new(), hdr, Offset32::new(HDR_LEN));
    let cap = b
        .ins()
        .load(types::I64, MemFlagsData::new(), hdr, Offset32::new(HDR_CAP));
    let full = b.ins().icmp(IntCC::SignedGreaterThanOrEqual, len, cap);
    let grow = c.block(b);
    let cont = c.block(b);
    b.ins().brif(full, grow, &[], cont, &[]);

    b.switch_to_block(grow);
    let doubled = b.ins().imul_imm_s(cap, 2);
    let four = b.ins().iconst(types::I64, 4);
    let too_small = b.ins().icmp(IntCC::SignedLessThan, doubled, four);
    let new_cap = b.ins().select(too_small, four, doubled);
    let bytes = b.ins().imul_imm_s(new_cap, esize);
    let arena = b.use_var(c.arena_var);
    let new_data = call_ret(b, c.module, c.func.arena_alloc, &[arena, bytes]);
    let old_data = b.ins().load(
        types::I64,
        MemFlagsData::new(),
        hdr,
        Offset32::new(HDR_DATA),
    );
    let copy_bytes = b.ins().imul_imm_s(len, esize);
    call_void(
        b,
        c.module,
        c.func.memcpy,
        &[new_data, old_data, copy_bytes],
    );
    b.ins()
        .store(MemFlagsData::new(), new_data, hdr, Offset32::new(HDR_DATA));
    b.ins()
        .store(MemFlagsData::new(), new_cap, hdr, Offset32::new(HDR_CAP));
    b.ins().jump(cont, &[]);

    b.switch_to_block(cont);
    let data = b.ins().load(
        types::I64,
        MemFlagsData::new(),
        hdr,
        Offset32::new(HDR_DATA),
    );
    let scaled = b.ins().imul_imm_s(len, esize);
    let addr = b.ins().iadd(data, scaled);
    b.ins()
        .store(MemFlagsData::new(), v, addr, Offset32::new(0));
    let new_len = b.ins().iadd_imm_s(len, 1);
    b.ins()
        .store(MemFlagsData::new(), new_len, hdr, Offset32::new(HDR_LEN));
}

fn array_fill(
    b: &mut FunctionBuilder,
    c: &mut Ctx,
    hdr: Value,
    count: Value,
    v: Value,
    elem: ElemType,
) {
    let bytes = b.ins().imul_imm_s(count, elem_size(elem));
    let arena = b.use_var(c.arena_var);
    let data = call_ret(b, c.module, c.func.arena_alloc, &[arena, bytes]);
    b.ins()
        .store(MemFlagsData::new(), data, hdr, Offset32::new(HDR_DATA));
    b.ins()
        .store(MemFlagsData::new(), count, hdr, Offset32::new(HDR_LEN));
    b.ins()
        .store(MemFlagsData::new(), count, hdr, Offset32::new(HDR_CAP));
    let fill = match elem {
        ElemType::Bool => c.func.fill_u8,
        ElemType::I64 => c.func.fill_i64,
        ElemType::F64 => c.func.fill_f64,
    };
    call_void(b, c.module, fill, &[data, count, v]);
}

fn array_addr(
    b: &mut FunctionBuilder,
    c: &mut Ctx,
    hdr: Value,
    idx: Value,
    elem: ElemType,
) -> Value {
    let len = b
        .ins()
        .load(types::I64, MemFlagsData::new(), hdr, Offset32::new(HDR_LEN));
    let zero = b.ins().iconst(types::I64, 0);
    let negative = b.ins().icmp(IntCC::SignedLessThan, idx, zero);
    let adjusted = b.ins().iadd(idx, len);
    let idx = b.ins().select(negative, adjusted, idx);
    let below = b.ins().icmp(IntCC::SignedLessThan, idx, zero);
    let above = b.ins().icmp(IntCC::SignedGreaterThanOrEqual, idx, len);
    let bad = b.ins().bor(below, above);
    guard(b, c, bad);
    let data = b.ins().load(
        types::I64,
        MemFlagsData::new(),
        hdr,
        Offset32::new(HDR_DATA),
    );
    let scaled = b.ins().imul_imm_s(idx, elem_size(elem));
    b.ins().iadd(data, scaled)
}
