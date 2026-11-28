//! I/O failure outside module loading (for example writing a rendered plot), retaining the
//! original OS error as the source (spec appendix C.2 `R0007`).

use std::{io, sync::Arc};

#[derive(Debug, Clone, thiserror::Error)]
#[error("cannot {operation} `{path}`: {cause}")]
pub struct IoFailure {
    /// Short verb phrase, e.g. `write`.
    pub operation: &'static str,
    pub path: String,
    #[source]
    pub cause: Arc<io::Error>,
}

impl IoFailure {
    pub fn new(operation: &'static str, path: impl Into<String>, cause: io::Error) -> Self {
        Self {
            operation,
            path: path.into(),
            cause: Arc::new(cause),
        }
    }
}

/// `io::Error` is not `PartialEq`; compare the observable fields.
impl PartialEq for IoFailure {
    fn eq(&self, other: &Self) -> bool {
        self.operation == other.operation
            && self.path == other.path
            && self.cause.kind() == other.cause.kind()
            && self.cause.raw_os_error() == other.cause.raw_os_error()
            && self.cause.to_string() == other.cause.to_string()
    }
}
