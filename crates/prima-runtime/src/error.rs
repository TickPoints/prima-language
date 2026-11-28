use prima_core::Value;

mod io_failure;
pub mod module_load;

pub use io_failure::IoFailure;
pub use module_load::{ModuleImportError, ModuleIoError, ModuleIoTarget};

/// How many positional arguments a callable accepts, used by [`RuntimeError::Arity`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Arity {
    Exact(usize),
    AtLeast(usize),
    /// A small closed set of accepted counts, e.g. `1` or `2`.
    OneOf(&'static [usize]),
}

impl std::fmt::Display for Arity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Exact(n) => write!(f, "{n} argument(s)"),
            Self::AtLeast(n) => write!(f, "at least {n} argument(s)"),
            Self::OneOf(options) => {
                let parts: Vec<String> = options.iter().map(usize::to_string).collect();
                write!(f, "{} argument(s)", parts.join(" or "))
            }
        }
    }
}

/// A concrete reason a value is not acceptable to a library function (spec §18).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValueProblem {
    MustBePositive,
    MustBeNonNegative,
    MustBeFinite,
    MustBeNonZero,
    /// A numeric value below an inclusive minimum.
    BelowMinimum {
        what: &'static str,
        minimum: i64,
        got: i64,
    },
    /// A numeric value above an inclusive maximum.
    AboveMaximum {
        what: &'static str,
        maximum: i64,
        got: i64,
    },
    /// An option or format outside the supported set.
    Unsupported {
        got: String,
        supported: &'static str,
    },
    /// A computed size above a technical limit.
    ExceedsLimit {
        value: String,
        limit: String,
    },
    /// An empty collection where at least one element is required.
    Empty {
        what: &'static str,
    },
    /// A numeric series without any finite element.
    NoFinite {
        what: &'static str,
    },
    /// Paired arrays that must have the same length.
    UnequalLengths {
        left: &'static str,
        right: &'static str,
        first: usize,
        second: usize,
    },
    /// A nested grid with rows of differing widths.
    NotRectangular {
        row: usize,
        got: usize,
        expected: usize,
    },
    /// A half-open range whose bounds are inconsistent.
    InvalidRange {
        what: &'static str,
        start: usize,
        end: usize,
        length: usize,
    },
}

impl std::fmt::Display for ValueProblem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MustBePositive => f.write_str("must be positive"),
            Self::MustBeNonNegative => f.write_str("must be non-negative"),
            Self::MustBeFinite => f.write_str("must be finite"),
            Self::MustBeNonZero => f.write_str("must be non-zero"),
            Self::BelowMinimum { what, minimum, got } => {
                write!(f, "{what} must be at least {minimum} (got {got})")
            }
            Self::AboveMaximum { what, maximum, got } => {
                write!(f, "{what} must be at most {maximum} (got {got})")
            }
            Self::Unsupported { got, supported } => {
                write!(f, "unsupported `{got}`; supported: {supported}")
            }
            Self::ExceedsLimit { value, limit } => {
                write!(f, "{value} exceeds the {limit} limit")
            }
            Self::Empty { what } => write!(f, "{what} must not be empty"),
            Self::NoFinite { what } => {
                write!(f, "{what} must contain at least one finite number")
            }
            Self::UnequalLengths {
                left,
                right,
                first,
                second,
            } => write!(
                f,
                "{left} and {right} must have equal length ({first} vs {second})"
            ),
            Self::NotRectangular { row, got, expected } => write!(
                f,
                "grid must be rectangular: row {row} has {got} columns, expected {expected}"
            ),
            Self::InvalidRange {
                what,
                start,
                end,
                length,
            } => write!(f, "invalid {what} range {start}..{end} (length {length})"),
        }
    }
}

/// The role a value plays when it must be hashable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HashRole {
    DictKey,
    SetElement,
    MembershipKey,
    MembershipElement,
}

impl std::fmt::Display for HashRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::DictKey => "dict key",
            Self::SetElement => "set element",
            Self::MembershipKey => "membership key",
            Self::MembershipElement => "membership element",
        })
    }
}

/// Which boolean-valued position received a non-boolean value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BooleanContext {
    Condition,
    Not,
    LogicalOperands,
}

impl std::fmt::Display for BooleanContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Condition => "condition must be a boolean",
            Self::Not => "`!` requires a boolean",
            Self::LogicalOperands => "`&&`/`||` require boolean operands",
        })
    }
}

