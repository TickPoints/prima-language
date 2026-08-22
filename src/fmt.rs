//! `prima fmt` AST→source pretty-printer (spec §20 tool command).
//!
//! The printer walks the parsed `Program` and emits canonical source text:
//! `;`-separated statements (spec §4.2), 4-space block indentation, spaced
//! binary operators, tight unary/postfix operators, and re-escaped string
//! literals. Parentheses are re-inserted from operator precedence when the AST
//! would otherwise re-parse to a different tree (the Pratt table from the
//! parser, spec appendix A / implementation plan §2.2).

use std::path::Path;
use std::process::ExitCode;

use prima_syntax::ast::{
    Annotation, AssignOp, BinOp, Block, ClassMember, ClassMemberKind, CompKind,
    ComprehensionClause, ConfigBlock, ConfigEntry, Expr, ExprKind, FStringPart, Import, ImportItem,
    ImportKind, ImplOp, Index, IndexItem, Literal, MatchArm, Param, Pattern, Program, Stmt,
    StringQuote, Type, UnOp, Visibility,
};
use prima_syntax::parse;

use crate::diagnostics;

/// CLI entry for `prima fmt` (spec §20): parse, format, and either write back
/// (`--write`), verify formatting (`--check`), or print to stdout.
pub fn run(path: &Path, write: bool, check: bool) -> ExitCode {
    let source = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            diagnostics::print_colored_error(&format!("cannot read {}: {e}", path.display()));
            return ExitCode::FAILURE;
        }
    };
    let program = match parse(&source) {
        Ok(p) => p,
        Err(errors) => {
            diagnostics::report_syntax_errors(path, &source, &errors);
            return ExitCode::FAILURE;
        }
    };
    let formatted = format_program(&program);

    if check {
        if formatted == source {
            return ExitCode::SUCCESS;
        }
        diagnostics::print_colored_error(&format!("{} is not formatted (spec §20 `fmt --check`)", path.display()));
        return ExitCode::FAILURE;
    }
    if write {
        if formatted != source
            && let Err(e) = std::fs::write(path, &formatted)
        {
            diagnostics::print_colored_error(&format!("cannot write {}: {e}", path.display()));
            return ExitCode::FAILURE;
        }
        return ExitCode::SUCCESS;
    }
    // Plain mode prints even when the output is identical (idempotent formatting).
    print!("{formatted}");
    ExitCode::SUCCESS
}

/// Unary binding power from the parser (`UNARY_BP`, spec §4.3): unary binds looser than `^`.
const UNARY_BP: u8 = 7;
/// Precedence of atoms and postfix chains (calls, index, field, method, literals): tighter than any operator.
const ATOM_BP: u8 = 100;

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

/// Emit the doc lines of a `///`/`//!` comment (spec §4.1) as `///` comment lines at `indent`.
/// Leaves the output at the start of a new line (no trailing indent).
fn format_doc_lines(docs: &prima_syntax::ast::DocComment, indent: usize, out: &mut String, module: bool) {
    let marker = if module { "//! " } else { "/// " };
    for (line, _) in &docs.lines {
        push_indent(indent, out);
        out.push_str(marker);
        out.push_str(line);
        out.push('\n');
    }
}

/// The `///` doc attached to a definition statement, recursing through `pub` (spec §4.1).
fn stmt_docs(stmt: &Stmt) -> Option<&prima_syntax::ast::DocComment> {
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

fn push_indent(indent: usize, out: &mut String) {
    for _ in 0..indent {
        out.push_str("    ");
    }
}

fn format_config_block(cfg: &ConfigBlock, out: &mut String) {
    out.push_str("config { ");
    for (i, entry) in cfg.entries.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        format_config_entry(entry, out);
    }
    out.push_str(" }");
}

