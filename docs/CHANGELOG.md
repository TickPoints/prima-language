# Changelog

All notable changes to the Prima toolchain are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- (New changes land here during development.)

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
