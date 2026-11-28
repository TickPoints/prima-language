//! Typed module-loading failures (spec §15.3 / appendix C.2).
//!
//! The loader reports two spec categories: I/O failures that prevent access or reading (`R0007`)
//! and resolution/parse failures (`R0008`). Keeping the operation context in `ModuleIoTarget` and
//! the underlying `io::Error` as the source lets Rust callers inspect them, while `RuntimeError`
//! renders the same diagnostic text the CLI has always shown.

use std::{fmt, io, path::PathBuf, sync::Arc};

use prima_syntax::SyntaxError;

/// The filesystem operation a module-load I/O failure came from (spec appendix C.2 `R0007`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModuleIoTarget {
    /// Canonicalizing the root (entry) module.
    AccessRoot(PathBuf),
    /// Canonicalizing a dependency module before reading it.
    AccessFile(PathBuf),
    /// Canonicalizing a resolved import target.
    AccessImport { module: String, path: PathBuf },
    /// Reading a module file whose path is already canonical.
    ReadFile { module: String, path: PathBuf },
}

impl fmt::Display for ModuleIoTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AccessRoot(path) => write!(f, "cannot access root module `{}`", path.display()),
            Self::AccessFile(path) => write!(f, "cannot access module `{}`", path.display()),
            Self::AccessImport { module, path } => {
                write!(f, "cannot access module `{module}` (`{}`)", path.display())
            }
            Self::ReadFile { module, path } => {
                write!(f, "cannot read module `{module}` (`{}`)", path.display())
            }
        }
    }
}

/// A module-load I/O failure (`R0007`) that retains the original OS error as its source.
#[derive(Debug, Clone, thiserror::Error)]
#[error("{target}: {cause}")]
pub struct ModuleIoError {
    pub target: ModuleIoTarget,
    #[source]
    pub cause: Arc<io::Error>,
}

impl ModuleIoError {
    pub(crate) fn new(target: ModuleIoTarget, cause: io::Error) -> Self {
        Self {
            target,
            cause: Arc::new(cause),
        }
    }
}

/// `io::Error` is not `PartialEq`, so compare the observable fields instead of skipping equality.
impl PartialEq for ModuleIoError {
    fn eq(&self, other: &Self) -> bool {
        self.target == other.target
            && self.cause.kind() == other.cause.kind()
            && self.cause.raw_os_error() == other.cause.raw_os_error()
            && self.cause.to_string() == other.cause.to_string()
    }
}

/// A module-resolution or parse failure (`R0008`, spec appendix C.2).
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ModuleImportError {
    #[error(
        "module `{module}` not found relative to `{dir}` (tried `{stem}.pra` and `{stem}/main.pra`)",
        module = .segments.join("::"),
        dir = .directory.display(),
        stem = .segments.join("/")
    )]
    Missing {
        directory: PathBuf,
        segments: Vec<String>,
    },
    #[error("import cycle: {}", .chain.join(" -> "))]
    Cycle { chain: Vec<String> },
    #[error(
        "module `{module}` (`{file}`) failed to parse: {details}",
        file = .path.display(),
        details = diagnostic_details(.diagnostics)
    )]
    ParseFile {
        module: String,
        path: PathBuf,
        diagnostics: Vec<SyntaxError>,
    },
    #[error(
        "embedded stdlib module `{module}` failed to parse: {}",
        diagnostic_details(.diagnostics)
    )]
    ParseEmbedded {
        module: String,
        diagnostics: Vec<SyntaxError>,
    },
}

/// Join collected parser diagnostics the way the loader has always reported them.
fn diagnostic_details(diagnostics: &[SyntaxError]) -> String {
    diagnostics
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}
