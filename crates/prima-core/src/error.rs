#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum CoreError {
    #[error("division by zero")]
    DivisionByZero,
    #[error("{0}")]
    Other(String),
}
