# Prima vs Python vs Rust — benchmark results

Deterministic, scalar-valued kernels measured in steady state: Prima and Rust run in-process
(warm interpreter/native, so parsing and module loading are excluded); Python times its own
kernel with `perf_counter` so interpreter startup is excluded too. Times are medians of
repeated runs; the `Python ×` and `Rust ×` columns are the multiplier by which the reference
implementation is *faster* than Prima (1.0× = equal, higher = reference wins).

NOTE: the `Prima default` column is the default execution path — the whole-function JIT
(spec §19.2) at `opt_level >= O2`, else the bytecode VM (spec §19.5), else the AST; the
`Prima AST` column is the authoritative AST interpreter (`Evaluator::ast_call_function`).
`Python ×` and `Rust ×` are the default-path multipliers — the acceptance metric.

Regenerate with `cargo bench --bench bench_suite` (see benches/bench_suite.rs).

`default/AST ×` is how much faster the default path is than the AST interpreter.

| workload | n | Prima AST (ns) | Prima default (ns) | Python (ns) | Rust (ns) | default/AST × | Python × | Rust × |
|---|---|---|---|---|---|---|---|---|
| sumsq | 200000 | 124459363 ns | 801209 ns | 17639000 ns | 91674 ns | 155.3× | 0.0× | 8.74× |
| pi | 100000 | 150630086 ns | 599790 ns | 12570000 ns | 111147 ns | 251.1× | 0.0× | 5.40× |
| fib | 30 | 27021 ns | 281 ns | 5000 ns | 54 ns | 96.2× | 0.1× | 5.20× |
| sieve | 5000 | 163739411 ns | 96787 ns | 520000 ns | 4678 ns | 1691.8× | 0.2× | 20.69× |
| dot | 3000 | 154843175 ns | 65956 ns | 983000 ns | 11041 ns | 2347.7× | 0.1× | 5.97× |
| poly | 50000 | 109793568 ns | 243597 ns | 9057000 ns | 100690 ns | 450.7× | 0.0× | 2.42× |
