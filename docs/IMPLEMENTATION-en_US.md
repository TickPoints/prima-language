# Prima Language — Implementation Plan v2.3

> **Translation note**: this is the English counterpart of the authoritative Chinese implementation plan [`IMPLEMENTATION-zh_CN.md`](./IMPLEMENTATION-zh_CN.md) (v2.3); the Chinese original is the final authority.
> **Position**: this document records the implementation decisions that implement [`SPECIFICATIONS-zh_CN.md`](./SPECIFICATIONS-zh_CN.md) v2.3.
> Where the spec does not cover something, this document takes precedence; several **initial suggestions** in spec §19.1 (logos/chumsky/latex crates) were evaluated and **rejected**, with the reasons given in §2 and §7.
> Intended audience: implementers (including AI agents). All subsequent development work proceeds according to the division of labor and ordering in this document.
> **v2.1 increments**: base-type usability enhancements (variable-length `Array`, `Dict`/`Set`, convenience functions, `print`/`println` distinction, `input`, comprehensions) enter the language spec, with implementation scheduling in §5; Phase 3 (`@parallel` broadcast parallelism + `parfor` + symbolic differentiation `derivative`/`partial`/`grad`/`limit`) is implemented in this v2.1 document.
> **v2.2 increments**: f-strings replace `format`, doc-comment stabilization, `@builtin(ON)` layered optimization, `opt_level` optimization levels, the builtin method system (`String` and others), stdlib expansion (`math`/`physics`/`sys`/`plot`/`render`/`mem`), and host-layer GC. Chunked scheduling in §5 (Phase 6–12); **this v2.2 document only revises the spec and implementation docs, with no code landing**.
> **v2.3 increments**: the two deprecated features `|>` pipeline and newline-separated statements are removed and become hard errors — `|>` no longer participates in parsing and is a parse error `E0010` (removed-syntax hint, same style as `try/catch`), with class method chaining / direct calls as the replacement; statement separation converges on `;` as the sole separator, and newline separation reports `E0011 expected_separator`; the `W0001`/`W0002` warning codes and the parser's `pending_newline` machinery are deleted.

---

## 1. Technology Choices Overview

Domain | Choice | Version | Alternatives | Decision notes |
------|------|------|------|---------|
  Lexing | **Handwritten** | — | logos 0.16 | §2.1 |
  Parsing | **Handwritten recursive descent + Pratt climbing** | — | chumsky 1.0-alpha / lalrpop 0.23 / pest 2.8 | §2.2, full coverage of the appendix A BNF |
  Arbitrary-precision integers | `num-bigint` | 0.5 | `rug` 1.30 (GMP, LGPL, feature-gated) | fixed by spec §21-30; pure Rust, MIT/Apache-2.0 |
  Arbitrary-precision rationals | `num-rational` | 0.4 | same as above | fraction-first preference (`fraction := true`) |
  Complex numbers | `num-complex` | 0.4 | same as above | §6.4 promotion rules implemented by hand at the `Number` layer |
  Generic numeric trait | `num-traits` | 0.2 | — | generic algorithms (absolute value, power, inverse) |
  BigFloat (optional) | `num-bigfloat` | 1.7 | `rug` | required by `to_bigfloat`; pure-Rust fallback, rug as a speed-up feature |
  Concurrent interner | `dashmap` | 6.x (7 still in RC, not tracked yet) | — | §8.1 `global: DashMap<u64, ExprId>` |
  Parallelism | `rayon` | 1.12 | — | §17.2 `parfor`, `@parallel` broadcast, named in the spec |
  Matrices / linear algebra | `nalgebra` | 0.35 | `faer` 0.24 | §12.4 names it; MVP uses nalgebra, faer kept as a performance replacement (§6) |
  CLI | `clap` (derive) | 4.6 | — | §20 subcommands run/compile/repl/fmt/check/test/doc |
  Error types | `thiserror` | 2.0 | — | §16.1 `Error` enum derived directly |
  Diagnostic rendering | `codespan-reporting` | 0.13 | `miette` | §16.4 is rustc-style (`error[E00xx]: ...` + `--> file:line:col` + carets) |
  REPL | `rustyline` | 18 | `reedline` 0.49 | `prima repl` |
  Unicode identifiers | `unicode-ident` | 1.0 | — | §three: identifiers may contain Unicode such as Greek letters |
  Lazy globals | std `OnceLock` / `thread_local!` | — | `once_cell` | covered by the standard library, no extra dependency |
  LaTeX / Unicode / ASCII rendering | **Handwritten renderer** | — | `latex` 0.3 (a document typesetting library, not applicable) | §7 built-in symbols are independent of TeX; rendering is tightly coupled to the ExprDAG, §2.3 |
  Testing | cargo test + `insta` 1.48 (snapshots) + `assert_cmd` 2.2 (CLI) + `proptest` (parser property tests) | — | — | per-phase acceptance in §5 |
  Benchmarking | `criterion` | 0.8 | — | simplification engine, JIT trigger-threshold tuning |
  Plotting (stdlib phase) | `plotly` | 0.14 | hand-drawn SVG | §eighteen plot module, starting with SVG |
  Formula-image rendering (v2.2 `render` module) | Handwritten SVG + `resvg`/`tiny-skia` | latest stable | — | §eighteen render: ExprDAG → SVG → (optional) PNG rasterization |
  Terminal formula rendering (v2.2 feature) | Reuses the handwritten Unicode/ASCII renderer | — | — | the `term-render` cargo feature enables `print`'s terminal formula rendering (§eighteen render) |
  Doc generation (v2.2 `prima doc`) | Handwritten Markdown rendering (parsing `///`/`//!`) | — | `comrak` | Outputs Markdown; no codegen step (§2.2 principle) |
  Host-layer GC (v2.2) | **Handwritten mark-sweep/generational** | — | `gc-arena` / `shifgrethor` | single-threaded GC integrated with the evaluator; determinism first (§4.12) |
  JIT (Phase 5) | `cranelift-codegen` | latest stable | `inkwell` (LLVM) | §2.4 / §5 |

**Toolchain constraints**: stay on **stable Rust, edition 2024** (already set in Cargo.toml), no nightly dependency; no parser framework that generates code via build.rs (no codegen step).

---

## 2. Parser Selection Rationale (the most important decision in this document)

### 2.1 Lexing: handwritten, not logos

Prima's token set is roughly 40 kinds in v2.0, but with unusual shapes:

- TeX symbol literals `\pi`, `\speed_of_light` (§7), which are easy to confuse with backslash escapes;
- `tex"..."` literals (§three) contain arbitrary TeX text and cannot be tokenized as ordinary strings;
- **v2.2 string family**: ordinary `"..."`/`'...'` (equivalent escapes), raw `r"..."`/`r'...'` (no escaping), **f-strings** `f"..."`/`f'...'` (with `{}` interpolation, `{:spec}`, `{{`/`}}` escapes), `rf"..."` combinations — the lexer must dispatch by the `f`/`r`/`rf` prefix + delimiter, and f-string interpolation bodies are sub-scanned as expressions (balanced `}`) when splitting;
- `///`/`//!` **doc comments** (v2.2) must preserve the raw text and span (feeding the AST/`prima doc`/diagnostic notes), collected separately from ordinary comments;
- `@` (matrix multiplication), `@.` (broadcast operator, §11.4), `@parallel`/`@builtin`/`@builtin(O1)`/`@c_api::extern` annotations (`@`-prefixed, `::` participates in paths, `@builtin` may take a parenthesized optimization-level argument);
- `..` (ranges and slices), `..=` (inclusive ranges, used in patterns), `^`/`**` aliases, `?` (try operator), `|>` (kept as of v2.3 only to report the removed-syntax error `E0010`);
- reserved keywords (async/yield/macro/trait) must be kept as tokens for future use; `impl` takes effect in `ops` modules.

A handwritten lexer of roughly 400–500 lines can give **precise token-level errors and spans** for every item above (e.g. locating unterminated string/TeX literals), and naturally produces a `Token { kind, span }` stream. logos's derive macro offers weaker control over custom literals and error recovery, and the benefit (speed) is meaningless at the current scale.

**v2.0 new tokens**: `;` (statement separator, spec), `?` (try), `..=` (inclusive range), keywords `class`/`self`/`Self`/`impl`/`match`, annotation prefix `@`. **As of v2.3**: the newline token is kept only to detect the removed newline-separated statement form and report `E0011` (`expected_separator`); `;` is the sole legal statement separator (spec §4.2).

### 2.2 Grammar: handwritten recursive descent + Pratt precedence climbing

**Conclusion: handwritten Parser**, for the following reasons:

1. **Precise diagnostics are a hard requirement**. §16.4 requires "code number + file:line:col + relevant expression + hint", and compile-time errors must be **collected** rather than fail-fast. A handwritten parser has full control over spans and error sync points (`;`, `}`, end of file); chumsky's recovery machinery and custom diagnostic formats are costly to integrate.
2. **The grammar has context-sensitive constructs**, which table-driven grammars (lalrpop/pest) handle awkwardly:
   - `let f(x) = expr` (mathematical definition §4.3) vs `let x = v` (variable binding) vs `let (a, b) = v` (pattern destructuring) — needs lookahead after `let` to distinguish;
   - `config {}` / `import` must appear at the top of the file (three-section ordering), violations are errors;
   - trailing annotations: `let f(x) @parallel = x^2` (§17.1); `::` inside `@c_api::extern`;
   - ambiguity between patterns (§4.4) and expressions (`Some(x)` — constructor call or pattern? parsed as a pattern in pattern context);
   - `with config { ... } { ... }` (§13.3 local policies).
   These are just a few branches in recursive descent, but in LR/PEG they require heavy disambiguation and semantic predicates.
3. **Incremental evolution**: §22 reserves macro/async/trait syntax; adding rules and error recovery to a handwritten parser is a localized change, whereas rewriting combinator/grammar files is costly. rustc, Zig, and Gleam all use handwritten recursive descent, the mainstream approach for language implementations.
4. **No codegen step**: AST types are code — no build.rs, no procedural macros, which helps debugging and AI maintenance.

**Expression parsing**: Pratt (precedence climbing). Precedence table (low → high):

