use num_bigint::BigInt;
use num_rational::BigRational;

use crate::builtins::BuiltinSymbols;
use crate::expr_pool::{ExprData, ExprId, ExprPool};
use crate::number::Number;

pub fn simplify(pool: &ExprPool, builtins: &BuiltinSymbols, id: ExprId) -> ExprId {
    let node = match pool.get(id) {
        Some(n) => n,
        None => return id,
    };
    match node {
        ExprData::Add(items) => {
            if items.is_empty() {
                return id;
            }
            let mut acc = simplify(pool, builtins, items[0]);
            for &it in &items[1..] {
                let s = simplify(pool, builtins, it);
                acc = pool.add2(acc, s);
            }
            acc
        }
        ExprData::Mul(items) => {
            if items.is_empty() {
                return id;
            }
            let mut acc = simplify(pool, builtins, items[0]);
            for &it in &items[1..] {
                let s = simplify(pool, builtins, it);
                acc = pool.mul2(acc, s);
            }
            acc
        }
        ExprData::Pow { base, exp } => {
            let b = simplify(pool, builtins, base);
            let e = simplify(pool, builtins, exp);
            if let Some(ExprData::Apply { f, args }) = pool.get(b)
                && f == pool.symbol(builtins.sqrt)
                && args.len() == 1
                && e == pool.integer(2)
            {
                return simplify(pool, builtins, args[0]);
            }
            if b == pool.symbol(builtins.e) && let Some(r) = euler(pool, builtins, e) {
                return r;
            }
            pool.pow2(b, e)
        }
        ExprData::Apply { f, args } => {
            let mut new_args = Vec::with_capacity(args.len());
            for &a in args.iter() {
                new_args.push(simplify(pool, builtins, a));
            }
            if let Some(r) = apply_rule(pool, builtins, f, &new_args) {
                r
            } else {
                pool.apply(f, &new_args)
            }
        }
        _ => id,
    }
}

fn apply_rule(pool: &ExprPool, builtins: &BuiltinSymbols, f: ExprId, args: &[ExprId]) -> Option<ExprId> {
    if args.len() != 1 {
        return None;
    }
    let arg = args[0];
    if f == pool.symbol(builtins.sqrt) {
        if let Some(n) = pool.const_number(arg) && let Some(s) = n.sqrt() {
            return Some(pool.number(&s));
        }
        return None;
    }
    let sin = pool.symbol(builtins.sin);
    let cos = pool.symbol(builtins.cos);
    let tan = pool.symbol(builtins.tan);
    if f == sin || f == cos || f == tan {
        if let Some((c, s)) = trig_of_angle(pool, builtins, arg) {
            if f == sin {
                return Some(pool.number(&s));
            }
            if f == cos {
                return Some(pool.number(&c));
            }
            if !c.is_zero() {
                return Some(pool.number(&(s / c)));
            }
        }
        return None;
    }
    if f == pool.symbol(builtins.exp) {
        if arg == pool.integer(0) {
            return Some(pool.integer(1));
        }
        return None;
    }
    if f == pool.symbol(builtins.log) || f == pool.symbol(builtins.ln) {
        if arg == pool.integer(1) {
            return Some(pool.integer(0));
        }
        if arg == pool.symbol(builtins.e) {
            return Some(pool.integer(1));
        }
        return None;
    }
    if f == pool.symbol(builtins.abs) {
        if let Some(n) = pool.const_number(arg) {
            return Some(pool.number(&n.abs()));
        }
        return None;
    }
    None
}

fn trig_of_angle(pool: &ExprPool, builtins: &BuiltinSymbols, expr: ExprId) -> Option<(Number, Number)> {
    let c = rational_pi_coefficient(pool, builtins, expr)?;
    exact_trig(&c)
}

fn rational_pi_coefficient(pool: &ExprPool, builtins: &BuiltinSymbols, expr: ExprId) -> Option<BigRational> {
    let node = pool.get(expr)?;
    match node {
        ExprData::Symbol(s) if s == builtins.pi => Some(BigRational::new(BigInt::from(1), BigInt::from(1))),
        ExprData::Integer(_) | ExprData::Rational(_) | ExprData::Real(_) => {
            let n = pool.const_number(expr)?;
            if n.is_zero() {
                Some(BigRational::new(BigInt::from(0), BigInt::from(1)))
            } else {
                None
            }
        }
        ExprData::Mul(items) => {
            let mut coeff: Option<BigRational> = None;
            let mut found_pi = false;
            for &it in items.iter() {
                match pool.get(it)? {
                    ExprData::Symbol(s) if s == builtins.pi => found_pi = true,
                    ExprData::Integer(_) | ExprData::Rational(_) => {
                        let c = match pool.const_number(it)? {
                            Number::Integer(i) => BigRational::from_integer(i),
                            Number::Rational(r) => r,
                            _ => return None,
                        };
                        coeff = Some(match coeff {
                            Some(acc) => acc * c,
                            None => c,
                        });
                    }
                    _ => return None,
                }
            }
            if !found_pi {
                return None;
            }
            Some(coeff.unwrap_or_else(|| BigRational::new(BigInt::from(1), BigInt::from(1))))
        }
        _ => None,
    }
}

fn exact_trig(c: &BigRational) -> Option<(Number, Number)> {
    let two = BigRational::new(BigInt::from(2), BigInt::from(1));
    let mut c = c % two.clone();
    if c < BigRational::new(BigInt::from(0), BigInt::from(1)) {
        c += two;
    }
    let zero = BigRational::new(BigInt::from(0), BigInt::from(1));
    let half = BigRational::new(BigInt::from(1), BigInt::from(2));
    let one = BigRational::new(BigInt::from(1), BigInt::from(1));
    let three_halves = BigRational::new(BigInt::from(3), BigInt::from(2));
    if c == zero {
        Some((Number::from(1), Number::from(0)))
    } else if c == half {
        Some((Number::from(0), Number::from(1)))
    } else if c == one {
        Some((Number::from(-1), Number::from(0)))
    } else if c == three_halves {
        Some((Number::from(0), Number::from(-1)))
    } else {
        None
    }
}

fn euler(pool: &ExprPool, builtins: &BuiltinSymbols, z: ExprId) -> Option<ExprId> {
    let i = pool.symbol(builtins.i);
    let theta = match pool.get(z)? {
        ExprData::Symbol(s) if s == builtins.i => return None,
        ExprData::Mul(items) => {
            let mut theta_items = Vec::new();
            let mut has_i = false;
            for &it in items.iter() {
                if it == i {
                    has_i = true;
                } else {
                    theta_items.push(it);
                }
            }
            if !has_i || theta_items.is_empty() {
                return None;
            }
            let mut acc = theta_items[0];
            for &it in &theta_items[1..] {
                acc = pool.mul2(acc, it);
            }
            acc
        }
        _ => return None,
    };
    let (c, s) = trig_of_angle(pool, builtins, theta)?;
    if s == Number::from(0) {
        Some(pool.number(&c))
    } else if c == Number::from(0) && s == Number::from(1) {
        Some(i)
    } else if c == Number::from(0) && s == Number::from(-1) {
        Some(pool.mul2(pool.integer(-1), i))
    } else {
        None
    }
}
