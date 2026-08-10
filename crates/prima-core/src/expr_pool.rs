use dashmap::DashMap;
use num_bigint::BigInt;
use num_rational::BigRational;
use std::cell::RefCell;
use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::{Mutex, OnceLock, RwLock};

use crate::value::IndeterminateForm;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ExprId(u32);

#[derive(Debug, Clone, PartialEq, Hash)]
pub enum ExprData {
    Symbol(u32),
    Integer(Box<BigInt>),
    Rational(Box<BigRational>),
    Add(Box<[ExprId]>),
    Mul(Box<[ExprId]>),
    Pow { base: ExprId, exp: ExprId },
    Apply { f: ExprId, args: Box<[ExprId]> },
    Indeterminate(IndeterminateForm),
}

thread_local! {
    static LOCAL_CACHE: RefCell<HashMap<u64, ExprId>> = RefCell::new(HashMap::new());
}

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

    pub fn global() -> &'static ExprPool {
        static POOL: OnceLock<ExprPool> = OnceLock::new();
        POOL.get_or_init(ExprPool::new)
    }

    fn hash_data(data: &ExprData) -> u64 {
        let mut h = DefaultHasher::new();
        data.hash(&mut h);
        h.finish()
    }

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

    pub fn symbol(&self, sym: u32) -> ExprId {
        self.intern(ExprData::Symbol(sym))
    }

    pub fn integer(&self, n: i64) -> ExprId {
        self.intern(ExprData::Integer(Box::new(BigInt::from(n))))
    }

    pub fn add(&self, items: &[ExprId]) -> ExprId {
        let mut v = items.to_vec();
        v.sort_unstable();
        self.intern(ExprData::Add(v.into_boxed_slice()))
    }

    pub fn mul(&self, items: &[ExprId]) -> ExprId {
        let mut v = items.to_vec();
        v.sort_unstable();
        self.intern(ExprData::Mul(v.into_boxed_slice()))
    }

    pub fn pow(&self, base: ExprId, exp: ExprId) -> ExprId {
        self.intern(ExprData::Pow { base, exp })
    }

    pub fn apply(&self, f: ExprId, args: &[ExprId]) -> ExprId {
        self.intern(ExprData::Apply { f, args: args.to_vec().into_boxed_slice() })
    }
}

impl Default for ExprPool {
    fn default() -> Self {
        Self::new()
    }
}