| Level | Operator | Associativity | Notes |
|------|------|--------|------|
| 1 | `\|\|` | left | |
| 2 | `&&` | left | |
| 3 | `==` `!=` `<` `<=` `>` `>=` | left | |
| 4 | `+` `-` | left | |
| 5 | `*` `/` `%` `@` `@.` | left | `@`=matrix multiplication, `@.`=broadcast (§11.4) |
| 6 | unary `-` `!` `+` | right | |
| 7 | `^` `**` | right | power binds tighter than unary minus (mathematical convention: `-x^2 = -(x^2)`, same as Julia) |
| 8 | postfix: call `()`, index `[]` (incl. slices `..`), path `::`, method `.name()`, field `.name`, `?` | — | `?` binds at level 8, taking priority over binary operators |

The `|>` pipeline (§9.7) is removed in v2.3 and no longer participates in precedence (its occurrence is a parse error `E0010`).

`^` and `**` are normalized to the same BinOp node at the parsing layer (aliases, §three).

**Parser error strategy**: panic-mode + sync token set (`;`, `}`, `)`, end of file), collecting all syntax errors in one compilation.

**Statement separation strategy** (§4.2):

- During parsing, `;` terminates a statement and is the sole legal statement separator.
- A non-block statement not terminated by `;` (and not at end of input / before `}`) is now always the error `E0011` (`expected_separator`). Since v2.3 removed newline separation, the parser no longer has a `pending_newline` newline-compatibility warning mechanism.
- A trailing `;` after block-level statements (the `{}` following `if`/`while`/`for`/`fn`/`class`/`match`/`with config`) may be omitted without triggering an error.

### 2.3 Rendering: handwritten, not the `latex` crate

The crates.io `latex` crate is a typesetting library for "programmatically generating LaTeX documents/reports", unrelated to this language's symbol rendering (ExprDAG → LaTeX/Unicode/ASCII strings). The renderer is tightly coupled to the ExprDAG structure (§8.4 canonical forms, §7 built-in symbols' TeX names, `print_format` policy switching), so all three backends must be handwritten against the same `Renderer` trait (§4.9). That piece is roughly 300–500 lines and is one of the MVP gates (§19.1 milestone 1).

### 2.4 JIT selection (Phase 5, direction fixed in advance)

`cranelift-codegen` is preferred over `inkwell`: pure Rust (no system LLVM dependency), fast compilation, steadily evolving API; and it is compatible with "query-based incremental compilation" (§19.3). LLVM (inkwell) will only be re-evaluated if AOT needs deep optimization and linking against BLAS static libraries.

---

## 3. Workspace and Crate Layout

This repository is the **Prima toolchain itself** (the user-project directory layout is described in spec §20 and is unrelated to this repository). It uses a Cargo workspace with a strictly one-way dependency direction:

```text
prima-language/                        # workspace root = root package (CLI binary, bin name prima)
├── Cargo.toml                         # [package] + [workspace] member declarations
├── src/main.rs                        # prima binary (clap subcommands) + re-export via src/lib.rs
├── crates/
│   ├── prima-syntax/                  # lexing, parsing, AST, Span/SourceMap, syntax diagnostics
│   ├── prima-core/                    # Number tower, Value, ExprPool/ExprId, simplification engine, Symbol table, renderers
│   ├── prima-jit/                     # cranelift JIT: numeric scalar ExprDAG → bytecode → native code (Phase 5)
│   ├── prima-runtime/                 # interpreter, module system, policy system (Config), built-in functions, type checking, symbolic differentiation, parfor, AD, jit registry
│   └── prima-stdlib/                  # linalg (nalgebra bridge)/stats/io/physics/plot etc., explicitly imported modules
├── tests/                             # integration tests (root): lexer/parser snapshots, core, CLI, proptest
├── benches/                           # criterion benchmarks (simplification, ExprPool, numeric layer, JIT)
└── examples/                          # .pra samples (used by CLI integration tests)
```

Dependencies: `syntax → core → runtime → stdlib`, reverse is forbidden; the CLI lives in the root package and depends on all crates. Rationale: isomorphic to rustc; compile isolation between crates (especially important after the JIT arrives); `prima-syntax` can be reused independently by fmt/check; the root package carrying the CLI makes `cargo run`/`cargo test` work directly at the repository root, and tests uniformly live in the root `tests/`.
> **Phase 5 finalization**: the dependency order extends to `syntax → core → prima-jit → runtime → stdlib` (`prima-jit` depends only on core/syntax; the runtime triggers compilation and holds `CompiledScalar`).

> Note: the originally planned `crates/prima-cli` was changed to be carried by the root package (landed adjustment, 2026-08); the placeholder Hello world in `src/main.rs` was replaced with the clap CLI.

---

## 4. Finalized Core Data Structures (spec → concrete Rust types)

### 4.1 Span and Diagnostic Infrastructure (prima-syntax)

```rust
pub struct Span { pub start: u32, pub end: u32 }        // byte range (compact u32)
pub struct SourceLocation { pub file: Arc<PathBuf>, pub line: usize, pub column: usize }
// SourceMap: file → source + LineIndex (byte offset ↔ line:col, via codespan-reporting or hand-implemented)
// Diagnostic pipeline: DiagnosticCollector — collecting at compile time (reports all errors at once)
```

§16.4's `error[E00xx]: ...` / `warning[W00xx]: ...` format is rendered by `codespan-reporting`; error codes (§16.4/appendix C) prefix the diagnostic titles (`E`/`R`/`W` + four-digit number). The `Error` enum (§16.1) is derived with `thiserror`, and its `location` field is filled automatically from the current execution frame when the interpreter raises an error.

**Warning collection**: `DiagnosticCollector` collects both errors and warnings; warnings do not block compilation, and `prima check --deny W0005` can escalate a given warning to an error (tooling layer). The `W0001` (newline separation) and `W0002` (`|>` pipeline) codes were removed in v2.3 along with the corresponding deprecated features.

### 4.2 AST (prima-syntax)

**A single AST covers the entire grammar**; the three-section ordering (config → import → statement) is validated at parse time. **New in v2.2**: doc comments, f-strings, `@builtin(ON)`:

```rust
// v2.2: doc comments (§4.1) — raw text + source location, retained with the item
pub struct DocComment { pub lines: Vec<(String, Span)>, pub span: Span }

pub struct Program {
    pub config: Option<ConfigBlock>,
    pub module_docs: Option<DocComment>,     // //! module docs (§4.1)
    pub imports: Vec<Import>,                // Import { docs: Option<DocComment>, ... }
    pub stmts: Vec<Stmt>,
}

pub enum Stmt {
    Let { pat: Pattern, type_ann: Option<Type>, value: Expr, annotation: Option<Annotation>, docs: Option<DocComment> }, // let (a, b) = ...
    Const { name: Ident, type_ann: Type, value: Expr, docs: Option<DocComment> },
    FnDef { name: Ident, params: Vec<Param>, ret: Option<Type>, annotation: Option<Annotation>, body: Block, docs: Option<DocComment> },
    MathDef { name: Ident, params: Vec<Param>, ret: Option<Type>, annotation: Option<Annotation>, body: Expr, docs: Option<DocComment> },
    ClassDef { name: Ident, vis: Visibility, members: Vec<ClassMember>, docs: Option<DocComment> },
    Impl { op: ImplOp, target: Ident, members: Vec<FnDef> },            // ops::Add for T (§18.5)
    Expr(Expr),
    For { var: Ident, range: (Expr, Expr), step: Option<Expr>, body: Block },
    ParFor { /* same as For */ },
    While { cond: Expr, body: Block },
    If { cond: Expr, then: Block, elifs: Vec<(Expr, Block)>, else_: Option<Block> },
    IfLet { pat: Pattern, value: Expr, then: Block, else_: Option<Block> },   // if let (§4.4)
    WhileLet { pat: Pattern, value: Expr, body: Block },                      // while let (§4.4)
    Match { scrutinee: Expr, arms: Vec<MatchArm> },                           // match (expression; statement form is the same)
    Return(Option<Expr>),
    WithConfig { entries: Vec<ConfigEntry>, body: Block },
    Pub(Box<Stmt>),
}

pub enum Visibility { Private, Module, Public }   // none / pub(mod) / pub (§15.2)

pub struct ClassMember {                          // §4.5
    pub vis: Visibility,
    pub kind: ClassMemberKind,
}
pub enum ClassMemberKind {
    Field { name: Ident, ty: Type, docs: Option<DocComment> },  // field
    Method { name: Ident, params: Vec<Param>, ret: Option<Type>, body: Block, docs: Option<DocComment> }, // method
    // Associated functions and ordinary methods share one form; a leading self parameter makes it a method
}

pub enum Param {
    Normal { name: Ident, ty: Option<Type> },
    Self_ { ty: Option<Type> },                   // self parameter (method)
    MutSelf { ty: Option<Type> },                 // mut self (reserved extension)
}

pub enum Pattern {                                // §4.4 all patterns
    Wildcard,                                     // _
    Binding { name: Ident },                      // x
    Literal(Literal),                             // 0 / "s" / true / \pi
    Tuple(Vec<Pattern>, bool /* .. tail */),        // (a, b, ..)
    Array(Vec<Pattern>, bool /* .. */),           // [x, ..]
    Struct { name: Ident, fields: Vec<FieldPattern>, rest: bool }, // Point { x, y: 0, .. }
    Variant { name: Ident, inner: Option<Box<Pattern>> },          // Some(x) / Ok(v) / None
    Range { lo: Literal, hi: Literal, inclusive: bool },           // 0..9 / 1..=5
    Or(Vec<Pattern>),                             // pat1 | pat2
    Group(Box<Pattern>),
}
pub struct FieldPattern { pub name: Ident, pub pat: Option<Pattern> }

pub struct MatchArm { pub pat: Pattern, pub guard: Option<Expr>, pub body: Expr }

// v2.2: @builtin may take an optimization-level argument; @builtin(O0) == @builtin
pub enum Annotation {
    Parallel, Jit, Gpu,
    Builtin { opt_level: u8 },                    // O0..=O3, §10.2/18.4
    CApiExtern,
}

pub enum Expr {
    Literal(Literal),            // Integer/Float/Hex/Bin/String/Char/Bool/TexString
    // v2.2: f-string `f"..."` (§18.1) — template literal segments and interpolation expressions alternate
    FString { parts: Vec<FStringPart> },
    Symbol(Ident),               // TeX names in \pi form
    Self_,
    SelfType,                    // Self (in type position: method returns / fields)
    Path(Vec<Ident>),            // a::b::c (module path / qualified access)
    Call { f: Box<Expr>, args: Vec<Expr> },
    MethodCall { receiver: Box<Expr>, name: Ident, args: Vec<Expr> }, // obj.method(...) (§4.5)
    Index { base: Box<Expr>, index: Index },          // Index::Elem / Index::Slice(RangeExpr)
    Field { base: Box<Expr>, name: Ident },           // obj.field (class field access)
    StructLiteral { name: Ident, fields: Vec<FieldValue>, base: Option<Box<Expr>> }, // T { a, b } / T { ..base }
    Binary { op: BinOp, lhs: Box<Expr>, rhs: Box<Expr> },  // op carries precedence, power is right-associative
    Unary { op: UnOp, e: Box<Expr> },
    Try(Box<Expr>),              // expr? (§16.3)
    Array(Vec<Expr>),            // variable-length array literal (v2.1: any element values, §11.3)
    Dict(Vec<(Expr, Expr)>),     // v2.1 Dict literal { k: v, ... } (§4.6)
    Set(Vec<Expr>),              // v2.1 Set literal { a, b, ... } (§4.6)
    Comprehension {              // v2.1 comprehension (§11.7):
        frame: CompFrame,        //   Array/Dict/Set/Tuple outer frame
        clauses: Vec<CompClause>,//   for x in iterable [if cond] chain
        body: Expr,              //   element expression (for Dict comprehensions the element is a (k, v) pair)
    },
    Tuple(Vec<Expr>),
    Lambda { params: Vec<Param>, body: Box<Expr> },   // |x| expr
    Match { scrutinee: Box<Expr>, arms: Vec<MatchArm> },  // match expression
    // v2.3: the `Pipeline` variant is removed — `|>` no longer enters the AST (parse error `E0010`); `BinOp` likewise no longer contains `Pipeline`
}
// v2.2: FStringPart = Literal(String) | Interp { expr: Box<Expr>, spec: Option<String> } | EscapedBrace
// v2.2: Literal::String gains `Single`/`Raw` markers (delimiter and whether it escapes); nested f-string literals are compile-time errors
pub struct FieldValue { pub name: Ident, pub value: Option<Expr> }
// CompFrame: output container (Array | Dict | Set | Tuple); CompClause: For{var, iter} | If{cond} (§4.6/11.7)
// Every node carries a Span; Block = Vec<Stmt>
```

