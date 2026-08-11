use crate::error::CoreError;
use std::fmt;

use num_bigint::BigInt;
use num_rational::BigRational;
use num_traits::{One, Signed, ToPrimitive, Zero};

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
#[derive(Debug, Clone, PartialEq)]
pub enum Number {
    Integer(BigInt),
    Rational(BigRational),
    Real(Real),
    Complex { re: Box<Number>, im: Box<Number> },
    // —— fixed-width collapsed layer (spec §6.1, maps 1:1 to Rust primitives) ——
    I8(i8), I16(i16), I32(i32), I64(i64), I128(i128),
    U8(u8), U16(u16), U32(u32), U64(u64), U128(u128),
    Isize(isize), Usize(usize),
    BigFloat(f64),
}

impl Number {
    pub fn complex(re: i64, im: i64) -> Number {
        Number::Complex {
            re: Box::new(Number::Integer(BigInt::from(re))),
            im: Box::new(Number::Integer(BigInt::from(im))),
        }
    }

    pub fn is_complex(&self) -> bool {
        matches!(self, Number::Complex { .. })
    }

    pub fn is_zero(&self) -> bool {
        match self {
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
            Number::Integer(i) => i == &BigInt::from(1),
            Number::Rational(r) => r == &BigRational::new(BigInt::from(1), BigInt::from(1)),
            Number::Real(Real::F32(f)) => *f == 1.0,
            Number::Real(Real::F64(f)) => *f == 1.0,
            Number::Complex { .. } => false,
            other => normalize(other.clone()).is_one(),
        }
    }

    pub fn abs(&self) -> Number {
        match self {
            Number::Integer(i) => Number::Integer(i.abs()),
            Number::Rational(r) => Number::Rational(r.abs()),
            Number::Real(Real::F32(x)) => Number::Real(Real::F32(x.abs())),
            Number::Real(Real::F64(x)) => Number::Real(Real::F64(x.abs())),
            Number::Complex { .. } => self.clone(),
            other => normalize(other.clone()).abs(),
        }
    }

    pub fn sqrt(&self) -> Option<Number> {
        match self {
            Number::Integer(n) => isqrt(n).map(Number::Integer),
            Number::Rational(r) => {
                let p = isqrt(r.numer())?;
                let q = isqrt(r.denom())?;
                Some(Number::Rational(BigRational::new(p, q)))
            }
            Number::Real(Real::F32(x)) => Some(Number::Real(Real::F32(x.sqrt()))),
            Number::Real(Real::F64(x)) => Some(Number::Real(Real::F64(x.sqrt()))),
            Number::Complex { .. } => None,
            other => normalize(other.clone()).sqrt(),
        }
    }

    pub fn pow(&self, exp: &Number) -> Option<Number> {
        let base = normalize(self.clone());
        let exp = normalize(exp.clone());
        match (&base, &exp) {
            (Number::Integer(a), Number::Integer(b)) => {
                if b.is_zero() {
                    return Some(Number::Integer(BigInt::one()));
                }
                let neg = *b < BigInt::zero();
                let mag = if neg { -b } else { b.clone() };
                let e = mag.to_u32()?;
                if neg && a.is_zero() {
                    return None;
                }
                let p = a.pow(e);
                if neg {
                    Some(normalized(BigInt::one(), p))
                } else {
                    Some(Number::Integer(p))
                }
            }
            (Number::Rational(a), Number::Integer(b)) => {
                if b.is_zero() {
                    return Some(Number::Integer(BigInt::one()));
                }
                let neg = *b < BigInt::zero();
                let mag = if neg { -b } else { b.clone() };
                let e = mag.to_u32()?;
                if neg && a.is_zero() {
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
                    return base.pow(&Number::Integer(r.numer().clone()));
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
                    return base.pow(&Number::Integer(r.numer().clone()));
                }
                if *r.denom() == BigInt::from(2) && *r.numer() == BigInt::one() {
                    return base.sqrt();
                }
                let _ = a;
                None
            }
            _ => None,
        }
    }