fn format_config_entry(entry: &ConfigEntry, out: &mut String) {
    out.push_str(&entry.name.value);
    match &entry.type_ann {
        // `fraction: bool = true` (appendix BNF) when annotated, `fraction := true` otherwise (spec §13.1).
        Some(t) => {
            out.push_str(": ");
            format_type(t, out);
            out.push_str(" = ");
        }
        None => out.push_str(" := "),
    }
    format_expr(&entry.value, 0, out);
}

fn format_import(imp: &Import, out: &mut String) {
    match &imp.kind {
        ImportKind::Namespace { path, alias } => {
            out.push_str("import ");
            format_path(path, out);
            if let Some(a) = alias {
                out.push_str(" as ");
                out.push_str(&a.value);
            }
        }
        ImportKind::From { path, items } => {
            out.push_str("from ");
            format_path(path, out);
            out.push_str(" import ");
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                match item {
                    ImportItem::Star => out.push('*'),
                    ImportItem::Name { name, alias } => {
                        out.push_str(&name.value);
                        if let Some(a) = alias {
                            out.push_str(" as ");
                            out.push_str(&a.value);
                        }
                    }
                }
            }
        }
    }
    out.push(';');
}

fn format_path(segments: &[prima_syntax::ast::Spanned<String>], out: &mut String) {
    for (i, seg) in segments.iter().enumerate() {
        if i > 0 {
            out.push_str("::");
        }
        out.push_str(&seg.value);
    }
}

/// Block-level statements may omit the trailing `;` (spec §4.2); the rest require it.
fn needs_semicolon(stmt: &Stmt) -> bool {
    match stmt {
        Stmt::FnDef { .. }
        | Stmt::ClassDef { .. }
        | Stmt::Impl { .. }
        | Stmt::For { .. }
        | Stmt::ParFor { .. }
        | Stmt::While { .. }
        | Stmt::If { .. }
        | Stmt::IfLet { .. }
        | Stmt::WhileLet { .. }
        | Stmt::Match { .. }
        | Stmt::WithConfig { .. } => false,
        Stmt::Pub(inner) => needs_semicolon(inner),
        _ => true,
    }
}