**Key design: mathematical expressions and host expressions share the same AST** (spec §4.3: `math_expr := expr`). The distinction between the "symbolic world / numeric world" is **not** at the parsing layer but at the **lowering layer**: the same AST subtree follows one of two paths depending on context (§4.8) — this is the landing seam of the "three-world architecture", where the diagram in spec §two is placed.

### 4.3 Number Tower and Value (prima-core)

```rust
pub enum Number {
    Integer(BigInt),             // §6.1 exact; overflow escalation decided by the num_to_big policy
    Rational(BigRational),       // auto-reduced, positive denominator (§6.4 rule 3)
    Real(Real),                  // F32(f32) | F64(f64); NaN/Inf exist only here (§6.2)
    Complex(Box<Complex<Number>>), // recursive; re/im normalized to Rational or Real
}

// v2.0: fixed-width collapsed numeric types (§6.1) — implementation strategy: folded into the F32/F64 variants of Number::Real + new fixed-width integer variants
pub enum Number {                // v2.0 finalized
    Integer(BigInt),             // symbolic/exact layer
    Rational(BigRational),
    Real(Real),                  // F32 | F64
    Complex(Box<Complex<Number>>),
    // — collapse layer (one-to-one correspondence with Rust, §6.1) —
    I8(i8), I16(i16), I32(i32), I64(i64), I128(i128),
    U8(u8), U16(u16), U32(u32), U64(u64), U128(u128),
    Isize(isize), Usize(usize),
    BigFloat(BigFloat),
}

pub struct Complex<T> { pub re: T, pub im: T }   // a type that does not depend directly on num-complex,
                                                 // but reuses its trait implementations (T: Num) to assist generic arithmetic
pub enum Value {  // §5 landed verbatim
    Number(Number), Bool(bool), Char(char), String(String),
    Array(Array),            // v2.1: variable-length sequence, `Vec<Value>` (§11.3, see the v2.1 note below)
    Dict(Dict),              // v2.1: `HashMap<ValueKey, Value>` (§11.6)
    Set(Set),                // v2.1: `HashSet<ValueKey>` (§11.6)
    Matrix(Matrix), Function(Function),
    Class(ClassId),            // §5 class instance handle (§4.7)
    Expr(ExprId), Symbol(SymbolId),
    Option(Option<Box<Value>>), // §5 Option<T>: Some(T)/None
    Indeterminate(IndeterminateForm), Undefined, Error(Error), Nil,
    Tuple(Vec<Value>), Result(Result<Box<Value>, Error>),
}
```

> **v2.1 landing note**: `Value::Array` changes from v1.x's `Vec<Number>` to `Vec<Value>` (elements may be arbitrary values); `Dict`/`Set` are new variants whose keys/elements are wrapped in `ValueKey` (the hashable form of `Number`/`String`/`Char`/`Bool`/`Expr`/`Symbol`). The broadcast/matrix interface validates at the **call site** that array elements are numeric (`R0009`), no longer forcing homogeneity at the literal-construction layer. Interfaces such as `grad` that return multiple expressions are carried by `Value::Tuple` until the `Vec<Value>` conversion is complete.

**Promotion rule implementation** (§6.4 final): `promote(a, b) -> (Number, Number)` raises two numbers to a common layer — sequence `Integer < Rational < Complex<Rational> < F64 < Complex<F64>`; encountering a `Real` promotes the whole Complex to Complex<Real>. Reduction/normalization happens at `Rational` construction (`num-rational` supports it natively).

**Fixed-width collapse types**: `I8/U8/.../F64` exist only **after explicit collapse** (§6.1); `promote` does not participate (collapsed values do not participate in implicit promotion; conversions among fixed widths require explicit `to_*`, §6.3). Numeric arithmetic operates at the fixed-width layer under Rust-native semantics (overflow reports `R0001` via `checked_*`; operators such as `+` overflow directly into Rust semantics at the fixed-width layer).

> Performance note: MVP uses plain `BigInt`; `num-bigint` already has an internal small-integer optimization for small values, so no custom small-integer fast path is needed. If benchmarks show a hotspot, an `i64` inline tag can be added later.

### 4.4 ExprPool: hash-consing DAG (prima-core)

Spec §8.1 lands directly, with one change only: big integers in `ExprData` are boxed (the spec example already does this):

```rust
#[derive(Copy, Clone, PartialEq, Eq, Hash)]
pub struct ExprId(u32);

pub enum ExprData {
    Symbol(SymbolId),
    Integer(Box<BigInt>),
    Rational(Box<BigRational>),
    Add(Box<[ExprId]>), Mul(Box<[ExprId]>),
    Pow { base: ExprId, exp: ExprId },
    Apply { f: ExprId, args: Box<[ExprId]> },
    Indeterminate(IndeterminateForm),
}

pub struct ExprPool {
    global: DashMap<u64, ExprId>,           // content hash → existing node
    store: RwLock<Vec<ExprData>>,           // central store (append-only, no deletion: the symbolic layer is acyclic and resident)
}
thread_local! { static LOCAL_CACHE: RefCell<HashMap<u64, ExprId>> }   // §8.1 thread-local cache
```

**Intern flow**: compute the node's content hash `h = hash(ExprData)` → check `LOCAL_CACHE` → on a miss check `global` → if still a miss, allocate a new `ExprId`, append to `store`, and write back to both cache levels. Equality is `ExprId ==` (O(1), §8.2).

**Canonical forms** (§8.4): the children of `Add`/`Mul` are interned after sorting in a fixed order (numbers/constants → symbols → composite nodes, then by (kind, id)), guaranteeing `x+1 ≡ 1+x`.

**Simplification levels** (§8.3):

- only **levels 0/1** local rules during intern: `0*x→0`, `1*x→x`, `x+0→x`, same-layer numeric merging (`2+3→5`);
- **level 2** triggers in numeric contexts / via `simplify()` (`sin(0)→0`, `2*5→10`);
- **level 3** only via explicit `simplify()` (rationalization, trigonometric identities, factorization; the rule set grows incrementally from Phase 3+);
- printing/rendering always uses **level 0** (preserving the original form unless `simplify()` is called first, §8.3 default policy).

### 4.5 Symbol System (prima-core)

```rust
pub struct SymbolId(u32);
enum SymbolKey { TeX(&'static str), Ident(String), Physics(&'static str) }  // registry + global table
```

- **Built-in symbols** (§7): mathematical constants (`\e` `\pi` `\i` `\tau` `\infty` `\gamma` `\phi`), operators (`\log` `\ln` `\exp` `\sqrt` `\sin` `\cos` `\tan` `\sigma` `\prod` `\int` `\partial`), physical constants (§7.3 CODATA 2022, stored at high precision, not collapsed by default). Physical-constant values are stored as `BigRational` or `num-bigfloat`, requiring explicit collapse such as `to_f64(...)`.
- **Name resolution and conflicts** (§15.4) happen at the `prima-runtime` module table: `import` registers public items into the current scope, and same-name conflicts are compile-time errors.

### 4.6 Policy System Config (prima-runtime)

```rust
pub struct Config {
    pub domain: Domain,                       // §6.5
    pub undefined_handling: UndefinedHandling,// strict | custom(HashMap<ExprKey, Value>)
    pub fraction: bool,                       // true
    pub broadcast: bool,                      // true
    pub loop_optimization: bool,              // true
    pub opt_level: OptLevel,                  // v2.2: O0..=O3, default O2 (§10.2)
    pub simplify_level: u8,                   // 0..=3, default 2 (symbolic layer, independent of opt_level)
    pub num_to_big: bool,                     // true
    pub print_format: PrintFormat,            // latex | unicode | ascii
    pub overload_policy: OverloadPolicy,      // warn | allow | deny (added in v2.0, §13.2/18.5)
}

// v2.2: O0 < O1 < O2 < O3, with ordering comparison (`@builtin(ON)` decides by >=)
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum OptLevel { O0, O1, O2, O3 }
```

