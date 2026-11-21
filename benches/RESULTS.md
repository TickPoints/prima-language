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
- Commit: `5cbb1cf-dirty`
- Measured: 2026-11-21T08:10:52Z
- Samples: 9 timed runs after 3 warmup runs per combination

| workload | n | AST (ns) | VM (ns) | default (ns) | Python (ns) | Rust (ns) | AST/default | VM/default | Python × | Rust × |
|---|---|---|---|---|---|---|---|---|---|---|
| sumsq | 200000 | 118301508 ns | 4941376 ns | 888680 ns | 16802000 ns | 91693 ns | 133.1× | 5.6× | 0.05× | 9.7× |
| pi | 100000 | 149734647 ns | 7917331 ns | 502465 ns | 12009000 ns | 111150 ns | 298.0× | 15.8× | 0.04× | 4.5× |
| fib | 30 | 26932 ns | 2087 ns | 289 ns | 4000 ns | 55 ns | 93.2× | 7.2× | 0.07× | 5.3× |
| sieve | 5000 | 143617190 ns | 525483 ns | 110206 ns | 513000 ns | 4720 ns | 1303.2× | 4.8× | 0.2× | 23.3× |
| dot | 3000 | 140567647 ns | 647487 ns | 62869 ns | 793000 ns | 9985 ns | 2235.9× | 10.3× | 0.08× | 6.3× |
| poly | 50000 | 107700432 ns | 7377340 ns | 243340 ns | 8409000 ns | 100639 ns | 442.6× | 30.3× | 0.03× | 2.4× |
| simplify | 2000 | 52394589 ns | 51519465 ns | 52669572 ns | - | - | 1.0× | 1.0× | - | - |
| derivative | 2000 | 28373729 ns | 29041833 ns | 28259648 ns | - | - | 1.0× | 1.0× | - | - |
| sort | 5000 | 838762275 ns | 4455172 ns | 4479733 ns | - | - | 187.2× | 1.0× | - | - |
| dict | 2000 | 227489360 ns | 111342010 ns | 109050472 ns | - | - | 2.1× | 1.0× | - | - |
| strings | 5000 | 46035567 ns | 49679429 ns | 47772330 ns | - | - | 1.0× | 1.0× | - | - |
