# Changelog

All notable changes to the Prima toolchain are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Performance

This round adds whole-function native compilation (`opt_level >= O2`), and the cross-language
benchmark now runs **7×–1700× faster than CPython on every kernel** (`benches/RESULTS.md`,
default path). An earlier register channel plus lock-free arrays had already brought the VM to
CPython parity (Python × 0.3–0.9); the JIT supersedes it on pure numeric bodies.

- **Whole-function JIT for the pure numeric subset (spec §19.2).** A typed register IR
  (`prima_jit::ir`) and a cranelift lowering (`prima_jit::func`) compile complete `fn` bodies:
  `i64`/`f64`/`bool` locals, dense local arrays, arithmetic/comparisons, `to_f64`,
  `if`/`while`/`for`/`return`, `local.push`/`len` and indexing. The interpreter lowers a body to
  the IR (`prima-runtime::jit_fn`); only pure numeric bodies lower, everything else stays on the
  VM/AST. Generated code is exact — integer arithmetic is checked and array accesses are
  bounds-checked, and an overflow or out-of-range index sets an error flag so the call is re-run
  on the interpreter (the JIT is pure). Loop back-edges poll host cancellation, so interruption
  behaves like the interpreter. Dense arrays are arena-allocated and freed at the single
  epilogue; compilations are cached per function definition.

- **Pooled VM frame and operand-stack buffers (spec §19.5).** `run_vm` reuses frame/stack buffers
  across calls (and nested calls), avoiding reallocation.

- **VM subset: dict index assignment and slice assignment (spec §11.3/§11.6).** `d[k] = v` and
  `arr[lo..hi] = rhs` now run natively in the VM (dict/set literals compile too), removing AST
  fallbacks for them; dict/set *mutating method* calls still fall back to the AST.

- **Register-form local instructions (spec §19.5).** A new instruction family operates directly
  on local slots (`RegBin`/`RegBinImm`, `RegMulAdd`, `RegNeg`, `RegToF64`, `RegMove`, `RegIndex`,
  `RegIndexPush`, `RegIndexStore`/`Imm`, `RegPush`/`Imm`), plus fused loop forms
  (`BranchLocalCmpSum`, `RegIndexBranchFalse`) and a bulk `RegFill` for the
  `for _ in lo..hi { a.push(const) }` idiom. The compiler emits them when operands are local
  slots or inline immediates; every non-numeric/non-array shape falls back to the existing stack
  path, so observable behavior is unchanged. `Op` is now `Copy`, and the dispatch loop matches
  slot references directly on the `Small`/`F64` fast paths instead of cloning operands.

- **Lock-free copy-on-write arrays (spec §11.3).** `ArrayVal` now wraps `Arc<Vec<Value>>` and
  copies on write through `Arc::make_mut` instead of `Arc<RwLock<Vec<Value>>>`. Array reads are
  plain dereferences (no lock), removing the per-element `RwLock` atomics from every index read
  and array loop; `Send + Sync` (and therefore the `parfor`/`@parallel` paths) is preserved.

- **Fast-hash environments.** `Env`'s value/function namespaces and the VM's function-dispatch
  table use `rustc_hash::FxHashMap` instead of SipHash, cutting the cost of every non-local name
  lookup and call resolution.

### Fixed

- **`to_f64` register fast path honors a user shadow.** The register `to_f64` lowering now
  validates against the process-wide function-definition epoch and calls a user `fn to_f64` when
  one is defined, matching the AST interpreter (regression-tested).

- **`ExprPool::number` no longer panics on complex numbers.** A new `try_number` returns `None`
  for values with no symbolic representation (complex); callers that can receive one report a
  clear error instead of panicking (spec §6.1).

- **CSV parsing is UTF-8-correct (spec §18.1).** `io::csv_parse` decoded fields byte-by-byte as
  Latin-1, corrupting any non-ASCII data; it now parses by Unicode scalar values.

- **VM operand truncation is rejected.** Constant-pool indices, parameter/argument counts, and
  array/tuple element counts beyond the `u16` instruction-field range now reject compilation
  (AST fallback) instead of silently wrapping (spec §19.5).

- **Terminal escape injection.** Program output is filtered for C0 control characters and DEL at
  the `print`/`println` sink, so untrusted strings cannot emit ANSI escapes; newline/tab are
  preserved. The stdlib arbitrary-path I/O trust boundary is now documented.

- **f-string `{:spec}` width counts characters, not bytes.** Multi-byte content and fill
  characters now pad to the correct visual width (spec §18.1).