- **Storage**: `thread_local! { RefCell<ConfigStack> }` — a stack model: global (entry file) → module (pushed) → local `with config` (temporarily pushed), popped on block exit, naturally implementing "local > module > global" (§13.1).
- **Parallel propagation**: `parfor`/`@parallel` tasks **snapshot the Config and pass it into the rayon closure** at creation time (worker threads' thread_locals are not inherited); local policies inside a task stack on top of the snapshot.
- **Contamination check** (§13.2): `domain`/`undefined_handling` appearing in a non-entry file → compile-time error.

### 4.7 Module System (prima-runtime)

- `FileResolver`: root module `src/main.pra`; `import` resolution follows the §15.3 file mapping (`physics.pra` → module `physics`; a directory → submodule, with `main.pra` as the entry); import cycle detection.
- `Module`: `{ items: HashMap<String, (Visibility, Value)>, path: Vec<Ident>, config: Config, module_docs: Option<DocComment> }`; **private by default, `pub`/`pub(mod)` to expose** (§15.2); variables do not cross module boundaries. **v2.2: the item values carry `docs`** (item-level `DocComment`), read by `prima doc` and diagnostic notes (§4.10).
- **Class registry**: `ClassRegistry` (process-level `OnceLock`): `ClassId → ClassDef { name, fields: Vec<(Ident, Type, Option<DocComment>)>, methods: HashMap<Ident, MethodDef>, vis, docs }`. Class instances are `Value::Class(ClassId)` + a runtime **host GC handle** (replacing `Rc<RefCell<ClassInstance>>` as of v2.2, §4.12; `mem::Arc` provides explicit reference counting).
- Pre-import `core` (§15.5): all public items of core are injected into the root scope at startup.
- Evaluation order: at parse time, **pre-scan** all reachable modules (two passes: first collect symbol tables, then evaluate), so that `pub` items are immediately resolvable after `import`; module bodies are evaluated in import-dependency order.

### 4.8 Evaluation Model (prima-runtime interpreter)

The unified AST's **dual lowering** (the landing point of §4.2):

```text
expr_ast
 ├─ Symbolic context (MFn bodies, let RHS defaults to symbolic) → lower_to_expr() → ExprDAG → simplify → Value::Expr(ExprId)
 └─ Host context (fn bodies, control-flow conditions/loop variables)      → eval() → Value (numeric stack value / host object)
```

- **MFn** (`let f(x) = body`): the closure holds the body AST; on call, `substitute(params → actual ExprId)` yields the instantiated DAG → simplify → return `Expr`. When actuals are numeric, collapse on demand (§10 example: `f(3.0) → 15.0`).
- **Fn** (`fn`): a host closure that interpretively executes a Block; may have side effects (§11.2); may return `Result` (§16.3).
- **Broadcast** (§11.4): checked at the call site — elementwise when the argument is an `Array` and `broadcast := true`; **array elements must be numeric** (non-numeric → `R0009`), **empty arrays rejected** (`R0014`, §16 diagnostics); `@.` is the explicit broadcast operator; with `broadcast := false`, `map`/`@.` are provided.
- **Collections and comprehensions** (v2.1, §4.6/11.6/11.7): `Array`/`Dict`/`Set` literals evaluate to the corresponding `Value`; `for`/`parfor`/`in`/comprehensions share a unified iteration protocol (`iter_values(v) -> Vec<Value>`: Array elements, Dict keys, Set elements, ranges, String characters); comprehension evaluation = nested loops + filtering + collection into the frame container; the `in` binary operation does linear search on Arrays and O(1) on Dict/Set.
- **Mutable collection methods** (v2.1, §11.3/11.6): `v.push`/`pop`/`append`/`extend`/`insert`/`remove`/`clear`, `d[k]=v`/`get`/`insert`/`remove`/`keys`/`values`/`items`, `s.add`/`remove`/`discard`/`union`/`intersection`/`difference` are dispatched by `eval_method_call` with special-casing for `Value::Array/Dict/Set`; slice assignment `v[a..b] = [...]` is rewritten as a splice.
- **Patterns and destructuring** (§4.4): `match`/`if let`/`while let`/`let` all go through `match_pattern(pattern, value) -> Result<Bindings, MatchFail>`; constructor patterns (`Some`/`Ok`/`Err`) match against built-in variants; `..` wildcards the rest.
- **Classes** (§4.5/12.3): `Test::new(...)` looks up associated functions in the class registry; `obj.method(...)` looks up the method table via the `ClassId` of `Value::Class`; `self` binds as a shallow copy of the instance's GC handle; field access `obj.x` checks visibility; `T { a, b }` constructs a new instance (missing field `E0061`, unknown field `E0060`).
- **f-string** (v2.2, §18.1): evaluating `ExprKind::FString` = processing `parts` segment by segment — `Literal` segments are concatenated as-is, `Interp` segments evaluate `expr` and then render it per `print_format` (reusing `Renderer::render_to_string`), `EscapedBrace` segments output `{`/`}`; `spec` refinement (e.g. float precision) is applied before rendering; as of v2.2 nested f-string literals are rejected at parse time.
- **`format` deprecation** (v2.2): `format` is no longer pre-imported and no longer registered as a builtin; the evaluator emits `W0006` (hint to switch to f-strings) when it encounters a call named `format` (transition period), after which it is treated as an undefined name.
- **`@builtin(ON)` dispatch** (v2.2, §18.4): `bind_builtin` dispatches on `Annotation::Builtin{ opt_level }` — `O0`: must be registered (`E0055`), a function body is forbidden (`E0056`), binds the Rust implementation directly; `O1..O3`: must **have** a `.pra` function body (fallback implementation), the Rust implementation is optional; at call sites `config.opt_level >= opt_level && registered` decides between the Rust implementation and evaluating the `.pra` body; an invalid level argument → `E0057`.
- **Method-call diagnostic note** (v2.2, §16.4): when method resolution fails or a method-internal error is raised, `eval_method_call`/`check` fetches the method's (or its containing class's) `DocComment` and signature from the class registry/module table and appends them to the diagnostic note.
- **`?` operator** (§16.3): `expr?` inside a function returning `Result`: `Err(e)` → early return `Err(e)`; inside a function returning `Option`: `None` → early return `None`; context mismatch → compile-time `E0054`.
- **`|>` pipeline** (§9.7, removed in v2.3): `|>` no longer participates in parsing; its occurrence is a parse error `E0010` (removed-syntax hint, same style as `try/catch`); the replacement is class method chaining / direct calls.
- **Loop optimization** (§10): with `loop_optimization := true` **and `opt_level >= O1`**, a `for i in a..b { acc += i }` shape is recognized as a closed-form formula (implemented in Phase 2, starting with the `0..n` and `1..n` arithmetic-progression patterns).
- **parfor** (§17.2, landed in v2.1): `rayon::par_iter`; the loop body undergoes **side-effect static checking** — only assignment to index slots `A[i]`/`A[i] +=` and pure-function calls are allowed, violations report `E0082`; each slot is evaluated independently (each thread chunk gets an independent Evaluator sharing the process-level `ExprPool`/`SymbolTable`), and the whole array is written back to the binding at the end.
- **@parallel broadcast parallelism** (§17.1/17.4, landed in v2.1): `Function::User` gains `parallel: bool` (true when a `MathDef` carries `Annotation::Parallel`); in the broadcast path, when array length ≥ the threshold (default 1024), work is chunked by the `rayon` thread count, each chunk with an **independent Evaluator** (snapshotting the current Config, dropping the output sink), evaluating the `@parallel` MFn's formal-parameter environment within the chunk (the function body is required to be **self-contained**, not referencing free variables); small arrays take the sequential path.
- **Symbolic differentiation** (§19.4, landed in v2.1): `crates/prima-runtime/src/diff.rs` implements `derivative`/`partial`/`grad`/`limit` on the `ExprPool` DAG; `eval_call` intercepts these four names, and `derivative(f, x)`/`grad(f)` accept MFn names (taking the function body via `resolve_func` and binding the formals as symbols) or symbolic expressions; `limit` first tries direct substitution, and on 0/0 applies l'Hôpital's rule (at most 8 rounds).
- **Console** (§18.1b, landed in v2.1): `print` appends no newline, `println` appends a newline (dispatch difference between `Builtin::Print`/`Println`); `input`/`read_line` read stdin (EOF/error returns an empty string).
- **Error flow** (§16.2/16.3): recoverable errors are expressed as `Value::Result`; `to_*`/`unwrap`/`expect` turn into **terminating panics** in the interpreter (`panic!`, §16.2 class 3). **There is no `try/catch`**; parsing the `try` keyword reports `E0010` and suggests using `Result` instead.
- **Undefined strictness** (§6.2): `Undefined` participating in any unary/binary operation raises `UndefinedError` (the operation does not propagate); `Indeterminate` exists only in the symbolic layer and becomes `Undefined` when collapse fails.

### 4.9 Renderer (prima-core)

```rust
trait Renderer { fn render_expr(&self, pool: &ExprPool, id: ExprId, out: &mut String); }
// Implementations: LatexRenderer / UnicodeRenderer / AsciiRenderer, chosen by the print_format policy
```

- LaTeX is an MVP gate (§19.1 milestone 1): output at the `\sqrt{2} + \pi` level; `tex"..."` literals are parsed into a render tree (a small embedded TeX parser, supporting the MVP subset in Phase 1 first, expanding incrementally).
- Spec §7 emphasizes that "built-in symbols are independent of TeX; TeX is only a view": `ExprDAG → (LaTeX|Unicode|ASCII)` is a pure view transformation, while the reverse (`tex"..."` → DAG) is parsing; the two are decoupled.
- **v2.2 reuse**: f-string interpolation, the `render` module (`to_svg`/`to_png`/`to_terminal`), and `print`'s terminal formula rendering (cargo feature `term-render`) all reuse these three backends, changing only the output target.

### 4.10 Error Model Summary

| Category | Timing | Form |
|------|------|------|
| syntax / type / import / visibility / contamination | compile time | collecting diagnostics (§16.2), numbered `E####`, rendered by codespan-reporting |
| `Undefined` participating in operations | compile time when statically decidable, otherwise run time | `R0006`, returned as `Result` (no panic) |
| out-of-bounds / dimension / I/O / collapse failure (`try_*`) | run time | `R####`, returned as `Result` |
| explicit error abandonment (`to_*` / `unwrap` / `expect`) | run time | terminating panic (with cross-language stack) |
| internal unrecoverable errors | run time | panic (last resort) |

### 4.11 `prima doc` (v2.2, tooling layer)

- Input: the entry `.pra` and all its reachable modules (including embedded stdlib modules, §4.7); `--stdlib` outputs only the built-in standard library.
- Output: Markdown (stdout by default; `-o` writes a file), structured as module (`//!` docs) + the signatures of each public item (`fn`/`let` (MFn)/`class`/methods/fields/`const`) + the `///` doc text; configurable rendering style.
- Single data source for docs: `Module.items`' `docs` and the `ClassRegistry`'s `docs` (§4.7), sharing the same data as diagnostic notes (§4.8).
- `prima check` and the interpreter share this data, guaranteeing that "what the docs can see is exactly what diagnostics can give".

