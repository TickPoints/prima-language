//! `math` module (spec §18.6): integer number theory (`gcd`/`lcm`/`factor`/`primes`/`crt`/
//! `mod_pow`) and polynomial/series numeric tools (`poly_*`, `continued_fraction`).
//!
//! Integer kernels reuse the shared arbitrary-precision helpers from `num`; polynomial
//! coefficients are `F64` and **lowest degree first** (index 0 is the constant term), with the
//! zero polynomial represented by the empty array. `primes` is a layered `@builtin(O1)` (spec
//! §18.4): the Rust sieve runs at `opt_level >= O1`, an identical `.pra` body otherwise.

use std::cmp::Ordering;

use num_bigint::BigInt;
use prima_core::{Number, Real, Value};
use prima_runtime::builtin;
use prima_runtime::{Evaluator, RuntimeError};

/// Largest `limit` accepted by `primes` (bounds the sieve allocation to ~50 MB of flags).
const MAX_SIEVE_LIMIT: usize = 50_000_000;
/// Largest term count accepted by `continued_fraction` (bounds the output allocation).
const MAX_CF_TERMS: usize = 1_000_000;

/// Register the `math` `@builtin` implementations (spec §18.4 / §18.6). Each `@builtin`
/// declaration in the embedded `math.pra` signature module binds to the implementation registered
/// under its fully-qualified `math::<name>` key.
pub fn register() {
    builtin!("math::gcd", gcd);
    builtin!("math::lcm", lcm);
    builtin!("math::factor", factor);
    // Layered `@builtin(O1)` (spec §18.4): the Rust sieve, plus an identical `.pra` fallback body.
    builtin!("math::primes", primes, O1);
    builtin!("math::crt", crt);
    builtin!("math::mod_pow", mod_pow);
    builtin!("math::poly_eval", poly_eval);
    builtin!("math::poly_add", poly_add);
    builtin!("math::poly_mul", poly_mul);
    builtin!("math::poly_derivative", poly_derivative);
    builtin!("math::poly_roots", poly_roots);
    builtin!("math::continued_fraction", continued_fraction);
}

// —— argument helpers (mirroring the `num`/`stats` module conventions) ——

fn arity(args: &[Value], n: usize, fname: &str) -> Result<(), RuntimeError> {
    if args.len() == n {
        Ok(())
    } else {
        Err(RuntimeError::Message(format!(
            "`{fname}` expects {n} argument(s), got {}",
            args.len()
        )))
    }
}

fn int_arg(args: &[Value], i: usize, fname: &str) -> Result<BigInt, RuntimeError> {
    match args.get(i) {
        Some(Value::Number(n)) => n.as_bigint().ok_or_else(|| {
            RuntimeError::Type(format!(
                "`{fname}` argument {i} must be an integer, got {n}"
            ))
        }),
        Some(other) => Err(RuntimeError::Type(format!(
            "`{fname}` argument {i} must be an integer, got {other:?}"
        ))),
        None => Err(RuntimeError::Message(format!(
            "`{fname}` missing argument {i}"
        ))),
    }
}

fn f64_arg(args: &[Value], i: usize, fname: &str) -> Result<f64, RuntimeError> {
    match args.get(i) {
        Some(Value::Number(n)) => Ok(n.to_f64_lossy()),
        Some(other) => Err(RuntimeError::Type(format!(
            "`{fname}` argument {i} must be a number, got {other:?}"
        ))),
        None => Err(RuntimeError::Message(format!(
            "`{fname}` missing argument {i}"
        ))),
    }
}

/// Extract an `Array<F64>` argument to a `Vec<f64>`.
fn f64_slice_arg(args: &[Value], i: usize, fname: &str) -> Result<Vec<f64>, RuntimeError> {
    match args.get(i) {
        Some(Value::Array(items)) => items
            .iter()
            .enumerate()
            .map(|(j, v)| match v {
                Value::Number(n) => Ok(n.to_f64_lossy()),
                other => Err(RuntimeError::Type(format!(
                    "`{fname}` element {j} must be a number, got {other:?}"
                ))),
            })
            .collect(),
        Some(other) => Err(RuntimeError::Type(format!(
            "`{fname}` argument {i} must be an array of numbers, got {other:?}"
        ))),
        None => Err(RuntimeError::Message(format!(
            "`{fname}` missing argument {i}"
        ))),
    }
}

