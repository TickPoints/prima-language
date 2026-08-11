use crate::span::Span;

/// Syntax error (spec §16.1 `SyntaxError`): carries the location span and a message,
/// **collection-based** at parse time (multiple errors reported in one compilation, spec §16.2 compile-time errors).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{} (span {})", self.message, self.span)]
pub struct SyntaxError {
    pub span: Span,
    pub message: String,
}

/// Non-fatal syntax warning (spec §16.5): carries a numbered code (`W0001` etc., spec appendix C)
/// and the source span. Warnings do not block compilation; deprecated constructs emit them.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("[{}] {} (span {})", self.code, self.message, self.span)]
pub struct SyntaxWarning {
    pub span: Span,
    pub code: &'static str,
    pub message: String,
}
