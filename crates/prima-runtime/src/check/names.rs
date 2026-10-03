//! Lexical scope & name-resolution checks (spec §16.2 / appendix C compile-time errors).
//!
//! A lightweight scope-stack walk over the AST that detects statically-decidable name errors
//! without evaluating: `E0040 undefined_name` (a single-segment variable path used outside its
//! scope), `E0041 duplicate_definition` (a `fn`/`class`/`const`/math def, parameter, field or method
//! repeated in the same scope), `E0052 unknown_type` (a `Type::User` name not in scope),
//! `E0062 self_outside_method` (`self` outside a method), `E0063 self_not_first` (`self` not the
//! first method parameter), `E0080 return_outside_fn` (a `return` outside any function/method body),
//! and `W0003 unused_binding` (`let`-bound names never referenced in a later expression).
//!
//! It is deliberately conservative: only *single-segment* path/symbol references in value position
//! are considered variable uses, and the pre-imported `core` builtins plus the primitive type names
//! are seeded into the root scope, so defined-elsewhere symbols are not misreported as undefined.

use std::collections::HashSet;

use prima_syntax::ast::{
    ClassMemberKind, ComprehensionClause, Expr, FStringPart, ImportKind, IndexItem, Param, Pattern,
    Program, Spanned, Stmt, Type,
};
use prima_syntax::{Span, SyntaxWarning, parse};

use prima_core::suggest::did_you_mean_help;

use super::TypeError;
use super::error::push_err_with_help;

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
    "Value", "Any", "Error", "Nil", "Self", "SelfType", "Array", "Matrix", "Tuple", "Option",
    "Result", "Dict", "Set",
];

/// One lexical scope: `name → binding span` plus a manual "used" set; the used set is reset by the
/// check walk until a binding is referenced.
struct Scope {
    binds: Vec<(String, Span)>,
    /// Program declarations (`fn`/`class`/`const`/math def) recorded in this scope, for same-scope
    /// duplicate detection (spec §16.2 `E0041`). Seeded builtins and `let` bindings are excluded.
    defs: HashSet<String>,
    used: HashSet<String>,
}

