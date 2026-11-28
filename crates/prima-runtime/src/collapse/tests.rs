use super::call;
use crate::error::RuntimeError;
use num_bigint::BigInt;
use prima_core::{BuiltinSymbols, ExprPool, Number, Real, Value};

fn setup() -> (ExprPool, &'static BuiltinSymbols) {
    (ExprPool::new(), BuiltinSymbols::global())
}

#[test]
fn to_f64_of_symbolic_sqrt2() {
    let (pool, builtins) = setup();
    let f = pool.symbol(builtins.sqrt);
    let app = pool.apply(f, &[pool.integer(2)]);
    let v = call("to_f64", &[Value::Expr(app)], &pool, builtins).unwrap();
    match v {
        Value::Number(Number::Real(Real::F64(x))) => {
            assert!((x - std::f64::consts::SQRT_2).abs() < 1e-9)
        }
        other => panic!("expected F64, got {other:?}"),
    }
}

#[test]
fn to_integer_family_produces_fixed_width_variants() {
    let (pool, builtins) = setup();
    assert_eq!(
        call("to_i8", &[Value::Number(Number::from(7))], &pool, builtins).unwrap(),
        Value::Number(Number::I8(7))
    );
    assert_eq!(
        call(
            "to_i16",
            &[Value::Number(Number::from(300))],
            &pool,
            builtins
        )
        .unwrap(),
        Value::Number(Number::I16(300))
    );
    assert_eq!(
        call(
            "to_i64",
            &[Value::Number(Number::from(42))],
            &pool,
            builtins
        )
        .unwrap(),
        Value::Number(Number::I64(42))
    );
    assert_eq!(
        call(
            "to_i128",
            &[Value::Number(Number::from(-7))],
            &pool,
            builtins
        )
        .unwrap(),
        Value::Number(Number::I128(Box::new(-7)))
    );
    assert_eq!(
        call(
            "to_u64",
            &[Value::Number(Number::from(42))],
            &pool,
            builtins
        )
        .unwrap(),
        Value::Number(Number::U64(42))
    );
    assert_eq!(
        call(
            "to_usize",
            &[Value::Number(Number::from(42))],
            &pool,
            builtins
        )
        .unwrap(),
        Value::Number(Number::Usize(42))
    );
    // Rational with integral value also collapses.
    let r = Number::from(10) / Number::from(2);
    assert_eq!(
        call("to_i32", &[Value::Number(r)], &pool, builtins).unwrap(),
        Value::Number(Number::I32(5))
    );
}

#[test]
fn to_integer_overflow_errors() {
    let (pool, builtins) = setup();
    let big = Value::Number(Number::Integer(Box::new(BigInt::from(2_147_483_648i64))));
    let err = call("to_i32", std::slice::from_ref(&big), &pool, builtins).unwrap_err();
    assert!(matches!(err, RuntimeError::Overflow(_)));

    let err = call(
        "to_i8",
        &[Value::Number(Number::from(200))],
        &pool,
        builtins,
    )
    .unwrap_err();
    assert!(matches!(err, RuntimeError::Overflow(_)));

    let err = call(
        "to_u64",
        &[Value::Number(Number::from(-1))],
        &pool,
        builtins,
    )
    .unwrap_err();
    // A value below the target minimum is a distinct underflow category (`R0002`, appendix C.2).
    assert_eq!(err.code(), "R0002");

    // Non-integral values also fail with overflow (spec §9.8: only collapse failure errors).
    let frac = Number::from(7) / Number::from(2);
    let err = call("to_i8", &[Value::Number(frac)], &pool, builtins).unwrap_err();
    assert!(matches!(err, RuntimeError::Overflow(_)));
}

#[test]
fn to_below_minimum_is_underflow_r0002() {
    let (pool, builtins) = setup();

    // `to_u8(-1)`: below the unsigned minimum → `R0002` underflow.
    let err = call("to_u8", &[Value::Number(Number::from(-1))], &pool, builtins).unwrap_err();
    assert_eq!(err.code(), "R0002");
    assert!(matches!(err, RuntimeError::Underflow(_)));
    assert!(err.to_string().starts_with("underflow:"), "got: {err}");

    // `to_u8(256)`: above the maximum → stays the `R0001` overflow category.
    let err = call(
        "to_u8",
        &[Value::Number(Number::from(256))],
        &pool,
        builtins,
    )
    .unwrap_err();
    assert!(matches!(err, RuntimeError::Overflow(_)));
    assert_eq!(err.code(), "R0001");

    // A signed target below its minimum is also underflow.
    let below = Number::Integer(Box::new(BigInt::from(i64::from(i32::MIN) - 1)));
    let err = call("to_i32", &[Value::Number(below)], &pool, builtins).unwrap_err();
    assert_eq!(err.code(), "R0002");

    // An `isize` target (no `checked_` form) uses the same classification.
    let err = call(
        "to_usize",
        &[Value::Number(Number::from(-1))],
        &pool,
        builtins,
    )
    .unwrap_err();
    assert_eq!(err.code(), "R0002");
}

