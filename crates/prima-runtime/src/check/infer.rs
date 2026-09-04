//! Static type inference and assignment rules (spec §6.3 literal/local type inference, §18.4
//! call-site checking).
//!
//! `infer` gives the statically-decidable type name of a literal/simple expression (returning
//! `"unknown"` when not decidable); `assignable`/`annot_accepts` evaluate whether a value of one
//! type is accepted where another is expected, under the spec's implicit promotion. These are pure,
//! side-effect-free predicates shared across the checker.

use std::collections::HashMap;

use prima_syntax::ast::{BinOp, CompKind, Expr, ExprKind, Literal, Pattern, Type, UnOp};

use super::signature::{Signature, lookup_call_signature};

/// Refutable-pattern check for `let` (spec §4.4): only bindings, wildcards and grouped tuples/arrays
/// of irrefutable patterns are irrefutable.
pub(crate) fn pattern_is_refutable(p: &Pattern) -> bool {
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
pub(crate) fn infer(expr: &Expr, sigs: &HashMap<String, Vec<Signature>>) -> String {
    match &expr.kind {
        ExprKind::Literal(lit) => match lit {
            Literal::Integer(_) | Literal::Hex(_) | Literal::Binary(_) => "Integer".into(),
            Literal::Float(_) => "F64".into(),
            Literal::Bool(_) => "Bool".into(),
            Literal::String { .. } => "String".into(),
            Literal::Char(_) => "Char".into(),
            Literal::Tex(_) => "Expr".into(),
        },
        ExprKind::FString(_) => "String".into(),
        ExprKind::Symbol(_) => "Expr".into(),
        ExprKind::Call { callee, .. } => {
            if let ExprKind::Path { segments } = &callee.kind {
                if let Some(candidates) = lookup_call_signature(segments, sigs)
                    && let Some(sig) = candidates.first()
                {
                    return match &sig.ret {
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
                        name if name.starts_with("try_") || name.starts_with("checked_") => {
                            "Result".into()
                        }
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
        ExprKind::Unary {
            op: UnOp::Neg | UnOp::Pos,
            operand,
        } => infer(operand, sigs),
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
pub(crate) fn combine_numeric(l: &str, r: &str) -> &'static str {
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
pub(crate) fn annot_accepts(t: &Type, inf: &str) -> bool {
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
pub(crate) fn assignable(param: &Type, got: &str) -> bool {
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
pub(crate) fn collection_inner<'a>(s: &'a str, name: &str) -> Option<&'a str> {
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
pub(crate) fn type_name(t: &Type) -> String {
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
        Type::Tuple(ts) => format!(
            "Tuple<{}>",
            ts.iter().map(type_name).collect::<Vec<_>>().join(", ")
        ),
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
pub(crate) fn annot_name(t: &Type) -> &'static str {
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
