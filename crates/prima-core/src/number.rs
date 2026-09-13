use crate::error::CoreError;
use std::fmt;

use num_bigint::BigInt;
use num_rational::BigRational;
use num_traits::{One, Signed, ToPrimitive, Zero};

/// Resource limit for exact-integer exponentiation (OOM guard): the largest bit length an
/// exact `Integer^Integer`/`Rational^Integer` result may have. `2^24` bits ≈ 2 MiB of precision
/// (≈ 5 million decimal digits) is far beyond any working exact-arithmetic workload while
/// capping the single-allocation blowup of expressions like `(10^9)^(10^9)`; the estimate is
/// computed in `u128`, so the multiplication itself cannot overflow. Exceeding the limit makes
/// `Number::pow` return `None` (callers keep their symbolic `Pow` fallback instead of computing).
const MAX_POW_BITS: u128 = 1 << 24;

/// Inexact real (spec §6.1). `NaN`/`Inf` are allowed to exist only in this layer (spec §6.2),
/// and only arise from explicit collapse; they never enter the symbolic layer.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Real {
    F32(f32),
    F64(f64),
}

impl std::hash::Hash for Real {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        match self {
            Real::F32(f) => f.to_bits().hash(state),
            Real::F64(f) => f.to_bits().hash(state),
        }
    }
}

/// Numeric tower (spec §6.1): the exact layer `Integer`/`Rational`/`Complex`, the inexact layer `Real`,
/// and the fixed-width collapsed layer (`I8`…`U128`/`Isize`/`Usize`/`BigFloat`) that maps 1:1 to Rust
/// primitives. Collapsed types exist **only after explicit collapse** and do not participate in implicit
/// promotion; they are normalized to the exact/`Real` layer before any arithmetic (spec §6.1).
/// The exact layer stays exact by default; a `Real` infects the result to inexact (spec §6.4 promotion rules).
#[derive(Debug, Clone)]
pub enum Number {
    /// Boxed to keep the enum small on the interpreter/VM hot path (see `Small` below).
    Integer(Box<BigInt>),
    /// Inlined small integer (spec §6.1 exact layer): semantically identical to `Integer`,
    /// kept as an `i64` to avoid heap allocation in hot interpreter loops. Every operation
    /// must treat `Small(v)` and `Integer(BigInt::from(v))` as the same value.
    Small(i64),
    /// Boxed to keep the enum small on the interpreter/VM hot path (see `Small` above).
    Rational(Box<BigRational>),
    Real(Real),
    Complex {
        re: Box<Number>,
        im: Box<Number>,
    },
    // —— fixed-width collapsed layer (spec §6.1, maps 1:1 to Rust primitives) ——
    // `I128`/`U128` are boxed: their 16-byte alignment would otherwise grow this enum to 32
    // bytes; they only exist after explicit collapse, so the allocation is off the hot path.
    I8(i8),
    I16(i16),
    I32(i32),
    I64(i64),
    I128(Box<i128>),
    U8(u8),
    U16(u16),
    U32(u32),
    U64(u64),
    U128(Box<u128>),
    Isize(isize),
    Usize(usize),
    BigFloat(f64),
}

/// Manual equality: `Small(v)` and `Integer(BigInt::from(v))` compare equal (same semantic
/// value, spec §6.1); all other variants keep the derived cross-variant-rejecting semantics
/// (including `NaN != NaN` for the float layers).
impl PartialEq for Number {
    fn eq(&self, other: &Number) -> bool {
        use Number::*;
        match (self, other) {
            (Small(a), Integer(b)) | (Integer(b), Small(a)) => BigInt::from(*a) == **b,
            _ => match (self, other) {
                (Small(a), Small(b)) => a == b,
                (Integer(a), Integer(b)) => a == b,
                (Rational(a), Rational(b)) => a == b,
                (Real(a), Real(b)) => a == b,
                (Complex { re: a, im: b }, Complex { re: c, im: d }) => a == c && b == d,
                (I8(a), I8(b)) => a == b,
                (I16(a), I16(b)) => a == b,
                (I32(a), I32(b)) => a == b,
                (I64(a), I64(b)) => a == b,
                (I128(a), I128(b)) => a == b,
                (U8(a), U8(b)) => a == b,
                (U16(a), U16(b)) => a == b,
                (U32(a), U32(b)) => a == b,
                (U64(a), U64(b)) => a == b,
                (U128(a), U128(b)) => a == b,
                (Isize(a), Isize(b)) => a == b,
                (Usize(a), Usize(b)) => a == b,
                (BigFloat(a), BigFloat(b)) => a == b,
                _ => false,
            },
        }
    }
}

impl Number {
    pub fn complex(re: i64, im: i64) -> Number {
        Number::Complex {
            re: Box::new(Number::from(re)),
            im: Box::new(Number::from(im)),
        }
    }

    pub fn is_complex(&self) -> bool {
        matches!(self, Number::Complex { .. })
    }

    pub fn is_zero(&self) -> bool {
        match self {
            Number::Small(v) => *v == 0,
            Number::Integer(i) => i.is_zero(),
            Number::Rational(r) => r.is_zero(),
            Number::Real(Real::F32(f)) => *f == 0.0,
            Number::Real(Real::F64(f)) => *f == 0.0,
            Number::Complex { re, im } => re.is_zero() && im.is_zero(),
            other => normalize(other.clone()).is_zero(),
        }
    }

    pub fn is_one(&self) -> bool {
        match self {
            Number::Small(v) => *v == 1,
            Number::Integer(i) => **i == BigInt::from(1),
            Number::Rational(r) => **r == BigRational::new(BigInt::from(1), BigInt::from(1)),
            Number::Real(Real::F32(f)) => *f == 1.0,
            Number::Real(Real::F64(f)) => *f == 1.0,
            Number::Complex { .. } => false,
            other => normalize(other.clone()).is_one(),
        }
    }

    pub fn abs(&self) -> Number {
        match self {
            // `i64::MIN` has no `i64` absolute value; widen instead of wrapping (spec §6.1 exact layer).
            Number::Small(v) => v
                .checked_abs()
                .map(Number::Small)
                .unwrap_or_else(|| Number::Integer(Box::new(BigInt::from(*v).abs()))),
            Number::Integer(i) => Number::Integer(Box::new(i.abs())),
            Number::Rational(r) => Number::Rational(Box::new(r.abs())),
            Number::Real(Real::F32(x)) => Number::Real(Real::F32(x.abs())),
            Number::Real(Real::F64(x)) => Number::Real(Real::F64(x.abs())),
            Number::Complex { .. } => self.clone(),
            other => normalize(other.clone()).abs(),
        }
    }

    pub fn sqrt(&self) -> Option<Number> {
        match self {
            Number::Small(v) => isqrt(&BigInt::from(*v)).map(big_to_number),
            Number::Integer(n) => isqrt(n).map(|v| Number::Integer(Box::new(v))),
            Number::Rational(r) => {
                let p = isqrt(r.numer())?;
                let q = isqrt(r.denom())?;
                Some(Number::Rational(Box::new(BigRational::new(p, q))))
            }
            Number::Real(Real::F32(x)) => Some(Number::Real(Real::F32(x.sqrt()))),
            Number::Real(Real::F64(x)) => Some(Number::Real(Real::F64(x.sqrt()))),
            Number::Complex { .. } => None,
            other => normalize(other.clone()).sqrt(),
        }
    }