#[test]
fn checked_and_try_below_minimum_message_is_underflow() {
    let (pool, builtins) = setup();

    let out = call(
        "checked_u8",
        &[Value::Number(Number::from(-1))],
        &pool,
        builtins,
    )
    .unwrap();
    match out {
        Value::Result(Err(msg)) => assert!(msg.starts_with("underflow:"), "got: {msg:?}"),
        other => panic!("expected an Err result, got {other:?}"),
    }

    let out = call(
        "try_u8",
        &[Value::Number(Number::from(-1))],
        &pool,
        builtins,
    )
    .unwrap();
    match out {
        Value::Result(Err(msg)) => assert!(msg.starts_with("underflow:"), "got: {msg:?}"),
        other => panic!("expected an Err result, got {other:?}"),
    }

    // Above the maximum stays an overflow message.
    let out = call(
        "checked_u8",
        &[Value::Number(Number::from(256))],
        &pool,
        builtins,
    )
    .unwrap();
    match out {
        Value::Result(Err(msg)) => assert!(msg.starts_with("overflow:"), "got: {msg:?}"),
        other => panic!("expected an Err result, got {other:?}"),
    }
}

#[test]
fn to_f64_rejects_complex() {
    let (pool, builtins) = setup();
    let c = Number::complex(3, 4);
    let err = call("to_f64", &[Value::Number(c)], &pool, builtins).unwrap_err();
    assert!(matches!(err, RuntimeError::Domain(_)));
}

#[test]
fn try_family_returns_result_values() {
    let (pool, builtins) = setup();
    let in_range = call(
        "try_i8",
        &[Value::Number(Number::from(42))],
        &pool,
        builtins,
    )
    .unwrap();
    assert_eq!(
        in_range,
        Value::Result(Ok(Box::new(Value::Number(Number::I8(42)))))
    );

    let out = call(
        "try_i8",
        &[Value::Number(Number::Integer(Box::new(BigInt::from(200))))],
        &pool,
        builtins,
    )
    .unwrap();
    assert!(
        matches!(out, Value::Result(Err(_))),
        "expected Err result, got {out:?}"
    );

    let nonnum = call("try_i8", &[Value::Bool(true)], &pool, builtins).unwrap();
    assert!(
        matches!(nonnum, Value::Result(Err(_))),
        "expected Err result, got {nonnum:?}"
    );

    let ok_f64 = call(
        "try_f64",
        &[Value::Number(Number::from(3))],
        &pool,
        builtins,
    )
    .unwrap();
    assert!(
        matches!(ok_f64, Value::Result(Ok(v)) if matches!(*v, Value::Number(Number::Real(Real::F64(_)))))
    );

    let ok_usize = call(
        "try_usize",
        &[Value::Number(Number::from(3))],
        &pool,
        builtins,
    )
    .unwrap();
    assert_eq!(
        ok_usize,
        Value::Result(Ok(Box::new(Value::Number(Number::Usize(3)))))
    );
}