- **Rounding collapse no longer saturates.** `Number::rounded_digits` and the collapse rounding
  family use checked float→integer conversions and reject out-of-range digit counts instead of
  saturating to `i64::MAX` (spec §9.6).

- **REPL no longer replays the whole session.** Each entry is evaluated once against a persistent
  environment (`Evaluator::eval_value_keep_env`), removing the O(n²) session replay that made
  long sessions lag (spec §20).

- **Dedicated nesting-depth error code.** "Expression nesting is too deep" now reports
  `E0012 expression_nesting_too_deep` (new spec appendix C entry) instead of borrowing the
  generic `E0010`.

## [0.4.0] - 2026-09-12

### Performance

This release is a ground-up pass over the interpreter's hot paths. The cross-language benchmark
(`benches/RESULTS.md`) improves from 11.8×–4159× slower than CPython to 1.07×–7.1×: `sumsq` is at
parity (1.07×) and `fib` is at parity, while `pi`/`poly`/`dot` sit at 2.1×–2.8× and `sieve` at
7.1× (the remaining gap is the boxed-`Value` stack-machine cost measured by `perf`; closing it
fully needs register-style specialization).

- **Bytecode VM promoted to the default execution path (spec §19.5).** `vm := true` is now the
  default; the AST interpreter remains the authoritative fallback outside the compiled subset. The
  dispatch loop was restructured around a cached top frame with the hot instruction set handled
  inline (local load/store, constants, jumps, typed arithmetic, fused loop forms), so numeric loops
  no longer pay a per-instruction delegation to the evaluator.

- **Number/`Value` shrank from 64 to 32/24 bytes.** `Number::Integer`/`Number::Rational` (and the
  fixed-width `I128`/`U128`) now box their payloads, and `Value` boxes `Dict`/`Set`/`Result`
  payloads, halving every clone/push/pop in the interpreter and VM (number semantics, rendering,
  keys, and equality are unchanged — regression-tested).

- **Fused bytecode instructions** (spec §14/§12.2): `SetLocalNc` (bind without stack round-trip),
  `AddImmLocal` (`x += <small literal>` in place), `AddToSlot` (fused `x = x + expr`), and
  `BranchLocalLt`/`BranchLocalLe` (fused `while`/`for` loop tests on two local slots). A `while`
  loop iteration compiles to 3–4 instructions instead of 10–18.

- **Typed arithmetic fast paths in the VM dispatch** (spec §6.1/§6.5): `Small`/`Small` uses checked
  i64 arithmetic, `F64`/`F64` plain IEEE `+ - * / %` (division keeps the `fraction` policy and the
  exact-layer zero-divisor diagnostics), mixed exact/`F64` division promotes directly, and array
  indexing/index-assignment with integer indices run inline with the authoritative diagnostics.

- **Per-call-site callee cache (spec §19.5).** `CallName` sites cache their resolved builtin or
  program-function callee, validated against a process-wide function-definition epoch (bumped by
  every `Env::set_func`), so a user redefinition (e.g. shadowing a core builtin) always
  re-resolves while repeated calls skip the environment walk. `to_f64` with a numeric argument
  converts directly on the cached path.

- **Inlined small-integer representation for `Number` (spec §6.1).** The numeric tower's integer
  layer gains an internal `Small(i64)` variant that is semantically identical to `Integer`
  (`Small(5) == Integer(5)`, same rendering, conversions, and hash keys) but avoids the per-value
  heap allocation of `num_bigint::BigInt`. Integer literals, `as_i64`, and the add/subtract/
  multiply hot paths now run allocation-free with checked i64 arithmetic, falling back to the
  exact BigInt path on overflow (results stay exact).

- **Shared copy-on-write array representation for `Value::Array` (spec §11.3).** `Value::Array`
  now holds a shared handle (`prima_core::ArrayVal`, an `Arc<RwLock<Vec<Value>>>` — `Arc`/`RwLock`
  keep `Value: Send + Sync` for the `parfor`/`@parallel` rayon paths) instead of an owned
  `Vec<Value>`, so cloning an array value is O(1) instead of a full element copy. Mutation goes
  through `ArrayVal::with_mut`, which mutates the buffer in place when the handle is uniquely
  owned and copy-on-writes when shared, preserving value semantics exactly: `let b = a; b[0] = 9`
  (or `b.push(x)`) still leaves `a` unchanged, and storing an array into its own buffer (`a[0] =
  a`) stores a snapshot copy rather than creating a reference cycle. Equality, hashing keys,
  rendering/`print` output, and error messages are unchanged.

