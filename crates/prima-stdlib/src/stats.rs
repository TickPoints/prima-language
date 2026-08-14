//! `stats` module (spec §18.4 / appendix B.3): descriptive statistics, rank correlation, and the
//! Normal/Uniform/Exponential/Binomial/Poisson distributions. Distribution constructors return a
//! descriptor `Dict` (`"kind"` + named f64 parameters); `pdf`/`cdf`/`quantile`/`sample` consume it.
//!
//! Only `prima_core` + `std` are used. The PRNG is a self-contained xorshift64 (spec §18.3 note:
//! no external `rand` dependency); normal samples use Box–Muller, discrete samples use inverse-CDF.

use std::cmp::Ordering;
use std::collections::HashMap;
use std::f64::consts::{PI, SQRT_2};
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use std::time::{SystemTime, UNIX_EPOCH};

use prima_core::{Number, Real, Value, ValueKey};
use prima_runtime::stdlib::register_namespace;
use prima_runtime::{Evaluator, Function, NamespaceItem, RuntimeError};

type Native = fn(&mut Evaluator, &[Value]) -> Result<Value, RuntimeError>;

fn native(name: &'static str, call: Native) -> NamespaceItem {
    NamespaceItem::Func(Function::Native { name, call })
}

fn arity(args: &[Value], n: usize, fname: &str) -> Result<(), RuntimeError> {
    if args.len() == n {
        Ok(())
    } else {
        Err(RuntimeError::Message(format!("`{fname}` expects {n} argument(s), got {}", args.len())))
    }
}

/// A numeric argument converted to `f64` (spec §B.3: stats are computed in f64).
fn f64_arg(args: &[Value], i: usize, fname: &str) -> Result<f64, RuntimeError> {
    match args.get(i) {
        Some(Value::Number(n)) => Ok(n.to_f64_lossy()),
        Some(other) => Err(RuntimeError::Type(format!(
            "`{fname}` argument {i} must be a number, got {other:?}"
        ))),
        None => Err(RuntimeError::Message(format!("`{fname}` missing argument {i}"))),
    }
}

/// A numeric-array argument converted to `Vec<f64>`.
fn data_arg(args: &[Value], i: usize, fname: &str) -> Result<Vec<f64>, RuntimeError> {
    match args.get(i) {
        Some(Value::Array(items)) => {
            let mut out = Vec::with_capacity(items.len());
            for (j, v) in items.iter().enumerate() {
                match v {
                    Value::Number(n) => out.push(n.to_f64_lossy()),
                    other => {
                        return Err(RuntimeError::Type(format!(
                            "`{fname}` data element {j} must be a number, got {other:?}"
                        )))
                    }
                }
            }
            Ok(out)
        }
        Some(other) => Err(RuntimeError::Type(format!(
            "`{fname}` argument {i} must be an array of numbers, got {other:?}"
        ))),
        None => Err(RuntimeError::Message(format!("`{fname}` missing argument {i}"))),
    }
}

/// Wrap an `f64` as a `Value::Number`.
fn num(v: f64) -> Value {
    Value::Number(Number::Real(Real::F64(v)))
}

fn mean_of(d: &[f64]) -> f64 {
    d.iter().sum::<f64>() / d.len() as f64
}

fn sorted(d: &[f64]) -> Vec<f64> {
    let mut s = d.to_vec();
    s.sort_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));
    s
}

fn nonempty(d: &[f64], fname: &str) -> Result<(), RuntimeError> {
    if d.is_empty() {
        Err(RuntimeError::Type(format!("`{fname}` expects non-empty data")))
    } else {
        Ok(())
    }
}

/// Sample variance (n − 1 denominator, spec §B.3); `len` must be ≥ 2.
fn sample_variance(d: &[f64]) -> f64 {
    let m = mean_of(d);
    d.iter().map(|x| (x - m).powi(2)).sum::<f64>() / (d.len() - 1) as f64
}

/// Type-7 quantile: linear interpolation between the closest ranks (spec §B.3).
fn data_quantile(d: &[f64], q: f64) -> f64 {
    let s = sorted(d);
    let h = (s.len() as f64 - 1.0) * q;
    let lo = h.floor() as usize;
    let hi = h.ceil() as usize;
    let frac = h - h.floor();
    s[lo] + frac * (s[hi] - s[lo])
}

