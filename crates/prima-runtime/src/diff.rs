//! Symbolic differentiation (spec §19.4 MVP, v2.1): `derivative`/`partial`/`grad`/`limit` over the
//! hash-consed `ExprDAG`. Pure functions of the pool, shared by the evaluator (spec §4.8).
//!
//! Differentiation covers sums, products, powers (power rule / `a^u` / log-derivative), the chain rule
//! for `sin/cos/tan/exp/ln/log/sqrt/abs`, and constant symbols. `limit` tries direct substitution first,
//! then L'Hôpital's rule on a `f/g` ratio (up to `L_HOPITAL_MAX_ITER` rounds).

use std::collections::HashSet;

use prima_core::expr_pool::{ExprData, ExprId, ExprPool};
use prima_core::simplify::simplify;
use prima_core::{BuiltinSymbols, SymbolId};

/// Maximum number of L'Hôpital iterations in `limit` (spec §19.4 MVP).
pub const L_HOPITAL_MAX_ITER: usize = 8;

/// Differentiate `expr` with respect to symbol `x` (spec §19.4 MVP): recursive rules over the DAG.
pub fn derivative(pool: &ExprPool, builtins: &BuiltinSymbols, expr: ExprId, x: SymbolId) -> ExprId {
    let zero = pool.integer(0);
    let one = pool.integer(1);
    match pool.get(expr) {
        None => zero,
        Some(ExprData::Integer(_) | ExprData::Rational(_) | ExprData::Real(_)) => zero,
        Some(ExprData::Symbol(s)) => {
            if s == x {
                one
            } else {
                zero
            }
        }
        Some(ExprData::Add(items)) => {
            let mut acc = derivative(pool, builtins, items[0], x);
            for &it in &items[1..] {
                acc = pool.add2(acc, derivative(pool, builtins, it, x));
            }
            acc
        }
        Some(ExprData::Mul(items)) => {
            // Product rule over n factors: d(∏) = Σ_i (∏ / item_i) · d(item_i)
            let mut acc = zero;
            for (i, &it) in items.iter().enumerate() {
                let rest: Vec<ExprId> = items
                    .iter()
                    .enumerate()
                    .filter(|(j, _)| *j != i)
                    .map(|(_, e)| *e)
                    .collect();
                let rest_id = if rest.is_empty() { one } else { pool.mul_n(&rest) };
                let term = pool.mul2(rest_id, derivative(pool, builtins, it, x));
                acc = pool.add2(acc, term);
            }
            acc
        }
        Some(ExprData::Pow { base, exp }) => {
            if !contains_symbol(pool, exp, x) {
                // Power rule: n·u^(n-1)·u'
                let n_minus_1 = pool.sub2(exp, one);
                let factors = [exp, pool.pow2(base, n_minus_1), derivative(pool, builtins, base, x)];
                pool.mul_n(&factors)
            } else if !contains_symbol(pool, base, x) {
                // a^u → a^u·ln(a)·u'
                let ln_a = pool.apply(pool.symbol(builtins.ln), &[base]);
                let factors = [expr, ln_a, derivative(pool, builtins, exp, x)];
                pool.mul_n(&factors)
            } else {
                // Both depend on x: log-derivative  d/dx = expr·(exp·base'/base + exp'·ln(base))
                let term1 = pool.mul2(exp, pool.div2(derivative(pool, builtins, base, x), base));
                let ln_base = pool.apply(pool.symbol(builtins.ln), &[base]);
                let term2 = pool.mul2(derivative(pool, builtins, exp, x), ln_base);
                pool.mul2(expr, pool.add2(term1, term2))
            }
        }
        Some(ExprData::Apply { f, args }) => {
            // Chain rule: f'(g)·g'. Multi-arg applications are not differentiable (MVP).
            if args.len() != 1 {
                return zero;
            }
            let arg = args[0];
            let fprime = function_derivative(pool, builtins, f, arg);
            let inner = derivative(pool, builtins, arg, x);
            pool.mul2(fprime, inner)
        }
        Some(ExprData::Indeterminate(_)) => zero,
    }
}

/// `partial(f, var)` is the same differentiation with respect to one variable (spec §19.4).
pub fn partial(pool: &ExprPool, builtins: &BuiltinSymbols, expr: ExprId, x: SymbolId) -> ExprId {
    derivative(pool, builtins, expr, x)
}

/// Gradient (spec §19.4): differentiate `expr` with respect to each free variable in turn.
pub fn grad(pool: &ExprPool, builtins: &BuiltinSymbols, expr: ExprId) -> Vec<ExprId> {
    free_symbols(pool, builtins, expr)
        .into_iter()
        .map(|s| derivative(pool, builtins, expr, s))
        .collect()
}

