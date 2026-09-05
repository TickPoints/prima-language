//! Bytecode VM instruction set and chunk (spec §19.5, Milestone B).
//!
//! The VM is a stack machine over `Value`-typed slots. A `Chunk` is the compiled form of one
//! function/method/closure entry: a flat instruction array, a constant pool, a local-variable table
//! with slot allocation, upvalue descriptors, and a line table for diagnostics.
//!
//! The instruction set is deliberately minimal and monomorphic on purpose: operands are stack-typed
//! (`Value`), so the dispatch loop stays small and branch-predictable. Specialization opportunities
//! (numeric fast paths, string/integer interning) are added later as `F64`/tagged fast ops.

/// A constant reference into the chunk's constant pool.
pub type Reg = u16;

/// A single bytecode instruction with an inline operand where present.
///
/// Every instruction consumes its inputs from the operand stack and pushes its result back, except
/// for the control-flow and access instructions that read/write locals/upvalues/offset fields.
#[derive(Debug, Clone, PartialEq, Eq)]
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
    /// Store the stack top into upvalue `slot` and pop it.
    SetUpvalue(Reg),
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
    /// Index assign: `base[index] = value`, leaving the assigned value on the stack.
    IndexStore,
    /// `SetLocal` for an array slot write-back (receiver mutation, spec §11.3).
    // ————— calls —————
    /// Call a function value on the stack with the top `argc` arguments (args pushed in order);
    /// the callee is immediately below the args.
    Call { argc: u16 },
    /// Call a method by name constant index with `argc` arguments pushed above the receiver.
    Method { name: Reg, argc: u16 },
    /// Call a function name (const pool `Const::Name` index) with `argc` arguments pushed above it.
    CallName { name: Reg, argc: u16 },
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

/// One compiled function entry: bytecode + constants + local/upvalue metadata.
#[derive(Debug, Clone)]
pub struct Chunk {
    pub code: Vec<Op>,
    pub constants: Vec<Const>,
    pub locals: Vec<Local>,
    /// Number of stack slots reserved for locals/upvalues (max slot index + 1).
    pub slot_count: u16,
    pub upvalues: Vec<Upvalue>,
    /// Source line per instruction (offset-indexed), for diagnostics.
    pub lines: Vec<u32>,
}

impl Chunk {
    pub fn new() -> Chunk {
        Chunk {
            code: Vec::new(),
            constants: Vec::new(),
            locals: Vec::new(),
            slot_count: 0,
            upvalues: Vec::new(),
            lines: Vec::new(),
        }
    }

    /// Push an instruction, recording its source line.
    pub fn emit(&mut self, op: Op, line: u32) {
        self.code.push(op);
        self.lines.push(line);
    }

    /// Intern a string constant, returning a pool index.
    pub fn add_string(&mut self, s: String) -> Reg {
        self.constants.push(Const::Str(s));
        (self.constants.len() - 1) as Reg
    }

    /// Intern a name constant (for `Method`/`CallName`).
    pub fn add_name(&mut self, s: String) -> Reg {
        self.constants.push(Const::Name(s));
        (self.constants.len() - 1) as Reg
    }

    /// Intern a `Value` constant.
    pub fn add_value(&mut self, v: prima_core::Value) -> Reg {
        self.constants.push(Const::Value(v));
        (self.constants.len() - 1) as Reg
    }

    /// Intern a `Value` constant, returning its index (re-exposed for clarity).
    pub fn add_const(&mut self, v: prima_core::Value) -> Reg {
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
}

impl Default for Chunk {
    fn default() -> Self {
        Self::new()
    }
}

/// A compiled VM program entry: the root chunk, the set of function chunks (indexed by function id),
/// and the function-name → chunk-index table.
#[derive(Debug, Clone)]
pub struct Program {
    pub root: Chunk,
    /// All non-root function/method/closure chunks, keyed by function id (`u32`).
    pub functions: Vec<Chunk>,
    /// Name → index into `functions` for the VM's call dispatch.
    pub names: std::collections::HashMap<String, u32>,
}