/// Extract an `Array<Integer>` argument to a `Vec<BigInt>`.
fn int_slice_arg(args: &[Value], i: usize, fname: &str) -> Result<Vec<BigInt>, RuntimeError> {
    match args.get(i) {
        Some(Value::Array(items)) => items
            .iter()
            .enumerate()
            .map(|(j, v)| match v {
                Value::Number(n) => n.as_bigint().ok_or_else(|| {
                    RuntimeError::Type(format!("`{fname}` element {j} must be an integer, got {n}"))
                }),
                other => Err(RuntimeError::Type(format!(
                    "`{fname}` element {j} must be an integer, got {other:?}"
                ))),
            })
            .collect(),
        Some(other) => Err(RuntimeError::Type(format!(
            "`{fname}` argument {i} must be an array of integers, got {other:?}"
        ))),
        None => Err(RuntimeError::Message(format!(
            "`{fname}` missing argument {i}"
        ))),
    }
}

fn big_value(n: BigInt) -> Value {
    Value::Number(Number::from_bigint(n))
}

fn f64_value(x: f64) -> Value {
    Value::Number(Number::Real(Real::F64(x)))
}

fn result_err(msg: impl Into<String>) -> Value {
    Value::Result(Err(Box::new(msg.into())))
}

/// Non-negative conversion to `usize` through the decimal form (no `num-traits` dependency);
/// `None` for negative or out-of-range values.
fn bigint_to_usize(v: &BigInt) -> Option<usize> {
    v.to_str_radix(10).parse::<usize>().ok()
}

// —— integer number theory (spec §18.6) ——

fn gcd(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    arity(args, 2, "math::gcd")?;
    let a = int_arg(args, 0, "math::gcd")?;
    let b = int_arg(args, 1, "math::gcd")?;
    Ok(big_value(crate::num::bigint_gcd(&a, &b)))
}

fn lcm(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    arity(args, 2, "math::lcm")?;
    let a = int_arg(args, 0, "math::lcm")?;
    let b = int_arg(args, 1, "math::lcm")?;
    let g = crate::num::bigint_gcd(&a, &b);
    let result = if g == BigInt::from(0) {
        BigInt::from(0)
    } else {
        crate::num::bigint_abs(a) * (crate::num::bigint_abs(b) / g)
    };
    Ok(big_value(result))
}

/// Prime factorization by trial division: `2`, then odd divisors up to `sqrt(m)`. Ascending with
/// multiplicity; `n <= 0` is rejected.
fn factor(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    arity(args, 1, "math::factor")?;
    let n = int_arg(args, 0, "math::factor")?;
    if n <= BigInt::from(0) {
        return Err(RuntimeError::Message(
            "`math::factor` expects a positive integer".into(),
        ));
    }
    let zero = BigInt::from(0);
    let two = BigInt::from(2);
    let mut out: Vec<Value> = Vec::new();
    let mut m = n;
    while &m % &two == zero {
        out.push(big_value(two.clone()));
        m /= &two;
    }
    let mut d = BigInt::from(3);
    while &d * &d <= m {
        while &m % &d == zero {
            out.push(big_value(d.clone()));
            m /= &d;
        }
        d += &two;
    }
    if m > BigInt::from(1) {
        out.push(big_value(m));
    }
    Ok(Value::Array(out.into()))
}

/// Sieve of Eratosthenes: all primes `<= limit`, ascending (empty when `limit < 2`).
fn primes(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    arity(args, 1, "math::primes")?;
    let limit = int_arg(args, 0, "math::primes")?;
    if limit < BigInt::from(2) {
        return Ok(Value::Array(Vec::new().into()));
    }
    let limit = bigint_to_usize(&limit)
        .filter(|&v| v <= MAX_SIEVE_LIMIT)
        .ok_or_else(|| {
            RuntimeError::Message(format!(
                "`math::primes` limit must be at most {MAX_SIEVE_LIMIT}"
            ))
        })?;
    let mut sieve = vec![true; limit + 1];
    sieve[0] = false;
    sieve[1] = false;
    let mut p = 2usize;
    while p * p <= limit {
        if sieve[p] {
            let mut multiple = p * p;
            while multiple <= limit {
                sieve[multiple] = false;
                multiple += p;
            }
        }
        p += 1;
    }
    let out: Vec<Value> = (0..=limit)
        .filter(|&i| sieve[i])
        .map(|i| big_value(BigInt::from(i as u64)))
        .collect();
    Ok(Value::Array(out.into()))
}

