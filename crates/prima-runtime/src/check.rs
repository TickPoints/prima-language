use std::collections::HashMap;

use crate::builtins::Builtin;
use crate::capi::c_type;
use prima_syntax::ast::{Annotation, BinOp, ClassMemberKind, CompKind, Expr, ExprKind, ImportItem, ImportKind, Literal, MatchArm, Param, Pattern, Program, Spanned, Stmt, Type, UnOp};
use prima_syntax::parse;
use prima_syntax::Span;

/// A statically decidable type error (located via `--> file:line:col` per spec §16.4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeError {
    pub line: usize,
    pub column: usize,
    /// Source span of the offending value, for caret rendering (spec §16.4).
    pub span: Span,
    pub message: String,
}

/// A stdlib function signature harvested from an embedded `.pra` signature module (spec §18.4):
/// parameter types plus optional return type (`None` for void functions).
type Signature = (Vec<Type>, Option<Type>);

/// Static-check context: whether `?` (spec §16.3 `E0054`) is allowed — i.e. inside a `fn`/method
/// whose return type is `Result<..>`/`Option<..>`.
#[derive(Debug, Clone, Copy)]
struct Ctx {
    allow_try: bool,
}

/// Statically check source code (spec §6.3 / §16.2 compile-time errors): return all type errors (collecting, not fail-fast).
pub fn check_src(src: &str) -> Vec<TypeError> {
    let program = match parse(src) {
        Ok(program) => program,
        Err(errors) => {
            return errors
                .iter()
                .map(|e| {
                    let (line, column) = line_col(src, e.span.start);
                    TypeError { line, column, span: e.span, message: e.message.clone() }
                })
                .collect();
        }
    };

    let sigs = build_signature_table(&program);
    let mut errors = Vec::new();
    let ctx = Ctx { allow_try: false };
    for stmt in &program.stmts {
        collect_stmt_errors(src, stmt, &mut errors, ctx, false, &sigs);
    }
    // Statement order is source order; sorting stably by (line, column) keeps it consistent with span.start.
    errors.sort_by_key(|e| (e.line, e.column));
    errors
}

/// Collect the stdlib `@builtin pub fn` signatures reachable through the program's imports (spec
/// §15.4 import forms, §18.4 signatures). Keys are fully-qualified `"module::name"`; `from`
/// imports additionally expose the imported bare name (and any alias). Flattened `::`-joined item
/// names (e.g. `Matrix::zeros`) are keyed under the joined module path, mirroring how the runtime
/// registers and resolves them (`module.rs`/`eval.rs` `lookup_module_item_flat`).
fn build_signature_table(program: &Program) -> HashMap<String, Signature> {
    let mut table = HashMap::new();
    for imp in &program.imports {
        let segments: Vec<String> = match &imp.kind {
            ImportKind::Namespace { path, .. } | ImportKind::From { path, .. } => {
                path.iter().map(|s| s.value.clone()).collect()
            }
        };
        let module_key = segments.join("::");
        let Some(src) = crate::stdlib::get_module_source(&module_key) else { continue };
        // Embedded sources are ours and known-good; a parse failure just yields no signatures.
        let Ok(parsed) = parse(src) else { continue };
        let mut sigs = Vec::new();
        for stmt in &parsed.stmts {
            let Stmt::Pub(inner) = stmt else { continue };
            let Stmt::FnDef { name, params, ret, .. } = inner.as_ref() else { continue };
            let param_types = params
                .iter()
                .map(|p| {
                    p.type_ann.clone().unwrap_or_else(|| {
                        Type::User(Spanned { value: "Value".into(), span: p.name.span })
                    })
                })
                .collect();
            sigs.push((name.value.clone(), (param_types, ret.clone())));
        }
        for (name, sig) in &sigs {
            table.insert(format!("{module_key}::{name}"), sig.clone());
        }
        match &imp.kind {
            ImportKind::From { items, .. } => {
                for (name, sig) in &sigs {
                    for item in items {
                        if let ImportItem::Name { name: item_name, alias } = item
                            && item_name.value == *name
                        {
                            // Bind exactly what the runtime binds (eval.rs `bind_imports`): the
                            // alias when present, else the item name.
                            let target = alias
                                .as_ref()
                                .map_or_else(|| item_name.value.clone(), |a| a.value.clone());
                            table.entry(target).or_insert_with(|| sig.clone());
                        }
                    }
                }
            }
            ImportKind::Namespace { alias, .. } => {
                if let Some(a) = alias {
                    for (name, sig) in &sigs {
                        table.insert(format!("{}::{name}", a.value), sig.clone());
                    }
                }
            }
        }
    }
    table
}