fn format_stmt(stmt: &Stmt, indent: usize, out: &mut String) {
    // Re-emit the `///` doc comment above the statement it documents (spec §4.1); `prima fmt`
    // preserves doc comments as part of the AST.
    if let Some(docs) = stmt_docs(stmt) {
        format_doc_lines(docs, indent, out, false);
    }
    push_indent(indent, out);
    // Unwrap `pub` so the inner statement renders with the visibility prefix and the
    // `;` rule follows the inner statement kind (spec §15.2).
    let (is_pub, inner) = match stmt {
        Stmt::Pub(inner) => (true, &**inner),
        other => (false, other),
    };
    match inner {
        Stmt::Let { pat, mut_, type_ann, value, .. } => {
            if is_pub {
                out.push_str("pub ");
            }
            out.push_str("let ");
            if *mut_ {
                out.push_str("mut ");
            }
            format_pattern(pat, out);
            if let Some(t) = type_ann {
                out.push_str(": ");
                format_type(t, out);
            }
            out.push_str(" = ");
            format_expr(value, 0, out);
        }
        Stmt::Const { name, type_ann, value, .. } => {
            if is_pub {
                out.push_str("pub ");
            }
            out.push_str("const ");
            out.push_str(&name.value);
            out.push_str(": ");
            format_type(type_ann, out);
            out.push_str(" = ");
            format_expr(value, 0, out);
        }
        Stmt::FnDef { name, params, ret, annotations, body, .. } => {
            if is_pub {
                out.push_str("pub ");
            }
            format_annotations(annotations, out);
            out.push_str("fn ");
            out.push_str(&name.value);
            format_params(params, out);
            format_ret(ret, out);
            out.push(' ');
            format_block(body, indent, out);
        }
        Stmt::MathDef { name, params, ret, annotations, body, .. } => {
            if is_pub {
                out.push_str("pub ");
            }
            out.push_str("let ");
            out.push_str(&name.value);
            format_params(params, out);
            format_ret(ret, out);
            format_annotations(annotations, out);
            out.push_str(" = ");
            format_expr(body, 0, out);
        }
        Stmt::ClassDef { name, annotations, members, .. } => {
            if is_pub {
                out.push_str("pub ");
            }
            format_annotations(annotations, out);
            out.push_str("class ");
            out.push_str(&name.value);
            out.push_str(" {\n");
            for member in members {
                format_class_member(member, indent + 1, out);
            }
            push_indent(indent, out);
            out.push('}');
        }
        Stmt::Impl { op, target, members, .. } => {
            if is_pub {
                out.push_str("pub ");
            }
            out.push_str("impl ops::");
            out.push_str(impl_op_name(*op));
            out.push_str(" for ");
            out.push_str(&target.value);
            out.push_str(" {\n");
            for member in members {
                format_stmt(member, indent + 1, out);
                out.push('\n');
            }
            push_indent(indent, out);
            out.push('}');
        }
        Stmt::Assign { target, op, value, .. } => {
            format_expr(target, ATOM_BP, out);
            out.push(' ');
            out.push_str(assign_op_name(*op));
            out.push(' ');
            format_expr(value, 0, out);
        }
        Stmt::Expr(e) => format_expr(e, 0, out),
        Stmt::For { var, range, step, body, .. } => {
            out.push_str("for ");
            out.push_str(&var.value);
            out.push_str(" in ");
            format_expr(&range.0, 0, out);
            out.push_str("..");
            format_expr(&range.1, 0, out);
            format_step(step, out);
            out.push(' ');
            format_block(body, indent, out);
        }
        Stmt::ParFor { var, range, step, body, .. } => {
            out.push_str("parfor ");
            out.push_str(&var.value);
            out.push_str(" in ");
            format_expr(&range.0, 0, out);
            out.push_str("..");
            format_expr(&range.1, 0, out);
            format_step(step, out);
            out.push(' ');
            format_block(body, indent, out);
        }
        Stmt::While { cond, body, .. } => {
            out.push_str("while ");
            format_expr(cond, 0, out);
            out.push(' ');
            format_block(body, indent, out);
        }
        Stmt::If { cond, then, elifs, else_, .. } => {
            out.push_str("if ");
            format_expr(cond, 0, out);
            out.push(' ');
            format_block(then, indent, out);
            for (cond, body) in elifs {
                out.push_str(" else if ");
                format_expr(cond, 0, out);
                out.push(' ');
                format_block(body, indent, out);
            }
            if let Some(body) = else_ {
                out.push_str(" else ");
                format_block(body, indent, out);
            }
        }
        Stmt::IfLet { pat, value, then, else_, .. } => {
            out.push_str("if let ");
            format_pattern(pat, out);
            out.push_str(" = ");
            format_expr(value, 0, out);
            out.push(' ');
            format_block(then, indent, out);
            if let Some(body) = else_ {
                out.push_str(" else ");
                format_block(body, indent, out);
            }
        }
        Stmt::WhileLet { pat, value, body, .. } => {
            out.push_str("while let ");
            format_pattern(pat, out);
            out.push_str(" = ");
            format_expr(value, 0, out);
            out.push(' ');
            format_block(body, indent, out);
        }
        Stmt::Match { scrutinee, arms, .. } => {
            out.push_str("match ");
            format_expr(scrutinee, 0, out);
            out.push_str(" {\n");
            for arm in arms {
                format_match_arm(arm, indent + 1, out);
            }
            push_indent(indent, out);
            out.push('}');
        }
        Stmt::Return { value, .. } => {
            out.push_str("return");
            if let Some(v) = value {
                out.push(' ');
                format_expr(v, 0, out);
            }
        }
        Stmt::WithConfig { entries, body, .. } => {
            out.push_str("with config { ");
            for (i, entry) in entries.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                format_config_entry(entry, out);
            }
            out.push_str(" } ");
            format_block(body, indent, out);
        }
        Stmt::Pub(_) => unreachable!("`pub` is unwrapped at the top of `format_stmt`"),
    }
    if needs_semicolon(inner) {
        out.push(';');
    }
}

