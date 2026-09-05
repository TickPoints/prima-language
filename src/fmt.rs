//! `prima fmt` AST→source pretty-printer (spec §20 tool command).
//!
//! The printer walks the parsed `Program` and emits canonical source text:
//! `;`-separated statements (spec §4.2), 4-space block indentation, spaced
//! binary operators, tight unary/postfix operators, and re-escaped string
//! literals. Parentheses are re-inserted from operator precedence when the AST
//! would otherwise re-parse to a different tree (the Pratt table from the
//! parser, spec appendix A / implementation plan §2.2).
//!
//! The rendering is split by AST-node family into the submodules below; this
//! root module declares them and re-exports the helpers shared with `prima doc`
//! (`format_params`/`format_ret`/`format_type`/`format_visibility`/`format_pattern`).

mod config;
mod expr;
mod import;
mod pattern;
mod stmt;
mod text;
mod ty;

use std::path::Path;
use std::process::ExitCode;

use anyhow::Context;

use prima_syntax::ast::Program;
use prima_syntax::parse;

use crate::diagnostics;

// Helpers rendered by `prima doc` (spec §20) via `crate::fmt::` (see `doc.rs`).
pub(crate) use pattern::format_pattern;
pub(crate) use stmt::format_visibility;
pub(crate) use ty::{format_params, format_ret, format_type};

use config::format_config_block;
use import::format_import;
use stmt::format_stmt;
use text::format_doc_lines;

/// CLI entry for `prima fmt` (spec §20): parse, format, and either write back
/// (`--write`), verify formatting (`--check`), or print to stdout.
pub fn run(path: &Path, write: bool, check: bool) -> anyhow::Result<ExitCode> {
    let source =
        std::fs::read_to_string(path).with_context(|| format!("cannot read {}", path.display()))?;
    let program = match parse(&source) {
        Ok(p) => p,
        Err(errors) => {
            diagnostics::report_syntax_errors(path, &source, &errors);
            return Ok(ExitCode::FAILURE);
        }
    };
    let formatted = format_program(&program);

    if check {
        if formatted == source {
            return Ok(ExitCode::SUCCESS);
        }
        diagnostics::print_colored_error(&format!(
            "{} is not formatted (spec §20 `fmt --check`)",
            path.display()
        ));
        return Ok(ExitCode::FAILURE);
    }
    if write {
        if formatted != source {
            std::fs::write(path, &formatted)
                .with_context(|| format!("cannot write {}", path.display()))?;
        }
        return Ok(ExitCode::SUCCESS);
    }
    // Plain mode prints even when the output is identical (idempotent formatting).
    print!("{formatted}");
    Ok(ExitCode::SUCCESS)
}

