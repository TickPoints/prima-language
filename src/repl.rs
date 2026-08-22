//! `prima repl` interactive interpreter loop (spec §20).
//!
//! Uses `rustyline` for line editing and history. Input is accumulated across
//! continuation lines until the delimiters balance, then evaluated.
//!
//! The crate's `Evaluator::eval_value` creates a fresh `Env` on every call, so
//! variable bindings would not survive across entries. The REPL therefore keeps
//! one persistent evaluator and replays the whole session on each complete entry;
//! `println`/`print` output is captured and only the tail generated since the
//! previous entry is shown, so side effects appear exactly once.

use std::cell::RefCell;
use std::io::{self, Write};
use std::process::ExitCode;
use std::rc::Rc;

use prima_core::Value;
use prima_runtime::Evaluator;
use prima_syntax::ast::Stmt;
use rustyline::DefaultEditor;
use rustyline::error::ReadlineError;

const PROMPT: &str = ">> ";
const CONTINUATION: &str = "... ";
const BANNER: &str = "Prima REPL v0.1.0 — Ctrl-D to exit";

/// Run the interactive REPL loop. Returns the process exit code.
pub fn run() -> ExitCode {
    println!("{BANNER}");
    let mut editor = match DefaultEditor::new() {
        Ok(e) => e,
        Err(e) => {
            eprintln!("error: failed to initialize the REPL: {e}");
            return ExitCode::FAILURE;
        }
    };
    let printed = Rc::new(RefCell::new(String::new()));
    let sink = printed.clone();
    let mut ev = Evaluator::with_sink(move |s| sink.borrow_mut().push_str(&s));

    // The committed session (bindings) and the currently accumulated line.
    let mut session = String::new();
    let mut prev_output = String::new();
    let mut buffer = String::new();
    loop {
        let prompt = if buffer.is_empty() { PROMPT } else { CONTINUATION };
        match editor.readline(prompt) {
            Ok(line) => {
                let _ = editor.add_history_entry(line.as_str());
                if buffer.is_empty() && is_quit(&line) {
                    break;
                }
                buffer.push_str(&line);
                buffer.push('\n');
                if buffer.trim().is_empty() {
                    buffer.clear();
                    continue;
                }
                if balanced_delimiters(&buffer) {
                    let mut candidate = session.clone();
                    candidate.push_str(&buffer);
                    eval_candidate(&mut ev, &printed, &candidate, &buffer, &mut session, &mut prev_output);
                    buffer.clear();
                }
            }
            Err(ReadlineError::Eof) | Err(ReadlineError::Interrupted) => break,
            Err(e) => {
                eprintln!("error: {e}");
                break;
            }
        }
    }
    ExitCode::SUCCESS
}

/// REPL exit commands, only recognized on an empty buffer (so a pending multi-line
/// buffer is not aborted by a stray `quit`).
fn is_quit(line: &str) -> bool {
    matches!(line.trim(), ":q" | ":quit" | "quit" | "exit")
}

/// Evaluate the full session (previous entries + the new buffer) on the persistent
/// evaluator. On success the newly-emitted output tail and the resulting value are
/// printed and the candidate becomes the committed session; on failure the buffer is
/// discarded so it can be corrected and re-entered.
fn eval_candidate(
    ev: &mut Evaluator,
    printed: &Rc<RefCell<String>>,
    candidate: &str,
    buffer: &str,
    session: &mut String,
    prev_output: &mut String,
) {
    // Clear the capture sink so it holds only this run's output (it otherwise accumulates).
    *printed.borrow_mut() = String::new();
    match ev.eval_value(candidate) {
        Ok(result) => {
            let captured = printed.borrow().clone();
            // Re-evaluation replays earlier output deterministically; show only what the new
            // entry produced. If the prefix drifted (e.g. `input()`), fall back to full output.
            let tail = match captured.strip_prefix(prev_output.as_str()) {
                Some(rest) => rest.to_string(),
                None => captured.clone(),
            };
            print!("{tail}");
            let _ = io::stdout().flush();
            *prev_output = captured;
            *session = terminate_with_semicolon(candidate);
            // `eval_value` reports the last expression's value over the whole session; print it
            // only when the new entry itself ends in a value-yielding statement.
            if yields_value(buffer) && !matches!(result, Value::Nil) {
                let text = ev.format_value(&result);
                let mut stdout = io::stdout().lock();
                let _ = writeln!(stdout, "{text}");
                let _ = stdout.flush();
            }
        }
        Err(e) => eprintln!("error: {e}"),
    }
}

/// Whether the entry's trailing statement yields a value (an expression or a `match`,
/// per `Evaluator::eval_value`, spec §4.4); a `let`/`class`/control-flow entry does not.
fn yields_value(buffer: &str) -> bool {
    let Ok(program) = prima_syntax::parse(buffer) else { return false };
    matches!(program.stmts.last(), Some(Stmt::Expr(_)) | Some(Stmt::Match { .. }))
}

/// The committed session replays as a single program (session + next entry), so each entry
/// must be `;`-terminated: newline is no longer a statement separator (spec §4.2). Block-level
/// statements accept the trailing `;`; an entry that already ends in `;` is left unchanged.
fn terminate_with_semicolon(src: &str) -> String {
    let trimmed = src.trim_end();
    if trimmed.ends_with(';') {
        src.to_string()
    } else {
        format!("{trimmed};\n")
    }
}

/// Whether a (possibly multi-line) buffer has balanced `{ } [ ] ( )`, ignoring
/// delimiters inside `"..."`/`'...'` string and char literals and `//` comments.
fn balanced_delimiters(src: &str) -> bool {
    let mut stack: Vec<char> = Vec::new();
    let mut chars = src.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                // Skip to the closing quote, honoring backslash escapes (spec §18.1).
                while let Some(next) = chars.next() {
                    if next == '\\' {
                        chars.next();
                    } else if next == '"' {
                        break;
                    }
                }
            }
            '\'' => {
                while let Some(next) = chars.next() {
                    if next == '\\' {
                        chars.next();
                    } else if next == '\'' {
                        break;
                    }
                }
            }
            '/' if chars.peek() == Some(&'/') => {
                // Line comment: skip to end of line.
                for next in chars.by_ref() {
                    if next == '\n' {
                        break;
                    }
                }
            }
            '{' | '[' | '(' => stack.push(c),
            '}' | ']' | ')' => match (stack.last(), c) {
                (Some('{'), '}') | (Some('['), ']') | (Some('('), ')') => {
                    stack.pop();
                }
                _ => return false,
            },
            _ => {}
        }
    }
    stack.is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn balanced_simple() {
        assert!(balanced_delimiters("1 + 2;\n"));
        assert!(balanced_delimiters("let f(x) = x^2;\n"));
    }

    #[test]
    fn unbalanced_waits_for_continuation() {
        assert!(!balanced_delimiters("if x > 0 {\n"));
        assert!(balanced_delimiters("if x > 0 {\n    println(x);\n}\n"));
    }

    #[test]
    fn delimiters_inside_strings_and_comments_are_ignored() {
        assert!(balanced_delimiters("let s = \"{[\";\n"));
        assert!(balanced_delimiters("let s = \"a\\\"{\";\n"));
        assert!(balanced_delimiters("let s = '{';\n"));
        assert!(balanced_delimiters("// } ] ) not counted\nlet x = 1;\n"));
    }

    #[test]
    fn mismatched_delimiters_are_not_balanced() {
        assert!(!balanced_delimiters("(1 + 2];\n"));
    }
}