    pub fn pow(&self, exp: &Number) -> Option<Number> {
        // `pow` is not on the hot arithmetic path: widen `Small` to the BigInt layer and let the
        // `Integer` arms handle it; fitting results are narrowed back to `Small` on return.
        let base = to_exact_layer(self.clone());
        let exp = to_exact_layer(exp.clone());
        match (&base, &exp) {
            (Number::Integer(a), Number::Integer(b)) => {
                if b.is_zero() {
                    return Some(Number::Integer(Box::new(BigInt::one())));
                }
                let neg = **b < BigInt::zero();
                let mag = if neg { -(**b).clone() } else { (**b).clone() };
                let e = mag.to_u32()?;
                if neg && a.is_zero() {
                    return None;
                }
                // Resource limit (OOM guard): the exact result of `a^e` needs `a.bits() * e` bits;
                // beyond [`MAX_POW_BITS`] the allocation itself would exhaust memory, so give up
                // here (`None` keeps the caller's symbolic `Pow` fallback without computing).
                if (a.bits() as u128) * (e as u128) > MAX_POW_BITS {
                    return None;
                }
                let p = a.pow(e);
                if neg {
                    Some(normalized(BigInt::one(), p))
                } else {
                    Some(Number::Integer(Box::new(p)))
                }
            }
            (Number::Rational(a), Number::Integer(b)) => {
                if b.is_zero() {
                    return Some(Number::Integer(Box::new(BigInt::one())));
                }
                let neg = **b < BigInt::zero();
                let mag = if neg { -(**b).clone() } else { (**b).clone() };
                let e = mag.to_u32()?;
                if neg && a.is_zero() {
                    return None;
                }
                // Resource limit (OOM guard): see the `Integer^Integer` arm — `p`/`q` each need
                // `numer.bits() * e`/`denom.bits() * e` bits.
                if (a.numer().bits().max(a.denom().bits()) as u128) * (e as u128) > MAX_POW_BITS {
                    return None;
                }
                let p = a.numer().pow(e);
                let q = a.denom().pow(e);
                if neg {
                    Some(normalized(q, p))
                } else {
                    Some(normalized(p, q))
                }
            }
            (Number::Real(x), Number::Integer(b)) => {
                let n = b.to_i32()?;
                match x {
                    Real::F32(f) => Some(Number::Real(Real::F32(f.powi(n)))),
                    Real::F64(f) => Some(Number::Real(Real::F64(f.powi(n)))),
                }
            }
            (Number::Real(x), Number::Rational(r)) => {
                let v = r.to_f64()?;
                match x {
                    Real::F32(f) => Some(Number::Real(Real::F32(f.powf(v as f32)))),
                    Real::F64(f) => Some(Number::Real(Real::F64(f.powf(v)))),
                }
            }
            (Number::Integer(a), Number::Rational(r)) => {
                if *r.denom() == BigInt::one() {
                    return base.pow(&Number::Integer(Box::new(r.numer().clone())));
                }
                // Exact x^(1/2): return an exact square root for perfect (rational) squares, otherwise leave it to the symbolic layer (spec §7.4: `sqrt(-1)→\i` depends on the domain).
                if *r.denom() == BigInt::from(2) && *r.numer() == BigInt::one() {
                    return base.sqrt();
                }
                let _ = a;
                None
            }
            (Number::Rational(a), Number::Rational(r)) => {
                if *r.denom() == BigInt::one() {
                    return base.pow(&Number::Integer(Box::new(r.numer().clone())));
                }
                if *r.denom() == BigInt::from(2) && *r.numer() == BigInt::one() {
                    return base.sqrt();
                }
                let _ = a;
                None
            }
            _ => None,
        }
        .map(smallify)
    }

    /// Numeric conversion (spec §9.2 `to_f64`): both the exact layer and `Real` convert; complex returns `NaN` (callers must check `is_complex` first).
    pub fn to_f64_lossy(&self) -> f64 {
        match self {
            Number::Small(v) => *v as f64,
            Number::Integer(i) => i.to_f64().unwrap_or(f64::NAN),
            Number::Rational(r) => r.to_f64().unwrap_or(f64::NAN),
            Number::Real(Real::F32(f)) => *f as f64,
            Number::Real(Real::F64(f)) => *f,
            Number::Complex { .. } => f64::NAN,
            Number::I8(v) => *v as f64,
            Number::I16(v) => *v as f64,
            Number::I32(v) => *v as f64,
            Number::I64(v) => *v as f64,
            Number::I128(v) => **v as f64,
            Number::U8(v) => *v as f64,
            Number::U16(v) => *v as f64,
            Number::U32(v) => *v as f64,
            Number::U64(v) => *v as f64,
            Number::U128(v) => **v as f64,
            Number::Isize(v) => *v as f64,
            Number::Usize(v) => *v as f64,
            Number::BigFloat(f) => *f,
        }
    }

