use crate::span::Span;

/// Syntax error (spec §16.1 `SyntaxError`): carries the location span, a numbered compile-time
/// code (spec appendix C), the message, and an optional `help` hint. **Collection-based** at parse
/// time (multiple errors reported in one compilation, spec §16.2 compile-time errors).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("[{code}] {message} (span {span})")]
pub struct SyntaxError {
    pub span: Span,
    pub code: &'static str,
    pub message: String,
    pub help: Option<String>,
}

impl SyntaxError {
    pub fn new(code: &'static str, span: Span, message: impl Into<String>) -> Self {
        Self {
            span,
            code,
            message: message.into(),
            help: None,
        }
    }

    pub fn with_help(mut self, help: impl Into<String>) -> Self {
        self.help = Some(help.into());
        self
    }

    /// Generic syntax error (`E0010`).
    pub fn syntax(span: Span, message: impl Into<String>) -> Self {
        Self::new("E0010", span, message)
    }
}

/// Non-fatal syntax warning (spec §16.5): carries a numbered code (W####, spec appendix C)
/// and the source span. Warnings do not block compilation; deprecated constructs emit them.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("[{}] {} (span {})", self.code, self.message, self.span)]
pub struct SyntaxWarning {
    pub span: Span,
    pub code: &'static str,
    pub message: String,
}