/// Average ranks (midranks) of the data, ties sharing the mean of their ranks (spec §B.3).
fn ranks(d: &[f64]) -> Vec<f64> {
    let mut idx: Vec<usize> = (0..d.len()).collect();
    idx.sort_by(|&a, &b| d[a].partial_cmp(&d[b]).unwrap_or(Ordering::Equal));
    let mut r = vec![0.0; d.len()];
    let mut i = 0usize;
    while i < idx.len() {
        let mut j = i + 1;
        while j < idx.len() && d[idx[j]] == d[idx[i]] {
            j += 1;
        }
        // Ranks are 1-based positions i+1..=j; the tied block shares their average.
        let avg = ((i + 1) as f64 + j as f64) / 2.0;
        for r in r.iter_mut().take(j).skip(i) {
            *r = avg;
        }
        i = j;
    }
    r
}

/// Register the `stats` namespace (spec §18.4 / appendix B.3).
pub fn register() {
    let mut items = HashMap::new();
    items.insert("mean".into(), native("stats::mean", mean));
    items.insert("median".into(), native("stats::median", median));
    items.insert("mode".into(), native("stats::mode", mode));
    items.insert("variance".into(), native("stats::variance", variance));
    items.insert("std".into(), native("stats::std", std_dev));
    items.insert("quantile".into(), native("stats::quantile", quantile));
    items.insert("percentile".into(), native("stats::percentile", percentile));
    items.insert("range".into(), native("stats::range", range));
    items.insert("min".into(), native("stats::min", min_val));
    items.insert("max".into(), native("stats::max", max_val));
    items.insert("cov".into(), native("stats::cov", cov));
    items.insert("corr".into(), native("stats::corr", corr));
    items.insert("spearman".into(), native("stats::spearman", spearman));
    items.insert("Normal".into(), native("stats::Normal", normal_dist));
    items.insert("Uniform".into(), native("stats::Uniform", uniform_dist));
    items.insert("Exponential".into(), native("stats::Exponential", exponential_dist));
    items.insert("Binomial".into(), native("stats::Binomial", binomial_dist));
    items.insert("Poisson".into(), native("stats::Poisson", poisson_dist));
    items.insert("pdf".into(), native("stats::pdf", pdf));
    items.insert("cdf".into(), native("stats::cdf", cdf));
    items.insert("sample".into(), native("stats::sample", sample));
    register_namespace("stats", items);
}

fn mean(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    arity(args, 1, "stats::mean")?;
    let d = data_arg(args, 0, "stats::mean")?;
    nonempty(&d, "stats::mean")?;
    Ok(num(mean_of(&d)))
}

fn median(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    arity(args, 1, "stats::median")?;
    let d = data_arg(args, 0, "stats::median")?;
    nonempty(&d, "stats::median")?;
    let s = sorted(&d);
    let n = s.len();
    let v = if n % 2 == 1 {
        s[n / 2]
    } else {
        0.5 * (s[n / 2 - 1] + s[n / 2])
    };
    Ok(num(v))
}

fn mode(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    arity(args, 1, "stats::mode")?;
    let d = data_arg(args, 0, "stats::mode")?;
    nonempty(&d, "stats::mode")?;
    let s = sorted(&d);
    let mut best_val = s[0];
    let mut best_cnt = 0usize;
    let mut i = 0usize;
    while i < s.len() {
        let mut j = i + 1;
        while j < s.len() && s[j] == s[i] {
            j += 1;
        }
        // Strict `>` keeps the lowest value on ties (data is ascending).
        if j - i > best_cnt {
            best_cnt = j - i;
            best_val = s[i];
        }
        i = j;
    }
    Ok(num(best_val))
}

fn variance(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    arity(args, 1, "stats::variance")?;
    let d = data_arg(args, 0, "stats::variance")?;
    if d.len() < 2 {
        return Err(RuntimeError::Type("`stats::variance` needs at least 2 data points".into()));
    }
    Ok(num(sample_variance(&d)))
}

