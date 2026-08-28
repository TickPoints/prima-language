use prima_core::SymbolId;
use prima_core::expr_pool::{ExprData, ExprId, ExprPool};

#[test]
fn interning_deduplicates() {
    let pool = ExprPool::new();
    let a = pool.integer(1);
    let b = pool.integer(1);
    assert_eq!(a, b);
    let s1 = pool.symbol(SymbolId(42));
    let s2 = pool.symbol(SymbolId(42));
    assert_eq!(s1, s2);
    assert_ne!(a, s1);
}

#[test]
fn add_is_normalized() {
    let pool = ExprPool::new();
    let x = pool.symbol(SymbolId(10));
    let y = pool.symbol(SymbolId(5));
    let s1 = pool.add(&[x, y]);
    let s2 = pool.add(&[y, x]);
    assert_eq!(s1, s2);
    assert!(matches!(pool.get(s1), Some(ExprData::Add(_))));
}

#[test]
fn mul_and_pow_roundtrip() {
    let pool = ExprPool::new();
    let two = pool.integer(2);
    let x = pool.symbol(SymbolId(1));
    let p = pool.pow(x, two);
    match pool.get(p) {
        Some(ExprData::Pow { base, exp }) => {
            assert_eq!(base, x);
            assert_eq!(exp, two);
        }
        other => panic!("expected Pow, got {other:?}"),
    }
}

#[test]
fn apply_builds_tree() {
    let pool = ExprPool::new();
    let f = pool.symbol(SymbolId(7));
    let arg = pool.integer(3);
    let app = pool.apply(f, &[arg]);
    assert!(matches!(pool.get(app), Some(ExprData::Apply { .. })));
}

#[test]
fn global_pool_is_shared() {
    let a = ExprPool::global().integer(7);
    let b = ExprPool::global().integer(7);
    assert_eq!(a, b);
}

#[test]
fn expr_id_is_copy() {
    let pool = ExprPool::new();
    let id: ExprId = pool.integer(9);
    let _id2 = id;
    assert_eq!(id, pool.integer(9));
}