- **Quadratic lexing on large inputs (spec §3).** The lexer's `cur`/`peek_char` decoded the
  current character by running `str::from_utf8` over the *entire remaining input* — O(n²) overall
  (lexing 400 000 tokens took ~70 s, a hang-DoS on untrusted source). Decoding is now a single O(1)
  incremental read: 400 000 tokens lex in ~0.18 s (~400× faster at that size).

- **Exact `BigInt` arithmetic-sum closed form (spec §10/§19.1).** The `for i in 0..n { acc += i }`
  closed-form optimization computed `n(n-1)/2` in `i64`; it now computes the product in `BigInt`,
  matching the real loop's exact accumulation at every magnitude.

### Fixed

- **C-ABI panic across the FFI boundary (spec §18.4).** Generated `extern "C"` wrappers called
  `call_file_export` unguarded, so any interpreter panic unwound across the C boundary — aborting
  the host process on Rust ≥ 1.81. `call_file_export` now wraps its body in `catch_unwind`: the
  panic payload is logged to stderr, the per-thread module cache is dropped, and the call surfaces
  as a `RuntimeError` so each wrapper returns its documented default value.

- **Parser/evaluator/checker stack exhaustion on deeply nested source (spec §16.4).** Tens of
  thousands of nested parentheses or a 100 000-term flat expression (`0+0+0+…`, which the iterative
  Pratt loop wraps into a deep chain without recursing) drove recursive consumers into
  stack-overflow SIGSEGV; `prima check` on untrusted source was the hard exposure. The parser now
  enforces an exact per-node AST depth budget (2 000) plus a balanced recursion guard (512) and
  reports `E0010`-coded syntax errors (appendix C has no dedicated "too deep" code), running on a
  dedicated 32 MB thread; the evaluator and static checker gained matching `Drop`-safe depth
  guards. All `examples/` still parse unchanged.

- **JIT parameter-index truncation and slot-offset overflow (spec §19.2).** `Op::Param` carries a
  `u8` index but the compiler cast parameter positions with `idx as u8` (≥ 257 parameters silently
  read the wrong slot), and the parameter-buffer offset was computed as `i32::from(8 * i)` with
  `i: u8` — a debug panic at ≥ 32 parameters and a silently wrong slot in release. Indices beyond
  the instruction set's range are now rejected (interpreter fallback, never a wrong value), the
  offset is computed in `i32`, and `validate_bytecode` enforces the arity limit. 32- and
  33-parameter functions are regression-tested.

- **JIT engine initialization panics.** `JITBuilder::new` and `declare_function` used `unwrap()`;
  an environment where cranelift cannot initialize now permanently marks the JIT unavailable and
  every compilation degrades to the interpreter fallback.

- **Silent lost write in the bytecode VM's array index assignment (spec §19.5).** Under the VM,
  `A[i] = v` evaluated the store against a stack copy of the array and never wrote back. The
  compiler now lowers index assignment to `IndexStoreLocal(slot)`/`IndexStoreName(name)` (in-place
  mutation through the slot or the environment chain) and mutating `Array` methods on local slots
  run through `MethodLocal`, mutating the slot's array directly.

- **Multi-parameter functions misbound in the bytecode VM (spec §11/§19.5).** Function chunks
  bound parameters with per-parameter `SetLocal` instructions, which pop the *last*-pushed
  argument and push it back — with two or more parameters every parameter received the wrong
  argument and the operand stack leaked the arguments. A single `BindParams` instruction now
  distributes the call arguments to the parameter slots in order (single-parameter kernels had
  worked by accident; two-parameter VM/AST parity is regression-tested).

- **Float→integer saturation in `Number::as_bigint`/`as_rational` (spec §9.2).** Converting a
  fractional-free float beyond the `i64` range (e.g. `to_bigint(1e19)`) silently saturated to
  `i64::MAX`. Both conversions now apply the same `i64` round-trip guard used by `as_i64` and
  return `None`; collapse callers report proper errors instead of a silently wrong result.

- **i64 overflow in loop index stepping and the `parfor` iteration count (spec §16.1 R0001).**
  `for … step s`, `range(start, end, step?)`, and `parfor` stepped/materialized with unchecked
  arithmetic (debug panic, or silent wrap → dropped iterations/infinite loop in release). All
  paths now use `checked_add`/`i128` counting and report `RuntimeError::Overflow`.

- **Hash-consing collision unsoundness (spec §8.1).** `ExprPool::intern` deduplicated by 64-bit
  content hash alone, so a hash collision silently replaced one symbolic expression with another
  (`DefaultHasher` is keyed identically in every process, making collisions constructible).
  Interning now keeps per-hash candidate buckets and confirms `ExprData` equality before reusing
  an `ExprId`, preserving the equal-content ⇒ equal-`ExprId` invariant under collisions.