    /// Exact conversion to `i64` (only integral values that do not overflow), otherwise `None`.
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Number::Small(v) => Some(*v),
            Number::Integer(i) => i.to_i64(),
            Number::Rational(r) if *r.denom() == BigInt::one() => r.numer().to_i64(),
            Number::Real(Real::F64(f)) if f.fract() == 0.0 && (*f as i64) as f64 == *f => {
                Some(*f as i64)
            }
            Number::Real(Real::F32(f)) if f.fract() == 0.0 && (*f as i64) as f64 == *f as f64 => {
                Some(*f as i64)
            }
            Number::I8(v) => Some(*v as i64),
            Number::I16(v) => Some(*v as i64),
            Number::I32(v) => Some(*v as i64),
            Number::I64(v) => Some(*v),
            Number::I128(v) => i64::try_from(**v).ok(),
            Number::U8(v) => Some(*v as i64),
            Number::U16(v) => Some(*v as i64),
            Number::U32(v) => Some(*v as i64),
            Number::U64(v) => i64::try_from(*v).ok(),
            Number::U128(v) => i64::try_from(**v).ok(),
            Number::Isize(v) => Some(*v as i64),
            Number::Usize(v) => i64::try_from(*v).ok(),
            Number::BigFloat(f) if f.fract() == 0.0 && (*f as i64) as f64 == *f => Some(*f as i64),
            _ => None,
        }
    }

    /// Exact conversion to `i32` (only integral values that do not overflow), otherwise `None`.
    pub fn as_i32(&self) -> Option<i32> {
        self.as_i64().and_then(|v| i32::try_from(v).ok())
    }

    /// Exact conversion to `u64` (only non-negative integral values that do not overflow), otherwise `None`.
    pub fn as_u64(&self) -> Option<u64> {
        match self {
            Number::Small(v) => u64::try_from(*v).ok(),
            Number::Integer(i) => i.to_u64(),
            Number::Rational(r) if *r.denom() == BigInt::one() => r.numer().to_u64(),
            Number::Real(Real::F64(f))
                if f.fract() == 0.0 && f.is_sign_positive() && (*f as u64) as f64 == *f =>
            {
                Some(*f as u64)
            }
            Number::Real(Real::F32(f))
                if f.fract() == 0.0 && f.is_sign_positive() && (*f as u64) as f64 == *f as f64 =>
            {
                Some(*f as u64)
            }
            Number::I8(v) if *v >= 0 => Some(*v as u64),
            Number::I16(v) if *v >= 0 => Some(*v as u64),
            Number::I32(v) if *v >= 0 => Some(*v as u64),
            Number::I64(v) if *v >= 0 => Some(*v as u64),
            Number::I128(v) => u64::try_from(**v).ok(),
            Number::U8(v) => Some(*v as u64),
            Number::U16(v) => Some(*v as u64),
            Number::U32(v) => Some(*v as u64),
            Number::U64(v) => Some(*v),
            Number::U128(v) => u64::try_from(**v).ok(),
            Number::Isize(v) if *v >= 0 => Some(*v as u64),
            Number::Usize(v) => u64::try_from(*v).ok(),
            Number::BigFloat(f)
                if f.fract() == 0.0 && f.is_sign_positive() && (*f as u64) as f64 == *f =>
            {
                Some(*f as u64)
            }
            _ => None,
        }
    }

    /// Conversion to `BigInt` (only integral values, spec §9.2 `to_bigint`).
    /// Floats are guarded by an `i64` round-trip check (like `as_i64`): a float too large for
    /// `i64` saturates on the cast, so it must return `None` instead of a silently wrong value.
    pub fn as_bigint(&self) -> Option<BigInt> {
        match self {
            Number::Small(v) => Some(BigInt::from(*v)),
            Number::Integer(i) => Some((**i).clone()),
            Number::Rational(r) if *r.denom() == BigInt::one() => Some(r.numer().clone()),
            Number::Real(Real::F64(f)) if f.fract() == 0.0 && (*f as i64) as f64 == *f => {
                Some(BigInt::from(*f as i64))
            }
            Number::Real(Real::F32(f)) if f.fract() == 0.0 && (*f as i64) as f64 == *f as f64 => {
                Some(BigInt::from(*f as i64))
            }
            Number::I8(v) => Some(BigInt::from(*v)),
            Number::I16(v) => Some(BigInt::from(*v)),
            Number::I32(v) => Some(BigInt::from(*v)),
            Number::I64(v) => Some(BigInt::from(*v)),
            Number::I128(v) => Some(BigInt::from(**v)),
            Number::U8(v) => Some(BigInt::from(*v)),
            Number::U16(v) => Some(BigInt::from(*v)),
            Number::U32(v) => Some(BigInt::from(*v)),
            Number::U64(v) => Some(BigInt::from(*v)),
            Number::U128(v) => Some(BigInt::from(**v)),
            Number::Isize(v) => Some(BigInt::from(*v)),
            Number::Usize(v) => Some(BigInt::from(*v)),
            Number::BigFloat(f) if f.fract() == 0.0 && (*f as i64) as f64 == *f => {
                Some(BigInt::from(*f as i64))
            }
            _ => None,
        }
    }

    /// Conversion to `BigRational` (exact layer, spec §9.2 `to_rational`).
    /// Floats are guarded by an `i64` round-trip check (see `as_bigint`).
    pub fn as_rational(&self) -> Option<BigRational> {
        match self {
            Number::Small(v) => Some(BigRational::from_integer(BigInt::from(*v))),
            Number::Integer(i) => Some(BigRational::from_integer((**i).clone())),
            Number::Rational(r) => Some((**r).clone()),
            Number::Real(Real::F64(f)) if f.fract() == 0.0 && (*f as i64) as f64 == *f => {
                Some(BigRational::from_integer(BigInt::from(*f as i64)))
            }
            Number::Real(Real::F32(f)) if f.fract() == 0.0 && (*f as i64) as f64 == *f as f64 => {
                Some(BigRational::from_integer(BigInt::from(*f as i64)))
            }
            Number::I8(v) => Some(BigRational::from_integer(BigInt::from(*v))),
            Number::I16(v) => Some(BigRational::from_integer(BigInt::from(*v))),
            Number::I32(v) => Some(BigRational::from_integer(BigInt::from(*v))),
            Number::I64(v) => Some(BigRational::from_integer(BigInt::from(*v))),
            Number::I128(v) => Some(BigRational::from_integer(BigInt::from(**v))),
            Number::U8(v) => Some(BigRational::from_integer(BigInt::from(*v))),
            Number::U16(v) => Some(BigRational::from_integer(BigInt::from(*v))),
            Number::U32(v) => Some(BigRational::from_integer(BigInt::from(*v))),
            Number::U64(v) => Some(BigRational::from_integer(BigInt::from(*v))),
            Number::U128(v) => Some(BigRational::from_integer(BigInt::from(**v))),
            Number::Isize(v) => Some(BigRational::from_integer(BigInt::from(*v))),
            Number::Usize(v) => Some(BigRational::from_integer(BigInt::from(*v))),
            Number::BigFloat(f) if f.fract() == 0.0 && (*f as i64) as f64 == *f => {
                Some(BigRational::from_integer(BigInt::from(*f as i64)))
            }
            _ => None,
        }
    }

    /// Whether this is an integral value (no fractional part, prerequisite for integer collapse in spec §9.2).
    pub fn is_integer_value(&self) -> bool {
        self.as_bigint().is_some()
    }

    /// Range-checked conversion to `i8` (spec §6.1 collapse layer): exact/fixed-width integral values
    /// convert if representable; `Real`/`BigFloat` convert only when integral and in range; complex never converts.
    pub fn as_i8(&self) -> Option<i8> {
        exact_integer(self).and_then(|b| b.to_i8())
    }

    /// Range-checked conversion to `i16` (spec §6.1 collapse layer); see `as_i8`.
    pub fn as_i16(&self) -> Option<i16> {
        exact_integer(self).and_then(|b| b.to_i16())
    }

    /// Range-checked conversion to `i128` (spec §6.1 collapse layer); see `as_i8`.
    pub fn as_i128(&self) -> Option<i128> {
        exact_integer(self).and_then(|b| b.to_i128())
    }

    /// Range-checked conversion to `u8` (spec §6.1 collapse layer); see `as_i8`.
    pub fn as_u8(&self) -> Option<u8> {
        exact_integer(self).and_then(|b| b.to_u8())
    }

    /// Range-checked conversion to `u16` (spec §6.1 collapse layer); see `as_i8`.
    pub fn as_u16(&self) -> Option<u16> {
        exact_integer(self).and_then(|b| b.to_u16())
    }

    /// Range-checked conversion to `u32` (spec §6.1 collapse layer); see `as_i8`.
    pub fn as_u32(&self) -> Option<u32> {
        exact_integer(self).and_then(|b| b.to_u32())
    }

    /// Range-checked conversion to `u128` (spec §6.1 collapse layer); see `as_i8`.
    pub fn as_u128(&self) -> Option<u128> {
        exact_integer(self).and_then(|b| b.to_u128())
    }

    /// Range-checked conversion to `isize` (spec §6.1 collapse layer); see `as_i8`.
    pub fn as_isize(&self) -> Option<isize> {
        exact_integer(self).and_then(|b| b.to_isize())
    }

    /// Range-checked conversion to `usize` (spec §6.1 collapse layer); see `as_i8`.
    pub fn as_usize(&self) -> Option<usize> {
        exact_integer(self).and_then(|b| b.to_usize())
    }

    /// Lossy conversion to `f32` (like `to_f64_lossy`); complex values never convert (`None`).
    pub fn as_f32(&self) -> Option<f32> {
        match self {
            Number::Complex { .. } => None,
            Number::Real(Real::F32(f)) => Some(*f),
            _ => Some(self.to_f64_lossy() as f32),
        }
    }

    /// Truncate toward zero to an integer (spec §9.6 `truncated_i32`).
    pub fn truncate(&self) -> Number {
        match self {
            Number::Small(_) | Number::Integer(_) => self.clone(),
            Number::Rational(r) => {
                let t = r.to_integer();
                normalized(t, BigInt::one())
            }
            Number::Real(Real::F64(f)) => Number::Real(Real::F64(f.trunc())),
            Number::Real(Real::F32(f)) => Number::Real(Real::F32(f.trunc())),
            Number::Complex { .. } => self.clone(),
            other => normalize(other.clone()).truncate(),
        }
    }

    /// Round to the nearest integer (spec §9.6 `rounded_i32`).
    pub fn round(&self) -> Number {
        match self {
            Number::Small(_) | Number::Integer(_) => self.clone(),
            Number::Rational(r) => normalized(r.round().numer().clone(), BigInt::one()),
            Number::Real(Real::F64(f)) => Number::Real(Real::F64(f.round())),
            Number::Real(Real::F32(f)) => Number::Real(Real::F32(f.round())),
            Number::Complex { .. } => self.clone(),
            other => normalize(other.clone()).round(),
        }
    }

    /// Round to a fixed number of decimal digits (spec §9.6 `rounded_f64(x, digits)`).
    ///
    /// Returns `None` when the digit count cannot be applied in `f64`: it does not fit `i32`
    /// (the exponent domain of `10f64.powi`, so an unchecked `digits as i32` would wrap to a bogus
    /// exponent), or `10^digits` overflows to infinity / underflows to zero (which would turn the
    /// computation into `NaN`/`Inf`). Callers surface a proper error instead of a silently wrong value.
    pub fn rounded_digits(&self, digits: i64) -> Option<Number> {
        let exp = i32::try_from(digits).ok()?;
        let mult = 10f64.powi(exp);
        if !mult.is_finite() || mult == 0.0 {
            return None;
        }
        let v = (self.to_f64_lossy() * mult).round() / mult;
        Some(Number::Real(Real::F64(v)))
    }

    /// Clamp to `[min, max]` (spec §9.5 `clamped_f64`).
    pub fn clamped_f64(&self, min: f64, max: f64) -> Number {
        let v = self.to_f64_lossy();
        Number::Real(Real::F64(v.clamp(min, max)))
    }
}

