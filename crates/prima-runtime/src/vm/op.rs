//! Bytecode VM instruction set and chunk (spec §19.5, Milestone B).
//!
//! The VM is a stack machine over `Value`-typed slots. A `Chunk` is the compiled form of one
//! function/method/closure entry: a flat instruction array, a constant pool, a local-variable table
//! with slot allocation, upvalue descriptors, and a line table for diagnostics.
//!
//! The instruction set is deliberately minimal and monomorphic on purpose: operands are stack-typed
//! (`Value`), so the dispatch loop stays small and branch-predictable. Numeric fast paths live in
//! the executor (spec §19.5); further specialization opportunities are added later as `F64`/tagged
//! fast ops.

use std::cell::RefCell;
use std::rc::Rc;

/// A constant reference into the chunk's constant pool.
pub type Reg = u16;

/// Arithmetic operator for the register-form instructions (spec §6.1/§6.5). Mirrors the stack
/// arithmetic ops; `Div` keeps the `fraction` policy and the exact zero-divisor diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArithOp {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
}

/// A small inline operand for the register-form instructions. `Float` stores the `f64` bit pattern
/// so the type stays `Copy`/`Eq` (the value is recovered with `f64::from_bits`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Imm {
    Int(i64),
    Float(u64),
    Bool(bool),
}

impl Imm {
    /// A float immediate.
    pub fn float(x: f64) -> Imm {
        Imm::Float(x.to_bits())
    }

    /// The immediate as a `Value` (used by the generic fallback path).
    pub fn to_value(self) -> prima_core::Value {
        match self {
            Imm::Int(v) => prima_core::Value::Number(prima_core::Number::Small(v)),
            Imm::Float(bits) => prima_core::Value::Number(prima_core::Number::Real(
                prima_core::Real::F64(f64::from_bits(bits)),
            )),
            Imm::Bool(b) => prima_core::Value::Bool(b),
        }
    }
}