    /// Numeric conversion (spec §9.2 `to_f64`): both the exact layer and `Real` convert; complex returns `NaN` (callers must check `is_complex` first).
    pub fn to_f64_lossy(&self) -> f64 {
        match self {
            Number::Integer(i) => i.to_f64().unwrap_or(f64::NAN),
            Number::Rational(r) => r.to_f64().unwrap_or(f64::NAN),
            Number::Real(Real::F32(f)) => *f as f64,
            Number::Real(Real::F64(f)) => *f,
            Number::Complex { .. } => f64::NAN,
            Number::I8(v) => *v as f64,
            Number::I16(v) => *v as f64,
            Number::I32(v) => *v as f64,
            Number::I64(v) => *v as f64,
            Number::I128(v) => *v as f64,
            Number::U8(v) => *v as f64,
            Number::U16(v) => *v as f64,
            Number::U32(v) => *v as f64,
            Number::U64(v) => *v as f64,
            Number::U128(v) => *v as f64,
            Number::Isize(v) => *v as f64,
            Number::Usize(v) => *v as f64,
            Number::BigFloat(f) => *f,
        }
    }

    /// Exact conversion to `i64` (only integral values that do not overflow), otherwise `None`.
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Number::Integer(i) => i.to_i64(),
            Number::Rational(r) if *r.denom() == BigInt::one() => r.numer().to_i64(),
            Number::Real(Real::F64(f)) if f.fract() == 0.0 && (*f as i64) as f64 == *f => Some(*f as i64),
            Number::Real(Real::F32(f)) if f.fract() == 0.0 && (*f as i64) as f64 == *f as f64 => Some(*f as i64),
            Number::I8(v) => Some(*v as i64),
            Number::I16(v) => Some(*v as i64),
            Number::I32(v) => Some(*v as i64),
            Number::I64(v) => Some(*v),
            Number::I128(v) => i64::try_from(*v).ok(),
            Number::U8(v) => Some(*v as i64),
            Number::U16(v) => Some(*v as i64),
            Number::U32(v) => Some(*v as i64),
            Number::U64(v) => i64::try_from(*v).ok(),
            Number::U128(v) => i64::try_from(*v).ok(),
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
            Number::Integer(i) => i.to_u64(),
            Number::Rational(r) if *r.denom() == BigInt::one() => r.numer().to_u64(),
            Number::Real(Real::F64(f)) if f.fract() == 0.0 && f.is_sign_positive() && (*f as u64) as f64 == *f => {
                Some(*f as u64)
            }
            Number::Real(Real::F32(f)) if f.fract() == 0.0 && f.is_sign_positive() && (*f as u64) as f64 == *f as f64 => {
                Some(*f as u64)
            }
            Number::I8(v) if *v >= 0 => Some(*v as u64),
            Number::I16(v) if *v >= 0 => Some(*v as u64),
            Number::I32(v) if *v >= 0 => Some(*v as u64),
            Number::I64(v) if *v >= 0 => Some(*v as u64),
            Number::I128(v) => u64::try_from(*v).ok(),
            Number::U8(v) => Some(*v as u64),
            Number::U16(v) => Some(*v as u64),
            Number::U32(v) => Some(*v as u64),
            Number::U64(v) => Some(*v),
            Number::U128(v) => u64::try_from(*v).ok(),
            Number::Isize(v) if *v >= 0 => Some(*v as u64),
            Number::Usize(v) => u64::try_from(*v).ok(),
            Number::BigFloat(f) if f.fract() == 0.0 && f.is_sign_positive() && (*f as u64) as f64 == *f => {
                Some(*f as u64)
            }
            _ => None,
        }
    }

    /// Conversion to `BigInt` (only integral values, spec §9.2 `to_bigint`).
    pub fn as_bigint(&self) -> Option<BigInt> {
        match self {
            Number::Integer(i) => Some(i.clone()),
            Number::Rational(r) if *r.denom() == BigInt::one() => Some(r.numer().clone()),
            Number::Real(Real::F64(f)) if f.fract() == 0.0 => Some(BigInt::from(*f as i64)),
            Number::Real(Real::F32(f)) if f.fract() == 0.0 => Some(BigInt::from(*f as i64)),
            Number::I8(v) => Some(BigInt::from(*v)),
            Number::I16(v) => Some(BigInt::from(*v)),
            Number::I32(v) => Some(BigInt::from(*v)),
            Number::I64(v) => Some(BigInt::from(*v)),
            Number::I128(v) => Some(BigInt::from(*v)),
            Number::U8(v) => Some(BigInt::from(*v)),
            Number::U16(v) => Some(BigInt::from(*v)),
            Number::U32(v) => Some(BigInt::from(*v)),
            Number::U64(v) => Some(BigInt::from(*v)),
            Number::U128(v) => Some(BigInt::from(*v)),
            Number::Isize(v) => Some(BigInt::from(*v)),
            Number::Usize(v) => Some(BigInt::from(*v)),
            Number::BigFloat(f) if f.fract() == 0.0 => Some(BigInt::from(*f as i64)),
            _ => None,
        }
    }

    /// Conversion to `BigRational` (exact layer, spec §9.2 `to_rational`).
    pub fn as_rational(&self) -> Option<BigRational> {
        match self {
            Number::Integer(i) => Some(BigRational::from_integer(i.clone())),
            Number::Rational(r) => Some(r.clone()),
            Number::Real(Real::F64(f)) if f.fract() == 0.0 => Some(BigRational::from_integer(BigInt::from(*f as i64))),
            Number::Real(Real::F32(f)) if f.fract() == 0.0 => Some(BigRational::from_integer(BigInt::from(*f as i64))),
            Number::I8(v) => Some(BigRational::from_integer(BigInt::from(*v))),
            Number::I16(v) => Some(BigRational::from_integer(BigInt::from(*v))),
            Number::I32(v) => Some(BigRational::from_integer(BigInt::from(*v))),
            Number::I64(v) => Some(BigRational::from_integer(BigInt::from(*v))),
            Number::I128(v) => Some(BigRational::from_integer(BigInt::from(*v))),
            Number::U8(v) => Some(BigRational::from_integer(BigInt::from(*v))),
            Number::U16(v) => Some(BigRational::from_integer(BigInt::from(*v))),
            Number::U32(v) => Some(BigRational::from_integer(BigInt::from(*v))),
            Number::U64(v) => Some(BigRational::from_integer(BigInt::from(*v))),
            Number::U128(v) => Some(BigRational::from_integer(BigInt::from(*v))),
            Number::Isize(v) => Some(BigRational::from_integer(BigInt::from(*v))),
            Number::Usize(v) => Some(BigRational::from_integer(BigInt::from(*v))),
            Number::BigFloat(f) if f.fract() == 0.0 => Some(BigRational::from_integer(BigInt::from(*f as i64))),
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
            Number::Integer(_) => self.clone(),
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
            Number::Integer(_) => self.clone(),
            Number::Rational(r) => normalized(r.round().numer().clone(), BigInt::one()),
            Number::Real(Real::F64(f)) => Number::Real(Real::F64(f.round())),
            Number::Real(Real::F32(f)) => Number::Real(Real::F32(f.round())),
            Number::Complex { .. } => self.clone(),
            other => normalize(other.clone()).round(),
        }
    }

    /// Round to a fixed number of decimal digits (spec §9.6 `rounded_f64(x, digits)`).
    pub fn rounded_digits(&self, digits: i64) -> Number {
        let mult = 10f64.powi(digits as i32);
        let v = (self.to_f64_lossy() * mult).round() / mult;
        Number::Real(Real::F64(v))
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
    if &x * &x == *n {
        Some(x)
    } else {
        None
    }
}

