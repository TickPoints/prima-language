use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::{Arc, RwLock};

use num_bigint::BigInt;
use num_rational::BigRational;
use num_traits::{One, ToPrimitive};

use crate::expr_pool::ExprId;
use crate::number::{Number, Real};

/// Shared, copy-on-write array buffer (spec §11.3 value semantics).
///
/// `Value::Array` holds this handle instead of an owned `Vec<Value>`, so cloning an array value is
/// O(1) (an `Arc` bump) and the three hot mutation paths — `A[i] = v`, the mutating `Array` methods
/// (`push`/`pop`/…), and bytecode `IndexStore` — mutate the buffer in place when it is uniquely
/// owned. `with_mut` performs the copy-on-write: when the handle is shared the buffer is cloned
/// first, so aliases (`let b = a; b.push(x)`) keep the old contents.
///
/// `Arc`/`RwLock` (not `Rc`/`RefCell`) keep `Value: Send + Sync`, which the parallel paths require
/// (`parfor` snapshots and `@parallel` broadcast share array values across rayon threads, reading
/// them through `with`).
#[derive(Clone, Default)]
pub struct ArrayVal(Arc<RwLock<Vec<Value>>>);

impl ArrayVal {
    /// An empty array.
    pub fn new() -> ArrayVal {
        ArrayVal(Arc::new(RwLock::new(Vec::new())))
    }

    /// Wrap an owned element buffer.
    pub fn from_vec(items: Vec<Value>) -> ArrayVal {
        ArrayVal(Arc::new(RwLock::new(items)))
    }

    /// Read-only access to the element buffer. The closure must not mutate the same array
    /// (no `with_mut` re-entrancy) — keep the borrow window minimal.
    pub fn with<R>(&self, f: impl FnOnce(&[Value]) -> R) -> R {
        let buf = self.0.read().expect("array buffer read");
        f(&buf)
    }

    /// Mutating access with copy-on-write: a shared handle is cloned first (value semantics,
    /// spec §11.3), a unique handle mutates in place. Requires `&mut self`, so callers mutate
    /// an owned handle taken from a binding/slot (the env slot itself for in-place updates).
    pub fn with_mut<R>(&mut self, f: impl FnOnce(&mut Vec<Value>) -> R) -> R {
        if Arc::strong_count(&self.0) > 1 {
            let buf = self.0.read().expect("array buffer read").clone();
            self.0 = Arc::new(RwLock::new(buf));
        }
        let mut buf = self.0.write().expect("array buffer write");
        f(&mut buf)
    }

    /// Number of elements.
    pub fn len(&self) -> usize {
        self.0.read().expect("array buffer read").len()
    }

    /// Whether the array is empty.
    pub fn is_empty(&self) -> bool {
        self.0.read().expect("array buffer read").is_empty()
    }

    /// Clone the element at `i`, or `None` out of bounds.
    pub fn get(&self, i: usize) -> Option<Value> {
        self.0.read().expect("array buffer read").get(i).cloned()
    }

    /// Append an element (copy-on-write through `with_mut`).
    pub fn push(&mut self, v: Value) {
        self.with_mut(|items| items.push(v));
    }

    /// Remove and return the last element (copy-on-write through `with_mut`).
    pub fn pop(&mut self) -> Option<Value> {
        self.with_mut(|items| items.pop())
    }

    /// Copy the element buffer out (O(n)); prefer `with` for read-only access.
    pub fn to_vec(&self) -> Vec<Value> {
        self.0.read().expect("array buffer read").clone()
    }

    /// Owned iteration (clones elements out; O(n)). Prefer `with` for read-only access.
    pub fn iter(&self) -> std::vec::IntoIter<Value> {
        self.to_vec().into_iter()
    }

    /// Whether two handles share the same underlying buffer.
    pub fn is_same_buffer(&self, other: &ArrayVal) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }

    /// A fresh buffer holding a copy of the elements (never shares storage with `self`).
    pub fn snapshot(&self) -> ArrayVal {
        ArrayVal::from_vec(self.to_vec())
    }
}