// Integer square root via Newton iteration: returns `None` for non-perfect squares so exact `sqrt` stays symbolic.
fn isqrt(n: &BigInt) -> Option<BigInt> {
    if n < &BigInt::zero() {
        return None;
    }
    if n.is_zero() {
        return Some(BigInt::zero());
    }
    let bits = n.bits();
    let mut x = BigInt::one() << bits.div_ceil(2);
    loop {
        let y = (&x + n / &x) >> 1;
        if y >= x {
            break;
        }
        x = y;
    }
    if &x * &x == *n { Some(x) } else { None }
}

impl From<i32> for Number {
    fn from(v: i32) -> Number {
        Number::Small(v as i64)
    }
}

impl From<i64> for Number {
    fn from(v: i64) -> Number {
        Number::Small(v)
    }
}

impl From<f64> for Number {
    fn from(v: f64) -> Number {
        Number::Real(Real::F64(v))
    }
}

/// Narrow a `BigInt` to `Small` when it fits `i64`; otherwise keep it as `Integer` (spec §6.1).
fn big_to_number(b: BigInt) -> Number {
    match b.to_i64() {
        Some(v) => Number::Small(v),
        None => Number::Integer(Box::new(b)),
    }
}

impl Number {
    /// Build a `Number` from a `BigInt`, narrowing to the inlined `Small` representation when the
    /// value fits `i64` (spec §6.1 exact layer). Integer literals and parsed integers must go
    /// through this so hot loops never see a heap-allocated `Integer` for small values.
    pub fn from_bigint(b: BigInt) -> Number {
        big_to_number(b)
    }
}

/// Narrow an exact-layer result to `Small` when it fits (spec §6.1); non-integer results pass through.
fn smallify(n: Number) -> Number {
    match n {
        Number::Integer(b) => big_to_number(*b),
        other => other,
    }
}

/// Widen `Small` to the `Integer` layer (used by non-hot paths like `pow`, spec §6.1).
fn to_exact_layer(n: Number) -> Number {
    match n {
        Number::Small(v) => Number::Integer(Box::new(BigInt::from(v))),
        other => normalize(other),
    }
}

fn to_rational(n: &Number) -> Number {
    match n {
        Number::Small(v) => Number::Rational(Box::new(BigRational::from_integer(BigInt::from(*v)))),
        Number::Integer(i) => {
            Number::Rational(Box::new(BigRational::new((**i).clone(), BigInt::one())))
        }
        Number::Rational(_) => n.clone(),
        _ => unreachable!("to_rational called on non-rational"),
    }
}

fn normalized(numer: BigInt, denom: BigInt) -> Number {
    if denom == BigInt::one() {
        big_to_number(numer)
    } else {
        Number::Rational(Box::new(BigRational::new(numer, denom)))
    }
}

fn to_f64(n: &Number) -> Number {
    match n {
        Number::Small(v) => Number::Real(Real::F64(*v as f64)),
        Number::Integer(i) => Number::Real(Real::F64(i.to_f64().unwrap_or(f64::NAN))),
        Number::Rational(r) => Number::Real(Real::F64(r.to_f64().unwrap_or(f64::NAN))),
        Number::Real(Real::F32(f)) => Number::Real(Real::F64(*f as f64)),
        Number::Real(Real::F64(f)) => Number::Real(Real::F64(*f)),
        _ => unreachable!("to_f64 called on complex"),
    }
}

fn to_real(n: &Number, like: &Real) -> Number {
    let v = match n {
        Number::Small(v) => *v as f64,
        Number::Integer(i) => i.to_f64().unwrap_or(f64::NAN),
        Number::Rational(r) => r.to_f64().unwrap_or(f64::NAN),
        Number::Real(Real::F32(f)) => *f as f64,
        Number::Real(Real::F64(f)) => *f,
        _ => unreachable!("to_real called on complex"),
    };
    match like {
        Real::F32(_) => Number::Real(Real::F32(v as f32)),
        Real::F64(_) => Number::Real(Real::F64(v)),
    }
}

fn convert_to(n: &Number, like: &Number) -> Number {
    match like {
        Number::Rational(_) => to_rational(n),
        Number::Real(Real::F64(_)) => to_f64(n),
        Number::Real(Real::F32(_)) => to_real(n, &Real::F32(0.0)),
        _ => n.clone(),
    }
}

