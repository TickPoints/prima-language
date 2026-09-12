//! Numeric scalar bytecode (spec §19.2): a stack machine over `f64` produced from a numeric
//! `ExprDAG`. Each `Op` pops operands from the stack and pushes a result; the final stack top is the
//! function result. `Const`/`Param` push; binary ops pop two and push one; unary ops pop one.

/// Bytecode operation. `Param(i)` reads the i-th function argument; `Const` pushes a constant.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Op {
    Const(f64),
    Param(u8),
    /// Arithmetic (native f64 semantics).
    Neg,
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    /// Unary math functions (lowered to registered trampolines, see `engine`).
    Abs,
    Sqrt,
    Exp,
    Ln,
    Log10,
    Sin,
    Cos,
    Tan,
    /// `a.powf(b)`.
    Pow,
}

/// A bytecode program: a straight-line sequence of stack operations with no control flow.
/// The program stack height never dips below 1 after the first push; the top at the end is the result.
#[derive(Debug, Clone, PartialEq)]
pub struct Bytecode(pub Vec<Op>);

/// Maximum number of function parameters the bytecode can address: `Op::Param` carries a `u8`
/// index, so a function with more parameters cannot be represented and must be rejected up front
/// (never truncated to a wrong slot, spec §19.2).
pub const MAX_PARAMS: usize = u8::MAX as usize + 1;
