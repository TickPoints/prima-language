//! SIMD-accelerated elementwise numeric array operations (spec §10.2, tier `O3`).
//!
//! The interpreter can process dense, homogeneous `F64` array elementwise binary ops (array ⊕ array
//! and array ⊕ scalar) on wide SIMD lanes via the portable `wide` crate. It is applied only when the
//! active `opt_level` is at least `O3` (spec §10.2) **and** every element is a `Real::F64` — a dense
//! numeric array (spec §10.2). Lane-wise IEEE `f64` arithmetic is bit-identical to the scalar path,
//! so this is a pure speed-up that never changes observable results; empty, nested, or non-`F64`
//! arrays simply fall back to the scalar loops in the evaluator.

use wide::f64x4;

use prima_core::{Number, Real};
use prima_syntax::ast::BinOp;

/// Number of `f64` lanes in an `f64x4` (the `wide` portable SIMD vector used here).
const LANES: usize = 4;

/// Whether a slice of numbers is a dense `F64` array (all elements are `Real::F64`).
fn is_dense_f64(numbers: &[Number]) -> bool {
    numbers
        .iter()
        .all(|n| matches!(n, Number::Real(Real::F64(_))))
}

/// Extract the `f64` payload of a dense `F64` number; call only after [`is_dense_f64`].
fn lane_value(n: &Number) -> f64 {
    match n {
        Number::Real(Real::F64(x)) => *x,
        _ => unreachable!("SIMD called on a non-F64 number"),
    }
}

/// Scalar `f64 ∘ f64` (bit-identical to the vector kernel) for the tail lane remainder.
/// `Pow`/`Mod`/comparisons are left to the scalar evaluator so their per-element semantics (domain
/// checks, exact integer handling, NaN/Inf propagation) stay untouched.
fn scalar_binary(op: BinOp, x: f64, y: f64) -> f64 {
    match op {
        BinOp::Add => x + y,
        BinOp::Sub => x - y,
        BinOp::Mul => x * y,
        // F64 division is pure IEEE (spec §6.2): the custom `0/0` rule and the division-by-zero error
        // apply only to the exact layer, and `fraction := false` is a no-op for an already-`F64`
        // operand — so lane-wise division matches the scalar path bit-for-bit.
        BinOp::Div => x / y,
        _ => unreachable!("scalar_binary called on a non-arithmetic op"),
    }
}

fn push_lanes(r: f64x4, out: &mut Vec<Number>) {
    for v in r.to_array() {
        out.push(Number::Real(Real::F64(v)));
    }
}

/// Whether a `BinOp` is supported on the SIMD kernel (only IEEE-safe arithmetic).
fn is_vectorizable(op: BinOp) -> bool {
    matches!(op, BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div)
}

/// Try a SIMD elementwise binary op over two dense `F64` arrays (spec §10.2, tier `O3`).
/// Returns `None` when the inputs are not a straightforward dense-`F64` case.
pub fn try_f64x4_arrays(op: BinOp, a: &[Number], b: &[Number]) -> Option<Vec<Number>> {
    if a.is_empty() || b.len() != a.len() || a.len() < LANES || !is_dense_f64(a) || !is_dense_f64(b) || !is_vectorizable(op) {
        return None;
    }
    let mut out = Vec::with_capacity(a.len());
    let mut i = 0;
    while i + LANES <= a.len() {
        let va = f64x4::from([lane_value(&a[i]), lane_value(&a[i + 1]), lane_value(&a[i + 2]), lane_value(&a[i + 3])]);
        let vb = f64x4::from([lane_value(&b[i]), lane_value(&b[i + 1]), lane_value(&b[i + 2]), lane_value(&b[i + 3])]);
        push_lanes(binary_vec(op, va, vb), &mut out);
        i += LANES;
    }
    for (x, y) in a[i..].iter().zip(b[i..].iter()) {
        out.push(Number::Real(Real::F64(scalar_binary(op, lane_value(x), lane_value(y)))));
    }
    Some(out)
}

/// Try a SIMD elementwise binary op broadcasting a dense-`F64` scalar across a dense-`F64` array
/// (`array ⊕ scalar`; spec §11.4).
pub fn try_f64x4_scalar(op: BinOp, arr: &[Number], scalar: &Number) -> Option<Vec<Number>> {
    if arr.is_empty() || arr.len() < LANES || !is_dense_f64(arr) || !matches!(scalar, Number::Real(Real::F64(_))) || !is_vectorizable(op) {
        return None;
    }
    let s = lane_value(scalar);
    let mut out = Vec::with_capacity(arr.len());
    let mut i = 0;
    while i + LANES <= arr.len() {
        let va = f64x4::from([lane_value(&arr[i]), lane_value(&arr[i + 1]), lane_value(&arr[i + 2]), lane_value(&arr[i + 3])]);
        push_lanes(binary_vec(op, va, f64x4::splat(s)), &mut out);
        i += LANES;
    }
    for x in &arr[i..] {
        out.push(Number::Real(Real::F64(scalar_binary(op, lane_value(x), s))));
    }
    Some(out)
}

