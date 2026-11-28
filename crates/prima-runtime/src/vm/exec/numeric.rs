//! Numeric fast paths for VM arithmetic and comparison (spec §6.1/§6.4).

use prima_core::{Number, Real, Value};
use prima_syntax::ast::BinOp as AstBinOp;

use super::ops::arith_binop;
use crate::error::RuntimeError;
use crate::eval::Evaluator;
use crate::vm::op::ArithOp;

impl Evaluator {
    /// Numeric binary fast path (spec §6.1): `+ - * %` compute on the `Number` tower directly (the
    /// Small i64 checked fast paths and exact BigInt overflow handling live inside `Number`).
    /// Division replicates `eval_number_binary`: exact-layer zero divisors error (with the
    /// `undefined_handling := custom` black magic, spec §13.4) and `fraction := false` drops the
    /// result to F64 (spec §13.3).
    pub(crate) fn vm_number_binary(
        &mut self,
        op: ArithOp,
        x: prima_core::Number,
        y: prima_core::Number,
    ) -> Result<Value, RuntimeError> {
        use prima_core::{Number, Real};
        Ok(match op {
            ArithOp::Add => Value::Number(x + y),
            ArithOp::Sub => Value::Number(x - y),
            ArithOp::Mul => Value::Number(x * y),
            ArithOp::Div => {
                // Exact-layer division by zero: `0/0` is evaluated by black magic under the custom
                // policy (spec §13.4), otherwise the numeric layer errors (spec §6.2).
                if y.is_zero() && !matches!(y, Number::Real(_)) {
                    if x.is_zero()
                        && let Some(v) = self.custom_zero_div()
                    {
                        return Ok(v);
                    }
                    return crate::error::type_err("division by zero");
                }
                let r = x / y;
                // `fraction := false` (spec §13.3): the division result drops to F64.
                if self.current_config().fraction {
                    Value::Number(r)
                } else {
                    Value::Number(Number::Real(Real::F64(r.to_f64_lossy())))
                }
            }
            ArithOp::Rem => Value::Number(crate::eval::number_mod(&x, &y)?),
        })
    }

    /// Register-form arithmetic (spec §6.1/§12.2): `Small`/`F64` operands use the same inline fast
    /// paths as the stack instructions (`F64` division/remainder are already collapsed, so they
    /// are policy-independent); the exact tower handles everything else, and non-numbers keep the
    /// full `eval_binary` semantics (operator overloads, array concatenation, `Undefined`
    /// strictness), with the instruction's span attached.
    #[inline]
    pub(crate) fn reg_arith(
        &mut self,
        op: ArithOp,
        x: Value,
        y: Value,
        span: prima_syntax::Span,
    ) -> Result<Value, RuntimeError> {
        match (x, y) {
            (Value::Number(Number::Small(a)), Value::Number(Number::Small(b))) => {
                let narrowed = match op {
                    ArithOp::Add => a.checked_add(b).map(Number::Small),
                    ArithOp::Sub => a.checked_sub(b).map(Number::Small),
                    ArithOp::Mul => a.checked_mul(b).map(Number::Small),
                    // Integer remainder is exact and allocation-free; a zero divisor keeps the
                    // exact-tower diagnostic path below. Division keeps the policy path.
                    ArithOp::Rem => {
                        if b != 0 {
                            a.checked_rem(b).map(Number::Small)
                        } else {
                            None
                        }
                    }
                    ArithOp::Div => None,
                };
                match narrowed {
                    Some(n) => Ok(Value::Number(n)),
                    None => self
                        .vm_number_binary(op, Number::Small(a), Number::Small(b))
                        .map_err(|e| crate::error::attach_span(e, span)),
                }
            }
            (
                Value::Number(Number::Real(Real::F64(a))),
                Value::Number(Number::Real(Real::F64(b))),
            ) => {
                let r = match op {
                    ArithOp::Add => a + b,
                    ArithOp::Sub => a - b,
                    ArithOp::Mul => a * b,
                    ArithOp::Div => a / b,
                    ArithOp::Rem => a % b,
                };
                Ok(Value::Number(Number::Real(Real::F64(r))))
            }
            (Value::Number(a), Value::Number(b)) => self
                .vm_number_binary(op, a, b)
                .map_err(|e| crate::error::attach_span(e, span)),
            (a, b) => self
                .eval_binary(arith_binop(op), a, b)
                .map_err(|e| crate::error::attach_span(e, span)),
        }
    }

    /// Fused multiply-accumulate `acc + a*b` (spec §10): `Small`/`F64` inline fast paths with the
    /// exact-tower widening fallback.
    pub(crate) fn reg_mul_add(
        &mut self,
        acc: Value,
        a: Value,
        b: Value,
        span: prima_syntax::Span,
    ) -> Result<Value, RuntimeError> {
        use prima_core::{Number, Real};
        if let (
            Value::Number(Number::Small(s)),
            Value::Number(Number::Small(x)),
            Value::Number(Number::Small(y)),
        ) = (&acc, &a, &b)
        {
            if let Some(p) = x.checked_mul(*y)
                && let Some(r) = s.checked_add(p)
            {
                return Ok(Value::Number(Number::Small(r)));
            }
        } else if let (
            Value::Number(Number::Real(Real::F64(s))),
            Value::Number(Number::Real(Real::F64(x))),
            Value::Number(Number::Real(Real::F64(y))),
        ) = (&acc, &a, &b)
        {
            return Ok(Value::Number(Number::Real(Real::F64(s + x * y))));
        }
        let prod = self.reg_arith(ArithOp::Mul, a, b, span)?;
        self.reg_arith(ArithOp::Add, acc, prod, span)
    }

    /// Numeric comparison fast path: promote to a common type before comparing (spec §6.4, so
    /// `1 == 1.0` holds), mirroring `eval_compare`'s numeric arm.
    pub(crate) fn vm_compare_numbers(
        &mut self,
        op: AstBinOp,
        x: prima_core::Number,
        y: prima_core::Number,
    ) -> Result<Value, RuntimeError> {
        use prima_core::{Number, Real, number::promote};
        use std::cmp::Ordering;
        let ord = if let (Number::Small(a), Number::Small(b)) = (&x, &y) {
            Some(a.cmp(b))
        } else {
            let (px, py) = promote(&x, &y);
            match (&px, &py) {
                (Number::Small(a), Number::Small(b)) => Some(a.cmp(b)),
                (Number::Integer(a), Number::Integer(b)) => Some(a.cmp(b)),
                (Number::Rational(a), Number::Rational(b)) => Some(a.cmp(b)),
                (Number::Real(Real::F32(a)), Number::Real(Real::F32(b))) => a.partial_cmp(b),
                (Number::Real(Real::F64(a)), Number::Real(Real::F64(b))) => a.partial_cmp(b),
                _ => None,
            }
        }
        .ok_or_else(|| RuntimeError::UncomparableNumbers)?;
        Ok(Value::Bool(match op {
            AstBinOp::Eq => ord == Ordering::Equal,
            AstBinOp::Ne => ord != Ordering::Equal,
            AstBinOp::Lt => ord == Ordering::Less,
            AstBinOp::Le => ord != Ordering::Greater,
            AstBinOp::Gt => ord == Ordering::Greater,
            AstBinOp::Ge => ord != Ordering::Less,
            _ => unreachable!("vm_compare_numbers only handles comparison ops"),
        }))
    }
}
