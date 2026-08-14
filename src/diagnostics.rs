//! Colored, rustc-style diagnostics via `codespan-reporting` (spec §16.4).
//!
//! `codespan-reporting` + `termcolor` provide portable ANSI color output
//! (auto-detected TTY, Windows console support). Errors with a byte span render
//! a `--> file:line:col` header with a caret; location-less runtime errors fall
//! back to a bold red `error:` line.

use std::io::Write;
use std::path::Path;

use codespan_reporting::diagnostic::{Diagnostic, Label, Severity};
use codespan_reporting::files::SimpleFile;
use codespan_reporting::term::termcolor::{Color, ColorChoice, ColorSpec, StandardStream, WriteColor};
use codespan_reporting::term::{emit_to_write_style, Chars, Config};
use prima_runtime::check::TypeError;
use prima_runtime::error::RuntimeError;
use prima_syntax::error::{SyntaxError, SyntaxWarning};

/// rustc-style rendering (`--> file:line:col`, spec §16.4).
fn term_config() -> Config {
    Config { chars: Chars::ascii(), ..Config::default() }
}

/// Render a full diagnostic: `<severity>[<code>]: <message>` header (code optional),
/// `--> file:line:col`, and a caret over the offending span (spec §16.4).
fn emit(file: &Path, source: &str, severity: Severity, code: Option<&str>, message: String, span: Option<(u32, u32)>, notes: Vec<String>) {
    let files = SimpleFile::new(file.display().to_string(), source.to_string());
    let mut diagnostic = Diagnostic::new(severity).with_message(message);
    if let Some(code) = code {
        diagnostic = diagnostic.with_code(code);
    }
    if let Some((start, end)) = span {
        diagnostic = diagnostic.with_labels(vec![Label::primary((), start as usize..end as usize)]);
    }
    if !notes.is_empty() {
        diagnostic = diagnostic.with_notes(notes);
    }
    let mut writer = StandardStream::stderr(ColorChoice::Auto);
    let config = term_config();
    let _ = emit_to_write_style(&mut writer, &config, &files, &diagnostic);
}

/// Bold red `error: <message>` line for location-less errors.
pub fn print_colored_error(message: &str) {
    let mut writer = StandardStream::stderr(ColorChoice::Auto);
    let _ = writer.set_color(ColorSpec::new().set_fg(Some(Color::Red)).set_bold(true));
    let _ = write!(writer, "error: ");
    let _ = writer.reset();
    let _ = writeln!(writer, "{message}");
}

/// Report collected parse errors (spec §16.4 diagnostic format).
pub fn report_syntax_errors(file: &Path, source: &str, errors: &[SyntaxError]) {
    for e in errors {
        emit(file, source, Severity::Error, None, e.message.clone(), Some((e.span.start, e.span.end)), Vec::new());
    }
}

/// Report static type errors from `prima check` (spec §16.2/§16.4).
pub fn report_type_errors(file: &Path, source: &str, errors: &[TypeError]) {
    for e in errors {
        // Suggest the explicit collapse for the common `Expr` → numeric mismatch (spec §16.4 提示).
        let notes = if e.message.contains("Expr") {
            vec!["help: collapse the expression explicitly, e.g. `to_f64(...)`".into()]
        } else {
            Vec::new()
        };
        emit(file, source, Severity::Error, None, e.message.clone(), Some((e.span.start, e.span.end)), notes);
    }
}

/// Report non-fatal warnings (spec §16.5): `warning[W####]: message` + caret. Warnings
/// do not affect the exit code; `prima check --deny W####` promotes a subset to errors.
pub fn report_warnings(file: &Path, source: &str, warnings: &[SyntaxWarning]) {
    for w in warnings {
        emit(file, source, Severity::Warning, Some(w.code), w.message.clone(), Some((w.span.start, w.span.end)), Vec::new());
    }
}

/// Report warnings promoted to errors by `--deny W####` (spec §16.5): re-render the same
/// diagnostic with an error severity and the numbered code, so the promoted failure is visible.
pub fn report_denied_warnings(file: &Path, source: &str, warnings: &[SyntaxWarning]) {
    for w in warnings {
        emit(
            file,
            source,
            Severity::Error,
            Some(w.code),
            w.message.clone(),
            Some((w.span.start, w.span.end)),
            vec![format!("help: `{}` is denied by `--deny` and promoted to an error", w.code)],
        );
    }
}

/// Report a runtime error. When the error carries a source span within the given
/// file, render the full diagnostic; otherwise fall back to a colored line.
pub fn report_runtime_error(file: &Path, source: &str, e: &RuntimeError) {
    let notes = match e.kind() {
        "Domain" => vec!["help: allow the operation with `with config { domain := complex }`".into()],
        "Undefined" => vec!["help: `Undefined` is a numeric-layer error state and cannot take part in operations (spec §6.2)".into()],
        "Collapse" => vec!["help: collapse the value with `to_<type>` before using it numerically (spec §9)".into()],
        _ => Vec::new(),
    };
    match e.location() {
        Some(span) if (span.end as usize) <= source.len() => {
            emit(file, source, Severity::Error, None, e.to_string(), Some((span.start, span.end)), notes);
        }
        _ => print_colored_error(&e.to_string()),
    }
}
