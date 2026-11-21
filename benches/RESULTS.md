# Prima execution-path and cross-language baseline

Deterministic, scalar-valued kernels measured in steady state: Prima runs in-process
(warm environment, so parsing and module loading are excluded); Python times its own
kernel with `perf_counter` so interpreter startup is excluded too.

**Columns**: `AST` is the authoritative interpreter, `VM` forces the bytecode VM, and
`default` is the JIT → VM → AST dispatch. `AST/default` and `VM/default` are how many
times faster the default path is than each interpreter (higher = the fast path helps more).
`Python ×` and `Rust ×` are the multiplier by which the reference is *faster* than Prima
(1.0× = equal; below 1 means Prima is faster).

Numeric kernels have Python/Rust references; `-` marks symbolic and collection
workloads that only establish an internal before/after baseline.

**Comparability**: these results are only meaningful on the same machine, toolchain, and
build profile. Existing numbers are historical references, not cross-machine thresholds.

- CPU: Intel(R) Core(TM) i3-8100 CPU @ 3.60GHz
- OS: linux x86_64
- Toolchain: rustc 1.100.0 (0ef1bc386 2026-11-09)
- Build: `release` (cargo bench)
- Commit: `58343cb-dirty`
- Measured: 2026-11-21T09:35:31Z
- Samples: 9 timed runs after 3 warmup runs per combination

| workload | n | AST (ns) | VM (ns) | default (ns) | Python (ns) | Rust (ns) | AST/default | VM/default | Python × | Rust × |
|---|---|---|---|---|---|---|---|---|---|---|
| sumsq | 200000 | 119121771 ns | 5478084 ns | 901768 ns | 16622000 ns | 91630 ns | 132.1× | 6.1× | 0.05× | 9.8× |
| pi | 100000 | 155560639 ns | 7903862 ns | 510802 ns | 11679000 ns | 111147 ns | 304.5× | 15.5× | 0.04× | 4.6× |
| fib | 30 | 28202 ns | 2079 ns | 294 ns | 4000 ns | 56 ns | 95.9× | 7.1× | 0.07× | 5.2× |
| sieve | 5000 | 143312554 ns | 555461 ns | 104639 ns | 504000 ns | 7794 ns | 1369.6× | 5.3× | 0.2× | 13.4× |
| dot | 3000 | 141995074 ns | 704243 ns | 64710 ns | 792000 ns | 11035 ns | 2194.3× | 10.9× | 0.08× | 5.9× |
| poly | 50000 | 112094409 ns | 7297266 ns | 260415 ns | 8458000 ns | 100664 ns | 430.4× | 28.0× | 0.03× | 2.6× |
| simplify | 2000 | 50819682 ns | 52267810 ns | 52709554 ns | - | - | 1.0× | 1.0× | - | - |
| derivative | 2000 | 28310965 ns | 26956967 ns | 28503853 ns | - | - | 1.0× | 0.9× | - | - |
| sort | 5000 | 838339543 ns | 4464200 ns | 4466925 ns | - | - | 187.7× | 1.0× | - | - |
| dict | 2000 | 217822022 ns | 108693024 ns | 108694397 ns | - | - | 2.0× | 1.0× | - | - |
| strings | 5000 | 46004070 ns | 48277303 ns | 49120551 ns | - | - | 0.9× | 1.0× | - | - |
