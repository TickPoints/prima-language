#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum RuntimeError {
    #[error("{0}")]
    Message(String),
}

pub fn err<T>(message: impl Into<String>) -> Result<T, RuntimeError> {
    Err(RuntimeError::Message(message.into()))
}