fn std_dev(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    arity(args, 1, "stats::std")?;
    let d = data_arg(args, 0, "stats::std")?;
    if d.len() < 2 {
        return Err(RuntimeError::Type("`stats::std` needs at least 2 data points".into()));
    }
    Ok(num(sample_variance(&d).sqrt()))
}

/// `stats::quantile` is overloaded (spec §B.3): `quantile(data, q)` interpolates ranks for a data
/// array, while `quantile(dist, p)` is the inverse CDF of a distribution descriptor.
fn quantile(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    arity(args, 2, "stats::quantile")?;
    match args.first() {
        Some(Value::Array(_)) => quantile_data(args),
        Some(Value::Dict(_)) => quantile_dist(args),
        _ => Err(RuntimeError::Type(
            "`stats::quantile` first argument must be a data array or a distribution descriptor".into(),
        )),
    }
}

fn quantile_data(args: &[Value]) -> Result<Value, RuntimeError> {
    let d = data_arg(args, 0, "stats::quantile")?;
    nonempty(&d, "stats::quantile")?;
    let q = f64_arg(args, 1, "stats::quantile")?;
    if !(0.0..=1.0).contains(&q) {
        return Err(RuntimeError::Domain(format!("`stats::quantile` q must be in [0, 1], got {q}")));
    }
    Ok(num(data_quantile(&d, q)))
}

fn percentile(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    arity(args, 2, "stats::percentile")?;
    let d = data_arg(args, 0, "stats::percentile")?;
    nonempty(&d, "stats::percentile")?;
    let p = f64_arg(args, 1, "stats::percentile")?;
    if !(0.0..=100.0).contains(&p) {
        return Err(RuntimeError::Domain(format!("`stats::percentile` p must be in [0, 100], got {p}")));
    }
    Ok(num(data_quantile(&d, p / 100.0)))
}

fn range(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    arity(args, 1, "stats::range")?;
    let d = data_arg(args, 0, "stats::range")?;
    nonempty(&d, "stats::range")?;
    Ok(num(max_of(&d) - min_of(&d)))
}

fn min_of(d: &[f64]) -> f64 {
    d.iter().copied().fold(f64::INFINITY, f64::min)
}

fn max_of(d: &[f64]) -> f64 {
    d.iter().copied().fold(f64::NEG_INFINITY, f64::max)
}

fn min_val(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    arity(args, 1, "stats::min")?;
    let d = data_arg(args, 0, "stats::min")?;
    nonempty(&d, "stats::min")?;
    Ok(num(min_of(&d)))
}

fn max_val(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    arity(args, 1, "stats::max")?;
    let d = data_arg(args, 0, "stats::max")?;
    nonempty(&d, "stats::max")?;
    Ok(num(max_of(&d)))
}

/// Pair of numeric arrays of equal length, length ≥ 2.
fn pair_arg(args: &[Value], fname: &str) -> Result<(Vec<f64>, Vec<f64>), RuntimeError> {
    let x = data_arg(args, 0, fname)?;
    let y = data_arg(args, 1, fname)?;
    if x.len() != y.len() {
        return Err(RuntimeError::Type(format!("`{fname}` expects equally sized arrays")));
    }
    if x.len() < 2 {
        return Err(RuntimeError::Type(format!("`{fname}` needs at least 2 data points")));
    }
    Ok((x, y))
}

/// Pearson correlation coefficient of two series (spec §B.3).
fn pearson(x: &[f64], y: &[f64]) -> f64 {
    let mx = mean_of(x);
    let my = mean_of(y);
    let cov: f64 = x.iter().zip(y).map(|(a, b)| (a - mx) * (b - my)).sum::<f64>() / (x.len() - 1) as f64;
    let sx = sample_variance(x).sqrt();
    let sy = sample_variance(y).sqrt();
    cov / (sx * sy)
}

fn cov(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    arity(args, 2, "stats::cov")?;
    let (x, y) = pair_arg(args, "stats::cov")?;
    let mx = mean_of(&x);
    let my = mean_of(&y);
    let s: f64 = x.iter().zip(&y).map(|(a, b)| (a - mx) * (b - my)).sum();
    Ok(num(s / (x.len() - 1) as f64))
}

