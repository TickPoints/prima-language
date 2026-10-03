//! Colored, rustc-style diagnostics via `codespan-reporting` (spec §16.4).
//!
//! All user-facing diagnostics flow through this module so the output stays consistent and can
//! switch between:
//! - human text: `error[CODE]: message`, `--> file:line:col`, a caret, `= note:` / `= help:` lines;
//! - machine JSON: one rustc-style JSON object per line (NDJSON) on stderr, keeping program
//!   stdout clean for pipes and editors.
//!
//! Options (color policy, `--json`, `--quiet`) are installed once from the CLI and read by every
//! report function, so callers do not have to thread them through the toolchain.

use std::io::Write;
use std::path::Path;
use std::sync::OnceLock;

use codespan_reporting::diagnostic::{Diagnostic, Label, Severity};
use codespan_reporting::files::SimpleFile;
use codespan_reporting::term::termcolor::{
    Color, ColorChoice, ColorSpec, StandardStream, WriteColor,
};
use codespan_reporting::term::{Chars, Config, emit_to_write_style};
use prima_runtime::check::TypeError;
use prima_runtime::error::RuntimeError;
use prima_syntax::error::{SyntaxError, SyntaxWarning};
use serde_json::{Value, json};

/// Color policy for human diagnostics (`--color`).
#[derive(Clone, Copy, Debug, Default)]
pub enum ColorMode {
    /// Color only when stderr is a terminal (default).
    #[default]
    Auto,
    Always,
    Never,
}

impl ColorMode {
    fn choice(self) -> ColorChoice {
        match self {
            ColorMode::Auto => ColorChoice::Auto,
            ColorMode::Always => ColorChoice::Always,
            ColorMode::Never => ColorChoice::Never,
        }
    }
}

/// Output options installed once from the CLI (spec §20).
#[derive(Clone, Copy, Debug, Default)]
pub struct RenderOptions {
    pub color: ColorMode,
    /// Emit NDJSON diagnostics on stderr instead of human-readable text.
    pub json: bool,
    /// Suppress non-fatal warnings.
    pub quiet: bool,
}

static OPTIONS: OnceLock<RenderOptions> = OnceLock::new();

/// Install the CLI's render options. Idempotent: the first call wins.
pub fn set_options(options: RenderOptions) {
    let _ = OPTIONS.set(options);
}

fn options() -> &'static RenderOptions {
    OPTIONS.get_or_init(RenderOptions::default)
}

/// Whether `--json` was requested (used by non-diagnostic commands such as `prima test`).
pub fn json_enabled() -> bool {
    options().json
}

/// Whether `--quiet` was requested.
pub fn quiet_enabled() -> bool {
    options().quiet
}

/// rustc-style rendering (`--> file:line:col`, spec §16.4).
fn term_config() -> Config {
    Config {
        chars: Chars::ascii(),
        ..Config::default()
    }
}

#[derive(Clone, Copy)]
enum Level {
    Error,
    Warning,
}

/// A single structured diagnostic, independent of the renderer.
struct Diag {
    level: Level,
    code: Option<&'static str>,
    message: String,
    span: Option<(u32, u32)>,
    notes: Vec<String>,
    help: Option<String>,
}

/// Render a diagnostic in the configured format.
fn emit(file: &Path, source: &str, diag: Diag) {
    if options().json {
        emit_json(file, source, diag);
    } else {
        emit_text(file, source, diag);
    }
}

/// Human-readable rendering: header (with code), location, caret, notes, `= help:`.
fn emit_text(file: &Path, source: &str, diag: Diag) {
    let files = SimpleFile::new(file.display().to_string(), source.to_string());
    let severity = match diag.level {
        Level::Error => Severity::Error,
        Level::Warning => Severity::Warning,
    };
    let mut diagnostic = Diagnostic::new(severity).with_message(&diag.message);
    if let Some(code) = diag.code {
        diagnostic = diagnostic.with_code(code);
    }
    if let Some((start, end)) = diag.span {
        diagnostic =
            diagnostic.with_labels(vec![Label::primary((), start as usize..end as usize)]);
    }
    if !diag.notes.is_empty() {
        diagnostic = diagnostic.with_notes(diag.notes.clone());
    }
    let buffer_writer = codespan_reporting::term::termcolor::BufferWriter::stderr(
        options().color.choice(),
    );
    let mut buffer = buffer_writer.buffer();
    let _ = emit_to_write_style(&mut buffer, &term_config(), &files, &diagnostic);
    if let Some(help) = &diag.help {
        let _ = writeln!(buffer, "= help: {help}");
    }
    let _ = buffer_writer.print(&buffer);
}

/// Machine-readable rendering: one JSON object per diagnostic on stderr (NDJSON).
fn emit_json(file: &Path, source: &str, diag: Diag) {
    let spans: Value = match diag.span {
        Some((start, end)) => {
            let (line, column) = line_col(source, start);
            json!([{
                "file": file.display().to_string(),
                "line": line,
                "column": column,
                "byte_start": start,
                "byte_end": end,
                "is_primary": true,
            }])
        }
        None => json!([]),
    };
    let value = json!({
        "severity": match diag.level { Level::Error => "error", Level::Warning => "warning" },
        "code": diag.code,
        "message": diag.message,
        "spans": spans,
        "notes": diag.notes,
        "help": diag.help,
    });
    eprintln!("{value}");
}

/// 1-based line and character column of a byte offset, for JSON spans.
fn line_col(source: &str, byte: u32) -> (usize, usize) {
    let upto = &source[..(byte as usize).min(source.len())];
    let line = upto.bytes().filter(|&b| b == b'\n').count() + 1;
    let column = upto.rsplit('\n').next().map_or(1, |l| l.chars().count() + 1);
    (line, column)
}