/// An index/slice operand or assignment target that cannot be used (spec §11.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IndexProblem {
    NotInteger { what: &'static str },
    Negative { what: &'static str },
    OutOfRange { start: i64, end: i64, length: usize },
    NotAnArray { got: String },
    SliceRhsNotArray,
    NotIndexable { got: String },
    NotAssignable { got: String },
}

impl std::fmt::Display for IndexProblem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotInteger { what } => write!(f, "{what} must be an integer"),
            Self::Negative { what } => write!(f, "{what} must be non-negative"),
            Self::OutOfRange { start, end, length } => {
                write!(f, "invalid slice range {start}..{end} (length {length})")
            }
            Self::NotAnArray { got } => {
                write!(f, "assignment target must be an array or dict, got {got}")
            }
            Self::SliceRhsNotArray => {
                f.write_str("slice assignment right-hand side must be an array")
            }
            Self::NotIndexable { got } => write!(f, "cannot index {got}"),
            Self::NotAssignable { got } => write!(f, "cannot index-assign {got}"),
        }
    }
}

/// Which literal spelling failed to parse.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LiteralKind {
    Integer,
    Hex,
    Binary,
    Float,
}

impl std::fmt::Display for LiteralKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Integer => "integer",
            Self::Hex => "hex",
            Self::Binary => "binary",
            Self::Float => "float",
        })
    }
}

/// An internal invariant violation (never a user mistake).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InternalProblem {
    EmbeddedModuleParse {
        module: String,
    },
    ArcCountNotInteger,
    ArcMissingField {
        field: String,
    },
    BindingChanged,
    /// A method declaration has no body and no registered implementation.
    UnregisteredBuiltin,
    Panic {
        export: String,
        message: String,
    },
}

impl std::fmt::Display for InternalProblem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmbeddedModuleParse { module } => {
                write!(
                    f,
                    "internal error: embedded `{module}` module does not parse"
                )
            }
            Self::ArcCountNotInteger => f.write_str("`Arc` count is not an integer"),
            Self::ArcMissingField { field } => {
                write!(f, "`Arc` instance has no `{field}` field")
            }
            Self::BindingChanged => {
                f.write_str("binding changed kind during a mutating method call")
            }
            Self::UnregisteredBuiltin => f.write_str("unregistered `@builtin` method"),
            Self::Panic { export, message } => {
                write!(f, "internal panic in export `{export}`: {message}")
            }
        }
    }
}