impl From<i32> for Number {
    fn from(v: i32) -> Number {
        Number::Integer(BigInt::from(v))
    }
}

impl From<i64> for Number {
    fn from(v: i64) -> Number {
        Number::Integer(BigInt::from(v))
    }
}

impl From<f64> for Number {
    fn from(v: f64) -> Number {
        Number::Real(Real::F64(v))
    }
}

fn to_rational(n: &Number) -> Number {
    match n {
        Number::Integer(i) => Number::Rational(BigRational::new(i.clone(), BigInt::one())),
        Number::Rational(_) => n.clone(),
        _ => unreachable!("to_rational called on non-rational"),
    }
}

fn normalized(numer: BigInt, denom: BigInt) -> Number {
    if denom == BigInt::one() {
        Number::Integer(numer)
    } else {
        Number::Rational(BigRational::new(numer, denom))
    }
}

fn to_f64(n: &Number) -> Number {
    match n {
        Number::Integer(i) => Number::Real(Real::F64(i.to_f64().unwrap_or(f64::NAN))),
        Number::Rational(r) => Number::Real(Real::F64(r.to_f64().unwrap_or(f64::NAN))),
        Number::Real(Real::F32(f)) => Number::Real(Real::F64(*f as f64)),
        Number::Real(Real::F64(f)) => Number::Real(Real::F64(*f)),
        _ => unreachable!("to_f64 called on complex"),
    }
}

