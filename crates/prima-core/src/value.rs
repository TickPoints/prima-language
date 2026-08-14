use std::collections::{HashMap, HashSet};

use num_rational::BigRational;
use num_traits::ToPrimitive;

use crate::expr_pool::ExprId;
use crate::number::{Number, Real};

/// Indeterminate form (spec §6.2): mathematically undefined forms (0/0 etc.) that exist **only in the symbolic layer**;
/// they can take part in later simplification; when collapse to the numeric layer fails they become `Undefined`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum IndeterminateForm {
    ZeroOverZero,
    InfOverInf,
    ZeroTimesInf,
    InfMinusInf,
}

/// Value type (spec §5): covers the value forms of each layer of the three-world architecture —
/// the symbolic layer (`Expr`/`Symbol`/`Indeterminate`), the numeric layer (`Number`), and the host layer
/// (`Bool`/`String`/`Error`, etc.). `Array` is a variable-length heterogeneous sequence (v2.1, spec §11.3);
/// `Dict`/`Set` are variable host collections keyed/elemented by immutable hashable `ValueKey`s (spec §4.6/§11.6).
/// `Result`/`Error` carry a structured `Error` as a message string (the structured enum from spec §16.1 is deferred to a later stage).
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Nil,
    Number(Number),
    Bool(bool),
    Char(char),
    String(String),
    Array(Vec<Value>),
    Dict(HashMap<ValueKey, Value>),
    Set(HashSet<ValueKey>),
    Expr(ExprId),
    Symbol(u32),
    Indeterminate(IndeterminateForm),
    Undefined,
    Error(String),
    Tuple(Vec<Value>),
    Result(std::result::Result<Box<Value>, String>),
    Class(u32),                       // class instance handle (spec §5); registry lives in prima-runtime
    Option(Option<Box<Value>>),       // Option<T>: Some(T) / None
}

/// Hashable key for `Dict`/`Set` (spec §11.6): a value-semantic, immutable subset of `Value` —
/// numbers (canonicalized), strings, chars, bools, and symbol/expr handles.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ValueKey {
    Int(i64),
    BigInt(num_bigint::BigInt),
    Rational(num_bigint::BigInt, num_bigint::BigInt), // reduced fraction; denominator positive
    Float(u64),          // f64 bit pattern; NaN keys are rejected
    Str(String),
    Char(char),
    Bool(bool),
    Symbol(u32),         // SymbolId.0
    Expr(u32),           // ExprId inner value
}

impl ValueKey {
    /// Convert a `Value` to a hashable key, or `None` if the value is not a valid key type
    /// (complex numbers, arrays, dicts, sets, class instances, etc. → `None`; NaN → `None`).
    pub fn from_value(v: &Value) -> Option<ValueKey> {
        match v {
            Value::Number(n) => number_to_key(n),
            Value::String(s) => Some(ValueKey::Str(s.clone())),
            Value::Char(c) => Some(ValueKey::Char(*c)),
            Value::Bool(b) => Some(ValueKey::Bool(*b)),
            Value::Symbol(s) => Some(ValueKey::Symbol(*s)),
            Value::Expr(id) => Some(ValueKey::Expr(id.as_u32())),
            _ => None,
        }
    }

    /// Reconstruct the corresponding `Value` (numeric keys produce `Number`, `Symbol` produces
    /// `Value::Symbol`, `Expr` produces `Value::Expr`).
    pub fn to_value(&self) -> Value {
        match self {
            ValueKey::Int(i) => Value::Number(Number::from(*i)),
            ValueKey::BigInt(b) => Value::Number(Number::Integer(b.clone())),
            ValueKey::Rational(n, d) => Value::Number(Number::Rational(BigRational::new(n.clone(), d.clone()))),
            ValueKey::Float(bits) => Value::Number(Number::Real(Real::F64(f64::from_bits(*bits)))),
            ValueKey::Str(s) => Value::String(s.clone()),
            ValueKey::Char(c) => Value::Char(*c),
            ValueKey::Bool(b) => Value::Bool(*b),
            ValueKey::Symbol(s) => Value::Symbol(*s),
            ValueKey::Expr(u) => Value::Expr(ExprId::from_u32(*u)),
        }
    }
}

