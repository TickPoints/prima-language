/// Runtime error (spec §16): structured categories so `try/catch` can filter by type (spec §16.3),
/// carrying a human-readable message. The complete fields of the structured `Error` enum (§16.1) are deferred to a later phase.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum RuntimeError {
    #[error("{0}")]
    Message(String),
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
    /// Wraps an error with the source span of the statement/expression being evaluated,
    /// so diagnostics can point at the offending location (spec §16.4).
    #[error("{error}")]
    Located { span: prima_syntax::Span, error: Box<RuntimeError> },
}

impl RuntimeError {
    /// Error category name, used to match the filter in `catch e: Error::Overflow` (spec §16.3).
    pub fn kind(&self) -> &'static str {
        match self {
            RuntimeError::Message(_) => "Message",
            RuntimeError::Overflow(_) => "Overflow",
            RuntimeError::IndexOutOfBounds(_) => "IndexOutOfBounds",
            RuntimeError::Undefined(_) => "Undefined",
            RuntimeError::Domain(_) => "Domain",
            RuntimeError::Type(_) => "Type",
            RuntimeError::Collapse(_) => "Collapse",
            RuntimeError::Located { error, .. } => error.kind(),
        }
    }

    /// The source span attached to this error, if any (spec §16.4).
    pub fn location(&self) -> Option<prima_syntax::Span> {
        match self {
            RuntimeError::Located { span, .. } => Some(*span),
            _ => None,
        }
    }
}

/// Attach a source span to an error unless it already carries one (keep the deepest/most precise).
pub(crate) fn attach_span(e: RuntimeError, span: prima_syntax::Span) -> RuntimeError {
    match e {
        RuntimeError::Located { .. } => e,
        other => RuntimeError::Located { span, error: Box::new(other) },
    }
}

pub fn err<T>(message: impl Into<String>) -> Result<T, RuntimeError> {
    Err(RuntimeError::Message(message.into()))
}