#[test]
fn checked_family_and_arithmetic() {
    let (pool, builtins) = setup();
    let ok = call(
        "checked_i32",
        &[Value::Number(Number::from(5))],
        &pool,
        builtins,
    )
    .unwrap();
    assert_eq!(
        ok,
        Value::Result(Ok(Box::new(Value::Number(Number::I32(5)))))
    );

    let out = call(
        "checked_i32",
        &[Value::Number(Number::Integer(Box::new(BigInt::from(
            2_147_483_648i64,
        ))))],
        &pool,
        builtins,
    )
    .unwrap();
    assert!(
        matches!(out, Value::Result(Err(_))),
        "expected Err result, got {out:?}"
    );

    let ok_u128 = call(
        "checked_u128",
        &[Value::Number(Number::Integer(Box::new(BigInt::from(
            u128::MAX,
        ))))],
        &pool,
        builtins,
    )
    .unwrap();
    assert_eq!(
        ok_u128,
        Value::Result(Ok(Box::new(Value::Number(Number::U128(Box::new(
            u128::MAX
        ))))))
    );

    let out_of_range = call(
        "checked_u128",
        &[Value::Number(Number::Integer(Box::new(BigInt::from(-1))))],
        &pool,
        builtins,
    )
    .unwrap();
    assert!(matches!(out_of_range, Value::Result(Err(_))));

    let sum = call(
        "checked_add",
        &[
            Value::Number(Number::from(5)),
            Value::Number(Number::from(7)),
        ],
        &pool,
        builtins,
    )
    .unwrap();
    assert_eq!(
        sum,
        Value::Result(Ok(Box::new(Value::Number(Number::from(12)))))
    );

    let prod = call(
        "checked_mul",
        &[
            Value::Number(Number::from(1_000_000_000)),
            Value::Number(Number::from(1_000_000_000)),
        ],
        &pool,
        builtins,
    )
    .unwrap();
    assert_eq!(
        prod,
        Value::Result(Ok(Box::new(Value::Number(Number::from(
            1_000_000_000_000_000_000i64
        )))))
    );

    let overflow = call(
        "checked_mul",
        &[
            Value::Number(Number::from(10_000_000_000i64)),
            Value::Number(Number::from(10_000_000_000i64)),
        ],
        &pool,
        builtins,
    )
    .unwrap();
    assert!(
        matches!(overflow, Value::Result(Err(_))),
        "expected overflow, got {overflow:?}"
    );
}

#[test]
fn clamped_i32_clamps_to_bounds() {
    let (pool, builtins) = setup();
    let args = |x: i64| {
        vec![
            Value::Number(Number::from(x)),
            Value::Number(Number::from(0)),
            Value::Number(Number::from(255)),
        ]
    };
    assert_eq!(
        call("clamped_i32", &args(1000), &pool, builtins).unwrap(),
        Value::Number(Number::I32(255))
    );
    assert_eq!(
        call("clamped_i32", &args(-5), &pool, builtins).unwrap(),
        Value::Number(Number::I32(0))
    );
    assert_eq!(
        call("clamped_i32", &args(128), &pool, builtins).unwrap(),
        Value::Number(Number::I32(128))
    );
    // Out-of-i8-range bounds saturate to the i8 range on the cast back.
    assert_eq!(
        call("clamped_i8", &args(200), &pool, builtins).unwrap(),
        Value::Number(Number::I8(127))
    );
}

#[test]
fn clamped_u64_bounds() {
    let (pool, builtins) = setup();
    let neg = call(
        "clamped_u64",
        &[Value::Number(Number::from(-5))],
        &pool,
        builtins,
    )
    .unwrap();
    assert_eq!(neg, Value::Number(Number::U64(0)));

    let huge = Number::Real(Real::F64(1e30));
    let v = call("clamped_u64", &[Value::Number(huge)], &pool, builtins).unwrap();
    assert_eq!(v, Value::Number(Number::U64(u64::MAX)));
}

#[test]
fn rounded_f64_digits() {
    let (pool, builtins) = setup();
    let pi = pool.symbol(builtins.pi);
    let v = call(
        "rounded_f64",
        &[Value::Expr(pi), Value::Number(Number::from(3))],
        &pool,
        builtins,
    )
    .unwrap();
    let expected = (std::f64::consts::PI * 1000.0).round() / 1000.0;
    match v {
        Value::Number(Number::Real(Real::F64(x))) => assert_eq!(x, expected),
        other => panic!("expected F64, got {other:?}"),
    }
}

#[test]
fn rounded_f32_digits() {
    let (pool, builtins) = setup();
    let v = call(
        "rounded_f32",
        &[
            Value::Number(Number::from(std::f32::consts::PI as f64)),
            Value::Number(Number::from(2)),
        ],
        &pool,
        builtins,
    )
    .unwrap();
    match v {
        Value::Number(Number::Real(Real::F32(x))) => {
            let expected = (std::f32::consts::PI * 100.0).round() / 100.0;
            assert_eq!(x, expected);
        }
        other => panic!("expected F32, got {other:?}"),
    }
}