- **`parfor` + JIT-fallback data race (spec §17.2).** A `jit(...)` callable whose compilation
  failed runs through an interpreted fallback that dereferences the registering thread's
  `Rc<RefCell<Env>>`; inside a `parfor` rayon task that is a cross-thread `Rc` race (UB). Worker
  threads are now marked for the duration of a `parfor` task and the JIT fallback refuses to run
  on them with a clear error; single-threaded fallback behavior is unchanged.

- **No interruption path for long-running evaluations (spec §16).** The evaluator now checks a
  process-wide cancellation flag at loop back-edges and statement boundaries (`RuntimeError
  "interrupted"`), and the C ABI exports `prima_cancel`/`prima_cancel_reset` so an embedded host
  can stop a runaway export instead of hanging forever.

- **Process-level leaks.** The JIT registry (spec §19.2) grew without bound for loops like
  `while … { f = jit(x^2); f(1.0); }` — callables are now evicted (oldest-first) once the registry
  reaches its capacity, and `Value::JitFunction` lookups prune dead entries. The generated C-ABI
  wrappers' `CSTR_KEEP` buffer (spec §18.4) accumulated one `CString` per string-returning call;
  it now uses a double buffer that is recycled across calls.

- **Dict/Set key semantics diverged from membership tests (spec §11.6).** `d[1]` and `d[1.0]`
  used different keys while `1.0 in d` (numeric comparison) reported the key present, and
  `0.0`/`-0.0` were distinct keys. Numeric keys are now canonicalized by value (integral floats
  and denominator-1 rationals key as the integers they equal; `-0.0` keys as `0.0`), matching
  membership tests; NaN keys remain rejected.

- **Resource-exhaustion guards (OOM aborts → runtime errors).** `Integer^Integer` exponentiation
  (including constant folding) refuses results beyond a bit-length budget, `range`/`parfor`
  refuse to materialize beyond an element/iteration limit, `String.repeat` refuses results beyond
  a byte limit, and f-string `{:spec}` width/precision are clamped — each reports a runtime error
  instead of aborting the process on allocation failure.

- **`vm_parity` test concurrency race.** `tests/vm_parity.rs` wrote kernels to per-pid temp files
  that parallel test threads could truncate concurrently; each invocation now uses a uniquely
  created `NamedTempFile`.

- **Tail-recursive `fn` bodies skip the bytecode VM (spec §10.2/§19.5).** The compiled subset has
  no constant-stack recursion, so a 100 000-deep tail-recursive function overflowed the stack
  under `vm := true` where the AST trampoline handled it; tail-recursive bodies now stay on the
  AST path (results identical, `tail_call_optimization_avoids_stack_overflow` green).

## [0.3.5] - 2026-09-05

### Added

- **Cross-language benchmark suite (benches/RESULTS.md).** A new `cargo bench --bench bench_suite` (harness-free) measures six deterministic, scalar-valued kernels (integer accumulation, Leibniz π, iterative Fibonacci, Sieve of Eratosthenes, sparse dot-product, Horner polynomial) across three implementations of identical semantics: Prima (in-process, warm `Evaluator::call_function`), CPython (`bench_ref.py`, kernel timed with `perf_counter` to exclude startup), and a native Rust closure. Each run verifies the three results agree (cross-language correctness), then writes a Markdown table to `benches/RESULTS.md`. This is the AST-interpreter baseline; the `vm := true` bytecode VM (spec §19.5) and JIT hot path (spec §19.2) are the mechanisms tracked against it.

- **Doc-test support in `prima doc` (spec §20 / §4.1).** `prima doc --test` extracts ```pra fenced code blocks from `///`/`//!` doc comments, statically checks each with `check_src_checked`, and (`--run`) executes them and compares captured `print`/`println` output to a trailing `// expect: <text>` line — Rust doc-test style. The Markdown renderer now preserves fenced code blocks verbatim plus Markdown list/heading lines, and reports per-block pass/fail with correct exit codes.

- **Conservative name/scope checks in `prima check` (spec §16.2 / appendix C).** A new `check`/`names` pass detects statically-decidable name errors without evaluating: `E0040 undefined_name` (single-segment path/symbol outside scope), `E0080 return_outside_fn`, `E0062 self_outside_method`, and the `W0003 unused_binding` warning, with the pre-imported `core` builtins and primitive type names seeded into the root scope to avoid false positives. `prima check` emits these via a new `check_src_checked` API that also returns warnings, and `--deny W0003` promotes them.

