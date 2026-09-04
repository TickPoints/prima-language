//! Type-signature rendering for `prima fmt` (spec appendix A / BNF): the `Type` grammar — primitive
//! names, generic (`Array<T>`/`Option<T>`/`Result<A, B>`), tuple and function (`Fn(...) -> T`/
//! `MFn(...) -> T`) forms — plus the `(param, ...)` parameter list and the `-> Ret` return suffix.
//! Shared with `prima doc`, so `format_params`/`format_ret`/`format_type` are `pub(crate)` and
//! re-exported at `crate::fmt`.

use prima_syntax::ast::{Param, Type};

pub(crate) fn format_ret(ret: &Option<Type>, out: &mut String) {
    if let Some(t) = ret {
        out.push_str(" -> ");
        format_type(t, out);
    }
}

pub(crate) fn format_params(params: &[Param], out: &mut String) {
    out.push('(');
    format_params_bare(params, out);
    out.push(')');
}

/// Param list without the enclosing parens (used by lambdas, spec §4.6).
pub(crate) fn format_params_bare(params: &[Param], out: &mut String) {
    for (i, p) in params.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        if p.is_self {
            out.push_str("self");
        } else {
            out.push_str(&p.name.value);
        }
        if let Some(t) = &p.type_ann {
            out.push_str(": ");
            format_type(t, out);
        }
    }
}

pub(crate) fn format_type(ty: &Type, out: &mut String) {
    match ty {
        Type::Number => out.push_str("Number"),
        Type::Integer => out.push_str("Integer"),
        Type::Rational => out.push_str("Rational"),
        Type::F64 => out.push_str("F64"),
        Type::F32 => out.push_str("F32"),
        Type::I8 => out.push_str("I8"),
        Type::I16 => out.push_str("I16"),
        Type::I32 => out.push_str("I32"),
        Type::I64 => out.push_str("I64"),
        Type::I128 => out.push_str("I128"),
        Type::U8 => out.push_str("U8"),
        Type::U16 => out.push_str("U16"),
        Type::U32 => out.push_str("U32"),
        Type::U64 => out.push_str("U64"),
        Type::U128 => out.push_str("U128"),
        Type::Isize => out.push_str("Isize"),
        Type::Usize => out.push_str("Usize"),
        Type::Complex => out.push_str("Complex"),
        Type::Expr => out.push_str("Expr"),
        Type::Symbol => out.push_str("Symbol"),
        Type::Bool => out.push_str("Bool"),
        Type::String => out.push_str("String"),
        Type::Char => out.push_str("Char"),
        Type::Array(t) => {
            out.push_str("Array<");
            format_type(t, out);
            out.push('>');
        }
        Type::Matrix(t) => {
            out.push_str("Matrix<");
            format_type(t, out);
            out.push('>');
        }
        Type::Tuple(ts) => {
            out.push_str("Tuple<");
            for (i, t) in ts.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                format_type(t, out);
            }
            out.push('>');
        }
        Type::Option(t) => {
            out.push_str("Option<");
            format_type(t, out);
            out.push('>');
        }
        Type::Result(a, b) => {
            out.push_str("Result<");
            format_type(a, out);
            out.push_str(", ");
            format_type(b, out);
            out.push('>');
        }
        Type::Fn { params, ret } => {
            out.push_str("Fn(");
            format_type_list(params, out);
            out.push_str(") -> ");
            format_type(ret, out);
        }
        Type::MFn { params, ret } => {
            out.push_str("MFn(");
            format_type_list(params, out);
            out.push_str(") -> ");
            format_type(ret, out);
        }
        Type::SelfType => out.push_str("Self"),
        Type::User(name) => out.push_str(&name.value),
    }
}

fn format_type_list(types: &[Type], out: &mut String) {
    for (i, t) in types.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        format_type(t, out);
    }
}