pub(crate) fn format_ret(ret: &Option<Type>, out: &mut String) {
    if let Some(t) = ret {
        out.push_str(" -> ");
        format_type(t, out);
    }
}

fn format_step(step: &Option<Expr>, out: &mut String) {
    if let Some(s) = step {
        out.push_str(" step ");
        format_expr(s, 0, out);
    }
}

fn format_annotations(annotations: &[Annotation], out: &mut String) {
    for a in annotations {
        match a {
            Annotation::Parallel => out.push_str("@parallel "),
            Annotation::Jit => out.push_str("@jit "),
            Annotation::Gpu => out.push_str("@gpu "),
            Annotation::Builtin => out.push_str("@builtin "),
            Annotation::CApiExtern => out.push_str("@c_api::extern "),
        }
    }
}

fn format_class_member(member: &ClassMember, indent: usize, out: &mut String) {
    if let Some(docs) = &member.docs {
        format_doc_lines(docs, indent, out, false);
    }
    push_indent(indent, out);
    format_visibility(member.vis, out);
    match &member.kind {
        ClassMemberKind::Field { name, ty } => {
            out.push_str(&name.value);
            out.push_str(": ");
            format_type(ty, out);
            // Fields are comma-terminated in the class body (spec appendix A `field_decl`).
            out.push_str(",\n");
        }
        ClassMemberKind::Method { name, params, ret, annotations, body } => {
            format_annotations(annotations, out);
            out.push_str("fn ");
            out.push_str(&name.value);
            format_params(params, out);
            format_ret(ret, out);
            match body {
                Some(b) => {
                    out.push(' ');
                    format_block(b, indent, out);
                }
                // Signature-only method (`@builtin` class, spec §18.4).
                None => out.push(';'),
            }
            out.push('\n');
        }
    }
}

pub(crate) fn format_visibility(vis: Visibility, out: &mut String) {
    match vis {
        Visibility::Private => {}
        Visibility::Module => out.push_str("pub(mod) "),
        Visibility::Public => out.push_str("pub "),
    }
}

fn impl_op_name(op: ImplOp) -> &'static str {
    match op {
        ImplOp::Add => "Add",
        ImplOp::Sub => "Sub",
        ImplOp::Mul => "Mul",
        ImplOp::Div => "Div",
        ImplOp::Rem => "Rem",
        ImplOp::Neg => "Neg",
        ImplOp::Eq => "Eq",
        ImplOp::Cmp => "Cmp",
        ImplOp::Index => "Index",
    }
}

fn assign_op_name(op: AssignOp) -> &'static str {
    match op {
        AssignOp::Assign => "=",
        AssignOp::AddAssign => "+=",
        AssignOp::SubAssign => "-=",
    }
}

fn format_block(block: &Block, indent: usize, out: &mut String) {
    out.push_str("{\n");
    for stmt in &block.stmts {
        format_stmt(stmt, indent + 1, out);
        out.push('\n');
    }
    push_indent(indent, out);
    out.push('}');
}

pub(crate) fn format_params(params: &[Param], out: &mut String) {
    out.push('(');
    format_params_bare(params, out);
    out.push(')');
}

/// Param list without the enclosing parens (used by lambdas, spec §4.6).
fn format_params_bare(params: &[Param], out: &mut String) {
    for (i, p) in params.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        if p.is_self {
            out.push_str("self");
        } else {
            out.push_str(&p.name.value);
        }
        if let Some(t) = &p.type_ann {
            out.push_str(": ");
            format_type(t, out);
        }
    }
}

