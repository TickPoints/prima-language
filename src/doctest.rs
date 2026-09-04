//! Doc-test runner (spec §20): extract ` ```pra ` fenced code blocks from `///` doc comments and
//! validate them — statically via `prima_runtime::check::check_src_checked` and, optionally, by
//! executing them and comparing the captured `print`/`println` output to the block's following
//! `// expect: ...` line (Rust-doc-test style expected output).
//!
//! Extraction walks the parsed `Program` and every `DocComment` (module `//!`, statement `///`, and
//! class member `///`). A block is any fenced group whose info string is `pra` (or empty). The
//! check is always run; execution is gated by `run` so `prima doc` stays lightweight.

use std::path::Path;
use std::process::ExitCode;

use prima_runtime::Evaluator;
use prima_runtime::check::check_src_checked;
use prima_syntax::ast::{ClassMemberKind, DocComment, Program, Stmt};
use prima_syntax::parse;

/// One extracted doc code block, with its source location (for diagnostics).
pub struct DocBlock {
    /// The fenced content (the `.pra` snippet).
    pub code: String,
    /// 1-based line of the `///` doc comment the block came from (in the containing file).
    pub line: usize,
    /// The `// expect: <text>` expected output line, if present (Rust-doc-test style).
    pub expected: Option<String>,
}

/// Extract all ```pra doc code blocks from a parsed program, with real file line numbers.
pub fn extract_blocks(program: &Program, src: &str) -> Vec<DocBlock> {
    let mut out = Vec::new();
    if let Some(docs) = &program.module_docs {
        extract_from_doc(docs, src, &mut out);
    }
    for stmt in &program.stmts {
        extract_from_stmt(stmt, src, &mut out);
    }
    out
}

fn extract_from_stmt(stmt: &Stmt, src: &str, out: &mut Vec<DocBlock>) {
    match stmt {
        Stmt::Pub(inner) => extract_from_stmt(inner, src, out),
        Stmt::Let { docs, .. }
        | Stmt::Const { docs, .. }
        | Stmt::FnDef { docs, .. }
        | Stmt::MathDef { docs, .. } => {
            if let Some(docs) = docs {
                extract_from_doc(docs, src, out);
            }
        }
        Stmt::ClassDef { members, docs, .. } => {
            if let Some(d) = docs {
                extract_from_doc(d, src, out);
            }
            for m in members {
                if let Some(d) = &m.docs {
                    extract_from_doc(d, src, out);
                }
                if let ClassMemberKind::Method { body, .. } = &m.kind
                    && let Some(b) = body
                {
                    for s in &b.stmts {
                        extract_from_stmt(s, src, out);
                    }
                }
            }
        }
        _ => {}
    }
}

fn extract_from_doc(docs: &DocComment, src: &str, out: &mut Vec<DocBlock>) {
    // Compute the 1-based file line of the doc comment's start.
    let start = docs.span.start as usize;
    let base_line = src[..start.min(src.len())]
        .bytes()
        .filter(|&b| b == b'\n')
        .count()
        + 1;
    let text = docs.text();
    let mut lines = text.lines().enumerate().peekable();
    while let Some((idx, line)) = lines.next() {
        let trimmed = line.trim();
        // Fenced opener: ```lang (lang == "pra" or empty).
        if let Some(lang) = trimmed.strip_prefix("```") {
            let lang = lang.trim();
            if lang != "pra" && !lang.is_empty() {
                continue; // not a Prima block (e.g. ```text); skip
            }
            let mut code = String::new();
            let mut expected = None;
            let mut closing = false;
            for (_j, l) in lines.by_ref() {
                if l.trim() == "```" {
                    closing = true;
                    break;
                }
                // A trailing `// expect: <text>` inside the fence is the expected-output marker.
                if let Some(rest) = l.trim_start().strip_prefix("// expect:") {
                    expected = Some(rest.trim().to_string());
                } else {
                    code.push_str(l);
                    code.push('\n');
                }
            }
            if closing {
                out.push(DocBlock {
                    code,
                    line: base_line + idx,
                    expected,
                });
            }
        }
    }
}

/// Outcome of checking one doc block: `Ok(())` on pass, `Err(message)` on failure.
pub type BlockOutcome = Result<(), String>;

