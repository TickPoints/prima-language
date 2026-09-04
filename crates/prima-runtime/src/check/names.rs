//! Lexical scope & name-resolution checks (spec §16.2 / appendix C compile-time errors).
//!
//! A lightweight scope-stack walk over the AST that detects statically-decidable name errors
//! without evaluating: `E0040 undefined_name` (a single-segment variable path used outside its
//! scope), `E0080 return_outside_fn` (a `return` outside any function/method body), `E0062
//! self_outside_method` (`self` outside a method), and `W0003 unused_binding` (`let`-bound names
//! never referenced in a later expression).
//!
//! It is deliberately conservative: only *single-segment* path/symbol references in value position
//! are considered variable uses, and the pre-imported `core` builtins plus the primitive type names
//! are seeded into the root scope, so defined-elsewhere symbols are not misreported as undefined.

use std::collections::HashSet;

use prima_syntax::ast::{
    ClassMemberKind, ComprehensionClause, Expr, FStringPart, IndexItem, Param, Pattern, Stmt, Type,
};
use prima_syntax::{Span, SyntaxWarning};

use super::TypeError;
use super::line_col;

/// Names always bound at the top of any module (pre-imported `core` values/functions, control and
/// collapse builtins, constructors). Never reported as undefined.
const ROOT_NAMES: &[&str] = &[
    // control / I/O / math builtins
    "print",
    "println",
    "input",
    "read_line",
    "simplify",
    "derivative",
    "partial",
    "grad",
    "limit",
    "jit",
    "range",
    "sqrt",
    "exp",
    "log",
    "ln",
    "sin",
    "cos",
    "tan",
    "abs",
    "map",
    "filter",
    "reduce",
    "len",
    "enumerate",
    "zip",
    "sorted",
    "reversed",
    "sum",
    "prod",
    "min",
    "max",
    "all",
    "any",
    "join",
    "count",
    "index",
    "first",
    "last",
    "linspace",
    "concat",
    "to_string",
    "get",
    // collapse families
    "to_i8",
    "to_i16",
    "to_i32",
    "to_i64",
    "to_i128",
    "to_u8",
    "to_u16",
    "to_u32",
    "to_u64",
    "to_u128",
    "to_isize",
    "to_usize",
    "to_f32",
    "to_f64",
    "to_bigint",
    "to_rational",
    "to_bigfloat",
    "to_complex",
    "try_i8",
    "try_i16",
    "try_i32",
    "try_i64",
    "try_i128",
    "try_u8",
    "try_u16",
    "try_u32",
    "try_u64",
    "try_u128",
    "try_isize",
    "try_usize",
    "try_f32",
    "try_f64",
    "try_bigint",
    "try_rational",
    "try_complex",
    "checked_i8",
    "checked_i16",
    "checked_i32",
    "checked_i64",
    "checked_i128",
    "checked_u8",
    "checked_u16",
    "checked_u32",
    "checked_u64",
    "checked_u128",
    "checked_add",
    "checked_mul",
    "clamped_i8",
    "clamped_i16",
    "clamped_i32",
    "clamped_i64",
    "clamped_i128",
    "clamped_u8",
    "clamped_u16",
    "clamped_u32",
    "clamped_u64",
    "clamped_u128",
    "clamped_f32",
    "clamped_f64",
    "rounded_f64",
    "rounded_f32",
    "rounded_i32",
    "truncated_i32",
    "unwrap",
    "unwrap_or",
    "expect",
    // constructors / enum variants
    "Some",
    "None",
    "Ok",
    "Err",
];

/// Names that are primitive type references (usable in signatures); never reported as undefined.
const TYPE_NAMES: &[&str] = &[
    "Number", "Integer", "Rational", "F64", "F32", "I8", "I16", "I32", "I64", "I128", "U8", "U16",
    "U32", "U64", "U128", "Isize", "Usize", "Complex", "Expr", "Symbol", "Bool", "String", "Char",
    "Value", "Error", "Self", "SelfType",
];

