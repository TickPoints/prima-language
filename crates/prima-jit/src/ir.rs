//! Typed register IR for whole-function JIT compilation (spec §19.2).
//!
//! The interpreter lowers a numeric `fn` body into this IR (a flat register machine with explicit
//! blocks/branches) and [`crate::func::compile_ir`] lowers it to cranelift. Only the pure numeric
//! subset is representable: `I64`/`F64`/`Bool` scalars and dense local arrays of those element
//! types. Any construct outside the subset makes the lowering bail, and the caller stays on the
//! interpreter.
//!
//! Semantics are exact: integer arithmetic is checked (an overflow or an out-of-range array index
//! sets the error flag and the caller re-runs the call on the interpreter), and integer remainder
//! uses the language's modulo convention via a trampoline.

/// A scalar value type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScalarType {
    I64,
    F64,
    Bool,
}

/// A dense array's element type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ElemType {
    I64,
    F64,
    Bool,
}

/// The type of a register/slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotType {
    Scalar(ScalarType),
    Array(ElemType),
}

/// Integer arithmetic (no `Div`: exact integer division yields a Rational, outside the JIT).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntOp {
    Add,
    Sub,
    Mul,
    Rem,
}

/// IEEE float arithmetic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FloatOp {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
}

/// A comparison producing `Bool`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CmpOp {
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
    Ne,
}

/// One IR instruction. `dst`/`a`/`b`/`arr`/`idx` are slot indices; branch targets are instruction
/// indices (leaders).
#[derive(Debug, Clone)]
pub enum IrOp {
    ConstI64 {
        dst: u16,
        v: i64,
    },
    ConstF64 {
        dst: u16,
        v: f64,
    },
    ConstBool {
        dst: u16,
        v: bool,
    },
    Copy {
        dst: u16,
        src: u16,
    },

    IntBin {
        dst: u16,
        a: u16,
        b: u16,
        op: IntOp,
    },
    IntBinImm {
        dst: u16,
        a: u16,
        v: i64,
        op: IntOp,
    },
    FloatBin {
        dst: u16,
        a: u16,
        b: u16,
        op: FloatOp,
    },
    FloatBinImm {
        dst: u16,
        a: u16,
        v: f64,
        op: FloatOp,
    },
    IntNeg {
        dst: u16,
        src: u16,
    },
    FloatNeg {
        dst: u16,
        src: u16,
    },
    I64ToF64 {
        dst: u16,
        src: u16,
    },

    IntCmp {
        dst: u16,
        a: u16,
        b: u16,
        op: CmpOp,
    },
    FloatCmp {
        dst: u16,
        a: u16,
        b: u16,
        op: CmpOp,
    },
    BoolNot {
        dst: u16,
        src: u16,
    },

    Jump {
        target: u32,
    },
    /// If `cond` (negated when `negate`) is true, jump to `target`; otherwise fall through.
    BranchBool {
        cond: u16,
        negate: bool,
        target: u32,
    },
    Return {
        src: u16,
    },

    NewArray {
        dst: u16,
        elem: ElemType,
    },
    ArrayLen {
        dst: u16,
        arr: u16,
    },
    ArrayPush {
        arr: u16,
        src: u16,
    },
    ArrayPushI64Imm {
        arr: u16,
        v: i64,
    },
    ArrayPushF64Imm {
        arr: u16,
        v: f64,
    },
    ArrayPushBoolImm {
        arr: u16,
        v: bool,
    },
    ArrayFillI64 {
        arr: u16,
        count: u16,
        v: i64,
    },
    ArrayFillF64 {
        arr: u16,
        count: u16,
        v: f64,
    },
    ArrayFillBool {
        arr: u16,
        count: u16,
        v: bool,
    },
    ArrayGet {
        dst: u16,
        arr: u16,
        idx: u16,
    },
    ArraySet {
        arr: u16,
        idx: u16,
        src: u16,
    },
    ArraySetI64Imm {
        arr: u16,
        idx: u16,
        v: i64,
    },
    ArraySetF64Imm {
        arr: u16,
        idx: u16,
        v: f64,
    },
    ArraySetBoolImm {
        arr: u16,
        idx: u16,
        v: bool,
    },
}

/// A lowered function ready for cranelift.
#[derive(Debug, Clone)]
pub struct IrFunction {
    pub ops: Vec<IrOp>,
    /// Type of each slot (index = slot).
    pub slots: Vec<SlotType>,
    /// Parameter slots (`0..arity`) and their types.
    pub params: Vec<ScalarType>,
    /// Return type.
    pub ret: ScalarType,
}

impl IrFunction {
    /// The number of parameters.
    pub fn arity(&self) -> usize {
        self.params.len()
    }
}