/// Render a parsed program as canonical source text (spec §20).
///
/// Layout: the `config {}` block first, then imports, then statements (spec §4.1);
/// each statement is `;`-terminated where the grammar requires it (spec §4.2).
pub fn format_program(program: &Program) -> String {
    let mut out = String::new();
    if let Some(docs) = &program.module_docs {
        format_doc_lines(docs, 0, &mut out, true);
        out.push('\n');
    }
    if let Some(cfg) = &program.config {
        format_config_block(cfg, &mut out);
        out.push_str("\n\n");
    }
    for imp in &program.imports {
        if let Some(docs) = &imp.docs {
            format_doc_lines(docs, 0, &mut out, false);
        }
        format_import(imp, &mut out);
        out.push('\n');
    }
    if !program.imports.is_empty() {
        out.push('\n');
    }
    for stmt in &program.stmts {
        format_stmt(stmt, 0, &mut out);
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use prima_syntax::parse;

    /// Formatting must be stable across rounds: a formatted program re-parses and formats
    /// back to identical text (spec §20 idempotence).
    fn assert_idempotent(src: &str) {
        let program = parse(src).expect("parse failed");
        let once = format_program(&program);
        let second = parse(&once).expect("formatted output failed to re-parse");
        let twice = format_program(&second);
        assert_eq!(once, twice, "fmt not idempotent for {src:?}");
    }

    /// The canonical rendering of `src`, for explicit formatting assertions.
    fn fmt_of(src: &str) -> String {
        format_program(&parse(src).expect("parse failed"))
    }

    #[test]
    fn fmt_basic_let_is_spaced_and_semicolon_separated() {
        assert_idempotent("let a=1;let b=2;");
        let out = fmt_of("let a=1;let b=2;");
        assert!(out.contains("let a = 1;\nlet b = 2;"), "got {out:?}");
    }

    #[test]
    fn fmt_preserves_required_parens() {
        assert_idempotent("let x = (a + b) * c;");
        assert_eq!(fmt_of("let x = (a + b) * c;"), "let x = (a + b) * c;\n");
        assert_eq!(fmt_of("let y = a * (b + c);"), "let y = a * (b + c);\n");
        // `^` is right-associative (spec §4.3): a same-precedence left operand needs parens.
        assert_eq!(fmt_of("let z = (x^y)^2;"), "let z = (x^y)^2;\n");
        assert_eq!(fmt_of("let w = (-x)^2;"), "let w = (-x)^2;\n");
        assert_idempotent("let a = (b + c);");
    }

    #[test]
    fn fmt_unary_pow_binding() {
        // `-x^2` is `-(x^2)` (spec §4.3); the printer must not re-parenthesize it.
        assert_idempotent("let z = -x^2;");
        assert_eq!(fmt_of("let z = -x^2;"), "let z = -x^2;\n");
    }

    #[test]
    fn fmt_covers_control_flow_and_functions() {
        assert_idempotent(
            "fn f(a: Integer) -> Integer {\nif a > 0 {\nreturn a * 2;\n} else {\nreturn 0;\n}\n}\nlet r = f(3);",
        );
    }

    #[test]
    fn fmt_covers_classes() {
        assert_idempotent(
            "class Counter {\n    pub count: Integer,\n    pub fn new(start: Integer) -> Self { Counter { count: start } }\n    pub fn value(self) -> Integer { self.count }\n}\nlet c = Counter::new(1);\nc.value();",
        );
    }

    #[test]
    fn fmt_covers_collections_and_comprehensions() {
        assert_idempotent("let d = { \"a\": 1, \"b\": 2 };");
        assert_idempotent("let s = {1, 2, 3, 2};");
        assert_idempotent("let sq = [x^2 for x in range(0, 10) if x % 2 == 0];");
        assert_idempotent("let t = {k: k^2 for k in range(0, 5)};");
        assert_idempotent("let o = {x for x in range(0, 4)};");
        assert_idempotent("let g = ((x, x + 1) for x in range(0, 3));");
    }

    #[test]
    fn fmt_covers_patterns_and_match() {
        assert_idempotent("let (a, b) = (1, 2);");
        assert_idempotent("if let Some(x) = v.get(0) { println(x); } else { println(\"none\"); }");
        assert_idempotent(
            "let r = match n {\n0 => \"zero\",\n1 | 2 => \"small\",\nm if m > 100 => \"large\",\n_ => \"other\",\n};",
        );
    }

    #[test]
    fn fmt_covers_strings_and_tex() {
        assert_idempotent("let a = tex\"\\sqrt{2} + \\pi\";");
        assert_idempotent("let s = \"a\\nb\\t\\\"q\\'\";");
        assert_idempotent("let t = \"\\u{1F600} smile\";");
    }

    #[test]
    fn fmt_covers_fstrings_and_string_forms() {
        // f-strings re-emit in escaped canonical form (spec §18.1), idempotently.
        assert_idempotent(r#"let s = f"a = {x} b = {y + 1:0.2}";"#);
        assert_idempotent(r#"let s = f"{{literal}} {a}";"#);
        assert_idempotent(r#"let s = f"d = { {"a": 1}["a"] }";"#);
        // `r"..."` raw and `'...'` single-quoted forms are preserved when lossless.
        assert_eq!(fmt_of(r#"let s = r"a\nb";"#), "let s = r\"a\\nb\";\n");
        assert_eq!(fmt_of("let s = 'ab';"), "let s = 'ab';\n");
        assert_eq!(fmt_of("let c = 'a';"), "let c = 'a';\n");
        // A raw string containing the delimiter falls back to the escaped form.
        assert_idempotent("let s = \"a\\\"b\";");
    }

    #[test]
    fn fmt_covers_config_and_imports() {
        assert_idempotent("config { fraction := false }\nlet x = 1/3;");
        assert_idempotent("import mymath;\nprintln(mymath::square(3));");
        assert_idempotent("import a::b as c;\nfrom x import y as z, w;\nlet q = 1;");
    }

    #[test]
    fn fmt_covers_pub_definitions() {
        assert_idempotent("pub let square(x) = x^2;\nlet helper(x) = x + 1;");
        assert_eq!(
            fmt_of("pub let square(x) = x^2;"),
            "pub let square(x) = x^2;\n"
        );
        assert_idempotent("pub fn f(a: Integer) -> Integer { a }\npub class C { pub x: Integer }");
    }

    #[test]
    fn fmt_covers_custom_and_with_config() {
        assert_idempotent("config { undefined_handling := custom { 0/0 := 1 } }\nlet q = 1;");
        assert_idempotent("with config { domain := complex } {\nlet z = (-1)^0.5;\n}");
    }

    #[test]
    fn fmt_covers_loop_and_indexing() {
        assert_idempotent("for i in 0..10 step 2 { println(i); }");
        assert_idempotent("let m = M[.., 1];");
        assert_idempotent("let v = a[1..3];");
    }

    #[test]
    fn fmt_covers_lambdas_and_methods() {
        assert_idempotent("let f = |a, b| a + b;");
        assert_idempotent("let y = obj.method(1, 2).field;");
    }
}
