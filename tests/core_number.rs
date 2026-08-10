use num_bigint::BigInt;
use prima_core::number::{Number, Real};

#[test]
fn integer_arithmetic() {
    assert_eq!(Number::from(2) + Number::from(3), Number::from(5));
    assert_eq!(Number::from(5) * Number::from(6), Number::from(30));
    assert_eq!(Number::from(7) - Number::from(10), Number::from(-3));
    assert_eq!(-Number::from(4), Number::from(-4));
}

#[test]
fn integer_division_is_rational() {
    let q = Number::from(1) / Number::from(3);
    match q {
        Number::Rational(r) => {
            assert_eq!(*r.numer(), BigInt::from(1));
            assert_eq!(*r.denom(), BigInt::from(3));
        }
        other => panic!("expected Rational, got {other:?}"),
    }
}

#[test]
fn rational_arithmetic_and_reduction() {
    let q = Number::from(1) / Number::from(3) + Number::from(2) / Number::from(5);
    match q {
        Number::Rational(r) => {
            assert_eq!(*r.numer(), BigInt::from(11));
            assert_eq!(*r.denom(), BigInt::from(15));
        }
        other => panic!("expected Rational, got {other:?}"),
    }
    let q = Number::from(2) / Number::from(4);
    match q {
        Number::Rational(r) => {
            assert_eq!(*r.numer(), BigInt::from(1));
            assert_eq!(*r.denom(), BigInt::from(2));
        }
        other => panic!("expected Rational, got {other:?}"),
    }
}

#[test]
fn integer_rational_promotion() {
    let q = Number::from(1) + Number::from(1) / Number::from(2);
    match q {
        Number::Rational(r) => {
            assert_eq!(*r.numer(), BigInt::from(3));
            assert_eq!(*r.denom(), BigInt::from(2));
        }
        other => panic!("expected Rational, got {other:?}"),
    }
}

#[test]
fn f64_contagion() {
    let r = Number::from(1) + Number::Real(Real::F64(0.5));
    match r {
        Number::Real(Real::F64(v)) => assert_eq!(v, 1.5),
        other => panic!("expected Real, got {other:?}"),
    }
    let r = Number::Real(Real::F64(2.0)) * Number::from(3);
    match r {
        Number::Real(Real::F64(v)) => assert_eq!(v, 6.0),
        other => panic!("expected Real, got {other:?}"),
    }
}

#[test]
fn f32_to_f64_promotion() {
    let r = Number::Real(Real::F32(1.5)) + Number::Real(Real::F64(1.0));
    match r {
        Number::Real(Real::F64(v)) => assert_eq!(v, 2.5),
        other => panic!("expected Real F64, got {other:?}"),
    }
}

#[test]
fn complex_promotion() {
    let z = Number::complex(1, 2) + Number::from(3);
    match z {
        Number::Complex { re, im } => {
            assert_eq!(*re, Number::from(4));
            assert_eq!(*im, Number::from(2));
        }
        other => panic!("expected Complex, got {other:?}"),
    }
}

#[test]
fn display_forms() {
    assert_eq!(Number::from(3).to_string(), "3");
    assert_eq!((Number::from(1) / Number::from(2)).to_string(), "1/2");
    assert_eq!(Number::from(1.5).to_string(), "1.5");
}
