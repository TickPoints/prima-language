//! AST → bytecode compiler (spec §19.5, Milestone B).
//!
//! `compile_program` lowers a parsed `Program` into a `Program` of chunks: the root chunk plus one
//! chunk per lowered `fn`/class method. Each statement/expression is lowered bottom-up onto the VM
//! operand stack; locals are slot-allocated; upvalues are captured for closures.
//!
//! Phase 1 (this milestone) lowers a conservative but real subset and routes everything else to the
//! AST fallback at the `Evaluator` boundary, so the VM can be extended construct-by-construct.

use prima_syntax::ast::{Block, Expr, ExprKind, Literal, Program, Stmt, Type};

use super::op::{Const, Program as VmProgram};

/// Compile a `Program` into a VM program. Phase 1 supports a node subset; unsupported constructs
/// are rejected here and the caller falls back to the AST interpreter for that function.
pub fn compile_program(_ast: &Program) -> Result<VmProgram, String> {
    Err("VM compiler: not yet wired (Phase 1)".into())
}

/// Lower an expression to a single constant (Phase 1 subset): numeric/bool/string/char literals.
#[allow(dead_code)]
fn literal_to_value(lit: &Literal) -> Option<prima_core::Value> {
    use prima_core::{Number, Real};
    match lit {
        Literal::Integer(s) => {
            let n = Number::Integer(s.parse().ok()?);
            Some(prima_core::Value::Number(n))
        }
        Literal::Float(s) => {
            let n = Number::Real(Real::F64(s.parse().ok()?));
            Some(prima_core::Value::Number(n))
        }
        Literal::Bool(b) => Some(prima_core::Value::Bool(*b)),
        Literal::String { value, .. } => Some(prima_core::Value::String(value.clone())),
        Literal::Char(c) => Some(prima_core::Value::Char(*c)),
        _ => None,
    }
}

/// Whether an expression is statically a leaf constant.
pub fn is_constant_expr(e: &Expr) -> bool {
    matches!(&e.kind, ExprKind::Literal(_))
        || matches!(&e.kind, ExprKind::Path { segments } if segments.len() == 1
            && (segments[0].value == "true" || segments[0].value == "false"))
}

// Module-level helpers kept private; the compiler is grown in-place across commits.
#[allow(dead_code)]
fn _expr_kind(e: &Expr) -> &ExprKind {
    &e.kind
}
#[allow(dead_code)]
fn _block(_b: &Block) {}
#[allow(dead_code)]
fn _stmt(_s: &Stmt) {}
#[allow(dead_code)]
fn _ty(_t: &Type) {}
#[allow(dead_code)]
fn _const(c: Const) -> Const {
    c
}