/// Map a `Number` to a hashable key (spec §11.6): integers that fit `i64` → `Int` (else `BigInt`),
/// rationals → reduced `Rational` with positive denominator, reals → `Float` bit pattern (NaN → `None`),
/// complex → `None`. Fixed-width collapsed variants are keyed by value after normalizing to the
/// exact/`Real` layer (spec §6.1).
fn number_to_key(n: &Number) -> Option<ValueKey> {
    if n.is_complex() {
        return None;
    }
    match n {
        Number::Integer(i) => match i.to_i64() {
            Some(v) => Some(ValueKey::Int(v)),
            None => Some(ValueKey::BigInt(i.clone())),
        },
        Number::Rational(r) => {
            // Re-normalize so the key always holds a reduced fraction with a positive denominator (spec §6.1).
            let r = BigRational::new(r.numer().clone(), r.denom().clone());
            Some(ValueKey::Rational(r.numer().clone(), r.denom().clone()))
        }
        Number::Real(Real::F64(x)) => (!x.is_nan()).then_some(ValueKey::Float(x.to_bits())),
        Number::Real(Real::F32(x)) => (!x.is_nan()).then_some(ValueKey::Float((*x as f64).to_bits())),
        Number::BigFloat(f) => (!f.is_nan()).then_some(ValueKey::Float(f.to_bits())),
        // Fixed-width collapsed integers normalize to the exact layer (spec §6.1).
        other => match other.as_i64() {
            Some(v) => Some(ValueKey::Int(v)),
            None => other.as_bigint().map(ValueKey::BigInt),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use num_bigint::BigInt;
    use num_rational::BigRational;

    use crate::expr_pool::ExprPool;

    /// `from_value` → key, `to_value` → the original value (round-trip, spec §11.6).
    fn assert_roundtrip(v: Value, key: ValueKey) {
        let k = ValueKey::from_value(&v).unwrap_or_else(|| panic!("expected a key for {v:?}"));
        assert_eq!(k, key);
        assert_eq!(k.to_value(), v);
    }

    #[test]
    fn int_key_roundtrip() {
        assert_roundtrip(Value::Number(Number::from(1)), ValueKey::Int(1));
        assert_roundtrip(Value::Number(Number::from(-7)), ValueKey::Int(-7));
    }

    #[test]
    fn bigint_key_roundtrip() {
        let big = BigInt::from(i64::MAX) + BigInt::from(1);
        assert_roundtrip(Value::Number(Number::Integer(big.clone())), ValueKey::BigInt(big));
    }

    #[test]
    fn rational_key_roundtrip() {
        assert_roundtrip(
            Value::Number(Number::Rational(BigRational::new(BigInt::from(1), BigInt::from(3)))),
            ValueKey::Rational(BigInt::from(1), BigInt::from(3)),
        );
        // The key always holds a reduced fraction with a positive denominator (spec §6.1).
        assert_roundtrip(
            Value::Number(Number::Rational(BigRational::new(BigInt::from(2), BigInt::from(-3)))),
            ValueKey::Rational(BigInt::from(-2), BigInt::from(3)),
        );
    }

    #[test]
    fn float_key_roundtrip() {
        assert_roundtrip(Value::Number(Number::from(2.5)), ValueKey::Float(2.5f64.to_bits()));
        // F32 promotes to F64 when keyed (spec §6.1 promotion); the key is the F64 bit pattern.
        let v = Value::Number(Number::Real(Real::F32(1.5)));
        assert_eq!(ValueKey::from_value(&v), Some(ValueKey::Float((1.5f32 as f64).to_bits())));
        assert_eq!(
            ValueKey::Float((1.5f32 as f64).to_bits()).to_value(),
            Value::Number(Number::Real(Real::F64(1.5)))
        );
    }

    #[test]
    fn scalar_key_roundtrip() {
        assert_roundtrip(Value::String("hello".to_string()), ValueKey::Str("hello".to_string()));
        assert_roundtrip(Value::Char('x'), ValueKey::Char('x'));
        assert_roundtrip(Value::Bool(true), ValueKey::Bool(true));
    }

    #[test]
    fn symbol_and_expr_keys() {
        assert_eq!(ValueKey::from_value(&Value::Symbol(42)), Some(ValueKey::Symbol(42)));
        assert_eq!(ValueKey::Symbol(42).to_value(), Value::Symbol(42));

        let pool = ExprPool::new();
        let id = pool.integer(3);
        assert_eq!(ValueKey::from_value(&Value::Expr(id)), Some(ValueKey::Expr(id.as_u32())));
        assert_eq!(ValueKey::Expr(id.as_u32()).to_value(), Value::Expr(id));
    }

    #[test]
    fn unsupported_values_are_none() {
        assert_eq!(ValueKey::from_value(&Value::Number(Number::complex(1, 2))), None);
        assert_eq!(ValueKey::from_value(&Value::Array(vec![Value::Number(Number::from(1))])), None);
        assert_eq!(ValueKey::from_value(&Value::Dict(HashMap::new())), None);
        assert_eq!(ValueKey::from_value(&Value::Set(HashSet::new())), None);
        assert_eq!(ValueKey::from_value(&Value::Undefined), None);
        assert_eq!(ValueKey::from_value(&Value::Nil), None);
    }

    #[test]
    fn nan_is_not_a_key() {
        assert_eq!(ValueKey::from_value(&Value::Number(Number::Real(Real::F64(f64::NAN)))), None);
        assert_eq!(ValueKey::from_value(&Value::Number(Number::Real(Real::F32(f32::NAN)))), None);
    }

    #[test]
    fn to_value_reconstructs_values() {
        assert_eq!(ValueKey::Int(5).to_value(), Value::Number(Number::from(5)));
        assert_eq!(
            ValueKey::BigInt(BigInt::from(1u64 << 40)).to_value(),
            Value::Number(Number::Integer(BigInt::from(1u64 << 40)))
        );
        assert_eq!(ValueKey::Float(2.5f64.to_bits()).to_value(), Value::Number(Number::Real(Real::F64(2.5))));
        assert_eq!(ValueKey::Str("abc".to_string()).to_value(), Value::String("abc".to_string()));
        assert_eq!(ValueKey::Char('z').to_value(), Value::Char('z'));
        assert_eq!(ValueKey::Bool(false).to_value(), Value::Bool(false));
    }

    #[test]
    fn dict_and_set_hold_hashable_keys() {
        let mut m = HashMap::new();
        m.insert(ValueKey::Str("a".into()), Value::Number(Number::from(1)));
        let Value::Dict(d) = Value::Dict(m) else { unreachable!() };
        assert_eq!(d.get(&ValueKey::Str("a".into())), Some(&Value::Number(Number::from(1))));

        let mut s = HashSet::new();
        s.insert(ValueKey::Int(1));
        s.insert(ValueKey::Int(1));
        s.insert(ValueKey::Float(2.5f64.to_bits()));
        let Value::Set(s) = Value::Set(s) else { unreachable!() };
        assert_eq!(s.len(), 2);
        assert!(s.contains(&ValueKey::Int(1)));
        assert!(s.contains(&ValueKey::Float(2.5f64.to_bits())));
    }
}