/// Look up the signature for a `Path` callee, mirroring the runtime's flattened module-item lookup
/// (`eval.rs` `lookup_module_item_flat`): the joined segments first, then every module prefix.
fn lookup_call_signature<'a>(segments: &[Spanned<String>], sigs: &'a HashMap<String, Signature>) -> Option<&'a Signature> {
    if segments.is_empty() {
        return None;
    }
    let joined = segments.iter().map(|s| s.value.as_str()).collect::<Vec<_>>().join("::");
    if let Some(sig) = sigs.get(&joined) {
        return Some(sig);
    }
    for i in 1..segments.len() - 1 {
        let key = format!(
            "{}::{}",
            segments[..i].iter().map(|s| s.value.as_str()).collect::<Vec<_>>().join("::"),
            segments[i..].iter().map(|s| s.value.as_str()).collect::<Vec<_>>().join("::")
        );
        if let Some(sig) = sigs.get(&key) {
            return Some(sig);
        }
    }
    None
}

/// Check a call against the harvested stdlib signature (spec §18.4, §16.2 `E0050`): positive arity
/// and per-argument type mismatches only — unknown/unresolved types never error.
fn check_call_signature(
    src: &str,
    call_span: Span,
    segments: &[Spanned<String>],
    args: &[Expr],
    errors: &mut Vec<TypeError>,
    sigs: &HashMap<String, Signature>,
) {
    let Some((params, _)) = lookup_call_signature(segments, sigs) else { return };
    let name = segments.iter().map(|s| s.value.as_str()).collect::<Vec<_>>().join("::");
    if args.len() > params.len() {
        push_err(
            src,
            errors,
            call_span,
            format!("function `{name}` expects {} argument(s), got {} (E0050)", params.len(), args.len()),
        );
    }
    for (i, arg) in args.iter().enumerate().take(params.len()) {
        let got = infer(arg, sigs);
        if !assignable(&params[i], &got) {
            push_err(
                src,
                errors,
                arg.span,
                format!("argument {} of `{name}` expects {}, got {} (E0050)", i + 1, type_name(&params[i]), got),
            );
        }
    }
}

/// Collect static errors for one statement. Only the type annotations of `let`/`const` (with a
/// plain binding pattern) are checked; all bodies are descended into to catch `?` misuse and to
/// validate stdlib call sites. `is_pub` records whether the statement is wrapped in `Stmt::Pub`
/// (spec §15.2), required by `@c_api::extern` exports (spec §18.4, E0072).
fn collect_stmt_errors(
    src: &str,
    stmt: &Stmt,
    errors: &mut Vec<TypeError>,
    ctx: Ctx,
    is_pub: bool,
    sigs: &HashMap<String, Signature>,
) {
    match stmt {
        Stmt::Let { pat, type_ann, value, span, .. } => {
            // `let` rejects refutable patterns (spec §4.4 `E0053`).
            if pattern_is_refutable(pat) {
                let (line, column) = line_col(src, span.start);
                errors.push(TypeError {
                    line,
                    column,
                    span: *span,
                    message: "refutable pattern in `let` (E0053)".into(),
                });
            }
            if let Some(t) = type_ann {
                let inf = infer(value, sigs);
                if !annot_accepts(t, &inf) {
                    let (line, column) = line_col(src, value.span.start);
                    errors.push(TypeError {
                        line,
                        column,
                        span: value.span,
                        message: format!("type mismatch: expected {}, got {}", annot_name(t), inf),
                    });
                }
            }
            collect_expr_errors(src, value, errors, ctx, sigs);
        }
        Stmt::Const { type_ann: t, value, .. } => {
            let inf = infer(value, sigs);
            if !annot_accepts(t, &inf) {
                let (line, column) = line_col(src, value.span.start);
                errors.push(TypeError {
                    line,
                    column,
                    span: value.span,
                    message: format!("type mismatch: expected {}, got {}", annot_name(t), inf),
                });
            }
            collect_expr_errors(src, value, errors, ctx, sigs);
        }
        Stmt::FnDef { name, params, ret, annotations, body, .. } => {
            errors.extend(check_annotation_errors(src, name, params, ret, annotations, !body.stmts.is_empty(), is_pub));
            let allow = matches!(ret, Some(Type::Result(..) | Type::Option(..)));
            collect_block_errors(src, body, errors, Ctx { allow_try: allow }, sigs);
        }
        Stmt::MathDef { body, .. } => {
            collect_expr_errors(src, body, errors, Ctx { allow_try: false }, sigs);
        }
        Stmt::ClassDef { name, annotations, members, .. } => {
            // Only the builtin `String` class is meaningful, and it is implicit — never declared in
            // source — so any user `@builtin class` has no registered implementation (spec §18.4, E0055).
            if annotations.contains(&Annotation::Builtin) && name.value != "String" {
                push_err(src, errors, name.span, format!("unregistered `@builtin` class `{}` (E0055)", name.value));
            }
            for m in members {
                if let ClassMemberKind::Method { ret, body: Some(b), .. } = &m.kind {
                    let allow = matches!(ret, Some(Type::Result(..) | Type::Option(..)));
                    collect_block_errors(src, b, errors, Ctx { allow_try: allow }, sigs);
                }
            }
        }
        Stmt::Impl { members, .. } => {
            for m in members {
                collect_stmt_errors(src, m, errors, ctx, false, sigs);
            }
        }
        Stmt::IfLet { value, then, else_, .. } => {
            collect_expr_errors(src, value, errors, ctx, sigs);
            collect_block_errors(src, then, errors, ctx, sigs);
            if let Some(b) = else_ {
                collect_block_errors(src, b, errors, ctx, sigs);
            }
        }
        Stmt::WhileLet { value, body, .. } => {
            collect_expr_errors(src, value, errors, ctx, sigs);
            collect_block_errors(src, body, errors, ctx, sigs);
        }
        Stmt::Match { scrutinee, arms, .. } => {
            collect_expr_errors(src, scrutinee, errors, ctx, sigs);
            collect_arms_errors(src, arms, errors, ctx, sigs);
        }
        Stmt::If { cond, then, elifs, else_, .. } => {
            collect_expr_errors(src, cond, errors, ctx, sigs);
            collect_block_errors(src, then, errors, ctx, sigs);
            for (c, b) in elifs {
                collect_expr_errors(src, c, errors, ctx, sigs);
                collect_block_errors(src, b, errors, ctx, sigs);
            }
            if let Some(b) = else_ {
                collect_block_errors(src, b, errors, ctx, sigs);
            }
        }
        Stmt::While { cond, body, .. } => {
            collect_expr_errors(src, cond, errors, ctx, sigs);
            collect_block_errors(src, body, errors, ctx, sigs);
        }
        Stmt::For { range, step, body, .. } | Stmt::ParFor { range, step, body, .. } => {
            collect_expr_errors(src, &range.0, errors, ctx, sigs);
            collect_expr_errors(src, &range.1, errors, ctx, sigs);
            if let Some(s) = step {
                collect_expr_errors(src, s, errors, ctx, sigs);
            }
            collect_block_errors(src, body, errors, ctx, sigs);
        }
        Stmt::Return { value, .. } => {
            if let Some(e) = value {
                collect_expr_errors(src, e, errors, ctx, sigs);
            }
        }
        Stmt::Assign { target, value, .. } => {
            collect_expr_errors(src, target, errors, ctx, sigs);
            collect_expr_errors(src, value, errors, ctx, sigs);
        }
        Stmt::WithConfig { entries, body, .. } => {
            for e in entries {
                collect_expr_errors(src, &e.value, errors, ctx, sigs);
            }
            collect_block_errors(src, body, errors, ctx, sigs);
        }
        Stmt::Expr(e) => collect_expr_errors(src, e, errors, ctx, sigs),
        Stmt::Pub(inner) => collect_stmt_errors(src, inner, errors, ctx, true, sigs),
    }
}

