//! `prima doc` definition listing (spec §20 tool command).
//!
//! Parses a `.pra` file and prints a deterministic, Markdown-ish listing of the
//! top-level definitions: `fn`/`let f(x) = ...` signatures, `const` bindings,
//! and `class` blocks (fields + methods). A `///` doc comment immediately
//! preceding a definition in the source is included as its doc paragraph.

use std::path::Path;
use std::process::ExitCode;

use prima_syntax::ast::{ClassMemberKind, Stmt};
use prima_syntax::parse;

use crate::fmt;
use crate::{diagnostics, read_src};

/// Emit the definition listing for `path`.
pub fn run(path: &Path) -> ExitCode {
    let source = match read_src(path) {
        Ok(s) => s,
        Err(code) => return code,
    };
    let program = match parse(&source) {
        Ok(p) => p,
        Err(errors) => {
            diagnostics::report_syntax_errors(path, &source, &errors);
            return ExitCode::FAILURE;
        }
    };

    for stmt in &program.stmts {
        let (inner, is_pub) = match stmt {
            Stmt::Pub(inner) => (&**inner, true),
            other => (other, false),
        };
        if !is_definition(inner) {
            continue;
        }
        if let Some(doc) = doc_comment(&source, stmt_start(inner)) {
            println!("{doc}");
        }
        println!("{}", render_definition(inner, is_pub).trim_end());
    }
    ExitCode::SUCCESS
}

/// The top-level statements that `prima doc` lists.
fn is_definition(stmt: &Stmt) -> bool {
    matches!(
        stmt,
        Stmt::FnDef { .. } | Stmt::MathDef { .. } | Stmt::ClassDef { .. } | Stmt::Const { .. } | Stmt::Let { .. }
    )
}

fn stmt_start(stmt: &Stmt) -> u32 {
    match stmt {
        Stmt::Let { span, .. }
        | Stmt::Const { span, .. }
        | Stmt::FnDef { span, .. }
        | Stmt::MathDef { span, .. }
        | Stmt::ClassDef { span, .. } => span.start,
        _ => 0,
    }
}

/// Collect consecutive `///` comment lines directly above the definition at byte
/// offset `start`, returning their concatenated text with the `///` prefix stripped.
fn doc_comment(source: &str, start: u32) -> Option<String> {
    let start = start as usize;
    let line_start = source[..start].rfind('\n').map(|i| i + 1).unwrap_or(0);
    let def_line = source[..line_start].bytes().filter(|&b| b == b'\n').count();
    let lines: Vec<&str> = source.lines().collect();

    let mut docs: Vec<String> = Vec::new();
    let mut i = def_line;
    while i > 0 {
        let prev = lines[i - 1].trim_start();
        let Some(text) = prev.strip_prefix("///") else { break };
        // `///` itself, plus one optional separating space, is stripped.
        docs.push(text.strip_prefix(' ').unwrap_or(text).to_string());
        i -= 1;
    }
    if docs.is_empty() {
        None
    } else {
        docs.reverse();
        Some(docs.join("\n"))
    }
}

fn render_definition(stmt: &Stmt, is_pub: bool) -> String {
    let prefix = if is_pub { "pub " } else { "" };
    let mut out = String::new();
    match stmt {
        Stmt::FnDef { name, params, ret, .. } => {
            out.push_str("## ");
            out.push_str(prefix);
            out.push_str("fn ");
            out.push_str(&name.value);
            fmt::format_params(params, &mut out);
            fmt::format_ret(ret, &mut out);
        }
        Stmt::MathDef { name, params, ret, .. } => {
            out.push_str("## ");
            out.push_str(prefix);
            out.push_str("let ");
            out.push_str(&name.value);
            fmt::format_params(params, &mut out);
            fmt::format_ret(ret, &mut out);
        }
        Stmt::Const { name, type_ann, .. } => {
            out.push_str("## ");
            out.push_str(prefix);
            out.push_str("const ");
            out.push_str(&name.value);
            out.push_str(": ");
            fmt::format_type(type_ann, &mut out);
        }
        Stmt::ClassDef { name, members, .. } => {
            out.push_str("## ");
            out.push_str(prefix);
            out.push_str("class ");
            out.push_str(&name.value);
            out.push('\n');
            for member in members {
                match &member.kind {
                    ClassMemberKind::Field { name: fname, ty } => {
                        out.push_str("- field ");
                        fmt::format_visibility(member.vis, &mut out);
                        out.push_str(&fname.value);
                        out.push_str(": ");
                        fmt::format_type(ty, &mut out);
                        out.push('\n');
                    }
                    ClassMemberKind::Method { name: mname, params, ret, .. } => {
                        out.push_str("- method ");
                        fmt::format_visibility(member.vis, &mut out);
                        out.push_str(&mname.value);
                        fmt::format_params(params, &mut out);
                        fmt::format_ret(ret, &mut out);
                        out.push('\n');
                    }
                }
            }
        }
        Stmt::Let { pat, type_ann, .. } => {
            out.push_str("## ");
            out.push_str(prefix);
            out.push_str("let ");
            fmt::format_pattern(pat, &mut out);
            if let Some(t) = type_ann {
                out.push_str(": ");
                fmt::format_type(t, &mut out);
            }
        }
        _ => {}
    }
    out
}