/// One lexical scope: `name → binding span` plus a manual "used" set; the used set is reset by the
/// check walk until a binding is referenced.
struct Scope {
    binds: Vec<(String, Span)>,
    used: HashSet<String>,
}

impl Scope {
    fn new() -> Scope {
        Scope {
            binds: Vec::new(),
            used: HashSet::new(),
        }
    }

    fn bind(&mut self, name: &str, span: Span) {
        // Recorded as a vector so the *most recent* binding of the same name wins for `mark_used`;
        // shadowing keeps the old binding visible for the unused-report at scope exit.
        if !self.binds.iter().any(|(n, _)| n == name) {
            self.binds.push((name.to_string(), span));
        }
    }
}

/// Name-resolution context: a stack of scopes plus the function-depth counter (for `return`/`self`).
struct NameCtx {
    scopes: Vec<Scope>,
    fn_depth: usize,
}

impl NameCtx {
    fn new() -> NameCtx {
        let mut root = Scope::new();
        for &n in ROOT_NAMES.iter().chain(TYPE_NAMES.iter()) {
            root.bind(n, Span::new(0, 0));
        }
        NameCtx {
            scopes: vec![root],
            fn_depth: 0,
        }
    }

    fn push_scope(&mut self) {
        self.scopes.push(Scope::new());
    }

    fn pop_scope(&mut self) {
        self.scopes.pop();
    }

    fn bind(&mut self, name: &str, span: Span) {
        if let Some(s) = self.scopes.last_mut() {
            s.bind(name, span);
        }
    }

    fn is_bound(&self, name: &str) -> bool {
        self.scopes
            .iter()
            .rev()
            .any(|s| s.binds.iter().any(|(n, _)| n == name))
    }

    fn mark_used(&mut self, name: &str) {
        for s in self.scopes.iter_mut().rev() {
            if s.binds.iter().any(|(n, _)| n == name) {
                s.used.insert(name.to_string());
                return;
            }
        }
    }
}

/// Collect unused bindings surviving in every scope at the end of a walk, as `(name, span)` pairs.
fn collect_unused(ctx: &NameCtx) -> Vec<(String, Span)> {
    let mut out = Vec::new();
    for s in &ctx.scopes {
        for (name, span) in &s.binds {
            // Seeded builtins/type names never participate in the unused check.
            if ROOT_NAMES.contains(&name.as_str()) || TYPE_NAMES.contains(&name.as_str()) {
                continue;
            }
            if !s.used.contains(name) {
                out.push((name.clone(), *span));
            }
        }
    }
    out
}

/// Run the name/scope checks over a whole parsed program, appending `TypeError`s and `W0003` warnings.
pub(crate) fn check_program_names(
    src: &str,
    program: &prima_syntax::ast::Program,
    errors: &mut Vec<TypeError>,
    warnings: &mut Vec<SyntaxWarning>,
) {
    let mut ctx = NameCtx::new();
    check_block(src, &program.stmts, &mut ctx, errors);
    for (name, span) in collect_unused(&ctx) {
        warnings.push(SyntaxWarning {
            span,
            code: "W0003",
            message: format!("unused binding: `{name}`"),
        });
    }
}

fn check_block(src: &str, stmts: &[Stmt], ctx: &mut NameCtx, errors: &mut Vec<TypeError>) {
    for stmt in stmts {
        check_stmt(src, stmt, ctx, errors);
    }
}