#[test]
fn rounded_family_large_float_and_digit_range() {
    let (pool, builtins) = setup();
    // A very large float rounded to 0 digits is unchanged — the result must not saturate.
    let huge = Number::Real(Real::F64(1e300));
    let v = call(
        "rounded_f64",
        &[Value::Number(huge.clone()), Value::Number(Number::from(0))],
        &pool,
        builtins,
    )
    .unwrap();
    assert_eq!(v, Value::Number(huge));

    // A digit count beyond `i32` is rejected instead of wrapping to a bogus exponent.
    let err = call(
        "rounded_f64",
        &[
            Value::Number(Number::Real(Real::F64(1.5))),
            Value::Number(Number::from(i64::from(i32::MAX) + 1)),
        ],
        &pool,
        builtins,
    )
    .unwrap_err();
    assert!(matches!(err, RuntimeError::Overflow(_)));

    // `10^digits` overflowing to infinity must not silently produce `NaN`.
    let err = call(
        "rounded_f64",
        &[
            Value::Number(Number::Real(Real::F64(1.5))),
            Value::Number(Number::from(1000)),
        ],
        &pool,
        builtins,
    )
    .unwrap_err();
    assert!(matches!(err, RuntimeError::Overflow(_)));

    // The f32 entry point shares the same guard.
    let err = call(
        "rounded_f32",
        &[
            Value::Number(Number::Real(Real::F64(1.5))),
            Value::Number(Number::from(i64::from(i32::MAX) + 1)),
        ],
        &pool,
        builtins,
    )
    .unwrap_err();
    assert!(matches!(err, RuntimeError::Overflow(_)));
}

#[test]
fn round_method_does_not_saturate_wide_integers() {
    let (pool, builtins) = setup();
    // The old path cast through `as i64`, saturating `u128::MAX` to `i64::MAX`; `Number::round`
    // normalizes the collapsed layer first and keeps the exact value.
    let v = call(
        "round",
        &[Value::Number(Number::U128(Box::new(u128::MAX)))],
        &pool,
        builtins,
    )
    .unwrap();
    assert_eq!(
        v,
        Value::Number(Number::Integer(Box::new(BigInt::from(u128::MAX))))
    );
}

#[test]
fn rounding_family_on_rational() {
    let (pool, builtins) = setup();
    let r = Number::from(22) / Number::from(7);
    assert!(matches!(r, Number::Rational(_)));

    let v = call("rounded_i32", &[Value::Number(r.clone())], &pool, builtins).unwrap();
    assert_eq!(v, Value::Number(Number::from(3)));

    let v = call("truncated_i32", &[Value::Number(r)], &pool, builtins).unwrap();
    assert_eq!(v, Value::Number(Number::from(3)));

    let neg = Number::from(-22) / Number::from(7);
    let v = call("truncated_i32", &[Value::Number(neg)], &pool, builtins).unwrap();
    assert_eq!(v, Value::Number(Number::from(-3)));
}

#[test]
fn to_bigint_and_to_rational() {
    let (pool, builtins) = setup();
    let v = call(
        "to_bigint",
        &[Value::Number(Number::from(7))],
        &pool,
        builtins,
    )
    .unwrap();
    assert_eq!(v, Value::Number(Number::Integer(Box::new(BigInt::from(7)))));

    let frac = Number::from(7) / Number::from(2);
    let err = call("to_bigint", &[Value::Number(frac.clone())], &pool, builtins).unwrap_err();
    assert!(matches!(err, RuntimeError::Collapse(_)));

    let v = call(
        "to_rational",
        &[Value::Number(Number::from(3))],
        &pool,
        builtins,
    )
    .unwrap();
    assert!(matches!(v, Value::Number(Number::Rational(_))));

    let v = call("to_rational", &[Value::Number(frac)], &pool, builtins).unwrap();
    assert_eq!(v, Value::Number(Number::from(7) / Number::from(2)));
}

#[test]
fn to_complex_wraps_real() {
    let (pool, builtins) = setup();
    let v = call(
        "to_complex",
        &[Value::Number(Number::from(3))],
        &pool,
        builtins,
    )
    .unwrap();
    assert_eq!(
        v,
        Value::Number(Number::from_complex(Number::from(3), Number::from(0)))
    );
}

