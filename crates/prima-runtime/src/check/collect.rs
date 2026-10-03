//! AST-walking static-error collectors (spec §16.2 compile-time errors, §18.4 annotations).
//!
//! These descend statements/expressions, checking `let`/`const` type annotations, refutable `let`
//! patterns (`E0053`), `?` misuse outside a `Result`/`Option`-returning function (`E0054`),
//! `@builtin`/`@c_api::extern` annotations (`E0055`/`E0056`/`E0071`/`E0072`), and stdlib call
//! sites (`E0050`). They collect errors rather than fail-fast.

use std::collections::HashMap;

use crate::builtins::Builtin;
use crate::capi::c_type;
use crate::eval::CORE_BUILTIN_NAMES;
use prima_core::suggest::did_you_mean_help;
use prima_syntax::ast::{
    Annotation, ClassMemberKind, Expr, ExprKind, ImplOp, MatchArm, Param, Spanned, Stmt, Type,
};

use super::Ctx;
use super::TypeError;
use super::depth::enter_walk_depth;
use super::error::{c_param_ok, push_err, push_err_with_help, type_display, type_span};
use super::infer::{annot_accepts, annot_name, infer, pattern_is_refutable};
use super::signature::{Signature, check_call_signature};

/// Rule-based `= help:` for a static type mismatch (spec §16.4): when a symbolic `Expr` value is
/// assigned to a numeric-typed binding, point at the explicit collapse function for that target.
/// Returns `None` for every other mismatch so the hint is never noise.
fn collapse_help(t: &Type, got: &str) -> Option<String> {
    if got != "Expr" {
        return None;
    }
    let target = match t {
        Type::F64 => "to_f64",
        Type::F32 => "to_f32",
        Type::I8 => "to_i8",
        Type::I16 => "to_i16",
        Type::I32 => "to_i32",
        Type::I64 => "to_i64",
        Type::I128 => "to_i128",
        Type::U8 => "to_u8",
        Type::U16 => "to_u16",
        Type::U32 => "to_u32",
        Type::U64 => "to_u64",
        Type::U128 => "to_u128",
        Type::Isize => "to_isize",
        Type::Usize => "to_usize",
        Type::Integer => "to_bigint",
        Type::Complex => "to_complex",
        _ => return None,
    };
    Some(format!(
        "the value is symbolic; collapse it explicitly, e.g. `{target}(...)`"
    ))
}

