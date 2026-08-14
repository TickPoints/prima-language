use crate::builtins::Builtin;
use crate::capi::c_type;
use prima_syntax::ast::{Annotation, BinOp, ClassMemberKind, CompKind, Expr, ExprKind, Literal, MatchArm, Param, Pattern, Spanned, Stmt, Type, UnOp};
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

    let mut errors = Vec::new();
    let ctx = Ctx { allow_try: false };
    for stmt in &program.stmts {
        collect_stmt_errors(src, stmt, &mut errors, ctx, false);
    }
    // Statement order is source order; sorting stably by (line, column) keeps it consistent with span.start.
    errors.sort_by_key(|e| (e.line, e.column));
    errors
}

/// Collect static errors for one statement. Only the type annotations of `let`/`const` (with a
/// plain binding pattern) are checked; all bodies are descended into to catch `?` misuse.
/// `is_pub` records whether the statement is wrapped in `Stmt::Pub` (spec §15.2), required by
/// `@c_api::extern` exports (spec §18.4, E0072).
fn collect_stmt_errors(src: &str, stmt: &Stmt, errors: &mut Vec<TypeError>, ctx: Ctx, is_pub: bool) {
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
                let inf = infer(value);
                if !annot_accepts(t, inf) {
                    let (line, column) = line_col(src, value.span.start);
                    errors.push(TypeError {
                        line,
                        column,
                        span: value.span,
                        message: format!("type mismatch: expected {}, got {}", annot_name(t), inf),
                    });
                }
            }
            collect_expr_errors(src, value, errors, ctx);
        }
        Stmt::Const { type_ann: t, value, .. } => {
            let inf = infer(value);
            if !annot_accepts(t, inf) {
                let (line, column) = line_col(src, value.span.start);
                errors.push(TypeError {
                    line,
                    column,
                    span: value.span,
                    message: format!("type mismatch: expected {}, got {}", annot_name(t), inf),
                });
            }
            collect_expr_errors(src, value, errors, ctx);
        }
        Stmt::FnDef { name, params, ret, annotations, body, .. } => {
            errors.extend(check_annotation_errors(src, name, params, ret, annotations, !body.stmts.is_empty(), is_pub));
            let allow = matches!(ret, Some(Type::Result(..) | Type::Option(..)));
            collect_block_errors(src, body, errors, Ctx { allow_try: allow });
        }
        Stmt::MathDef { body, .. } => {
            collect_expr_errors(src, body, errors, Ctx { allow_try: false });
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
                    collect_block_errors(src, b, errors, Ctx { allow_try: allow });
                }
            }
        }
        Stmt::Impl { members, .. } => {
            for m in members {
                collect_stmt_errors(src, m, errors, ctx, false);
            }
        }
        Stmt::IfLet { value, then, else_, .. } => {
            collect_expr_errors(src, value, errors, ctx);
            collect_block_errors(src, then, errors, ctx);
            if let Some(b) = else_ {
                collect_block_errors(src, b, errors, ctx);
            }
        }
        Stmt::WhileLet { value, body, .. } => {
            collect_expr_errors(src, value, errors, ctx);
            collect_block_errors(src, body, errors, ctx);
        }
        Stmt::Match { scrutinee, arms, .. } => {
            collect_expr_errors(src, scrutinee, errors, ctx);
            collect_arms_errors(src, arms, errors, ctx);
        }
        Stmt::If { cond, then, elifs, else_, .. } => {
            collect_expr_errors(src, cond, errors, ctx);
            collect_block_errors(src, then, errors, ctx);
            for (c, b) in elifs {
                collect_expr_errors(src, c, errors, ctx);
                collect_block_errors(src, b, errors, ctx);
            }
            if let Some(b) = else_ {
                collect_block_errors(src, b, errors, ctx);
            }
        }
        Stmt::While { cond, body, .. } => {
            collect_expr_errors(src, cond, errors, ctx);
            collect_block_errors(src, body, errors, ctx);
        }
        Stmt::For { range, step, body, .. } | Stmt::ParFor { range, step, body, .. } => {
            collect_expr_errors(src, &range.0, errors, ctx);
            collect_expr_errors(src, &range.1, errors, ctx);
            if let Some(s) = step {
                collect_expr_errors(src, s, errors, ctx);
            }
            collect_block_errors(src, body, errors, ctx);
        }
        Stmt::Return { value, .. } => {
            if let Some(e) = value {
                collect_expr_errors(src, e, errors, ctx);
            }
        }
        Stmt::Assign { target, value, .. } => {
            collect_expr_errors(src, target, errors, ctx);
            collect_expr_errors(src, value, errors, ctx);
        }
        Stmt::WithConfig { entries, body, .. } => {
            for e in entries {
                collect_expr_errors(src, &e.value, errors, ctx);
            }
            collect_block_errors(src, body, errors, ctx);
        }
        Stmt::Expr(e) => collect_expr_errors(src, e, errors, ctx),
        Stmt::Pub(inner) => collect_stmt_errors(src, inner, errors, ctx, true),
    }
}