/// Runtime error (spec §16): structured categories so diagnostics can be classified by kind
/// (spec §16.3), carrying a human-readable message. The complete fields of the structured `Error`
/// enum (§16.1) are deferred to a later release.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum RuntimeError {
    #[error("overflow: {0}")]
    Overflow(String),
    #[error("index out of bounds: {0}")]
    IndexOutOfBounds(String),
    #[error("undefined: {0}")]
    Undefined(String),
    #[error("domain error: {0}")]
    Domain(String),
    #[error("type error: {0}")]
    Type(String),
    #[error("collapse error: {0}")]
    Collapse(String),
    /// A call passed the wrong number of positional arguments.
    #[error("`{callable}` expects {expected}, got {got}")]
    Arity {
        callable: String,
        expected: Arity,
        got: usize,
    },
    /// A call omitted a positional argument.
    #[error("`{callable}` missing argument {position}")]
    MissingArgument { callable: String, position: usize },
    /// A method call omitted its receiver.
    #[error("`{callable}` missing receiver")]
    MissingReceiver { callable: String },
    /// An argument has the wrong type or shape.
    #[error("`{callable}` argument {position} must be {expected}, got {got}")]
    ArgumentType {
        callable: String,
        position: usize,
        expected: &'static str,
        got: Box<String>,
    },
    /// A method receiver has the wrong type.
    #[error("`{callable}` expects {expected} receiver, got {got}")]
    ReceiverType {
        callable: String,
        expected: &'static str,
        got: Box<String>,
    },
    /// A library function rejected a concrete value (spec §18).
    #[error("`{callable}` {problem}")]
    InvalidValue {
        callable: String,
        problem: Box<ValueProblem>,
    },
    /// A builtin must be invoked through its dedicated call form.
    #[error("`{callable}` must be called directly")]
    NotDirectlyCallable { callable: String },
    /// A backend computation failed.
    #[error("`{operation}` failed: {detail}")]
    ComputationFailed { operation: String, detail: String },
    /// An I/O operation outside module loading failed (`R0007`).
    #[error(transparent)]
    Io(#[from] IoFailure),
    #[error("underflow: {0}")]
    Underflow(String),
    #[error("dimension mismatch: {0}")]
    DimensionMismatch(String),
    #[error("import error: {0}")]
    RuntimeImport(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("empty collection: {0}")]
    EmptyCollection(String),
    /// A compile-time diagnostic surfaced through evaluation (spec §16.4).
    #[error("{message}")]
    CompileDiagnostic { code: &'static str, message: String },
    #[error("unknown variable `{name}`")]
    UnknownVariable { name: String },
    #[error("unknown function `{name}`")]
    UnknownFunction { name: String },
    #[error("unknown class `{name}`")]
    UnknownClass { name: String },
    #[error("unknown class instance")]
    UnknownClassInstance,
    #[error("unknown method `{name}` on `{class}`")]
    UnknownMethod { name: String, class: String },
    #[error("unknown `{class}` method `{name}`")]
    UnknownNativeMethod { class: String, name: String },
    #[error("field `{field}` is uninitialized")]
    UninitializedField { field: String },
    #[error("`{subject}` methods are unavailable: the standard library is not initialized")]
    NotInitialized { subject: String },
    #[error("unknown associated function `{class}::{name}`")]
    UnknownAssociatedFunction { class: String, name: String },
    #[error("`{class}` has no field `{name}`")]
    UnknownField { class: String, name: String },
    #[error("unknown JIT function handle `{id}`")]
    UnknownJitFunction { id: u32 },
    #[error("unknown VM entry `{name}`")]
    UnknownVmEntry { name: String },
    #[error("unknown collapse function `{name}`")]
    UnknownCollapseFunction { name: String },
    #[error("unknown numeric method `{name}`")]
    UnknownNumericMethod { name: String },
    #[error("unregistered `@builtin` method `{target}`")]
    UnregisteredBuiltin { target: String },
    #[error("{role} must be a hashable value")]
    Unhashable { role: HashRole },
    #[error("{context}")]
    ExpectedBoolean { context: BooleanContext },
    #[error("unknown config key `{key}`")]
    UnknownConfigKey { key: String },
    #[error("unknown `{key}` value `{value}`")]
    UnknownConfigValue { key: String, value: String },
    #[error("config `{key}` expects a bool")]
    ConfigExpectedBool { key: String },
    #[error("config `{key}` expects an integer 0..={max}")]
    ConfigExpectedInteger { key: String, max: u8 },
    #[error("config `{key}` expects an integer")]
    ConfigExpectedIntegerUnbounded { key: String },
    #[error("config `{key}` expects an enum value")]
    ConfigExpectedEnum { key: String },
    #[error("invalid value for `undefined_handling`")]
    InvalidUndefinedHandling,
    #[error("unknown `opt_level` value `{value}` (expected O0..=O3)")]
    UnknownOptLevel { value: String },
    #[error("interrupted")]
    Interrupted,
    #[error("division by zero")]
    DivisionByZero,
    #[error("modulo by zero")]
    ModuloByZero,
    #[error("cannot compare these numbers")]
    UncomparableNumbers,
    #[error("{problem}")]
    InvalidIndex { problem: IndexProblem },
    #[error("{feature} is not supported yet")]
    UnsupportedFeature { feature: &'static str },
    #[error("implicit broadcast is disabled (`broadcast := false`); use `@.`")]
    BroadcastDisabled,
    #[error("`self` outside of a method")]
    SelfOutsideMethod,
    #[error("private method `{name}` cannot be called")]
    PrivateMethod { name: String },
    #[error("method `{name}` of `{class}` is `pub(mod)` and not accessible from this module")]
    ModuleOnlyMethod { name: String, class: String },
    #[error("`{class}::{member}` is a method; call it on an instance")]
    MethodNeedsInstance { class: String, member: String },
    #[error(
        "`{member}` on `{class}` is an associated function; call it as `{class}::{member}(...)`"
    )]
    AssociatedFunctionCalled { class: String, member: String },
    #[error("method requires a `self` receiver")]
    MissingSelfReceiver,
    #[error("{what} {detail}")]
    LimitExceeded { what: &'static str, detail: String },
    #[error("invalid {kind} literal")]
    InvalidLiteral { kind: LiteralKind },
    #[error("no `{operation}` overload registered for `{operand}`")]
    NoOverload { operation: String, operand: String },
    #[error("{problem}")]
    Internal { problem: InternalProblem },
    #[error("missing dict key")]
    MissingKey,
    #[error("cannot call method `{name}` on {got}")]
    NotMethodReceiver { name: String, got: String },
    /// A user-defined error surfaced through `unwrap`/`expect` (spec §16.1 `Custom`).
    #[error("{message}")]
    CustomError { message: String },
    /// A module-load I/O failure (`R0007`) that retains its cause (spec appendix C.2).
    #[error(transparent)]
    ModuleIo(Box<ModuleIoError>),
    /// A module-resolution or parse failure (`R0008`, spec appendix C.2).
    #[error(transparent)]
    ModuleImport(Box<ModuleImportError>),
    /// Internal `?` propagation (spec §16.3): carries the value the enclosing function must return
    /// (`Err(..)`/`None`). Caught at function-call boundaries and never rendered as a diagnostic.
    #[error("propagated `?` value")]
    Propagate(Box<Value>),
    /// Wraps an error with the source span of the statement/expression being evaluated,
    /// so diagnostics can point at the offending location (spec §16.4).
    #[error("{error}")]
    Located {
        span: prima_syntax::Span,
        error: Box<RuntimeError>,
    },
    /// Wraps an error with diagnostic notes (spec §16.4): failed method calls attach the method
    /// signature/definition/`///` doc as a note plus an optional `did you mean` help. The notes are
    /// collected by `notes()`/`help()`; the CLI renders them under the primary message.
    #[error("{error}")]
    WithNotes {
        notes: Vec<String>,
        help: Option<String>,
        error: Box<RuntimeError>,
    },
}

impl From<ModuleIoError> for RuntimeError {
    fn from(error: ModuleIoError) -> Self {
        Self::ModuleIo(Box::new(error))
    }
}

impl From<ModuleImportError> for RuntimeError {
    fn from(error: ModuleImportError) -> Self {
        Self::ModuleImport(Box::new(error))
    }
}

impl RuntimeError {
    /// Build an [`RuntimeError::ArgumentType`] with a boxed type name (keeps the error compact).
    pub fn argument_type(
        callable: String,
        position: usize,
        expected: &'static str,
        got: String,
    ) -> RuntimeError {
        RuntimeError::ArgumentType {
            callable,
            position,
            expected,
            got: Box::new(got),
        }
    }

    /// Build a [`RuntimeError::ReceiverType`] with a boxed type name (keeps the error compact).
    pub fn receiver_type(callable: String, expected: &'static str, got: String) -> RuntimeError {
        RuntimeError::ReceiverType {
            callable,
            expected,
            got: Box::new(got),
        }
    }

    /// Build an [`RuntimeError::InvalidValue`] with a boxed problem (keeps the error compact).
    pub fn invalid(callable: String, problem: ValueProblem) -> RuntimeError {
        RuntimeError::InvalidValue {
            callable,
            problem: Box::new(problem),
        }
    }

    /// Error category name, used to classify diagnostics (spec §16.3).
    pub fn kind(&self) -> &'static str {
        match self {
            RuntimeError::Overflow(_) => "Overflow",
            RuntimeError::IndexOutOfBounds(_) => "IndexOutOfBounds",
            RuntimeError::Undefined(_) => "Undefined",
            RuntimeError::Domain(_) => "Domain",
            RuntimeError::Type(_) => "Type",
            RuntimeError::Collapse(_) => "Collapse",
            RuntimeError::Arity { .. } => "Arity",
            RuntimeError::MissingArgument { .. } => "MissingArgument",
            RuntimeError::MissingReceiver { .. } => "MissingReceiver",
            RuntimeError::ArgumentType { .. } => "ArgumentType",
            RuntimeError::ReceiverType { .. } => "ReceiverType",
            RuntimeError::InvalidValue { .. } => "InvalidValue",
            RuntimeError::NotDirectlyCallable { .. } => "NotDirectlyCallable",
            RuntimeError::ComputationFailed { .. } => "ComputationFailed",
            RuntimeError::Io(_) => "Io",
            RuntimeError::Underflow(_) => "Underflow",
            RuntimeError::DimensionMismatch(_) => "DimensionMismatch",
            RuntimeError::RuntimeImport(_) => "RuntimeImport",
            RuntimeError::NotFound(_) => "NotFound",
            RuntimeError::EmptyCollection(_) => "EmptyCollection",
            RuntimeError::CompileDiagnostic { .. } => "CompileDiagnostic",
            RuntimeError::UnknownVariable { .. } => "UnknownVariable",
            RuntimeError::UnknownFunction { .. } => "UnknownFunction",
            RuntimeError::UnknownClass { .. } => "UnknownClass",
            RuntimeError::UnknownClassInstance => "UnknownClassInstance",
            RuntimeError::UnknownMethod { .. } => "UnknownMethod",
            RuntimeError::UnknownNativeMethod { .. } => "UnknownNativeMethod",
            RuntimeError::UninitializedField { .. } => "UninitializedField",
            RuntimeError::NotInitialized { .. } => "NotInitialized",
            RuntimeError::UnknownAssociatedFunction { .. } => "UnknownAssociatedFunction",
            RuntimeError::UnknownField { .. } => "UnknownField",
            RuntimeError::UnknownJitFunction { .. } => "UnknownJitFunction",
            RuntimeError::UnknownVmEntry { .. } => "UnknownVmEntry",
            RuntimeError::UnknownCollapseFunction { .. } => "UnknownCollapseFunction",
            RuntimeError::UnknownNumericMethod { .. } => "UnknownNumericMethod",
            RuntimeError::UnregisteredBuiltin { .. } => "UnregisteredBuiltin",
            RuntimeError::Unhashable { .. } => "Unhashable",
            RuntimeError::ExpectedBoolean { .. } => "ExpectedBoolean",
            RuntimeError::UnknownConfigKey { .. } => "UnknownConfigKey",
            RuntimeError::UnknownConfigValue { .. } => "UnknownConfigValue",
            RuntimeError::UnknownOptLevel { .. } => "UnknownOptLevel",
            RuntimeError::ConfigExpectedBool { .. } => "ConfigExpectedBool",
            RuntimeError::ConfigExpectedInteger { .. } => "ConfigExpectedInteger",
            RuntimeError::ConfigExpectedIntegerUnbounded { .. } => "ConfigExpectedIntegerUnbounded",
            RuntimeError::ConfigExpectedEnum { .. } => "ConfigExpectedEnum",
            RuntimeError::InvalidUndefinedHandling => "InvalidUndefinedHandling",
            RuntimeError::Interrupted => "Interrupted",
            RuntimeError::DivisionByZero => "DivisionByZero",
            RuntimeError::ModuloByZero => "ModuloByZero",
            RuntimeError::UncomparableNumbers => "UncomparableNumbers",
            RuntimeError::InvalidIndex { .. } => "InvalidIndex",
            RuntimeError::UnsupportedFeature { .. } => "UnsupportedFeature",
            RuntimeError::BroadcastDisabled => "BroadcastDisabled",
            RuntimeError::SelfOutsideMethod => "SelfOutsideMethod",
            RuntimeError::PrivateMethod { .. } => "PrivateMethod",
            RuntimeError::ModuleOnlyMethod { .. } => "ModuleOnlyMethod",
            RuntimeError::MethodNeedsInstance { .. } => "MethodNeedsInstance",
            RuntimeError::AssociatedFunctionCalled { .. } => "AssociatedFunctionCalled",
            RuntimeError::MissingSelfReceiver => "MissingSelfReceiver",
            RuntimeError::LimitExceeded { .. } => "LimitExceeded",
            RuntimeError::InvalidLiteral { .. } => "InvalidLiteral",
            RuntimeError::NoOverload { .. } => "NoOverload",
            RuntimeError::Internal { .. } => "Internal",
            RuntimeError::MissingKey => "MissingKey",
            RuntimeError::NotMethodReceiver { .. } => "NotMethodReceiver",
            RuntimeError::CustomError { .. } => "CustomError",
            RuntimeError::ModuleIo(_) => "ModuleIo",
            RuntimeError::ModuleImport(_) => "ModuleImport",
            RuntimeError::Propagate(_) => "Propagate",
            RuntimeError::Located { error, .. } => error.kind(),
            RuntimeError::WithNotes { error, .. } => error.kind(),
        }
    }

    /// The spec appendix C.2 runtime error code (spec §16.4). Concrete categories map to their
    /// `R####` code; compile diagnostics carry their `E####` code; wrappers delegate to the error
    /// they enclose.
    pub fn code(&self) -> &'static str {
        match self {
            RuntimeError::Overflow(_) => "R0001",
            RuntimeError::IndexOutOfBounds(_) => "R0003",
            RuntimeError::Undefined(_) => "R0006",
            RuntimeError::Domain(_) => "R0005",
            RuntimeError::Type(_) => "R0009",
            RuntimeError::Collapse(_) => "R0010",
            RuntimeError::Arity { .. }
            | RuntimeError::MissingArgument { .. }
            | RuntimeError::MissingReceiver { .. } => "R0011",
            RuntimeError::ArgumentType { .. } | RuntimeError::ReceiverType { .. } => "R0009",
            RuntimeError::InvalidValue { .. }
            | RuntimeError::NotDirectlyCallable { .. }
            | RuntimeError::ComputationFailed { .. } => "R0011",
            RuntimeError::Io(_) => "R0007",
            RuntimeError::Underflow(_) => "R0002",
            RuntimeError::DimensionMismatch(_) => "R0004",
            RuntimeError::RuntimeImport(_) => "R0008",
            RuntimeError::NotFound(_) => "R0013",
            RuntimeError::EmptyCollection(_) => "R0014",
            RuntimeError::CompileDiagnostic { code, .. } => code,
            RuntimeError::UnknownVariable { .. }
            | RuntimeError::UnknownFunction { .. }
            | RuntimeError::UnknownClass { .. }
            | RuntimeError::UnknownClassInstance
            | RuntimeError::UnknownMethod { .. }
            | RuntimeError::UnknownNativeMethod { .. }
            | RuntimeError::UnknownAssociatedFunction { .. }
            | RuntimeError::UnknownField { .. }
            | RuntimeError::UnknownJitFunction { .. }
            | RuntimeError::UnknownVmEntry { .. }
            | RuntimeError::UnknownCollapseFunction { .. }
            | RuntimeError::UnknownNumericMethod { .. }
            | RuntimeError::UnregisteredBuiltin { .. }
            | RuntimeError::UnknownConfigKey { .. }
            | RuntimeError::UnknownConfigValue { .. }
            | RuntimeError::ConfigExpectedBool { .. }
            | RuntimeError::ConfigExpectedInteger { .. }
            | RuntimeError::ConfigExpectedIntegerUnbounded { .. }
            | RuntimeError::ConfigExpectedEnum { .. }
            | RuntimeError::InvalidUndefinedHandling
            | RuntimeError::Interrupted
            | RuntimeError::UnsupportedFeature { .. }
            | RuntimeError::BroadcastDisabled
            | RuntimeError::SelfOutsideMethod
            | RuntimeError::PrivateMethod { .. }
            | RuntimeError::ModuleOnlyMethod { .. }
            | RuntimeError::MethodNeedsInstance { .. }
            | RuntimeError::AssociatedFunctionCalled { .. }
            | RuntimeError::MissingSelfReceiver
            | RuntimeError::LimitExceeded { .. }
            | RuntimeError::InvalidLiteral { .. }
            | RuntimeError::UninitializedField { .. }
            | RuntimeError::NotInitialized { .. }
            | RuntimeError::UnknownOptLevel { .. }
            | RuntimeError::Internal { .. } => "R0011",
            RuntimeError::MissingKey => "R0012",
            RuntimeError::CustomError { .. } => "R0011",
            RuntimeError::NotMethodReceiver { .. } => "R0009",
            RuntimeError::Unhashable { .. }
            | RuntimeError::ExpectedBoolean { .. }
            | RuntimeError::UncomparableNumbers
            | RuntimeError::NoOverload { .. } => "R0009",
            RuntimeError::DivisionByZero | RuntimeError::ModuloByZero => "R0005",
            RuntimeError::InvalidIndex { problem } => match problem {
                IndexProblem::OutOfRange { .. } => "R0003",
                _ => "R0009",
            },
            RuntimeError::ModuleIo(_) => "R0007",
            RuntimeError::ModuleImport(_) => "R0008",
            RuntimeError::Propagate(_) => "R0011",
            RuntimeError::Located { error, .. } => error.code(),
            RuntimeError::WithNotes { error, .. } => error.code(),
        }
    }

    /// Wrap this error with a `= help:` suggestion (spec §16.4), keeping any notes already attached.
    pub fn with_help(self, help: impl Into<String>) -> RuntimeError {
        RuntimeError::WithNotes {
            notes: Vec::new(),
            help: Some(help.into()),
            error: Box::new(self),
        }
    }

    /// Wrap this error with a `= note:` line (spec §16.4).
    pub fn with_note(self, note: impl Into<String>) -> RuntimeError {
        RuntimeError::WithNotes {
            notes: vec![note.into()],
            help: None,
            error: Box::new(self),
        }
    }

    /// The source span attached to this error, if any (spec §16.4). Delegates through wrapper
    /// variants so the deepest/most precise span wins.
    pub fn location(&self) -> Option<prima_syntax::Span> {
        match self {
            RuntimeError::Located { span, .. } => Some(*span),
            RuntimeError::WithNotes { error, .. } => error.location(),
            _ => None,
        }
    }

    /// All diagnostic notes attached along the wrapper chain (spec §16.4), outermost first.
    /// The primary error message is *not* part of the notes; the CLI renders it separately.
    pub fn notes(&self) -> Vec<String> {
        let mut out = Vec::new();
        self.collect_notes(&mut out);
        out
    }

    /// The `did you mean` help attached along the wrapper chain (spec §16.4); the outermost
    /// suggestion wins when several errors are nested.
    pub fn help(&self) -> Option<String> {
        let mut out = None;
        self.collect_help(&mut out);
        out
    }

    fn collect_notes(&self, out: &mut Vec<String>) {
        match self {
            RuntimeError::Located { error, .. } => error.collect_notes(out),
            RuntimeError::WithNotes { notes, error, .. } => {
                out.extend(notes.iter().cloned());
                error.collect_notes(out);
            }
            _ => {}
        }
    }

    fn collect_help(&self, out: &mut Option<String>) {
        match self {
            RuntimeError::Located { error, .. } => error.collect_help(out),
            RuntimeError::WithNotes { help, error, .. } => {
                if out.is_none() {
                    *out = help.clone();
                }
                error.collect_help(out);
            }
            _ => {}
        }
    }
}

