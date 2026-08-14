# Prima

**Symbol-first scientific computing language** — exact by default, explicit where it counts.

Prima is a language and toolchain for scientific computing that puts the **symbolic layer first**: every expression is exact by default (integers, rationals, and symbolic `Expr` DAGs), and floating point, matrices, and parallelism are opt-in through an explicit strategy system. It blends ideas from Julia (numeric tower and promotion), Mathematica/SymPy (symbol-first), Rust (types, modules, ownership, error handling), and Python (ergonomic base types, comprehensions, `Dict`/`Set`, `print`/`println`).

## Highlights

- **Exact symbolic math by default** — `1/3` stays `\frac{1}{3}`; `simplify(\e^{i\pi} + 1)` → `0`; `sqrt(2)` renders as `\sqrt{2}` and stays symbolic until you collapse it.
- **Numeric tower** — `Integer < Rational < Complex < F64 < Complex<F64>` promotion, arbitrary precision via `num-bigint`, and a one-to-one set of fixed-width collapse types (`i8…u128/isize/usize/f32/f64`) that only appear after explicit `to_*`/`try_*`/`checked_*`/`clamped_*`.
- **Strategy system** (`config {}`) — `fraction`, `domain`, `broadcast`, `overload_policy`, `undefined_handling` and more, scoped global → module → `with config` block.
- **Python-flavored collections** — variable-length heterogeneous `Array`, `Dict`/`Set` with `ValueKey` hashing, comprehensions (`[x^2 for x in v if x > 0]`), negative indices and slice assignment, `in`, and convenience functions (`len`/`enumerate`/`zip`/`sorted`/`sum`/…).
- **Rust-style semantics** — `Result`/`Option` with `?`, `match`/`if let`/`while let`, full patterns and destructuring, classes with `pub`/`pub(mod)` visibility, `;`-terminated statements.
- **Symbolic differentiation** — `derivative`/`partial`/`grad`/`limit` built into core.
- **Parallelism** — `@parallel` broadcasting and `parfor` (rayon) with static side-effect checks.
- **Standard library as typed `@builtin` modules** — each stdlib module is an embedded `.pra` signature file (`linalg`, `stats`, `io`, `plot`, `physics`, `sys`, `time`, `num`) whose `@builtin` declarations bind to Rust implementations; `prima check` validates call sites against those signatures (`E0050`).
- **Toolchain** — `prima run` / `check` / `parse` / `compile --emit-headers` / `repl` / `fmt` / `test` / `doc`, with rustc-style colored diagnostics (`error[E00xx]: --> file:line:col`).

## Quick start

```bash
cargo build --release
cargo run -- release run examples/simple.pra
cargo run -- run examples/linear_algebra.pra   # stdlib via import
```

A taste of the language (`examples/comprehension.pra`):

```prima
let squares = [x^2 for x in range(0, 10)];
println(squares);                         // [0, 1, 4, ..., 81]

let d = { "a": 1, "b": 2 };
println(d["b"]);                          // 2

let s = {1, 2, 3, 2};
println(s ∪ {5, 6});                      // {1, 2, 3, 5, 6}

let f(x) = x^2;
println(f([1, 2, 3]));                    // [1, 4, 9] (broadcast)
println(derivative(x^2 + sin(x), x));     // 2 x + cos(x)
```

## Toolchain

```text
prima run    <file.pra>            interpret a program (file = root module)
prima check  [--deny W####] file   static checks incl. stdlib call-site types
prima parse  <file.pra>            dump the AST
prima compile --emit-headers file  emit a C header for @c_api::extern exports
prima repl                         interactive session (rustyline)
prima fmt    [-w|--check] file     format source
prima test   [dir]                 run every *.pra under dir (default examples/)
prima doc    <file.pra>            list definitions with /// doc comments
```

## Documentation

All docs are provided in both Chinese (authoritative) and English (translation):

| Document | 中文（权威） | English |
|---|---|---|
| Language specification v2.1 | [`SPECIFICATIONS-zh_CN.md`](docs/SPECIFICATIONS-zh_CN.md) | [`SPECIFICATIONS-en_US.md`](docs/SPECIFICATIONS-en_US.md) |
| Implementation plan v2.1 | [`IMPLEMENTATION-zh_CN.md`](docs/IMPLEMENTATION-zh_CN.md) | [`IMPLEMENTATION-en_US.md`](docs/IMPLEMENTATION-en_US.md) |

Where the spec and the implementation plan conflict, the implementation plan wins (its §7 records the ADRs).

## Development

```bash
cargo test --workspace     # full test suite (insta snapshots, assert_cmd CLI tests, proptest)
cargo clippy --workspace --all-targets
INSTA_UPDATE=always cargo test   # refresh insta snapshots
```

Conventional Commits (`feat:` / `fix:` / `test:` / `docs:` / `refactor:` / `chore:`), one topic per commit.

## License

[MIT](./LICENSE)
