//! `config {}` / `with config {}` key validation (spec §13.2 / appendix C.1 `E0022`).
//!
//! Unknown strategy keys used to fail at runtime with a generic message. They are statically
//! decidable, so `prima check` reports `E0022 unknown_config` with a `did you mean` suggestion.

use prima_core::suggest::did_you_mean_help;
use prima_syntax::ast::{Block, ConfigEntry, Program, Stmt};

use super::TypeError;
use super::error::push_err_with_help;

/// Check every config entry in the program (top-level block and nested `with config` blocks).
pub(crate) fn check_program_config(
    src: &str,
    program: &Program,
    errors: &mut Vec<TypeError>,
) {
    if let Some(block) = &program.config {
        check_entries(src, &block.entries, errors);
    }
    check_stmts(src, &program.stmts, errors);
}

fn check_entries(src: &str, entries: &[ConfigEntry], errors: &mut Vec<TypeError>) {
    for entry in entries {
        let name = entry.name.value.as_str();
        if crate::config::KNOWN_KEYS.contains(&name) {
            continue;
        }
        let help = did_you_mean_help(name, crate::config::KNOWN_KEYS.iter().copied());
        push_err_with_help(
            src,
            errors,
            entry.name.span,
            "E0022",
            format!("unknown config key `{name}`"),
            None,
            help,
        );
    }
}

fn check_stmts(src: &str, stmts: &[Stmt], errors: &mut Vec<TypeError>) {
    for stmt in stmts {
        check_stmt(src, stmt, errors);
    }
}

fn check_block(src: &str, block: &Block, errors: &mut Vec<TypeError>) {
    check_stmts(src, &block.stmts, errors);
}

fn check_stmt(src: &str, stmt: &Stmt, errors: &mut Vec<TypeError>) {
    match stmt {
        Stmt::FnDef { body, .. } => check_block(src, body, errors),
        Stmt::ClassDef { members, .. } => {
            for member in members {
                if let prima_syntax::ast::ClassMemberKind::Method { body, .. } = &member.kind
                    && let Some(body) = body
                {
                    check_block(src, body, errors);
                }
            }
        }
        Stmt::Impl { members, .. } => {
            for member in members {
                check_stmt(src, member, errors);
            }
        }
        Stmt::For { body, .. }
        | Stmt::ParFor { body, .. }
        | Stmt::While { body, .. }
        | Stmt::WhileLet { body, .. } => check_block(src, body, errors),
        Stmt::If {
            then,
            elifs,
            else_,
            ..
        } => {
            check_block(src, then, errors);
            for (_, block) in elifs {
                check_block(src, block, errors);
            }
            if let Some(block) = else_ {
                check_block(src, block, errors);
            }
        }
        Stmt::IfLet { then, else_, .. } => {
            check_block(src, then, errors);
            if let Some(block) = else_ {
                check_block(src, block, errors);
            }
        }
        Stmt::WithConfig { entries, body, .. } => {
            check_entries(src, entries, errors);
            check_block(src, body, errors);
        }
        Stmt::Pub(inner) => check_stmt(src, inner, errors),
        Stmt::Match { .. }
        | Stmt::Let { .. }
        | Stmt::Const { .. }
        | Stmt::MathDef { .. }
        | Stmt::Assign { .. }
        | Stmt::Expr(_)
        | Stmt::Return { .. } => {}
    }
}
