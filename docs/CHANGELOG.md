# Changelog

All notable changes to the Prima toolchain are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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