impl From<Vec<Value>> for ArrayVal {
    fn from(items: Vec<Value>) -> ArrayVal {
        ArrayVal::from_vec(items)
    }
}

/// Structural equality by content (spec §11.3): elementwise `Value` equality, independent of
/// buffer sharing. Read-only borrows nest safely (multiple readers).
impl PartialEq for ArrayVal {
    fn eq(&self, other: &Self) -> bool {
        if Arc::ptr_eq(&self.0, &other.0) {
            return true;
        }
        if self.len() != other.len() {
            return false;
        }
        self.with(|a| other.with(|b| a.iter().zip(b.iter()).all(|(x, y)| x == y)))
    }
}

/// Renders exactly like the underlying `Vec<Value>` (the shared handle is not observable).
impl fmt::Debug for ArrayVal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.with(|items| f.debug_list().entries(items.iter()).finish())
    }
}

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
/// (`Bool`/`String`/`Error`, etc.). `Array` is a variable-length heterogeneous sequence with a shared
/// copy-on-write buffer (v2.1, spec §11.3; see [`ArrayVal`]);
/// `Dict`/`Set` are variable host collections keyed/elemented by immutable hashable `ValueKey`s (spec §4.6/§11.6).
/// `Result`/`Error` carry a structured `Error` as a message string (the structured enum from spec §16.1 is deferred to a later release).
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Nil,
    Number(Number),
    Bool(bool),
    Char(char),
    String(String),
    Array(ArrayVal),
    // `Dict`/`Set` are boxed: the 48-byte `HashMap`/`HashSet` headers would otherwise dominate
    // this enum's size on the interpreter/VM hot path; the boxed handle keeps `Value` compact.
    Dict(Box<HashMap<ValueKey, Value>>),
    Set(Box<HashSet<ValueKey>>),
    Expr(ExprId),
    Symbol(u32),
    Indeterminate(IndeterminateForm),
    Undefined,
    Error(String),
    Tuple(Vec<Value>),
    // `Err` carries `Box<String>` so the inlined `Result` payload stays pointer-sized.
    Result(std::result::Result<Box<Value>, Box<String>>),
    Class(u32), // class instance handle (spec §5); registry lives in prima-runtime
    /// Compiled/JIT-ed function handle (spec §19.2/§19.4): a process-local id into the
    /// `prima-runtime::jit` registry. Only produced by the `jit(...)` builtin. Like `Class`,
    /// the id is process-local (no cross-process serialization) and lives for the process lifetime.
    JitFunction(u32),
    Option(Option<Box<Value>>), // Option<T>: Some(T) / None
}

/// Hashable key for `Dict`/`Set` (spec §11.6): a value-semantic, immutable subset of `Value` —
/// numbers (canonicalized), strings, chars, bools, and symbol/expr handles.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ValueKey {
    Int(i64),
    BigInt(num_bigint::BigInt),
    Rational(num_bigint::BigInt, num_bigint::BigInt), // reduced fraction; denominator positive
    Float(u64),                                       // f64 bit pattern; NaN keys are rejected
    Str(String),
    Char(char),
    Bool(bool),
    Symbol(u32), // SymbolId.0
    Expr(u32),   // ExprId inner value
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

    /// Reconstruct the corresponding `Value` (numeric keys produce `Number` at the canonical
    /// layer — `Int`/`BigInt` → integer, `Float` → `Real`; a key's original representation (e.g.
    /// `2.0` keyed as `Int(2)`) is deliberately not preserved, since all Dict/Set lookups match
    /// on keys, not on `to_value` output).
    pub fn to_value(&self) -> Value {
        match self {
            ValueKey::Int(i) => Value::Number(Number::from(*i)),
            ValueKey::BigInt(b) => Value::Number(Number::Integer(Box::new(b.clone()))),
            ValueKey::Rational(n, d) => Value::Number(Number::Rational(Box::new(
                BigRational::new(n.clone(), d.clone()),
            ))),
            ValueKey::Float(bits) => Value::Number(Number::Real(Real::F64(f64::from_bits(*bits)))),
            ValueKey::Str(s) => Value::String(s.clone()),
            ValueKey::Char(c) => Value::Char(*c),
            ValueKey::Bool(b) => Value::Bool(*b),
            ValueKey::Symbol(s) => Value::Symbol(*s),
            ValueKey::Expr(u) => Value::Expr(ExprId::from_u32(*u)),
        }
    }
}

