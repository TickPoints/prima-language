use crate::error::CoreError;
use std::fmt;

use num_bigint::BigInt;
use num_rational::BigRational;
use num_traits::{One, ToPrimitive, Zero};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Real {
    F32(f32),
    F64(f64),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Number {
    Integer(BigInt),
    Rational(BigRational),
    Real(Real),
    Complex { re: Box<Number>, im: Box<Number> },
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
        }
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
    }
}

fn promote_real(a: &Number, b: &Number) -> (Number, Number) {
    match (a, b) {
        (Number::Integer(_), Number::Integer(_)) => (a.clone(), b.clone()),
        (Number::Rational(_), Number::Rational(_)) => (a.clone(), b.clone()),
        (Number::Integer(_), Number::Rational(_)) | (Number::Rational(_), Number::Integer(_)) => {
            (to_rational(a), to_rational(b))
        }
        (Number::Real(Real::F32(_)), Number::Real(Real::F32(_))) => (a.clone(), b.clone()),
        (Number::Real(Real::F64(_)), Number::Real(Real::F64(_))) => (a.clone(), b.clone()),
        (Number::Real(Real::F64(_)), Number::Real(Real::F32(_))) | (Number::Real(Real::F32(_)), Number::Real(Real::F64(_))) => {
            (to_f64(a), to_f64(b))
        }
        (Number::Real(x), Number::Integer(_)) | (Number::Real(x), Number::Rational(_)) => {
            (a.clone(), to_real(b, x))
        }
        (Number::Integer(_), Number::Real(x)) | (Number::Rational(_), Number::Real(x)) => {
            (to_real(a, x), b.clone())
        }
        (Number::Complex { .. }, _) | (_, Number::Complex { .. }) => unreachable!("complex promoted by caller"),
    }
}

pub fn promote(a: &Number, b: &Number) -> (Number, Number) {
    use Number::*;
    let a_complex = matches!(a, Complex { .. });
    let b_complex = matches!(b, Complex { .. });
    match (a_complex, b_complex) {
        (false, false) => promote_real(a, b),
        (true, true) => {
            let (Complex { re: rea, im: ima }, Complex { re: reb, im: imb }) = (a, b) else {
                unreachable!()
            };
            let (nrea, nreb) = promote_real(rea, reb);
            let (nima, nimb) = promote_real(ima, imb);
            (
                Complex { re: Box::new(nrea), im: Box::new(nima) },
                Complex { re: Box::new(nreb), im: Box::new(nimb) },
            )
        }
        (true, false) => {
            let Complex { re, im } = a else { unreachable!() };
            let (nre, nb) = promote_real(re, b);
            let nima = convert_to(im, &nre);
            let nb_c = Complex { re: Box::new(nb), im: Box::new(zero_like(&nima)) };
            (Complex { re: Box::new(nre), im: Box::new(nima) }, nb_c)
        }
        (false, true) => {
            let Complex { re, im } = b else { unreachable!() };
            let (na, nre) = promote_real(a, re);
            let nima = convert_to(im, &nre);
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
        let (a, b) = promote(&self, &rhs);
        use Number::*;
        match (a, b) {
            (Integer(x), Integer(y)) => Integer(x + y),
            (Rational(x), Rational(y)) => Rational(x + y),
            (Real(x), Real(y)) => Real(add_real(x, y)),
            (Complex { re, im }, Complex { re: u, im: v }) => Complex { re: Box::new(*re + *u), im: Box::new(*im + *v) },
            _ => unreachable!("promote must align operands"),
        }
    }
}

impl std::ops::Sub for Number {
    type Output = Number;
    fn sub(self, rhs: Number) -> Number {
        let (a, b) = promote(&self, &rhs);
        match (a, b) {
            (Number::Integer(x), Number::Integer(y)) => Number::Integer(x - y),
            (Number::Rational(x), Number::Rational(y)) => Number::Rational(x - y),
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
        let (a, b) = promote(&self, &rhs);
        use Number::*;
        match (a, b) {
            (Integer(x), Integer(y)) => Integer(x * y),
            (Rational(x), Rational(y)) => Rational(x * y),
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
        let (a, b) = promote(&self, &rhs);
        use Number::*;
        match (a, b) {
            (Integer(x), Integer(y)) => {
                if y.is_zero() {
                    panic!("division by zero");
                }
                Rational(BigRational::new(x, y))
            }
            (Rational(x), Rational(y)) => {
                if y.is_zero() {
                    panic!("division by zero");
                }
                Rational(x / y)
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
        match self {
            Number::Integer(i) => Number::Integer(-i),
            Number::Rational(r) => Number::Rational(-r),
            Number::Real(Real::F32(f)) => Number::Real(Real::F32(-f)),
            Number::Real(Real::F64(f)) => Number::Real(Real::F64(-f)),
            Number::Complex { re, im } => Number::Complex { re: Box::new(-*re), im: Box::new(-*im) },
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
        }
    }
}
