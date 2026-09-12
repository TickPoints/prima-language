use dashmap::DashMap;
use num_bigint::BigInt;
use num_rational::BigRational;
use num_traits::One;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::{Mutex, OnceLock, RwLock};

use crate::number::{Number, Real};
use crate::symbol::SymbolId;
use crate::value::IndeterminateForm;

/// Handle to an expression in the symbolic world (spec §8.1). In-process hash-consing depends on
/// creation order, so `ExprId` is **forbidden from cross-process serialization/caching** (ADR §6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ExprId(u32);

impl ExprId {
    /// Raw index into the expression store (spec §8.1). Exposed for hashable `ValueKey` construction
    /// (spec §11.6); the index is process-local and must not cross process boundaries (ADR §6).
    pub fn as_u32(self) -> u32 {
        self.0
    }

    /// Reconstruct an `ExprId` from a raw index; the index must originate from the same process pool (spec §8.1).
    pub fn from_u32(u: u32) -> ExprId {
        ExprId(u)
    }
}

/// Node in the symbolic world (spec §8.1). `Add`/`Mul` are stored as canonically ordered n-ary lists (spec §8.4),
/// so equality is `ExprId` equality (O(1)).
#[derive(Debug, Clone, PartialEq, Hash)]
pub enum ExprData {
    Symbol(SymbolId),
    Integer(Box<BigInt>),
    Rational(Box<BigRational>),
    Real(Real),
    Add(Box<[ExprId]>),
    Mul(Box<[ExprId]>),
    Pow { base: ExprId, exp: ExprId },
    Apply { f: ExprId, args: Box<[ExprId]> },
    Indeterminate(IndeterminateForm),
}

/// Process-wide shared hash-consing pool (spec §8.1/§12.4): maps content hash → candidate `ExprId`s.
/// The central store is append-only (the symbolic layer is acyclic and resident), concurrency-safe.
///
/// Each hash bucket holds **every** interned expression with that content hash; a hit is confirmed
/// by an `ExprData` equality comparison against the store. This keeps the hash-consing invariant
/// (equal content ⇒ equal `ExprId`) even under hash collisions — `DefaultHasher` is not
/// cryptographic and is keyed identically in every process, so collisions are constructible.
pub struct ExprPool {
    global: DashMap<u64, Vec<ExprId>>,
    store: RwLock<Vec<ExprData>>,
    alloc: Mutex<()>,
}

impl ExprPool {
    pub fn new() -> ExprPool {
        ExprPool {
            global: DashMap::new(),
            store: RwLock::new(Vec::new()),
            alloc: Mutex::new(()),
        }
    }