fn zero_like(like: &Number) -> Number {
    match like {
        Number::Small(_) => Number::Small(0),
        Number::Integer(_) => Number::Integer(Box::new(BigInt::zero())),
        Number::Rational(_) => {
            Number::Rational(Box::new(BigRational::new(BigInt::zero(), BigInt::one())))
        }
        Number::Real(Real::F32(_)) => Number::Real(Real::F32(0.0)),
        Number::Real(Real::F64(_)) => Number::Real(Real::F64(0.0)),
        Number::Complex { re, im } => Number::Complex {
            re: Box::new(zero_like(re)),
            im: Box::new(zero_like(im)),
        },
        // Fixed-width collapsed variants normalize to the zero of the exact/`Real` layer (spec §6.1).
        other => zero_like(&normalize(other.clone())),
    }
}

/// Normalize a fixed-width collapsed value to the exact/inexact layer (spec §6.1): fixed-width
/// integers become `Integer`, `BigFloat` becomes `Real(F64)`; everything else is identity.
/// Collapsed types exist only after explicit collapse and never meet the promotion code raw.
fn normalize(n: Number) -> Number {
    match n {
        Number::I8(v) => Number::Integer(Box::new(BigInt::from(v))),
        Number::I16(v) => Number::Integer(Box::new(BigInt::from(v))),
        Number::I32(v) => Number::Integer(Box::new(BigInt::from(v))),
        Number::I64(v) => Number::Integer(Box::new(BigInt::from(v))),
        Number::I128(v) => Number::Integer(Box::new(BigInt::from(*v))),
        Number::U8(v) => Number::Integer(Box::new(BigInt::from(v))),
        Number::U16(v) => Number::Integer(Box::new(BigInt::from(v))),
        Number::U32(v) => Number::Integer(Box::new(BigInt::from(v))),
        Number::U64(v) => Number::Integer(Box::new(BigInt::from(v))),
        Number::U128(v) => Number::Integer(Box::new(BigInt::from(*v))),
        Number::Isize(v) => Number::Integer(Box::new(BigInt::from(v))),
        Number::Usize(v) => Number::Integer(Box::new(BigInt::from(v))),
        Number::BigFloat(f) => Number::Real(Real::F64(f)),
        other => other,
    }
}

/// Exact integral value as `BigInt`, guarded like `as_i64`/`as_u64` (only integral values that do not
/// overflow i64), else `None`. Backs the range-checked collapse conversions (spec §6.1/§9.2).
fn exact_integer(n: &Number) -> Option<BigInt> {
    match n {
        Number::Small(v) => Some(BigInt::from(*v)),
        Number::Integer(i) => Some((**i).clone()),
        Number::Rational(r) if *r.denom() == BigInt::one() => Some(r.numer().clone()),
        Number::Real(Real::F64(f)) if f.fract() == 0.0 && (*f as i64) as f64 == *f => {
            Some(BigInt::from(*f as i64))
        }
        Number::Real(Real::F32(f)) if f.fract() == 0.0 && (*f as i64) as f64 == *f as f64 => {
            Some(BigInt::from(*f as i64))
        }
        Number::I8(v) => Some(BigInt::from(*v)),
        Number::I16(v) => Some(BigInt::from(*v)),
        Number::I32(v) => Some(BigInt::from(*v)),
        Number::I64(v) => Some(BigInt::from(*v)),
        Number::I128(v) => Some(BigInt::from(**v)),
        Number::U8(v) => Some(BigInt::from(*v)),
        Number::U16(v) => Some(BigInt::from(*v)),
        Number::U32(v) => Some(BigInt::from(*v)),
        Number::U64(v) => Some(BigInt::from(*v)),
        Number::U128(v) => Some(BigInt::from(**v)),
        Number::Isize(v) => Some(BigInt::from(*v)),
        Number::Usize(v) => Some(BigInt::from(*v)),
        Number::BigFloat(f) if f.fract() == 0.0 && (*f as i64) as f64 == *f => {
            Some(BigInt::from(*f as i64))
        }
        _ => None,
    }
}

fn promote_real(a: &Number, b: &Number) -> (Number, Number) {
    let a = normalize(a.clone());
    let b = normalize(b.clone());
    match (&a, &b) {
        (Number::Small(_), Number::Small(_)) => (a.clone(), b.clone()),
        // Mixed `Small`/`Integer` are both exact integers: widen `Small` so the aligned pair
        // downstream only sees `Integer` (spec §6.1).
        (Number::Small(_), Number::Integer(_)) | (Number::Integer(_), Number::Small(_)) => {
            (to_exact_layer(a.clone()), to_exact_layer(b.clone()))
        }
        (Number::Integer(_), Number::Integer(_)) => (a.clone(), b.clone()),
        (Number::Rational(_), Number::Rational(_)) => (a.clone(), b.clone()),
        (Number::Small(_), Number::Rational(_)) | (Number::Rational(_), Number::Small(_)) => {
            (to_rational(&a), to_rational(&b))
        }
        (Number::Integer(_), Number::Rational(_)) | (Number::Rational(_), Number::Integer(_)) => {
            (to_rational(&a), to_rational(&b))
        }
        (Number::Real(Real::F32(_)), Number::Real(Real::F32(_))) => (a.clone(), b.clone()),
        (Number::Real(Real::F64(_)), Number::Real(Real::F64(_))) => (a.clone(), b.clone()),
        (Number::Real(Real::F64(_)), Number::Real(Real::F32(_)))
        | (Number::Real(Real::F32(_)), Number::Real(Real::F64(_))) => (to_f64(&a), to_f64(&b)),
        (Number::Real(x), Number::Integer(_) | Number::Small(_) | Number::Rational(_)) => {
            (a.clone(), to_real(&b, x))
        }
        (Number::Integer(_) | Number::Small(_) | Number::Rational(_), Number::Real(x)) => {
            (to_real(&a, x), b.clone())
        }
        (Number::Complex { .. }, _) | (_, Number::Complex { .. }) => {
            unreachable!("complex promoted by caller")
        }
        // Fixed-width variants are normalized before promotion (spec §6.1); never reached.
        _ => unreachable!("fixed-width variants must be normalized before promote_real"),
    }
}

/// Promote two numbers to a common type (spec §6.4).
/// Promotion sequence: `Integer < Rational < Complex<Rational> < F64 < Complex<F64>`;
/// a `Real` infects, promoting the whole `Complex` to `Complex<Real>`.
/// Fixed-width collapsed variants are normalized to the exact/`Real` layer first (spec §6.1).
pub fn promote(a: &Number, b: &Number) -> (Number, Number) {
    let a = normalize(a.clone());
    let b = normalize(b.clone());
    use Number::*;
    let a_complex = matches!(&a, Complex { .. });
    let b_complex = matches!(&b, Complex { .. });
    match (a_complex, b_complex) {
        (false, false) => promote_real(&a, &b),
        (true, true) => {
            let (Complex { re: rea, im: ima }, Complex { re: reb, im: imb }) = (a, b) else {
                unreachable!()
            };
            let (nrea, nreb) = promote_real(&rea, &reb);
            let (nima, nimb) = promote_real(&ima, &imb);
            (
                Complex {
                    re: Box::new(nrea),
                    im: Box::new(nima),
                },
                Complex {
                    re: Box::new(nreb),
                    im: Box::new(nimb),
                },
            )
        }
        (true, false) => {
            let Complex { re, im } = a else {
                unreachable!()
            };
            let (nre, nb) = promote_real(&re, &b);
            let nima = convert_to(&im, &nre);
            let nb_c = Complex {
                re: Box::new(nb),
                im: Box::new(zero_like(&nima)),
            };
            (
                Complex {
                    re: Box::new(nre),
                    im: Box::new(nima),
                },
                nb_c,
            )
        }
        (false, true) => {
            let Complex { re, im } = b else {
                unreachable!()
            };
            let (na, nre) = promote_real(&a, &re);
            let nima = convert_to(&im, &nre);
            let na_c = Complex {
                re: Box::new(na),
                im: Box::new(zero_like(&nima)),
            };
            (
                na_c,
                Complex {
                    re: Box::new(nre),
                    im: Box::new(nima),
                },
            )
        }
    }
}