/// Chinese remainder theorem (spec §18.6): the least non-negative `x` with
/// `x ≡ residues[i] (mod moduli[i])`. Failures are reported as `Result::Err` (length mismatch,
/// zero modulus, non-pairwise-coprime moduli).
fn crt(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    arity(args, 2, "math::crt")?;
    let residues = int_slice_arg(args, 0, "math::crt")?;
    let moduli = int_slice_arg(args, 1, "math::crt")?;
    if residues.len() != moduli.len() {
        return Ok(result_err(
            "`math::crt` requires residues and moduli of equal length",
        ));
    }
    if residues.is_empty() {
        return Ok(result_err("`math::crt` requires at least one congruence"));
    }
    let zero = BigInt::from(0);
    let one = BigInt::from(1);
    let mut ms = Vec::with_capacity(moduli.len());
    for m in &moduli {
        if *m == zero {
            return Ok(result_err("`math::crt` moduli must be non-zero"));
        }
        ms.push(crate::num::bigint_abs(m.clone()));
    }
    for (i, mi) in ms.iter().enumerate() {
        for mj in &ms[i + 1..] {
            if crate::num::bigint_gcd(mi, mj) != one {
                return Ok(result_err("`math::crt` moduli must be pairwise coprime"));
            }
        }
    }
    let mut combined = BigInt::from(1);
    for m in &ms {
        combined *= m;
    }
    let mut acc = BigInt::from(0);
    for (i, mi) in ms.iter().enumerate() {
        let mi_part = &combined / mi;
        let Some(inv) = mod_inverse(&mi_part, mi) else {
            return Ok(result_err("`math::crt` is unsolvable"));
        };
        let ri = mod_floor(&residues[i], mi);
        acc += ri * mi_part * inv;
    }
    let solution = mod_floor(&acc, &combined);
    Ok(Value::Result(Ok(Box::new(big_value(solution)))))
}

/// Fast modular exponentiation (spec §18.6); `exp >= 0` and `modulus != 0` are required. The
/// result is the least non-negative residue modulo `|modulus|`.
fn mod_pow(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    arity(args, 3, "math::mod_pow")?;
    let base = int_arg(args, 0, "math::mod_pow")?;
    let exp = int_arg(args, 1, "math::mod_pow")?;
    let modulus = int_arg(args, 2, "math::mod_pow")?;
    if exp < BigInt::from(0) {
        return Err(RuntimeError::Message(
            "`math::mod_pow` requires exp >= 0".into(),
        ));
    }
    if modulus == BigInt::from(0) {
        return Err(RuntimeError::Message(
            "`math::mod_pow` requires a non-zero modulus".into(),
        ));
    }
    let m = crate::num::bigint_abs(modulus);
    if m == BigInt::from(1) {
        return Ok(big_value(BigInt::from(0)));
    }
    let two = BigInt::from(2);
    let mut result = BigInt::from(1);
    let mut b = mod_floor(&base, &m);
    let mut e = exp;
    while e > BigInt::from(0) {
        if &e % &two != BigInt::from(0) {
            result = mod_floor(&(&result * &b), &m);
        }
        b = mod_floor(&(&b * &b), &m);
        e >>= 1;
    }
    Ok(big_value(result))
}

/// Non-negative remainder of `a` modulo `m` (`m > 0`); `BigInt`'s `%` keeps the dividend's sign.
fn mod_floor(a: &BigInt, m: &BigInt) -> BigInt {
    let r = a % m;
    if r < BigInt::from(0) { r + m } else { r }
}

/// Modular inverse of `a` modulo `m`, or `None` when `gcd(a, m) != 1`.
fn mod_inverse(a: &BigInt, m: &BigInt) -> Option<BigInt> {
    let (g, x, _) = extended_gcd(a, m);
    if g != BigInt::from(1) {
        None
    } else {
        Some(mod_floor(&x, m))
    }
}

/// Iterative extended Euclid: returns `(g, x, y)` with `a*x + b*y = g = gcd(a, b)`.
fn extended_gcd(a: &BigInt, b: &BigInt) -> (BigInt, BigInt, BigInt) {
    let mut old_r = a.clone();
    let mut r = b.clone();
    let mut old_s = BigInt::from(1);
    let mut s = BigInt::from(0);
    let mut old_t = BigInt::from(0);
    let mut t = BigInt::from(1);
    while r != BigInt::from(0) {
        let q = &old_r / &r;
        let new_r = &old_r - &q * &r;
        old_r = std::mem::replace(&mut r, new_r);
        let new_s = &old_s - &q * &s;
        old_s = std::mem::replace(&mut s, new_s);
        let new_t = &old_t - &q * &t;
        old_t = std::mem::replace(&mut t, new_t);
    }
    (old_r, old_s, old_t)
}

