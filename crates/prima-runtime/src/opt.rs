//! Tail-call optimization analysis (spec §10.2 item 6): detects when a function block ends in a
//! direct `return f(args)` tail call after only effect-free statements, enabling a trampoline loop.

use prima_syntax::ast::{Block, Expr, ExprKind, Stmt};

/// A detected tail call: the callee expression and the argument expressions, evaluated in the
/// caller's scope before the jump.
#[derive(Debug, Clone, PartialEq)]
pub struct TailCall {
    pub callee: Expr,
    pub args: Vec<Expr>,
}

/// Whether a statement has no observable side effects and no non-local control flow except `return`:
/// `let`/`const` with a pure RHS, an `Expr` statement whose expression is pure, and `if`/`if-else`
/// whose branches are themselves effect-free-or-return. Assignments, `print`/`println`/`input`,
/// `while`/`for`, mutation, and calls to unknown/impure functions are NOT effect-free.
pub fn is_effect_free_stmt(stmt: &Stmt) -> bool {
    match stmt {
        Stmt::Let { value, .. } | Stmt::Const { value, .. } => is_pure_expr(value),
        Stmt::Expr(e) => is_pure_expr(e),
        Stmt::If { cond, then, elifs, else_, .. } => {
            is_pure_expr(cond)
                && elifs.iter().all(|(c, _)| is_pure_expr(c))
                && is_effect_free_or_return(then)
                && elifs.iter().all(|(_, b)| is_effect_free_or_return(b))
                && else_.as_ref().is_none_or(is_effect_free_or_return)
        }
        Stmt::Return { .. } => true,
        _ => false,
    }
}

/// All statements of `block` are effect-free; a `return` inside the block counts as effect-free
/// (it is non-local control flow, but never observes or mutates state).
fn is_effect_free_or_return(block: &Block) -> bool {
    block.stmts.iter().all(is_effect_free_stmt)
}

/// Conservative pure-expression test. Anything not provably side-effect-free is rejected: a missed
/// optimization is safe, an unsound one is a bug.
fn is_pure_expr(e: &Expr) -> bool {
    match &e.kind {
        ExprKind::Literal(_) | ExprKind::Symbol(_) | ExprKind::Path { .. } => true,
        ExprKind::Binary { lhs, rhs, .. } => {
            // All binary math/comparison/logic/set operators are pure when their operands are.
            is_pure_expr(lhs) && is_pure_expr(rhs)
        }
        ExprKind::Unary { operand, .. } => is_pure_expr(operand),
        ExprKind::Array(items) | ExprKind::Tuple(items) | ExprKind::Set(items) => {
            items.iter().all(is_pure_expr)
        }
        ExprKind::Dict(pairs) => pairs.iter().all(|(k, v)| is_pure_expr(k) && is_pure_expr(v)),
        ExprKind::Call { callee, args } => {
            // Only direct named calls to a known-pure builtin; `obj.method(...)` and any unknown
            // callee (including user functions and the function under TCO itself) are rejected.
            match &callee.kind {
                ExprKind::Path { segments } => {
                    segments
                        .last()
                        .is_some_and(|s| is_pure_builtin(&s.value))
                        && args.iter().all(is_pure_expr)
                }
                _ => false,
            }
        }
        ExprKind::Match { scrutinee, arms } => {
            is_pure_expr(scrutinee)
                && arms.iter().all(|arm| {
                    arm.guard.as_ref().is_none_or(is_pure_expr) && is_pure_expr(&arm.body)
                })
        }
        ExprKind::Lambda { body, .. } => is_pure_expr(body),
        ExprKind::Comprehension { output, clauses, .. } => {
            is_pure_expr(output)
                && clauses.iter().all(|c| match c {
                    prima_syntax::ast::ComprehensionClause::For { iter, .. } => is_pure_expr(iter),
                    prima_syntax::ast::ComprehensionClause::If { cond } => is_pure_expr(cond),
                })
        }
        ExprKind::KeyValue { key, value } => is_pure_expr(key) && is_pure_expr(value),
        // Everything else (field/index access, method calls, `?`, pipelines, struct literals,
        // mutation-adjacent forms, `self` references) is conservatively impure.
        _ => false,
    }
}

/// Known-pure builtin names (spec §10.2/§9): math functions, size/aggregation helpers, string
/// construction, plus the whole `to_*`/`try_*`/`checked_*` collapse family (spec §9.2–§9.6).
fn is_pure_builtin(name: &str) -> bool {
    if name.starts_with("to_") || name.starts_with("try_") || name.starts_with("checked_") {
        return true;
    }
    matches!(
        name,
        "sqrt" | "exp" | "log" | "ln" | "sin" | "cos" | "tan" | "abs" | "simplify"
            | "derivative" | "partial" | "grad" | "limit" | "len" | "sum" | "min" | "max"
            | "concat" | "to_string"
    )
}

/// If `block` ends with `return f(args)` (an `ExprKind::Path` callee — single or `a::b` path) and
/// every statement before it is effect-free (see [`is_effect_free_stmt`]), return the tail call.
/// `None` otherwise.
///
/// Conservative limitation: the trailing statement must itself be a `return`. A block whose last
/// statement is `if c { return g(x); } else { return h(x); }` is not converted, even though both
/// branches are tail calls, because the conditional structure would need `else`-arm pairing —
/// such a pattern stays untransformed.
pub fn tail_call_of(block: &Block) -> Option<TailCall> {
    let (last, prior) = block.stmts.split_last()?;
    if !prior.iter().all(is_effect_free_stmt) {
        return None;
    }
    let Stmt::Return { value: Some(e), .. } = last else {
        return None;
    };
    let ExprKind::Call { callee, args } = &e.kind else {
        return None;
    };
    if !matches!(callee.kind, ExprKind::Path { .. }) {
        return None;
    }
    Some(TailCall { callee: (**callee).clone(), args: args.clone() })
}

