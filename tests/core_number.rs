use num_bigint::BigInt;
use num_rational::BigRational;
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

#[test]
fn fixed_width_stays_distinct() {
    assert_ne!(Number::I8(5), Number::from(5));
    assert_eq!(Number::I8(5).to_string(), "5");
    assert_eq!(Number::U64(42).to_string(), "42");
    assert_eq!(Number::BigFloat(2.5).to_string(), "2.5");
}

#[test]
fn fixed_width_arithmetic_normalizes_to_exact() {
    assert_eq!(Number::I8(100) + Number::from(50), Number::Integer(BigInt::from(150)));
    assert_eq!(Number::I16(7) * Number::from(6), Number::Integer(BigInt::from(42)));
    assert_eq!(Number::U8(3) - Number::from(10), Number::Integer(BigInt::from(-7)));
    assert_eq!(-Number::I8(5), Number::Integer(BigInt::from(-5)));
    let exact_div = Number::I64(6) / Number::I64(2);
    match exact_div {
        Number::Rational(r) => {
            assert_eq!(*r.numer(), BigInt::from(3));
            assert_eq!(*r.denom(), BigInt::from(1));
        }
        other => panic!("expected Rational, got {other:?}"),
    }
    let q = Number::I8(1) / Number::I8(3);
    match q {
        Number::Rational(r) => {
            assert_eq!(*r.numer(), BigInt::from(1));
            assert_eq!(*r.denom(), BigInt::from(3));
        }
        other => panic!("expected Rational, got {other:?}"),
    }
    assert_eq!(Number::U128(1) + Number::from(2), Number::Integer(BigInt::from(3)));
}

#[test]
fn bigfloat_normalizes_to_real() {
    let r = Number::BigFloat(2.5) + Number::from(0.5);
    match r {
        Number::Real(Real::F64(v)) => assert_eq!(v, 3.0),
        other => panic!("expected Real F64, got {other:?}"),
    }
    assert_eq!(Number::BigFloat(4.0).sqrt(), Some(Number::Real(Real::F64(2.0))));
    assert_eq!(Number::BigFloat(3.0).to_string(), "3");
}

#[test]
fn fixed_width_collapse_conversions() {
    assert_eq!(Number::I8(5).as_i8(), Some(5));
    assert_eq!(Number::I8(127).as_i8(), Some(127));
    assert_eq!(Number::I16(300).as_i8(), None);
    assert_eq!(Number::U8(200).as_i8(), None);
    assert_eq!(Number::Integer(BigInt::from(7)).as_i8(), Some(7));
    assert_eq!(Number::Integer(BigInt::from(128)).as_i8(), None);
    assert_eq!(Number::from(3).as_i8(), Some(3));
    assert_eq!(Number::I8(-128).as_i16(), Some(-128));
    assert_eq!(Number::I128(1).as_i128(), Some(1));
    assert_eq!(Number::from(300).as_u8(), None);
    assert_eq!(Number::from(255).as_u8(), Some(255));
    assert_eq!(Number::U64(5).as_u64(), Some(5));
    assert_eq!(Number::I8(-1).as_u64(), None);
    assert_eq!(Number::U128(9).as_u128(), Some(9));
    assert_eq!(Number::Usize(5).as_usize(), Some(5));
    assert_eq!(Number::I128(-3).as_usize(), None);
    assert_eq!(Number::from(0).as_usize(), Some(0));
    assert_eq!(Number::I8(5).as_isize(), Some(5));
    assert_eq!(Number::I8(5).as_bigint(), Some(BigInt::from(5)));
    assert_eq!(
        Number::I8(5).as_rational(),
        Some(BigRational::from_integer(BigInt::from(5)))
    );
}

#[test]
fn real_conversions_are_integral_and_range_checked() {
    assert_eq!(Number::Real(Real::F64(7.0)).as_i8(), Some(7));
    assert_eq!(Number::Real(Real::F64(7.5)).as_i8(), None);
    assert_eq!(Number::Real(Real::F32(-2.0)).as_u64(), None);
    assert_eq!(Number::Real(Real::F64(2.0)).as_u64(), Some(2));
    assert_eq!(Number::Real(Real::F64(300.0)).as_u8(), None);
    assert_eq!(Number::from(0.5).as_i8(), None);
    assert_eq!(Number::BigFloat(4.0).as_u8(), Some(4));
    assert_eq!(Number::BigFloat(4.5).as_i8(), None);
}

#[test]
fn non_integral_and_complex_do_not_convert() {
    let third = Number::from(1) / Number::from(3);
    assert_eq!(third.as_i8(), None);
    assert_eq!(third.as_u64(), None);
    assert_eq!(third.as_usize(), None);
    assert_eq!(third.as_i128(), None);
    assert_eq!(Number::from(2.5).as_u64(), None);
    assert_eq!(Number::complex(1, 0).as_i8(), None);
    assert_eq!(Number::complex(1, 0).as_u64(), None);
    assert_eq!(Number::complex(1, 0).as_usize(), None);
    assert_eq!(Number::complex(1, 0).as_i128(), None);
    assert_eq!(Number::complex(1, 0).as_f32(), None);
}

#[test]
fn as_f32_lossy() {
    assert_eq!(Number::I8(3).as_f32(), Some(3.0));
    assert_eq!(Number::from(0.5).as_f32(), Some(0.5));
    assert_eq!(Number::from(1).as_f32(), Some(1.0));
    assert_eq!(Number::BigFloat(-2.0).as_f32(), Some(-2.0));
}

#[test]
fn fixed_width_promotes_with_real() {
    let r = Number::I8(3) + Number::Real(Real::F64(0.5));
    match r {
        Number::Real(Real::F64(v)) => assert_eq!(v, 3.5),
        other => panic!("expected Real F64, got {other:?}"),
    }
    let z = Number::I16(2) + Number::complex(1, 2);
    match z {
        Number::Complex { re, im } => {
            assert_eq!(*re, Number::from(3));
            assert_eq!(*im, Number::from(2));
        }
        other => panic!("expected Complex, got {other:?}"),
    }
}

#[test]
fn fixed_width_zero_and_one_predicates() {
    assert!(Number::I8(0).is_zero());
    assert!(!Number::U64(1).is_zero());
    assert!(Number::I8(1).is_one());
    assert!(!Number::U128(2).is_one());
}