fn collect_block_errors(src: &str, block: &prima_syntax::ast::Block, errors: &mut Vec<TypeError>, ctx: Ctx) {
    for s in &block.stmts {
        collect_stmt_errors(src, s, errors, ctx, false);
    }
}

fn collect_arms_errors(src: &str, arms: &[MatchArm], errors: &mut Vec<TypeError>, ctx: Ctx) {
    for arm in arms {
        if let Some(g) = &arm.guard {
            collect_expr_errors(src, g, errors, ctx);
        }
        collect_expr_errors(src, &arm.body, errors, ctx);
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

/// Descend an expression tree, flagging `?` outside a `Result`/`Option`-returning function (spec §16.3 `E0054`).
fn collect_expr_errors(src: &str, expr: &Expr, errors: &mut Vec<TypeError>, ctx: Ctx) {
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
            collect_expr_errors(src, inner, errors, ctx);
        }
        ExprKind::Call { callee, args } => {
            collect_expr_errors(src, callee, errors, ctx);
            for a in args {
                collect_expr_errors(src, a, errors, ctx);
            }
        }
        ExprKind::MethodCall { receiver, args, .. } => {
            collect_expr_errors(src, receiver, errors, ctx);
            for a in args {
                collect_expr_errors(src, a, errors, ctx);
            }
        }
        ExprKind::Field { receiver, .. } => collect_expr_errors(src, receiver, errors, ctx),
        ExprKind::StructLiteral { fields, base, .. } => {
            for f in fields {
                if let Some(v) = &f.value {
                    collect_expr_errors(src, v, errors, ctx);
                }
            }
            if let Some(b) = base {
                collect_expr_errors(src, b, errors, ctx);
            }
        }
        ExprKind::Index { base, index } => {
            collect_expr_errors(src, base, errors, ctx);
            for item in &index.items {
                match item {
                    prima_syntax::ast::IndexItem::Elem(e) => collect_expr_errors(src, e, errors, ctx),
                    prima_syntax::ast::IndexItem::Slice { start, end } => {
                        if let Some(s) = start {
                            collect_expr_errors(src, s, errors, ctx);
                        }
                        if let Some(e) = end {
                            collect_expr_errors(src, e, errors, ctx);
                        }
                    }
                }
            }
        }
        ExprKind::Binary { lhs, rhs, .. } => {
            collect_expr_errors(src, lhs, errors, ctx);
            collect_expr_errors(src, rhs, errors, ctx);
        }
        ExprKind::Unary { operand, .. } => collect_expr_errors(src, operand, errors, ctx),
        ExprKind::Array(items) | ExprKind::Tuple(items) | ExprKind::Set(items) => {
            for i in items {
                collect_expr_errors(src, i, errors, ctx);
            }
        }
        ExprKind::Dict(entries) => {
            for (k, v) in entries {
                collect_expr_errors(src, k, errors, ctx);
                collect_expr_errors(src, v, errors, ctx);
            }
        }
        ExprKind::Comprehension { output, clauses, .. } => {
            collect_expr_errors(src, output, errors, ctx);
            for c in clauses {
                match c {
                    prima_syntax::ast::ComprehensionClause::For { iter, .. } => {
                        collect_expr_errors(src, iter, errors, ctx);
                    }
                    prima_syntax::ast::ComprehensionClause::If { cond } => {
                        collect_expr_errors(src, cond, errors, ctx);
                    }
                }
            }
        }
        ExprKind::KeyValue { key, value } => {
            collect_expr_errors(src, key, errors, ctx);
            collect_expr_errors(src, value, errors, ctx);
        }
        ExprKind::Lambda { body, .. } => collect_expr_errors(src, body, errors, ctx),
        ExprKind::Match { scrutinee, arms } => {
            collect_expr_errors(src, scrutinee, errors, ctx);
            collect_arms_errors(src, arms, errors, ctx);
        }
        ExprKind::Pipeline { lhs, rhs } => {
            collect_expr_errors(src, lhs, errors, ctx);
            collect_expr_errors(src, rhs, errors, ctx);
        }
        ExprKind::Custom(items) => {
            for (p, v) in items {
                collect_expr_errors(src, p, errors, ctx);
                collect_expr_errors(src, v, errors, ctx);
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

/// Static type name of a literal/simple expression (spec §6.3 literal type inference).
fn infer(expr: &Expr) -> &'static str {
    match &expr.kind {
        ExprKind::Literal(lit) => match lit {
            Literal::Integer(_) | Literal::Hex(_) | Literal::Binary(_) => "Integer",
            Literal::Float(_) => "F64",
            Literal::Bool(_) => "Bool",
            Literal::Str(_) => "String",
            Literal::Char(_) => "Char",
            Literal::Tex(_) => "Expr",
        },
        ExprKind::Symbol(_) => "Expr",
        ExprKind::Call { callee, .. } => {
            if let ExprKind::Path { segments } = &callee.kind
                && segments.len() == 1
            {
                return match segments[0].value.as_str() {
                    "to_f64" => "F64",
                    "to_f32" => "F32",
                    "to_i8" => "I8",
                    "to_i16" => "I16",
                    "to_i32" => "I32",
                    "to_i64" => "I64",
                    "to_i128" => "I128",
                    "to_u8" => "U8",
                    "to_u16" => "U16",
                    "to_u32" => "U32",
                    "to_u64" => "U64",
                    "to_u128" => "U128",
                    "to_isize" => "Isize",
                    "to_usize" => "Usize",
                    "to_bigint" => "Integer",
                    "to_rational" => "Rational",
                    "to_complex" => "Complex",
                    "print" | "println" => "Nil",
                    name if name.starts_with("try_") || name.starts_with("checked_") => "Result",
                    "Some" | "None" => "Option",
                    "Ok" | "Err" => "Result",
                    "get" => "Option",
                    _ => "Expr",
                };
            }
            "Expr"
        }
        ExprKind::Unary { op: UnOp::Neg | UnOp::Pos, operand } => infer(operand),
        ExprKind::Unary { op: UnOp::Not, .. } => "Bool",
        ExprKind::Binary { op, lhs, rhs } => match op {
            BinOp::Add | BinOp::Sub | BinOp::Mul => combine_numeric(infer(lhs), infer(rhs)),
            BinOp::Div => {
                let exact = |s: &str| matches!(s, "Integer" | "Rational" | "Complex");
                if exact(infer(lhs)) && exact(infer(rhs)) {
                    "Rational"
                } else {
                    "F64"
                }
            }
            _ => "Expr",
        },
        ExprKind::Array(_) => "Array",
        ExprKind::Dict(_) => "Dict",
        ExprKind::Set(_) => "Set",
        ExprKind::Comprehension { kind, .. } => match kind {
            CompKind::Array => "Array",
            CompKind::Dict => "Dict",
            CompKind::Set => "Set",
            CompKind::Tuple => "Tuple",
        },
        ExprKind::KeyValue { .. } => "value",
        ExprKind::Tuple(_) => "Tuple",
        ExprKind::Lambda { .. } => "Fn",
        _ => "Expr",
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
        Type::Array(_) => inf == "Array",
        Type::Tuple(_) => inf == "Tuple",
        Type::Option(_) => inf == "Option",
        Type::Result(..) => inf == "Result",
        Type::SelfType => true,
        Type::Fn { .. } | Type::MFn { .. } | Type::User(_) | Type::Matrix(_) => true,
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
}