- **`anyhow` error propagation at the CLI (spec §16).** The root `prima-language` crate now depends on `anyhow`: every CLI subcommand (`run`/`parse`/`check`/`compile`/`repl`/`fmt`/`test`/`doc`) returns `anyhow::Result<ExitCode>`, and `read_src` plus the I/O/build steps in `cabi`/`fmt`/`doc` carry a contextual `source` chain (rendered as `caused by:` lines). The library crates keep structured `thiserror` enums (`RuntimeError`/`SyntaxError`/`CoreError`) and the rustc-style diagnostics renderer still owns source-level output, so the numbered error/warning-code surface is unchanged.

- **Bytecode VM execution path (spec §19.5, gated, default off).** A working stack bytecode VM in `prima-runtime` `vm/` (`op` instruction set + `Chunk`/`Program` IR, `comp` AST→bytecode compiler, `exec` dispatch loop, `helpers` value-level utilities). `compile_program`/`compile_function_body` lower a numeric/control-flow/function-call subset (literals, local/name loads, binary/unary ops, `let`/assignment/`if`/`while`/`for`/`return`, array/tuple literals, indexing, calls by name and methods); the executor delegates every value-producing op to the `Evaluator` (`eval_binary`/`eval_compare`/`call_method`/`apply_function`) so VM results equal AST results by construction. `Evaluator::vm_call_function` exposes an explicit VM entry. Unsupported constructs cause a whole-function fallback to the AST interpreter (the authoritative path). On the benchmark kernels the VM is ≈1.5–1.8× faster than the AST interpreter (`benches/RESULTS.md`), with `sieve`/`dot` (mutating array methods) at parity via fallback.

### Changed

- **Memory-strategy docs reconciled to reference counting (spec §12.3/12.4).** The spec and implementation docs no longer plan a host-layer tracing GC: class instances are `Rc<RefCell<ClassInstance>>` (matching the implementation), the `mem::collect()` mutation/GC control is removed, and `mem::Arc` remains a planned explicit-reference-counting wrapper (Phase 12). The ADR, risk table, W_host memory row, and `mem` stdlib rows were updated in both the Chinese (authoritative) and English mirror docs. `docs/AGENTS.md` now records the "spec-first" priority rule (conflict → ask → `SPECIFICATIONS → IMPLEMENTATION → code`).

- **Modularized the interpreter (spec §4.8).** The single 6.5k-line `prima-runtime` `eval.rs` god module is split into a `src/eval/` module directory with one cohesive file per concern: `env` (environment/function values), `helpers` (stateless diagnostic & numeric helpers), `entry` (construction + module system), `stmt` (statement/control-flow), `expr` (expression + numeric ops), `call` (call dispatch/JIT/higher-order), `class` (classes & builtin value-type methods), `apply` (indexing/function application/broadcast/SIMD), `pattern` (match/pattern-routing), and `builtin` (builtins + I/O). `eval.rs` now holds only the module root (type/`Flow`/re-exports). All 607 tests pass unchanged; no behavior, formatting, or public API changed.


## [0.3.0] - 2026-08-29

### Added

- **Builtin method system for every core type (spec §18.1/§11.3/§11.6/§9, Phase 10).** The embedded `core::<class>` `.pra` modules (`String`/`Array`/`Dict`/`Set`/`Number`/`Char`/`Tuple`/`Option`/`Result`) are now the single source of truth for each type's method set and `///` docs, and builtin-value method calls dispatch through their class definitions with the `@builtin(ON)` layering (spec §18.4): the registered Rust fast path runs at `opt_level >= N` and the `.pra` fallback body is the semantic authority. `String` gains the full Python-`str`-inspired set (~50 methods: predicates, case transforms, padding, `count`/`rfind`/`removeprefix`/`removesuffix`/`splitlines`/`expandtabs`/`partition`/…; `split("")` yields the single characters); `Array`/`Dict`/`Set` fill the Python gaps (`copy`, `setdefault`, `popitem`, `symmetric_difference`, `issubset`/`issuperset`/`isdisjoint`, `pop`/`clear`/`update`); `Number` adds predicates/accessors (`is_integer`/`abs`/`sign`/`floor`/`ceil`/`round`/`sqrt`/`numerator`/`denominator`/`real`/`imag`/`bit_length`/`is_*`); `Char`/`Tuple`/`Option`/`Result` get their full small method sets (`is_*`/`code`/`count`/`get`/`is_some`/`is_ok`/`value_or`/…). `prima doc --stdlib` and failed-call diagnostics now cover every core type. Builtin-class method definitions live in the standard library; without `prima_stdlib::init()` the methods are unavailable (the interpreter reports a clear error).

### Fixed