// —— polynomials (coefficients lowest-degree first) and series (spec §18.6) ——

/// Evaluate the polynomial `coeffs` at `x` with Horner's rule; the empty polynomial is `0`.
fn poly_eval(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    arity(args, 2, "math::poly_eval")?;
    let coeffs = f64_slice_arg(args, 0, "math::poly_eval")?;
    let x = f64_arg(args, 1, "math::poly_eval")?;
    let mut acc = 0.0;
    for c in coeffs.iter().rev() {
        acc = acc * x + c;
    }
    Ok(f64_value(acc))
}

/// Sum of two polynomials (missing high-degree coefficients are zero).
fn poly_add(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    arity(args, 2, "math::poly_add")?;
    let a = f64_slice_arg(args, 0, "math::poly_add")?;
    let b = f64_slice_arg(args, 1, "math::poly_add")?;
    let n = a.len().max(b.len());
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let av = a.get(i).copied().unwrap_or(0.0);
        let bv = b.get(i).copied().unwrap_or(0.0);
        out.push(f64_value(av + bv));
    }
    Ok(Value::Array(out.into()))
}

/// Product of two polynomials (convolution); the zero polynomial is the empty array.
fn poly_mul(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    arity(args, 2, "math::poly_mul")?;
    let a = f64_slice_arg(args, 0, "math::poly_mul")?;
    let b = f64_slice_arg(args, 1, "math::poly_mul")?;
    if a.is_empty() || b.is_empty() {
        return Ok(Value::Array(Vec::new().into()));
    }
    let mut acc = vec![0.0f64; a.len() + b.len() - 1];
    for (i, &ai) in a.iter().enumerate() {
        for (j, &bj) in b.iter().enumerate() {
            acc[i + j] += ai * bj;
        }
    }
    let out: Vec<Value> = acc.into_iter().map(f64_value).collect();
    Ok(Value::Array(out.into()))
}

/// Formal derivative; a constant (or the empty) polynomial yields the empty array.
fn poly_derivative(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    arity(args, 1, "math::poly_derivative")?;
    let a = f64_slice_arg(args, 0, "math::poly_derivative")?;
    if a.len() <= 1 {
        return Ok(Value::Array(Vec::new().into()));
    }
    let out: Vec<Value> = a
        .iter()
        .enumerate()
        .skip(1)
        .map(|(i, &c)| f64_value(c * i as f64))
        .collect();
    Ok(Value::Array(out.into()))
}

/// A tiny `f64` complex value used only for the Durand–Kerner polynomial root finder.
#[derive(Clone, Copy)]
struct C64 {
    re: f64,
    im: f64,
}

impl C64 {
    fn new(re: f64, im: f64) -> C64 {
        C64 { re, im }
    }

    fn add(self, o: C64) -> C64 {
        C64::new(self.re + o.re, self.im + o.im)
    }

    fn sub(self, o: C64) -> C64 {
        C64::new(self.re - o.re, self.im - o.im)
    }

    fn mul(self, o: C64) -> C64 {
        C64::new(
            self.re * o.re - self.im * o.im,
            self.re * o.im + self.im * o.re,
        )
    }

    fn div(self, o: C64) -> C64 {
        let d = o.re * o.re + o.im * o.im;
        C64::new(
            (self.re * o.re + self.im * o.im) / d,
            (self.im * o.re - self.re * o.im) / d,
        )
    }

    fn abs(self) -> f64 {
        (self.re * self.re + self.im * self.im).sqrt()
    }
}

/// Evaluate a polynomial (lowest-degree-first, real coefficients) at a complex point (Horner).
fn poly_eval_c(coeffs: &[f64], z: C64) -> C64 {
    let mut acc = C64::new(0.0, 0.0);
    for &c in coeffs.iter().rev() {
        acc = acc.mul(z).add(C64::new(c, 0.0));
    }
    acc
}