    /// Process-wide shared instance (`OnceLock`): the interpreter and the symbolic engine share one pool.
    pub fn global() -> &'static ExprPool {
        static POOL: OnceLock<ExprPool> = OnceLock::new();
        POOL.get_or_init(ExprPool::new)
    }

    fn hash_data(data: &ExprData) -> u64 {
        let mut h = DefaultHasher::new();
        data.hash(&mut h);
        h.finish()
    }

    /// Intern flow (spec §8.1): content hash → bucket scan with equality confirmation →
    /// append-allocate under the allocation lock. The same `ExprData` always yields the same
    /// `ExprId` (hash-consing invariant), even when two distinct contents hash to the same bucket.
    pub fn intern(&self, data: ExprData) -> ExprId {
        let key = Self::hash_data(&data);
        if let Some(id) = self.find(key, &data) {
            return id;
        }
        let _guard = self.alloc.lock().unwrap();
        // Re-check under the allocation lock: a concurrent intern may have appended the same
        // content between the first scan and the lock acquisition.
        if let Some(id) = self.find(key, &data) {
            return id;
        }
        // Lock order is `store` → `global` here and `global` → `store` in `find`, but never the
        // reverse pair while holding: `find` releases both guards before returning, and the
        // `alloc` mutex serializes this section against other interns.
        let mut store = self.store.write().unwrap();
        let id = ExprId(store.len() as u32);
        store.push(data);
        drop(store);
        self.global.entry(key).or_default().push(id);
        id
    }

    /// Scan the hash bucket for an entry whose stored content equals `data`; the bucket holds
    /// every expression interned under this content hash, so an equality-confirmed hit is *the*
    /// canonical `ExprId` for `data` (spec §8.1).
    fn find(&self, key: u64, data: &ExprData) -> Option<ExprId> {
        self.global
            .get(&key)?
            .iter()
            .copied()
            .find(|&id| self.eq_entry(id, data))
    }

    /// Whether the store entry at `id` equals `data` (structural equality; child `ExprId`s are
    /// themselves content-canonical by induction, so `ExprData` equality is content equality).
    fn eq_entry(&self, id: ExprId, data: &ExprData) -> bool {
        self.store
            .read()
            .unwrap()
            .get(id.0 as usize)
            .is_some_and(|d| d == data)
    }

    pub fn get(&self, id: ExprId) -> Option<ExprData> {
        self.store.read().unwrap().get(id.0 as usize).cloned()
    }

    pub fn symbol(&self, id: SymbolId) -> ExprId {
        self.intern(ExprData::Symbol(id))
    }

    pub fn integer(&self, n: i64) -> ExprId {
        self.intern(ExprData::Integer(Box::new(BigInt::from(n))))
    }

    pub fn real(&self, x: f64) -> ExprId {
        self.intern(ExprData::Real(Real::F64(x)))
    }

    pub fn number(&self, n: &Number) -> ExprId {
        match n {
            // `Small` interns to the same integer node as `Integer` (spec §6.1 exact layer).
            Number::Small(v) => self.intern(ExprData::Integer(Box::new(BigInt::from(*v)))),
            Number::Integer(i) => self.intern(ExprData::Integer(i.clone())),
            Number::Rational(r) => {
                if *r.denom() == BigInt::one() {
                    self.intern(ExprData::Integer(Box::new(r.numer().clone())))
                } else {
                    self.intern(ExprData::Rational(r.clone()))
                }
            }
            Number::Real(r) => self.intern(ExprData::Real(*r)),
            Number::Complex { .. } => {
                panic!("complex numbers cannot be interned as expression nodes yet")
            }
            // Fixed-width collapsed layer interns to the exact/`Real` node (spec §6.1).
            Number::I8(v) => self.intern(ExprData::Integer(Box::new(BigInt::from(*v)))),
            Number::I16(v) => self.intern(ExprData::Integer(Box::new(BigInt::from(*v)))),
            Number::I32(v) => self.intern(ExprData::Integer(Box::new(BigInt::from(*v)))),
            Number::I64(v) => self.intern(ExprData::Integer(Box::new(BigInt::from(*v)))),
            Number::I128(v) => self.intern(ExprData::Integer(Box::new(BigInt::from(**v)))),
            Number::U8(v) => self.intern(ExprData::Integer(Box::new(BigInt::from(*v)))),
            Number::U16(v) => self.intern(ExprData::Integer(Box::new(BigInt::from(*v)))),
            Number::U32(v) => self.intern(ExprData::Integer(Box::new(BigInt::from(*v)))),
            Number::U64(v) => self.intern(ExprData::Integer(Box::new(BigInt::from(*v)))),
            Number::U128(v) => self.intern(ExprData::Integer(Box::new(BigInt::from(**v)))),
            Number::Isize(v) => self.intern(ExprData::Integer(Box::new(BigInt::from(*v)))),
            Number::Usize(v) => self.intern(ExprData::Integer(Box::new(BigInt::from(*v)))),
            Number::BigFloat(f) => self.intern(ExprData::Real(Real::F64(*f))),
        }
    }

    pub fn const_number(&self, id: ExprId) -> Option<Number> {
        match self.get(id)? {
            ExprData::Integer(i) => Some(Number::Integer(i)),
            ExprData::Rational(r) => Some(Number::Rational(r)),
            ExprData::Real(r) => Some(Number::Real(r)),
            _ => None,
        }
    }

    fn node_rank(&self, id: ExprId) -> u8 {
        match self.get(id) {
            Some(ExprData::Integer(_) | ExprData::Rational(_) | ExprData::Real(_)) => 0,
            Some(ExprData::Symbol(_)) => 1,
            _ => 2,
        }
    }

    pub fn is_const_zero(&self, id: ExprId) -> bool {
        self.const_number(id).is_some_and(|n| n.is_zero())
    }

    pub fn is_const_one(&self, id: ExprId) -> bool {
        self.const_number(id).is_some_and(|n| n.is_one())
    }

    /// Raw `Add` node (no simplification), stored in canonical order (spec §8.4).
    pub fn add(&self, items: &[ExprId]) -> ExprId {
        let mut v = items.to_vec();
        v.sort_by_key(|&id| (self.node_rank(id), id));
        self.intern(ExprData::Add(v.into_boxed_slice()))
    }

    /// Raw `Mul` node (no simplification), stored in canonical order (spec §8.4).
    pub fn mul(&self, items: &[ExprId]) -> ExprId {
        let mut v = items.to_vec();
        v.sort_by_key(|&id| (self.node_rank(id), id));
        self.intern(ExprData::Mul(v.into_boxed_slice()))
    }

    pub fn pow(&self, base: ExprId, exp: ExprId) -> ExprId {
        self.intern(ExprData::Pow { base, exp })
    }

    pub fn apply(&self, f: ExprId, args: &[ExprId]) -> ExprId {
        self.intern(ExprData::Apply {
            f,
            args: args.to_vec().into_boxed_slice(),
        })
    }

    /// Level 0/1 addition simplification (spec §8.3): `Add` flattening, constant merging, `x+0→x`;
    /// the result is sorted in canonical order (numbers/constants → symbols → composite nodes, spec §8.4).
    pub fn add_n(&self, items: &[ExprId]) -> ExprId {
        let mut flat = Vec::new();
        for &it in items {
            if let Some(ExprData::Add(inner)) = self.get(it) {
                flat.extend_from_slice(&inner);
            } else {
                flat.push(it);
            }
        }
        let mut const_sum: Option<Number> = None;
        let mut rest = Vec::new();
        for &it in &flat {
            if let Some(n) = self.const_number(it) {
                const_sum = Some(match const_sum {
                    Some(acc) => acc + n,
                    None => n,
                });
            } else {
                rest.push(it);
            }
        }
        if let Some(n) = const_sum.filter(|n| !n.is_zero()) {
            rest.push(self.number(&n));
        }
        if rest.is_empty() {
            return self.integer(0);
        }
        if rest.len() == 1 {
            return rest[0];
        }
        rest.sort_by_key(|&id| (self.node_rank(id), id));
        self.intern(ExprData::Add(rest.into_boxed_slice()))
    }

    /// Level 0/1 multiplication simplification (spec §8.3): `Mul` flattening, constant merging, `0*x→0`, `1*x→x`.
    pub fn mul_n(&self, items: &[ExprId]) -> ExprId {
        let mut flat = Vec::new();
        for &it in items {
            if let Some(ExprData::Mul(inner)) = self.get(it) {
                flat.extend_from_slice(&inner);
            } else {
                flat.push(it);
            }
        }
        let mut const_prod: Option<Number> = None;
        let mut rest = Vec::new();
        for &it in &flat {
            if let Some(n) = self.const_number(it) {
                if n.is_zero() {
                    return self.integer(0);
                }
                const_prod = Some(match const_prod {
                    Some(acc) => acc * n,
                    None => n,
                });
            } else {
                rest.push(it);
            }
        }
        if let Some(n) = const_prod.filter(|n| !n.is_one()) {
            rest.push(self.number(&n));
        }
        if rest.is_empty() {
            return self.integer(1);
        }
        if rest.len() == 1 {
            return rest[0];
        }
        rest.sort_by_key(|&id| (self.node_rank(id), id));
        self.intern(ExprData::Mul(rest.into_boxed_slice()))
    }

    pub fn add2(&self, a: ExprId, b: ExprId) -> ExprId {
        self.add_n(&[a, b])
    }

    pub fn mul2(&self, a: ExprId, b: ExprId) -> ExprId {
        self.mul_n(&[a, b])
    }

    /// Level 0/1 power simplification (spec §8.3): `x^0→1`, `x^1→x`, `1^x→1`, plus constant folding at the same level.
    pub fn pow2(&self, base: ExprId, exp: ExprId) -> ExprId {
        if self.is_const_zero(exp) {
            return self.integer(1);
        }
        if self.is_const_one(exp) {
            return base;
        }
        if self.is_const_one(base) {
            return self.integer(1);
        }
        if let (Some(b), Some(e)) = (self.const_number(base), self.const_number(exp))
            && let Some(r) = b.pow(&e)
        {
            return self.number(&r);
        }
        self.pow(base, exp)
    }

    pub fn sub2(&self, a: ExprId, b: ExprId) -> ExprId {
        self.add2(a, self.mul2(self.integer(-1), b))
    }

    pub fn div2(&self, a: ExprId, b: ExprId) -> ExprId {
        self.mul2(a, self.pow2(b, self.integer(-1)))
    }
}

