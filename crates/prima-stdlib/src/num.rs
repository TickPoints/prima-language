//! `num` module (spec §18.3 note / appendix B.5): integer number theory (`gcd`/`lcm`/`is_prime`/
//! `next_prime`), a self-contained PRNG (`random_integer`), and base conversion (`to_base`/`from_base`).

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use num_bigint::BigInt;
use prima_core::{Number, Value};
use prima_runtime::builtin;
use prima_runtime::{Evaluator, RuntimeError};

fn arity(args: &[Value], n: usize, fname: &str) -> Result<(), RuntimeError> {
    if args.len() == n {
        Ok(())
    } else {
        Err(RuntimeError::Message(format!("`{fname}` expects {n} argument(s), got {}", args.len())))
    }
}

fn int_arg(args: &[Value], i: usize, fname: &str) -> Result<BigInt, RuntimeError> {
    match args.get(i) {
        Some(Value::Number(n)) => n.as_bigint().ok_or_else(|| {
            RuntimeError::Type(format!("`{fname}` argument {i} must be an integer, got {n}"))
        }),
        Some(other) => Err(RuntimeError::Type(format!(
            "`{fname}` argument {i} must be an integer, got {other:?}"
        ))),
        None => Err(RuntimeError::Message(format!("`{fname}` missing argument {i}"))),
    }
}

fn string_arg(args: &[Value], i: usize, fname: &str) -> Result<String, RuntimeError> {
    match args.get(i) {
        Some(Value::String(s)) => Ok(s.clone()),
        Some(other) => Err(RuntimeError::Type(format!(
            "`{fname}` argument {i} must be a string, got {other:?}"
        ))),
        None => Err(RuntimeError::Message(format!("`{fname}` missing argument {i}"))),
    }
}

fn radix_arg(args: &[Value], i: usize, fname: &str) -> Result<u32, RuntimeError> {
    match args.get(i) {
        Some(Value::Number(n)) => match n.as_i64() {
            Some(r) if (2..=36).contains(&r) => Ok(r as u32),
            Some(r) => Err(RuntimeError::Message(format!("`{fname}` radix must be in 2..=36, got {r}"))),
            None => Err(RuntimeError::Type(format!("`{fname}` radix must be an integer, got {n}"))),
        },
        Some(other) => Err(RuntimeError::Type(format!("`{fname}` radix must be an integer, got {other:?}"))),
        None => Err(RuntimeError::Message(format!("`{fname}` missing argument {i}"))),
    }
}

/// Register the `num` `@builtin` implementations (spec §18.4 / §18.3 note / appendix B.5). Each
/// `@builtin` declaration in the embedded `num.pra` signature module binds to the implementation
/// registered under its fully-qualified `num::<name>` key (spec §18.4).
pub fn register() {
    builtin!("num::gcd", gcd);
    builtin!("num::lcm", lcm);
    builtin!("num::is_prime", is_prime);
    builtin!("num::next_prime", next_prime);
    builtin!("num::random_integer", random_integer);
    builtin!("num::to_base", to_base);
    builtin!("num::from_base", from_base);
    // Layered `@builtin(O1)` (spec §18.4): a Rust fast path used when `opt_level >= O1`, plus a `.pra`
    // fallback body in `num.pra`. The two implementations must agree (spec §18.4).
    builtin!("num::fibonacci", fibonacci, O1);
}

fn fibonacci(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    arity(args, 1, "num::fibonacci")?;
    let n = int_arg(args, 0, "num::fibonacci")?;
    if n < BigInt::from(0) {
        return Err(RuntimeError::Message("num::fibonacci expects a non-negative integer".into()));
    }
    let mut a = BigInt::from(0);
    let mut b = BigInt::from(1);
    let mut i = BigInt::from(0);
    while i < n {
        let t = &a + &b;
        a = std::mem::replace(&mut b, t);
        i += 1;
    }
    Ok(Value::Number(Number::Integer(a)))
}

fn gcd(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    arity(args, 2, "num::gcd")?;
    let a = int_arg(args, 0, "num::gcd")?;
    let b = int_arg(args, 1, "num::gcd")?;
    Ok(Value::Number(Number::Integer(bigint_gcd(&a, &b))))
}

fn lcm(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    arity(args, 2, "num::lcm")?;
    let a = int_arg(args, 0, "num::lcm")?;
    let b = int_arg(args, 1, "num::lcm")?;
    let g = bigint_gcd(&a, &b);
    let result = if g == BigInt::from(0) {
        BigInt::from(0)
    } else {
        bigint_abs(a) * (bigint_abs(b) / g)
    };
    Ok(Value::Number(Number::Integer(result)))
}

fn is_prime(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    arity(args, 1, "num::is_prime")?;
    let n = int_arg(args, 0, "num::is_prime")?;
    Ok(Value::Bool(bigint_is_prime(&n)))
}

