use dashmap::DashMap;
use num_bigint::BigInt;
use num_rational::BigRational;
use num_traits::One;
use std::cell::RefCell;
use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::{Mutex, OnceLock, RwLock};

use crate::number::{Number, Real};
use crate::symbol::SymbolId;
use crate::value::IndeterminateForm;

/// Handle to an expression in the symbolic world (spec §8.1). In-process hash-consing depends on
/// creation order, so `ExprId` is **forbidden from cross-process serialization/caching** (ADR §6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ExprId(u32);

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

// Thread-local cache (spec §8.1): hit the local cache first, fall back to the global pool, write back on a hit.
thread_local! {
    static LOCAL_CACHE: RefCell<HashMap<u64, ExprId>> = RefCell::new(HashMap::new());
}

/// Process-wide shared hash-consing pool (spec §8.1/§12.4): maps content hash → `ExprId`.
/// The central store is append-only (the symbolic layer is acyclic and resident), concurrency-safe.
pub struct ExprPool {
    global: DashMap<u64, ExprId>,
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

    /// Intern flow (spec §8.1): content hash → local cache → global pool → append-allocate and write
    /// back to both caches. The same `ExprData` always yields the same `ExprId`.
    pub fn intern(&self, data: ExprData) -> ExprId {
        let key = Self::hash_data(&data);
        let cached = LOCAL_CACHE.with(|c| c.borrow().get(&key).copied());
        if let Some(id) = cached {
            return id;
        }
        if let Some(id) = self.global.get(&key) {
            let id = *id;
            LOCAL_CACHE.with(|c| c.borrow_mut().insert(key, id));
            return id;
        }
        let _guard = self.alloc.lock().unwrap();
        if let Some(id) = self.global.get(&key) {
            let id = *id;
            LOCAL_CACHE.with(|c| c.borrow_mut().insert(key, id));
            return id;
        }
        let mut store = self.store.write().unwrap();
        let id = ExprId(store.len() as u32);
        store.push(data);
        self.global.insert(key, id);
        LOCAL_CACHE.with(|c| c.borrow_mut().insert(key, id));
        id
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
            Number::Integer(i) => self.intern(ExprData::Integer(Box::new(i.clone()))),
            Number::Rational(r) => {
                if *r.denom() == BigInt::one() {
                    self.intern(ExprData::Integer(Box::new(r.numer().clone())))
                } else {
                    self.intern(ExprData::Rational(Box::new(r.clone())))
                }
            }
            Number::Real(r) => self.intern(ExprData::Real(*r)),
            Number::Complex { .. } => panic!("complex numbers cannot be interned as expression nodes yet"),
        }
    }

    pub fn const_number(&self, id: ExprId) -> Option<Number> {
        match self.get(id)? {
            ExprData::Integer(i) => Some(Number::Integer(*i)),
            ExprData::Rational(r) => Some(Number::Rational(*r)),
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
        self.intern(ExprData::Apply { f, args: args.to_vec().into_boxed_slice() })
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