fn corr(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    arity(args, 2, "stats::corr")?;
    let (x, y) = pair_arg(args, "stats::corr")?;
    Ok(num(pearson(&x, &y)))
}

fn spearman(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    arity(args, 2, "stats::spearman")?;
    let (x, y) = pair_arg(args, "stats::spearman")?;
    Ok(num(pearson(&ranks(&x), &ranks(&y))))
}

// ——————————————————————————————— distributions ———————————————————————————————

/// Build a distribution descriptor `Dict` (spec §B.3): a `"kind"` string plus named f64 parameters.
fn dist_dict(kind: &str, params: &[(&str, f64)]) -> Value {
    let mut m: HashMap<ValueKey, Value> = HashMap::new();
    m.insert(ValueKey::Str("kind".into()), Value::String(kind.into()));
    for (k, v) in params {
        m.insert(ValueKey::Str((*k).into()), num(*v));
    }
    Value::Dict(m)
}

/// Parse a distribution descriptor back into `(kind, params)`.
fn dist_arg(args: &[Value], i: usize, fname: &str) -> Result<(String, HashMap<String, f64>), RuntimeError> {
    match args.get(i) {
        Some(Value::Dict(m)) => {
            let kind = match m.get(&ValueKey::Str("kind".into())) {
                Some(Value::String(s)) => s.clone(),
                _ => return Err(RuntimeError::Type(format!("`{fname}` descriptor lacks a `\"kind\"` string"))),
            };
            let mut params = HashMap::new();
            for (k, v) in m {
                if let (ValueKey::Str(k), Value::Number(n)) = (k, v) {
                    params.insert(k.clone(), n.to_f64_lossy());
                }
            }
            Ok((kind, params))
        }
        Some(other) => Err(RuntimeError::Type(format!(
            "`{fname}` argument {i} must be a distribution descriptor, got {other:?}"
        ))),
        None => Err(RuntimeError::Message(format!("`{fname}` missing argument {i}"))),
    }
}

fn param(params: &HashMap<String, f64>, name: &str) -> f64 {
    params.get(name).copied().unwrap_or(0.0)
}

fn normal_dist(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    arity(args, 2, "stats::Normal")?;
    let mu = f64_arg(args, 0, "stats::Normal")?;
    let sigma = f64_arg(args, 1, "stats::Normal")?;
    if sigma <= 0.0 {
        return Err(RuntimeError::Domain(format!("`stats::Normal` sigma must be > 0, got {sigma}")));
    }
    Ok(dist_dict("normal", &[("mu", mu), ("sigma", sigma)]))
}

fn uniform_dist(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    arity(args, 2, "stats::Uniform")?;
    let a = f64_arg(args, 0, "stats::Uniform")?;
    let b = f64_arg(args, 1, "stats::Uniform")?;
    if a >= b {
        return Err(RuntimeError::Domain(format!("`stats::Uniform` requires a < b, got a={a}, b={b}")));
    }
    Ok(dist_dict("uniform", &[("a", a), ("b", b)]))
}

fn exponential_dist(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    arity(args, 1, "stats::Exponential")?;
    let lambda = f64_arg(args, 0, "stats::Exponential")?;
    if lambda <= 0.0 {
        return Err(RuntimeError::Domain(format!("`stats::Exponential` lambda must be > 0, got {lambda}")));
    }
    Ok(dist_dict("exponential", &[("lambda", lambda)]))
}

fn binomial_dist(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    arity(args, 2, "stats::Binomial")?;
    let n = f64_arg(args, 0, "stats::Binomial")?;
    let p = f64_arg(args, 1, "stats::Binomial")?;
    if n <= 0.0 || n != n.floor() {
        return Err(RuntimeError::Domain(format!("`stats::Binomial` n must be a positive integer, got {n}")));
    }
    if !(0.0..=1.0).contains(&p) {
        return Err(RuntimeError::Domain(format!("`stats::Binomial` p must be in [0, 1], got {p}")));
    }
    Ok(dist_dict("binomial", &[("n", n), ("p", p)]))
}

