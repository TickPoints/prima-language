use crate::span::Span;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{} (span {})", self.message, self.span)]
pub struct SyntaxError {
    pub span: Span,
    pub message: String,
}
