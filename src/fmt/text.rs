//! Text-emission helpers for `prima fmt` (spec §4.1/§18.1): block indentation, `///`/`//!` doc
//! comments, string/char re-escaping, and literal token emission. These are the leaf rendering
//! utilities shared by every other `fmt` submodule — they hold no AST-traversal state and only
//! write canonical text into the output buffer.

use prima_syntax::ast::{DocComment, Literal, Stmt, StringQuote};

/// Emit the doc lines of a `///`/`//!` comment (spec §4.1) as `///` comment lines at `indent`.
/// Leaves the output at the start of a new line (no trailing indent).
pub(crate) fn format_doc_lines(docs: &DocComment, indent: usize, out: &mut String, module: bool) {
    let marker = if module { "//! " } else { "/// " };
    for (line, _) in &docs.lines {
        push_indent(indent, out);
        out.push_str(marker);
        out.push_str(line);
        out.push('\n');
    }
}

/// The `///` doc attached to a definition statement, recursing through `pub` (spec §4.1).
pub(crate) fn stmt_docs(stmt: &Stmt) -> Option<&DocComment> {
    match stmt {
        Stmt::Let { docs, .. }
        | Stmt::Const { docs, .. }
        | Stmt::FnDef { docs, .. }
        | Stmt::MathDef { docs, .. }
        | Stmt::ClassDef { docs, .. } => docs.as_ref(),
        Stmt::Pub(inner) => stmt_docs(inner),
        _ => None,
    }
}

pub(crate) fn push_indent(indent: usize, out: &mut String) {
    for _ in 0..indent {
        out.push_str("    ");
    }
}

/// Re-escape a decoded char for re-emission (spec §18.1): the lexer accepts
/// `\n \t \r \0 \\ \" \'` and `\u{HEX}`; other control characters use `\u{...}`.
pub(crate) fn escape_string(c: char) -> String {
    match c {
        '\n' => "\\n".to_string(),
        '\t' => "\\t".to_string(),
        '\r' => "\\r".to_string(),
        '\0' => "\\0".to_string(),
        '\\' => "\\\\".to_string(),
        '"' => "\\\"".to_string(),
        '\'' => "\\'".to_string(),
        c if c.is_control() => {
            // `\u{...}` round-trips through the lexer's unicode escape (spec §18.1).
            format!("\\u{{{:X}}}", c as u32)
        }
        c => c.to_string(),
    }
}

/// Re-escape a decoded char literal: the char literal grammar has no `\u{}` form, so only
/// the shared escape set plus the raw char round-trip.
pub(crate) fn escape_char(c: char) -> String {
    match c {
        '\n' => "\\n".to_string(),
        '\t' => "\\t".to_string(),
        '\r' => "\\r".to_string(),
        '\0' => "\\0".to_string(),
        '\\' => "\\\\".to_string(),
        '\'' => "\\'".to_string(),
        '"' => "\\\"".to_string(),
        c => c.to_string(),
    }
}

/// Emit a literal in canonical source form (impl §3): numeric literals keep their raw text; string
/// literals round-trip via [`escape_string`] (or verbatim when a raw string stays lossless); char
/// literals via [`escape_char`]; TeX literals are lexed verbatim (spec §3 `tex"..."`, no escapes).
pub(crate) fn format_literal(lit: &Literal, out: &mut String) {
    match lit {
        // Numeric literals keep their raw source text (impl §3): re-emitting it is lossless.
        Literal::Integer(s) | Literal::Float(s) | Literal::Hex(s) | Literal::Binary(s) => {
            out.push_str(s)
        }
        Literal::String { value, quote, raw } => {
            let delim = match quote {
                StringQuote::Double => '"',
                StringQuote::Single => '\'',
            };
            // A raw string is re-emitted verbatim when the value does not contain the delimiter
            // (raw strings cannot escape it); otherwise fall back to the lossless escaped form.
            if *raw && !value.contains(delim) {
                out.push('r');
                out.push(delim);
                out.push_str(value);
                out.push(delim);
            } else {
                out.push(delim);
                for c in value.chars() {
                    out.push_str(&escape_string(c));
                }
                out.push(delim);
            }
        }
        Literal::Char(c) => {
            out.push('\'');
            out.push_str(&escape_char(*c));
            out.push('\'');
        }
        Literal::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        // TeX strings are lexed verbatim (no escapes, spec §3 `tex"..."`), so re-emit raw.
        Literal::Tex(s) => {
            out.push_str("tex\"");
            out.push_str(s);
            out.push('"');
        }
    }
}