/// A single bytecode instruction with an inline operand where present.
///
/// Every instruction consumes its inputs from the operand stack and pushes its result back, except
/// for the control-flow and access instructions that read/write locals/upvalues/offset fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    // —— constants / loads ——
    /// Push the constant at pool index `k`.
    Const(Reg),
    /// Push local variable `slot`.
    LoadLocal(Reg),
    /// Push upvalue `slot`.
    LoadUpvalue(Reg),
    /// Read the `self`-receiver value slot.
    LoadSelf,
    // —— stores / pops ——
    /// Store the stack top into local `slot` and pop it.
    SetLocal(Reg),
    /// Store the stack top into local `slot` and discard it (no push-back). Emitted where the
    /// compiler previously emitted `SetLocal` + `Pop` (bindings and assignments, spec §12.2).
    SetLocalNc(Reg),
    /// `slots[slot] += imm` in place (no stack traffic): `Small` checked addition, widening
    /// through the `Number` tower on overflow (spec §6.1). Emitted for `x += <small literal>`.
    AddImmLocal { slot: Reg, imm: i64 },
    /// `slots[slot] = slots[slot] + <popped rhs>` (no push-back): the fused form of
    /// `x = x + expr` for a local target (spec §12.2).
    AddToSlot(Reg),
    /// Store the stack top into upvalue `slot` and pop it.
    SetUpvalue(Reg),
    /// Loop condition test on two local slots: if NOT `slots[a] < slots[b]`, jump by `off`
    /// (forward, out of the loop). The fused form of `LoadLocal a; LoadLocal b; Lt;
    /// JumpIfFalse` (spec §14 `while`/`for` loops); non-`Small` operands fall back to the
    /// general comparison path.
    BranchLocalLt { a: Reg, b: Reg, off: i32 },
    /// Loop condition test on two local slots: if NOT `slots[a] <= slots[b]`, jump by `off`
    /// (forward, out of the loop). The fused form of `LoadLocal a; LoadLocal b; Le;
    /// JumpIfFalse` (spec §14); non-`Small` operands fall back to the general comparison path.
    BranchLocalLe { a: Reg, b: Reg, off: i32 },
    /// Bind the top `n` operand-stack values to the frame's parameter slots `0..n` (in push
    /// order: the value pushed first is slot 0) and remove them from the stack. Emitted once at
    /// the start of a function chunk; both call paths (entry args and `CallName` frames) push
    /// arguments in order, so binding is uniform (spec §11).
    BindParams(Reg),
    /// Pop one value and discard it.
    Pop,
    // —— arithmetic / logic (stack → stack) ——
    /// `+`
    Add,
    /// `-`
    Sub,
    /// `*`
    Mul,
    /// `/`
    Div,
    /// `%`
    Rem,
    /// `^` / `**`
    Pow,
    /// `==`
    EqCmp,
    /// `!=`
    NeCmp,
    /// `<` `<=` `>` `>=` (one comparison dispatch)
    Lt,
    /// `<=`
    Le,
    /// `>`
    Gt,
    /// `>=`
    Ge,
    /// `&&`
    And,
    /// `||`
    Or,
    /// unary `-`
    Neg,
    /// unary `!`
    Not,
    // —— collections ——
    /// Build an array from the top `n` stack values (in order).
    MakeArray(u16),
    /// Build a tuple from the top `n` stack values.
    MakeTuple(u16),
    /// Build a set from the top `n` stack values.
    MakeSet(u16),
    /// Build a dict from the top `n` key/value pairs (key pushed then value).
    MakeDict(u16),
    /// Index a value: `base[index]` → push result.
    Index,
    /// Index assign into a local slot's array: `slot[index] = value` (stack: `[index, value]`).
    /// The slot's array is mutated in place (copy-on-write when its handle is aliased) and the
    /// assigned value is left on the stack (spec §11.3).
    IndexStoreLocal(Reg),
    /// Index assign into an environment name's array: `name[index] = value` (stack:
    /// `[index, value]`); the binding is mutated in place along the chain and the assigned value
    /// is left on the stack (spec §11.3/§12.2).
    IndexStoreName(Reg),
    // ————— calls —————
    /// Call a function value on the stack with the top `argc` arguments (args pushed in order);
    /// the callee is immediately below the args.
    Call { argc: u16 },
    /// Call a method by name constant index with `argc` arguments pushed above the receiver.
    Method { name: Reg, argc: u16 },
    /// Call a mutating `Array` method (spec §11.3) on a local slot's array: the slot's array is
    /// mutated in place and stored back so later loads observe the mutation. `argc` arguments are
    /// on the stack; the receiver stays in its slot.
    MethodLocal { name: Reg, argc: u16, slot: Reg },
    /// Call a function name (const pool `Const::Name` index) with `argc` arguments pushed above it.
    CallName { name: Reg, argc: u16 },
    /// Call a mutating `Array` method (spec §11.3) on an environment binding (a non-local receiver
    /// name, const pool `Const::Name` index): the binding's array is mutated in place along the
    /// scope chain, mirroring the AST's `mutate_array` path. `argc` arguments are on the stack.
    MethodName { name: Reg, argc: u16 },
    /// Push a global/builtin name reference (const pool `Const::Name` index), resolved against the
    /// environment at runtime. Used for non-local symbols and builtins.
    LoadName(Reg),
    // —— control flow ——
    /// Jump by the signed offset.
    Jump(i32),
    /// Pop the top value; if it is falsey (or `false`/`Nil`/0 per spec §12.1), jump by the offset.
    JumpIfFalse(i32),
    /// Jump if the top value is truthy, leaving it on the stack.
    JumpIfTrue(i32),
    /// Pop a value; if the value is a `Flow::Return`, carry it.
    Return,
    /// Return from the current frame, popping the top value.
    ReturnValue,
    // —— pattern / match ——
    /// Duplicate the stack top.
    Dup,
    // —— register-form local ops (locals as registers; no operand-stack traffic) ——
    /// `slots[dst] = slots[a] <op> slots[b]`. Numeric operands use the inline `Small`/`F64` fast
    /// paths; anything else keeps the full `eval_binary` semantics (spec §6.1/§12.2).
    RegBin {
        op: ArithOp,
        dst: Reg,
        a: Reg,
        b: Reg,
    },
    /// `slots[dst] = slots[a] <op> imm` (inline immediate; `Div` keeps the `fraction` policy).
    RegBinImm {
        op: ArithOp,
        dst: Reg,
        a: Reg,
        imm: Imm,
    },
    /// Fused multiply-accumulate: `slots[dst] = slots[dst] + slots[a] * slots[b]` (numeric fast
    /// path; the accumulation pattern `s = s + i*i`, spec §10).
    RegMulAdd { dst: Reg, a: Reg, b: Reg },
    /// `slots[dst] = slots[arr][slots[idx]]` — array index read without materializing the receiver
    /// handle on the operand stack (spec §11.3).
    RegIndex { dst: Reg, arr: Reg, idx: Reg },
    /// Push `slots[arr][slots[idx]]` (condition reads like `if prime[i]`, spec §11.3).
    RegIndexPush { arr: Reg, idx: Reg },
    /// `slots[dst] = -slots[a]` (numeric fast path; spec §6.1).
    RegNeg { dst: Reg, a: Reg },
    /// `slots[dst] = to_f64(slots[a])` — the collapse fast path (spec §9.2).
    RegToF64 { dst: Reg, a: Reg },
    /// `slots[dst] = slots[a]` (register move).
    RegMove { dst: Reg, a: Reg },
    /// Register-form array push: `slots[arr].push(clone slots[src])` (spec §11.3).
    RegPush { arr: Reg, src: Reg },
    /// Register-form array push of an inline immediate.
    RegPushImm { arr: Reg, imm: Imm },
    /// Register-form index store: `slots[arr][slots[idx]] = clone slots[src]` (in place, CoW when
    /// the handle is aliased; spec §11.3).
    RegIndexStore { arr: Reg, idx: Reg, src: Reg },
    /// Register-form index store of an inline immediate.
    RegIndexStoreImm { arr: Reg, idx: Reg, imm: Imm },
    /// Fused loop test `slots[a] <op> slots[b] + imm` (`le` selects `<=`): jump out when false.
    /// Covers `for i in lo..(hi ± k)` without re-materializing the constant (spec §14).
    BranchLocalCmpSum {
        le: bool,
        a: Reg,
        b: Reg,
        imm: i64,
        off: i32,
    },
    /// Fill idiom (spec §14): `for _ in slots[lo]..slots[hi] { slots[arr].push(imm) }` lowered to a
    /// single bulk extend. Emitted only when the body is exactly one constant push, so the loop
    /// bound cannot change and the loop variable is unused.
    RegFill {
        arr: Reg,
        lo: Reg,
        hi: Reg,
        imm: Imm,
    },
    /// Fused `if slots[arr][slots[idx]] { … }` test (spec §11.3/§12.1): read the element and jump
    /// when it is `false`; a non-boolean element is the usual condition error.
    RegIndexBranchFalse { arr: Reg, idx: Reg, off: i32 },
    /// Slice assignment into a local slot's array (spec §11.3): stack `[lo, hi, rhs]`, where a
    /// `Nil` bound means "omitted" (defaults 0 / length). The array is spliced in place (CoW when
    /// the handle is aliased).
    SliceStoreLocal { slot: Reg },
    /// Slice assignment into an environment binding's array.
    SliceStoreName { name: Reg },
}