fn add_real(a: Real, b: Real) -> Real {
    match (a, b) {
        (Real::F32(x), Real::F32(y)) => Real::F32(x + y),
        _ => {
            let x = match a {
                Real::F32(f) => f as f64,
                Real::F64(f) => f,
            };
            let y = match b {
                Real::F32(f) => f as f64,
                Real::F64(f) => f,
            };
            Real::F64(x + y)
        }
    }
}

fn mul_real(a: Real, b: Real) -> Real {
    match (a, b) {
        (Real::F32(x), Real::F32(y)) => Real::F32(x * y),
        _ => {
            let x = match a {
                Real::F32(f) => f as f64,
                Real::F64(f) => f,
            };
            let y = match b {
                Real::F32(f) => f as f64,
                Real::F64(f) => f,
            };
            Real::F64(x * y)
        }
    }
}

fn div_real(a: Real, b: Real) -> Real {
    match (a, b) {
        (Real::F32(x), Real::F32(y)) => Real::F32(x / y),
        _ => {
            let x = match a {
                Real::F32(f) => f as f64,
                Real::F64(f) => f,
            };
            let y = match b {
                Real::F32(f) => f as f64,
                Real::F64(f) => f,
            };
            Real::F64(x / y)
        }
    }
}

fn checked_denominator(n: &Number) -> Result<(), CoreError> {
    if n.is_zero() {
        Err(CoreError::DivisionByZero)
    } else {
        Ok(())
    }
}

fn complex_div(a: Number, b: Number, c: Number, d: Number) -> Number {
    let c2 = c.clone() * c.clone();
    let d2 = d.clone() * d.clone();
    let denom = c2 + d2;
    checked_denominator(&denom).expect("division by zero");
    let re = (a.clone() * c.clone() + b.clone() * d.clone()) / denom.clone();
    let im = (b * c - a * d) / denom;
    Number::Complex {
        re: Box::new(re),
        im: Box::new(im),
    }
}

impl std::ops::Add for Number {
    type Output = Number;
    fn add(self, rhs: Number) -> Number {
        // Small-integer fast path (spec §6.1 exact layer): skip promotion entirely; on overflow
        // fall through to the BigInt path so the result stays exact.
        if let (Number::Small(x), Number::Small(y)) = (&self, &rhs)
            && let Some(z) = x.checked_add(*y)
        {
            return Number::Small(z);
        }
        let (a, b) = promote(&normalize(self), &normalize(rhs));
        use Number::*;
        match (a, b) {
            (Small(x), Small(y)) => x
                .checked_add(y)
                .map(Small)
                .unwrap_or_else(|| Integer(Box::new(BigInt::from(x) + BigInt::from(y)))),
            (Integer(x), Integer(y)) => Integer(Box::new(*x + *y)),
            (Rational(x), Rational(y)) => {
                let r = *x + *y;
                normalized(r.numer().clone(), r.denom().clone())
            }
            (Real(x), Real(y)) => Real(add_real(x, y)),
            (Complex { re, im }, Complex { re: u, im: v }) => Complex {
                re: Box::new(*re + *u),
                im: Box::new(*im + *v),
            },
            _ => unreachable!("promote must align operands"),
        }
    }
}

impl std::ops::Sub for Number {
    type Output = Number;
    fn sub(self, rhs: Number) -> Number {
        // Small-integer fast path; on overflow fall through to the BigInt path (spec §6.1).
        if let (Number::Small(x), Number::Small(y)) = (&self, &rhs)
            && let Some(z) = x.checked_sub(*y)
        {
            return Number::Small(z);
        }
        let (a, b) = promote(&normalize(self), &normalize(rhs));
        match (a, b) {
            (Number::Small(x), Number::Small(y)) => x
                .checked_sub(y)
                .map(Number::Small)
                .unwrap_or_else(|| Number::Integer(Box::new(BigInt::from(x) - BigInt::from(y)))),
            (Number::Integer(x), Number::Integer(y)) => Number::Integer(Box::new(*x - *y)),
            (Number::Rational(x), Number::Rational(y)) => {
                let r = *x - *y;
                normalized(r.numer().clone(), r.denom().clone())
            }
            (Number::Real(rx), Number::Real(ry)) => match (rx, ry) {
                (Real::F32(x), Real::F32(y)) => Number::Real(Real::F32(x - y)),
                _ => {
                    let x = match rx {
                        Real::F32(f) => f as f64,
                        Real::F64(f) => f,
                    };
                    let y = match ry {
                        Real::F32(f) => f as f64,
                        Real::F64(f) => f,
                    };
                    Number::Real(Real::F64(x - y))
                }
            },
            (Number::Complex { re, im }, Number::Complex { re: u, im: v }) => Number::Complex {
                re: Box::new(*re - *u),
                im: Box::new(*im - *v),
            },
            _ => unreachable!("promote must align operands"),
        }
    }
}

impl std::ops::Mul for Number {
    type Output = Number;
    fn mul(self, rhs: Number) -> Number {
        // Small-integer fast path; on overflow fall through to the BigInt path (spec §6.1).
        if let (Number::Small(x), Number::Small(y)) = (&self, &rhs)
            && let Some(z) = x.checked_mul(*y)
        {
            return Number::Small(z);
        }
        let (a, b) = promote(&normalize(self), &normalize(rhs));
        use Number::*;
        match (a, b) {
            (Small(x), Small(y)) => x
                .checked_mul(y)
                .map(Small)
                .unwrap_or_else(|| Integer(Box::new(BigInt::from(x) * BigInt::from(y)))),
            (Integer(x), Integer(y)) => Integer(Box::new(*x * *y)),
            (Rational(x), Rational(y)) => {
                let r = *x * *y;
                normalized(r.numer().clone(), r.denom().clone())
            }
            (Real(x), Real(y)) => Real(mul_real(x, y)),
            (Complex { re, im }, Complex { re: u, im: v }) => {
                let re_new = *re.clone() * *u.clone() - *im.clone() * *v.clone();
                let im_new = *re * *v + *im * *u;
                Complex {
                    re: Box::new(re_new),
                    im: Box::new(im_new),
                }
            }
            _ => unreachable!("promote must align operands"),
        }
    }
}