fn to_real(n: &Number, like: &Real) -> Number {
    let v = match n {
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
        Number::Integer(_) => Number::Integer(BigInt::zero()),
        Number::Rational(_) => Number::Rational(BigRational::new(BigInt::zero(), BigInt::one())),
        Number::Real(Real::F32(_)) => Number::Real(Real::F32(0.0)),
        Number::Real(Real::F64(_)) => Number::Real(Real::F64(0.0)),
        Number::Complex { re, im } => Number::Complex { re: Box::new(zero_like(re)), im: Box::new(zero_like(im)) },
        // Fixed-width collapsed variants normalize to the zero of the exact/`Real` layer (spec §6.1).
        other => zero_like(&normalize(other.clone())),
    }
}

/// Normalize a fixed-width collapsed value to the exact/inexact layer (spec §6.1): fixed-width
/// integers become `Integer`, `BigFloat` becomes `Real(F64)`; everything else is identity.
/// Collapsed types exist only after explicit collapse and never meet the promotion code raw.
fn normalize(n: Number) -> Number {
    match n {
        Number::I8(v) => Number::Integer(BigInt::from(v)),
        Number::I16(v) => Number::Integer(BigInt::from(v)),
        Number::I32(v) => Number::Integer(BigInt::from(v)),
        Number::I64(v) => Number::Integer(BigInt::from(v)),
        Number::I128(v) => Number::Integer(BigInt::from(v)),
        Number::U8(v) => Number::Integer(BigInt::from(v)),
        Number::U16(v) => Number::Integer(BigInt::from(v)),
        Number::U32(v) => Number::Integer(BigInt::from(v)),
        Number::U64(v) => Number::Integer(BigInt::from(v)),
        Number::U128(v) => Number::Integer(BigInt::from(v)),
        Number::Isize(v) => Number::Integer(BigInt::from(v)),
        Number::Usize(v) => Number::Integer(BigInt::from(v)),
        Number::BigFloat(f) => Number::Real(Real::F64(f)),
        other => other,
    }
}

/// Exact integral value as `BigInt`, guarded like `as_i64`/`as_u64` (only integral values that do not
/// overflow i64), else `None`. Backs the range-checked collapse conversions (spec §6.1/§9.2).
fn exact_integer(n: &Number) -> Option<BigInt> {
    match n {
        Number::Integer(i) => Some(i.clone()),
        Number::Rational(r) if *r.denom() == BigInt::one() => Some(r.numer().clone()),
        Number::Real(Real::F64(f)) if f.fract() == 0.0 && (*f as i64) as f64 == *f => Some(BigInt::from(*f as i64)),
        Number::Real(Real::F32(f)) if f.fract() == 0.0 && (*f as i64) as f64 == *f as f64 => Some(BigInt::from(*f as i64)),
        Number::I8(v) => Some(BigInt::from(*v)),
        Number::I16(v) => Some(BigInt::from(*v)),
        Number::I32(v) => Some(BigInt::from(*v)),
        Number::I64(v) => Some(BigInt::from(*v)),
        Number::I128(v) => Some(BigInt::from(*v)),
        Number::U8(v) => Some(BigInt::from(*v)),
        Number::U16(v) => Some(BigInt::from(*v)),
        Number::U32(v) => Some(BigInt::from(*v)),
        Number::U64(v) => Some(BigInt::from(*v)),
        Number::U128(v) => Some(BigInt::from(*v)),
        Number::Isize(v) => Some(BigInt::from(*v)),
        Number::Usize(v) => Some(BigInt::from(*v)),
        Number::BigFloat(f) if f.fract() == 0.0 && (*f as i64) as f64 == *f => Some(BigInt::from(*f as i64)),
        _ => None,
    }
}

