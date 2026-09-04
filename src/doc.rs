//! `prima doc` documentation generator (spec §20 / §4.1).
//!
//! Parses a `.pra` file (or the embedded stdlib modules with `--stdlib`) and renders Markdown
//! from the AST's `///`/`//!` doc comments: a `#` module title, the `//!` module doc, and one
//! `##` section per definition (`fn`/`let`/`const`/`class`) with its `///` doc and signature.
//! Class members (fields/methods) are listed with their own `///` docs.
//!
//! Output goes to stdout by default, or to a file with `-o`.

use std::path::Path;
use std::process::ExitCode;

use anyhow::Context;
use prima_syntax::ast::{ClassMemberKind, DocComment, Program, Stmt};
use prima_syntax::parse;

use crate::doctest;
use crate::fmt;
use crate::{diagnostics, read_src};

/// Emit documentation for one file, or for every embedded stdlib module (`stdlib == true`).
/// When `test` is set, also validate `///` doc code blocks (static check; `run` executes them);
/// `doc` mode renders Markdown regardless.
pub fn run(
    path: Option<&Path>,
    output: Option<&Path>,
    stdlib: bool,
    test: bool,
    run: bool,
) -> anyhow::Result<ExitCode> {
    let mut out = String::new();
    if stdlib {
        for (module_path, source) in prima_runtime::stdlib::all_module_sources() {
            out.push_str(&render_module(&module_path, source, &module_path));
        }
    } else {
        let path = match path {
            Some(p) => p,
            None => {
                diagnostics::print_colored_error(
                    "`prima doc` needs a `.pra` file, or `--stdlib` for the built-in modules",
                );
                return Ok(ExitCode::FAILURE);
            }
        };
        let source = read_src(path)?;
        let label = path.to_string_lossy().into_owned();
        out = render_module(&label, &source, &label);
        if test {
            return doctest::run_doc_tests(path, &source, run);
        }
    }

    match output {
        Some(path) => {
            std::fs::write(path, &out)
                .with_context(|| format!("cannot write {}", path.display()))?;
            Ok(ExitCode::SUCCESS)
        }
        None => {
            print!("{out}");
            Ok(ExitCode::SUCCESS)
        }
    }
}

/// Render Markdown for one module body: `#` title, `//!` module doc, then one `##` section per item.
fn render_module(label: &str, source: &str, title: &str) -> String {
    let program = match parse(source) {
        Ok(p) => p,
        Err(errors) => {
            // For embedded stdlib sources the label is synthetic; only report syntax errors for real files.
            if !label.contains("<stdlib>") && label.ends_with(".pra") {
                diagnostics::report_syntax_errors(Path::new(label), source, &errors);
            }
            return String::new();
        }
    };
    render_program(title, &program)
}

/// Render a parsed program's definitions.
fn render_program(title: &str, program: &Program) -> String {
    let mut out = String::new();
    out.push_str(&format!("# Module `{title}`\n\n"));
    if let Some(docs) = &program.module_docs {
        out.push_str(&render_doc(docs));
        out.push('\n');
    }
    for stmt in &program.stmts {
        let (inner, is_pub) = match stmt {
            Stmt::Pub(inner) => (&**inner, true),
            other => (other, false),
        };
        if !is_definition(inner) {
            continue;
        }
        if let Some(docs) = stmt_docs(inner) {
            out.push_str(&render_doc(docs));
            out.push('\n');
        }
        out.push_str(&render_definition(inner, is_pub));
        out.push('\n');
    }
    out
}

/// The top-level statements that `prima doc` lists.
fn is_definition(stmt: &Stmt) -> bool {
    matches!(
        stmt,
        Stmt::FnDef { .. }
            | Stmt::MathDef { .. }
            | Stmt::ClassDef { .. }
            | Stmt::Const { .. }
            | Stmt::Let { .. }
    )
}

/// The `///` doc attached to a definition statement, if any (spec §4.1).
fn stmt_docs(stmt: &Stmt) -> Option<&DocComment> {
    match stmt {
        Stmt::Let { docs, .. }
        | Stmt::Const { docs, .. }
        | Stmt::FnDef { docs, .. }
        | Stmt::MathDef { docs, .. }
        | Stmt::ClassDef { docs, .. } => docs.as_ref(),
        _ => None,
    }
}

