#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum CoreError {
    #[error("division by zero")]
    DivisionByZero,
}