### 4.12 Host-layer GC (v2.2, prima-runtime)

- Scope: class instances of `Value::Class(ClassId)` and host objects (`Value::Array/Dict/Set` buffers are managed by ownership chains; the GC covers only instances that need to be shared); `ExprPool`/`SymbolTable`/the numeric layer do not participate.
- Structure: **one GC heap per Evaluator** (`Heap { objects: Vec<HeapObj>, free: Vec<u32>, bytes: usize, epoch: u64, watermark: usize }`); instances are referenced by index handles (`Value::Class(ClassId, HeapIndex)`), and the class registry still describes types by `ClassId`.
- **Mark-sweep**: safe points sit at block/function/loop boundaries and call sites (the evaluator proactively checks `bytes >= watermark`); the root set = the `EnvRef` chain + the evaluation stack + the module table; mark recursively from the roots by `Value` fields; the sweep phase merges unmarked slots into the free table and rolls back the byte count.
- Determinism: single-threaded, no background scanning thread; each `parfor`/`@parallel` task has its own heap, discarded in full when the task ends.
- External interface: `mem::collect()` manually triggers one safe-point collection; `mem::Arc` is a separate explicit reference-counting wrapper (`Weak` counts do not participate), not traced by the GC.
- Semantic red lines (§12.3): the GC is invisible to programs (no destructors, no `finalize`); cyclic references are collectable; use `mem::Arc` when deterministic release is truly needed. Memory-pressure monitoring (`/proc/self/status` VmRSS or allocator statistics) serves as one of the watermark trigger sources.

---

## 5. Implementation Roadmap (Phase 0 → 5)

Each Phase ends with runnable acceptance commands. Phases 0–2 have been delivered under v1.x; the v2.0 changes are spread across each Phase's incremental tasks — see the "v2.0 increments" subsections of each Phase.

### Phase 0: Project skeleton + front end (syntax crate)

- Set up a workspace with 4 lib crates + the root CLI package; the CLI uses clap to wire up `run` (the other subcommands are placeholders that report "not implemented").
- `SourceMap` / `Span` / lexer (full token set of §2.1) → unit tests covering every token type and error branch.
- Recursive-descent parser + Pratt (§2.2 precedence table), **covering every production of the Appendix A BNF**.
- Diagnostic collector: multiple errors can be reported from one source.
- **Acceptance**: `cargo test` passes; `insta` snapshot tests (`tests/parsing/*.pra` → AST dump); `proptest` random token sequences never panic or hang.

**v2.0 increments**:

- Tokens: `;`, `?`, `..=`, `class`/`self`/`Self`/`impl`/`match`, annotations `@builtin`/`@c_api::extern`.
- Pattern parser (all §4.4 patterns) + `match`/`if let`/`while let`/`let` destructuring.
- Statement separation: `;` as the norm + newline compatibility (`W0001` warning, §2.2). (As of v2.3 newline separation is removed and reports `E0011`; `W0001` is gone.)
- Class syntax (fields/methods/`Self`/visibility) + `impl ops::X for T`.
- Full string escapes (including `\u{XXXX}`) + `format` function signature (evaluation in core/runtime).

### Phase 1: MVP symbolic engine (core crate; §19.1 milestones 1–3)

- `Number`/`Value`/promotion rules; `ExprPool` + simplification levels 0/1 + canonical forms.
- Built-in symbol table (§7.1–7.2 constants and operators); LaTeX renderer (level-0 raw form).
- Minimal interpreter set: `let`/`const`, MFn (symbolic substitution evaluation), array literals, `print`, unary/binary operations, symbols like `\pi` taking part in simplification (goal: `simplify(tex"\e^{i\pi}+1") → 0`).
- **Acceptance**:

  ```text
  prima run milestone samples: f(v) broadcast → [1, 4, 9];
  tex"\sqrt{2}+\pi" prints LaTeX;
  simplify(tex"\e^{i\pi}+1") → 0
  ```

> **Phase 1 landing notes (2026-08)**: all complete; acceptance samples in `examples/phase1.pra`. Deviations from / finalizations of this document:
>
> - `ExprData` gained a `Real(Real)` variant (not listed in spec v1.0 §8.1, added so floats can enter the symbolic DAG); `ExprData::Symbol(SymbolId)` uses the new `core::symbol::SymbolId` type.
> - Simplification rules live in `core::simplify::simplify(pool, builtins, id)` (no level parameter; the MVP applies all levels): the intern layer (`ExprPool::add2/mul2/pow2/sub2/div2` + `add_n/mul_n` flattening/constant folding) handles levels 0/1; `simplify` handles levels 2/3.
> - The TeX literal parser lives in `prima-syntax::tex` (MVP subset) and produces the same AST as normal syntax.
> - The interpreter lives in `prima-runtime::eval`; broadcasting applies pure functions elementwise at the call site, rejecting empty/nested arrays; the `a |> f` pipeline is rewritten as a call (that pipeline lowering was removed in v2.3, which reports `E0010` instead).
> - Back then both `print`/`println` emitted a newline; **as of v2.1 they are distinguished**: `print` does not newline, `println` does (§4.8 console); LaTeX output is the default.

### Phase 2: Config, numeric layer, and error handling (milestones 4, 5, 7)

- Three-level Config policy (§4.6); `fraction := false` takes effect; F64 imprecision contagion (§6.4).
- Collapse function family (v1.0 scope) including Result wrapping and the `unwrap` family.
- `fn` host functions, `if`/`while`/`for step`/`return` (v1.0 included `try-catch`; **removed in v2.0**).
- Loop optimization (closed form for arithmetic progressions); `Undefined` strictness checks (§6.2).
- Module system (§4.7) + `pub`/`import`/conflict detection + core prelude.
- **Acceptance**: milestone 4/5/7 samples pass; `prima check` reports type errors in the §16.4 format.

> **Phase 2 landing notes (2026-08, v1.x)**: `examples/config_fraction.pra`, `examples/loop_optimization.pra`, `examples/try_catch.pra`. Deviations from / finalizations of this document:
>
> - Scopes are implemented as an `Rc<RefCell<Env>>` shared chain (`EnvRef`).
> - `Value::Result`/`Value::Error` carry errors as message strings (the structured `Error` enum was completed in v2.0).
> - `to_bigfloat` is a degenerate implementation; `print_format` only works with the latex renderer.
> - `prima check` first performs literal–annotation-level static checking.
> - Loop optimization first covers `0..n` and `1..n`.

**v2.0 increments**:

- **Remove `try/catch`**: the parser drops the `try` statement production; `examples/try_catch.pra` is rewritten as a `Result` + `match` version; `E0010` gives a "use Result instead" hint for the `try` keyword.
- **Statement separation**: all repo examples/tests switch to `;`; a new `W0001` warning channel is added. (As of v2.3 `W0001` is removed together with newline separation, which now reports `E0011`.)
- **`?` operator**: `eval_try` implemented in the evaluator (§4.8).
- **`Result`/`Option` first-class treatment**: `match`/`if let`/`unwrap` family, `Some/None/Ok/Err` constructor patterns.
- **Collapse family expansion**: full `to_*`/`try_*`/`checked_*`/`clamped_*` for `i8…u128/isize/usize` (§9 in full).
- **Numbered diagnostics**: `E`/`R`/`W` codes wired into `DiagnosticCollector` and `codespan-reporting` headers (§16.4/Appendix C).

### Phase 3: Parallelism and symbolic differentiation (milestone 6 + §19.4 MVP)