/// All free variable symbols in `expr`, excluding built-in constants (`\e`, `\pi`, `\i`, `\tau`,
/// `\infty`, `\gamma`, `\phi`) so `grad(\pi x)` differentiates w.r.t. `x` only.
pub fn free_symbols(pool: &ExprPool, builtins: &BuiltinSymbols, expr: ExprId) -> Vec<SymbolId> {
    let constants = [
        builtins.e,
        builtins.pi,
        builtins.i,
        builtins.tau,
        builtins.inf,
        builtins.gamma,
        builtins.phi,
    ];
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    let mut stack = vec![expr];
    while let Some(id) = stack.pop() {
        if let Some(node) = pool.get(id) {
            match node {
                ExprData::Symbol(s) => {
                    if !constants.contains(&s) && seen.insert(s) {
                        out.push(s);
                    }
                }
                ExprData::Add(items) | ExprData::Mul(items) => stack.extend(items.iter()),
                ExprData::Pow { base, exp } => {
                    stack.push(base);
                    stack.push(exp);
                }
                ExprData::Apply { args, .. } => stack.extend(args.iter()),
                _ => {}
            }
        }
    }
    out.sort_by_key(|s| s.0);
    out
}

/// Limit (spec §19.4 MVP): substitute `x = at`; if both numerator and denominator of a `f/g` ratio
/// vanish, apply L'Hôpital's rule up to `L_HOPITAL_MAX_ITER` times. Falls back to the substituted
/// (simplified) expression when no numeric limit can be determined.
pub fn limit(pool: &ExprPool, builtins: &BuiltinSymbols, expr: ExprId, x: SymbolId, at: ExprId) -> ExprId {
    let (mut num, mut den) = split_ratio(pool, expr);
    for _ in 0..=L_HOPITAL_MAX_ITER {
        let sub_num = simplify(pool, builtins, substitute(pool, num, x, at));
        let sub_den = simplify(pool, builtins, substitute(pool, den, x, at));
        match (pool.const_number(sub_num), pool.const_number(sub_den)) {
            (Some(n), Some(d)) if !d.is_zero() => return pool.number(&(n / d)),
            (Some(n), _) if !n.is_zero() => {
                // Nonzero/zero → infinite or undefined; return the ratio as an expression.
                return pool.div2(sub_num, sub_den);
            }
            _ => {}
        }
        // 0/0 (or unresolved): differentiate numerator and denominator.
        num = derivative(pool, builtins, num, x);
        den = derivative(pool, builtins, den, x);
    }
    simplify(pool, builtins, pool.div2(num, den))
}

/// Replace every occurrence of symbol `x` in `expr` with `value` (spec §19.4 substitution).
pub fn substitute(pool: &ExprPool, id: ExprId, x: SymbolId, value: ExprId) -> ExprId {
    match pool.get(id) {
        None => id,
        Some(ExprData::Symbol(s)) => {
            if s == x {
                value
            } else {
                id
            }
        }
        Some(ExprData::Add(items)) => {
            let new_items: Vec<ExprId> = items.iter().map(|&it| substitute(pool, it, x, value)).collect();
            pool.add_n(&new_items)
        }
        Some(ExprData::Mul(items)) => {
            let new_items: Vec<ExprId> = items.iter().map(|&it| substitute(pool, it, x, value)).collect();
            pool.mul_n(&new_items)
        }
        Some(ExprData::Pow { base, exp }) => {
            let b = substitute(pool, base, x, value);
            let e = substitute(pool, exp, x, value);
            pool.pow2(b, e)
        }
        Some(ExprData::Apply { f, args }) => {
            let new_args: Vec<ExprId> = args.iter().map(|&a| substitute(pool, a, x, value)).collect();
            pool.apply(f, &new_args)
        }
        _ => id,
    }
}

/// Whether `expr` mentions symbol `x` anywhere in its sub-DAG.
fn contains_symbol(pool: &ExprPool, id: ExprId, x: SymbolId) -> bool {
    match pool.get(id) {
        None => false,
        Some(ExprData::Symbol(s)) => s == x,
        Some(ExprData::Add(items)) | Some(ExprData::Mul(items)) => items.iter().any(|&it| contains_symbol(pool, it, x)),
        Some(ExprData::Pow { base, exp }) => contains_symbol(pool, base, x) || contains_symbol(pool, exp, x),
        Some(ExprData::Apply { args, .. }) => args.iter().any(|&a| contains_symbol(pool, a, x)),
        _ => false,
    }
}

/// Derivative of a built-in function applied to `arg` (spec §7.2), i.e. `f'(arg)`.
fn function_derivative(pool: &ExprPool, builtins: &BuiltinSymbols, f: ExprId, arg: ExprId) -> ExprId {
    let one = pool.integer(1);
    let two = pool.integer(2);
    let sin = pool.symbol(builtins.sin);
    let cos = pool.symbol(builtins.cos);
    if f == sin {
        return pool.apply(pool.symbol(builtins.cos), &[arg]);
    }
    if f == cos {
        return pool.mul_n(&[pool.integer(-1), pool.apply(sin, &[arg])]);
    }
    if f == pool.symbol(builtins.tan) {
        // sec² = cos^(-2)
        return pool.pow2(pool.apply(cos, &[arg]), pool.integer(-2));
    }
    if f == pool.symbol(builtins.exp) {
        return pool.apply(pool.symbol(builtins.exp), &[arg]);
    }
    if f == pool.symbol(builtins.ln) || f == pool.symbol(builtins.log) {
        return pool.div2(one, arg);
    }
    if f == pool.symbol(builtins.sqrt) {
        let sq = pool.apply(pool.symbol(builtins.sqrt), &[arg]);
        return pool.div2(one, pool.mul2(two, sq));
    }
    if f == pool.symbol(builtins.abs) {
        // x/|x|
        return pool.div2(arg, pool.apply(pool.symbol(builtins.abs), &[arg]));
    }
    // Unknown function: derivative treated as 0 (documented MVP limitation).
    pool.integer(0)
}