fn poisson_dist(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    arity(args, 1, "stats::Poisson")?;
    let lambda = f64_arg(args, 0, "stats::Poisson")?;
    if lambda <= 0.0 {
        return Err(RuntimeError::Domain(format!("`stats::Poisson` lambda must be > 0, got {lambda}")));
    }
    Ok(dist_dict("poisson", &[("lambda", lambda)]))
}

fn pdf(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    arity(args, 2, "stats::pdf")?;
    let (kind, params) = dist_arg(args, 0, "stats::pdf")?;
    let x = f64_arg(args, 1, "stats::pdf")?;
    let v = match kind.as_str() {
        "normal" => {
            let (mu, sigma) = (param(&params, "mu"), param(&params, "sigma"));
            let z = (x - mu) / sigma;
            (1.0 / (sigma * (2.0 * PI).sqrt())) * (-0.5 * z * z).exp()
        }
        "uniform" => {
            let (a, b) = (param(&params, "a"), param(&params, "b"));
            if x >= a && x <= b {
                1.0 / (b - a)
            } else {
                0.0
            }
        }
        "exponential" => {
            let lambda = param(&params, "lambda");
            if x >= 0.0 {
                lambda * (-lambda * x).exp()
            } else {
                0.0
            }
        }
        "binomial" => {
            let (n, p) = (param(&params, "n"), param(&params, "p"));
            binomial_pmf(x, n, p)
        }
        "poisson" => {
            let lambda = param(&params, "lambda");
            poisson_pmf(x, lambda)
        }
        other => return Err(RuntimeError::Type(format!("unknown distribution kind `{other}`"))),
    };
    Ok(num(v))
}

fn cdf(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    arity(args, 2, "stats::cdf")?;
    let (kind, params) = dist_arg(args, 0, "stats::cdf")?;
    let x = f64_arg(args, 1, "stats::cdf")?;
    let v = match kind.as_str() {
        "normal" => {
            let (mu, sigma) = (param(&params, "mu"), param(&params, "sigma"));
            std_normal_cdf((x - mu) / sigma)
        }
        "uniform" => {
            let (a, b) = (param(&params, "a"), param(&params, "b"));
            if x < a {
                0.0
            } else if x > b {
                1.0
            } else {
                (x - a) / (b - a)
            }
        }
        "exponential" => {
            let lambda = param(&params, "lambda");
            if x < 0.0 {
                0.0
            } else {
                1.0 - (-lambda * x).exp()
            }
        }
        "binomial" => {
            let (n, p) = (param(&params, "n"), param(&params, "p"));
            binomial_cdf(x, n, p)
        }
        "poisson" => {
            let lambda = param(&params, "lambda");
            poisson_cdf(x, lambda)
        }
        other => return Err(RuntimeError::Type(format!("unknown distribution kind `{other}`"))),
    };
    Ok(num(v))
}

/// Inverse CDF for a distribution descriptor (spec §B.3); `p` must lie in `[0, 1]`.
fn quantile_dist(args: &[Value]) -> Result<Value, RuntimeError> {
    let (kind, params) = dist_arg(args, 0, "stats::quantile")?;
    let p = f64_arg(args, 1, "stats::quantile")?;
    if !(0.0..=1.0).contains(&p) {
        return Err(RuntimeError::Domain(format!("`stats::quantile` p must be in [0, 1], got {p}")));
    }
    let v = match kind.as_str() {
        "normal" => {
            let (mu, sigma) = (param(&params, "mu"), param(&params, "sigma"));
            mu + sigma * normal_quantile(p)
        }
        "uniform" => {
            let (a, b) = (param(&params, "a"), param(&params, "b"));
            a + p * (b - a)
        }
        "exponential" => {
            let lambda = param(&params, "lambda");
            if p <= 0.0 {
                0.0
            } else if p >= 1.0 {
                f64::INFINITY
            } else {
                -((1.0 - p).ln()) / lambda
            }
        }
        "binomial" => {
            let (n, p0) = (param(&params, "n"), param(&params, "p"));
            if n > usize::MAX as f64 {
                return Err(RuntimeError::Domain(format!("`stats::quantile` Binomial n too large: {n}")));
            }
            binomial_quantile(p, n, p0)
        }
        "poisson" => {
            let lambda = param(&params, "lambda");
            poisson_quantile(p, lambda)
        }
        other => return Err(RuntimeError::Type(format!("unknown distribution kind `{other}`"))),
    };
    Ok(num(v))
}

