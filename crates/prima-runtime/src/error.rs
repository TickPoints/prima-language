/// Runtime error (spec §16): structured categories so `try/catch` can filter by type (spec §16.3),
/// carrying a human-readable message. The complete fields of the structured `Error` enum (§16.1) are deferred to a later release.
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
    /// A structured runtime error carrying its spec appendix C.2 code (spec §16.4). Used where the
    /// concrete category has no dedicated variant (underflow, dimension mismatch, I/O, import,
    /// missing key, not-found, empty collection) or where a precise code must override the
    /// category default.
    #[error("{message}")]
    Coded {
        code: &'static str,
        message: String,
    },
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
            // `Coded` is a general-purpose message carrier, so it maps to the `Message` category.
            RuntimeError::Coded { .. } => "Message",
            RuntimeError::Located { error, .. } => error.kind(),
            RuntimeError::WithNotes { error, .. } => error.kind(),
        }
    }

    /// The spec appendix C.2 runtime error code (spec §16.4). Concrete categories map to their
    /// `R####` code; `Coded` carries an explicit one; wrappers delegate to the error they enclose.
    pub fn code(&self) -> &'static str {
        match self {
            RuntimeError::Message(_) => "R0011",
            RuntimeError::Overflow(_) => "R0001",
            RuntimeError::IndexOutOfBounds(_) => "R0003",
            RuntimeError::Undefined(_) => "R0006",
            RuntimeError::Domain(_) => "R0005",
            RuntimeError::Type(_) => "R0009",
            RuntimeError::Collapse(_) => "R0010",
            RuntimeError::Coded { code, .. } => code,
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
        RuntimeError::Located { .. } => e,
        other => RuntimeError::Located {
            span,
            error: Box::new(other),
        },
    }
}

pub fn err<T>(message: impl Into<String>) -> Result<T, RuntimeError> {
    Err(RuntimeError::Message(message.into()))
}

/// Build a [`RuntimeError::Coded`] for a spec appendix C.2 category that has no dedicated variant
/// (underflow, dimension mismatch, I/O, import, key-not-found, not-found, empty collection).
pub(crate) fn coded(code: &'static str, message: impl Into<String>) -> RuntimeError {
    RuntimeError::Coded {
        code,
        message: message.into(),
    }
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
            error: Box::new(RuntimeError::Message("root failure".into())),
        };
        let outer = RuntimeError::WithNotes {
            notes: vec!["outer note".to_string()],
            help: Some("did you mean `outer`?".into()),
            error: Box::new(inner),
        };
        // Outermost first, and the wrapped `Message`'s own text stays the display string only.
        assert_eq!(
            outer.notes(),
            vec!["outer note".to_string(), "inner note".to_string()]
        );
        assert_eq!(outer.help().as_deref(), Some("did you mean `outer`?"));
        assert_eq!(outer.to_string(), "root failure");

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
            error: Box::new(RuntimeError::Message("m".into())),
        };
        assert_eq!(e.help(), None);
        assert_eq!(e.notes(), vec!["n".to_string()]);
    }

    #[test]
    fn code_maps_categories_and_delegates_through_wrappers() {
        assert_eq!(RuntimeError::Message("m".into()).code(), "R0011");
        assert_eq!(RuntimeError::Overflow("o".into()).code(), "R0001");
        assert_eq!(RuntimeError::IndexOutOfBounds("i".into()).code(), "R0003");
        assert_eq!(RuntimeError::Undefined("u".into()).code(), "R0006");
        assert_eq!(RuntimeError::Domain("d".into()).code(), "R0005");
        assert_eq!(RuntimeError::Type("t".into()).code(), "R0009");
        assert_eq!(RuntimeError::Collapse("c".into()).code(), "R0010");
        let coded = RuntimeError::Coded {
            code: "R0014",
            message: "empty".into(),
        };
        assert_eq!(coded.code(), "R0014");
        // `Coded` stays in the `Message` `kind` category for `catch` matching.
        assert_eq!(coded.kind(), "Message");
        // `Located`/`WithNotes` delegate to the enclosed error's code.
        assert_eq!(located(coded.clone()).code(), "R0014");
        assert_eq!(coded.clone().with_help("h").code(), "R0014");
        assert_eq!(coded.with_note("n").code(), "R0014");
    }
}