fn check_stmt(src: &str, stmt: &Stmt, ctx: &mut NameCtx, errors: &mut Vec<TypeError>) {
    match stmt {
        Stmt::Let { pat, value, .. } => {
            check_expr(src, value, ctx, errors);
            bind_pattern(pat, ctx);
        }
        Stmt::Const { name, value, .. } => {
            check_expr(src, value, ctx, errors);
            ctx.bind(&name.value, name.span);
        }
        Stmt::FnDef {
            name,
            params,
            ret,
            body,
            ..
        } => {
            ctx.bind(&name.value, name.span);
            if let Some(t) = ret {
                check_type(t);
            }
            ctx.push_scope();
            ctx.fn_depth += 1;
            bind_params(params, ctx);
            check_block(src, &body.stmts, ctx, errors);
            ctx.fn_depth -= 1;
            ctx.pop_scope();
        }
        Stmt::MathDef {
            name,
            params,
            ret,
            body,
            ..
        } => {
            ctx.bind(&name.value, name.span);
            if let Some(t) = ret {
                check_type(t);
            }
            ctx.push_scope();
            ctx.fn_depth += 1;
            bind_params(params, ctx);
            check_expr(src, body, ctx, errors);
            ctx.fn_depth -= 1;
            ctx.pop_scope();
        }
        Stmt::ClassDef { name, members, .. } => {
            ctx.bind(&name.value, name.span);
            for member in members {
                match &member.kind {
                    ClassMemberKind::Field {
                        name: fname, ty, ..
                    } => {
                        check_type(ty);
                        ctx.bind(&fname.value, fname.span);
                    }
                    ClassMemberKind::Method {
                        name: mname,
                        params,
                        ret,
                        body,
                        ..
                    } => {
                        ctx.bind(&mname.value, mname.span);
                        if let Some(t) = ret {
                            check_type(t);
                        }
                        ctx.push_scope();
                        ctx.fn_depth += 1;
                        ctx.bind("self", mname.span);
                        bind_params(params, ctx);
                        if let Some(b) = body {
                            check_block(src, &b.stmts, ctx, errors);
                        }
                        ctx.fn_depth -= 1;
                        ctx.pop_scope();
                    }
                }
            }
        }
        Stmt::Impl { members, .. } => {
            for m in members {
                check_stmt(src, m, ctx, errors);
            }
        }
        Stmt::Expr(e) => check_expr(src, e, ctx, errors),
        Stmt::Assign { target, value, .. } => {
            check_expr(src, target, ctx, errors);
            check_expr(src, value, ctx, errors);
        }
        Stmt::For {
            var,
            range,
            step,
            body,
            ..
        } => {
            check_expr(src, &range.0, ctx, errors);
            check_expr(src, &range.1, ctx, errors);
            if let Some(s) = step {
                check_expr(src, s, ctx, errors);
            }
            ctx.push_scope();
            ctx.bind(&var.value, var.span);
            check_block(src, &body.stmts, ctx, errors);
            ctx.pop_scope();
        }
        Stmt::ParFor {
            var,
            range,
            step,
            body,
            ..
        } => {
            check_expr(src, &range.0, ctx, errors);
            check_expr(src, &range.1, ctx, errors);
            if let Some(s) = step {
                check_expr(src, s, ctx, errors);
            }
            ctx.push_scope();
            ctx.bind(&var.value, var.span);
            check_block(src, &body.stmts, ctx, errors);
            ctx.pop_scope();
        }
        Stmt::While { cond, body, .. } => {
            check_expr(src, cond, ctx, errors);
            ctx.push_scope();
            check_block(src, &body.stmts, ctx, errors);
            ctx.pop_scope();
        }
        Stmt::If {
            cond,
            then,
            elifs,
            else_,
            ..
        } => {
            check_expr(src, cond, ctx, errors);
            ctx.push_scope();
            check_block(src, &then.stmts, ctx, errors);
            ctx.pop_scope();
            for (c, b) in elifs {
                check_expr(src, c, ctx, errors);
                ctx.push_scope();
                check_block(src, &b.stmts, ctx, errors);
                ctx.pop_scope();
            }
            if let Some(b) = else_ {
                ctx.push_scope();
                check_block(src, &b.stmts, ctx, errors);
                ctx.pop_scope();
            }
        }
        Stmt::IfLet {
            pat,
            value,
            then,
            else_,
            ..
        } => {
            check_expr(src, value, ctx, errors);
            ctx.push_scope();
            bind_pattern(pat, ctx);
            check_block(src, &then.stmts, ctx, errors);
            ctx.pop_scope();
            if let Some(b) = else_ {
                ctx.push_scope();
                check_block(src, &b.stmts, ctx, errors);
                ctx.pop_scope();
            }
        }
        Stmt::WhileLet {
            pat, value, body, ..
        } => {
            check_expr(src, value, ctx, errors);
            ctx.push_scope();
            bind_pattern(pat, ctx);
            check_block(src, &body.stmts, ctx, errors);
            ctx.pop_scope();
        }
        Stmt::Match {
            scrutinee, arms, ..
        } => {
            check_expr(src, scrutinee, ctx, errors);
            for arm in arms {
                if let Some(g) = &arm.guard {
                    check_expr(src, g, ctx, errors);
                }
                ctx.push_scope();
                bind_pattern(&arm.pattern, ctx);
                check_expr(src, &arm.body, ctx, errors);
                ctx.pop_scope();
            }
        }
        Stmt::Return { value, span } => {
            if ctx.fn_depth == 0 {
                let (line, column) = line_col(src, span.start);
                errors.push(TypeError {
                    line,
                    column,
                    span: *span,
                    message: "`return` outside a function (E0080)".into(),
                    notes: Vec::new(),
                });
            }
            if let Some(e) = value {
                check_expr(src, e, ctx, errors);
            }
        }
        Stmt::WithConfig { body, .. } => {
            ctx.push_scope();
            check_block(src, &body.stmts, ctx, errors);
            ctx.pop_scope();
        }
        Stmt::Pub(inner) => check_stmt(src, inner, ctx, errors),
    }
}

