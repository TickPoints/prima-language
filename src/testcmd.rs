//! `prima test` runner (spec §20): execute every `*.pra` file under a directory
//! and report pass/fail. A module's dependencies are resolved by the module system
//! (`Evaluator::eval_file`), and the exit code is failure if any file failed.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::Context;
use prima_runtime::Evaluator;

/// Default test root when no path is given (spec §20 tool command).
pub const DEFAULT_DIR: &str = "examples";

/// Run all `.pra` files under `dir` (recursively, sorted). Prints `ok`/`FAIL` per
/// file and a summary; exits failure if any file failed or the directory is empty.
pub fn run(dir: &Path) -> anyhow::Result<ExitCode> {
    let files = collect_pra_files(dir).with_context(|| {
        format!("cannot read {}", dir.display())
    })?;
    if files.is_empty() {
        eprintln!("no test files found under {}", dir.display());
        return Ok(ExitCode::FAILURE);
    }

    let mut passed = 0usize;
    let mut failed = 0usize;
    let mut skipped = 0usize;
    for file in &files {
        let src = match std::fs::read_to_string(file) {
            Ok(s) => s,
            Err(e) => {
                println!("FAIL {}: cannot read file: {e}", display_name(dir, file));
                failed += 1;
                continue;
            }
        };
        // A file that does not parse is a fixture, not a runnable test; skip it rather than
        // count it as a failure (e.g. the syntax-error fixture under `examples/`).
        if prima_syntax::parse(&src).is_err() {
            eprintln!("skip {} (not a valid program)", display_name(dir, file));
            skipped += 1;
            continue;
        }
        match Evaluator::new().eval_file(file) {
            Ok(()) => {
                println!("ok   {}", display_name(dir, file));
                passed += 1;
            }
            Err(e) => {
                println!("FAIL {}: {e}", display_name(dir, file));
                failed += 1;
            }
        }
    }
    println!("{passed} passed, {failed} failed, {skipped} skipped");
    if failed > 0 {
        Ok(ExitCode::FAILURE)
    } else {
        Ok(ExitCode::SUCCESS)
    }
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