/// Attach a source span to an error unless it already carries one (keep the deepest/most precise).
pub(crate) fn attach_span(e: RuntimeError, span: prima_syntax::Span) -> RuntimeError {
    match e {
        // `?` propagation is non-local control flow, not a diagnostic: keep it unwrapped so the
        // call boundary can recognise it after passing through nested expressions.
        RuntimeError::Located { .. } | RuntimeError::Propagate(_) => e,
        other => RuntimeError::Located {
            span,
            error: Box::new(other),
        },
    }
}

/// Catch an internal `?` propagation at a function boundary (spec §16.3): the carried value is the
/// result of the enclosing `Result`/`Option`-returning function.
pub(crate) fn catch_propagate(result: Result<Value, RuntimeError>) -> Result<Value, RuntimeError> {
    match result {
        Err(RuntimeError::Propagate(value)) => Ok(*value),
        other => other,
    }
}

/// Build a typed runtime type error (`R0009`) for shape/type checks without a more specific
/// structured variant.
pub(crate) fn type_err<T>(message: impl Into<String>) -> Result<T, RuntimeError> {
    Err(RuntimeError::Type(message.into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn located(e: RuntimeError) -> RuntimeError {
        attach_span(e, prima_syntax::Span::new(4, 9))
    }

    #[test]
    fn with_notes_delegates_kind_and_location() {
        let e = located(RuntimeError::WithNotes {
            notes: vec!["note".to_string()],
            help: Some("did you mean `x`?".into()),
            error: Box::new(RuntimeError::Type("bad".into())),
        });
        assert_eq!(e.kind(), "Type");
        assert_eq!(e.location(), Some(prima_syntax::Span::new(4, 9)));
    }

    #[test]
    fn notes_and_help_collect_across_the_wrapper_chain() {
        let inner = RuntimeError::WithNotes {
            notes: vec!["inner note".to_string()],
            help: None,
            error: Box::new(RuntimeError::Type("root failure".into())),
        };
        let outer = RuntimeError::WithNotes {
            notes: vec!["outer note".to_string()],
            help: Some("did you mean `outer`?".into()),
            error: Box::new(inner),
        };
        // Outermost first; the wrapped error's text stays the display string only.
        assert_eq!(
            outer.notes(),
            vec!["outer note".to_string(), "inner note".to_string()]
        );
        assert_eq!(outer.help().as_deref(), Some("did you mean `outer`?"));
        assert_eq!(outer.to_string(), "type error: root failure");

        // `Located` is transparent to the walk.
        let e = located(RuntimeError::WithNotes {
            notes: vec!["n".to_string()],
            help: Some("h".into()),
            error: Box::new(RuntimeError::Collapse("c".into())),
        });
        assert_eq!(e.notes(), vec!["n".to_string()]);
        assert_eq!(e.help().as_deref(), Some("h"));
        assert_eq!(e.to_string(), "collapse error: c");
    }

    #[test]
    fn notes_only_wrapper_has_no_help() {
        let e = RuntimeError::WithNotes {
            notes: vec!["n".to_string()],
            help: None,
            error: Box::new(RuntimeError::Type("m".into())),
        };
        assert_eq!(e.help(), None);
        assert_eq!(e.notes(), vec!["n".to_string()]);
    }

    #[test]
    fn code_maps_categories_and_delegates_through_wrappers() {
        assert_eq!(RuntimeError::Type("m".into()).code(), "R0009");
        assert_eq!(RuntimeError::Overflow("o".into()).code(), "R0001");
        assert_eq!(RuntimeError::IndexOutOfBounds("i".into()).code(), "R0003");
        assert_eq!(RuntimeError::Undefined("u".into()).code(), "R0006");
        assert_eq!(RuntimeError::Domain("d".into()).code(), "R0005");
        assert_eq!(RuntimeError::Type("t".into()).code(), "R0009");
        assert_eq!(RuntimeError::Collapse("c".into()).code(), "R0010");
        let empty = RuntimeError::EmptyCollection("empty".into());
        assert_eq!(empty.code(), "R0014");
        assert_eq!(empty.kind(), "EmptyCollection");
        // `Located`/`WithNotes` delegate to the enclosed error's code.
        assert_eq!(located(empty.clone()).code(), "R0014");
        assert_eq!(empty.clone().with_help("h").code(), "R0014");
        assert_eq!(empty.with_note("n").code(), "R0014");
    }

    #[test]
    fn value_problems_render_concretely() {
        let unequal = RuntimeError::invalid(
            "plot::line".into(),
            ValueProblem::UnequalLengths {
                left: "x",
                right: "y",
                first: 3,
                second: 4,
            },
        );
        assert_eq!(
            unequal.to_string(),
            "`plot::line` x and y must have equal length (3 vs 4)"
        );
        assert_eq!(unequal.code(), "R0011");

        let unsupported = RuntimeError::invalid(
            "plot::savefig".into(),
            ValueProblem::Unsupported {
                got: "png".into(),
                supported: "svg",
            },
        );
        assert_eq!(
            unsupported.to_string(),
            "`plot::savefig` unsupported `png`; supported: svg"
        );

        let grid = RuntimeError::invalid(
            "plot::heatmap".into(),
            ValueProblem::NotRectangular {
                row: 2,
                got: 3,
                expected: 4,
            },
        );
        assert_eq!(
            grid.to_string(),
            "`plot::heatmap` grid must be rectangular: row 2 has 3 columns, expected 4"
        );
    }

    #[test]
    fn runtime_error_stays_compact() {
        // `Result<Value, RuntimeError>` is returned on every evaluation step; keep the error
        // payload from inflating the success path.
        assert!(
            std::mem::size_of::<RuntimeError>() <= 64,
            "RuntimeError grew to {} bytes",
            std::mem::size_of::<RuntimeError>()
        );
    }

    #[test]
    fn call_errors_render_codes_and_kinds() {
        let arity = RuntimeError::Arity {
            callable: "f".into(),
            expected: Arity::Exact(2),
            got: 3,
        };
        assert_eq!(arity.to_string(), "`f` expects 2 argument(s), got 3");
        assert_eq!(arity.code(), "R0011");
        assert_eq!(arity.kind(), "Arity");
        assert_eq!(
            arity.with_note("n").to_string(),
            "`f` expects 2 argument(s), got 3"
        );

        let one_of = RuntimeError::Arity {
            callable: "norm".into(),
            expected: Arity::OneOf(&[1, 2]),
            got: 0,
        };
        assert_eq!(
            one_of.to_string(),
            "`norm` expects 1 or 2 argument(s), got 0"
        );
        assert_eq!(Arity::AtLeast(1).to_string(), "at least 1 argument(s)");

        let missing = RuntimeError::MissingArgument {
            callable: "f".into(),
            position: 1,
        };
        assert_eq!(missing.to_string(), "`f` missing argument 1");
        assert_eq!(missing.code(), "R0011");
        let missing_receiver = RuntimeError::MissingReceiver {
            callable: "String.len".into(),
        };
        assert_eq!(
            missing_receiver.to_string(),
            "`String.len` missing receiver"
        );

        let argument = RuntimeError::argument_type("f".into(), 1, "a number", "array".into());
        assert_eq!(
            argument.to_string(),
            "`f` argument 1 must be a number, got array"
        );
        assert_eq!(argument.code(), "R0009");
        let receiver =
            RuntimeError::receiver_type("String.len".into(), "a string", "number".into());
        assert_eq!(
            receiver.to_string(),
            "`String.len` expects a string receiver, got number"
        );
        assert_eq!(receiver.code(), "R0009");
    }
}
