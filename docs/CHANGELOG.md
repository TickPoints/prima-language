# Changelog

All notable changes to the Prima toolchain are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed

- Revised the language spec and implementation plan to **v2.2** (docs only, no code landed yet). Planning for: removing `format` in favor of Python-style f-strings (plus `'...'`/raw-string literals), stabilizing `///`/`//!` doc comments with `prima doc` coverage of the embedded standard library and method-error diagnostic notes, `@builtin(ON)` layered optimization by optimization level, the `opt_level` (`O0`–`O3`) optimization-level policy, a Python-`str`-referenced builtin method set maintained in `.pra` doc comments, stdlib expansion (`math`/`physics`/`sys`/`plot`/`render`/`mem`), and a host-layer GC replacing reference counting for class instances. The implementation roadmap is split into Phase 6–12 in `docs/IMPLEMENTATION-zh_CN.md` §5.

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
