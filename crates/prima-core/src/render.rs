use num_bigint::BigInt;
use num_rational::BigRational;
use num_traits::Signed;

use crate::expr_pool::{ExprData, ExprId, ExprPool};
use crate::number::{Number, Real};
use crate::symbol::SymbolTable;

pub fn render_number(n: &Number) -> String {
    match n {
        Number::Integer(i) => i.to_string(),
        Number::Rational(r) => format!("\\frac{{{}}}{{{}}}", r.numer(), r.denom()),
        Number::Real(Real::F64(f)) => f.to_string(),
        Number::Real(Real::F32(f)) => f.to_string(),
        Number::Complex { re, im } => format!("{} + {}i", render_number(re), render_number(im)),
    }
}

pub fn render_latex(pool: &ExprPool, symbols: &SymbolTable, id: ExprId) -> String {
    match pool.get(id) {
        Some(ExprData::Symbol(s)) => symbols.name(s).unwrap_or_else(|| format!("?{}", s.0)),
        Some(ExprData::Integer(i)) => i.to_string(),
        Some(ExprData::Rational(r)) => render_number(&Number::Rational(*r)),
        Some(ExprData::Real(Real::F64(f))) => f.to_string(),
        Some(ExprData::Real(Real::F32(f))) => f.to_string(),
        Some(ExprData::Add(items)) => render_add(pool, symbols, &items),
        Some(ExprData::Mul(items)) => render_mul(pool, symbols, &items),
        Some(ExprData::Pow { base, exp }) => render_pow(pool, symbols, base, exp),
        Some(ExprData::Apply { f, args }) => render_apply(pool, symbols, f, &args),
        Some(ExprData::Indeterminate(_)) => "\\text{indeterminate}".into(),
        None => "?".into(),
    }
}

fn render_add(pool: &ExprPool, symbols: &SymbolTable, items: &[ExprId]) -> String {
    let mut ordered: Vec<ExprId> = items.to_vec();
    ordered.sort_by_key(|&id| {
        match pool.get(id) {
            Some(ExprData::Integer(_) | ExprData::Rational(_) | ExprData::Real(_)) => (1u8, 0u8),
            Some(ExprData::Symbol(_)) => (0u8, 1u8),
            _ => (0u8, 0u8),
        }
    });
    let mut parts = Vec::new();
    for (i, &item) in ordered.iter().enumerate() {
        let s = render_signed(pool, symbols, item);
        if i == 0 {
            let trimmed = s.strip_prefix("+ ").unwrap_or(&s);
            parts.push(trimmed.to_string());
        } else {
            parts.push(s);
        }
    }
    parts.join(" ")
}

fn render_signed(pool: &ExprPool, symbols: &SymbolTable, id: ExprId) -> String {
    match pool.get(id) {
        Some(ExprData::Integer(i)) if *i < BigInt::from(0) => format!("- {}", -(*i)),
        Some(ExprData::Rational(r)) if *r < BigRational::new(BigInt::from(0), BigInt::from(1)) => {
            format!("- {}", render_number(&Number::Rational(r.abs())))
        }
        Some(ExprData::Real(Real::F64(f))) if f < 0.0 => format!("- {}", -f),
        Some(ExprData::Real(Real::F32(f))) if f < 0.0 => format!("- {}", -f),
        _ => {
            let s = render_latex(pool, symbols, id);
            if s.starts_with('-') {
                s
            } else {
                format!("+ {s}")
            }
        }
    }
}

fn render_mul(pool: &ExprPool, symbols: &SymbolTable, items: &[ExprId]) -> String {
    let mut neg = false;
    let mut parts = Vec::new();
    for &item in items.iter() {
        match pool.get(item) {
            Some(ExprData::Integer(i)) if *i == BigInt::from(-1) && items.len() > 1 => neg = !neg,
            _ => {
                let s = render_latex(pool, symbols, item);
                if is_atomic(pool, item) {
                    parts.push(s);
                } else {
                    parts.push(format!("\\left({s}\\right)"));
                }
            }
        }
    }
    let body = if parts.is_empty() { "1".to_string() } else { parts.join(" ") };
    if neg {
        format!("-{body}")
    } else {
        body
    }
}

fn is_atomic(pool: &ExprPool, id: ExprId) -> bool {
    matches!(pool.get(id), Some(ExprData::Symbol(_) | ExprData::Integer(_) | ExprData::Rational(_) | ExprData::Real(_)))
}

fn render_pow(pool: &ExprPool, symbols: &SymbolTable, base: ExprId, exp: ExprId) -> String {
    let half = BigRational::new(BigInt::from(1), BigInt::from(2));
    if matches!(pool.const_number(exp), Some(Number::Rational(r)) if r == half) {
        return format!("\\sqrt{{{}}}", render_latex(pool, symbols, base));
    }
    let base_s = render_latex(pool, symbols, base);
    let base_s = if is_atomic(pool, base) { base_s } else { format!("\\left({base_s}\\right)") };
    format!("{base_s}^{{{}}}", render_latex(pool, symbols, exp))
}

fn render_apply(pool: &ExprPool, symbols: &SymbolTable, f: ExprId, args: &[ExprId]) -> String {
    let name = match pool.get(f) {
        Some(ExprData::Symbol(s)) => symbols.name(s).unwrap_or_else(|| "f".to_string()),
        _ => "f".to_string(),
    };
    let arg_strs: Vec<String> = args.iter().map(|&a| render_latex(pool, symbols, a)).collect();
    if name == "\\sqrt" && args.len() == 1 {
        return format!("\\sqrt{{{}}}", arg_strs[0]);
    }
    format!("{name}\\left({}\\right)", arg_strs.join(", "))
}