fn collect_block_errors(src: &str, block: &prima_syntax::ast::Block, errors: &mut Vec<TypeError>, ctx: Ctx, sigs: &HashMap<String, Signature>) {
    for s in &block.stmts {
        collect_stmt_errors(src, s, errors, ctx, false, sigs);
    }
}

fn collect_arms_errors(src: &str, arms: &[MatchArm], errors: &mut Vec<TypeError>, ctx: Ctx, sigs: &HashMap<String, Signature>) {
    for arm in arms {
        if let Some(g) = &arm.guard {
            collect_expr_errors(src, g, errors, ctx, sigs);
        }
        collect_expr_errors(src, &arm.body, errors, ctx, sigs);
    }
}

/// Annotation validation (spec §18.4), returning the errors for one `fn`:
/// - `@builtin`: signature-only and the name must name a registered host builtin (E0056/E0055).
/// - `@c_api::extern`: `pub` and `c_api::*` C-compatible parameter/return types (E0072/E0071).
fn check_annotation_errors(
    src: &str,
    name: &Spanned<String>,
    params: &[Param],
    ret: &Option<Type>,
    annotations: &[Annotation],
    has_body: bool,
    is_pub: bool,
) -> Vec<TypeError> {
    let mut errors = Vec::new();
    if annotations.contains(&Annotation::Builtin) {
        if has_body {
            push_err(src, &mut errors, name.span, "`@builtin` function must not have a body (E0056)".into());
        } else if Builtin::from_name(&name.value).is_none() {
            push_err(src, &mut errors, name.span, format!("unregistered `@builtin` function `{}` (E0055)", name.value));
        }
    }
    if annotations.contains(&Annotation::CApiExtern) {
        if !is_pub {
            push_err(src, &mut errors, name.span, "`@c_api::extern` function must be `pub` (E0072)".into());
        }
        for p in params {
            let ok = p.type_ann.as_ref().is_some_and(c_param_ok);
            if !ok {
                let ty = p.type_ann.as_ref().map_or_else(|| p.name.value.clone(), type_display);
                let sp = p.type_ann.as_ref().map_or(p.name.span, |t| type_span(t, p.name.span));
                push_err(src, &mut errors, sp, format!("`@c_api::extern` parameter/return type `{ty}` is not C-compatible (E0071)"));
            }
        }
        if let Some(t) = ret
            && c_type(t).is_none()
        {
            push_err(src, &mut errors, type_span(t, name.span), format!("`@c_api::extern` parameter/return type `{}` is not C-compatible (E0071)", type_display(t)));
        }
    }
    errors
}