fn sample(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    arity(args, 2, "stats::sample")?;
    let (kind, params) = dist_arg(args, 0, "stats::sample")?;
    let n = f64_arg(args, 1, "stats::sample")?;
    if n < 0.0 || n != n.floor() {
        return Err(RuntimeError::Type(format!(
            "`stats::sample` n must be a non-negative integer, got {n}"
        )));
    }
    if n > usize::MAX as f64 {
        return Err(RuntimeError::Domain(format!("`stats::sample` n too large: {n}")));
    }
    let count = n as usize;
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        let v = match kind.as_str() {
            "normal" => {
                let (mu, sigma) = (param(&params, "mu"), param(&params, "sigma"));
                normal_sample(mu, sigma)
            }
            "uniform" => {
                let (a, b) = (param(&params, "a"), param(&params, "b"));
                a + next_f64() * (b - a)
            }
            "exponential" => {
                let lambda = param(&params, "lambda");
                -(next_f64().ln()) / lambda
            }
            "binomial" => {
                let (n0, p0) = (param(&params, "n"), param(&params, "p"));
                if n0 > usize::MAX as f64 {
                    return Err(RuntimeError::Domain(format!("`stats::sample` Binomial n too large: {n0}")));
                }
                binomial_quantile(next_f64(), n0, p0)
            }
            "poisson" => {
                let lambda = param(&params, "lambda");
                poisson_quantile(next_f64(), lambda)
            }
            other => return Err(RuntimeError::Type(format!("unknown distribution kind `{other}`"))),
        };
        out.push(num(v));
    }
    Ok(Value::Array(out))
}

// ————————————————————————————————— math primitives —————————————————————————————————

fn std_normal_cdf(z: f64) -> f64 {
    0.5 * (1.0 + erf(z / SQRT_2))
}