/// A constant pool entry: an already-`Value`-shaped literal or a symbolic reference that resolves
/// too late to constant-fold (registry ids, strings interned at compile time).
#[derive(Debug, Clone)]
pub enum Const {
    Value(prima_core::Value),
    /// A string constant (half of a `Value`), stored separately to avoid cloning the enum.
    Str(String),
    /// A name used by `Method`/`CallName`.
    Name(String),
}

/// A local-variable slot: name (for diagnostics), the slot index, and whether it is `self`.
#[derive(Debug, Clone)]
pub struct Local {
    pub name: String,
    pub slot: Reg,
    pub is_self: bool,
}

/// A captured upvalue: the parent-frame slot index, or `Some(closure_slot)` for a sibling-level
/// closure-captured upvalue chain.
#[derive(Debug, Clone)]
pub struct Upvalue {
    pub slot: Reg,
}

/// A call-site resolution cached across executions (spec §19.5): either a core builtin (dispatched
/// directly) or a function chunk of the same program (entered as a frame). Cached entries are
/// validated against the process-wide function-definition epoch (see `crate::eval::env`), so a
/// user redefinition (shadowing a builtin, rebinding a `fn`) always re-resolves.
#[derive(Debug, Clone, Copy)]
pub enum Callee {
    Builtin(crate::builtins::Builtin),
    ProgramFn(u32),
}