/// Whether a type is allowed as a `@c_api::extern` parameter (spec appendix B.6): a C-compatible
/// type other than `c_api::unit` (`void` is only a return type).
fn c_param_ok(t: &Type) -> bool {
    matches!(c_type(t).as_deref(), Some(c) if c != "void")
}

/// Human-readable type name for diagnostics (`User` carries its source text).
fn type_display(t: &Type) -> String {
    match t {
        Type::User(sp) => sp.value.clone(),
        _ => annot_name(t).into(),
    }
}

/// Span of a type annotation for caret rendering; non-`User` types have no span, so a fallback is used.
fn type_span(t: &Type, fallback: Span) -> Span {
    match t {
        Type::User(sp) => sp.span,
        _ => fallback,
    }
}

/// Push a located error, deriving line/column from the span (spec §16.4).
fn push_err(src: &str, errors: &mut Vec<TypeError>, span: Span, message: String) {
    let (line, column) = line_col(src, span.start);
    errors.push(TypeError { line, column, span, message });
}

/// Descend an expression tree, flagging `?` outside a `Result`/`Option`-returning function (spec
/// §16.3 `E0054`) and validating stdlib call sites against harvested signatures (spec §18.4).
fn collect_expr_errors(src: &str, expr: &Expr, errors: &mut Vec<TypeError>, ctx: Ctx, sigs: &HashMap<String, Signature>) {
    match &expr.kind {
        ExprKind::Try(inner) => {
            if !ctx.allow_try {
                let (line, column) = line_col(src, expr.span.start);
                errors.push(TypeError {
                    line,
                    column,
                    span: expr.span,
                    message: "`?` can only be used inside a function returning `Result`/`Option` (E0054)".into(),
                });
            }
            collect_expr_errors(src, inner, errors, ctx, sigs);
        }
        ExprKind::Call { callee, args } => {
            if let ExprKind::Path { segments } = &callee.kind {
                check_call_signature(src, expr.span, segments, args, errors, sigs);
            }
            collect_expr_errors(src, callee, errors, ctx, sigs);
            for a in args {
                collect_expr_errors(src, a, errors, ctx, sigs);
            }
        }
        ExprKind::MethodCall { receiver, args, .. } => {
            collect_expr_errors(src, receiver, errors, ctx, sigs);
            for a in args {
                collect_expr_errors(src, a, errors, ctx, sigs);
            }
        }
        ExprKind::Field { receiver, .. } => collect_expr_errors(src, receiver, errors, ctx, sigs),
        ExprKind::StructLiteral { fields, base, .. } => {
            for f in fields {
                if let Some(v) = &f.value {
                    collect_expr_errors(src, v, errors, ctx, sigs);
                }
            }
            if let Some(b) = base {
                collect_expr_errors(src, b, errors, ctx, sigs);
            }
        }
        ExprKind::Index { base, index } => {
            collect_expr_errors(src, base, errors, ctx, sigs);
            for item in &index.items {
                match item {
                    prima_syntax::ast::IndexItem::Elem(e) => collect_expr_errors(src, e, errors, ctx, sigs),
                    prima_syntax::ast::IndexItem::Slice { start, end } => {
                        if let Some(s) = start {
                            collect_expr_errors(src, s, errors, ctx, sigs);
                        }
                        if let Some(e) = end {
                            collect_expr_errors(src, e, errors, ctx, sigs);
                        }
                    }
                }
            }
        }
        ExprKind::Binary { lhs, rhs, .. } => {
            collect_expr_errors(src, lhs, errors, ctx, sigs);
            collect_expr_errors(src, rhs, errors, ctx, sigs);
        }
        ExprKind::Unary { operand, .. } => collect_expr_errors(src, operand, errors, ctx, sigs),
        ExprKind::Array(items) | ExprKind::Tuple(items) | ExprKind::Set(items) => {
            for i in items {
                collect_expr_errors(src, i, errors, ctx, sigs);
            }
        }
        ExprKind::Dict(entries) => {
            for (k, v) in entries {
                collect_expr_errors(src, k, errors, ctx, sigs);
                collect_expr_errors(src, v, errors, ctx, sigs);
            }
        }
        ExprKind::Comprehension { output, clauses, .. } => {
            collect_expr_errors(src, output, errors, ctx, sigs);
            for c in clauses {
                match c {
                    prima_syntax::ast::ComprehensionClause::For { iter, .. } => {
                        collect_expr_errors(src, iter, errors, ctx, sigs);
                    }
                    prima_syntax::ast::ComprehensionClause::If { cond } => {
                        collect_expr_errors(src, cond, errors, ctx, sigs);
                    }
                }
            }
        }
        ExprKind::KeyValue { key, value } => {
            collect_expr_errors(src, key, errors, ctx, sigs);
            collect_expr_errors(src, value, errors, ctx, sigs);
        }
        ExprKind::Lambda { body, .. } => collect_expr_errors(src, body, errors, ctx, sigs),
        ExprKind::Match { scrutinee, arms } => {
            collect_expr_errors(src, scrutinee, errors, ctx, sigs);
            collect_arms_errors(src, arms, errors, ctx, sigs);
        }
        ExprKind::Pipeline { lhs, rhs } => {
            collect_expr_errors(src, lhs, errors, ctx, sigs);
            collect_expr_errors(src, rhs, errors, ctx, sigs);
        }
        ExprKind::Custom(items) => {
            for (p, v) in items {
                collect_expr_errors(src, p, errors, ctx, sigs);
                collect_expr_errors(src, v, errors, ctx, sigs);
            }
        }
        _ => {}
    }
}