impl Default for ExprPool {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn int_data(v: i64) -> ExprData {
        ExprData::Integer(Box::new(BigInt::from(v)))
    }

    /// Simulate a hash collision by re-keying two distinct interned expressions under one bucket:
    /// interning either content must resolve to its own `ExprId`, never the other's.
    #[test]
    fn intern_distinguishes_hash_collisions() {
        let pool = ExprPool::new();
        let a = pool.intern(int_data(7));
        let b = pool.intern(int_data(9));
        assert_ne!(a, b);
        // Force both candidates into the bucket of `a`'s content hash.
        let key_a = ExprPool::hash_data(&int_data(7));
        pool.global.insert(key_a, vec![a, b]);
        assert_eq!(pool.intern(int_data(7)), a);
        assert_eq!(pool.intern(int_data(9)), b);
    }

    /// A fresh content whose hash bucket already holds an unequal candidate must be appended as a
    /// new entry (not silently replaced by the collision candidate).
    #[test]
    fn intern_appends_new_entry_on_collision() {
        let pool = ExprPool::new();
        let decoy = pool.intern(ExprData::Symbol(SymbolId(0)));
        let target = ExprData::Integer(Box::new(BigInt::from(42)));
        // Plant the decoy under the target content's hash: the naive "hash only" intern would
        // return the decoy here.
        pool.global.insert(ExprPool::hash_data(&target), vec![decoy]);
        let id = pool.intern(target.clone());
        assert_ne!(id, decoy);
        assert_eq!(pool.get(id), Some(target));
        // Both candidates now share the bucket; interning the decoy's content still works.
        assert_eq!(pool.intern(ExprData::Symbol(SymbolId(0))), decoy);
    }

    /// The hash-consing invariant on the real path: repeated interning of equal content yields
    /// one `ExprId`, and structurally different content yields different ids.
    #[test]
    fn intern_is_identity_for_equal_content() {
        let pool = ExprPool::new();
        let x = pool.symbol(SymbolId(3));
        let a = pool.mul2(x, pool.integer(2));
        let b = pool.mul2(pool.integer(2), x);
        assert_eq!(a, b, "canonical ordering makes `x*2` and `2*x` the same node");
        assert_eq!(pool.intern(int_data(5)), pool.intern(int_data(5)));
        assert_ne!(pool.intern(int_data(5)), pool.intern(int_data(6)));
    }
}