/// Numeric complex roots via the Durand–Kerner method (no external crate). High-degree zero
/// coefficients are stripped; a degree-0 polynomial has no roots.
fn poly_roots(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    arity(args, 1, "math::poly_roots")?;
    let mut coeffs = f64_slice_arg(args, 0, "math::poly_roots")?;
    // Strip highest-degree zeros (trailing elements in lowest-degree-first order).
    while coeffs.len() > 1 && coeffs.last() == Some(&0.0) {
        coeffs.pop();
    }
    if coeffs.len() <= 1 {
        return Ok(Value::Array(Vec::new().into()));
    }
    // Normalize to a monic polynomial.
    let lead = *coeffs.last().unwrap();
    for c in coeffs.iter_mut() {
        *c /= lead;
    }
    let deg = coeffs.len() - 1;
    // Cauchy bound: every root lies within this radius.
    let radius = 1.0 + coeffs[..deg].iter().fold(0.0_f64, |m, &c| m.max(c.abs()));
    // Distinct, off-axis seeds on a circle of the Cauchy radius (0.4 + 0.9i avoids symmetric stalls).
    let seed = C64::new(0.4, 0.9);
    let mut z: Vec<C64> = (0..deg)
        .map(|k| {
            let theta = 2.0 * std::f64::consts::PI * k as f64 / deg as f64;
            seed.mul(C64::new(theta.cos(), theta.sin()))
                .mul(C64::new(radius, 0.0))
        })
        .collect();
    let tol = 1e-14;
    let max_iter = 100 * deg + 1000;
    for _ in 0..max_iter {
        let prev = z.clone();
        let mut max_delta = 0.0_f64;
        for (i, zi_out) in z.iter_mut().enumerate() {
            let zi = prev[i];
            let f = poly_eval_c(&coeffs, zi);
            let mut den = C64::new(1.0, 0.0);
            for (j, zj) in prev.iter().enumerate() {
                if j != i {
                    den = den.mul(zi.sub(*zj));
                }
            }
            if den.abs() > 0.0 {
                let delta = f.div(den);
                *zi_out = zi.sub(delta);
                max_delta = max_delta.max(delta.abs());
            }
        }
        if max_delta < tol {
            break;
        }
    }
    // Snap numerical noise on the real/imaginary axes, then sort for a deterministic order.
    for root in z.iter_mut() {
        let scale = root.re.abs().max(root.im.abs()).max(1.0);
        let eps = 1e-9 * scale;
        if root.im.abs() < eps {
            root.im = 0.0;
        }
        if root.re.abs() < eps {
            root.re = 0.0;
        }
    }
    z.sort_by(|a, b| {
        a.re.partial_cmp(&b.re)
            .unwrap_or(Ordering::Equal)
            .then(a.im.partial_cmp(&b.im).unwrap_or(Ordering::Equal))
    });
    let out: Vec<Value> = z
        .iter()
        .map(|c| {
            Value::Number(Number::from_complex(
                Number::Real(Real::F64(c.re)),
                Number::Real(Real::F64(c.im)),
            ))
        })
        .collect();
    Ok(Value::Array(out.into()))
}

/// The first `n` terms of the simple continued fraction of `x` (spec §18.6); the expansion stops
/// early when the remainder vanishes exactly (e.g. for a terminating rational).
fn continued_fraction(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    arity(args, 2, "math::continued_fraction")?;
    let x = f64_arg(args, 0, "math::continued_fraction")?;
    let n = int_arg(args, 1, "math::continued_fraction")?;
    if n < BigInt::from(1) {
        return Err(RuntimeError::Message(
            "`math::continued_fraction` expects n >= 1".into(),
        ));
    }
    if !x.is_finite() {
        return Err(RuntimeError::Message(
            "`math::continued_fraction` expects a finite x".into(),
        ));
    }
    let n = bigint_to_usize(&n)
        .filter(|&v| v <= MAX_CF_TERMS)
        .ok_or_else(|| {
            RuntimeError::Message(format!(
                "`math::continued_fraction` n must be at most {MAX_CF_TERMS}"
            ))
        })?;
    let mut out = Vec::with_capacity(n);
    let mut cur = x;
    for _ in 0..n {
        let a = cur.floor();
        out.push(big_value(f64_to_bigint(a)));
        let frac = cur - a;
        if frac == 0.0 {
            break;
        }
        cur = 1.0 / frac;
        if !cur.is_finite() {
            break;
        }
    }
    Ok(Value::Array(out.into()))
}

/// Convert a finite integral `f64` (as produced by `floor`) to a `BigInt` via its decimal form,
/// which sidesteps the absent `BigInt: From<f64>`.
fn f64_to_bigint(x: f64) -> BigInt {
    x.to_string()
        .parse::<BigInt>()
        .unwrap_or_else(|_| BigInt::from(0))
}