fn next_prime(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    arity(args, 1, "num::next_prime")?;
    let n = int_arg(args, 0, "num::next_prime")?;
    Ok(Value::Number(Number::Integer(bigint_next_prime(&n))))
}

fn random_integer(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    arity(args, 2, "num::random_integer")?;
    let a = int_arg(args, 0, "num::random_integer")?;
    let b = int_arg(args, 1, "num::random_integer")?;
    if a > b {
        return Err(RuntimeError::Message("`num::random_integer` requires a <= b".into()));
    }
    let range = &b - &a + BigInt::from(1);
    let offset = BigInt::from(next_u64()) % &range;
    Ok(Value::Number(Number::Integer(a + offset)))
}

fn to_base(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    arity(args, 2, "num::to_base")?;
    let n = int_arg(args, 0, "num::to_base")?;
    let radix = radix_arg(args, 1, "num::to_base")?;
    Ok(Value::String(n.to_str_radix(radix)))
}

fn from_base(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    arity(args, 2, "num::from_base")?;
    let s = string_arg(args, 0, "num::from_base")?;
    let radix = radix_arg(args, 1, "num::from_base")?;
    match parse_base(&s, radix) {
        Some(v) => Ok(Value::Result(Ok(Box::new(Value::Number(Number::Integer(v)))))),
        None => Ok(Value::Result(Err(format!("cannot parse `{s}` in base {radix}")))),
    }
}

/// Euclidean algorithm for arbitrary-precision integers (spec §18.3 note).
fn bigint_gcd(a: &BigInt, b: &BigInt) -> BigInt {
    let mut a = if *a < BigInt::from(0) { -a } else { a.clone() };
    let mut b = if *b < BigInt::from(0) { -b } else { b.clone() };
    while b != BigInt::from(0) {
        let r = &a % &b;
        a = b;
        b = r;
    }
    a
}

/// Absolute value of a `BigInt` (kept local so the module needs no `num-traits`).
fn bigint_abs(n: BigInt) -> BigInt {
    if n < BigInt::from(0) { -n } else { n }
}

/// Trial division by 2, 3, then `6k ± 1` up to `sqrt(n)`; `n < 2` is not prime.
fn bigint_is_prime(n: &BigInt) -> bool {
    let zero = BigInt::from(0);
    let two = BigInt::from(2);
    let three = BigInt::from(3);
    if *n < two {
        return false;
    }
    if *n == two || *n == three {
        return true;
    }
    if n % &two == zero || n % &three == zero {
        return false;
    }
    let limit = bigint_isqrt(n);
    let mut i = BigInt::from(5);
    while i <= limit {
        if n % &i == zero || n % &(&i + &two) == zero {
            return false;
        }
        i += &two;
    }
    true
}

/// Smallest prime `>= n` (spec §18.3 note).
fn bigint_next_prime(n: &BigInt) -> BigInt {
    let two = BigInt::from(2);
    let mut x = n.clone();
    if x < two {
        return BigInt::from(2);
    }
    if x == two {
        return BigInt::from(2);
    }
    // Beyond 2 every prime is odd; step to the first odd candidate first.
    if &x % &two == BigInt::from(0) {
        x += BigInt::from(1);
    }
    while !bigint_is_prime(&x) {
        x += &two;
    }
    x
}

/// Integer square root via Newton iteration (floor), for the trial-division bound.
fn bigint_isqrt(n: &BigInt) -> BigInt {
    if *n <= BigInt::from(0) {
        return BigInt::from(0);
    }
    let bits = n.bits();
    let mut x = BigInt::from(1u64) << bits.div_ceil(2);
    loop {
        let y = (&x + n / &x) >> 1;
        if y >= x {
            break;
        }
        x = y;
    }
    x
}

/// Parse `s` in the given radix, accepting a leading `-` (which `BigInt::parse_bytes` rejects).
fn parse_base(s: &str, radix: u32) -> Option<BigInt> {
    let (neg, digits) = match s.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, s),
    };
    let mut v = BigInt::parse_bytes(digits.trim().as_bytes(), radix)?;
    if neg {
        v = -v;
    }
    Some(v)
}

/// xorshift64 PRNG (spec §18.3 note: no external `rand` dependency), seeded from the wall clock and
/// a process counter on first use. Not cryptographically secure; fine for sampling.
static PRNG_STATE: AtomicU64 = AtomicU64::new(0);
static PRNG_SEED_COUNTER: AtomicU64 = AtomicU64::new(0);

fn next_u64() -> u64 {
    let mut x = PRNG_STATE.load(Ordering::Relaxed);
    if x == 0 {
        let seed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0x9E37_79B9_7F4A_7C15)
            ^ ((std::process::id() as u64) << 32)
            ^ PRNG_SEED_COUNTER.fetch_add(1, Ordering::Relaxed);
        x = if seed == 0 { 0x9E37_79B9_7F4A_7C15 } else { seed };
    }
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    PRNG_STATE.store(x, Ordering::Relaxed);
    x
}
