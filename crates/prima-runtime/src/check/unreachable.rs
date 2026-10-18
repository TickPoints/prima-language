//! `W0004 unreachable_code` (spec §16.5): statements after a `return` in the same block never run.
//!
//! The language has no `break`/`continue` (loops exit through conditions), so `return` is the only
//! statement that makes the rest of its block unreachable. The check is deliberately syntactic:
//! it does not try to prove whole branches unreachable.

use prima_syntax::ast::{Block, Program, Stmt};
use prima_syntax::{Span, SyntaxWarning};

/// Walk every block in the program and report the first statement after each `return`.
pub(crate) fn check_program_unreachable(program: &Program, warnings: &mut Vec<SyntaxWarning>) {
    check_statements(&program.stmts, warnings);
}

fn check_block(block: &Block, warnings: &mut Vec<SyntaxWarning>) {
    check_statements(&block.stmts, warnings);
}

fn check_statements(stmts: &[Stmt], warnings: &mut Vec<SyntaxWarning>) {
    for (i, stmt) in stmts.iter().enumerate() {
        if let Stmt::Return { .. } = stmt
            && let Some(next) = stmts.get(i + 1)
        {
            warnings.push(SyntaxWarning {
                span: stmt_span(next),
                code: "W0004",
                message: "unreachable code after `return`".into(),
            });
            // Everything after the first unreachable statement is unreachable too; one warning per
            // block is enough, and nested definitions below it cannot run.
            break;
        }
        check_stmt(stmt, warnings);
    }
}

fn check_stmt(stmt: &Stmt, warnings: &mut Vec<SyntaxWarning>) {
    match stmt {
        Stmt::FnDef { body, .. } => check_block(body, warnings),
        Stmt::ClassDef { members, .. } => {
            for member in members {
                if let prima_syntax::ast::ClassMemberKind::Method { body, .. } = &member.kind
                    && let Some(body) = body
                {
                    check_block(body, warnings);
                }
            }
        }
        Stmt::Impl { members, .. } => {
            for member in members {
                check_stmt(member, warnings);
            }
        }
        Stmt::For { body, .. }
        | Stmt::ParFor { body, .. }
        | Stmt::While { body, .. }
        | Stmt::WhileLet { body, .. } => check_block(body, warnings),
        Stmt::If {
            then, elifs, else_, ..
        } => {
            check_block(then, warnings);
            for (_, block) in elifs {
                check_block(block, warnings);
            }
            if let Some(block) = else_ {
                check_block(block, warnings);
            }
        }
        Stmt::IfLet { then, else_, .. } => {
            check_block(then, warnings);
            if let Some(block) = else_ {
                check_block(block, warnings);
            }
        }
        Stmt::WithConfig { body, .. } => check_block(body, warnings),
        Stmt::Pub(inner) => check_stmt(inner, warnings),
        Stmt::Match { .. }
        | Stmt::Let { .. }
        | Stmt::Const { .. }
        | Stmt::MathDef { .. }
        | Stmt::Assign { .. }
        | Stmt::Expr(_)
        | Stmt::Return { .. } => {}
    }
}

/// Source span of a statement (for the warning caret).
fn stmt_span(stmt: &Stmt) -> Span {
    match stmt {
        Stmt::Let { span, .. }
        | Stmt::Const { span, .. }
        | Stmt::FnDef { span, .. }
        | Stmt::MathDef { span, .. }
        | Stmt::ClassDef { span, .. }
        | Stmt::Impl { span, .. }
        | Stmt::Assign { span, .. }
        | Stmt::For { span, .. }
        | Stmt::ParFor { span, .. }
        | Stmt::While { span, .. }
        | Stmt::If { span, .. }
        | Stmt::IfLet { span, .. }
        | Stmt::WhileLet { span, .. }
        | Stmt::Match { span, .. }
        | Stmt::Return { span, .. }
        | Stmt::WithConfig { span, .. } => *span,
        Stmt::Expr(expr) => expr.span,
        Stmt::Pub(inner) => stmt_span(inner),
    }
}