/// Refutable-pattern check for `let` (spec §4.4): only bindings, wildcards and grouped tuples/arrays
/// of irrefutable patterns are irrefutable.
fn pattern_is_refutable(p: &Pattern) -> bool {
    match p {
        Pattern::Wildcard(_) | Pattern::Binding(_) => false,
        Pattern::Tuple(pats, _) | Pattern::Array(pats, _) => pats.iter().any(pattern_is_refutable),
        Pattern::Group(inner) => pattern_is_refutable(inner),
        Pattern::Or(pats) => pats.iter().any(pattern_is_refutable),
        _ => true,
    }
}

/// Static type name of a literal/simple expression (spec §6.3 literal type inference). Returns
/// `"unknown"` for anything not statically decidable; stdlib calls resolve through the harvested
/// signature table. `"Expr"` is the symbolic catch-all for builtin math functions.
fn infer(expr: &Expr, sigs: &HashMap<String, Signature>) -> String {
    match &expr.kind {
        ExprKind::Literal(lit) => match lit {
            Literal::Integer(_) | Literal::Hex(_) | Literal::Binary(_) => "Integer".into(),
            Literal::Float(_) => "F64".into(),
            Literal::Bool(_) => "Bool".into(),
            Literal::Str(_) => "String".into(),
            Literal::Char(_) => "Char".into(),
            Literal::Tex(_) => "Expr".into(),
        },
        ExprKind::Symbol(_) => "Expr".into(),
        ExprKind::Call { callee, .. } => {
            if let ExprKind::Path { segments } = &callee.kind {
                if let Some((_, ret)) = lookup_call_signature(segments, sigs) {
                    return match ret {
                        Some(t) => type_name(t),
                        None => "unit".into(),
                    };
                }
                if segments.len() == 1 {
                    return match segments[0].value.as_str() {
                        "to_f64" => "F64".into(),
                        "to_f32" => "F32".into(),
                        "to_i8" => "I8".into(),
                        "to_i16" => "I16".into(),
                        "to_i32" => "I32".into(),
                        "to_i64" => "I64".into(),
                        "to_i128" => "I128".into(),
                        "to_u8" => "U8".into(),
                        "to_u16" => "U16".into(),
                        "to_u32" => "U32".into(),
                        "to_u64" => "U64".into(),
                        "to_u128" => "U128".into(),
                        "to_isize" => "Isize".into(),
                        "to_usize" => "Usize".into(),
                        "to_bigint" => "Integer".into(),
                        "to_rational" => "Rational".into(),
                        "to_complex" => "Complex".into(),
                        "print" | "println" => "Nil".into(),
                        name if name.starts_with("try_") || name.starts_with("checked_") => "Result".into(),
                        "Some" | "None" => "Option".into(),
                        "Ok" | "Err" => "Result".into(),
                        "get" => "Option".into(),
                        // Unknown calls (symbolic builtins, user functions) stay symbolic `Expr`.
                        _ => "Expr".into(),
                    };
                }
            }
            "unknown".into()
        }
        ExprKind::Unary { op: UnOp::Neg | UnOp::Pos, operand } => infer(operand, sigs),
        ExprKind::Unary { op: UnOp::Not, .. } => "Bool".into(),
        ExprKind::Try(inner) => infer(inner, sigs),
        ExprKind::Binary { op, lhs, rhs } => match op {
            BinOp::Add | BinOp::Sub | BinOp::Mul => {
                let l = infer(lhs, sigs);
                let r = infer(rhs, sigs);
                combine_numeric(&l, &r).into()
            }
            BinOp::Div => {
                let l = infer(lhs, sigs);
                let r = infer(rhs, sigs);
                let exact = |s: &str| matches!(s, "Integer" | "Rational" | "Complex");
                if exact(&l) && exact(&r) {
                    "Rational".into()
                } else {
                    "F64".into()
                }
            }
            _ => "Expr".into(),
        },
        ExprKind::Array(items) => {
            if items.is_empty() {
                return "array".into();
            }
            let first = infer(&items[0], sigs);
            if items.iter().all(|i| infer(i, sigs) == first) {
                if first == "unknown" {
                    "array".into()
                } else {
                    format!("Array<{first}>")
                }
            } else {
                "array".into()
            }
        }
        ExprKind::Dict(_) => "dict".into(),
        ExprKind::Set(_) => "set".into(),
        ExprKind::Tuple(_) => "tuple".into(),
        ExprKind::Comprehension { kind, .. } => match kind {
            CompKind::Array => "Array".into(),
            CompKind::Dict => "Dict".into(),
            CompKind::Set => "Set".into(),
            CompKind::Tuple => "Tuple".into(),
        },
        ExprKind::KeyValue { .. } => "value".into(),
        ExprKind::Lambda { .. } => "Fn".into(),
        _ => "unknown".into(),
    }
}