/// Standard-normal quantile by binary search over its CDF (spec §B.3: inverse CDF). The bounds
/// `±20` cover `p` beyond `1 − 1e−88`; `p ∈ (0, 1)` is enforced by the caller.
fn normal_quantile(p: f64) -> f64 {
    if p <= 0.0 {
        return f64::NEG_INFINITY;
    }
    if p >= 1.0 {
        return f64::INFINITY;
    }
    let mut lo = -20.0;
    let mut hi = 20.0;
    for _ in 0..200 {
        let mid = 0.5 * (lo + hi);
        if std_normal_cdf(mid) < p {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    0.5 * (lo + hi)
}

fn erf(x: f64) -> f64 {
    let sign = if x < 0.0 { -1.0 } else { 1.0 };
    let x = x.abs();
    let t = 1.0 / (1.0 + 0.3275911 * x);
    let y = 1.0 - (((((1.061405429 * t - 1.453152027) * t) + 1.421413741) * t - 0.284496736) * t + 0.254829592) * t * (-x * x).exp();
    sign * y
}

fn binomial_pmf(x: f64, n: f64, p: f64) -> f64 {
    if x < 0.0 {
        return 0.0;
    }
    let k = x.floor();
    if k > n {
        return 0.0;
    }
    if p == 0.0 {
        return if k == 0.0 { 1.0 } else { 0.0 };
    }
    if p == 1.0 {
        return if k == n { 1.0 } else { 0.0 };
    }
    let lnf = gammln(n + 1.0) - gammln(k + 1.0) - gammln(n - k + 1.0) + k * p.ln() + (n - k) * (1.0 - p).ln();
    lnf.exp()
}

fn binomial_cdf(x: f64, n: f64, p: f64) -> f64 {
    if x < 0.0 {
        return 0.0;
    }
    let k = x.floor();
    if k >= n {
        return 1.0;
    }
    // I_{1-p}(n - k, k + 1): sum_{j=0}^{k} C(n,j) p^j (1-p)^{n-j}
    betai(n - k, k + 1.0, 1.0 - p)
}

fn poisson_pmf(x: f64, lambda: f64) -> f64 {
    if x < 0.0 {
        return 0.0;
    }
    let k = x.floor();
    (k * lambda.ln() - lambda - gammln(k + 1.0)).exp()
}

fn poisson_cdf(x: f64, lambda: f64) -> f64 {
    if x < 0.0 {
        return 0.0;
    }
    // Q(k + 1, lambda) = sum_{j=0}^{k} e^{-lambda} lambda^j / j! (regularized upper incomplete gamma)
    gammq(x.floor() + 1.0, lambda)
}

/// Smallest integer `k` with `Binomial(n, p)` CDF ≥ p̂ (spec §B.3: binary search over integer support).
fn binomial_quantile(phat: f64, n: f64, p: f64) -> f64 {
    if phat <= 0.0 {
        return 0.0;
    }
    if phat >= 1.0 {
        return n;
    }
    let mut lo = 0usize;
    let mut hi = n as usize;
    while lo < hi {
        let mid = (lo + hi) / 2;
        if binomial_cdf(mid as f64, n, p) < phat {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    lo as f64
}

/// Smallest integer `k` with `Poisson(lambda)` CDF ≥ p̂ (spec §B.3: exponential search + binary search).
fn poisson_quantile(phat: f64, lambda: f64) -> f64 {
    if phat <= 0.0 {
        return 0.0;
    }
    if phat >= 1.0 {
        return f64::INFINITY;
    }
    if poisson_cdf(0.0, lambda) >= phat {
        return 0.0;
    }
    let mut hi = 1.0;
    while poisson_cdf(hi, lambda) < phat {
        hi *= 2.0;
    }
    let mut lo = 0.0;
    while hi - lo > 1.0 {
        let mid = ((lo + hi) / 2.0).floor();
        if poisson_cdf(mid, lambda) < phat {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    hi
}

fn normal_sample(mu: f64, sigma: f64) -> f64 {
    // Box–Muller (spec §B.3); `next_f64` never returns 0, so the log is finite.
    let u1 = next_f64();
    let u2 = next_f64();
    let r = (-2.0 * u1.ln()).sqrt();
    mu + sigma * r * (2.0 * PI * u2).cos()
}

// ——————————————————————————————— PRNG (xorshift64) ———————————————————————————————

/// Self-contained xorshift64 PRNG (spec §18.3 note: no external `rand` dependency), seeded from the
/// wall clock and a process counter on first use. Not cryptographically secure; fine for sampling.
static PRNG_STATE: AtomicU64 = AtomicU64::new(0);
static PRNG_SEED_COUNTER: AtomicU64 = AtomicU64::new(0);

fn next_u64() -> u64 {
    let mut x = PRNG_STATE.load(AtomicOrdering::Relaxed);
    if x == 0 {
        let seed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0x9E37_79B9_7F4A_7C15)
            ^ ((std::process::id() as u64) << 32)
            ^ PRNG_SEED_COUNTER.fetch_add(1, AtomicOrdering::Relaxed);
        x = if seed == 0 { 0x9E37_79B9_7F4A_7C15 } else { seed };
    }
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    PRNG_STATE.store(x, AtomicOrdering::Relaxed);
    x
}

/// Uniform `f64` in `(0, 1)`: the `(0, 1)`-exclusive of 0 keeps `ln`-based transforms finite.
fn next_f64() -> f64 {
    loop {
        let u = (next_u64() >> 11) as f64 * (1.0 / 9_007_199_254_740_992.0); // / 2^53
        if u > 0.0 {
            return u;
        }
    }
}

// ————————————————————— special functions (log-gamma / incomplete gamma / beta) —————————————————————

/// Log-gamma (Lanczos, 6 terms; Numerical Recipes `gammln`). Accurate to ~1e-15 for `x > 0`.
fn gammln(xx: f64) -> f64 {
    const COF: [f64; 6] = [
        76.18009172947146,
        -86.50532032941677,
        24.01409824083091,
        -1.231739572450155,
        0.1208650973866179e-2,
        -0.5395239384953e-5,
    ];
    let x = xx;
    let y = x;
    let tmp = x + 5.5;
    let tmp = tmp - (x + 0.5) * tmp.ln();
    let ser = 1.000000000190015 + COF.iter().enumerate().fold(0.0, |acc, (i, c)| acc + c / (y + i as f64 + 1.0));
    -tmp + (2.5066282746310005 * ser / x).ln()
}

/// Regularized upper incomplete gamma Q(a, x) = Γ(a, x)/Γ(a) (Numerical Recipes `gammq`).
fn gammq(a: f64, x: f64) -> f64 {
    if x < a + 1.0 {
        1.0 - gser(a, x)
    } else {
        gcf(a, x)
    }
}

fn gser(a: f64, x: f64) -> f64 {
    const ITMAX: i32 = 100;
    const EPS: f64 = 3.0e-7;
    let mut ap = a;
    let mut sum = 1.0 / a;
    let mut del = sum;
    for _ in 0..ITMAX {
        ap += 1.0;
        del *= x / ap;
        sum += del;
        if del.abs() < sum.abs() * EPS {
            break;
        }
    }
    sum * (-x + a * x.ln() - gammln(a)).exp()
}

fn gcf(a: f64, x: f64) -> f64 {
    const ITMAX: i32 = 200;
    const EPS: f64 = 3.0e-7;
    const FPMIN: f64 = 1.0e-30;
    let mut b = x + 1.0 - a;
    let mut c = 1.0 / FPMIN;
    let mut d = 1.0 / b;
    let mut h = d;
    for i in 1..=ITMAX {
        let an = -i as f64 * (i as f64 - a);
        b += 2.0;
        d = an * d + b;
        if d.abs() < FPMIN {
            d = FPMIN;
        }
        c = b + an / c;
        if c.abs() < FPMIN {
            c = FPMIN;
        }
        d = 1.0 / d;
        let del = d * c;
        h *= del;
        if (del - 1.0).abs() < EPS {
            break;
        }
    }
    (-x + a * x.ln() - gammln(a)).exp() * h
}

/// Regularized incomplete beta I_x(a, b) (Numerical Recipes `betai`).
fn betai(a: f64, b: f64, x: f64) -> f64 {
    if x <= 0.0 {
        return 0.0;
    }
    if x >= 1.0 {
        return 1.0;
    }
    let bt = (gammln(a + b) - gammln(a) - gammln(b) + a * x.ln() + b * (1.0 - x).ln()).exp();
    if x < (a + 1.0) / (a + b + 2.0) {
        bt * betacf(a, b, x) / a
    } else {
        1.0 - bt * betacf(b, a, 1.0 - x) / b
    }
}

fn betacf(a: f64, b: f64, x: f64) -> f64 {
    const MAXIT: i32 = 100;
    const EPS: f64 = 3.0e-7;
    const FPMIN: f64 = 1.0e-30;
    let qab = a + b;
    let qap = a + 1.0;
    let qam = a - 1.0;
    let mut c = 1.0;
    let mut d = 1.0 - qab * x / qap;
    if d.abs() < FPMIN {
        d = FPMIN;
    }
    d = 1.0 / d;
    let mut h = d;
    for m in 1..=MAXIT {
        let m2 = 2 * m;
        let mut aa = m as f64 * (b - m as f64) * x / ((qam + m2 as f64) * (a + m2 as f64));
        d = 1.0 + aa * d;
        if d.abs() < FPMIN {
            d = FPMIN;
        }
        c = 1.0 + aa / c;
        if c.abs() < FPMIN {
            c = FPMIN;
        }
        d = 1.0 / d;
        h *= d * c;
        aa = -(a + m as f64) * (qab + m as f64) * x / ((a + m2 as f64) * (qap + m2 as f64));
        d = 1.0 + aa * d;
        if d.abs() < FPMIN {
            d = FPMIN;
        }
        c = 1.0 + aa / c;
        if c.abs() < FPMIN {
            c = FPMIN;
        }
        d = 1.0 / d;
        let del = d * c;
        h *= del;
        if (del - 1.0).abs() < EPS {
            break;
        }
    }
    h
}
