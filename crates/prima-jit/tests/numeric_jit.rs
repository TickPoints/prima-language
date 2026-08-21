//! Integration tests for the numeric JIT pipeline (spec §19.2): `ExprDAG → Bytecode → cranelift
//! IR → native code`. Covers bytecode compilation, DAG lowering, non-compilable inputs, and
//! concurrent calls of a shared compiled scalar.

use std::f64::consts::{E, PI, TAU};
use std::sync::Arc;

use prima_core::expr_pool::ExprData;
use prima_core::symbol::SymbolTable;
use prima_core::{BuiltinSymbols, ExprPool, IndeterminateForm};
use prima_jit::bytecode::{Bytecode, Op};
use prima_jit::compiler::dag_to_bytecode;
use prima_jit::engine::compile_bytecode;
use prima_jit::CompiledScalar;

fn compile(bc: &Bytecode, arity: usize) -> Arc<CompiledScalar> {
    compile_bytecode(bc, arity).expect("bytecode compiles")
}

fn approx(a: f64, b: f64, tol: f64) -> bool {
    (a - b).abs() < tol
}

// ————————————————————— bytecode → native —————————————————————

#[test]
fn square_is_x_mul_x() {
    let bc = Bytecode(vec![Op::Param(0), Op::Param(0), Op::Mul]);
    let f = compile(&bc, 1);
    assert_eq!(f.call(&[3.0]), 9.0);
    assert_eq!(f.call(&[-2.0]), 4.0);
}

#[test]
fn x_times_2_plus_x() {
    // (x*2) + x  —  at x = 5 that is 15.
    let bc = Bytecode(vec![Op::Param(0), Op::Const(2.0), Op::Mul, Op::Param(0), Op::Add]);
    let f = compile(&bc, 1);
    assert_eq!(f.call(&[5.0]), 15.0);
    assert_eq!(f.call(&[0.0]), 0.0);
}

#[test]
fn binary_ops_sub_div_rem() {
    // x - 3
    let f = compile(&Bytecode(vec![Op::Param(0), Op::Const(3.0), Op::Sub]), 1);
    assert_eq!(f.call(&[10.0]), 7.0);
    // x / 2
    let f = compile(&Bytecode(vec![Op::Param(0), Op::Const(2.0), Op::Div]), 1);
    assert_eq!(f.call(&[10.0]), 5.0);
    // x % 3  (lowered to the `pj_rem` trampoline; cranelift has no `frem`)
    let f = compile(&Bytecode(vec![Op::Param(0), Op::Const(3.0), Op::Rem]), 1);
    assert_eq!(f.call(&[10.0]), 1.0);
    assert_eq!(f.call(&[-10.0]), -1.0);
}

#[test]
fn unary_ops() {
    // neg
    let f = compile(&Bytecode(vec![Op::Param(0), Op::Neg]), 1);
    assert_eq!(f.call(&[3.5]), -3.5);
    // exp / ln
    let f = compile(&Bytecode(vec![Op::Param(0), Op::Exp]), 1);
    assert!(approx(f.call(&[1.0]), E, 1e-12));
    let f = compile(&Bytecode(vec![Op::Param(0), Op::Ln]), 1);
    assert!(approx(f.call(&[E]), 1.0, 1e-12));
    // log10
    let f = compile(&Bytecode(vec![Op::Param(0), Op::Log10]), 1);
    assert!(approx(f.call(&[1000.0]), 3.0, 1e-12));
    // sqrt
    let f = compile(&Bytecode(vec![Op::Param(0), Op::Sqrt]), 1);
    assert!(approx(f.call(&[9.0]), 3.0, 1e-12));
    // abs
    let f = compile(&Bytecode(vec![Op::Param(0), Op::Abs]), 1);
    assert_eq!(f.call(&[-3.7]), 3.7);
    // cos / tan
    let f = compile(&Bytecode(vec![Op::Param(0), Op::Cos]), 1);
    assert!(approx(f.call(&[0.0]), 1.0, 1e-12));
    let f = compile(&Bytecode(vec![Op::Param(0), Op::Tan]), 1);
    assert!(approx(f.call(&[0.0]), 0.0, 1e-12));
}

#[test]
fn sin_trampoline() {
    let bc = Bytecode(vec![Op::Param(0), Op::Sin]);
    let f = compile(&bc, 1);
    assert!(approx(f.call(&[0.0]), 0.0, 1e-12));
    assert!(approx(f.call(&[PI / 2.0]), 1.0, 1e-12));
    assert!(approx(f.call(&[PI]), 0.0, 1e-9));
}

