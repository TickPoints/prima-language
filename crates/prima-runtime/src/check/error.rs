//! Static-check error insertion and type/span rendering helpers (spec §16.4).
//!
//! These helpers build located `TypeError` values (deriving line/column from a span) and render
//! `Type` annotations for diagnostics — the shared plumbing used by the collector and the
//! call-signature checker.

use prima_syntax::Span;
use prima_syntax::ast::Type;

use crate::capi::c_type;

use super::TypeError;
use super::infer::annot_name;
use super::line_col;

/// Push a located error with a spec appendix C.1 code and no extra diagnostics (spec §16.4).
pub(crate) fn push_err(
    src: &str,
    errors: &mut Vec<TypeError>,
    span: Span,
    code: &'static str,
    message: String,
) {
    let (line, column) = line_col(src, span.start);
    errors.push(TypeError {
        line,
        column,
        span,
        code,
        message,
        notes: Vec::new(),
        help: None,
    });
}

/// Push a located error carrying a diagnostic note and an actionable `= help:` suggestion
/// (spec §16.4).
pub(crate) fn push_err_with_help(
    src: &str,
    errors: &mut Vec<TypeError>,
    span: Span,
    code: &'static str,
    message: String,
    note: Option<String>,
    help: Option<String>,
) {
    let (line, column) = line_col(src, span.start);
    errors.push(TypeError {
        line,
        column,
        span,
        code,
        message,
        notes: note.into_iter().collect(),
        help,
    });
}

/// Human-readable type name for diagnostics (`User` carries its source text).
pub(crate) fn type_display(t: &Type) -> String {
    match t {
        Type::User(sp) => sp.value.clone(),
        _ => annot_name(t).into(),
    }
}

/// Span of a type annotation for caret rendering; non-`User` types have no span, so a fallback is used.
pub(crate) fn type_span(t: &Type, fallback: Span) -> Span {
    match t {
        Type::User(sp) => sp.span,
        _ => fallback,
    }
}

/// Whether a type is allowed as a `@c_api::extern` parameter (spec appendix B.6): a C-compatible
/// type other than `c_api::unit` (`void` is only a return type).
pub(crate) fn c_param_ok(t: &Type) -> bool {
    matches!(c_type(t).as_deref(), Some(c) if c != "void")
}