/// Bind every name a pattern introduces (spec §4.4).
fn bind_pattern(pat: &Pattern, ctx: &mut NameCtx) {
    match pat {
        Pattern::Binding(n) => ctx.bind(&n.value, n.span),
        Pattern::Wildcard(_) => {}
        Pattern::Tuple(pats, _) | Pattern::Array(pats, _) | Pattern::Or(pats) => {
            for p in pats {
                bind_pattern(p, ctx);
            }
        }
        Pattern::Struct { fields, .. } => {
            for f in fields {
                if let Some(sub) = &f.pat {
                    bind_pattern(sub, ctx);
                }
            }
        }
        Pattern::Variant { args, .. } => {
            for a in args {
                bind_pattern(a, ctx);
            }
        }
        Pattern::Group(inner) => bind_pattern(inner, ctx),
        Pattern::Literal(_) | Pattern::Range { .. } => {}
    }
}

fn bind_params(params: &[Param], ctx: &mut NameCtx) {
    for p in params {
        if !p.is_self {
            ctx.bind(&p.name.value, p.name.span);
        }
        if let Some(t) = &p.type_ann {
            check_type(t);
        }
    }
}

/// Check an expression tree for name/`self`/`return` issues.
fn check_expr(src: &str, e: &Expr, ctx: &mut NameCtx, errors: &mut Vec<TypeError>) {
    match &e.kind {
        // A single-segment path is a variable reference (or a known root name).
        prima_syntax::ast::ExprKind::Path { segments } if segments.len() == 1 => {
            let name = segments[0].value.as_str();
            if !ctx.is_bound(name) {
                let (line, column) = line_col(src, e.span.start);
                errors.push(TypeError {
                    line,
                    column,
                    span: e.span,
                    message: format!("undefined name `{name}` (E0040)"),
                    notes: Vec::new(),
                });
            } else {
                ctx.mark_used(name);
            }
        }
        prima_syntax::ast::ExprKind::Symbol(s) => {
            let name = s.value.as_str();
            if !ctx.is_bound(name) {
                let (line, column) = line_col(src, s.span.start);
                errors.push(TypeError {
                    line,
                    column,
                    span: s.span,
                    message: format!("undefined name `{name}` (E0040)"),
                    notes: Vec::new(),
                });
            } else {
                ctx.mark_used(name);
            }
        }
        prima_syntax::ast::ExprKind::Self_ => {
            if ctx.fn_depth == 0 {
                let (line, column) = line_col(src, e.span.start);
                errors.push(TypeError {
                    line,
                    column,
                    span: e.span,
                    message: "`self` outside a method (E0062)".into(),
                    notes: Vec::new(),
                });
            }
        }
        _ => check_expr_children(src, e, ctx, errors),
    }
}