/// Render a doc comment as Markdown: paragraphs of the `///` text, with any fenced
/// ```pra code blocks preserved verbatim as Markdown fenced blocks (so they render as code and
/// are the input for `prima doc --test`, spec §20 / doctest). Blank-separated text is one
/// paragraph; a line starting with `# ` / `* ` / `- ` / `[` is passed through as Markdown.
fn render_doc(docs: &DocComment) -> String {
    render_doc_text(&docs.text())
}

/// Render the concatenated `///` text as Markdown, preserving ```lang fenced code blocks verbatim.
fn render_doc_text(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        // A fenced code block opener ``` <lang>: emit it and every subsequent line verbatim
        // until the closing ```.
        if let Some(lang) = line.strip_prefix("```") {
            let lang = lang.trim();
            out.push('\n');
            out.push_str(&format!("```{lang}\n"));
            i += 1;
            while i < lines.len() {
                out.push_str(lines[i]);
                out.push('\n');
                if lines[i].trim() == "```" {
                    i += 1;
                    break;
                }
                i += 1;
            }
            out.push('\n');
            continue;
        }
        // Blank line: paragraph separator.
        if line.trim().is_empty() {
            // Collapse consecutive blanks into one blank line.
            while i < lines.len() && lines[i].trim().is_empty() {
                i += 1;
            }
            out.push('\n');
            continue;
        }
        // A Markdown list item, heading, blockquote, or plain paragraph is passed through as-is.
        let trimmed = line.trim_start();
        out.push_str(&format!("{trimmed}\n"));
        i += 1;
    }
    out
}

fn render_definition(stmt: &Stmt, is_pub: bool) -> String {
    let prefix = if is_pub { "pub " } else { "" };
    let mut out = String::new();
    match stmt {
        Stmt::FnDef {
            name, params, ret, ..
        } => {
            out.push_str("## `");
            out.push_str(prefix);
            out.push_str("fn ");
            out.push_str(&name.value);
            fmt::format_params(params, &mut out);
            fmt::format_ret(ret, &mut out);
            out.push_str("`\n");
        }
        Stmt::MathDef {
            name, params, ret, ..
        } => {
            out.push_str("## `");
            out.push_str(prefix);
            out.push_str("let ");
            out.push_str(&name.value);
            fmt::format_params(params, &mut out);
            fmt::format_ret(ret, &mut out);
            out.push_str("`\n");
        }
        Stmt::Const { name, type_ann, .. } => {
            out.push_str("## `");
            out.push_str(prefix);
            out.push_str("const ");
            out.push_str(&name.value);
            out.push_str(": ");
            fmt::format_type(type_ann, &mut out);
            out.push_str("`\n");
        }
        Stmt::ClassDef { name, members, .. } => {
            out.push_str("## `");
            out.push_str(prefix);
            out.push_str("class ");
            out.push_str(&name.value);
            out.push_str("`\n\n");
            for member in members {
                match &member.kind {
                    ClassMemberKind::Field { name: fname, ty } => {
                        out.push_str("- field `");
                        fmt::format_visibility(member.vis, &mut out);
                        out.push_str(&fname.value);
                        out.push_str(": ");
                        fmt::format_type(ty, &mut out);
                        out.push('`');
                        if let Some(docs) = &member.docs {
                            out.push_str(" — ");
                            out.push_str(&docs.text().replace('\n', " "));
                        }
                        out.push('\n');
                    }
                    ClassMemberKind::Method {
                        name: mname,
                        params,
                        ret,
                        ..
                    } => {
                        out.push_str("- method `");
                        fmt::format_visibility(member.vis, &mut out);
                        out.push_str(&mname.value);
                        fmt::format_params(params, &mut out);
                        fmt::format_ret(ret, &mut out);
                        out.push('`');
                        if let Some(docs) = &member.docs {
                            out.push_str(" — ");
                            out.push_str(&docs.text().replace('\n', " "));
                        }
                        out.push('\n');
                    }
                }
            }
        }
        Stmt::Let { pat, type_ann, .. } => {
            out.push_str("## `");
            out.push_str(prefix);
            out.push_str("let ");
            fmt::format_pattern(pat, &mut out);
            if let Some(t) = type_ann {
                out.push_str(": ");
                fmt::format_type(t, &mut out);
            }
            out.push_str("`\n");
        }
        _ => {}
    }
    out
}