impl Scope {
    fn new() -> Scope {
        Scope {
            binds: Vec::new(),
            defs: HashSet::new(),
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

    /// Record a program definition in this scope, returning `false` when the name was already
    /// defined here (a same-scope duplicate, spec §16.2 `E0041`). Shadowing an outer scope is fine.
    fn bind_definition(&mut self, name: &str, span: Span) -> bool {
        if !self.defs.insert(name.to_string()) {
            return false;
        }
        self.bind(name, span);
        true
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

    /// Define a program item in the current scope; `false` means a same-scope duplicate.
    fn define(&mut self, name: &str, span: Span) -> bool {
        match self.scopes.last_mut() {
            Some(s) => s.bind_definition(name, span),
            None => true,
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

    /// Every name visible from the current scope stack, used for `did you mean` suggestions on
    /// `E0040 undefined_name` (spec §16.4).
    fn candidate_names(&self) -> Vec<String> {
        let mut out = Vec::new();
        for s in &self.scopes {
            for (name, _) in &s.binds {
                if !out.contains(name) {
                    out.push(name.clone());
                }
            }
        }
        out
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
    let known = collect_known_types(program);
    check_block(src, &program.stmts, &mut ctx, errors, &known);
    for (name, span) in collect_unused(&ctx) {
        warnings.push(SyntaxWarning {
            span,
            code: "W0003",
            message: format!("unused binding: `{name}`"),
        });
    }
}

/// Report a same-scope duplicate definition (`E0041`) at the later definition's name span (spec §16.2).
fn define_name(ctx: &mut NameCtx, src: &str, name: &Spanned<String>, errors: &mut Vec<TypeError>) {
    if !ctx.define(&name.value, name.span) {
        push_err_with_help(
            src,
            errors,
            name.span,
            "E0041",
            format!("duplicate definition of `{}`", name.value),
            None,
            Some("rename one of the definitions".into()),
        );
    }
}

fn check_block(
    src: &str,
    stmts: &[Stmt],
    ctx: &mut NameCtx,
    errors: &mut Vec<TypeError>,
    known: &HashSet<String>,
) {
    for stmt in stmts {
        check_stmt(src, stmt, ctx, errors, known);
    }
}

fn check_stmt(
    src: &str,
    stmt: &Stmt,
    ctx: &mut NameCtx,
    errors: &mut Vec<TypeError>,
    known: &HashSet<String>,
) {
    match stmt {
        Stmt::Let {
            pat,
            type_ann,
            value,
            ..
        } => {
            if let Some(t) = type_ann {
                check_type(t, known, src, errors);
            }
            check_expr(src, value, ctx, errors, known);
            bind_pattern(pat, ctx);
        }
        Stmt::Const {
            name,
            type_ann,
            value,
            ..
        } => {
            check_type(type_ann, known, src, errors);
            check_expr(src, value, ctx, errors, known);
            define_name(ctx, src, name, errors);
        }
        Stmt::FnDef {
            name,
            params,
            ret,
            body,
            ..
        } => {
            define_name(ctx, src, name, errors);
            if let Some(t) = ret {
                check_type(t, known, src, errors);
            }
            ctx.push_scope();
            ctx.fn_depth += 1;
            bind_params(params, ctx, src, errors, known);
            check_block(src, &body.stmts, ctx, errors, known);
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
            define_name(ctx, src, name, errors);
            if let Some(t) = ret {
                check_type(t, known, src, errors);
            }
            ctx.push_scope();
            ctx.fn_depth += 1;
            bind_params(params, ctx, src, errors, known);
            check_expr(src, body, ctx, errors, known);
            ctx.fn_depth -= 1;
            ctx.pop_scope();
        }
        Stmt::ClassDef { name, members, .. } => {
            define_name(ctx, src, name, errors);
            // Duplicate field names and duplicate method names within one class are `E0041`
            // (spec §16.2). Fields and methods are *separate* namespaces: a field and an accessor
            // method may share a name (e.g. `total` field + `total(self)` method), so they are
            // tracked independently.
            let mut field_names: HashSet<String> = HashSet::new();
            let mut method_names: HashSet<String> = HashSet::new();
            for member in members {
                match &member.kind {
                    ClassMemberKind::Field {
                        name: fname, ty, ..
                    } => {
                        if !field_names.insert(fname.value.clone()) {
                            push_err_with_help(
                                src,
                                errors,
                                fname.span,
                                "E0041",
                                format!("duplicate definition of `{}`", fname.value),
                                None,
                                Some("rename one of the definitions".into()),
                            );
                        }
                        check_type(ty, known, src, errors);
                        ctx.bind(&fname.value, fname.span);
                    }
                    ClassMemberKind::Method {
                        name: mname,
                        params,
                        ret,
                        body,
                        ..
                    } => {
                        if !method_names.insert(mname.value.clone()) {
                            push_err_with_help(
                                src,
                                errors,
                                mname.span,
                                "E0041",
                                format!("duplicate definition of `{}`", mname.value),
                                None,
                                Some("rename one of the definitions".into()),
                            );
                        }
                        ctx.bind(&mname.value, mname.span);
                        check_self_position(src, params, errors);
                        if let Some(t) = ret {
                            check_type(t, known, src, errors);
                        }
                        ctx.push_scope();
                        ctx.fn_depth += 1;
                        ctx.bind("self", mname.span);
                        bind_params(params, ctx, src, errors, known);
                        if let Some(b) = body {
                            check_block(src, &b.stmts, ctx, errors, known);
                        }
                        ctx.fn_depth -= 1;
                        ctx.pop_scope();
                    }
                }
            }
        }
        Stmt::Impl { members, .. } => {
            for m in members {
                check_stmt(src, m, ctx, errors, known);
            }
        }
        Stmt::Expr(e) => check_expr(src, e, ctx, errors, known),
        Stmt::Assign { target, value, .. } => {
            check_expr(src, target, ctx, errors, known);
            check_expr(src, value, ctx, errors, known);
        }
        Stmt::For {
            var,
            range,
            step,
            body,
            ..
        } => {
            check_expr(src, &range.0, ctx, errors, known);
            check_expr(src, &range.1, ctx, errors, known);
            if let Some(s) = step {
                check_expr(src, s, ctx, errors, known);
            }
            ctx.push_scope();
            ctx.bind(&var.value, var.span);
            check_block(src, &body.stmts, ctx, errors, known);
            ctx.pop_scope();
        }
        Stmt::ParFor {
            var,
            range,
            step,
            body,
            ..
        } => {
            check_expr(src, &range.0, ctx, errors, known);
            check_expr(src, &range.1, ctx, errors, known);
            if let Some(s) = step {
                check_expr(src, s, ctx, errors, known);
            }
            ctx.push_scope();
            ctx.bind(&var.value, var.span);
            check_block(src, &body.stmts, ctx, errors, known);
            ctx.pop_scope();
        }
        Stmt::While { cond, body, .. } => {
            check_expr(src, cond, ctx, errors, known);
            ctx.push_scope();
            check_block(src, &body.stmts, ctx, errors, known);
            ctx.pop_scope();
        }
        Stmt::If {
            cond,
            then,
            elifs,
            else_,
            ..
        } => {
            check_expr(src, cond, ctx, errors, known);
            ctx.push_scope();
            check_block(src, &then.stmts, ctx, errors, known);
            ctx.pop_scope();
            for (c, b) in elifs {
                check_expr(src, c, ctx, errors, known);
                ctx.push_scope();
                check_block(src, &b.stmts, ctx, errors, known);
                ctx.pop_scope();
            }
            if let Some(b) = else_ {
                ctx.push_scope();
                check_block(src, &b.stmts, ctx, errors, known);
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
            check_expr(src, value, ctx, errors, known);
            ctx.push_scope();
            bind_pattern(pat, ctx);
            check_block(src, &then.stmts, ctx, errors, known);
            ctx.pop_scope();
            if let Some(b) = else_ {
                ctx.push_scope();
                check_block(src, &b.stmts, ctx, errors, known);
                ctx.pop_scope();
            }
        }
        Stmt::WhileLet {
            pat, value, body, ..
        } => {
            check_expr(src, value, ctx, errors, known);
            ctx.push_scope();
            bind_pattern(pat, ctx);
            check_block(src, &body.stmts, ctx, errors, known);
            ctx.pop_scope();
        }
        Stmt::Match {
            scrutinee, arms, ..
        } => {
            check_expr(src, scrutinee, ctx, errors, known);
            for arm in arms {
                if let Some(g) = &arm.guard {
                    check_expr(src, g, ctx, errors, known);
                }
                ctx.push_scope();
                bind_pattern(&arm.pattern, ctx);
                check_expr(src, &arm.body, ctx, errors, known);
                ctx.pop_scope();
            }
        }
        Stmt::Return { value, span } => {
            if ctx.fn_depth == 0 {
                push_err_with_help(
                    src,
                    errors,
                    *span,
                    "E0080",
                    "`return` outside a function".into(),
                    None,
                    Some("`return` is only valid inside a `fn` or method body".into()),
                );
            }
            if let Some(e) = value {
                check_expr(src, e, ctx, errors, known);
            }
        }
        Stmt::WithConfig { body, .. } => {
            ctx.push_scope();
            check_block(src, &body.stmts, ctx, errors, known);
            ctx.pop_scope();
        }
        Stmt::Pub(inner) => check_stmt(src, inner, ctx, errors, known),
    }
}

/// `E0063 self_not_first` (spec §4.5): a method's `self` receiver must be the first parameter.
fn check_self_position(src: &str, params: &[Param], errors: &mut Vec<TypeError>) {
    for (i, p) in params.iter().enumerate() {
        if p.is_self && i != 0 {
            push_err_with_help(
                src,
                errors,
                p.name.span,
                "E0063",
                "`self` must be the first parameter of a method".into(),
                None,
                None,
            );
        }
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

fn bind_params(
    params: &[Param],
    ctx: &mut NameCtx,
    src: &str,
    errors: &mut Vec<TypeError>,
    known: &HashSet<String>,
) {
    let mut seen: HashSet<String> = HashSet::new();
    for p in params {
        if !p.is_self {
            if !seen.insert(p.name.value.clone()) {
                push_err_with_help(
                    src,
                    errors,
                    p.name.span,
                    "E0041",
                    format!("duplicate definition of `{}`", p.name.value),
                    None,
                    Some("rename one of the definitions".into()),
                );
            }
            ctx.bind(&p.name.value, p.name.span);
        }
        if let Some(t) = &p.type_ann {
            check_type(t, known, src, errors);
        }
    }
}

/// Check an expression tree for name/`self`/`return` issues.
fn check_expr(
    src: &str,
    e: &Expr,
    ctx: &mut NameCtx,
    errors: &mut Vec<TypeError>,
    known: &HashSet<String>,
) {
    match &e.kind {
        // A single-segment path is a variable reference (or a known root name).
        prima_syntax::ast::ExprKind::Path { segments } if segments.len() == 1 => {
            let name = segments[0].value.as_str();
            if !ctx.is_bound(name) {
                let help = did_you_mean_help(name, ctx.candidate_names());
                push_err_with_help(
                    src,
                    errors,
                    e.span,
                    "E0040",
                    format!("undefined name `{name}`"),
                    None,
                    help,
                );
            } else {
                ctx.mark_used(name);
            }
        }
        prima_syntax::ast::ExprKind::Symbol(s) => {
            let name = s.value.as_str();
            if !ctx.is_bound(name) {
                let help = did_you_mean_help(name, ctx.candidate_names());
                push_err_with_help(
                    src,
                    errors,
                    s.span,
                    "E0040",
                    format!("undefined name `{name}`"),
                    None,
                    help,
                );
            } else {
                ctx.mark_used(name);
            }
        }
        prima_syntax::ast::ExprKind::Self_ => {
            if ctx.fn_depth == 0 {
                push_err_with_help(
                    src,
                    errors,
                    e.span,
                    "E0062",
                    "`self` outside a method".into(),
                    None,
                    Some("`self` is only available inside a class method".into()),
                );
            }
        }
        _ => check_expr_children(src, e, ctx, errors, known),
    }
}

/// Descend the structural children of an expression for further name checks. Multi-segment paths
/// (`a::b`) and callable callees are treated as module/constructor references and not as variables.
fn check_expr_children(
    src: &str,
    e: &Expr,
    ctx: &mut NameCtx,
    errors: &mut Vec<TypeError>,
    known: &HashSet<String>,
) {
    use prima_syntax::ast::ExprKind;
    match &e.kind {
        ExprKind::Path { .. } | ExprKind::Symbol(_) | ExprKind::Literal(_) | ExprKind::Self_ => {}
        ExprKind::FString(parts) => {
            for p in parts {
                if let FStringPart::Interp { expr, .. } = p {
                    check_expr(src, expr, ctx, errors, known);
                }
            }
        }
        ExprKind::Call { callee, args } => {
            check_expr(src, callee, ctx, errors, known);
            for a in args {
                check_expr(src, a, ctx, errors, known);
            }
        }
        ExprKind::MethodCall { receiver, args, .. } => {
            check_expr(src, receiver, ctx, errors, known);
            for a in args {
                check_expr(src, a, ctx, errors, known);
            }
        }
        ExprKind::Field { receiver, .. } => check_expr(src, receiver, ctx, errors, known),
        ExprKind::StructLiteral { fields, base, .. } => {
            for f in fields {
                if let Some(v) = &f.value {
                    check_expr(src, v, ctx, errors, known);
                }
            }
            if let Some(b) = base {
                check_expr(src, b, ctx, errors, known);
            }
        }
        ExprKind::Index { base, index } => {
            check_expr(src, base, ctx, errors, known);
            for it in &index.items {
                match it {
                    IndexItem::Elem(e) => check_expr(src, e, ctx, errors, known),
                    IndexItem::Slice { start, end } => {
                        if let Some(s) = start {
                            check_expr(src, s, ctx, errors, known);
                        }
                        if let Some(s) = end {
                            check_expr(src, s, ctx, errors, known);
                        }
                    }
                }
            }
        }
        ExprKind::Binary { lhs, rhs, .. } => {
            check_expr(src, lhs, ctx, errors, known);
            check_expr(src, rhs, ctx, errors, known);
        }
        ExprKind::Unary { operand, .. } | ExprKind::Try(operand) => {
            check_expr(src, operand, ctx, errors, known)
        }
        ExprKind::Array(items) | ExprKind::Tuple(items) | ExprKind::Set(items) => {
            for i in items {
                check_expr(src, i, ctx, errors, known);
            }
        }
        ExprKind::Dict(entries) => {
            for (k, v) in entries {
                check_expr(src, k, ctx, errors, known);
                check_expr(src, v, ctx, errors, known);
            }
        }
        ExprKind::KeyValue { key, value } => {
            check_expr(src, key, ctx, errors, known);
            check_expr(src, value, ctx, errors, known);
        }
        ExprKind::Comprehension {
            output, clauses, ..
        } => {
            ctx.push_scope();
            check_expr(src, output, ctx, errors, known);
            for c in clauses {
                match c {
                    ComprehensionClause::For { var, iter } => {
                        check_expr(src, iter, ctx, errors, known);
                        ctx.bind(&var.value, var.span);
                    }
                    ComprehensionClause::If { cond } => {
                        check_expr(src, cond, ctx, errors, known);
                    }
                }
            }
            ctx.pop_scope();
        }
        ExprKind::Lambda { params, body } => {
            ctx.push_scope();
            bind_params(params, ctx, src, errors, known);
            check_expr(src, body, ctx, errors, known);
            ctx.pop_scope();
        }
        ExprKind::Match { scrutinee, arms } => {
            check_expr(src, scrutinee, ctx, errors, known);
            for arm in arms {
                if let Some(g) = &arm.guard {
                    check_expr(src, g, ctx, errors, known);
                }
                ctx.push_scope();
                bind_pattern(&arm.pattern, ctx);
                check_expr(src, &arm.body, ctx, errors, known);
                ctx.pop_scope();
            }
        }
        ExprKind::Custom(items) => {
            for (p, v) in items {
                check_expr(src, p, ctx, errors, known);
                check_expr(src, v, ctx, errors, known);
            }
        }
    }
}

/// Collect the type names visible to `check_type` (spec §16.2 `E0052`): the primitive `TYPE_NAMES`,
/// every `class` declared in the program, and — for each embedded stdlib module the program imports
/// — its class names plus any `Type::User` names appearing in its signatures. This mirrors
/// `check/signature.rs::build_signature_table`.
fn collect_known_types(program: &Program) -> HashSet<String> {
    let mut known: HashSet<String> = TYPE_NAMES.iter().map(|s| (*s).to_string()).collect();
    for stmt in &program.stmts {
        collect_program_classes(stmt, &mut known);
    }
    for imp in &program.imports {
        let segments: Vec<String> = match &imp.kind {
            ImportKind::Namespace { path, .. } | ImportKind::From { path, .. } => {
                path.iter().map(|s| s.value.clone()).collect()
            }
        };
        let module_key = segments.join("::");
        let Some(src) = crate::stdlib::get_module_source(&module_key) else {
            continue;
        };
        // Embedded sources are ours and known-good; a parse failure just yields no type names.
        let Ok(parsed) = parse(src) else { continue };
        for stmt in &parsed.stmts {
            collect_module_type_names(stmt, &mut known);
        }
    }
    known
}

/// Collect the `class` names declared by the program (nested under `pub`), spec §16.2.
fn collect_program_classes(stmt: &Stmt, known: &mut HashSet<String>) {
    match stmt {
        Stmt::ClassDef { name, .. } => {
            known.insert(name.value.clone());
        }
        Stmt::Pub(inner) => collect_program_classes(inner, known),
        _ => {}
    }
}

/// Collect the `class` names and every `Type::User` name appearing in an embedded module's
/// signatures/members (spec §18.4), so imported stdlib types are not reported as unknown.
fn collect_module_type_names(stmt: &Stmt, known: &mut HashSet<String>) {
    match stmt {
        Stmt::Pub(inner) => collect_module_type_names(inner, known),
        Stmt::FnDef { name, params, ret, .. } | Stmt::MathDef { name, params, ret, .. } => {
            add_qualified_type_names(name, known);
            for p in params {
                if let Some(t) = &p.type_ann {
                    add_user_type_names(t, known);
                }
            }
            if let Some(t) = ret {
                add_user_type_names(t, known);
            }
        }
        Stmt::ClassDef { name, members, .. } => {
            known.insert(name.value.clone());
            for m in members {
                match &m.kind {
                    ClassMemberKind::Field { ty, .. } => add_user_type_names(ty, known),
                    ClassMemberKind::Method { params, ret, .. } => {
                        for p in params {
                            if let Some(t) = &p.type_ann {
                                add_user_type_names(t, known);
                            }
                        }
                        if let Some(t) = ret {
                            add_user_type_names(t, known);
                        }
                    }
                }
            }
        }
        _ => {}
    }
}

/// Add the qualifier of a `::`-joined declaration name (`Matrix::zeros` → `Matrix`,
/// `Duration::from_secs` → `Duration`) as a known type, so stdlib associated types are not `E0052`.
fn add_qualified_type_names(name: &Spanned<String>, known: &mut HashSet<String>) {
    if let Some((qualifier, _)) = name.value.split_once("::") {
        known.insert(qualifier.to_string());
    }
}

/// Insert every `Type::User` name reachable in a type into the known set.
fn add_user_type_names(t: &Type, known: &mut HashSet<String>) {
    match t {
        Type::User(sp) => {
            known.insert(sp.value.clone());
        }
        Type::Array(inner) | Type::Matrix(inner) | Type::Option(inner) => {
            add_user_type_names(inner, known);
        }
        Type::Tuple(ts) => {
            for x in ts {
                add_user_type_names(x, known);
            }
        }
        Type::Result(a, b) => {
            add_user_type_names(a, known);
            add_user_type_names(b, known);
        }
        Type::Fn { params, ret } | Type::MFn { params, ret } => {
            for p in params {
                add_user_type_names(p, known);
            }
            add_user_type_names(ret, known);
        }
        _ => {}
    }
}

/// Validate a type annotation (spec §16.2 `E0052`): recursively descend collection/function type
/// forms and report any `Type::User` name that is not in the known set. Module-qualified names
/// (`c_api::int`, appendix B.6) cannot be resolved here and are treated as known — a false positive
/// is worse than a missed check.
fn check_type(t: &Type, known: &HashSet<String>, src: &str, errors: &mut Vec<TypeError>) {
    match t {
        Type::User(sp) => {
            if sp.value.contains("::") || known.contains(&sp.value) {
                return;
            }
            let help = did_you_mean_help(&sp.value, known.iter());
            push_err_with_help(
                src,
                errors,
                sp.span,
                "E0052",
                format!("unknown type `{}`", sp.value),
                None,
                help,
            );
        }
        Type::Array(inner) | Type::Matrix(inner) | Type::Option(inner) => {
            check_type(inner, known, src, errors);
        }
        Type::Tuple(ts) => {
            for x in ts {
                check_type(x, known, src, errors);
            }
        }
        Type::Result(a, b) => {
            check_type(a, known, src, errors);
            check_type(b, known, src, errors);
        }
        Type::Fn { params, ret } | Type::MFn { params, ret } => {
            for p in params {
                check_type(p, known, src, errors);
            }
            check_type(ret, known, src, errors);
        }
        _ => {}
    }
}
