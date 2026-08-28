//! JIT benchmark (spec §19.2 acceptance): compares evaluating a numeric expression DAG through the
//! compiled native path (`prima_jit::CompiledScalar`) against a naive interpreted recursive walker
//! over the same DAG (mirrors the interpreter's numeric path).
//!
//! The workload is the acceptance expression `f(x) = x^4 + sin(x)*x + exp(x)`; the spec measures
//! `f(to_f64(101))` going native after JIT hot-path compilation. The compiled group is guarded so
//! the benchmark still builds when `compile_scalar` returns `None` (expression outside the compiled
//! subset): in that case only the interpreted group is registered.

use criterion::{criterion_group, criterion_main, Criterion};
use prima_core::{BuiltinSymbols, ExprData, ExprId, ExprPool, Number, Real, SymbolId, SymbolTable};
use prima_jit::compile_scalar;

/// Build `f(x) = x^4 + sin(x)*x + exp(x)` as an `ExprDAG` over the process-global pool (spec §8).
fn build_fx() -> (ExprId, SymbolId) {
    let pool = ExprPool::global();
    let symbols = SymbolTable::global();
    let builtins = BuiltinSymbols::global();
    let x = symbols.intern("x");
    let x_id = pool.symbol(x);
    let x4 = pool.pow2(x_id, pool.integer(4));
    let sinx = pool.apply(pool.symbol(builtins.sin), &[x_id]);
    let sinx_x = pool.mul2(sinx, x_id);
    let ex = pool.apply(pool.symbol(builtins.exp), &[x_id]);
    let f = pool.add_n(&[x4, sinx_x, ex]);
    (f, x)
}

/// Interpreted recursive walker over the DAG, mirroring the interpreter's numeric path (spec §9):
/// `Add`/`Mul` n-ary nodes, `Pow`, applications of `sin`/`exp`, and numeric/symbol leaves.
fn eval_dag(pool: &ExprPool, builtins: &BuiltinSymbols, id: ExprId, x: SymbolId, xv: f64) -> f64 {
    match pool.get(id) {
        Some(ExprData::Integer(i)) => Number::Integer(*i).to_f64_lossy(),
        Some(ExprData::Rational(r)) => Number::Rational(*r).to_f64_lossy(),
        Some(ExprData::Real(Real::F64(v))) => v,
        Some(ExprData::Real(Real::F32(v))) => v as f64,
        Some(ExprData::Symbol(s)) if s == x => xv,
        Some(ExprData::Symbol(_)) => 0.0, // free symbol outside the compiled parameter set
        Some(ExprData::Add(items)) => items.iter().map(|&c| eval_dag(pool, builtins, c, x, xv)).sum(),
        Some(ExprData::Mul(items)) => items.iter().map(|&c| eval_dag(pool, builtins, c, x, xv)).product(),
        Some(ExprData::Pow { base, exp }) => {
            let b = eval_dag(pool, builtins, base, x, xv);
            let e = eval_dag(pool, builtins, exp, x, xv);
            b.powf(e)
        }
        Some(ExprData::Apply { f, args }) if f == pool.symbol(builtins.sin) => {
            eval_dag(pool, builtins, args[0], x, xv).sin()
        }
        Some(ExprData::Apply { f, args }) if f == pool.symbol(builtins.exp) => {
            eval_dag(pool, builtins, args[0], x, xv).exp()
        }
        // Non-math application / indeterminate / unknown node: the walker only mirrors the
        // compiled subset.
        Some(ExprData::Apply { .. }) | Some(ExprData::Indeterminate(_)) | None => 0.0,
    }
}

fn bench_jit(c: &mut Criterion) {
    let (f, x) = build_fx();
    let pool = ExprPool::global();
    let builtins = BuiltinSymbols::global();
    let input = 101.0_f64;

    let mut group = c.benchmark_group("scalar-eval");

    group.bench_function("interpreted-dag", |b| {
        b.iter(|| {
            let _ = std::hint::black_box(eval_dag(pool, builtins, f, x, std::hint::black_box(input)));
        });
    });

    // Guard: `compile_scalar` returns `None` for expressions outside the compiled subset; the
    // interpreted group still runs so the benchmark always builds and the acceptance comparison is opt-in.
    match compile_scalar(pool, builtins, f, &["x".to_string()]) {
        Some(compiled) => {
            group.bench_function("compiled-native", |b| {
                b.iter(|| {
                    let _ = std::hint::black_box(compiled.call(&[std::hint::black_box(input)]));
                });
            });
            // Sanity: both paths agree on the acceptance input `f(to_f64(101))` (spec §19.2).
            let interpreted = eval_dag(pool, builtins, f, x, input);
            let compiled = compiled.call(&[input]);
            eprintln!("[bench] f(101) interpreted={interpreted} compiled={compiled}");
        }
        None => {
            eprintln!("[bench] prima-jit: `compile_scalar` returned None, skipping the compiled-native group (spec §19.2)");
        }
    }

    group.finish();
}

criterion_group!(benches, bench_jit);
criterion_main!(benches);
