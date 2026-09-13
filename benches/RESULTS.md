# Prima vs Python vs Rust — benchmark results

Deterministic, scalar-valued kernels measured in steady state: Prima and Rust run in-process
(warm interpreter/native, so parsing and module loading are excluded); Python times its own
kernel with `perf_counter` so interpreter startup is excluded too. Times are medians of
repeated runs; the `Python ×` and `Rust ×` columns are the multiplier by which the reference
implementation is *faster* than Prima (1.0× = equal, higher = reference wins).

NOTE: the `Prima VM` column is the default execution path (`vm := true`, bytecode VM,
spec §19.5); the `Prima AST` column is the authoritative AST interpreter, forced with
`Evaluator::ast_call_function` so the two paths are measured independently. `Python ×`
is the default-path (VM) multiplier — the acceptance metric.

Regenerate with `cargo bench --bench bench_suite` (see benches/bench_suite.rs).

The `Prima VM` column runs the same kernel through the bytecode VM (spec §19.5);
`VM/AST ×` is how much faster the VM is than the AST interpreter on that kernel.

| workload | n | Prima AST (ns) | Prima VM (ns) | Python (ns) | Rust (ns) | VM/AST × | Python × | Rust × |
|---|---|---|---|---|---|---|---|---|
| sumsq | 200000 | 122378298 ns | 4661749 ns | 17544000 ns | 91766 ns | 26.3× | 0.3× | 1333.59× |
| pi | 100000 | 152223094 ns | 8156173 ns | 12403000 ns | 111151 ns | 18.7× | 0.7× | 1369.52× |
| fib | 30 | 26717 ns | 2111 ns | 4000 ns | 57 ns | 12.7× | 0.5× | 468.72× |
| sieve | 5000 | 185590775 ns | 469996 ns | 509000 ns | 7809 ns | 394.9× | 0.9× | 23766.27× |
| dot | 3000 | 155634684 ns | 670463 ns | 834000 ns | 9926 ns | 232.1× | 0.8× | 15679.50× |
| poly | 50000 | 119497867 ns | 7941221 ns | 8697000 ns | 100596 ns | 15.0× | 0.9× | 1187.90× |
