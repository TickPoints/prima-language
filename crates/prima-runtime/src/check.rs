use prima_syntax::ast::{BinOp, Expr, ExprKind, Literal, Stmt, Type, UnOp};
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
    for stmt in &program.stmts {
        collect_stmt_errors(src, stmt, &mut errors);
    }
    // Statement order is source order; sorting stably by (line, column) keeps it consistent with span.start.
    errors.sort_by_key(|e| (e.line, e.column));
    errors
}

/// Only check the type annotations of top-level (and `pub`-wrapped) `let`/`const`; do not descend into blocks/function bodies.
fn collect_stmt_errors(src: &str, stmt: &Stmt, errors: &mut Vec<TypeError>) {
    match stmt {
        Stmt::Let { type_ann: Some(t), value, .. } | Stmt::Const { type_ann: t, value, .. } => {
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
        Stmt::Pub(inner) => collect_stmt_errors(src, inner, errors),
        _ => {}
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
                    "to_i32" => "I32",
                    "to_i64" | "to_bigint" => "Integer",
                    "to_rational" => "Rational",
                    "to_complex" => "Complex",
                    "print" | "println" => "Nil",
                    name if name.starts_with("try_") || name.starts_with("checked_") => "Result",
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
        Type::I32 => inf == "Integer",
        Type::Complex => matches!(inf, "Integer" | "Rational" | "Complex"),
        Type::Number => matches!(inf, "Integer" | "Rational" | "F64" | "F32" | "Complex"),
        Type::Expr | Type::Symbol => true,
        Type::Bool => inf == "Bool",
        Type::String => inf == "String",
        Type::Char => inf == "Char",
        Type::Array(_) => inf == "Array",
        Type::Tuple(_) => inf == "Tuple",
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
        Type::I32 => "I32",
        Type::Complex => "Complex",
        Type::Expr => "Expr",
        Type::Symbol => "Symbol",
        Type::Bool => "Bool",
        Type::String => "String",
        Type::Char => "Char",
        Type::Array(_) => "Array",
        Type::Matrix(_) => "Matrix",
        Type::Tuple(_) => "Tuple",
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
        let errs = check_src("let x: F64 = sqrt(2)");
        assert_eq!(errs.len(), 1);
        assert!(errs[0].message.contains("F64"));
        assert!(errs[0].message.contains("Expr"));
    }

    #[test]
    fn explicit_conversion_satisfies_annotation() {
        assert!(check_src("let y: F64 = to_f64(sqrt(2))").is_empty());
    }

    #[test]
    fn integer_annotation_rejects_float() {
        let errs = check_src("let z: Integer = 3.14");
        assert_eq!(errs.len(), 1);
        assert!(errs[0].message.contains("Integer"));
        assert!(errs[0].message.contains("F64"));
    }

    #[test]
    fn string_annotation_rejects_integer() {
        let errs = check_src("let s: String = 5");
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
        assert!(check_src("let n: Integer = 7\nlet r: F64 = 1").is_empty());
    }

    #[test]
    fn errors_are_reported_in_source_order() {
        let errs = check_src("let a: String = 1\nlet b: Integer = 2.5");
        assert_eq!(errs.len(), 2);
        assert!(errs[0].line < errs[1].line);
    }
}