#[test]
fn pow_trampoline() {
    let bc = Bytecode(vec![Op::Param(0), Op::Const(2.0), Op::Pow]);
    let f = compile(&bc, 1);
    assert_eq!(f.call(&[4.0]), 16.0);
    assert_eq!(f.call(&[3.0]), 9.0);
    assert_eq!(f.call(&[0.0]), 0.0);
}

#[test]
fn zero_arity_constant_function() {
    let bc = Bytecode(vec![Op::Const(PI), Op::Const(2.0), Op::Mul]);
    let f = compile(&bc, 0);
    assert!(approx(f.call(&[]), 2.0 * PI, 1e-12));
}

#[test]
fn multi_param_function() {
    // (a - b) * c
    let bc = Bytecode(vec![
        Op::Param(0),
        Op::Param(1),
        Op::Sub,
        Op::Param(2),
        Op::Mul,
    ]);
    let f = compile(&bc, 3);
    assert_eq!(f.call(&[10.0, 4.0, 3.0]), 18.0);
}

// ————————————————————— malformed bytecode —————————————————————

#[test]
fn out_of_range_param_is_none() {
    assert!(compile_bytecode(&Bytecode(vec![Op::Param(5)]), 1).is_none());
    assert!(compile_bytecode(&Bytecode(vec![Op::Const(1.0), Op::Param(1)]), 1).is_none());
}

#[test]
fn empty_bytecode_is_none() {
    assert!(compile_bytecode(&Bytecode(vec![]), 0).is_none());
}

#[test]
fn underflow_is_none() {
    assert!(compile_bytecode(&Bytecode(vec![Op::Add]), 0).is_none());
    assert!(compile_bytecode(&Bytecode(vec![Op::Sin]), 0).is_none());
    assert!(compile_bytecode(&Bytecode(vec![Op::Const(1.0), Op::Pow]), 0).is_none());
}

// ————————————————————— DAG → bytecode —————————————————————

#[test]
fn dag_x2_plus_sinx() {
    let pool = ExprPool::global();
    let builtins = BuiltinSymbols::global();
    let x = pool.symbol(SymbolTable::global().intern("x"));
    let pow = pool.pow2(x, pool.integer(2));
    let sin_x = pool.apply(pool.symbol(builtins.sin), &[x]);
    let expr = pool.add2(pow, sin_x);

    let f = prima_jit::compile_scalar(pool, builtins, expr, &["x".into()])
        .expect("x^2 + sin(x) compiles");
    assert_eq!(f.arity, 1);
    let expected = |x: f64| x * x + x.sin();
    for x in [0.0, 1.0, 2.5, -1.0, 10.0] {
        let got = f.call(&[x]);
        assert!(
            approx(got, expected(x), 1e-9),
            "x={x}: got {got}, want {}",
            expected(x)
        );
    }
}

#[test]
fn dag_x2_plus_sinx_exact_bytecode() {
    // A fresh pool keeps interning order deterministic, so the emitted bytecode is exactly
    // Param(0), Const(2), Pow, Param(0), Sin, Add.
    let pool = ExprPool::new();
    let builtins = BuiltinSymbols::global();
    let x = pool.symbol(SymbolTable::global().intern("x"));
    let pow = pool.pow2(x, pool.integer(2));
    let sin_x = pool.apply(pool.symbol(builtins.sin), &[x]);
    let expr = pool.add2(pow, sin_x);

    let bc = dag_to_bytecode(&pool, builtins, expr, &["x".into()]).expect("x^2 + sin(x) lowers");
    assert_eq!(
        bc,
        Bytecode(vec![
            Op::Param(0),
            Op::Const(2.0),
            Op::Pow,
            Op::Param(0),
            Op::Sin,
            Op::Add,
        ])
    );
}

#[test]
fn dag_two_params() {
    let pool = ExprPool::new();
    let builtins = BuiltinSymbols::global();
    let x = pool.symbol(SymbolTable::global().intern("x"));
    let y = pool.symbol(SymbolTable::global().intern("y"));
    let expr = pool.add2(pool.mul2(x, y), pool.integer(1)); // x*y + 1
    let f = prima_jit::compile_scalar(&pool, builtins, expr, &["x".into(), "y".into()])
        .expect("x*y + 1 compiles");
    assert_eq!(f.call(&[3.0, 4.0]), 13.0);
    assert_eq!(f.call(&[0.0, 5.0]), 1.0);
}

#[test]
fn dag_sub2_div2() {
    let pool = ExprPool::new();
    let builtins = BuiltinSymbols::global();
    let x = pool.symbol(SymbolTable::global().intern("x"));
    // x - 1  via `sub2` → Add([x, -1])
    let f = prima_jit::compile_scalar(&pool, builtins, pool.sub2(x, pool.integer(1)), &["x".into()])
        .expect("x - 1 compiles");
    assert_eq!(f.call(&[10.0]), 9.0);
    // x / 2  via `div2` → Mul([1/2, x])
    let f = prima_jit::compile_scalar(&pool, builtins, pool.div2(x, pool.integer(2)), &["x".into()])
        .expect("x / 2 compiles");
    assert_eq!(f.call(&[10.0]), 5.0);
    assert_eq!(f.call(&[-10.0]), -5.0);
}