/// Bold red `error: <message>` line for location-less errors (or a JSON object under `--json`).
pub fn print_colored_error(message: &str) {
    if options().json {
        let value = json!({
            "severity": "error",
            "code": Value::Null,
            "message": message,
            "spans": [],
            "notes": [],
            "help": Value::Null,
        });
        eprintln!("{value}");
        return;
    }
    print_coded_error(None, message, &[], None);
}

/// Location-less human diagnostic with an optional code and note/help lines.
fn print_coded_error(code: Option<&str>, message: &str, notes: &[String], help: Option<&str>) {
    let mut writer = StandardStream::stderr(options().color.choice());
    let _ = writer.set_color(ColorSpec::new().set_fg(Some(Color::Red)).set_bold(true));
    match code {
        Some(code) => {
            let _ = write!(writer, "error[{code}]: ");
        }
        None => {
            let _ = write!(writer, "error: ");
        }
    }
    let _ = writer.reset();
    let _ = writeln!(writer, "{message}");
    for note in notes {
        let _ = writeln!(writer, "= note: {note}");
    }
    if let Some(help) = help {
        let _ = writeln!(writer, "= help: {help}");
    }
}

/// Report a runtime error without a source file (the REPL): `error[CODE]: message`, notes and help.
pub fn report_runtime_error_line(e: &RuntimeError) {
    let notes = e.notes();
    let help = e.help();
    if options().json {
        emit_json(
            Path::new("<repl>"),
            "",
            Diag {
                level: Level::Error,
                code: Some(e.code()),
                message: e.to_string(),
                span: None,
                notes,
                help,
            },
        );
    } else {
        print_coded_error(Some(e.code()), &e.to_string(), &notes, help.as_deref());
    }
}

/// Report collected parse errors (spec §16.4 diagnostic format).
pub fn report_syntax_errors(file: &Path, source: &str, errors: &[SyntaxError]) {
    for e in errors {
        emit(
            file,
            source,
            Diag {
                level: Level::Error,
                code: Some(e.code),
                message: e.message.clone(),
                span: Some((e.span.start, e.span.end)),
                notes: Vec::new(),
                help: e.help.clone(),
            },
        );
    }
}

/// Report static type errors from `prima check` (spec §16.2/§16.4).
pub fn report_type_errors(file: &Path, source: &str, errors: &[TypeError]) {
    for e in errors {
        emit(
            file,
            source,
            Diag {
                level: Level::Error,
                code: Some(e.code),
                message: e.message.clone(),
                span: Some((e.span.start, e.span.end)),
                notes: e.notes.clone(),
                help: e.help.clone(),
            },
        );
    }
}

/// Report non-fatal warnings (spec §16.5): `warning[W####]: message` + caret. Warnings
/// do not affect the exit code; `prima check --deny W####` promotes a subset to errors.
pub fn report_warnings(file: &Path, source: &str, warnings: &[SyntaxWarning]) {
    if options().quiet {
        return;
    }
    for w in warnings {
        emit(
            file,
            source,
            Diag {
                level: Level::Warning,
                code: Some(w.code),
                message: w.message.clone(),
                span: Some((w.span.start, w.span.end)),
                notes: Vec::new(),
                help: None,
            },
        );
    }
}

/// Report warnings promoted to errors by `--deny W####` (spec §16.5): re-render the same
/// diagnostic with an error severity and the numbered code, so the promoted failure is visible.
pub fn report_denied_warnings(file: &Path, source: &str, warnings: &[SyntaxWarning]) {
    for w in warnings {
        emit(
            file,
            source,
            Diag {
                level: Level::Error,
                code: Some(w.code),
                message: w.message.clone(),
                span: Some((w.span.start, w.span.end)),
                notes: vec![format!(
                    "help: `{}` is denied by `--deny` and promoted to an error",
                    w.code
                )],
                help: None,
            },
        );
    }
}

/// Report a runtime error. When the error carries a source span within the given file, render
/// the full diagnostic; otherwise fall back to a colored/JSON line. Category hints (spec §16.4)
/// and evaluator-attached notes are rendered as `= note:`, a `did you mean` help as `= help:`.
pub fn report_runtime_error(file: &Path, source: &str, e: &RuntimeError) {
    // Category-specific hints become the `= help:` line unless the evaluator attached a more
    // specific suggestion (a `did you mean`), which wins.
    let kind_hint = match e.kind() {
        "Domain" => Some("allow the operation with `with config { domain := complex }`"),
        "Undefined" => {
            Some("`Undefined` is a numeric-layer error state and cannot take part in operations (spec §6.2)")
        }
        "Collapse" => Some("collapse the value with `to_<type>` before using it numerically (spec §9)"),
        _ => None,
    };
    let notes: Vec<String> = e.notes();
    let help = e.help().or_else(|| kind_hint.map(String::from));
    let diag = |span: Option<(u32, u32)>| Diag {
        level: Level::Error,
        code: Some(e.code()),
        message: e.to_string(),
        span,
        notes: notes.clone(),
        help: help.clone(),
    };
    match e.location() {
        Some(span) if (span.end as usize) <= source.len() => {
            emit(file, source, diag(Some((span.start, span.end))));
        }
        _ => {
            if options().json {
                emit_json(file, source, diag(None));
            } else {
                print_coded_error(Some(e.code()), &e.to_string(), &notes, help.as_deref());
            }
        }
    }
}
