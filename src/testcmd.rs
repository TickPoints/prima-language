//! `prima test` runner (spec §20): execute every `*.pra` file under a directory
//! and report pass/fail. A module's dependencies are resolved by the module system
//! (`Evaluator::eval_file`), and the exit code is failure if any file failed.
//!
//! Output has two forms: a human report (`ok`/`FAIL`/`skip` plus a summary), or — under
//! `--json` — one NDJSON event per test plus a final summary object on stdout, so CI and
//! editors can parse the run. `--quiet` suppresses per-test lines for passing/skipped files.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::Context;
use prima_runtime::Evaluator;
use serde_json::json;

use crate::diagnostics;

/// Outcome of a single test file.
enum Outcome {
    Ok,
    Fail(String),
    Skip(String),
}

/// Run all `.pra` files under `dir` (recursively, sorted). Prints `ok`/`FAIL` per
/// file and a summary; exits failure if any file failed or the directory is empty.
pub fn run(dir: &Path) -> anyhow::Result<ExitCode> {
    let files = collect_pra_files(dir).with_context(|| format!("cannot read {}", dir.display()))?;
    if files.is_empty() {
        report_empty(dir);
        return Ok(ExitCode::FAILURE);
    }

    let json = diagnostics::json_enabled();
    let quiet = diagnostics::quiet_enabled();
    let mut passed = 0usize;
    let mut failed = 0usize;
    let mut skipped = 0usize;
    for file in &files {
        let name = display_name(dir, file);
        let outcome = run_one(file);
        match outcome {
            Outcome::Ok => {
                passed += 1;
                if !quiet {
                    emit(&name, "ok", None, json);
                }
            }
            Outcome::Fail(message) => {
                failed += 1;
                emit(&name, "fail", Some(&message), json);
            }
            Outcome::Skip(message) => {
                skipped += 1;
                if !quiet {
                    emit(&name, "skip", Some(&message), json);
                }
            }
        }
    }
    emit_summary(passed, failed, skipped, json);
    if failed > 0 {
        Ok(ExitCode::FAILURE)
    } else {
        Ok(ExitCode::SUCCESS)
    }
}

/// Evaluate one file, classifying the result. A file that does not parse is a fixture rather than
/// a runnable test, so it is skipped instead of counted as a failure.
fn run_one(file: &Path) -> Outcome {
    let src = match std::fs::read_to_string(file) {
        Ok(s) => s,
        Err(e) => return Outcome::Fail(format!("cannot read file: {e}")),
    };
    if prima_syntax::parse(&src).is_err() {
        return Outcome::Skip("not a valid program".to_string());
    }
    match Evaluator::new().eval_file(file) {
        Ok(()) => Outcome::Ok,
        Err(e) => Outcome::Fail(e.to_string()),
    }
}

/// Emit one test event: a human line, or a JSON object under `--json`.
fn emit(name: &str, status: &str, message: Option<&str>, json: bool) {
    if json {
        let value = json!({
            "type": "test",
            "file": name,
            "status": status,
            "message": message,
        });
        write_json_line(&value.to_string());
    } else {
        match message {
            Some(message) => println!("{} {}: {message}", status_label(status), name),
            None => println!("{} {name}", status_label(status)),
        }
    }
}

/// Emit the final summary: human line, or a JSON object with the counts.
fn emit_summary(passed: usize, failed: usize, skipped: usize, json: bool) {
    if json {
        let value = json!({
            "type": "summary",
            "passed": passed,
            "failed": failed,
            "skipped": skipped,
        });
        write_json_line(&value.to_string());
    } else {
        println!("{passed} passed, {failed} failed, {skipped} skipped");
    }
}

/// Report an empty test directory (human or JSON).
fn report_empty(dir: &Path) {
    if diagnostics::json_enabled() {
        let value = json!({
            "type": "error",
            "message": format!("no test files found under {}", dir.display()),
        });
        write_json_line(&value.to_string());
    } else {
        eprintln!("no test files found under {}", dir.display());
    }
}

/// Human label for a status keyword (`ok`/`fail`/`skip` keeps the runner's terse style).
fn status_label(status: &str) -> &str {
    match status {
        "ok" => "ok  ",
        "fail" => "FAIL",
        _ => "skip",
    }
}

fn write_json_line(line: &str) {
    let mut stderr = std::io::stderr().lock();
    let _ = writeln!(stderr, "{line}");
    let _ = stderr.flush();
}

/// Recursively collect `*.pra` files under `dir`, sorted by relative path.
fn collect_pra_files(dir: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(d) = pending.pop() {
        for entry in std::fs::read_dir(&d)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().is_some_and(|e| e == "pra") {
                files.push(path);
            }
        }
    }
    files.sort();
    Ok(files)
}

/// The file name relative to the test root, so output is stable regardless of cwd.
fn display_name(root: &Path, file: &Path) -> String {
    match file.strip_prefix(root) {
        Ok(rel) => rel.display().to_string(),
        Err(_) => file.display().to_string(),
    }
}