fn promote_real(a: &Number, b: &Number) -> (Number, Number) {
    let a = normalize(a.clone());
    let b = normalize(b.clone());
    match (&a, &b) {
        (Number::Integer(_), Number::Integer(_)) => (a.clone(), b.clone()),
        (Number::Rational(_), Number::Rational(_)) => (a.clone(), b.clone()),
        (Number::Integer(_), Number::Rational(_)) | (Number::Rational(_), Number::Integer(_)) => {
            (to_rational(&a), to_rational(&b))
        }
        (Number::Real(Real::F32(_)), Number::Real(Real::F32(_))) => (a.clone(), b.clone()),
        (Number::Real(Real::F64(_)), Number::Real(Real::F64(_))) => (a.clone(), b.clone()),
        (Number::Real(Real::F64(_)), Number::Real(Real::F32(_))) | (Number::Real(Real::F32(_)), Number::Real(Real::F64(_))) => {
            (to_f64(&a), to_f64(&b))
        }
        (Number::Real(x), Number::Integer(_)) | (Number::Real(x), Number::Rational(_)) => {
            (a.clone(), to_real(&b, x))
        }
        (Number::Integer(_), Number::Real(x)) | (Number::Rational(_), Number::Real(x)) => {
            (to_real(&a, x), b.clone())
        }
        (Number::Complex { .. }, _) | (_, Number::Complex { .. }) => unreachable!("complex promoted by caller"),
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
                Complex { re: Box::new(nrea), im: Box::new(nima) },
                Complex { re: Box::new(nreb), im: Box::new(nimb) },
            )
        }
        (true, false) => {
            let Complex { re, im } = a else { unreachable!() };
            let (nre, nb) = promote_real(&re, &b);
            let nima = convert_to(&im, &nre);
            let nb_c = Complex { re: Box::new(nb), im: Box::new(zero_like(&nima)) };
            (Complex { re: Box::new(nre), im: Box::new(nima) }, nb_c)
        }
        (false, true) => {
            let Complex { re, im } = b else { unreachable!() };
            let (na, nre) = promote_real(&a, &re);
            let nima = convert_to(&im, &nre);
            let na_c = Complex { re: Box::new(na), im: Box::new(zero_like(&nima)) };
            (na_c, Complex { re: Box::new(nre), im: Box::new(nima) })
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
    Number::Complex { re: Box::new(re), im: Box::new(im) }
}

impl std::ops::Add for Number {
    type Output = Number;
    fn add(self, rhs: Number) -> Number {
        let (a, b) = promote(&normalize(self), &normalize(rhs));
        use Number::*;
        match (a, b) {
            (Integer(x), Integer(y)) => Integer(x + y),
            (Rational(x), Rational(y)) => { let r = x + y; normalized(r.numer().clone(), r.denom().clone()) },
            (Real(x), Real(y)) => Real(add_real(x, y)),
            (Complex { re, im }, Complex { re: u, im: v }) => Complex { re: Box::new(*re + *u), im: Box::new(*im + *v) },
            _ => unreachable!("promote must align operands"),
        }
    }
}

impl std::ops::Sub for Number {
    type Output = Number;
    fn sub(self, rhs: Number) -> Number {
        let (a, b) = promote(&normalize(self), &normalize(rhs));
        match (a, b) {
            (Number::Integer(x), Number::Integer(y)) => Number::Integer(x - y),
            (Number::Rational(x), Number::Rational(y)) => {
                let r = x - y;
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
            (Number::Complex { re, im }, Number::Complex { re: u, im: v }) => {
                Number::Complex { re: Box::new(*re - *u), im: Box::new(*im - *v) }
            }
            _ => unreachable!("promote must align operands"),
        }
    }
}

impl std::ops::Mul for Number {
    type Output = Number;
    fn mul(self, rhs: Number) -> Number {
        let (a, b) = promote(&normalize(self), &normalize(rhs));
        use Number::*;
        match (a, b) {
            (Integer(x), Integer(y)) => Integer(x * y),
            (Rational(x), Rational(y)) => { let r = x * y; normalized(r.numer().clone(), r.denom().clone()) },
            (Real(x), Real(y)) => Real(mul_real(x, y)),
            (Complex { re, im }, Complex { re: u, im: v }) => {
                let re_new = *re.clone() * *u.clone() - *im.clone() * *v.clone();
                let im_new = *re * *v + *im * *u;
                Complex { re: Box::new(re_new), im: Box::new(im_new) }
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
            (Integer(x), Integer(y)) => {
                if y.is_zero() {
                    panic!("division by zero");
                }
                normalized(x, y)
            }
            (Rational(x), Rational(y)) => {
                if y.is_zero() {
                    panic!("division by zero");
                }
                let r = x / y;
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
            Number::Integer(i) => Number::Integer(-i),
            Number::Rational(r) => Number::Rational(-r),
            Number::Real(Real::F32(f)) => Number::Real(Real::F32(-f)),
            Number::Real(Real::F64(f)) => Number::Real(Real::F64(-f)),
            Number::Complex { re, im } => Number::Complex { re: Box::new(-*re), im: Box::new(-*im) },
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