/// Statically check every extracted doc block. Returns a list of (block, outcome).
pub fn check_blocks(
    file: &Path,
    source: &str,
    run: bool,
) -> Result<Vec<(DocBlock, BlockOutcome)>, String> {
    let program = parse(source).map_err(|_| format!("cannot parse {file:?}"))?;
    let blocks = extract_blocks(&program, source);
    let mut results = Vec::new();
    for block in blocks {
        let (errors, warnings) = check_src_checked(&block.code);
        let mut fail = Vec::new();
        for e in &errors {
            fail.push(format!("E: {}", e.message));
        }
        for w in &warnings {
            fail.push(format!("warning W: {}", w.code));
        }
        if !fail.is_empty() {
            results.push((block, Err(fail.join("\n"))));
            continue;
        }
        // Optional execution: run the snippet and compare captured output to `expected`.
        if run && let Err(e) = run_block(&block) {
            results.push((block, Err(format!("runtime: {e}"))));
            continue;
        }
        results.push((block, Ok(())));
    }
    Ok(results)
}

/// Execute a doc block with a fresh evaluator, capturing `print`/`println` output.
fn run_block(block: &DocBlock) -> Result<(), String> {
    use std::cell::RefCell;
    use std::rc::Rc;
    let output = Rc::new(RefCell::new(String::new()));
    let sink = output.clone();
    let mut ev = Evaluator::with_sink(move |s| sink.borrow_mut().push_str(&s));
    if let Some(src) = wrap_in_main(block) {
        ev.eval_src(&src).map_err(|e| e.to_string())?;
    }
    if let Some(expected) = &block.expected {
        let got = output.borrow().trim().to_string();
        if got != expected.trim() {
            return Err(format!(
                "output mismatch\n  expected: {expected:?}\n  got:      {:?}",
                got
            ));
        }
    }
    Ok(())
}

/// Wrap a doc snippet in a full `program` form. A bare expression statement is evaluated for effect
/// only (doc snippets usually rely on `print`/`println`, so no implicit return matters). Snippets
/// that already contain statements/`;` are used as-is.
fn wrap_in_main(block: &DocBlock) -> Option<String> {
    let code = block.code.trim();
    if code.is_empty() {
        None
    } else {
        Some(code.to_string())
    }
}

/// Run `prima doc --test` for one file: statically check (and optionally execute) every doc block,
/// reporting each outcome and an all-pass summary. Returns `Ok(SUCCESS)` when every block passes.
pub fn run_doc_tests(file: &Path, source: &str, run: bool) -> anyhow::Result<ExitCode> {
    let results = check_blocks(file, source, run).map_err(|e| anyhow::anyhow!(e))?;
    let mut failed = 0usize;
    let mut passed = 0usize;
    for (block, outcome) in &results {
        match outcome {
            Ok(()) => {
                println!("ok   doc test at line {}", block.line);
                passed += 1;
            }
            Err(msg) => {
                failed += 1;
                eprintln!("FAIL doc test at line {}: {msg}", block.line);
            }
        }
    }
    println!("{passed} passed, {failed} failed");
    if failed > 0 {
        Ok(ExitCode::FAILURE)
    } else {
        Ok(ExitCode::SUCCESS)
    }
}

#[cfg(test)]
mod tests {
    use super::{check_blocks, extract_blocks};
    use std::path::Path;

    fn src() -> &'static str {
        "/// Add two numbers.\n///\n/// ```pra\n/// let x = 1 + 2;\n/// println(x);\n/// // expect: 3\n/// ```\npub fn add(a: Integer, b: Integer) -> Integer { a + b }\n"
    }

    #[test]
    fn extracts_pra_fenced_block() {
        let program = prima_syntax::parse(src()).unwrap();
        let blocks = extract_blocks(&program, src());
        assert_eq!(blocks.len(), 1);
        assert!(blocks[0].code.contains("let x = 1 + 2;"));
        assert_eq!(blocks[0].expected.as_deref(), Some("3"));
    }

    #[test]
    fn ignores_non_pra_fences() {
        let s =
            "/// Some text.\n///\n/// ```text\n/// hello\n/// ```\npub fn f() -> Integer { 0 }\n";
        let program = prima_syntax::parse(s).unwrap();
        assert!(extract_blocks(&program, s).is_empty());
    }

    #[test]
    fn check_ok_returns_pass() {
        let results = check_blocks(Path::new("t.pra"), src(), false).unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].1.is_ok());
    }

    #[test]
    fn check_catches_type_error() {
        let bad = "/// ```pra\n/// let x: Integer = \"s\";\n/// ```\npub fn f() -> Integer { 0 }\n";
        let results = check_blocks(Path::new("t.pra"), bad, false).unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].1.is_err());
    }
}