/// Promotion for numeric binary operations: symbolic contagion → float contagion → rational → integer (spec §8.1 promotion sequence).
fn combine_numeric(l: &str, r: &str) -> &'static str {
    if l == "Expr" || r == "Expr" {
        "Expr"
    } else if matches!(l, "F64" | "F32") || matches!(r, "F64" | "F32") {
        "F64"
    } else if l == "Rational" || r == "Rational" {
        "Rational"
    } else {
        "Integer"
    }
}

/// Whether the annotation accepts the inferred type (exact-layer constraints decidable at compile time only).
fn annot_accepts(t: &Type, inf: &str) -> bool {
    match t {
        Type::Integer => matches!(inf, "Integer" | "Rational" | "Complex"),
        Type::Rational => matches!(inf, "Integer" | "Rational"),
        Type::F64 => matches!(inf, "Integer" | "Rational" | "F64" | "F32"),
        Type::F32 => matches!(inf, "Integer" | "Rational" | "F32"),
        Type::I8 => inf == "I8",
        Type::I16 => inf == "I16",
        Type::I32 => inf == "I32",
        Type::I64 => inf == "I64",
        Type::I128 => inf == "I128",
        Type::U8 => inf == "U8",
        Type::U16 => inf == "U16",
        Type::U32 => inf == "U32",
        Type::U64 => inf == "U64",
        Type::U128 => inf == "U128",
        Type::Isize => inf == "Isize",
        Type::Usize => inf == "Usize",
        Type::Complex => matches!(inf, "Integer" | "Rational" | "Complex"),
        Type::Number => matches!(inf, "Integer" | "Rational" | "F64" | "F32" | "Complex"),
        Type::Expr | Type::Symbol => true,
        Type::Bool => inf == "Bool",
        Type::String => inf == "String",
        Type::Char => inf == "Char",
        Type::Array(_) => inf == "Array" || inf == "array" || inf.starts_with("Array<"),
        Type::Tuple(_) => inf == "Tuple" || inf == "tuple" || inf.starts_with("Tuple<"),
        Type::Option(_) => inf == "Option" || inf == "option" || inf.starts_with("Option<"),
        Type::Result(..) => inf == "Result" || inf == "result" || inf.starts_with("Result<"),
        Type::SelfType => true,
        Type::Fn { .. } | Type::MFn { .. } | Type::User(_) | Type::Matrix(_) => true,
    }
}

/// Whether a stdlib parameter type accepts an inferred argument type (conservative; spec §6.3
/// implicit promotion, §18.4 call-site checking). Unknown inferred types never reject.
fn assignable(param: &Type, got: &str) -> bool {
    // Never error on an undeclared/unsolvable type — false positives are worse than missed checks.
    if got == "unknown" || got == "Expr" {
        return true;
    }
    match param {
        Type::User(sp) => match sp.value.as_str() {
            // Wildcard: any value is accepted (spec §6.3 `Value`/`Any`).
            "Value" | "Any" => true,
            other => got == other,
        },
        Type::Number => matches!(got, "Integer" | "Rational" | "F64" | "F32" | "Complex"),
        Type::F64 => matches!(got, "Integer" | "Rational" | "F32" | "F64"),
        Type::Rational => matches!(got, "Integer" | "Rational"),
        Type::Integer => got == "Integer",
        Type::Complex => matches!(got, "Integer" | "Rational" | "Complex"),
        Type::Array(inner) => {
            if got == "array" {
                true
            } else if let Some(inner_got) = collection_inner(got, "Array") {
                assignable(inner, inner_got)
            } else {
                false
            }
        }
        Type::Matrix(inner) => {
            // A `Matrix<…>` value, or a 2D nested array literal `Array<Array<…>>` (spec B.2).
            if let Some(inner_got) = collection_inner(got, "Matrix") {
                assignable(inner, inner_got)
            } else if let Some(level2) = collection_inner(got, "Array")
                && let Some(elem) = collection_inner(level2, "Array")
            {
                assignable(inner, elem)
            } else {
                false
            }
        }
        Type::Option(_) => got == "Option" || got == "option" || got.starts_with("Option<"),
        Type::Result(_, _) => got == "Result" || got == "result" || got.starts_with("Result<"),
        Type::Tuple(_) => got == "Tuple" || got == "tuple" || got.starts_with("Tuple<"),
        base => got == type_name(base),
    }
}

