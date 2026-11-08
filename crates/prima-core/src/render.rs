use num_bigint::BigInt;
use num_rational::BigRational;
use num_traits::Signed;

use crate::expr_pool::{ExprData, ExprId, ExprPool};
use crate::number::{Number, Real};
use crate::symbol::SymbolTable;

/// LaTeX view of a number (spec §8.3 default `print_format := latex`): rationals render as `\frac{n}{d}`.
pub fn render_number(n: &Number) -> String {
    match n {
        Number::Small(v) => v.to_string(),
        Number::Integer(i) => i.to_string(),
        Number::Rational(r) => format!("\\frac{{{}}}{{{}}}", r.numer(), r.denom()),
        Number::Real(Real::F64(f)) => f.to_string(),
        Number::Real(Real::F32(f)) => f.to_string(),
        Number::Complex(c) => format!("{} + {}i", render_number(&c.re), render_number(&c.im)),
        Number::I8(v) => v.to_string(),
        Number::I16(v) => v.to_string(),
        Number::I32(v) => v.to_string(),
        Number::I64(v) => v.to_string(),
        Number::I128(v) => v.to_string(),
        Number::U8(v) => v.to_string(),
        Number::U16(v) => v.to_string(),
        Number::U32(v) => v.to_string(),
        Number::U64(v) => v.to_string(),
        Number::U128(v) => v.to_string(),
        Number::Isize(v) => v.to_string(),
        Number::Usize(v) => v.to_string(),
        Number::BigFloat(f) => f.to_string(),
    }
}

/// Plain text view of a number, using Unicode or ASCII mathematical notation.
pub fn render_number_text(n: &Number) -> String {
    match n {
        Number::Rational(r) => format!("({})/({})", r.numer(), r.denom()),
        Number::Complex(c) => format!(
            "{} + {}i",
            render_number_text(&c.re),
            render_number_text(&c.im)
        ),
        _ => render_number(n),
    }
}