#[test]
fn unwrap_family() {
    let (pool, builtins) = setup();
    let ok = Value::Result(Ok(Box::new(Value::Number(Number::from(7)))));
    let err = Value::Result(Err(Box::new("boom".to_string())));

    assert_eq!(
        call("unwrap", std::slice::from_ref(&ok), &pool, builtins).unwrap(),
        Value::Number(Number::from(7))
    );
    assert!(matches!(
        call("unwrap", std::slice::from_ref(&err), &pool, builtins),
        Err(RuntimeError::CustomError { message }) if message == "boom"
    ));

    assert_eq!(
        call(
            "unwrap_or",
            &[err.clone(), Value::Number(Number::from(0))],
            &pool,
            builtins
        )
        .unwrap(),
        Value::Number(Number::from(0))
    );
    assert_eq!(
        call(
            "unwrap_or",
            &[ok.clone(), Value::Number(Number::from(0))],
            &pool,
            builtins
        )
        .unwrap(),
        Value::Number(Number::from(7))
    );

    assert_eq!(
        call(
            "expect",
            &[ok.clone(), Value::String("failed".into())],
            &pool,
            builtins
        )
        .unwrap(),
        Value::Number(Number::from(7))
    );
    match call(
        "expect",
        &[err, Value::String("failed".into())],
        &pool,
        builtins,
    ) {
        Err(RuntimeError::CustomError { message }) => assert_eq!(message, "failed: boom"),
        other => panic!("expected `failed: boom`, got {other:?}"),
    }
}

#[test]
fn unwrap_family_accepts_option() {
    let (pool, builtins) = setup();
    let some = Value::Option(Some(Box::new(Value::Number(Number::from(3)))));
    let none = Value::Option(None);

    assert_eq!(
        call("unwrap", std::slice::from_ref(&some), &pool, builtins).unwrap(),
        Value::Number(Number::from(3))
    );
    assert!(matches!(
        call("unwrap", std::slice::from_ref(&none), &pool, builtins),
        Err(RuntimeError::Type(m)) if m.contains("None")
    ));

    assert_eq!(
        call(
            "unwrap_or",
            &[none.clone(), Value::Number(Number::from(0))],
            &pool,
            builtins
        )
        .unwrap(),
        Value::Number(Number::from(0))
    );
    assert_eq!(
        call(
            "unwrap_or",
            &[some.clone(), Value::Number(Number::from(0))],
            &pool,
            builtins
        )
        .unwrap(),
        Value::Number(Number::from(3))
    );
    assert_eq!(
        call(
            "expect",
            &[some, Value::String("wanted".into())],
            &pool,
            builtins
        )
        .unwrap(),
        Value::Number(Number::from(3))
    );
}

#[test]
fn option_result_constructors() {
    let (pool, builtins) = setup();
    assert_eq!(
        call("Some", &[Value::Number(Number::from(3))], &pool, builtins).unwrap(),
        Value::Option(Some(Box::new(Value::Number(Number::from(3)))))
    );
    assert_eq!(
        call("None", &[], &pool, builtins).unwrap(),
        Value::Option(None)
    );
    assert_eq!(
        call("Ok", &[Value::Number(Number::from(3))], &pool, builtins).unwrap(),
        Value::Result(Ok(Box::new(Value::Number(Number::from(3)))))
    );
    assert_eq!(
        call("Err", &[Value::String("boom".into())], &pool, builtins).unwrap(),
        Value::Result(Err(Box::new("boom".into())))
    );
}

#[test]
fn to_string_and_concat() {
    let (pool, builtins) = setup();
    let s = call(
        "to_string",
        &[Value::Number(Number::from(42))],
        &pool,
        builtins,
    )
    .unwrap();
    assert_eq!(s, Value::String("42".into()));

    let s = call(
        "concat",
        &[Value::String("a".into()), Value::String("b".into())],
        &pool,
        builtins,
    )
    .unwrap();
    assert_eq!(s, Value::String("ab".into()));

    let s = call(
        "concat",
        &[Value::Number(Number::from(1)), Value::String("x".into())],
        &pool,
        builtins,
    )
    .unwrap();
    assert_eq!(s, Value::String("1x".into()));
}

#[test]
fn unknown_name_is_a_message_and_arity_is_structured() {
    let (pool, builtins) = setup();
    assert!(matches!(
        call("to_i33", &[], &pool, builtins),
        Err(RuntimeError::UnknownCollapseFunction { .. })
    ));
    assert!(matches!(
        call("to_f64", &[], &pool, builtins),
        Err(RuntimeError::Arity { .. })
    ));
    assert!(matches!(
        call(
            "clamped_f64",
            &[Value::Number(Number::from(1))],
            &pool,
            builtins
        ),
        Err(RuntimeError::Arity { .. })
    ));
    assert!(matches!(
        call("Some", &[], &pool, builtins),
        Err(RuntimeError::Arity { .. })
    ));
    assert!(matches!(
        call("None", &[Value::Number(Number::from(1))], &pool, builtins),
        Err(RuntimeError::Arity { .. })
    ));
}