/// Descend the structural children of an expression for further name checks. Multi-segment paths
/// (`a::b`) and callable callees are treated as module/constructor references and not as variables.
fn check_expr_children(src: &str, e: &Expr, ctx: &mut NameCtx, errors: &mut Vec<TypeError>) {
    use prima_syntax::ast::ExprKind;
    match &e.kind {
        ExprKind::Path { .. } | ExprKind::Symbol(_) | ExprKind::Literal(_) | ExprKind::Self_ => {}
        ExprKind::FString(parts) => {
            for p in parts {
                if let FStringPart::Interp { expr, .. } = p {
                    check_expr(src, expr, ctx, errors);
                }
            }
        }
        ExprKind::Call { callee, args } => {
            check_expr(src, callee, ctx, errors);
            for a in args {
                check_expr(src, a, ctx, errors);
            }
        }
        ExprKind::MethodCall { receiver, args, .. } => {
            check_expr(src, receiver, ctx, errors);
            for a in args {
                check_expr(src, a, ctx, errors);
            }
        }
        ExprKind::Field { receiver, .. } => check_expr(src, receiver, ctx, errors),
        ExprKind::StructLiteral { fields, base, .. } => {
            for f in fields {
                if let Some(v) = &f.value {
                    check_expr(src, v, ctx, errors);
                }
            }
            if let Some(b) = base {
                check_expr(src, b, ctx, errors);
            }
        }
        ExprKind::Index { base, index } => {
            check_expr(src, base, ctx, errors);
            for it in &index.items {
                match it {
                    IndexItem::Elem(e) => check_expr(src, e, ctx, errors),
                    IndexItem::Slice { start, end } => {
                        if let Some(s) = start {
                            check_expr(src, s, ctx, errors);
                        }
                        if let Some(s) = end {
                            check_expr(src, s, ctx, errors);
                        }
                    }
                }
            }
        }
        ExprKind::Binary { lhs, rhs, .. } => {
            check_expr(src, lhs, ctx, errors);
            check_expr(src, rhs, ctx, errors);
        }
        ExprKind::Unary { operand, .. } | ExprKind::Try(operand) => {
            check_expr(src, operand, ctx, errors)
        }
        ExprKind::Array(items) | ExprKind::Tuple(items) | ExprKind::Set(items) => {
            for i in items {
                check_expr(src, i, ctx, errors);
            }
        }
        ExprKind::Dict(entries) => {
            for (k, v) in entries {
                check_expr(src, k, ctx, errors);
                check_expr(src, v, ctx, errors);
            }
        }
        ExprKind::KeyValue { key, value } => {
            check_expr(src, key, ctx, errors);
            check_expr(src, value, ctx, errors);
        }
        ExprKind::Comprehension {
            output, clauses, ..
        } => {
            ctx.push_scope();
            check_expr(src, output, ctx, errors);
            for c in clauses {
                match c {
                    ComprehensionClause::For { var, iter } => {
                        check_expr(src, iter, ctx, errors);
                        ctx.bind(&var.value, var.span);
                    }
                    ComprehensionClause::If { cond } => {
                        check_expr(src, cond, ctx, errors);
                    }
                }
            }
            ctx.pop_scope();
        }
        ExprKind::Lambda { params, body } => {
            ctx.push_scope();
            bind_params(params, ctx);
            check_expr(src, body, ctx, errors);
            ctx.pop_scope();
        }
        ExprKind::Match { scrutinee, arms } => {
            check_expr(src, scrutinee, ctx, errors);
            for arm in arms {
                if let Some(g) = &arm.guard {
                    check_expr(src, g, ctx, errors);
                }
                ctx.push_scope();
                bind_pattern(&arm.pattern, ctx);
                check_expr(src, &arm.body, ctx, errors);
                ctx.pop_scope();
            }
        }
        ExprKind::Custom(items) => {
            for (p, v) in items {
                check_expr(src, p, ctx, errors);
                check_expr(src, v, ctx, errors);
            }
        }
    }
}

/// Recursively descend a type to validate nested type references (currently a no-op hook; seeded
/// primitive `Type::User` names are not flagged since classes/modules have no symbol table here).
fn check_type(_t: &Type) {}