/// Plain text view of an expression, preserving operator grouping.
pub fn render_text(pool: &ExprPool, symbols: &SymbolTable, id: ExprId, unicode: bool) -> String {
    fn name(s: &str, unicode: bool) -> String {
        let bare = s.strip_prefix('\\').unwrap_or(s);
        if unicode {
            match bare {
                "alpha" => "α",
                "beta" => "β",
                "gamma" => "γ",
                "delta" => "δ",
                "epsilon" => "ε",
                "zeta" => "ζ",
                "eta" => "η",
                "theta" => "θ",
                "iota" => "ι",
                "kappa" => "κ",
                "lambda" => "λ",
                "mu" => "μ",
                "nu" => "ν",
                "xi" => "ξ",
                "pi" => "π",
                "rho" => "ρ",
                "sigma" => "σ",
                "tau" => "τ",
                "upsilon" => "υ",
                "phi" => "φ",
                "chi" => "χ",
                "psi" => "ψ",
                "omega" => "ω",
                "Gamma" => "Γ",
                "Delta" => "Δ",
                "Theta" => "Θ",
                "Lambda" => "Λ",
                "Xi" => "Ξ",
                "Pi" => "Π",
                "Sigma" => "Σ",
                "Phi" => "Φ",
                "Psi" => "Ψ",
                "Omega" => "Ω",
                _ => bare,
            }
            .to_string()
        } else {
            bare.chars()
                .flat_map(|c| {
                    if c.is_ascii() {
                        c.to_string()
                    } else {
                        format!("\\u{{{:x}}}", c as u32)
                    }
                    .chars()
                    .collect::<Vec<_>>()
                })
                .collect()
        }
    }
    fn go(pool: &ExprPool, symbols: &SymbolTable, id: ExprId, unicode: bool, parent: u8) -> String {
        let (prec, text) = match pool.get(id) {
            Some(ExprData::Symbol(s)) => (
                4,
                name(
                    &symbols.name(s).unwrap_or_else(|| format!("?{}", s.0)),
                    unicode,
                ),
            ),
            Some(ExprData::Integer(i)) => (if i.is_negative() { 2 } else { 4 }, i.to_string()),
            Some(ExprData::Rational(r)) => {
                let sign = if r.is_negative() { "-" } else { "" };
                (2, format!("{sign}({})/({})", r.numer().abs(), r.denom()))
            }
            Some(ExprData::Real(Real::F64(f))) => (if f < 0.0 { 2 } else { 4 }, f.to_string()),
            Some(ExprData::Real(Real::F32(f))) => (if f < 0.0 { 2 } else { 4 }, f.to_string()),
            Some(ExprData::Add(items)) => {
                let mut text = String::new();
                for &item in items.iter() {
                    let term = go(pool, symbols, item, unicode, 1);
                    if text.is_empty() {
                        text.push_str(&term);
                    } else if let Some(rest) = term.strip_prefix('-') {
                        text.push_str(" - ");
                        text.push_str(rest);
                    } else {
                        text.push_str(" + ");
                        text.push_str(&term);
                    }
                }
                (1, text)
            }
            Some(ExprData::Mul(items)) => {
                let mut neg = false;
                let mut parts = Vec::new();
                for &item in items.iter() {
                    if items.len() > 1
                        && matches!(pool.get(item), Some(ExprData::Integer(i)) if *i == BigInt::from(-1))
                    {
                        neg = !neg;
                    } else {
                        parts.push(go(pool, symbols, item, unicode, 2));
                    }
                }
                let body = if parts.is_empty() {
                    "1".to_string()
                } else {
                    parts.join(if unicode { " · " } else { " * " })
                };
                let text = if neg {
                    if let Some(rest) = body.strip_prefix('-') {
                        rest.to_string()
                    } else {
                        format!("-{body}")
                    }
                } else {
                    body
                };
                (2, text)
            }
            Some(ExprData::Pow { base, exp }) => (
                3,
                format!(
                    "{}^({})",
                    go(pool, symbols, base, unicode, 4),
                    go(pool, symbols, exp, unicode, 0)
                ),
            ),
            Some(ExprData::Apply { f, args }) => (
                4,
                format!(
                    "{}({})",
                    go(pool, symbols, f, unicode, 4),
                    args.iter()
                        .map(|&i| go(pool, symbols, i, unicode, 0))
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            ),
            Some(ExprData::Indeterminate(_)) => (4, "indeterminate".into()),
            None => (4, "?".into()),
        };
        if prec < parent {
            format!("({text})")
        } else {
            text
        }
    }
    go(pool, symbols, id, unicode, 0)
}

/// ExprDAG → LaTeX view (spec §8.3 level 0, preserving the original form). TeX names come from the symbol table (spec §7);
/// rendering is just a view conversion, decoupled from the reverse parsing of `tex"..."` (spec §4.9).
pub fn render_latex(pool: &ExprPool, symbols: &SymbolTable, id: ExprId) -> String {
    match pool.get(id) {
        Some(ExprData::Symbol(s)) => symbols.name(s).unwrap_or_else(|| format!("?{}", s.0)),
        Some(ExprData::Integer(i)) => i.to_string(),
        Some(ExprData::Rational(r)) => render_number(&Number::Rational(Box::new(*r.clone()))),
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
    ordered.sort_by_key(|&id| match pool.get(id) {
        Some(ExprData::Integer(_) | ExprData::Rational(_) | ExprData::Real(_)) => (1u8, 0u8),
        Some(ExprData::Symbol(_)) => (0u8, 1u8),
        _ => (0u8, 0u8),
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
            format!("- {}", render_number(&Number::Rational(Box::new(r.abs()))))
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
    let body = if parts.is_empty() {
        "1".to_string()
    } else {
        parts.join(" ")
    };
    if neg { format!("-{body}") } else { body }
}

fn is_atomic(pool: &ExprPool, id: ExprId) -> bool {
    matches!(
        pool.get(id),
        Some(
            ExprData::Symbol(_) | ExprData::Integer(_) | ExprData::Rational(_) | ExprData::Real(_)
        )
    )
}

fn render_pow(pool: &ExprPool, symbols: &SymbolTable, base: ExprId, exp: ExprId) -> String {
    let half = BigRational::new(BigInt::from(1), BigInt::from(2));
    if matches!(pool.const_number(exp), Some(Number::Rational(r)) if *r == half) {
        return format!("\\sqrt{{{}}}", render_latex(pool, symbols, base));
    }
    let base_s = render_latex(pool, symbols, base);
    let base_s = if is_atomic(pool, base) {
        base_s
    } else {
        format!("\\left({base_s}\\right)")
    };
    format!("{base_s}^{{{}}}", render_latex(pool, symbols, exp))
}

fn render_apply(pool: &ExprPool, symbols: &SymbolTable, f: ExprId, args: &[ExprId]) -> String {
    let name = match pool.get(f) {
        Some(ExprData::Symbol(s)) => symbols.name(s).unwrap_or_else(|| "f".to_string()),
        _ => "f".to_string(),
    };
    let arg_strs: Vec<String> = args
        .iter()
        .map(|&a| render_latex(pool, symbols, a))
        .collect();
    if name == "\\sqrt" && args.len() == 1 {
        return format!("\\sqrt{{{}}}", arg_strs[0]);
    }
    format!("{name}\\left({}\\right)", arg_strs.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_negative_terms_and_precedence() {
        let pool = ExprPool::new();
        let symbols = SymbolTable::new();
        let x = pool.symbol(symbols.intern("x"));
        let minus_one = pool.integer(-1);
        let minus_three = pool.integer(-3);
        let neg_x = pool.intern(ExprData::Mul(vec![minus_one, x].into_boxed_slice()));
        let sum = pool.intern(ExprData::Add(
            vec![x, minus_three, neg_x].into_boxed_slice(),
        ));
        let neg_sum = pool.intern(ExprData::Mul(vec![minus_one, sum].into_boxed_slice()));
        let power = pool.intern(ExprData::Pow {
            base: neg_x,
            exp: pool.integer(2),
        });
        let rational = pool.intern(ExprData::Rational(Box::new(BigRational::new(
            (-3).into(),
            2.into(),
        ))));
        let rational_sum = pool.intern(ExprData::Add(vec![x, rational].into_boxed_slice()));
        for unicode in [false, true] {
            assert_eq!(render_text(&pool, &symbols, sum, unicode), "x - 3 - x");
            assert_eq!(
                render_text(&pool, &symbols, neg_sum, unicode),
                "-(x - 3 - x)"
            );
            assert_eq!(render_text(&pool, &symbols, power, unicode), "(-x)^(2)");
            assert_eq!(
                render_text(&pool, &symbols, rational_sum, unicode),
                "x - (3)/(2)"
            );
        }
    }
}