- **Windows io tests.** `crates/prima-stdlib/tests/io.rs` embedded the temp-file path directly into a Prima string literal; on Windows the path's backslashes (e.g. `\U` in `C:\Users\...`) were read as invalid escape sequences and every test failed with `syntax error: invalid escape sequence`. Paths are now escaped with `primed_str` (doubling backslashes, quoting double-quotes, spec §18.1) before interpolation.
- **Windows plot tests.** `crates/prima-stdlib/tests/plot.rs` embedded the temp-file path directly into Prima string literals via `Path::display()`; on Windows the backslashes were consumed as escape sequences, so the three tests that actually write a file failed (`plot_savefig_writes_svg`, `plot_clear_then_plot_again`, `plot_scatter_and_bar_render`) while the two "rejection" tests only passed because a parse error made them fail coincidentally. Paths are now escaped with `primed_str` (doubling backslashes, quoting double-quotes, spec §18.1) before interpolation.
- **Termux/Android install.** `install.sh` defaulted to glibc (`*`-unknown-linux-gnu) on every Linux, which cannot load on Termux (bionic libc). The script now detects Termux via `$TERMUX_VERSION`/`$PREFIX` and selects the static musl target (`*`-unknown-linux-musl) and installs into `$PREFIX/bin`; `PRIMA_LIBC=gnu` still overrides on any Linux, and the target detection respects `PRIMA_LIBC` as before on regular distros.
- **`install.sh` download progress + resilient checksum.** The binary download now shows curl's progress bar when stderr is a terminal (stays quiet under CI or when piped, so stdout/output stays clean). SHA-256 verification no longer hard-fails when no checksum tool is installed: if neither `sha256sum` nor `shasum` is available it prints a warning and continues, while a genuine mismatch still aborts.
- **`install.sh` checksum verification against the release asset name.** The downloader saves the binary as `prima`, but the release `.sha256` file names the expected asset (e.g. `prima-v0.3.0-x86_64-unknown-linux-gnu`), so `sha256sum -c` could not find the file and every install aborted with a spurious mismatch. Verification now extracts the expected hash from the `.sha256` file and compares it against the actual bytes directly, so the check works regardless of the local filename.

## [0.2.4-beta] - 2026-08-28

### Added

- **Quick-install scripts.** `install.sh` (POSIX bash) and `install.ps1` (Windows PowerShell) at the repo root download the latest `prima` release binary for the detected OS/architecture (mapping to the release matrix targets), verify the SHA-256 checksum, and install to `~/.local/bin` (override with `PRIMA_INSTALL_DIR`; `PRIMA_TARGET`/`PRIMA_VERSION` override detection). The README now shows the one-line install commands.
- **CI workflow (`.github/workflows/ci.yml`).** Runs `cargo fmt --all --check` and `cargo clippy --workspace --all-targets -- -D warnings` on ubuntu and `cargo test --workspace` on ubuntu + windows, for every push to `main`/`dev` and every pull request.
- `tests/cli.rs`: `run_all_examples_succeed` now also runs the previously unasserted examples `autodiff`, `builtin_layers`, `capi`, `config_simplify`, `fstring`, `jit`, `mymath`, `opt_levels`.

### Changed

- **Release workflow macOS matrix.** `x86_64-apple-darwin` was pinned to `macos-13`, a GitHub-hosted runner retired in Dec 2025 — a retired `runs-on` label leaves the job queued forever instead of failing. Both macOS targets now run on `macos-15` (arm64): `x86_64-apple-darwin` is cross-compiled on Apple Silicon via `rustup target add` + `cargo build --target` (the Xcode SDK is universal), and `aarch64-apple-darwin` builds natively. The build job gained a `timeout-minutes` safety net and verifies the x86_64 macOS artifact's architecture with `file`.
- **Comment/consistency housekeeping.** Removed stray Chinese text from code comments (`src/diagnostics.rs`, `examples/jit.pra`), fixed a duplicated phrase in `examples/linear_algebra.pra`, dropped stale "prima-jit is a stub" wording in `benches/bench_jit.rs` (the crate is fully implemented), and reworded "deferred to a later phase/stage" doc comments to "later release". `docs/IMPLEMENTATION-*.md` §5 roadmap heading updated to reflect Phases 0–12 scope.
- **Workspace reformatted with `cargo fmt` (rustfmt 1.9).** The whole workspace is now canonical rustfmt output (`cargo fmt --all --check` passes), so formatting is enforced in CI like clippy and the tests.
- README (English + Chinese) rewritten with badges, an install section, and updated document version references (v2.1 → v2.3); the broken `cargo run -- release run` quick-start command was corrected.

