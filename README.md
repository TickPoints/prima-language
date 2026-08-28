# Prima

**Symbol-first scientific computing language** — exact by default, explicit where it counts.

![license](https://img.shields.io/badge/license-MIT-blue.svg)
![CI](https://github.com/TickPoints/prima-language/actions/workflows/ci.yml/badge.svg)
![release](https://img.shields.io/github/v/release/TickPoints/prima-language)
![stars](https://img.shields.io/github/stars/TickPoints/prima-language?style=social)
![built with Rust](https://img.shields.io/badge/built%20with-Rust-orange.svg)
![PRs welcome](https://img.shields.io/badge/PRs-welcome-brightgreen.svg)

Prima is a language and toolchain for scientific computing that puts the **symbolic layer first**. Every expression is exact by default — integers, rationals, and symbolic `Expr` DAGs stay symbolic until you explicitly collapse them — while floating point, matrices, and parallelism are opt-in through an explicit strategy system.

It blends the best ideas of its predecessors:

| From | Prima inherits |
|---|---|
| **Julia** | The numeric tower and promotion: `Integer < Rational < Complex < F64 < Complex<F64>` |
| **Mathematica / SymPy** | Symbol-first computation: `1/3` stays `\frac{1}{3}`, `sqrt(2)` stays `\sqrt{2}` |
| **Rust** | Types, modules, ownership, `Result`/`Option` with `?`, classes with visibility |
| **Python** | Ergonomics: heterogeneous `Array`, `Dict`/`Set`, comprehensions, `print`/`println` |

## Why Prima?

- **Exact math that never silently bites.** `1/3 + 1/6 = 1/2`, exactly. No `0.333...` drift, no comparison surprises. `simplify(\e^{i\pi} + 1)` → `0`. Floating point appears only when you ask for it.
- **Explicit over implicit.** No implicit float conversion, no hidden parallelism, no implicit precision loss. Collapse types (`i8…u128/isize/usize/f32/f64`) exist only after an explicit `to_*` / `try_*` / `checked_*` / `clamped_*`. `NaN`/`Inf`/`Undefined` can't sneak into an exact computation.
- **A strategy system, not a preprocessor.** `config {}` — `fraction`, `domain`, `broadcast`, `undefined_handling`, `opt_level` — is scoped global → module → `with config` block, so one file can mix exact and numerical regimes deliberately.
- **Composable optimization.** `derivative`, `partial`, `grad`, `limit` live in the core; a JIT (cranelift) compiles hot numeric functions, and `jit(grad(f))` chains reverse-mode AD with native speed.
- **A real toolchain.** Rustc-style colored diagnostics, static `prima check`, a formatter, a REPL, a test runner, doc generation, and C-ABI export (`--emit-c-abi` / `--emit-headers`).

## Features

- **Exact symbolic math by default** — `1/3` stays `\frac{1}{3}`; `simplify(\e^{i\pi} + 1)` → `0`; `sqrt(2)` renders as `\sqrt{2}` and stays symbolic until you collapse it.
- **Numeric tower** — `Integer < Rational < Complex < F64 < Complex<F64>` promotion, arbitrary precision via `num-bigint`, and a one-to-one set of fixed-width collapse types (`i8…u128/isize/usize/f32/f64`) that only appear after explicit collapse.
- **Strategy system** (`config {}`) — `fraction`, `domain`, `broadcast`, `overload_policy`, `undefined_handling`, `opt_level` (O0–O3) and more, scoped global → module → `with config` block.
- **Python-flavored collections** — variable-length heterogeneous `Array`, `Dict`/`Set` with `ValueKey` hashing, comprehensions (`[x^2 for x in v if x > 0]`), negative indices and slice assignment, `in`, and convenience functions (`len`/`enumerate`/`zip`/`sorted`/`sum`/…).
- **Rust-style semantics** — `Result`/`Option` with `?`, `match`/`if let`/`while let`, full patterns and destructuring, classes with `pub`/`pub(mod)` visibility, `;`-terminated statements.
- **Symbolic differentiation** — `derivative`/`partial`/`grad`/`limit` built into core; forward and reverse-mode AD for compiled code.
- **Parallelism** — `@parallel` broadcasting and `parfor` (rayon) with static side-effect checks.
- **Standard library as typed `@builtin` modules** — each stdlib module is an embedded `.pra` signature file (`linalg`, `stats`, `io`, `plot`, `physics`, `sys`, `time`, `num`) whose `@builtin` declarations bind to Rust implementations; `prima check` validates call sites against those signatures (`E0050`).
- **Toolchain** — `prima run` / `check` / `parse` / `compile` / `repl` / `fmt` / `test` / `doc`, with rustc-style colored diagnostics (`error[E00xx]: --> file:line:col`).

## Install

Grab a prebuilt binary for your platform (Linux, macOS, Windows):

```bash
# Linux / macOS / git-bash / WSL
curl -fsSL https://raw.githubusercontent.com/TickPoints/prima-language/main/install.sh | bash

# Windows (PowerShell)
irm https://raw.githubusercontent.com/TickPoints/prima-language/main/install.ps1 | iex
```

The scripts detect your OS/architecture, download the matching release binary, verify its SHA-256 checksum, and install to `~/.local/bin` (or `~\.local\bin` on Windows) — add that directory to your `PATH` if prompted. Overrides: `PRIMA_VERSION`, `PRIMA_TARGET`, `PRIMA_LIBC` (Linux musl), `PRIMA_INSTALL_DIR`, `--version`, `--target`, `--dir`.

Or build from source:

```bash
cargo install --git https://github.com/TickPoints/prima-language prima-language
```

## Quick start

```bash
cargo build --release
cargo run --release -- run examples/simple.pra     # → 9
cargo run --release -- run examples/linear_algebra.pra   # stdlib via import
```

A taste of the language (`examples/comprehension.pra`):

```prima
let squares = [x^2 for x in range(0, 10)];
println(squares);                    // → [0, 1, 4, 9, ..., 81]

let d = { "a": 1, "b": 2 };
println(d["b"]);                     // → 2

let s = {1, 2, 3, 2};
println(s ∪ {5, 6});                 // → {1, 2, 3, 5, 6}

let f(x) = x^2;
println(f([1, 2, 3]));               // → [1, 4, 9] (broadcast)
println(derivative(x^2 + sin(x), x));// → 2 x + \cos(x)
```

## Toolchain

```text
prima run    <file.pra>            interpret a program (file = root module)
prima check  [--deny W####] file   static checks incl. stdlib call-site types
prima parse  <file.pra>            dump the AST
prima compile --emit-headers file  emit a C header for @c_api::extern exports
prima compile --emit-c-abi file    build a shared library with the C-ABI exports
prima repl                         interactive session (rustyline)
prima fmt    [-w|--check] file     format source
prima test   [dir]                 run every *.pra under dir (default examples/)
prima doc    <file.pra> | --stdlib generate Markdown docs from /// comments
```

## Documentation

All docs are provided in both Chinese (authoritative) and English (translation):

| Document | 中文（权威） | English |
|---|---|---|
| Language specification v2.3 | [`SPECIFICATIONS-zh_CN.md`](docs/SPECIFICATIONS-zh_CN.md) | [`SPECIFICATIONS-en_US.md`](docs/SPECIFICATIONS-en_US.md) |
| Implementation plan v2.3 | [`IMPLEMENTATION-zh_CN.md`](docs/IMPLEMENTATION-zh_CN.md) | [`IMPLEMENTATION-en_US.md`](docs/IMPLEMENTATION-en_US.md) |
| Changelog | — | [`CHANGELOG.md`](docs/CHANGELOG.md) |

Where the spec and the implementation plan conflict, the implementation plan wins (its §7 records the ADRs).

## Development

```bash
cargo test --workspace     # full test suite (insta snapshots, assert_cmd CLI tests, proptest)
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
INSTA_UPDATE=always cargo test   # refresh insta snapshots
```

CI (`.github/workflows/ci.yml`) enforces formatting, clippy (warnings denied), and the full test suite on Linux and Windows for every push and PR.

Conventional Commits (`feat:` / `fix:` / `test:` / `docs:` / `refactor:` / `chore:`), one topic per commit.

## License

[MIT](./LICENSE)