/// Collect static errors for one statement. Only the type annotations of `let`/`const` (with a
/// plain binding pattern) are checked; all bodies are descended into to catch `?` misuse and to
/// validate stdlib call sites. `is_pub` records whether the statement is wrapped in `Stmt::Pub`
/// (spec §15.2), required by `@c_api::extern` exports (spec §18.4, E0072).
pub(crate) fn collect_stmt_errors(
    src: &str,
    stmt: &Stmt,
    errors: &mut Vec<TypeError>,
    ctx: Ctx,
    is_pub: bool,
    sigs: &HashMap<String, Vec<Signature>>,
) {
    match stmt {
        Stmt::Let {
            pat,
            type_ann,
            value,
            span,
            ..
        } => {
            // `let` rejects refutable patterns (spec §4.4 `E0053`).
            if pattern_is_refutable(pat) {
                push_err_with_help(
                    src,
                    errors,
                    *span,
                    "E0053",
                    "refutable pattern in `let`".into(),
                    None,
                    Some("use `if let` or `match` to handle the refutable pattern".into()),
                );
            }
            if let Some(t) = type_ann {
                let inf = infer(value, sigs);
                if !annot_accepts(t, &inf) {
                    push_err_with_help(
                        src,
                        errors,
                        value.span,
                        "E0050",
                        format!("type mismatch: expected {}, got {}", annot_name(t), inf),
                        None,
                        collapse_help(t, &inf),
                    );
                }
            }
            collect_expr_errors(src, value, errors, ctx, sigs);
        }
        Stmt::Const {
            type_ann: t, value, ..
        } => {
            let inf = infer(value, sigs);
            if !annot_accepts(t, &inf) {
                push_err_with_help(
                    src,
                    errors,
                    value.span,
                    "E0050",
                    format!("type mismatch: expected {}, got {}", annot_name(t), inf),
                    None,
                    collapse_help(t, &inf),
                );
            }
            collect_expr_errors(src, value, errors, ctx, sigs);
        }
        Stmt::FnDef {
            name,
            params,
            ret,
            annotations,
            body,
            ..
        } => {
            errors.extend(check_annotation_errors(
                src,
                name,
                params,
                ret,
                annotations,
                !body.stmts.is_empty(),
                is_pub,
            ));
            check_missing_type_ann(src, name, params, ret, annotations, errors);
            let allow = matches!(ret, Some(Type::Result(..) | Type::Option(..)));
            collect_block_errors(src, body, errors, Ctx { allow_try: allow }, sigs);
        }
        Stmt::MathDef {
            name,
            params,
            ret,
            annotations,
            body,
            ..
        } => {
            check_missing_type_ann(src, name, params, ret, annotations, errors);
            collect_expr_errors(src, body, errors, Ctx { allow_try: false }, sigs);
        }
        Stmt::ClassDef {
            name,
            annotations,
            members,
            ..
        } => {
            // Only the builtin `String` class is meaningful, and it is implicit — never declared in
            // source — so any user `@builtin class` has no registered implementation (spec §18.4, E0055).
            if annotations.iter().any(|a| a.is_builtin()) && name.value != "String" {
                push_err(
                    src,
                    errors,
                    name.span,
                    "E0055",
                    format!("unregistered `@builtin` class `{}`", name.value),
                );
            }
            for m in members {
                if let ClassMemberKind::Method {
                    name: mname,
                    params,
                    ret,
                    annotations,
                    body,
                } = &m.kind
                {
                    check_missing_type_ann(src, mname, params, ret, annotations, errors);
                    if let Some(b) = body {
                        let allow = matches!(ret, Some(Type::Result(..) | Type::Option(..)));
                        collect_block_errors(src, b, errors, Ctx { allow_try: allow }, sigs);
                    }
                }
            }
        }
        Stmt::Impl { op, members, .. } => {
            check_op_overload(src, *op, members, errors);
            for m in members {
                collect_stmt_errors(src, m, errors, ctx, false, sigs);
            }
        }
        Stmt::IfLet {
            value, then, else_, ..
        } => {
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
        Stmt::Match {
            scrutinee, arms, ..
        } => {
            collect_expr_errors(src, scrutinee, errors, ctx, sigs);
            collect_arms_errors(src, arms, errors, ctx, sigs);
        }
        Stmt::If {
            cond,
            then,
            elifs,
            else_,
            ..
        } => {
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
        Stmt::For {
            range, step, body, ..
        }
        | Stmt::ParFor {
            range, step, body, ..
        } => {
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

pub(crate) fn collect_block_errors(
    src: &str,
    block: &prima_syntax::ast::Block,
    errors: &mut Vec<TypeError>,
    ctx: Ctx,
    sigs: &HashMap<String, Vec<Signature>>,
) {
    for s in &block.stmts {
        collect_stmt_errors(src, s, errors, ctx, false, sigs);
    }
}

pub(crate) fn collect_arms_errors(
    src: &str,
    arms: &[MatchArm],
    errors: &mut Vec<TypeError>,
    ctx: Ctx,
    sigs: &HashMap<String, Vec<Signature>>,
) {
    for arm in arms {
        if let Some(g) = &arm.guard {
            collect_expr_errors(src, g, errors, ctx, sigs);
        }
        collect_expr_errors(src, &arm.body, errors, ctx, sigs);
    }
}

/// Annotation validation (spec §18.4), returning the errors for one `fn`:
/// - `@builtin(O0)`: signature-only and the name must name a registered host builtin (E0056/E0055).
/// - `@builtin(ON)` (`N >= 1`): must have a `.pra` fallback body (E0056); the Rust impl is optional.
/// - `@c_api::extern`: `pub` and `c_api::*` C-compatible parameter/return types (E0072/E0071).
pub(crate) fn check_annotation_errors(
    src: &str,
    name: &Spanned<String>,
    params: &[Param],
    ret: &Option<Type>,
    annotations: &[Annotation],
    has_body: bool,
    is_pub: bool,
) -> Vec<TypeError> {
    let mut errors = Vec::new();
    if let Some(level) = annotations
        .iter()
        .filter(|a| a.is_builtin())
        .map(|a| a.builtin_level())
        .max()
    {
        if level == 0 {
            if has_body {
                push_err_with_help(
                    src,
                    &mut errors,
                    name.span,
                    "E0056",
                    "`@builtin` function must not have a body".into(),
                    None,
                    Some(
                        "`@builtin(O0)` declarations are signature-only; remove the body, or \
                         declare a layered tier with `@builtin(O1)` and a `.pra` fallback body"
                            .into(),
                    ),
                );
            } else if Builtin::from_name(&name.value).is_none() {
                let help = did_you_mean_help(&name.value, CORE_BUILTIN_NAMES);
                push_err_with_help(
                    src,
                    &mut errors,
                    name.span,
                    "E0055",
                    format!("unregistered `@builtin` function `{}`", name.value),
                    None,
                    help,
                );
            }
        } else if !has_body {
            push_err_with_help(
                src,
                &mut errors,
                name.span,
                "E0056",
                format!("`@builtin(O{level})` function must have a body"),
                None,
                Some(format!(
                    "a layered `@builtin(O{level})` declaration needs a `.pra` fallback body"
                )),
            );
        }
    }
    if annotations.contains(&Annotation::CApiExtern) {
        if !is_pub {
            push_err_with_help(
                src,
                &mut errors,
                name.span,
                "E0072",
                "`@c_api::extern` function must be `pub`".into(),
                None,
                Some("mark the function `pub` so the C ABI can export it".into()),
            );
        }
        for p in params {
            let ok = p.type_ann.as_ref().is_some_and(c_param_ok);
            if !ok {
                let ty = p
                    .type_ann
                    .as_ref()
                    .map_or_else(|| p.name.value.clone(), type_display);
                let sp = p
                    .type_ann
                    .as_ref()
                    .map_or(p.name.span, |t| type_span(t, p.name.span));
                push_err_with_help(
                    src,
                    &mut errors,
                    sp,
                    "E0071",
                    format!(
                        "`@c_api::extern` parameter/return type `{ty}` is not C-compatible"
                    ),
                    None,
                    Some(
                        "`@c_api::extern` parameters and returns must use C-compatible `c_api::*` types"
                            .into(),
                    ),
                );
            }
        }
        if let Some(t) = ret
            && c_type(t).is_none()
        {
            push_err_with_help(
                src,
                &mut errors,
                type_span(t, name.span),
                "E0071",
                format!(
                    "`@c_api::extern` parameter/return type `{}` is not C-compatible",
                    type_display(t)
                ),
                None,
                Some(
                    "`@c_api::extern` parameters and returns must use C-compatible `c_api::*` types"
                        .into(),
                ),
            );
        }
    }
    errors
}

/// `E0051 missing_type_ann` (spec §18.4): an `@builtin` (any tier) or `@c_api::extern` signature is
/// typed, so every non-`self` parameter must carry a type annotation, and `@c_api::extern` must also
/// declare a return type. Ordinary untyped user functions are not affected.
pub(crate) fn check_missing_type_ann(
    src: &str,
    name: &Spanned<String>,
    params: &[Param],
    ret: &Option<Type>,
    annotations: &[Annotation],
    errors: &mut Vec<TypeError>,
) {
    let is_builtin = annotations.iter().any(|a| a.is_builtin());
    let is_extern = annotations.contains(&Annotation::CApiExtern);
    if !is_builtin && !is_extern {
        return;
    }
    for p in params {
        if !p.is_self && p.type_ann.is_none() {
            push_err_with_help(
                src,
                errors,
                p.name.span,
                "E0051",
                format!("missing type annotation for parameter `{}`", p.name.value),
                None,
                Some(
                    "add a type annotation; @builtin/@c_api::extern signatures are typed (spec §18.4)"
                        .into(),
                ),
            );
        }
    }
    if is_extern && ret.is_none() {
        push_err_with_help(
            src,
            errors,
            name.span,
            "E0051",
            "`@c_api::extern` function must declare a return type".into(),
            None,
            Some("add a return type; @c_api::extern signatures are typed (spec §18.4)".into()),
        );
    }
}

/// `E0063 self_not_first` / `E0081 op_overload_bad_arity` (spec §18.5): validate each `impl ops::Op`
/// member. `Neg` takes exactly one parameter (`self`); every other operator takes exactly two
/// (`self` and the operand). A `self` present but not first is `E0063`; a missing `self` or a wrong
/// parameter count is `E0081`.
fn check_op_overload(src: &str, op: ImplOp, members: &[Box<Stmt>], errors: &mut Vec<TypeError>) {
    let required = if op == ImplOp::Neg { 1 } else { 2 };
    for member in members {
        let (name, params) = match member.as_ref() {
            Stmt::FnDef { name, params, .. } | Stmt::MathDef { name, params, .. } => (name, params),
            Stmt::Pub(inner) => match inner.as_ref() {
                Stmt::FnDef { name, params, .. } | Stmt::MathDef { name, params, .. } => {
                    (name, params)
                }
                _ => continue,
            },
            _ => continue,
        };
        let self_pos = params.iter().position(|p| p.is_self);
        if let Some(pos) = self_pos
            && pos != 0
        {
            push_err_with_help(
                src,
                errors,
                params[pos].name.span,
                "E0063",
                "`self` must be the first parameter of a method".into(),
                None,
                None,
            );
        }
        if params.len() != required || self_pos.is_none() {
            push_err_with_help(
                src,
                errors,
                name.span,
                "E0081",
                format!("operator overload `{}` has an invalid signature", name.value),
                None,
                Some(
                    "operator overloads take `self` and the operand, e.g. `fn add(self, rhs: T) -> T`"
                        .into(),
                ),
            );
        }
    }
}

/// Descend an expression tree, flagging `?` outside a `Result`/`Option`-returning function (spec
/// §16.3 `E0054`) and validating stdlib call sites against harvested signatures (spec §18.4).
///
/// Recursion budget (spec §16.4): stops descending with a located "too deep" error when the walk
/// depth is exhausted, so pathological nesting cannot overflow the stack.
pub(crate) fn collect_expr_errors(
    src: &str,
    expr: &Expr,
    errors: &mut Vec<TypeError>,
    ctx: Ctx,
    sigs: &HashMap<String, Vec<Signature>>,
) {
    let Some(_depth) = enter_walk_depth() else {
        push_err(
            src,
            errors,
            expr.span,
            "E0012",
            "expression nesting is too deep to check".into(),
        );
        return;
    };
    match &expr.kind {
        ExprKind::Try(inner) => {
            if !ctx.allow_try {
                push_err_with_help(
                    src,
                    errors,
                    expr.span,
                    "E0054",
                    "`?` can only be used inside a function returning `Result`/`Option`".into(),
                    None,
                    Some(
                        "annotate the enclosing function's return type as `Result<..>` or `Option<..>`"
                            .into(),
                    ),
                );
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
                    prima_syntax::ast::IndexItem::Elem(e) => {
                        collect_expr_errors(src, e, errors, ctx, sigs)
                    }
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
        ExprKind::FString(parts) => {
            for p in parts {
                if let prima_syntax::ast::FStringPart::Interp { expr, .. } = p {
                    collect_expr_errors(src, expr, errors, ctx, sigs);
                }
            }
        }
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
        ExprKind::Comprehension {
            output, clauses, ..
        } => {
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
        ExprKind::Custom(items) => {
            for (p, v) in items {
                collect_expr_errors(src, p, errors, ctx, sigs);
                collect_expr_errors(src, v, errors, ctx, sigs);
            }
        }
        _ => {}
    }
}
