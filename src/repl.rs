//! `prima repl` interactive interpreter loop (spec §20).
//!
//! Uses `rustyline` for line editing and history. Input is accumulated across
//! continuation lines until the delimiters balance, then evaluated.
//!
//! The session keeps one persistent evaluator and one persistent `Env`, evaluating only the
//! new entry against it (`Evaluator::eval_value_keep_env`). Bindings survive across entries
//! without replaying the whole session, so long sessions stay O(1) per entry.

use std::cell::RefCell;
use std::io::{self, Write};
use std::process::ExitCode;
use std::rc::Rc;

use prima_core::Value;
use prima_runtime::{Env, EnvRef, Evaluator};
use prima_syntax::ast::Stmt;
use rustyline::DefaultEditor;
use rustyline::error::ReadlineError;

use anyhow::Context;

const PROMPT: &str = ">> ";
const CONTINUATION: &str = "... ";
const BANNER: &str = "Prima REPL v0.1.0 — Ctrl-D to exit";

/// Run the interactive REPL loop. Returns the process exit code; the only failure that
/// propagates with context is a failure to initialize the line editor.
pub fn run() -> anyhow::Result<ExitCode> {
    println!("{BANNER}");
    let mut editor = DefaultEditor::new().context("cannot initialize the REPL line editor")?;
    let printed = Rc::new(RefCell::new(String::new()));
    let sink = printed.clone();
    let mut ev = Evaluator::with_sink(move |s| sink.borrow_mut().push_str(&s));

    // Persistent session environment: bindings survive across entries (no session replay).
    let env = Env::new().into_ref();

    // The currently accumulated (possibly multi-line) entry.
    let mut buffer = String::new();
    loop {
        let prompt = if buffer.is_empty() {
            PROMPT
        } else {
            CONTINUATION
        };
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
                    eval_entry(&mut ev, &env, &printed, &buffer);
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
    Ok(ExitCode::SUCCESS)
}

/// REPL exit commands, only recognized on an empty buffer (so a pending multi-line
/// buffer is not aborted by a stray `quit`).
fn is_quit(line: &str) -> bool {
    matches!(line.trim(), ":q" | ":quit" | "quit" | "exit")
}

/// Evaluate one complete entry against the persistent session environment. The entry is
/// `;`-terminated (spec §4.2) and evaluated exactly once — no session replay — so long sessions
/// stay O(1) per entry. Its captured output is printed, and the trailing expression's value is
/// shown when the entry yields one. On error, the entry's earlier statements may already have
/// taken effect (standard REPL behavior); the buffer is discarded so it can be re-entered.
fn eval_entry(ev: &mut Evaluator, env: &EnvRef, printed: &Rc<RefCell<String>>, buffer: &str) {
    // Clear the capture sink so it holds only this entry's output.
    *printed.borrow_mut() = String::new();
    let src = terminate_with_semicolon(buffer);
    match ev.eval_value_keep_env(env, &src) {
        Ok(result) => {
            let captured = printed.borrow().clone();
            print!("{captured}");
            let _ = io::stdout().flush();
            // Print the trailing value only when the entry itself ends in a value-yielding
            // statement (an expression or a `match`, spec §4.4).
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
    let Ok(program) = prima_syntax::parse(buffer) else {
        return false;
    };
    matches!(
        program.stmts.last(),
        Some(Stmt::Expr(_)) | Some(Stmt::Match { .. })
    )
}

/// Each entry is evaluated as a standalone program, so it must be `;`-terminated: newline is no
/// longer a statement separator (spec §4.2). Block-level statements accept the trailing `;`; an
/// entry that already ends in `;` is left unchanged.
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
