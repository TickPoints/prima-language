# Prima vs Python vs Rust — benchmark results

Deterministic, scalar-valued kernels measured in steady state: Prima and Rust run in-process
(warm interpreter/native, so parsing and module loading are excluded); Python times its own
kernel with `perf_counter` so interpreter startup is excluded too. Times are medians of
repeated runs; the `Python ×` and `Rust ×` columns are the multiplier by which the reference
implementation is *faster* than Prima (1.0× = equal, higher = reference wins).

NOTE: the bytecode VM (spec §19.5) is now the default execution path (`vm := true`), so both Prima
columns run the same compiled-kernel pipeline; the AST interpreter remains the authoritative
fallback outside the compiled subset. The remaining gap to Python is the boxed-`Value`
stack-machine cost (see the perf notes in docs/CHANGELOG.md); closing it fully needs
register-style specialization.

Regenerate with `cargo bench --bench bench_suite` (see benches/bench_suite.rs).

The `Prima VM` column runs the same kernel through the bytecode VM (spec §19.5);
`VM/AST ×` is how much faster the VM is than the AST interpreter on that kernel.

| workload | n | Prima AST (ns) | Prima VM (ns) | Python (ns) | Rust (ns) | VM/AST × | Python × | Rust × |
|---|---|---|---|---|---|---|---|---|
| sumsq | 200000 | 18761069 ns | 18865414 ns | 17611000 ns | 91679 ns | 1.0× | 1.1× | 204.64× |
| pi | 100000 | 26153675 ns | 26226698 ns | 12257000 ns | 111147 ns | 1.0× | 2.1× | 235.31× |
| fib | 30 | 5046 ns | 5015 ns | 6000 ns | 53 ns | 1.0× | 0.8× | 95.21× |
| sieve | 5000 | 3607325 ns | 3607712 ns | 508000 ns | 7750 ns | 1.0× | 7.1× | 465.46× |
| dot | 3000 | 2236665 ns | 2279630 ns | 836000 ns | 9932 ns | 1.0× | 2.7× | 225.20× |
| poly | 50000 | 23045556 ns | 23012834 ns | 8746000 ns | 100697 ns | 1.0× | 2.6× | 228.86× |