impl std::ops::Div for Number {
    type Output = Number;
    fn div(self, rhs: Number) -> Number {
        let (a, b) = promote(&normalize(self), &normalize(rhs));
        use Number::*;
        match (a, b) {
            // Exact integer division (spec §6.1): divisible → integer quotient (narrowed back to
            // `Small` when it fits), otherwise the exact rational `x/y`. `checked_rem`/`checked_div`
            // guard `i64::MIN % -1` and `/ -1`, which have no `i64` results.
            (Small(x), Small(y)) => {
                if y == 0 {
                    panic!("division by zero");
                }
                match x.checked_rem(y) {
                    Some(0) => match x.checked_div(y) {
                        Some(q) => Small(q),
                        None => Integer(Box::new(BigInt::from(x) / BigInt::from(y))),
                    },
                    Some(_) => normalized(BigInt::from(x), BigInt::from(y)),
                    None => Integer(Box::new(BigInt::from(x) / BigInt::from(y))),
                }
            }
            (Integer(x), Integer(y)) => {
                if y.is_zero() {
                    panic!("division by zero");
                }
                normalized(*x, *y)
            }
            (Rational(x), Rational(y)) => {
                if y.is_zero() {
                    panic!("division by zero");
                }
                let r = *x / *y;
                normalized(r.numer().clone(), r.denom().clone())
            }
            (Real(x), Real(y)) => Real(div_real(x, y)),
            (Complex { re, im }, Complex { re: u, im: v }) => complex_div(*re, *im, *u, *v),
            _ => unreachable!("promote must align operands"),
        }
    }
}

impl std::ops::Neg for Number {
    type Output = Number;
    fn neg(self) -> Number {
        match normalize(self) {
            // `i64::MIN` has no `i64` negation; widen instead of wrapping (spec §6.1 exact layer).
            Number::Small(v) => v
                .checked_neg()
                .map(Number::Small)
                .unwrap_or_else(|| Number::Integer(Box::new(-BigInt::from(v)))),
            Number::Integer(i) => Number::Integer(Box::new(-*i)),
            Number::Rational(r) => Number::Rational(Box::new(-*r)),
            Number::Real(Real::F32(f)) => Number::Real(Real::F32(-f)),
            Number::Real(Real::F64(f)) => Number::Real(Real::F64(-f)),
            Number::Complex { re, im } => Number::Complex {
                re: Box::new(-*re),
                im: Box::new(-*im),
            },
            _ => unreachable!("normalize returns only the exact/Real/complex layer"),
        }
    }
}

impl fmt::Display for Real {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Real::F32(v) => write!(f, "{v}"),
            Real::F64(v) => write!(f, "{v}"),
        }
    }
}