/// Map a `Number` to a hashable key (spec §11.6): numeric keys are **canonicalized by value**
/// (Python `dict`/`set` semantics — `Dict`/`Set` are specified as modeled after them), so
/// numerically equal numbers share one key regardless of layer:
/// - integers (fitting `i64` → `Int`, else `BigInt`); integral-valued floats (`2.0`, `-0.0`) and
///   integral rationals (`4/2`) key as the integers they equal;
/// - other rationals → reduced `Rational` with positive denominator (spec §6.1);
/// - non-integral reals → `Float` f64 bit pattern (NaN → `None`);
/// - complex → `None`. Fixed-width collapsed variants are keyed by value after normalizing to
///   the exact/`Real` layer (spec §6.1).
///
/// Note the residual asymmetry with full Python semantics: a non-integral rational (`3/2`) and
/// the float it equals (`1.5`) remain distinct keys (the exact layer refuses to identify them
/// through a float bit pattern); exactness is the stronger invariant here (spec §6.1).
fn number_to_key(n: &Number) -> Option<ValueKey> {
    if n.is_complex() {
        return None;
    }
    match n {
        Number::Integer(i) => integer_to_key((**i).clone()),
        Number::Rational(r) => {
            // Re-normalize so the key always holds a reduced fraction with a positive denominator (spec §6.1).
            let r = BigRational::new(r.numer().clone(), r.denom().clone());
            if *r.denom() == BigInt::one() {
                integer_to_key(r.numer().clone())
            } else {
                Some(ValueKey::Rational(r.numer().clone(), r.denom().clone()))
            }
        }
        Number::Real(Real::F64(x)) => float_to_key(*x),
        Number::Real(Real::F32(x)) => float_to_key(*x as f64),
        Number::BigFloat(f) => float_to_key(*f),
        // Fixed-width collapsed integers normalize to the exact layer (spec §6.1).
        other => match other.as_i64() {
            Some(v) => Some(ValueKey::Int(v)),
            None => other.as_bigint().map(ValueKey::BigInt),
        },
    }
}

/// Canonical integer key: fitting `i64` → `Int`, else `BigInt` (spec §11.6 value normalization).
fn integer_to_key(i: BigInt) -> Option<ValueKey> {
    match i.to_i64() {
        Some(v) => Some(ValueKey::Int(v)),
        None => Some(ValueKey::BigInt(i)),
    }
}

/// `2^63` as `f64` (exactly representable): integral floats in `[-2^63, 2^63)` convert exactly
/// to `i64`; the bounds exclude `x = 2^63`, whose cast would overflow.
const TWO_POW_63_F64: f64 = 9_223_372_036_854_775_808.0;

fn float_to_key(x: f64) -> Option<ValueKey> {
    if x.is_nan() {
        return None;
    }
    // Integral-valued floats key as the integers they equal (spec §11.6 value normalization):
    // `1.0` shares `d[1]`'s key and `-0.0` shares `d[0]`'s (no negative-zero key).
    if x.is_finite() && x.fract() == 0.0 {
        if (-TWO_POW_63_F64..TWO_POW_63_F64).contains(&x) {
            return Some(ValueKey::Int(x as i64));
        }
        return Some(ValueKey::BigInt(integral_f64_to_bigint(x)));
    }
    Some(ValueKey::Float(x.to_bits()))
}