/// One compiled function entry: bytecode + constants + local/upvalue metadata.
#[derive(Debug, Clone)]
pub struct Chunk {
    pub code: Vec<Op>,
    pub constants: Vec<Const>,
    pub locals: Vec<Local>,
    /// Number of stack slots reserved for locals/upvalues (max slot index + 1).
    pub slot_count: u16,
    /// Expected parameter count for function chunks (spec §11); used to reject arity mismatches
    /// at the call site (the AST path reports the authoritative error). Root chunks carry `0`.
    pub arity: u16,
    pub upvalues: Vec<Upvalue>,
    /// Source line per instruction (offset-indexed), for diagnostics.
    pub lines: Vec<u32>,
    /// Per-`CallName`-site resolved-callee cache (spec §19.5), indexed by the instruction's name
    /// constant index, each entry `(epoch, callee)`. Interior-mutable: resolved lazily on first
    /// execution and invalidated by epoch changes. Only builtin/program-function callees are
    /// cached; environment-dependent callees (host `fn`, MFn, natives) always re-resolve.
    pub callee_cache: RefCell<Vec<Option<(u64, Callee)>>>,
}

impl Chunk {
    pub fn new() -> Chunk {
        Chunk {
            code: Vec::new(),
            constants: Vec::new(),
            locals: Vec::new(),
            slot_count: 0,
            arity: 0,
            upvalues: Vec::new(),
            lines: Vec::new(),
            callee_cache: RefCell::new(Vec::new()),
        }
    }

    /// Push an instruction, recording its source line.
    pub fn emit(&mut self, op: Op, line: u32) {
        self.code.push(op);
        self.lines.push(line);
    }

    /// Reserve the next constant-pool index, or fail when the pool would exceed the `Reg` (`u16`)
    /// index range. The compiler turns this into an AST fallback rather than a silently wrapped
    /// index (spec §19.5).
    fn next_const_index(&self) -> Result<Reg, String> {
        u16::try_from(self.constants.len())
            .map_err(|_| "VM compiler: constant pool exceeds the u16 index range".to_string())
    }

    /// Intern a string constant, returning a pool index.
    pub fn add_string(&mut self, s: String) -> Result<Reg, String> {
        let idx = self.next_const_index()?;
        self.constants.push(Const::Str(s));
        Ok(idx)
    }

    /// Intern a name constant (for `Method`/`CallName`).
    pub fn add_name(&mut self, s: String) -> Result<Reg, String> {
        let idx = self.next_const_index()?;
        self.constants.push(Const::Name(s));
        Ok(idx)
    }

    /// Intern a `Value` constant.
    pub fn add_value(&mut self, v: prima_core::Value) -> Result<Reg, String> {
        let idx = self.next_const_index()?;
        self.constants.push(Const::Value(v));
        Ok(idx)
    }

    /// Intern a `Value` constant, returning its index (re-exposed for clarity).
    pub fn add_const(&mut self, v: prima_core::Value) -> Result<Reg, String> {
        self.add_value(v)
    }

    /// Emit an unconditional `Jump` with a placeholder offset, returning the instruction index for
    /// later `patch_jump`.
    pub fn emit_jump(&mut self, line: u32) -> usize {
        let i = self.code.len();
        self.emit(Op::Jump(0), line);
        i
    }

    /// Patch a prior `Jump` placeholder to the given absolute code offset (as a relative offset).
    pub fn patch_jump(&mut self, at: usize, target: usize) {
        if let Some(Op::Jump(off)) = self.code.get_mut(at) {
            *off = target as i32 - at as i32;
        }
    }