fn format_match_arm(arm: &MatchArm, indent: usize, out: &mut String) {
    push_indent(indent, out);
    format_pattern(&arm.pattern, out);
    if let Some(guard) = &arm.guard {
        out.push_str(" if ");
        format_expr(guard, 0, out);
    }
    out.push_str(" => ");
    format_expr(&arm.body, 0, out);
    // The parser accepts a trailing `,`/`;` per arm; the canonical form is `,` (spec appendix A `match_arm`).
    out.push_str(",\n");
}

pub(crate) fn format_pattern(pat: &Pattern, out: &mut String) {
    match pat {
        Pattern::Wildcard(_) => out.push('_'),
        Pattern::Binding(name) => out.push_str(&name.value),
        Pattern::Literal(lit) => format_literal(lit, out),
        Pattern::Tuple(pats, rest) => {
            out.push('(');
            format_pattern_list(pats, rest, out);
            out.push(')');
        }
        Pattern::Array(pats, rest) => {
            out.push('[');
            format_pattern_list(pats, rest, out);
            out.push(']');
        }
        Pattern::Struct { name, fields, rest } => {
            out.push_str(&name.value);
            out.push_str(" { ");
            for (i, f) in fields.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                out.push_str(&f.name.value);
                if let Some(p) = &f.pat {
                    out.push_str(": ");
                    format_pattern(p, out);
                }
            }
            if *rest {
                if !fields.is_empty() {
                    out.push_str(", ");
                }
                out.push_str("..");
            }
            out.push_str(" }");
        }
        Pattern::Variant { name, args, .. } => {
            out.push_str(&name.value);
            if !args.is_empty() {
                out.push('(');
                for (i, a) in args.iter().enumerate() {
                    if i > 0 {
                        out.push_str(", ");
                    }
                    format_pattern(a, out);
                }
                out.push(')');
            }
        }
        Pattern::Range { lo, hi, inclusive } => {
            format_literal(lo, out);
            if *inclusive {
                out.push_str("..=");
            } else {
                out.push_str("..");
            }
            format_literal(hi, out);
        }
        Pattern::Or(pats) => {
            for (i, p) in pats.iter().enumerate() {
                if i > 0 {
                    out.push_str(" | ");
                }
                format_pattern(p, out);
            }
        }
        Pattern::Group(p) => {
            out.push('(');
            format_pattern(p, out);
            out.push(')');
        }
    }
}

fn format_pattern_list(pats: &[Pattern], rest: &bool, out: &mut String) {
    for (i, p) in pats.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        format_pattern(p, out);
    }
    if *rest {
        if !pats.is_empty() {
            out.push_str(", ");
        }
        out.push_str("..");
    }
}

pub(crate) fn format_type(ty: &Type, out: &mut String) {
    match ty {
        Type::Number => out.push_str("Number"),
        Type::Integer => out.push_str("Integer"),
        Type::Rational => out.push_str("Rational"),
        Type::F64 => out.push_str("F64"),
        Type::F32 => out.push_str("F32"),
        Type::I8 => out.push_str("I8"),
        Type::I16 => out.push_str("I16"),
        Type::I32 => out.push_str("I32"),
        Type::I64 => out.push_str("I64"),
        Type::I128 => out.push_str("I128"),
        Type::U8 => out.push_str("U8"),
        Type::U16 => out.push_str("U16"),
        Type::U32 => out.push_str("U32"),
        Type::U64 => out.push_str("U64"),
        Type::U128 => out.push_str("U128"),
        Type::Isize => out.push_str("Isize"),
        Type::Usize => out.push_str("Usize"),
        Type::Complex => out.push_str("Complex"),
        Type::Expr => out.push_str("Expr"),
        Type::Symbol => out.push_str("Symbol"),
        Type::Bool => out.push_str("Bool"),
        Type::String => out.push_str("String"),
        Type::Char => out.push_str("Char"),
        Type::Array(t) => {
            out.push_str("Array<");
            format_type(t, out);
            out.push('>');
        }
        Type::Matrix(t) => {
            out.push_str("Matrix<");
            format_type(t, out);
            out.push('>');
        }
        Type::Tuple(ts) => {
            out.push_str("Tuple<");
            for (i, t) in ts.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                format_type(t, out);
            }
            out.push('>');
        }
        Type::Option(t) => {
            out.push_str("Option<");
            format_type(t, out);
            out.push('>');
        }
        Type::Result(a, b) => {
            out.push_str("Result<");
            format_type(a, out);
            out.push_str(", ");
            format_type(b, out);
            out.push('>');
        }
        Type::Fn { params, ret } => {
            out.push_str("Fn(");
            format_type_list(params, out);
            out.push_str(") -> ");
            format_type(ret, out);
        }
        Type::MFn { params, ret } => {
            out.push_str("MFn(");
            format_type_list(params, out);
            out.push_str(") -> ");
            format_type(ret, out);
        }
        Type::SelfType => out.push_str("Self"),
        Type::User(name) => out.push_str(&name.value),
    }
}