/// Extract the balanced inner content of a rendered collection type string, e.g. the `Integer` in
/// `Array<Array<Integer>>` when given `name = "Array"`. `None` if the string is not `name<…>`.
fn collection_inner<'a>(s: &'a str, name: &str) -> Option<&'a str> {
    let prefix = format!("{name}<");
    let rest = s.strip_prefix(&prefix)?;
    let mut depth = 0i32;
    for (i, ch) in rest.char_indices() {
        match ch {
            '<' => depth += 1,
            '>' => {
                if depth == 0 {
                    return Some(&rest[..i]);
                }
                depth -= 1;
            }
            _ => {}
        }
    }
    None
}

/// Render a type as its source-level name (spec §6.3); `Type::User("Value")` is a wildcard.
fn type_name(t: &Type) -> String {
    match t {
        Type::Number => "Number".into(),
        Type::Integer => "Integer".into(),
        Type::Rational => "Rational".into(),
        Type::F64 => "F64".into(),
        Type::F32 => "F32".into(),
        Type::I8 => "I8".into(),
        Type::I16 => "I16".into(),
        Type::I32 => "I32".into(),
        Type::I64 => "I64".into(),
        Type::I128 => "I128".into(),
        Type::U8 => "U8".into(),
        Type::U16 => "U16".into(),
        Type::U32 => "U32".into(),
        Type::U64 => "U64".into(),
        Type::U128 => "U128".into(),
        Type::Isize => "Isize".into(),
        Type::Usize => "Usize".into(),
        Type::Complex => "Complex".into(),
        Type::Expr => "Expr".into(),
        Type::Symbol => "Symbol".into(),
        Type::Bool => "Bool".into(),
        Type::String => "String".into(),
        Type::Char => "Char".into(),
        Type::Array(inner) => format!("Array<{}>", type_name(inner)),
        Type::Matrix(inner) => format!("Matrix<{}>", type_name(inner)),
        Type::Tuple(ts) => format!("Tuple<{}>", ts.iter().map(type_name).collect::<Vec<_>>().join(", ")),
        Type::Option(inner) => format!("Option<{}>", type_name(inner)),
        Type::Result(a, b) => format!("Result<{}, {}>", type_name(a), type_name(b)),
        Type::Fn { params, ret } => format!(
            "Fn({}) -> {}",
            params.iter().map(type_name).collect::<Vec<_>>().join(", "),
            type_name(ret)
        ),
        Type::MFn { params, ret } => format!(
            "MFn({}) -> {}",
            params.iter().map(type_name).collect::<Vec<_>>().join(", "),
            type_name(ret)
        ),
        Type::SelfType => "Self".into(),
        Type::User(sp) => {
            if sp.value == "Value" {
                "unknown".into()
            } else {
                sp.value.clone()
            }
        }
    }
}

/// Name of the annotated type (for error messages, using the `Type` enum variant name).
fn annot_name(t: &Type) -> &'static str {
    match t {
        Type::Number => "Number",
        Type::Integer => "Integer",
        Type::Rational => "Rational",
        Type::F64 => "F64",
        Type::F32 => "F32",
        Type::I8 => "I8",
        Type::I16 => "I16",
        Type::I32 => "I32",
        Type::I64 => "I64",
        Type::I128 => "I128",
        Type::U8 => "U8",
        Type::U16 => "U16",
        Type::U32 => "U32",
        Type::U64 => "U64",
        Type::U128 => "U128",
        Type::Isize => "Isize",
        Type::Usize => "Usize",
        Type::Complex => "Complex",
        Type::Expr => "Expr",
        Type::Symbol => "Symbol",
        Type::Bool => "Bool",
        Type::String => "String",
        Type::Char => "Char",
        Type::Array(_) => "Array",
        Type::Matrix(_) => "Matrix",
        Type::Tuple(_) => "Tuple",
        Type::Option(_) => "Option",
        Type::Result(..) => "Result",
        Type::SelfType => "Self",
        Type::Fn { .. } => "Fn",
        Type::MFn { .. } => "MFn",
        Type::User(_) => "User",
    }
}