impl fmt::Display for Number {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Number::Small(v) => write!(f, "{v}"),
            Number::Integer(i) => write!(f, "{i}"),
            Number::Rational(r) => write!(f, "{}/{}", r.numer(), r.denom()),
            Number::Real(r) => write!(f, "{r}"),
            Number::Complex { re, im } => write!(f, "{re} + {im}i"),
            Number::I8(v) => write!(f, "{v}"),
            Number::I16(v) => write!(f, "{v}"),
            Number::I32(v) => write!(f, "{v}"),
            Number::I64(v) => write!(f, "{v}"),
            Number::I128(v) => write!(f, "{v}"),
            Number::U8(v) => write!(f, "{v}"),
            Number::U16(v) => write!(f, "{v}"),
            Number::U32(v) => write!(f, "{v}"),
            Number::U64(v) => write!(f, "{v}"),
            Number::U128(v) => write!(f, "{v}"),
            Number::Isize(v) => write!(f, "{v}"),
            Number::Usize(v) => write!(f, "{v}"),
            Number::BigFloat(x) => write!(f, "{x}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ops::Neg;

    fn big(v: i64) -> Number {
        Number::Integer(Box::new(BigInt::from(v)))
    }

    fn rational(n: i64, d: i64) -> Number {
        Number::Rational(Box::new(BigRational::new(BigInt::from(n), BigInt::from(d))))
    }

    #[test]
    fn small_arithmetic_stays_small() {
        assert_eq!(Number::from(2) + Number::from(3), Number::from(5));
        assert_eq!(Number::from(2) - Number::from(5), Number::from(-3));
        assert_eq!(Number::from(6) * Number::from(7), Number::from(42));
        assert!(matches!(
            Number::from(2) + Number::from(3),
            Number::Small(5)
        ));
        // Equality across the two integer representations (spec §6.1 exact layer).
        assert_eq!(Number::from(5), big(5));
        assert_eq!(big(5), Number::from(5));
        assert_ne!(Number::from(5), big(6));
        assert_ne!(Number::from(5), Number::from(6));
    }

    #[test]
    fn small_overflow_falls_back_to_bigint() {
        let max = Number::from(i64::MAX);
        let min = Number::from(i64::MIN);
        // i64::MAX + 1 and i64::MIN - 1 leave the `Small` range but stay exact (spec §6.1).
        assert_eq!(
            max.clone() + Number::from(1),
            Number::Integer(Box::new(BigInt::from(i64::MAX) + BigInt::from(1)))
        );
        assert_eq!(
            min.clone() - Number::from(1),
            Number::Integer(Box::new(BigInt::from(i64::MIN) - BigInt::from(1)))
        );
        // Multiplication and negation overflow: widen instead of wrapping.
        assert_eq!(
            max * Number::from(2),
            Number::Integer(Box::new(BigInt::from(i64::MAX) * BigInt::from(2)))
        );
        assert_eq!(
            min.clone().neg(),
            Number::Integer(Box::new(BigInt::from(i64::MIN).neg()))
        );
        assert_eq!(
            min.abs(),
            Number::Integer(Box::new(BigInt::from(i64::MIN).abs()))
        );
    }

    #[test]
    fn small_matches_bigint_reference() {
        // Spot-check against direct BigInt arithmetic so the fast path cannot drift from the
        // exact layer (spec §6.1).
        for (a, b) in [
            (1234567890123i64, 987654321i64),
            (-1234567890123i64, 987654321i64),
            (i64::MAX, 1),
            (i64::MIN, 1),
        ] {
            let (x, y) = (Number::from(a), Number::from(b));
            let (ba, bb) = (BigInt::from(a), BigInt::from(b));
            assert_eq!(
                x.clone() + y.clone(),
                Number::Integer(Box::new(ba.clone() + bb.clone()))
            );
            assert_eq!(
                x.clone() - y.clone(),
                Number::Integer(Box::new(ba.clone() - bb.clone()))
            );
            assert_eq!(
                x.clone() * y.clone(),
                Number::Integer(Box::new(ba.clone() * bb.clone()))
            );
        }
        // Mixed `Small`/`Integer` operands promote to the same result (spec §6.4).
        assert_eq!(
            Number::from(1234567890123i64) + big(987654321),
            Number::from(1234567890123i64) + Number::from(987654321)
        );
    }

    #[test]
    fn small_mixed_layer_promotion() {
        // Division keeps the exact rational layer (spec §6.1): `1/3` stays `1/3`.
        assert_eq!(Number::from(1) / Number::from(3), rational(1, 3));
        assert_eq!(Number::from(6) / Number::from(3), Number::from(2));
        assert_eq!(Number::from(7) / Number::from(-2), rational(-7, 2));
        // `i64::MIN / -1` has no `i64` quotient; the exact result is 2^63.
        assert_eq!(
            Number::from(i64::MIN) / Number::from(-1),
            Number::Integer(Box::new(BigInt::from(2).pow(63)))
        );
        // Integer + Rational promotes to Rational; Integer + Real promotes to Real (spec §6.4).
        assert_eq!(Number::from(1) + rational(1, 2), rational(3, 2));
        assert_eq!(
            Number::from(1) + Number::Real(Real::F64(0.5)),
            Number::Real(Real::F64(1.5))
        );
        // Promoting against an `Integer` beyond `i64` stays exact (spec §6.4).
        let huge = Number::Integer(Box::new(BigInt::from(2).pow(100)));
        assert_eq!(
            huge.clone() + Number::from(1),
            Number::Integer(Box::new(BigInt::from(2).pow(100) + BigInt::from(1)))
        );
    }

    /// Mirrors the runtime comparison contract (promote then compare, spec §6.4).
    fn cmp(a: &Number, b: &Number) -> Option<std::cmp::Ordering> {
        let (x, y) = promote(a, b);
        match (x, y) {
            (Number::Small(x), Number::Small(y)) => Some(x.cmp(&y)),
            (Number::Integer(x), Number::Integer(y)) => Some(x.cmp(&y)),
            (Number::Rational(x), Number::Rational(y)) => Some(x.cmp(&y)),
            (Number::Real(Real::F32(x)), Number::Real(Real::F32(y))) => x.partial_cmp(&y),
            (Number::Real(Real::F64(x)), Number::Real(Real::F64(y))) => x.partial_cmp(&y),
            _ => None,
        }
    }

    #[test]
    fn small_comparisons_and_ordering() {
        assert_eq!(
            cmp(&Number::from(2), &Number::from(3)),
            Some(std::cmp::Ordering::Less)
        );
        assert_eq!(
            cmp(&Number::from(-3), &Number::from(2)),
            Some(std::cmp::Ordering::Less)
        );
        assert_eq!(
            cmp(&Number::from(i64::MAX), &Number::from(i64::MIN)),
            Some(std::cmp::Ordering::Greater)
        );
        assert_eq!(
            cmp(&Number::from(5), &big(5)),
            Some(std::cmp::Ordering::Equal)
        );
        assert_eq!(
            cmp(&big(5), &Number::from(5)),
            Some(std::cmp::Ordering::Equal)
        );
        assert_eq!(
            cmp(&Number::from(4), &big(5)),
            Some(std::cmp::Ordering::Less)
        );
        assert_eq!(
            cmp(&big(5), &Number::from(5)),
            Some(std::cmp::Ordering::Equal)
        );
    }

    #[test]
    fn small_conversions_and_display() {
        assert_eq!(Number::from(42i64).as_i64(), Some(42));
        assert_eq!(Number::from(i64::MIN).as_i64(), Some(i64::MIN));
        assert_eq!(Number::from(-1).as_u64(), None);
        assert_eq!(Number::from(7).as_u64(), Some(7));
        assert_eq!(Number::from(42).as_bigint(), Some(BigInt::from(42)));
        assert_eq!(
            Number::from(42).as_rational(),
            Some(BigRational::from_integer(BigInt::from(42)))
        );
        assert_eq!(Number::from(-5).to_string(), "-5");
        assert_eq!(Number::from(0).to_string(), "0");
        assert_eq!(format!("{}", Number::from(144).sqrt().unwrap()), "12");
        assert_eq!(
            Number::from(2).pow(&Number::from(10)),
            Some(Number::from(1024))
        );
        assert_eq!(
            Number::from(2).pow(&Number::from(100)),
            Some(Number::Integer(Box::new(BigInt::from(2).pow(100))))
        );
    }

    #[test]
    fn pow_resource_limit_gives_up_instead_of_oom() {
        // Resource limit (OOM guard): `2^1_000_000` needs ~1M bits — below the limit, computed.
        let ok = Number::from(2).pow(&Number::from(1_000_000i64));
        assert!(matches!(ok, Some(Number::Integer(_))));
        // `2^2^24` needs 2^24 bits plus one — above the limit, give up (`None` → symbolic fallback).
        assert_eq!(Number::from(2).pow(&Number::from((1i64 << 24) + 1)), None);
        // Rational bases are limited the same way (the numerator/denominator blow up equally).
        let half = Number::Rational(Box::new(BigRational::new(BigInt::from(1), BigInt::from(2))));
        assert_eq!(half.pow(&Number::from(i64::from(u32::MAX))), None);
        // Negative exponents share the limit (the reciprocal has the same bit length).
        assert_eq!(Number::from(10).pow(&Number::from(-(1i64 << 25))), None);
    }

    #[test]
    fn float_to_bigint_rejects_i64_overflow() {
        // M6 regression: floats beyond the `i64` range saturated on the cast and silently
        // produced `i64::MAX`; they must now be rejected (spec §9.2 round-trip guard).
        let big_f64 = Number::Real(Real::F64(1e19));
        assert_eq!(big_f64.as_bigint(), None);
        assert_eq!(big_f64.as_rational(), None);
        let big_f32 = Number::Real(Real::F32(1e19f32));
        assert_eq!(big_f32.as_bigint(), None);
        assert_eq!(big_f32.as_rational(), None);
        // 2^70 is integral as f64 but far beyond `i64`.
        let pow70 = Number::Real(Real::F64(2f64.powi(70)));
        assert_eq!(pow70.as_bigint(), None);
        assert_eq!(pow70.as_rational(), None);
        // Values inside the `i64` range keep converting exactly.
        assert_eq!(
            Number::Real(Real::F64(1e15)).as_bigint(),
            Some(BigInt::from(1_000_000_000_000_000i64))
        );
        assert_eq!(
            Number::Real(Real::F64(2f64.powi(62))).as_rational(),
            Some(BigRational::from_integer(BigInt::from(2).pow(62)))
        );
        // Fractional floats are rejected as before (spec §9.2).
        assert_eq!(Number::Real(Real::F64(1.5)).as_bigint(), None);
    }

    #[test]
    fn rounded_digits_large_float_and_out_of_range_count() {
        // F7 regression: the digit count used to be cast unchecked (`digits as i32`), so an
        // out-of-range count wrapped to a bogus exponent, and an extreme count produced `NaN`/`Inf`
        // through `10f64.powi`. Both must now be rejected instead of returning a silently wrong value.
        let x = Number::Real(Real::F64(1.2345));
        assert_eq!(x.rounded_digits(i64::from(i32::MAX) + 1), None);
        assert_eq!(x.rounded_digits(i64::MIN), None);
        assert_eq!(x.rounded_digits(1000), None);
        assert_eq!(x.rounded_digits(-1000), None);

        // A very large float rounded to 0 digits is unchanged — no saturation to `i64::MAX`.
        let huge = Number::Real(Real::F64(1e300));
        assert_eq!(huge.rounded_digits(0), Some(Number::Real(Real::F64(1e300))));

        // In-range counts keep the existing rounding semantics exactly.
        assert_eq!(
            Number::Real(Real::F64(2.5)).rounded_digits(0),
            Some(Number::Real(Real::F64(3.0)))
        );
        let expected = (1.2345f64 * 100.0).round() / 100.0;
        assert_eq!(x.rounded_digits(2), Some(Number::Real(Real::F64(expected))));
    }

    #[test]
    #[should_panic(expected = "division by zero")]
    fn small_division_by_zero_panics() {
        let _ = Number::from(5) / Number::from(0);
    }
}