    /// Emit a `JumpIfFalse` with a placeholder offset, returning the instruction index.
    pub fn emit_jump_if_false(&mut self, line: u32) -> usize {
        let i = self.code.len();
        self.emit(Op::JumpIfFalse(0), line);
        i
    }

    /// Patch a prior `JumpIfFalse` placeholder to the given absolute code offset.
    pub fn patch_jump_if_false(&mut self, at: usize, target: usize) {
        if let Some(Op::JumpIfFalse(off)) = self.code.get_mut(at) {
            *off = target as i32 - at as i32;
        }
    }

    /// Emit a fused `BranchLocalLt` with a placeholder offset, returning the instruction index.
    pub fn emit_branch_local_lt(&mut self, a: Reg, b: Reg, line: u32) -> usize {
        let i = self.code.len();
        self.emit(Op::BranchLocalLt { a, b, off: 0 }, line);
        i
    }

    /// Patch a prior `BranchLocalLt` placeholder to the given absolute code offset.
    pub fn patch_branch_local_lt(&mut self, at: usize, target: usize) {
        if let Some(Op::BranchLocalLt { off, .. }) = self.code.get_mut(at) {
            *off = target as i32 - at as i32;
        }
    }

    /// Emit a fused `BranchLocalLe` with a placeholder offset, returning the instruction index.
    pub fn emit_branch_local_le(&mut self, a: Reg, b: Reg, line: u32) -> usize {
        let i = self.code.len();
        self.emit(Op::BranchLocalLe { a, b, off: 0 }, line);
        i
    }

    /// Patch a prior `BranchLocalLe` placeholder to the given absolute code offset.
    pub fn patch_branch_local_le(&mut self, at: usize, target: usize) {
        if let Some(Op::BranchLocalLe { off, .. }) = self.code.get_mut(at) {
            *off = target as i32 - at as i32;
        }
    }

    /// Emit a fused `BranchLocalCmpSum` (`slots[a] <[=] slots[b] + imm`) with a placeholder offset.
    pub fn emit_branch_local_cmp_sum(
        &mut self,
        le: bool,
        a: Reg,
        b: Reg,
        imm: i64,
        line: u32,
    ) -> usize {
        let i = self.code.len();
        self.emit(
            Op::BranchLocalCmpSum {
                le,
                a,
                b,
                imm,
                off: 0,
            },
            line,
        );
        i
    }

    /// Patch a prior `BranchLocalCmpSum` placeholder to the given absolute code offset.
    pub fn patch_branch_local_cmp_sum(&mut self, at: usize, target: usize) {
        if let Some(Op::BranchLocalCmpSum { off, .. }) = self.code.get_mut(at) {
            *off = target as i32 - at as i32;
        }
    }

    /// Emit a fused `RegIndexBranchFalse` with a placeholder offset, returning the instruction index.
    pub fn emit_branch_index_false(&mut self, arr: Reg, idx: Reg, line: u32) -> usize {
        let i = self.code.len();
        self.emit(Op::RegIndexBranchFalse { arr, idx, off: 0 }, line);
        i
    }

    /// Patch a prior `RegIndexBranchFalse` placeholder to the given absolute code offset.
    pub fn patch_branch_index_false(&mut self, at: usize, target: usize) {
        if let Some(Op::RegIndexBranchFalse { off, .. }) = self.code.get_mut(at) {
            *off = target as i32 - at as i32;
        }
    }
}

impl Default for Chunk {
    fn default() -> Self {
        Self::new()
    }
}

/// A compiled VM program entry: the root chunk, the set of function chunks (indexed by function id),
/// and the function-name → chunk-index table. Chunks are shared through `Rc` so entering the VM (or
/// calling a compiled function) never deep-copies bytecode (spec §19.5).
#[derive(Debug, Clone)]
pub struct Program {
    pub root: Rc<Chunk>,
    /// All non-root function/method/closure chunks, keyed by function id (`u32`).
    pub functions: Vec<Rc<Chunk>>,
    /// Name → index into `functions` for the VM's call dispatch (`FxHashMap`: short-string keys
    /// are looked up on every `CallName`/entry resolution).
    pub names: rustc_hash::FxHashMap<String, u32>,
}