- `@parallel` annotation + `parfor` (rayon, Config snapshot propagation, §4.6); side-effect static checks.
- Symbolic differentiation engine: `derivative`/`partial`/`grad` recursive differentiation rules (§19.4 MVP); `limit` (starting with Taylor expansion / l'Hôpital).
- **Acceptance**: `derivative(f, x)` on `x^2 + sin(x)` outputs `2x + cos(x)`; million-element `@parallel` broadcast verifies the speedup (criterion benchmark).

**v2.0 increments**:

- Automatic inlining (§10.2): `InlinePass` runs after type checking and before evaluation/code generation; the heuristics (MFn / side-effect-free fn, size thresholds, non-recursive) are decided internally by the compiler and not exposed as annotations.
- Constant folding / CSE / loop-invariant hoisting: implemented incrementally as a simplification/evaluation side path, working together with `simplify_level`.

> **Phase 3 landing notes (2026-08, v2.1)**: all complete. Deviations from / finalizations of this document:
>
> - `@parallel` is marked via `Function::User.parallel`; the broadcast path uses rayon chunking for `parallel && len ≥ 1024` (each chunk gets an independent `Evaluator`, snapshotting Config and discarding output). `@parallel` function bodies are required to be **self-contained** (must not reference free variables).
> - `parfor` side-effect static checks run in the `ParFor` branch of `eval_stmt` (reporting `E0082`); indexed-slot assignment (`A[i] = …`/`+=`) and pure function calls are allowed; each slot is evaluated independently, then the whole array is written back to the binding.
> - Symbolic differentiation lives in `crates/prima-runtime/src/diff.rs`: `derivative`/`partial` (the same differentiation, one variable), `grad` (auto-collects partial derivatives w.r.t. each free symbol; returns `Value::Tuple` until `Vec<Value>`-ification lands), `limit` (direct substitution → l'Hôpital for ≤8 rounds); `eval_call` intercepts the four names, accepting an MFn name or a symbolic expression.
> - The `print`/`println` distinction (`print` without newline) and `input`/`read_line` landed together.

### Phase 4: Standard library and toolchain

- `prima-stdlib`: `linalg` (nalgebra: Matrix construction/operations/decomposition/solving), `stats`, `io` (JSON/CSV), `physics` (§7.3 constants), `plot` (SVG).
- CLI completion: `repl` (rustyline, continuation-line detection), `fmt` (AST printer reusing the renderer), `check` (pure type checking, supporting `--deny W####`), `test`, `doc`.
- **Acceptance**: every entry in the Appendix B function quick-reference works (golden tests after importing the corresponding module).

**v2.0 increments**:

- **`String` class** (`@builtin`, §18.1): full method set (`push/insert/len/...`) + `format`/`to_string`.
- **`sys`** (`path`/`env`/`os`), **`time`**, **`num`** modules (§18.2/18.3, Appendix B.5).
- **`ops`** (§18.5): `impl ops::Add for T` registers into the operator dispatch table; at call sites `overload_policy` decides between `W0005`/allow/error.
- **`@builtin`** (§18.4): builtin registry (`BuiltinRegistry`), signature binding + visibility validation (`E0055`/`E0056`).
- **`@c_api::extern`** (§18.4): AST-level annotation + type validation (`E0071`/`E0072`); the MVP first produces an "export manifest" + an ABI header skeleton, with actual binary exports landing in Phase 5 AOT.

**v2.1 increments (base type usability, scheduled for Phase 4 + later increments)**:

- **Variable-length `Array`**: `Value::Array` → `Vec<Value>`; `push/pop/append/extend/insert/remove/clear`, slice assignment, negative indices, `+`/`+=` concatenation, `in` membership test (§4.3/4.8).
- **`Dict`/`Set` variants**: hashable `ValueKey` wrapper; literals/indexing/methods/set algebra (`∪`/`∩`/`\`); `R0012`/`R0013`/`R0014` error codes wired in.
- **Comprehensions**: `ExprKind::Comprehension` evaluation + a unified iteration protocol; BNF in spec Appendix A.
- **Convenience functions**: `len/enumerate/zip/sorted/reversed/sum/prod/min/max/all/any/join/count/index/first/last` (core prelude).
- **Console**: the `print` (no newline) / `println` (newline) distinction landed with Phase 3; `input`/`read_line` landed with `print` dispatch.

> **Phase 4 landing notes (2026-08, v2.1)**: all complete. Deviations from / finalizations of this document:
>
> - **The stdlib uses "embedded `.pra` signature modules + a `@builtin` implementation registry"** (aligning with the design intent of spec §18.4; ADR in §7): each module is a `.pra` embedded in the binary that only declares typed `@builtin pub fn` signatures (e.g. `linalg::determinant(M: Matrix<F64>) -> F64`); on the Rust side implementations are registered under a `"module::function"` key (`register_impl`), and the `.pra` is embedded via `register_module_source`. `import <module>` resolves to the embedded source and is **evaluated as an ordinary module** (`collect_pub` binds exports to the implementations' `Function::Native`), giving a single source of truth for the API surface.
> - **Module resolution priority**: embedded stdlib source → host namespace → local files; **registered stdlib path names are reserved** (Rust-like `std`; a local `linalg.pra` cannot shadow `import linalg`). Physical constants (pure data, no logic) stay as host-namespace `NamespaceItem::Val`, the only module that does not go through `.pra`.
> - **`@builtin` binding**: the root module binds by core builtin name (`Builtin::from_name`); inside stdlib modules the **implementation registry is consulted first** (`"module::name"`), with unregistered ones → `E0055` (core builtins do not shadow same-named implementations inside modules). `@builtin` function names support `::` paths (`Matrix::zeros`, `Duration::from_secs`).
> - **`prima check` call-site type checking**: stdlib calls are validated against the embedded signature table (`E0050` argument count/type); overloads (e.g. `stats::quantile` in array/distribution dual form) pass if any signature matches; `Value` type names act as wildcards; unknown types do not produce false positives. Fewer arguments than declared are allowed (optional trailing parameters).
> - **`@c_api::extern`**: `E0071`/`E0072` static validation; `prima compile --emit-headers` generates C headers from the export manifest (`crate::capi`).
> - **CLI completion**: `repl` (rustyline, bracket continuation), `fmt` (AST printer, `-w`/`--check`), `test` (runs all `examples/` samples by default), `doc` (definition listing + `///` comments), `check --deny W####` (warnings promoted to errors).
> - **Deviations recorded**: physical constants are accessed as `physics::planck_const` (bare name); the `physics::\planck_const` TeX name from spec §7.3 is not used as a module key; `import sys::path` binds the **full path** (§15.3 convention, `sys::path::join`); the `path::join` shorthand from the spec §18.2 example is unsupported; `String.split` returns `Array<String>` (§18.1 v2.1); `linalg::norm/solve/lstsq` use a `Value` wildcard for the first argument or RHS to cover the vector/matrix dual forms.

### Phase 5: JIT (§19.2)

- `cranelift-codegen` hot-path compilation: triggered by a call-count threshold (default 100) or the `@jit` annotation; `ExprDAG → bytecode → cranelift IR → native code`; the symbolic layer stays interpreted.
- AD forward (Dual) and reverse (Tape) modes (§19.4 stages two and three); `jit(grad(f))` combination.
- Optimization pipeline integration (§10.2 in full): constant folding, CSE, loop optimization, automatic inlining, TCO, DCE.
- C ABI export (§18.4): `--emit-c-abi` produces a dynamic library + header files.
- **Acceptance**: `f(to_f64(101))` takes the native path; criterion compares interpreted vs. compiled time, with threshold tuning.

> **Phase 5 landing record (2026-08)**: all complete. Deviations/finalizations vs. this document:
>
> - **New crate `prima-jit`** (dependency direction `syntax → core → prima-jit → runtime`): `ExprDAG → Bytecode → cranelift IR → native`. The bytecode is a pure `f64` stack machine (`Const`/`Param`/arithmetic/`Pow`/transcendentals); **transcendentals do not depend on cranelift's libcall symbol-name resolution** — they call `#[unsafe(no_mangle)] extern "C"` trampolines (`pj_sin`/`pj_cos`/…) registered via `JITBuilder::symbol`. `CompiledScalar::call` is lock-free and thread-safe; compilation is serialized under a process-global `OnceLock<Mutex<JitEngine>>`. cranelift 0.135 notes: no `frem` (`Rem` goes through `pj_rem`), `MemFlagsData`/`Offset32` paths, `JITModule::new` requires `is_pic=false`.
> - **Automatic hot-path compilation**: `Function::User` gains `hot: Arc<HotState>` (`force` + `AtomicU64` call counter + `OnceLock<Option<Arc<CompiledScalar>>>`, shared across clones). With all non-complex `Number` arguments the hot path applies: `@jit` (a `MathDef` annotation) compiles on the first call; otherwise the `JIT_CALL_THRESHOLD`-th (default 100) numeric call triggers compilation and returns natively, and failures are cached (never retried); non-numeric arguments fall back to the interpreted path. Semantics are unchanged (native and interpreted agree).
> - **`jit(...)` builtin** (`Builtin::Jit`, intercepted in `eval_call`): accepts an MFn name → compiles a forward scalar; `jit(grad(f))` → **reverse mode** (`ad::Tape`) multi-variable gradient (`Value::Array`); a bare symbolic expression → compiles with its free symbols as parameters; a `grad` symbolic tuple → per-component numeric evaluation. The product is **`Value::JitFunction(u32)`** (a new `prima-core` variant, a handle into the process-global `runtime::jit` registry, with `compiled`/`tape`/`expressions`/`fallback` forms — automatically falling back to interpretation when compilation is unavailable); the call site in `eval_call` supports a `JitFunction` as the callee.
> - **AD** (`crates/prima-runtime/src/ad.rs`): forward `Dual` (`forward_derivative`) + reverse `Tape` (post-order DFS + memo to build the graph; `grad(inputs)` computes all partials in one backward pass, supporting the `Pow` log-derivative chain rule and built-in constants). The reverse tape is the runtime engine behind `jit(grad(f))`.
> - **Optimization pipeline** (§10.2): `core::opt` (`const_fold` = simplify, `cse` = natural sharing via hash-consing, `optimize`); `runtime::opt` (`tail_call_of`, a pure-AST tail-call analysis: a final direct `return f(args)` preceded only by effect-free statements); the interpreter's `Function::Host` branch implements **TCO** with a trampoline loop (100k-deep tail recursion runs in constant stack space; early `return`s inside the effect-free prefix still exit correctly). Automatic inlining is inherent to MFn substitution; loop optimization reuses Phase 2; scalar bytecode is branch-free so DCE is vacuous (constant folding already removes dead code).
> - **C ABI export** (`--emit-c-abi`): per the maintainer's decision, a **cdylib shell crate** — after parsing, collect the `@c_api::extern` exports, generate the C header (`-o` base + `.h`), and in a temp directory generate a `cdylib` shell crate (absolute-path deps on `prima-runtime`/`prima-core`, embedding the source file's absolute path, one `#[unsafe(no_mangle)] extern "C"` wrapper per export backed by `call_file_export`'s thread-local cached evaluation), built with `cargo build --release` to produce `.so`/`.dylib`/`.dll`. Requires cargo at runtime; `--emit-headers` stays as the offline path. Verified via ctypes: `add(2.5,3.0)=5.5`, `hello("world")` round-trips.
> - **criterion benchmark** (`benches/bench_jit.rs`): the same `x^4 + sin(x)·x + exp(x)` DAG — interpreted recursive ~373ns vs. native ~34ns (≈11× speedup), with `f(101)` agreeing on both paths — i.e., the `f(to_f64(101))` acceptance criterion takes the native path.

> AOT (§19.3, WASM/standalone executables) is outside this roadmap; it will be scoped and evaluated only after Phase 5 completes.

### Phase 6: String and formatting overhaul (v2.2, spec §three/18.1, priority max)

**Work item 1** (`format` removal → Python-style f-strings). **Chunking rationale**: lexer/AST/evaluation/examples/docs must be aligned in one go; this is a breaking change and lands first to freeze the language surface.

- Lexing: `'...'` single-quoted strings (escape-equivalent to `"..."`), `r"..."`/`r'...'` raw strings (no escaping), `f"..."`/`f'...'`/`rf"..."` f-strings (`{}` interpolation, `{:spec}`, `{{`/`}}` escapes); `Literal::String` gains `Single`/`Raw` markers.
- Parsing: `ExprKind::FString` (`Vec<FStringPart>`, §4.2); **nested f-string literals → compile-time error**; `format` is no longer a keyword/builtin, and calling a function named `format` emits `W0006` (§4.8).
- Evaluation: f-strings concatenate segment by segment + interpolated rendering (`print_format`, §4.8); same-named functions inside modules such as `time::format`/`plot::savefig(format=…)` are unaffected.
- Migration: all repo `.pra`/tests/examples rewrite `format("...{}...", x)` to `f"...{x}..."`; all insta snapshots are updated.
- **Acceptance**:

  ```text
  cargo test                        # new snapshots (positive/negative f-string cases)
  prima run examples/fstring.pra    # output aligned with Python semantics
  prima check sample: W0006 hints to switch to f-strings
  ```

### Phase 7: Doc comments and diagnostic enhancements (v2.2, spec §4.1/16.4, priority max)

**Work item 2** (doc stabilization + stdlib docs reachable + method-error notes carrying the definition and docs).

- Lexing/parsing: `///`/`//!` are collected as `DocComment` (raw text + span preserved), entering the AST via `Program.module_docs`/`Stmt`/`ClassMember`/`Import` (§4.2).
- Data plane: `Module.items` and `ClassRegistry` carry `docs` (§4.7); the embedded stdlib module sources get their `///` docs filled in at the same time (`core/string.pra` etc.).
- Tooling: `prima doc` (Markdown output, `-o`, `--stdlib`, §4.11); `prima check` validates doc-comment legality (e.g. dangling `///`) and gives `W`-level hints (optional).
- Diagnostics: method-call failures (`E0040`/`E0050`/argument count/R codes) append **the method signature + definition location + `///` docs** to the note (§4.8); stdlib method docs are queryable offline (`prima doc --stdlib`).
- **Acceptance**:

  ```text
  prima doc --stdlib                # full method docs such as core/string.pra
  prima run triggering name.toUpperCase()   # diagnostic note includes the to_upper definition and docs
  cargo test                        # snapshots: doc notes output stably
  ```

### Phase 8: Optimization-level system (v2.2, spec §10.2/13.2, priority low, but a prerequisite for Phases 9/10)

**Work item 4** (`opt_level` policy + per-level optimization passes).

- Policy: `Config::opt_level: OptLevel` (`O0..=O3`, default `O2`, §4.6); the three-level policy works as usual (`with config { opt_level := O3 }`).
- Pass gating: enabled by level — `O1`: constant folding/DCE/loop closed forms; `O2`: +CSE/automatic inlining/TCO/automatic JIT compilation; `O3`: +SIMD recognition/vectorization/loop unrolling/unconditional inlining of small functions. Existing policies such as `loop_optimization`/`simplify_level` cooperate with `opt_level` (the §4.8 loop-optimization bullet).
- **SIMD recognition** (`O3`): `runtime::simd` recognizes elementwise patterns over dense numeric arrays (broadcast, element operations inside `for i in 0..n`), mapping them to `std::simd` (portable SIMD) or chunked parallelism; applied only when numeric semantics are provably unchanged; the order of checks such as `R0009`/`R0014` does not change under vectorization.
- **Acceptance**:

  ```text
  cargo run -- run examples/opt_levels.pra   # per-level behavior identical, performance different
  cargo bench --bench bench_jit              # SIMD speedup comparison O3 vs O2
  cargo test                                 # per-level result equivalence (numeric semantics unchanged)
  ```

### Phase 9: `@builtin` registration simplification and layered optimization (v2.2, spec §18.4, priority normal)

**Work item 3** (`@builtin(ON)` + simpler registration).

- Syntax/validation: `Annotation::Builtin{ opt_level }` (§4.2); `O0` forbids a function body (`E0056`) / must be registered (`E0055`); `O1..O3` must have a `.pra` function body (fallback implementation), the Rust implementation is optional; invalid level → `E0057`.
- Dispatch: at call sites `config.opt_level >= opt_level && registered` picks between Rust and `.pra` (§4.8); semantic consistency of the two implementations is guaranteed by integration tests (comparing outputs for the same input).
- Registration simplification: a declarative `builtin!` macro (in `stdlib.rs`, `builtin!("linalg::determinant", det_impl, MinLevel::O1)` form) replaces the manual `register_impl` string keys; the by-name fallback is kept (core builtins do not shadow same-named implementations inside modules).
- **Acceptance**:

  ```text
  cargo test                                 # @builtin(O1) positive/negative cases + two-implementation consistency
  cargo run -- run examples/builtin_layers.pra
  ```

### Phase 10: Builtin method system (full String method set, v2.2, spec §18.1, priority xhigh)

**Work item 5** (all of Python's stable `str` methods land; the method list and docs are authoritative in `.pra`).

- Expand `core/string.pra` into a complete `String` class: adapt item by item against **Python 3's stable `str` methods** (case conversion, find/replace, split/join, padding/alignment, Unicode normalization, slicing/iteration, encoding conversion, etc.), each method with a `///` doc comment.
- Performance layering (the Phase 9 mechanism): hot paths (`split`/`replace`/`to_upper`/`to_lower`/`strip`/`find` etc.) are marked `@builtin(O1)`/`@builtin(O2)` with Rust implementations; low-frequency/readable methods are written directly in `.pra`.
- Also: existing `Array`/`Dict`/`Set` methods fill in the missing items of the Python comparison list (likewise authoritative via `.pra` doc comments).
- **Acceptance**:

  ```text
  prima doc --stdlib                 # full String method docs (signatures and descriptions)
  cargo test -p prima-stdlib         # String methods vs Python behavior comparison table (golden)
  cargo test                         # layering: same method agrees at O0 and O2
  ```

### Phase 11: Standard library expansion (v2.2, spec §18.6, priority high)

**Work item 6** (`math` numeric utilities, `physics` common formulas, `sys` interaction, `plot`/`render` rendering).

- `math`: factorization (trial division/Pollard rho), prime sieves, `taylor` (truncated power series), polynomial operations — mostly `.pra`, with hot paths layered via `@builtin(ON)`.
- `physics`: common formulas **implemented directly in Rust** (kinematics/mechanics/harmonic oscillation/thermodynamics/basic electromagnetism, for fast optimization) + common Classes (e.g. `Vector3`); CODATA constants are kept (§7.3).
- `sys`: `sys::process`/`sys::fs`/`sys::term` submodules (running commands, file metadata, terminal dimensions, etc.).
- `plot`: line/scatter/bar/contour/heatmap, SVG by default, PNG optional (`resvg`).
- `render`: `to_svg`/`to_png`/`to_terminal` (reusing the §4.9 renderers); `print`'s terminal formula rendering is provided by the cargo feature `term-render`.
- Every module maintains its method list via `///`/`//!` doc comments (the spec §eighteen management principle).
- **Acceptance**:

  ```text
  cargo test -p prima-stdlib        # math/physics/sys/plot/render golden tests
  prima run examples/plot_render.pra # produces SVG/PNG and terminal formulas
  cargo build --features term-render # feature compiles
  ```

### Phase 12: Host-layer GC and `mem::Arc` (v2.2, spec §12.3/12.4, priority low)

**Work item 7** (a modern GC replaces reference counting + the standard library provides an Arc alternative).

- `prima-runtime` implements the §4.12 mark-sweep GC: `Value::Class` switches to holding GC-heap index handles; safe-point triggering; `parfor`/`@parallel` independent heaps; cyclic references collectable.
- Migration: all existing `Rc<RefCell<ClassInstance>>` are replaced with GC handles; copy semantics (shallow/deep) remain observably unchanged (§12.3 integration-test regression).
- `mem` module: `mem::Arc::new`/`strong_count`/`weak` (explicit reference counting, not GC-traced), `mem::collect()` (manual collection); internal Rust `Arc`s in `prima-core` such as `SourceLocation`/`HotState` are kept (unrelated to the language layer).
- **Acceptance**:

  ```text
  cargo test                       # class semantics regression (shallow copy/methods/cyclic-reference collection)
  cargo test -p prima-stdlib       # mem::Arc behavior tests
  prima run examples/gc_cycle.pra  # cyclic-reference instances collectable (memory-monitoring assertion)
  ```

> **v2.2 chunk ordering**: Phase 6 (f-strings) → 7 (doc) → 8 (opt_level, work item 4 brought forward as a prerequisite) → 9 (`@builtin(ON)`) → 10 (String) → 11 (stdlib expansion) → 12 (GC). Each chunk has independent acceptance; shared-file conflicts between chunks are reconciled by the main session under the "large-task working model" section.

---

## 6. Risks and Alternatives

| Risk | Mitigation |
|------|------|
| Hand-written parser coverage gaps | Appendix A BNF is the acceptance checklist; insta snapshots + proptest keep filling blind spots |
| Pattern-parsing ambiguity (constructor vs. call) | A dedicated `parse_pattern` parser function for pattern contexts, isolated from expression parsing; snapshots cover `Some(x)`/`Ok(v)`/nesting |
| Newline-compatible parsing misjudgments | This risk disappears with the v2.3 removal of newline separation — a non-block statement not terminated by `;` (and not at end of input / before `}`) is always `E0011` (`expected_separator`); proptest asserts stable error reporting |
| Simplification rule-base bloat (level 3) | Table-driven rules (`Vec<(Pattern, Rewrite)>`), not written into control flow; level 3 deferred to Phase 3+ |
| Class-instance ownership (shallow/deep copy) semantics complexity | GC handles + field-value copy dispatched by primitive value / class instance (§12.3); dedicated integration tests for the copy semantics of method arguments/returns (Phase 12 regression) |
| f-string interpolation parsing ambiguity/nesting | Interpolation bodies get an independent sub-scan (balanced `}`), and v2.2 explicitly forbids nested f-strings; proptest asserts no misjudgments or panics |
| Doc-comment parsing and AST consistency | `DocComment` preserves the raw text and span; `prima doc`/diagnostic notes share one data source (§4.11); snapshots cover doc output |
| `@builtin(ON)` dual-implementation semantic drift | Two-implementation consistency integration tests (comparing outputs for the same input, Phase 9); `.pra` is the only observable-semantics source |
| `opt_level` passes conflicting with existing policies | Semantic policies (`fraction`/`broadcast`/`simplify_level`) take priority over `opt_level`; per-level result-equivalence tests |
| SIMD (O3) numeric-semantics drift | Vectorize only when invariants are provable; IEEE rounding handled conservatively per the precision policy; equivalence benchmarks as a safety net |
| GC pauses/memory watermarks | Single-threaded collection at safe points, watermark-triggered; `parfor` independent heaps avoid cross-thread issues; `mem::Arc` provides a deterministic path |
| stdlib method list no longer doc-managed | The only source of method docs is `.pra` `///`; `prima doc --stdlib` and CI checks (failing on missing docs) guarantee nothing is omitted |
| Missed context checks for `?` propagation | Statically check the return type of the function containing `?`; `E0054` fully covered in the check phase |
| `num-bigint` performance below target | `rug` (GMP) replaces the backend under a feature flag; the `Number` wrapper layer is already isolated (§21 decision 30) |
| `nalgebra` performance insufficient | `faer` 0.24 as a replacement backend, with the stdlib layer trait-ified (`MatrixBackend`) |
| dashmap 7.0 RC instability | Pin 6.x; evaluate upgrading once 7 is released |
| ExprId cannot be serialized across processes | hash-consing Ids depend on in-process creation order → **caching/serializing ExprId is forbidden**; query-based incremental compilation (§19.3) caches only "replayable inputs", with the constraint documented |
| thread_local policy drift across rayon worker threads | Snapshot Config when tasks are created (§4.6); integration tests cover policy behavior inside parfor |

---

## 7. Decision Records on Spec Conflicts (ADR Summary)

| Spec clause | Spec suggestion | This plan | Rationale |
|---------|---------|--------|------|
| §19.1 | Use `logos` for lexing | Hand-written lexer | §2.1: token shapes are special; error localization is prioritized |
| §19.1 | Use `chumsky` for parsing | Hand-written recursive descent + Pratt | §2.2: diagnostic precision, context-sensitive grammar, incremental evolution |
| §19.1 | Use the `latex` crate for LaTeX output | Hand-written renderer (LaTeX/Unicode/ASCII) | §2.3: that crate is a document-typesetting library, unrelated to symbolic rendering |
| §19.1 | nalgebra or faer | MVP uses nalgebra; faer as the replacement backend | API maturity and documentation; the §6 risk table keeps a switch point |
| §19.2 | inkwell (LLVM) or cranelift | cranelift preferred | Pure Rust, no system dependencies, fast compilation (§2.4) |
| §19.2 | Threshold-triggered JIT | Default threshold of 100 calls + `@jit` annotation | Consistent with the spec, merely fixing "e.g. 100 calls" as the default value |

**v2.0 ADR additions**:

| Spec clause | Spec suggestion | This plan | Rationale |
|---------|---------|--------|------|
| §16.3 (v1.0) | `try/catch` error handling | **Removed**, replaced by `Result` + `?` + `match` | Finalized in spec §16.3 (v2.0): errors are values; `?` propagates and the `unwrap` family gives explicit fallbacks; avoids implicit exception flow disturbing symbolic evaluation |
| §4.2 (v1.0) | Newline-separated statements | **`;` as the norm, newlines deprecated (W0001)** | Finalized in spec §4.2 (v2.0): aligned with Rust, removes cross-line ambiguity; warns during the transition and gradually removes it |
| §9.7 (v1.0) | `\|>` pipeline composition | **Deprecated (W0002), replaced by class method chaining** | Finalized in spec §9.7/4.5 (v2.0): methods and chained calls are more expressive and avoid the readability loss of multi-stage pipelines |
| §6.1 (v1.0) | Collapse types I32/F32/F64 | **Extended to the full i8…u128/isize/usize/f32/f64 set** | Finalized in spec §6.1 (v2.0): one-to-one correspondence with Rust's primitive numerics, finer interop and numeric control |
| §18 (v1.0) | stdlib module set | **sys/time/num/ops/c_api added** | Finalized in spec §18 (v2.0): system layer / time / numeric extensions / operator overloading / interop |

**v2.1 ADR additions**:

| Spec clause | Spec suggestion | This plan | Rationale |
|---------|---------|--------|------|
| §5/§11.3 (v2.0) | `Value::Array(Vec<Number>)`, homogeneous, no nesting | **`Vec<Value>`, variable length, nestable as data; broadcast validates numeric homogeneity at the call site (R0009)** | Finalized in spec §11.3 (v2.1): Python-style usability prioritized; the symbolic/numeric layer invariants are still kept at the broadcast and matrix interfaces |
| §5 (v2.0) | No Dict/Set | **`Dict`/`Set` variants and `ValueKey` added** | Finalized in spec §4.6/11.6 (v2.1): maps/sets are high-frequency needs in scientific computing |
| §17.1 (v2.0) | `@parallel` has no self-containment requirement | **Function bodies must be self-contained (no free-variable references)** | Parallel subtasks evaluate independently with no shared environment; violations error on undefined names at evaluation time; documented constraint |
| §17.2 (v2.0) | parfor side-effect check timing unspecified | **Static check at evaluation time, reporting `E0082`** | Coexists with `prima check`'s incremental checking; moves into the compile phase after Phase 4 |
| §19.4 (v2.0) | `derivative(f, var)` requires a function value | **`eval_call` interception + accept MFn names/expressions** | `Value` currently has no `Function` variant and functions cannot be values; the interception approach supports `derivative(f, x)` without expanding the value system |
| §18 (v2.1) | stdlib modules as Rust namespaces | **Embedded `.pra` signature modules + a `@builtin` implementation registry** | Single source of truth for the API surface, `prima check` call-site type checking (E0050), unified error feedback and docs; physical constants (pure data) are the exception, staying in the `Val` namespace |
| §18.4 (v2.1) | `@builtin` binds builtins by name | **Inside modules the implementation registry is consulted first (`"module::name"`); core builtins bind by name only in the root module** | Avoids `sys::path::join` etc. being shadowed by same-named core convenience functions |
| §15.3 (v2.1) | Module resolution is file mapping only | **Resolution order: embedded stdlib → host namespace → local files; stdlib path names reserved** | Deterministic and Rust-like `std`; local same-named files no longer shadow built-in modules |
| §18.2 (v2.1) | After `import sys::path`, access via `path::join` | **Bind the full path `sys::path::join`** | Consistent with the §15.3 nested-import convention (`import linalg::fft` → `linalg::fft::double`); the §18.2 example's shorthand is unsupported |
| §7.3 (v2.1) | Physical constants accessed by the `\planck_const` symbol name | **Module keys use the bare name `physics::planck_const`** | Registry keys are plain strings; TeX names are only a display-layer concept |

**v2.1 Phase 5 ADRs added**:

| Spec clause | Spec suggestion | This plan | Rationale |
|---------|---------|--------|------|
| §19.3 (v2.0) | C ABI export (`--emit-c-abi`) directly produces a dynamic library | **cdylib shell crate**: generate a `cdylib` crate embedding the source file's absolute path (`#[no_mangle] extern "C"` wrappers run the interpreter through `call_file_export`), built with cargo to produce `.so/.dylib/.dll` | Exports are arbitrary-control-flow `pub fn` (`print`/strings/branches) that pure cranelift cannot compile directly; the shell reuses the full language semantics and is cross-platform; requires cargo at runtime, with `--emit-headers` kept as the offline path |
| §19.4 (v2.0) | `grad`/`jit` composition requires functions as values | **New `Value::JitFunction(u32)` handle + a process-global `runtime::jit` registry** | `jit(grad(f))` returns a callable value; the handle pattern mirrors `Value::Class` and avoids a `Function` value variant; falls back to interpretation when compilation is unavailable |
| §19.2 (v2.0) | JIT trigger threshold "e.g. 100 calls" | **Default `JIT_CALL_THRESHOLD = 100`; the 100th numeric call triggers compilation** | Aligns with the spec §19.2 example: after a `for i in 1..100` warm-up, `f(to_f64(101))` takes the native path |

**v2.2 ADR additions**:

| Spec clause | Spec suggestion | This plan | Rationale |
|---------|---------|--------|------|
| §18.1 (v2.1) | `format("...{}...", args)` function | **Removed, replaced by f-strings `f"...{expr}..."`; `W0006` during the transition** | Python convention, better readability/static analyzability (interpolation is an expression); eliminates the placeholder-index problem of functional templates; coexists with `print`'s multi-argument rendering |
| §18.1 (v2.1) | `String` class method list maintained by the spec | **The list and docs move into the `///` comments of the embedded `core/string.pra`; the spec keeps only the principles** | The method set evolves with Python's `str`; docs stay close to the implementation (same source as §4.11), avoiding spec/implementation doc drift |
| §16.4 (v2.1) | Diagnostic notes only carry help/expression | **Method-call errors append the method signature + definition location + `///` docs to the note** | In-language docs (`.pra` comments) are the only trustworthy source; errors are an entry point to the docs |
| §18.4 (v2.1) | `@builtin` has no parameters, function body forbidden | **`@builtin(ON)` layered optimization: `opt_level ≥ N` uses the Rust implementation, otherwise evaluates the `.pra` body** | Dual implementations of the same API satisfy both "fast" and "readable/portable"; `.pra` is the semantic authority, Rust the performance layer (Phase 9) |
| §13.2 (v2.1) | No optimization-level policy | **`opt_level` added (`O0`–`O3`, default `O2`)** | Separated from `simplify_level` (symbolic layer); `O3` carries SIMD/aggressive passes, enabled on demand |
| §10.2 (v2.0) | Optimization "not user-intervenable" | **Per-function non-intervention kept; leveling (global/module/local policy) optional** | A unified level makes performance configurable without exposing instruction-level annotations; semantic policies take priority |
| §12.3/12.4 (v2.1) | Class instances use `Rc<RefCell>` reference counting | **Host-layer mark-sweep GC; `mem::Arc` provides explicit reference counting** | Cyclic references collectable, zero counting overhead for shallow copies, and a deterministic path remains (`mem::Arc`); GC semantics are transparent to programs (§4.12) |
| §18 (v2.1) | stdlib module set fixed | **`render`/`mem` added; `math`/`physics`/`sys`/`plot` expanded** | Scientific plotting/formula rendering/memory control are high-frequency needs in scientific computing; physics formulas implemented in Rust for easy optimization (Phase 11) |

**v2.3 ADR additions**:

| Spec clause | Spec suggestion | This plan | Rationale |
|---------|---------|--------|------|
| §9.7 (v2.0) | `\|>` pipeline deprecated (W0002), to be gradually removed | **`\|>` removed, reporting the parse error `E0010` (removed-syntax hint, same as `try/catch`); `W0002` deleted** | Finalized in spec §9.7 (v2.3): class method chaining has fully replaced the pipeline; the syntax layer no longer accepts the form |
| §4.2 (v2.0) | Newline separation deprecated (W0001), to be gradually removed | **Newline separation removed; `;` is the sole statement separator, reporting `E0011` (`expected_separator`); `W0001` and the `pending_newline` machinery deleted** | Finalized in spec §4.2 (v2.3): `;` separates uniformly, removing cross-line ambiguity; no transition-period warning |

All remaining design (three-world architecture, Number tower, ExprPool, the three-level Config policy, module system, error model, parallelism philosophy, class ownership) is fully consistent with the spec.

---

*Implementation Plan Prima v2.3 · companion to SPECIFICATIONS-zh_CN.md v2.3 · the sole basis for implementation work*
