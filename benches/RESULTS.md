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
| sumsq | 200000 | 118528178 ns | 891777 ns | 17597000 ns | 91669 ns | 132.9× | 0.1× | 9.73× |
| pi | 100000 | 148568462 ns | 500356 ns | 12404000 ns | 111150 ns | 296.9× | 0.0× | 4.50× |
| fib | 30 | 26087 ns | 297 ns | 4000 ns | 51 ns | 87.8× | 0.1× | 5.82× |
| sieve | 5000 | 176589600 ns | 103696 ns | 505000 ns | 7777 ns | 1703.0× | 0.2× | 13.33× |
| dot | 3000 | 156823989 ns | 62887 ns | 836000 ns | 9991 ns | 2493.7× | 0.1× | 6.29× |
| poly | 50000 | 113700076 ns | 243291 ns | 8782000 ns | 100605 ns | 467.3× | 0.0× | 2.42× |
