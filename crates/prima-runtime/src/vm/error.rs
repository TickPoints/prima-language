//! Internal VM control-flow errors (spec §19.5).
//!
//! The compiled subset is deliberately partial. A construct it cannot execute on the current path
//! is a *capability limit*: the caller falls back to the AST interpreter. That is not a program
//! error, and re-running a body that already produced side effects would duplicate them. Keeping
//! the distinction in the type system replaces the former `"VM: "` message-prefix protocol.

use crate::error::RuntimeError;

/// Why the compiled subset cannot execute a construct on the current path.
///
/// Every variant names an invariant of the compiler/executor pair. The ones still reachable (for
/// example [`Self::InvalidLocalSlot`]) indicate a malformed chunk rather than a user error, so they
/// exist to name the defect precisely instead of surfacing a free-form message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VmLimit {
    /// A local slot referenced by an instruction is missing.
    InvalidLocalSlot,
    /// A call site's argument count disagrees with the compiled chunk's arity.
    ArityMismatch,
    /// A fallback statement index is not in the chunk's side table.
    InvalidFallbackStatement,
    /// A fallback statement returned from the enclosing function.
    FallbackStatementReturned,
    /// A multi-index specification index is not in the chunk's side table.
    InvalidMultiIndexShape,
    /// An upvalue op reached the executor although capturing bodies are rejected at compile time.
    UpvalueOutsideCompiledSubset,
    /// A register-form op executed outside the inline dispatch loop.
    RegisterOpOutsideDispatchLoop,
    /// A builtin that needs unevaluated arguments reached the executor; compilation rejects them.
    UnevaluatedBuiltinArgument,
    /// A function value was called although the compiler never emits that form.
    FunctionValueCallUnsupported,
    /// A name constant has the wrong constant kind.
    InvalidNameConstant,
    /// A resolved binding disappeared between the guard and the write-back.
    BindingDisappeared,
}

impl VmLimit {
    /// Human-readable label for internal debugging; never rendered as a user diagnostic.
    pub fn reason(self) -> &'static str {
        match self {
            Self::InvalidLocalSlot => "invalid local slot",
            Self::ArityMismatch => "argument count mismatch",
            Self::InvalidFallbackStatement => "invalid fallback statement",
            Self::FallbackStatementReturned => "fallback statement returned from the function",
            Self::InvalidMultiIndexShape => "invalid multi-index shape",
            Self::UpvalueOutsideCompiledSubset => {
                "upvalues are not supported in the compiled subset"
            }
            Self::RegisterOpOutsideDispatchLoop => "register op outside the dispatch loop",
            Self::UnevaluatedBuiltinArgument => "builtin receives un-evaluated arguments",
            Self::FunctionValueCallUnsupported => "function-value calls are not supported",
            Self::InvalidNameConstant => "invalid name constant",
            Self::BindingDisappeared => "binding disappeared during a write-back",
        }
    }
}

impl std::fmt::Display for VmLimit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.reason())
    }
}

/// How a bytecode-VM run ended.
#[derive(Debug, Clone, PartialEq)]
pub enum VmError {
    /// The compiled subset cannot execute this construct. The caller may fall back to the AST only
    /// if the run has not produced observable effects.
    Limit(VmLimit),
    /// A genuine program runtime error; propagate it and never re-run on the AST.
    Runtime(RuntimeError),
    /// The requested entry name is not in the VM program (a host API misuse).
    UnknownEntry(String),
}

impl From<VmLimit> for VmError {
    fn from(limit: VmLimit) -> Self {
        Self::Limit(limit)
    }
}

impl From<RuntimeError> for VmError {
    fn from(error: RuntimeError) -> Self {
        Self::Runtime(error)
    }
}

impl std::fmt::Display for VmError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Limit(limit) => write!(f, "VM capability limit: {limit}"),
            Self::Runtime(error) => error.fmt(f),
            Self::UnknownEntry(name) => write!(f, "unknown VM entry `{name}`"),
        }
    }
}

impl std::error::Error for VmError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Runtime(error) => Some(error),
            Self::Limit(_) | Self::UnknownEntry(_) => None,
        }
    }
}