fn format_type_list(types: &[Type], out: &mut String) {
    for (i, t) in types.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        format_type(t, out);
    }
}

/// Binding power (lbp) of a binary operator when embedded as a subexpression (parser `binop_bp`).
fn binop_lbp(op: BinOp) -> u8 {
    match op {
        BinOp::Or => 2,
        BinOp::And => 3,
        BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge | BinOp::In => 4,
        BinOp::Add | BinOp::Sub | BinOp::Union | BinOp::Difference => 5,
        BinOp::Mul | BinOp::Div | BinOp::Mod | BinOp::MatMul | BinOp::Broadcast | BinOp::Intersect => 6,
        BinOp::Pow => 8,
    }
}

/// Right binding power (rbp) of a binary operator for its right operand (parser `binop_bp`).
fn binop_rbp(op: BinOp) -> u8 {
    match op {
        BinOp::Or => 3,
        BinOp::And => 4,
        BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge | BinOp::In => 5,
        BinOp::Add | BinOp::Sub | BinOp::Union | BinOp::Difference => 6,
        BinOp::Mul | BinOp::Div | BinOp::Mod | BinOp::MatMul | BinOp::Broadcast | BinOp::Intersect => 7,
        BinOp::Pow => 8,
    }
}

fn binop_sym(op: BinOp) -> &'static str {
    match op {
        BinOp::Add => "+",
        BinOp::Sub => "-",
        BinOp::Mul => "*",
        BinOp::Div => "/",
        BinOp::Pow => "^",
        BinOp::MatMul => "@",
        BinOp::Mod => "%",
        BinOp::Eq => "==",
        BinOp::Ne => "!=",
        BinOp::Lt => "<",
        BinOp::Le => "<=",
        BinOp::Gt => ">",
        BinOp::Ge => ">=",
        BinOp::And => "&&",
        BinOp::Or => "||",
        BinOp::In => "in",
        BinOp::Union => "∪",
        BinOp::Intersect => "∩",
        BinOp::Difference => "\\",
        BinOp::Broadcast => "@.",
    }
}

fn unary_sym(op: UnOp) -> &'static str {
    match op {
        UnOp::Neg => "-",
        UnOp::Not => "!",
        UnOp::Pos => "+",
    }
}

/// Precedence at which an expression binds when embedded as an operand (for paren re-insertion).
fn root_prec(e: &Expr) -> u8 {
    match &e.kind {
        ExprKind::Binary { op, .. } => binop_lbp(*op),
        ExprKind::Unary { .. } => UNARY_BP,
        _ => ATOM_BP,
    }
}

