# Prima vs Python vs Rust — benchmark results

Deterministic, scalar-valued kernels measured in steady state: Prima and Rust run in-process
(warm interpreter/native, so parsing and module loading are excluded); Python times its own
kernel with `perf_counter` so interpreter startup is excluded too. Times are medians of
repeated runs; the `Python ×` and `Rust ×` columns are the multiplier by which the reference
implementation is *faster* than Prima (1.0× = equal, higher = reference wins).

NOTE: this is the AST-interpreter baseline. `vm := true` (bytecode VM, spec §19.5) and the
JIT hot path (spec §19.2) are the mechanisms targeted at closing the gap to Python/Rust;
see the milestone notes and docs/IMPLEMENTATION-zh_CN.md §5 for the tracked deltas.

Regenerate with `cargo bench --bench bench_suite` (see benches/bench_suite.rs).

| workload | n | Prima (ns) | Python (ns) | Rust (ns) | Python × | Rust × |
|---|---|---|---|---|---|---|
| sumsq | 200000 | 204219589 ns | 17182000 ns | 91842 ns | 11.9× | 2223.60× |
| pi | 100000 | 217219645 ns | 12106000 ns | 111147 ns | 17.9× | 1954.35× |
| fib | 30 | 43535 ns | 4000 ns | 60 ns | 10.9× | 725.58× |
| sieve | 5000 | 1808824084 ns | 508000 ns | 7778 ns | 3560.7× | 232556.45× |
| dot | 3000 | 854925454 ns | 831000 ns | 9913 ns | 1028.8× | 86242.86× |
| poly | 50000 | 142997222 ns | 8880000 ns | 100672 ns | 16.1× | 1420.43× |