/// Try a SIMD elementwise binary op broadcasting a dense-`F64` scalar to the LEFT of an array
/// (`scalar ⊕ array`; spec §11.4).
pub fn try_f64x4_scalar_left(op: BinOp, scalar: &Number, arr: &[Number]) -> Option<Vec<Number>> {
    if arr.is_empty() || arr.len() < LANES || !is_dense_f64(arr) || !matches!(scalar, Number::Real(Real::F64(_))) || !is_vectorizable(op) {
        return None;
    }
    let s = lane_value(scalar);
    let mut out = Vec::with_capacity(arr.len());
    let mut i = 0;
    while i + LANES <= arr.len() {
        let va = f64x4::from([lane_value(&arr[i]), lane_value(&arr[i + 1]), lane_value(&arr[i + 2]), lane_value(&arr[i + 3])]);
        push_lanes(binary_scalar_left(op, f64x4::splat(s), va), &mut out);
        i += LANES;
    }
    for x in &arr[i..] {
        out.push(Number::Real(Real::F64(scalar_binary(op, s, lane_value(x)))));
    }
    Some(out)
}

/// Lane-wise `left ∘ right` for a vectorized op.
fn binary_vec(op: BinOp, left: f64x4, right: f64x4) -> f64x4 {
    match op {
        BinOp::Add => left + right,
        BinOp::Sub => left - right,
        BinOp::Mul => left * right,
        BinOp::Div => left / right,
        _ => unreachable!("binary_vec called on a non-arithmetic op"),
    }
}

/// Lane-wise `scalar ∘ array` (non-commutative ops keep the scalar on the left).
fn binary_scalar_left(op: BinOp, left: f64x4, right: f64x4) -> f64x4 {
    match op {
        BinOp::Add => left + right,
        BinOp::Sub => left - right,
        BinOp::Mul => left * right,
        BinOp::Div => left / right,
        _ => unreachable!("binary_scalar_left called on a non-arithmetic op"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f64s(v: &[f64]) -> Vec<Number> {
        v.iter().map(|x| Number::Real(Real::F64(*x))).collect()
    }

    fn scalar_ref(op: BinOp, a: &[f64], b: &[f64]) -> Vec<f64> {
        a.iter()
            .zip(b.iter())
            .map(|(x, y)| scalar_binary(op, *x, *y))
            .collect()
    }

    #[test]
    fn array_array_matches_scalar() {
        let a = f64s(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0]);
        let b = f64s(&[-1.0, 2.5, 0.0, 4.0, 5.0, -6.0, 1.0, 8.0]);
        for op in [BinOp::Add, BinOp::Sub, BinOp::Mul, BinOp::Div] {
            let got = try_f64x4_arrays(op, &a, &b).expect("dense f64 arrays vectorize");
            let expect = scalar_ref(op, &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0], &[-1.0, 2.5, 0.0, 4.0, 5.0, -6.0, 1.0, 8.0]);
            for (g, e) in got.iter().zip(expect.iter()) {
                assert_eq!(lane_value(g), *e, "op {op:?}");
            }
        }
    }

    #[test]
    fn scalar_right_and_left_match_scalar() {
        let a = f64s(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0]);
        let s = Number::Real(Real::F64(2.0));
        for op in [BinOp::Add, BinOp::Sub, BinOp::Mul, BinOp::Div] {
            let right = try_f64x4_scalar(op, &a, &s).expect("array ∘ scalar vectorizes");
            for (g, x) in right.iter().zip(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0]) {
                assert_eq!(lane_value(g), scalar_binary(op, *x, 2.0), "right op {op:?}");
            }
            let left = try_f64x4_scalar_left(op, &s, &a).expect("scalar ∘ array vectorizes");
            for (g, x) in left.iter().zip(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0]) {
                assert_eq!(lane_value(g), scalar_binary(op, 2.0, *x), "left op {op:?}");
            }
        }
    }

    #[test]
    fn non_f64_or_short_falls_back() {
        // Empty, too short, or non-F64 inputs must not vectorize (caller uses the scalar path).
        let a = f64s(&[1.0, 2.0, 3.0]);
        let b = f64s(&[1.0, 2.0, 3.0]);
        assert!(try_f64x4_arrays(BinOp::Add, &a, &b).is_none());
        let mixed = vec![Number::from(1), Number::Real(Real::F64(2.0)), Number::Real(Real::F64(3.0)), Number::Real(Real::F64(4.0))];
        let dense = f64s(&[1.0, 2.0, 3.0, 4.0]);
        assert!(try_f64x4_arrays(BinOp::Add, &mixed, &dense).is_none());
        assert!(try_f64x4_arrays(BinOp::Pow, &dense, &dense).is_none());
        assert!(try_f64x4_scalar(BinOp::Add, &b, &Number::Real(Real::F64(2.0))).is_none());
    }
}