fn format_expr(e: &Expr, min_bp: u8, out: &mut String) {
    if root_prec(e) < min_bp {
        out.push('(');
        format_expr(e, 0, out);
        out.push(')');
        return;
    }
    match &e.kind {
        ExprKind::Literal(lit) => format_literal(lit, out),
        ExprKind::FString(parts) => format_fstring(parts, out),
        ExprKind::Symbol(name) => out.push_str(&name.value),
        ExprKind::Path { segments } => format_path(segments, out),
        ExprKind::Self_ => out.push_str("self"),
        ExprKind::Call { callee, args } => {
            format_expr(callee, ATOM_BP, out);
            format_args(args, out);
        }
        ExprKind::MethodCall { receiver, name, args } => {
            format_expr(receiver, ATOM_BP, out);
            out.push('.');
            out.push_str(&name.value);
            format_args(args, out);
        }
        ExprKind::Field { receiver, name } => {
            format_expr(receiver, ATOM_BP, out);
            out.push('.');
            out.push_str(&name.value);
        }
        ExprKind::StructLiteral { name, fields, base } => {
            out.push_str(&name.value);
            out.push_str(" { ");
            for (i, f) in fields.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                out.push_str(&f.name.value);
                if let Some(v) = &f.value {
                    out.push_str(": ");
                    format_expr(v, 0, out);
                }
            }
            if let Some(base) = base {
                if !fields.is_empty() {
                    out.push_str(", ");
                }
                out.push_str("..");
                format_expr(base, 0, out);
            }
            out.push_str(" }");
        }
        ExprKind::Index { base, index } => {
            format_expr(base, ATOM_BP, out);
            format_index(index, out);
        }
        ExprKind::Binary { op, lhs, rhs } => {
            let lbp = binop_lbp(*op);
            // `^` is right-associative (spec §4.3): a same-precedence left operand must be parenthesized.
            let lhs_min = if *op == BinOp::Pow { lbp + 1 } else { lbp };
            format_expr(lhs, lhs_min, out);
            if *op == BinOp::Pow {
                // Exponent notation is tight (`x^2`, spec §4.3), matching the canonical examples.
                out.push_str(binop_sym(*op));
            } else {
                out.push(' ');
                out.push_str(binop_sym(*op));
                out.push(' ');
            }
            format_expr(rhs, binop_rbp(*op), out);
        }
        ExprKind::Unary { op, operand } => {
            out.push_str(unary_sym(*op));
            format_expr(operand, UNARY_BP, out);
        }
        ExprKind::Try(operand) => {
            format_expr(operand, ATOM_BP, out);
            out.push('?');
        }
        ExprKind::Array(items) => {
            out.push('[');
            format_expr_list(items, out);
            out.push(']');
        }
        ExprKind::Tuple(items) => {
            out.push('(');
            format_expr_list(items, out);
            out.push(')');
        }
        ExprKind::Dict(pairs) => {
            if pairs.is_empty() {
                // An empty `{}` is an empty Dict (spec §11.6).
                out.push_str("{}");
            } else {
                out.push_str("{ ");
                for (i, (k, v)) in pairs.iter().enumerate() {
                    if i > 0 {
                        out.push_str(", ");
                    }
                    format_expr(k, 0, out);
                    out.push_str(": ");
                    format_expr(v, 0, out);
                }
                out.push_str(" }");
            }
        }
        ExprKind::Set(items) => {
            if items.is_empty() {
                out.push_str("{}");
            } else {
                out.push_str("{ ");
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        out.push_str(", ");
                    }
                    format_expr(item, 0, out);
                }
                out.push_str(" }");
            }
        }
        ExprKind::Comprehension { kind, output, clauses } => {
            format_comp_frame(*kind, out);
            format_expr(output, 0, out);
            for clause in clauses {
                match clause {
                    ComprehensionClause::For { var, iter } => {
                        out.push_str(" for ");
                        out.push_str(&var.value);
                        out.push_str(" in ");
                        format_expr(iter, 0, out);
                    }
                    ComprehensionClause::If { cond } => {
                        out.push_str(" if ");
                        format_expr(cond, 0, out);
                    }
                }
            }
            format_comp_close(*kind, out);
        }
        ExprKind::KeyValue { key, value } => {
            format_expr(key, 0, out);
            out.push_str(": ");
            format_expr(value, 0, out);
        }
        ExprKind::Lambda { params, body } => {
            out.push('|');
            format_params_bare(params, out);
            out.push_str("| ");
            format_expr(body, 0, out);
        }
        ExprKind::Match { scrutinee, arms } => {
            out.push_str("match ");
            format_expr(scrutinee, 0, out);
            out.push_str(" {\n");
            for arm in arms {
                format_match_arm(arm, 1, out);
            }
            out.push('}');
        }
        ExprKind::Custom(pairs) => {
            out.push_str("custom { ");
            for (i, (k, v)) in pairs.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                format_expr(k, 0, out);
                out.push_str(" := ");
                format_expr(v, 0, out);
            }
            out.push_str(" }");
        }
    }
}