#[test]
fn dag_builtin_constants() {
    let pool = ExprPool::new();
    let builtins = BuiltinSymbols::global();
    // π + e
    let expr = pool.add2(pool.symbol(builtins.pi), pool.symbol(builtins.e));
    let f = prima_jit::compile_scalar(&pool, builtins, expr, &[])
        .expect("pi + e compiles");
    assert!(approx(f.call(&[]), PI + E, 1e-12));
    // τ
    let f = prima_jit::compile_scalar(&pool, builtins, pool.symbol(builtins.tau), &[])
        .expect("tau compiles");
    assert!(approx(f.call(&[]), TAU, 1e-12));
}

#[test]
fn dag_math_functions() {
    let pool = ExprPool::new();
    let builtins = BuiltinSymbols::global();
    let x = pool.symbol(SymbolTable::global().intern("x"));
    // sqrt(x) with x = 9
    let expr = pool.apply(pool.symbol(builtins.sqrt), &[x]);
    let f = prima_jit::compile_scalar(&pool, builtins, expr, &["x".into()])
        .expect("sqrt");
    assert_eq!(f.call(&[9.0]), 3.0);
    // log/ln both mean natural log: ln(e) = 1, log(e) = 1
    let expr = pool.apply(pool.symbol(builtins.ln), &[x]);
    let f = prima_jit::compile_scalar(&pool, builtins, expr, &["x".into()]).expect("ln");
    assert!(approx(f.call(&[E]), 1.0, 1e-12));
    let expr = pool.apply(pool.symbol(builtins.log), &[x]);
    let f = prima_jit::compile_scalar(&pool, builtins, expr, &["x".into()]).expect("log");
    assert!(approx(f.call(&[E]), 1.0, 1e-12));
    // abs
    let expr = pool.apply(pool.symbol(builtins.abs), &[x]);
    let f = prima_jit::compile_scalar(&pool, builtins, expr, &["x".into()]).expect("abs");
    assert_eq!(f.call(&[-7.25]), 7.25);
}

// ————————————————————— non-compilable DAGs —————————————————————

#[test]
fn unknown_apply_is_none() {
    let pool = ExprPool::new();
    let builtins = BuiltinSymbols::global();
    let x = pool.symbol(SymbolTable::global().intern("x"));
    let foo = pool.symbol(SymbolTable::global().intern("foo"));
    let expr = pool.apply(foo, &[x]);
    assert!(dag_to_bytecode(&pool, builtins, expr, &["x".into()]).is_none());
}

#[test]
fn unknown_free_symbol_is_none() {
    let pool = ExprPool::new();
    let builtins = BuiltinSymbols::global();
    let foo = pool.symbol(SymbolTable::global().intern("foo"));
    assert!(dag_to_bytecode(&pool, builtins, foo, &["x".into()]).is_none());
}

#[test]
fn binary_apply_is_none() {
    let pool = ExprPool::new();
    let builtins = BuiltinSymbols::global();
    let x = pool.symbol(SymbolTable::global().intern("x"));
    let expr = pool.apply(pool.symbol(builtins.sin), &[x, x]);
    assert!(dag_to_bytecode(&pool, builtins, expr, &["x".into()]).is_none());
}

#[test]
fn indeterminate_is_none() {
    let pool = ExprPool::new();
    let builtins = BuiltinSymbols::global();
    let expr = pool.intern(ExprData::Indeterminate(IndeterminateForm::ZeroOverZero));
    assert!(dag_to_bytecode(&pool, builtins, expr, &["x".into()]).is_none());
}

// ————————————————————— concurrency —————————————————————

#[test]
fn concurrent_calls_share_code() {
    let bc = Bytecode(vec![Op::Param(0), Op::Const(2.0), Op::Pow]);
    let f = compile(&bc, 1);
    let f1 = f.clone();
    let f2 = f.clone();
    let thread_a = std::thread::spawn(move || (0..1000).map(|i| f1.call(&[i as f64])).collect::<Vec<_>>());
    let thread_b = std::thread::spawn(move || (0..1000).map(|i| f2.call(&[i as f64])).collect::<Vec<_>>());
    let a = thread_a.join().expect("thread a");
    let b = thread_b.join().expect("thread b");
    for i in 0..1000usize {
        let want = (i as f64) * (i as f64);
        assert_eq!(a[i], want);
        assert_eq!(b[i], want);
    }
}