## [0.2.3-beta] - 2026-08-28

### Added

- **`opt_level` optimization tiers (spec §10.2/§13.2, Phase 8).** A new `OptLevel` policy (`O0`–`O3`, default `O2`) gates the compiler optimization channels: the arithmetic-series loop closed form now requires `opt_level >= O1`, automatic JIT hot-path compilation and tail-call optimization require `opt_level >= O2`, and tier `O3` enables SIMD vectorization of dense `F64` array elementwise binary ops (`runtime::simd`, via the portable `wide` crate on stable Rust). `simplify_level` was wired into the symbolic simplify pipeline (`simplify_at`), so lowering it reduces only how deeply a symbolic value is canonicalized — never its mathematical value. Results are semantically identical across tiers (equivalence tests; IEEE lane arithmetic is bit-identical to scalar).

### Changed

- **`@builtin` layered optimization (spec §18.4, Phase 9).** `@builtin` may now take a tier, `@builtin(O0)`–`@builtin(O3)`: tier `O0` (bare) stays signature-only and must bind to a registered Rust implementation (`E0055`/`E0056`); tiers `O1`–`O3` carry a `.pra` fallback body (the semantic authority, `E0056` if absent) plus an optional Rust fast path used when `opt_level >= N`, with an invalid tier reported as `E0057`. A new `Function::Layered` variant dispatches between the two implementations at call time. `register_impl` was augmented with `register_impl_level` and a declarative `builtin!` macro (`builtin!("num::fibonacci", fibonacci_impl, O1)`) that replaces the manual string-keyed registration calls across the stdlib crates; the `.pra` `@builtin(ON)` annotation remains the authoritative dispatch tier.

## [0.2.2-alpha] - 2026-08-22
### Added

- **Phase 6: string & formatting rework (spec §3/§18.1, v2.2).** Python-style f-strings land: `f"..."`/`f'...'` with `{expr}` interpolation, `{:spec}` format refinements (float precision, zero-padding, width/alignment), `{{`/`}}` escapes, and raw `rf"..."`/`rf'...'` combined form. New literals: single-quoted strings `'...'` (escape-equivalent to `"..."`; a single character remains a `Char` per the spec BNF) and raw strings `r"..."`/`r'...'` (no escape processing). The lexer tracks `{{`/`}}` and brace/string nesting inside interpolations and rejects nested f-string literals as a compile-time error. Interpolations are rendered with the active `print_format` (default LaTeX).
- **Doc comments (`///`/`//!`) enter the AST (spec §4.1, v2.2).** Doc comments are now language semantics: the lexer emits them, the parser collects consecutive lines into `Program.module_docs`/`Import.docs`/`Stmt.*.docs`/`ClassMember.docs`, and `prima fmt` re-emits them. A `///`/`//!` with no following item warns `W0007 unattached_doc_comment` (spec §16.5; `//!` anywhere but the file top shares the code).
- **Method-call diagnostic notes (spec §16.4, v2.2).** When a method call fails — unknown method, wrong arity, visibility violation, or a runtime error thrown inside the method body — the diagnostic attaches a note with the method's signature, definition location, and `///` doc, plus a `did you mean` suggestion for typos (`String.toupper()` → `to_upper`). `prima check` attaches the same definition note to stdlib `@builtin` call-site errors (`E0050`).
- **`prima doc` Markdown output (spec §20).** Renders `#` module title, `//!` module doc, and one `##` section per definition with its `///` doc and signature; `-o FILE` writes to a file and `--stdlib` documents every embedded stdlib module, giving offline method docs (spec §16.4).
- **Stdlib & native-class doc comments (spec §4.1/§18.1/§18.4).** Every embedded stdlib signature module (`linalg`, `stats`, `io`, `num`, `plot`, `sys::path`, `sys::env`, `sys::os`, `time`) carries a `//!` module doc and a `///` doc per `@builtin` function, and a new embedded `core::string` module documents the native `String` class and its method set. At startup `prima-stdlib` parses `string.pra` and seeds the runtime doc registry (`String` class-level doc plus one `String::<method>` entry per member) with rendered signatures, `///` doc text, and `core/string.pra:<line>:<col>` definition locations, so diagnostics attach a method signature + doc note to failed calls (spec §16.4).

### Changed

- **`format` removed (spec §18.1).** It is no longer a pre-imported builtin; a call to a bare `format(...)` emits the transition warning `W0006` (visible in `prima check` and evaluator warnings; `--deny W0006` promotes it to an error) and then fails as an unknown function. Module functions such as `time::format` are unaffected. All examples/tests were migrated to f-strings (`examples/fstring.pra` is the new reference; `examples/try_catch.pra` now uses f-strings).
- `Literal::Str` renamed to `Literal::String { value, quote, raw }` (AST records the delimiter and raw-ness); `prima fmt` re-emits strings/f-strings losslessly and idempotently.