fn format_comp_frame(kind: CompKind, out: &mut String) {
    match kind {
        CompKind::Array => out.push('['),
        CompKind::Dict => out.push_str("{ "),
        CompKind::Set => out.push_str("{ "),
        CompKind::Tuple => out.push('('),
    }
}

fn format_comp_close(kind: CompKind, out: &mut String) {
    match kind {
        CompKind::Array => out.push(']'),
        CompKind::Dict | CompKind::Set => out.push_str(" }"),
        CompKind::Tuple => out.push(')'),
    }
}

fn format_args(args: &[Expr], out: &mut String) {
    out.push('(');
    format_expr_list(args, out);
    out.push(')');
}

fn format_expr_list(items: &[Expr], out: &mut String) {
    for (i, item) in items.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        format_expr(item, 0, out);
    }
}

fn format_index(index: &Index, out: &mut String) {
    out.push('[');
    for (i, item) in index.items.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        match item {
            IndexItem::Elem(e) => format_expr(e, 0, out),
            IndexItem::Slice { start, end } => {
                if let Some(s) = start {
                    format_expr(s, 0, out);
                }
                out.push_str("..");
                if let Some(e) = end {
                    format_expr(e, 0, out);
                }
            }
        }
    }
    out.push(']');
}

/// Re-escape a decoded char for re-emission (spec §18.1): the lexer accepts
/// `\n \t \r \0 \\ \" \'` and `\u{HEX}`; other control characters use `\u{...}`.
fn escape_string(c: char) -> String {
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
fn escape_char(c: char) -> String {
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

fn format_literal(lit: &Literal, out: &mut String) {
    match lit {
        // Numeric literals keep their raw source text (impl §3): re-emitting it is lossless.
        Literal::Integer(s) | Literal::Float(s) | Literal::Hex(s) | Literal::Binary(s) => out.push_str(s),
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

/// Re-emit an f-string in escaped canonical form (spec §18.1): literal text is escaped and
/// `{`/`}` doubled so the output round-trips to the same value (raw-ness is not preserved).
fn format_fstring(parts: &[FStringPart], out: &mut String) {
    out.push('f');
    out.push('"');
    for p in parts {
        match p {
            FStringPart::Literal(s) => {
                for c in s.chars() {
                    match c {
                        '{' => out.push_str("{{"),
                        '}' => out.push_str("}}"),
                        _ => out.push_str(&escape_string(c)),
                    }
                }
            }
            FStringPart::Interp { expr, spec } => {
                out.push('{');
                // A formatted expression that starts with `{` (e.g. a dict literal or a postfix
                // over one) would lex as an escaped `{{`, and one ending with `}` would collide
                // with the closing `}` — insert spaces so the output re-parses to the same parts.
                let mut rendered = String::new();
                format_expr(expr, 0, &mut rendered);
                let leading = rendered.starts_with('{');
                let trailing = rendered.ends_with('}');
                if leading {
                    out.push(' ');
                }
                out.push_str(&rendered);
                if let Some(s) = spec {
                    out.push(':');
                    out.push_str(s);
                }
                if trailing {
                    out.push(' ');
                }
                out.push('}');
            }
        }
    }
    out.push('"');
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
        assert_eq!(fmt_of("pub let square(x) = x^2;"), "pub let square(x) = x^2;\n");
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
