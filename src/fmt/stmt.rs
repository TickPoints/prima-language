//! Statement-source rendering for `prima fmt` (spec §4.2/§4.5/§15.2): `let`/`const`/`fn`/`class`/
//! `impl`, assignments, and the control-flow statements (`for`/`parfor`/`while`/`if`/`if let`/
//! `while let`/`match`/`return`/`with config`), plus class members, annotations, and the block-body
//! helper. `format_visibility` is shared with `prima doc`, so it is `pub(crate)` and re-exported
//! at `crate::fmt`.

use prima_syntax::ast::{
    Annotation, AssignOp, Block, ClassMember, ClassMemberKind, ImplOp, Stmt, Visibility,
};

use super::config::format_config_entry;
use super::expr::{ATOM_BP, format_expr, format_match_arm, format_step};
use super::pattern::format_pattern;
use super::text::{format_doc_lines, push_indent, stmt_docs};
use super::ty::{format_params, format_ret, format_type};

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

fn format_annotations(annotations: &[Annotation], out: &mut String) {
    for a in annotations {
        match a {
            Annotation::Parallel => out.push_str("@parallel "),
            Annotation::Jit => out.push_str("@jit "),
            Annotation::Gpu => out.push_str("@gpu "),
            Annotation::Builtin { opt_level } => {
                if *opt_level == 0 {
                    out.push_str("@builtin ");
                } else {
                    out.push_str(&format!("@builtin(O{}) ", opt_level));
                }
            }
            Annotation::CApiExtern => out.push_str("@c_api::extern "),
        }
    }
}

pub(crate) fn format_stmt(stmt: &Stmt, indent: usize, out: &mut String) {
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
        Stmt::Let {
            pat,
            mut_,
            type_ann,
            value,
            ..
        } => {
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
        Stmt::Const {
            name,
            type_ann,
            value,
            ..
        } => {
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
        Stmt::FnDef {
            name,
            params,
            ret,
            annotations,
            body,
            ..
        } => {
            format_annotations(annotations, out);
            if is_pub {
                out.push_str("pub ");
            }
            out.push_str("fn ");
            out.push_str(&name.value);
            format_params(params, out);
            format_ret(ret, out);
            out.push(' ');
            format_block(body, indent, out);
        }
        Stmt::MathDef {
            name,
            params,
            ret,
            annotations,
            body,
            ..
        } => {
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
        Stmt::ClassDef {
            name,
            annotations,
            members,
            ..
        } => {
            format_annotations(annotations, out);
            if is_pub {
                out.push_str("pub ");
            }
            out.push_str("class ");
            out.push_str(&name.value);
            out.push_str(" {\n");
            for member in members {
                format_class_member(member, indent + 1, out);
            }
            push_indent(indent, out);
            out.push('}');
        }
        Stmt::Impl {
            op,
            target,
            members,
            ..
        } => {
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
        Stmt::Assign {
            target, op, value, ..
        } => {
            format_expr(target, ATOM_BP, out);
            out.push(' ');
            out.push_str(assign_op_name(*op));
            out.push(' ');
            format_expr(value, 0, out);
        }
        Stmt::Expr(e) => format_expr(e, 0, out),
        Stmt::For {
            var,
            range,
            step,
            body,
            ..
        } => {
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
        Stmt::ParFor {
            var,
            range,
            step,
            body,
            ..
        } => {
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
        Stmt::If {
            cond,
            then,
            elifs,
            else_,
            ..
        } => {
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
        Stmt::IfLet {
            pat,
            value,
            then,
            else_,
            ..
        } => {
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
        Stmt::WhileLet {
            pat, value, body, ..
        } => {
            out.push_str("while let ");
            format_pattern(pat, out);
            out.push_str(" = ");
            format_expr(value, 0, out);
            out.push(' ');
            format_block(body, indent, out);
        }
        Stmt::Match {
            scrutinee, arms, ..
        } => {
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
        ClassMemberKind::Method {
            name,
            params,
            ret,
            annotations,
            body,
        } => {
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
