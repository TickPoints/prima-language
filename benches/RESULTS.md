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

The `Prima VM` column runs the same kernel through the bytecode VM (spec §19.5);
`VM/AST ×` is how much faster the VM is than the AST interpreter on that kernel.

| workload | n | Prima AST (ns) | Prima VM (ns) | Python (ns) | Rust (ns) | VM/AST × | Python × | Rust × |
|---|---|---|---|---|---|---|---|---|
| sumsq | 200000 | 205053757 ns | 134652475 ns | 17378000 ns | 91723 ns | 1.5× | 11.8× | 2235.58× |
| pi | 100000 | 228598088 ns | 126169498 ns | 12235000 ns | 111144 ns | 1.8× | 18.7× | 2056.77× |
| fib | 30 | 45788 ns | 24401 ns | 4000 ns | 54 ns | 1.9× | 11.4× | 847.93× |
| sieve | 5000 | 2138133073 ns | 2152143601 ns | 514000 ns | 4691 ns | 1.0× | 4159.8× | 455794.73× |
| dot | 3000 | 892532463 ns | 885742769 ns | 832000 ns | 10949 ns | 1.0× | 1072.8× | 81517.26× |
| poly | 50000 | 145225355 ns | 100168679 ns | 8622000 ns | 100707 ns | 1.4× | 16.8× | 1442.06× |