/// Byte offset → 1-based line/column (column counted in characters, spec §16.4 location).
fn line_col(src: &str, offset: u32) -> (usize, usize) {
    let offset = usize::try_from(offset).unwrap_or(usize::MAX).min(src.len());
    let before = &src[..offset];
    let line = before.bytes().filter(|&b| b == b'\n').count() + 1;
    let line_start = before.rfind('\n').map_or(0, |i| i + 1);
    let column = src[line_start..offset].chars().count() + 1;
    (line, column)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn f64_annotation_rejects_symbolic_value() {
        let errs = check_src("let x: F64 = sqrt(2);");
        assert_eq!(errs.len(), 1);
        assert!(errs[0].message.contains("F64"));
        assert!(errs[0].message.contains("Expr"));
    }

    #[test]
    fn explicit_conversion_satisfies_annotation() {
        assert!(check_src("let y: F64 = to_f64(sqrt(2));").is_empty());
    }

    #[test]
    fn integer_annotation_rejects_float() {
        let errs = check_src("let z: Integer = 3.14;");
        assert_eq!(errs.len(), 1);
        assert!(errs[0].message.contains("Integer"));
        assert!(errs[0].message.contains("F64"));
    }

    #[test]
    fn string_annotation_rejects_integer() {
        let errs = check_src("let s: String = 5;");
        assert_eq!(errs.len(), 1);
        assert!(errs[0].message.contains("String"));
        assert!(errs[0].message.contains("Integer"));
    }

    #[test]
    fn syntax_error_surfaces_as_type_error() {
        let errs = check_src("let x: =");
        assert_eq!(errs.len(), 1);
        assert!(!errs[0].message.is_empty());
    }

    #[test]
    fn promotion_is_allowed() {
        assert!(check_src("let n: Integer = 7; let r: F64 = 1;").is_empty());
    }

    #[test]
    fn errors_are_reported_in_source_order() {
        let errs = check_src("let a: String = 1\nlet b: Integer = 2.5\n");
        assert_eq!(errs.len(), 2);
        assert!(errs[0].line < errs[1].line);
    }

    #[test]
    fn try_operator_rejected_outside_result_fn() {
        let errs = check_src("let x = try_f64(\"a\")?;");
        assert_eq!(errs.len(), 1);
        assert!(errs[0].message.contains("E0054"));
    }

    #[test]
    fn try_operator_allowed_in_result_fn() {
        assert!(check_src("fn f() -> Result<F64, Error> {\n    let v = try_f64(\"a\")?;\n    return Ok(v);\n}").is_empty());
    }

    #[test]
    fn refutable_pattern_in_let_is_flagged() {
        let errs = check_src("let 0 = x;");
        assert_eq!(errs.len(), 1);
        assert!(errs[0].message.contains("E0053"));
    }

    #[test]
    fn collapse_targets_infer_fixed_width_types() {
        assert!(check_src("let a: I8 = to_i8(7); let b: Usize = to_usize(3); let c: Option<Integer> = get([1], 0);").is_empty());
    }

    /// Infer the type of a bare expression statement (spec §6.3).
    fn inf_of(src: &str) -> String {
        let p = parse(src).unwrap();
        let Stmt::Expr(e) = &p.stmts[0] else {
            panic!("expected an expression statement, got {:?}", p.stmts[0]);
        };
        infer(e, &HashMap::new())
    }

    #[test]
    fn infer_literal_and_collection_types() {
        assert_eq!(inf_of("1"), "Integer");
        assert_eq!(inf_of("1.5"), "F64");
        assert_eq!(inf_of("0x1F"), "Integer");
        assert_eq!(inf_of("\"s\""), "String");
        assert_eq!(inf_of("true"), "Bool");
        assert_eq!(inf_of("[1, 2, 3]"), "Array<Integer>");
        assert_eq!(inf_of("[1.0, 2.0]"), "Array<F64>");
        assert_eq!(inf_of("[[1, 2], [3, 4]]"), "Array<Array<Integer>>");
        assert_eq!(inf_of("[sqrt(2), sqrt(3)]"), "Array<Expr>");
        assert_eq!(inf_of("[my_unknown_f(), my_other()]"), "Array<Expr>");
        assert_eq!(inf_of("{ \"a\": 1 }"), "dict");
        assert_eq!(inf_of("{1, 2}"), "set");
        assert_eq!(inf_of("(1, \"a\")"), "tuple");
        assert_eq!(inf_of("-5"), "Integer");
        assert_eq!(inf_of("!true"), "Bool");
    }

    #[test]
    fn assignable_promotes_numeric_layers() {
        assert!(assignable(&Type::F64, "Integer"));
        assert!(assignable(&Type::F64, "Rational"));
        assert!(assignable(&Type::F64, "F64"));
        assert!(assignable(&Type::Integer, "Integer"));
        assert!(assignable(&Type::Rational, "Integer"));
        assert!(assignable(&Type::Number, "Complex"));
        assert!(!assignable(&Type::Integer, "F64"));
        assert!(!assignable(&Type::String, "Integer"));
        assert!(!assignable(&Type::Bool, "Integer"));
    }

    #[test]
    fn assignable_wildcards_and_collections() {
        let value = Type::User(Spanned { value: "Value".into(), span: Span::new(0, 0) });
        assert!(assignable(&value, "anything"));
        assert!(assignable(&value, "unknown"));
        assert!(assignable(&Type::String, "unknown"));
        assert!(assignable(&Type::Array(Box::new(Type::F64)), "array"));
        assert!(assignable(&Type::Array(Box::new(Type::F64)), "Array<Integer>"));
        assert!(!assignable(&Type::Array(Box::new(Type::String)), "Array<Integer>"));
        assert!(assignable(&Type::Matrix(Box::new(Type::F64)), "Array<Array<Integer>>"));
        assert!(!assignable(&Type::Matrix(Box::new(Type::F64)), "Array<Integer>"));
        assert!(assignable(&Type::Option(Box::new(Type::Integer)), "option"));
    }
}