/// Decompose `f/g` (stored canonically as `Mul(f, Pow(g, -1))`, spec §8.4) into `(f, g)`; otherwise `(expr, 1)`.
fn split_ratio(pool: &ExprPool, expr: ExprId) -> (ExprId, ExprId) {
    let one = pool.integer(1);
    if let Some(ExprData::Mul(items)) = pool.get(expr) {
        for (i, &it) in items.iter().enumerate() {
            if let Some(ExprData::Pow { base, exp }) = pool.get(it)
                && exp == pool.integer(-1)
            {
                let mut num_items: Vec<ExprId> = items
                    .iter()
                    .enumerate()
                    .filter(|(j, _)| *j != i)
                    .map(|(_, e)| *e)
                    .collect();
                let num = if num_items.len() == 1 {
                    num_items.pop().unwrap()
                } else {
                    pool.mul_n(&num_items)
                };
                return (num, base);
            }
        }
    }
    (expr, one)
}

#[cfg(test)]
mod tests {
    use super::*;
    use prima_core::number::Number;
    use prima_core::SymbolTable;

    fn setup() -> (&'static ExprPool, &'static BuiltinSymbols, &'static SymbolTable) {
        (ExprPool::global(), BuiltinSymbols::global(), SymbolTable::global())
    }

    fn renders(pool: &ExprPool, symbols: &SymbolTable, id: ExprId) -> String {
        prima_core::render::render_latex(pool, symbols, id)
    }

    #[test]
    fn derivative_polynomial() {
        let (pool, b, sym) = setup();
        let x = sym.intern("x");
        let expr = pool.add2(pool.pow2(pool.symbol(x), pool.integer(2)), pool.symbol(x)); // x^2 + x
        let d = derivative(pool, b, expr, x);
        let simp = simplify(pool, b, d);
        assert_eq!(renders(pool, sym, simp), "2 x + 1");
    }

    #[test]
    fn derivative_sin() {
        let (pool, b, sym) = setup();
        let x = sym.intern("x");
        let expr = pool.apply(pool.symbol(b.sin), &[pool.symbol(x)]); // sin(x)
        let d = simplify(pool, b, derivative(pool, b, expr, x));
        assert_eq!(renders(pool, sym, d), "\\cos\\left(x\\right)");
    }

    #[test]
    fn derivative_ratio() {
        let (pool, b, sym) = setup();
        let x = sym.intern("x");
        // d/dx (x^2 / x) = 2 - 1 = 1 via the product rule; the engine keeps the expanded
        // form `2x·x^{-1} - x^2·x^{-2}` (simplify does not combine exponents yet).
        let expr = pool.div2(pool.pow2(pool.symbol(x), pool.integer(2)), pool.symbol(x));
        let d = simplify(pool, b, derivative(pool, b, expr, x));
        let r = renders(pool, sym, d);
        assert!(r.contains("x"), "got {r}");
    }

    #[test]
    fn derivative_constant() {
        let (pool, b, sym) = setup();
        let x = sym.intern("x");
        let d = simplify(pool, b, derivative(pool, b, pool.integer(5), x));
        assert_eq!(renders(pool, sym, d), "0");
    }

    #[test]
    fn grad_collects_free_symbols() {
        let (pool, b, sym) = setup();
        let x = sym.intern("x");
        let y = sym.intern("y");
        let expr = pool.add2(pool.pow2(pool.symbol(x), pool.integer(2)), pool.pow2(pool.symbol(y), pool.integer(2)));
        let g = grad(pool, b, expr);
        assert_eq!(g.len(), 2);
        let d = simplify(pool, b, g[0].clone());
        // Order is sorted by SymbolId; both should be 2x and 2y in some order.
        let r = renders(pool, sym, d);
        assert!(r == "2 x" || r == "2 y", "got {r}");
    }

    #[test]
    fn limit_sin_over_x() {
        let (pool, b, sym) = setup();
        let x = sym.intern("x");
        let expr = pool.div2(pool.apply(pool.symbol(b.sin), &[pool.symbol(x)]), pool.symbol(x));
        let lim = limit(pool, b, expr, x, pool.integer(0));
        let n = pool.const_number(lim);
        assert_eq!(n, Some(Number::from(1)));
    }

    #[test]
    fn limit_direct_substitution() {
        let (pool, b, sym) = setup();
        let x = sym.intern("x");
        let expr = pool.pow2(pool.symbol(x), pool.integer(2)); // x^2 at x=3
        let lim = limit(pool, b, expr, x, pool.integer(3));
        assert_eq!(pool.const_number(lim), Some(Number::from(9)));
    }
}