## [0.2.1-alpha] - 2026-08-22

### Changed

- Removed the two deprecated syntax forms (spec v2.3): the `|>` pipeline operator is now a parse error `E0010` (use class methods/direct calls instead), and newline-separated statements are now a parse error `E0011` (`;` is the sole statement separator; a statement not followed by `;` before end-of-input or a block-closing `}` is rejected). The `W0001`/`W0002` warning codes and the parser's `pending_newline` machinery were deleted; the `ExprKind::Pipeline`/`BinOp::Pipeline` AST variants and the evaluator's pipeline lowering were removed.
- Completed the English translation of the language docs: `docs/SPECIFICATIONS-en_US.md` and `docs/IMPLEMENTATION-en_US.md` no longer contain untranslated Chinese (code comments, diagrams, and labels are now in English; the bilingual Glossary table is retained).

## [0.2.0-alpha] - 2026-08-21

### Added

- Phase 5 JIT (spec §19.2): new `prima-jit` crate compiles numeric scalar MFn bodies (`ExprDAG → bytecode → cranelift IR → native`) with cranelift; the interpreter auto-compiles a numeric MFn after 100 numeric calls (or on the first call with the `@jit` annotation), and runs it natively (`f(to_f64(101))` after a `1..100` warm-up loop goes native).
- `jit(...)` builtin: returns a callable `Value::JitFunction` for an MFn name, a symbolic expression, or `grad(f)` — single-output forward compilation, multi-variable reverse-mode gradient, with an interpreted fallback when compilation is unavailable (spec §19.2 composable optimization).
- Automatic differentiation (spec §19.4 stages 2–3): forward-mode dual numbers (`ad::forward_derivative`) and reverse-mode tape (`ad::Tape`) over numeric scalar DAGs; the tape powers `jit(grad(f))`.
- Optimization pipeline (spec §10.2): `core::opt` constant folding + CSE (hash-consing shares subexpressions); interpreter tail-call optimization for host `fn` bodies ending in a direct `return f(args)` (trampolined, constant stack space).
- C ABI export: `prima compile --emit-c-abi` builds a real shared library (`cdylib` shell crate re-exporting `@c_api::extern` functions via the interpreter) plus its C header (spec §18.4/§19.3).
- `prima_runtime::capi::call_file_export`: thread-cached evaluation of a `.pra` module followed by an exported-function call, powering the C ABI wrappers.
- Criterion benchmark `bench_jit` comparing interpreted vs compiled (JIT) evaluation of `x^4 + sin(x)*x + exp(x)` (≈11× faster native; spec §19.2 acceptance).
- Reference examples `examples/jit.pra` and `examples/autodiff.pra` for hot-path JIT and automatic differentiation.
- Release workflow now also builds a selection of Tier-2-with-host-tools targets: `x86_64-unknown-linux-musl`, `aarch64-unknown-linux-musl`, `riscv64gc-unknown-linux-gnu`, `armv7-unknown-linux-gnueabihf`, `powerpc64le-unknown-linux-gnu`, `s390x-unknown-linux-gnu`.

## [0.1.0] - 2026-08-14

### Added

- Symbol-first scientific computing toolchain: hand-written lexer, recursive-descent/Pratt parser, AST and diagnostics (`prima-syntax`).
- Numeric tower `Integer < Rational < Complex<Rational> < F64 < Complex<F64>` with hash-consing expression pool (`prima-core`).
- Symbolic engine: MFn evaluation, implicit broadcasting, TeX literals, simplification levels 0–3.
- Phase 2 runtime: config policy system (`fraction`/`domain`/`broadcast`/`undefined_handling`), collapse function families, `if`/`while`/`for`/`return` control flow, classes, module system, `prima check` static analysis.
- v2.1 collection types: mutable `Array`, `Dict`/`Set`, comprehensions and convenience functions.
- Standard library as embedded `.pra` signature modules bound to `@builtin` implementations (`linalg`/`stats`/`io`/`physics`/`plot`/`sys`/`time`/`num`).
- CLI subcommands: `run`, `check`, `repl`, `fmt`, `test`, `doc`, `compile --emit-headers`.
- Rustc-style colored diagnostics via codespan-reporting.
- MIT license and a release workflow building all Rust Tier-1 platforms with SHA-256 checksums.

### Changed

- Language spec and implementation plan revised to v2.1.