/// Exact `BigInt` of a finite integral `f64`: f64 integral values are exact binary integers, so
/// reconstruct `(implicit-1 mantissa) * 2^(unbiased exp - 52)` from the bit pattern (used for
/// integral floats beyond the `i64` range, e.g. `1e300`).
fn integral_f64_to_bigint(x: f64) -> BigInt {
    let bits = x.to_bits();
    let neg = bits >> 63 == 1;
    let exp = (((bits >> 52) & 0x7ff) as i64) - 1075; // unbiased exponent minus 52 mantissa bits
    let mantissa = (bits & 0x000f_ffff_ffff_ffff) | 0x0010_0000_0000_0000;
    let m = BigInt::from(mantissa);
    let scaled = if exp >= 0 { m << exp } else { m >> -exp };
    if neg { -scaled } else { scaled }
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
        assert_roundtrip(
            Value::Number(Number::Integer(Box::new(big.clone()))),
            ValueKey::BigInt(big),
        );
    }

    #[test]
    fn rational_key_roundtrip() {
        assert_roundtrip(
            Value::Number(Number::Rational(Box::new(BigRational::new(
                BigInt::from(1),
                BigInt::from(3),
            )))),
            ValueKey::Rational(BigInt::from(1), BigInt::from(3)),
        );
        // The key always holds a reduced fraction with a positive denominator (spec §6.1).
        assert_roundtrip(
            Value::Number(Number::Rational(Box::new(BigRational::new(
                BigInt::from(2),
                BigInt::from(-3),
            )))),
            ValueKey::Rational(BigInt::from(-2), BigInt::from(3)),
        );
    }

    #[test]
    fn float_key_roundtrip() {
        assert_roundtrip(
            Value::Number(Number::from(2.5)),
            ValueKey::Float(2.5f64.to_bits()),
        );
        // F32 promotes to F64 when keyed (spec §6.1 promotion); the key is the F64 bit pattern.
        let v = Value::Number(Number::Real(Real::F32(1.5)));
        assert_eq!(
            ValueKey::from_value(&v),
            Some(ValueKey::Float((1.5f32 as f64).to_bits()))
        );
        assert_eq!(
            ValueKey::Float((1.5f32 as f64).to_bits()).to_value(),
            Value::Number(Number::Real(Real::F64(1.5)))
        );
    }

    #[test]
    fn scalar_key_roundtrip() {
        assert_roundtrip(
            Value::String("hello".to_string()),
            ValueKey::Str("hello".to_string()),
        );
        assert_roundtrip(Value::Char('x'), ValueKey::Char('x'));
        assert_roundtrip(Value::Bool(true), ValueKey::Bool(true));
    }

    #[test]
    fn symbol_and_expr_keys() {
        assert_eq!(
            ValueKey::from_value(&Value::Symbol(42)),
            Some(ValueKey::Symbol(42))
        );
        assert_eq!(ValueKey::Symbol(42).to_value(), Value::Symbol(42));

        let pool = ExprPool::new();
        let id = pool.integer(3);
        assert_eq!(
            ValueKey::from_value(&Value::Expr(id)),
            Some(ValueKey::Expr(id.as_u32()))
        );
        assert_eq!(ValueKey::Expr(id.as_u32()).to_value(), Value::Expr(id));
    }

    #[test]
    fn unsupported_values_are_none() {
        assert_eq!(
            ValueKey::from_value(&Value::Number(Number::complex(1, 2))),
            None
        );
        assert_eq!(
            ValueKey::from_value(&Value::Array(vec![Value::Number(Number::from(1))].into())),
            None
        );
        assert_eq!(
            ValueKey::from_value(&Value::Dict(HashMap::new().into())),
            None
        );
        assert_eq!(
            ValueKey::from_value(&Value::Set(HashSet::new().into())),
            None
        );
        assert_eq!(ValueKey::from_value(&Value::Undefined), None);
        assert_eq!(ValueKey::from_value(&Value::Nil), None);
    }

    #[test]
    fn nan_is_not_a_key() {
        assert_eq!(
            ValueKey::from_value(&Value::Number(Number::Real(Real::F64(f64::NAN)))),
            None
        );
        assert_eq!(
            ValueKey::from_value(&Value::Number(Number::Real(Real::F32(f32::NAN)))),
            None
        );
    }

    #[test]
    fn numeric_keys_are_value_canonicalized() {
        // Integral floats share the keys of the integers they equal (spec §11.6, Python `dict`
        // semantics): `d[1] = x` is retrieved by `d[1.0]` and vice versa.
        assert_eq!(
            ValueKey::from_value(&Value::Number(Number::Real(Real::F64(1.0)))),
            Some(ValueKey::Int(1))
        );
        assert_eq!(
            ValueKey::from_value(&Value::Number(Number::from(1))),
            ValueKey::from_value(&Value::Number(Number::Real(Real::F64(1.0))))
        );
        // `-0.0` and `0.0` are one key (`0`); no negative-zero key exists.
        assert_eq!(
            ValueKey::from_value(&Value::Number(Number::Real(Real::F64(-0.0)))),
            Some(ValueKey::Int(0))
        );
        assert_eq!(
            ValueKey::from_value(&Value::Number(Number::Real(Real::F64(0.0)))),
            ValueKey::from_value(&Value::Number(Number::Real(Real::F64(-0.0))))
        );
        // Integral rationals share integer keys: `4/2` is `2`.
        assert_eq!(
            ValueKey::from_value(&Value::Number(Number::Rational(Box::new(
                BigRational::new(BigInt::from(4), BigInt::from(2))
            )))),
            Some(ValueKey::Int(2))
        );
        // Integral floats beyond `i64` key as the exact `BigInt` they equal (`1e300`).
        let big = ValueKey::from_value(&Value::Number(Number::Real(Real::F64(1e300)))).unwrap();
        let ValueKey::BigInt(b) = big else {
            panic!("1e300 must key as BigInt");
        };
        // The exact integer converts back to the very same float.
        assert_eq!(b.to_f64(), Some(1e300f64));
        // Non-integral floats keep bit-pattern keys.
        assert_eq!(
            ValueKey::from_value(&Value::Number(Number::Real(Real::F64(2.5)))),
            Some(ValueKey::Float(2.5f64.to_bits()))
        );
        // `in`-equality alignment: a Dict keyed at `1` is hit through the `1.0` key and the
        // `4/2` key alike, so index lookups and membership tests agree.
        let mut d = HashMap::new();
        d.insert(ValueKey::from_value(&num(1)).unwrap(), num(7));
        assert!(d.contains_key(&ValueKey::Int(1)));
        assert!(d.contains_key(
            &ValueKey::from_value(&Value::Number(Number::Real(Real::F64(1.0)))).unwrap()
        ));
    }

    #[test]
    fn rational_and_float_keys_stay_distinct_when_not_integral() {
        // Exactness invariant (spec §6.1): a non-integral rational and the float it approximates
        // are different numbers as keys — `3/2` does not collide with `1.5`.
        let half = ValueKey::from_value(&Value::Number(Number::Rational(Box::new(
            BigRational::new(BigInt::from(3), BigInt::from(2)),
        ))))
        .unwrap();
        let float = ValueKey::from_value(&Value::Number(Number::Real(Real::F64(1.5)))).unwrap();
        assert_ne!(half, float);
        // Non-integral rationals keep reduced rational keys.
        assert_eq!(half, ValueKey::Rational(BigInt::from(3), BigInt::from(2)));
    }

    #[test]
    fn to_value_reconstructs_values() {
        assert_eq!(ValueKey::Int(5).to_value(), Value::Number(Number::from(5)));
        assert_eq!(
            ValueKey::BigInt(BigInt::from(1u64 << 40)).to_value(),
            Value::Number(Number::Integer(Box::new(BigInt::from(1u64 << 40))))
        );
        assert_eq!(
            ValueKey::Float(2.5f64.to_bits()).to_value(),
            Value::Number(Number::Real(Real::F64(2.5)))
        );
        assert_eq!(
            ValueKey::Str("abc".to_string()).to_value(),
            Value::String("abc".to_string())
        );
        assert_eq!(ValueKey::Char('z').to_value(), Value::Char('z'));
        assert_eq!(ValueKey::Bool(false).to_value(), Value::Bool(false));
    }

    #[test]
    fn dict_and_set_hold_hashable_keys() {
        let mut m = HashMap::new();
        m.insert(ValueKey::Str("a".into()), Value::Number(Number::from(1)));
        let Value::Dict(d) = Value::Dict(Box::new(m)) else {
            unreachable!()
        };
        assert_eq!(
            d.get(&ValueKey::Str("a".into())),
            Some(&Value::Number(Number::from(1)))
        );

        let mut s = HashSet::new();
        s.insert(ValueKey::Int(1));
        s.insert(ValueKey::Int(1));
        s.insert(ValueKey::Float(2.5f64.to_bits()));
        let Value::Set(s) = Value::Set(Box::new(s)) else {
            unreachable!()
        };
        assert_eq!(s.len(), 2);
        assert!(s.contains(&ValueKey::Int(1)));
        assert!(s.contains(&ValueKey::Float(2.5f64.to_bits())));
    }

    fn num(i: i64) -> Value {
        Value::Number(Number::from(i))
    }

    #[test]
    fn arrayval_clone_is_shared_and_cow_on_write() {
        // Cloning an array value is O(1) buffer sharing (spec §11.3 value semantics via CoW).
        let a = ArrayVal::from_vec(vec![num(1), num(2)]);
        let mut b = a.clone();
        assert_eq!(a, b);
        // The first mutation of a shared handle copies the buffer, leaving the alias untouched.
        b.with_mut(|items| items[0] = num(9));
        assert_eq!(a.get(0), Some(num(1)));
        assert_eq!(b.get(0), Some(num(9)));
        assert_eq!(a.len(), 2);
    }

    #[test]
    fn arrayval_unique_handle_mutates_in_place() {
        let mut a = ArrayVal::from_vec(vec![num(1)]);
        a.with_mut(|items| items.push(num(2)));
        assert_eq!(a.len(), 2);
        assert_eq!(a.get(1), Some(num(2)));
        // A unique handle stays unique across `with_mut` (no copy was needed).
        a.with_mut(|_| {});
        assert_eq!(Arc::strong_count(&a.0), 1);
    }

    #[test]
    fn arrayval_push_pop_helpers_cow() {
        let mut a = ArrayVal::from_vec(vec![num(1)]);
        let b = a.clone();
        a.push(num(2));
        assert_eq!(a.len(), 2);
        assert_eq!(b.len(), 1);
        assert_eq!(a.pop(), Some(num(2)));
        assert_eq!(a.len(), 1);
    }

    #[test]
    fn arrayval_equality_is_structural() {
        let a = ArrayVal::from_vec(vec![num(1), num(2)]);
        let b = ArrayVal::from_vec(vec![num(1), num(2)]);
        assert_eq!(a, b); // different buffers, same content
        let shared = a.clone();
        assert_eq!(a, shared); // same buffer
        let c = ArrayVal::from_vec(vec![num(1), num(3)]);
        assert_ne!(a, c);
        let d = ArrayVal::from_vec(vec![num(1)]);
        assert_ne!(a, d);
        // Nested arrays compare structurally through the shared handles.
        let nested_a = ArrayVal::from_vec(vec![Value::Array(a)]);
        let nested_b = ArrayVal::from_vec(vec![Value::Array(b)]);
        assert_eq!(nested_a, nested_b);
    }

    #[test]
    fn arrayval_iter_and_to_vec() {
        let a = ArrayVal::from_vec(vec![num(1), num(2), num(3)]);
        assert!(!a.is_empty());
        assert_eq!(a.iter().collect::<Vec<_>>(), vec![num(1), num(2), num(3)]);
        assert_eq!(a.to_vec(), vec![num(1), num(2), num(3)]);
        assert!(ArrayVal::new().is_empty());
    }
}
