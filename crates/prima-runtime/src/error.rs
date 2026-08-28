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
            RuntimeError::Located { error, .. } => error.kind(),
            RuntimeError::WithNotes { error, .. } => error.kind(),
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
}