#[cfg(test)]
mod tests {
    use super::{is_effect_free_stmt, tail_call_of};
    use prima_syntax::ast::{Block, ExprKind, Stmt};
    use prima_syntax::parse;

    /// Parse a single `fn` definition and return a reference to its body block.
    fn body_of(src: &str) -> Block {
        let p = parse(src).unwrap();
        let Stmt::FnDef { body, .. } = &p.stmts[0] else {
            panic!("expected a function definition, got {:?}", p.stmts[0]);
        };
        body.clone()
    }

    fn path_tail_call_callee(block: &Block) -> Option<String> {
        tail_call_of(block).map(|tc| match tc.callee.kind {
            ExprKind::Path { segments } => segments
                .iter()
                .map(|s| s.value.clone())
                .collect::<Vec<_>>()
                .join("::"),
            _ => unreachable!("tail_call_of only yields `Path` callees"),
        })
    }

    #[test]
    fn tail_call_after_effect_free_let() {
        let body = body_of("fn f() { let a = 1; return g(a); }");
        assert_eq!(path_tail_call_callee(&body).as_deref(), Some("g"));
        let tc = tail_call_of(&body).unwrap();
        assert_eq!(tc.args.len(), 1);
        assert!(matches!(tc.args[0].kind, ExprKind::Path { .. }));
    }

    #[test]
    fn tail_call_single_statement() {
        let body = body_of("fn f() { return g(x); }");
        assert_eq!(path_tail_call_callee(&body).as_deref(), Some("g"));
    }

    #[test]
    fn tail_call_multisegment_path_callee() {
        let body = body_of("fn f() { return a::b::g(x); }");
        assert_eq!(path_tail_call_callee(&body).as_deref(), Some("a::b::g"));
    }

    #[test]
    fn no_tail_call_after_effectful_statement() {
        let body = body_of("fn f() { print(\"hi\"); return g(x); }");
        assert!(tail_call_of(&body).is_none());
    }

    #[test]
    fn no_tail_call_for_non_call_return() {
        let body = body_of("fn f() { return g(x) + 1; }");
        assert!(tail_call_of(&body).is_none());
    }

    #[test]
    fn no_tail_call_for_method_call_callee() {
        let body = body_of("fn f() { return obj.m(x); }");
        assert!(tail_call_of(&body).is_none());
    }

    #[test]
    fn no_tail_call_for_if_last_statement() {
        // Conservative: the last statement is `if`, not a direct `return f(args)`.
        let body = body_of("fn f() { if c { return g(x); } else { return h(x); } }");
        assert!(tail_call_of(&body).is_none());
    }

    #[test]
    fn no_tail_call_for_assignment_prior() {
        let body = body_of("fn f() { x = 2; return g(x); }");
        assert!(tail_call_of(&body).is_none());
    }

    #[test]
    fn effect_free_statements() {
        let p = parse("let x = 1;").unwrap();
        assert!(is_effect_free_stmt(&p.stmts[0]));

        let p = parse("let y = sqrt(4) + x;").unwrap();
        assert!(is_effect_free_stmt(&p.stmts[0]));

        let p = parse("const k: Number = 2;").unwrap();
        assert!(is_effect_free_stmt(&p.stmts[0]));

        let p = parse("if a { return 1; }").unwrap();
        assert!(is_effect_free_stmt(&p.stmts[0]));

        let p = parse("return g(x);").unwrap();
        assert!(is_effect_free_stmt(&p.stmts[0]));
    }

    #[test]
    fn effectful_statements_are_rejected() {
        let p = parse("print(x);").unwrap();
        assert!(!is_effect_free_stmt(&p.stmts[0]));

        let p = parse("x = 2;").unwrap();
        assert!(!is_effect_free_stmt(&p.stmts[0]));

        let p = parse("while a { }").unwrap();
        assert!(!is_effect_free_stmt(&p.stmts[0]));

        let p = parse("for i in 0..10 { }").unwrap();
        assert!(!is_effect_free_stmt(&p.stmts[0]));

        let p = parse("input(\"name\");").unwrap();
        assert!(!is_effect_free_stmt(&p.stmts[0]));

        let p = parse("unknown_fn(x);").unwrap();
        assert!(!is_effect_free_stmt(&p.stmts[0]));

        let p = parse("obj.m(x);").unwrap();
        assert!(!is_effect_free_stmt(&p.stmts[0]));
    }

    #[test]
    fn effect_free_if_with_pure_arms() {
        let p = parse("if c { let a = 1; } else { let b = 2; }").unwrap();
        assert!(is_effect_free_stmt(&p.stmts[0]));
    }

    #[test]
    fn effect_free_rejects_effectful_branch() {
        let p = parse("if c { print(1); }").unwrap();
        assert!(!is_effect_free_stmt(&p.stmts[0]));

        let p = parse("if c { return 1; } else { x = 2; }").unwrap();
        assert!(!is_effect_free_stmt(&p.stmts[0]));
    }
}
