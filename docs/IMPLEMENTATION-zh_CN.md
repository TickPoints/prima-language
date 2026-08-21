# Prima 语言 —— 实现方案（Implementation Plan）v2.1

> **定位**：本文档是 [`SPECIFICATIONS-zh_CN.md`](./SPECIFICATIONS-zh_CN.md) v2.1 的实现落地决策。
> 规范未覆盖处，以本文档为准；规范 §19.1 的若干**初步建议**（logos/chumsky/latex crate）经评估后**不采纳**，理由见 §2 与 §7。
> 本文档的读者：实现者（含 AI 代理）。后续所有开发工作按本文档的分工与顺序推进。
> **v2.1 增量**：基础类型可用性增强（可变长 `Array`、`Dict`/`Set`、便捷函数、`print`/`println` 区分、`input`、推导式）进入语言规范，落地排期见 §5；Phase 3（`@parallel` 广播并行 + `parfor` + 符号微分 `derivative`/`partial`/`grad`/`limit`）在本文档 v2.1 中落地。

---

## 1. 技术选型总览

 领域 | 选型 | 版本 | 备选 | 决策要点
------|------|------|------|---------|
 词法分析 | **手写** | — | logos 0.16 | §2.1 |
 语法分析 | **手写递归下降 + Pratt 爬升** | — | chumsky 1.0-alpha / lalrpop 0.23 / pest 2.8 | §2.2，附录 A BNF 全量覆盖 |
 任意精度整数 | `num-bigint` | 0.5 | `rug` 1.30（GMP，LGPL，feature 化） | 规范 §21-30 已定；纯 Rust，MIT/Apache-2.0 |
 任意精度有理数 | `num-rational` | 0.4 | 同上 | 分数默认偏好（`fraction := true`） |
 复数 | `num-complex` | 0.4 | 同上 | §6.4 提升规则在 `Number` 层自实现 |
 数字通用 trait | `num-traits` | 0.2 | — | 泛型算法（绝对值、幂、逆） |
 BigFloat（可选） | `num-bigfloat` | 1.7 | `rug` | `to_bigfloat` 需要；纯 Rust 备选，rug 作加速 feature |
 并发 interner | `dashmap` | 6.x（7 仍 RC，暂不追） | — | §8.1 `global: DashMap<u64, ExprId>` |
 并行 | `rayon` | 1.12 | — | §17.2 `parfor`、`@parallel` 广播，规范点名 |
 矩阵 / 线性代数 | `nalgebra` | 0.35 | `faer` 0.24 | §12.4 点名；MVP 用 nalgebra，faer 留作性能替换（§6） |
 CLI | `clap`（derive） | 4.6 | — | §20 子命令 run/compile/repl/fmt/check/test/doc |
 错误类型 | `thiserror` | 2.0 | — | §16.1 `Error` 枚举直接 derive |
 诊断渲染 | `codespan-reporting` | 0.13 | `miette` | §16.4 即 rustc 风格（`error[E00xx]: ...` + `--> file:line:col` + 脱字符） |
 REPL | `rustyline` | 18 | `reedline` 0.49 | `prima repl` |
 Unicode 标识符 | `unicode-ident` | 1.0 | — | §三：标识符可含希腊字母等 Unicode |
 惰性全局 | std `OnceLock` / `thread_local!` | — | `once_cell` | 标准库已覆盖，不引额外依赖 |
 LaTeX / Unicode / ASCII 渲染 | **手写渲染器** | — | `latex` 0.3（文档排版库，不适用） | §7 内置符号独立于 TeX，渲染与 ExprDAG 强耦合，§2.3 |
 测试 | cargo test + `insta` 1.48（快照）+ `assert_cmd` 2.2（CLI）+ `proptest`（解析器属性测试） | — | — | 见 §5 每阶段验收 |
 基准 | `criterion` | 0.8 | — | 化简引擎、JIT 触发阈值调优 |
 绘图（stdlib 阶段） | `plotly` | 0.14 | 自绘 SVG | §十八 plot 模块，SVG 起步 |
 JIT（Phase 5） | `cranelift-codegen` | 最新稳定 | `inkwell`（LLVM） | §2.4 / §5 |

**工具链约束**：保持 **stable Rust，edition 2024**（Cargo.toml 已定），不依赖 nightly；不引入 build.rs 生成代码的解析器框架（无生成步骤）。

---

## 2. 解析器选型论证（本文档最重要的决策）

### 2.1 词法：手写，不用 logos

Prima 的 token 集在 v2.0 中约 40 类，但形状特殊：

- TeX 符号字面量 `\pi`、`\speed_of_light`（§7），与反斜杠转义易混淆；
- `tex"..."` 字面量（§三）内含任意 TeX 文本，不能按普通字符串切词；
- `@`（矩阵乘法）、`@.`（广播算子，§11.4）、`@parallel`/`@builtin`/`@c_api::extern` 注解（`@` 起始、`::` 参与路径）；
- `..`（区间与切片）、`..=`（含端区间，模式用）、`^`/`**` 别名、`?`（try 运算符）、`|>`（弃用管道）；
- 保留关键字（async/yield/macro/trait）需保留为 token 供未来使用；`impl` 在 `ops` 模块中生效。

手写 lexer 约 400–500 行，能对上述每一项给出**精确的 token 级错误与 span**（如未闭合字符串/TeX 字面量定位），并天然产出 `Token { kind, span }` 流。logos 的派生宏对自定义字面量与错误恢复控制较弱，收益（速度）在当前规模下无意义。

**v2.0 新增 token**：`;`（语句分隔，规范）、`?`（try）、`..=`（含端区间）、关键字 `class`/`self`/`Self`/`impl`/`match`、注解起始 `@`。换行不再作为**语法要求**的分隔符，仅在 `;` 缺失时作为**弃用分隔**处理（规范 §4.2，产出 `W0001` 警告）。

### 2.2 语法：手写递归下降 + Pratt 优先级爬升

**结论：Parser 手写**，理由：

1. **精确诊断是硬需求**。§16.4 要求「编号 + 文件:行:列 + 相关表达式 + 提示」，且编译期错误要**收集多个**而非 fail-fast。手写解析器对 span 与错误同步点（`;`、`}`、文件尾）有完全控制；chumsky 的恢复机制与自定义诊断格式对接成本高。
2. **语法有上下文敏感结构**，表驱动文法（lalrpop/pest）处理别扭：
   - `let f(x) = expr`（数学定义 §4.3）vs `let x = v`（变量绑定）vs `let (a, b) = v`（模式解构）——`let` 后需前瞻区分；
   - `config {}` / `import` 必须位于文件顶部（三区顺序），违反即报错；
   - 注解后置：`let f(x) @parallel = x^2`（§17.1）；`@c_api::extern` 中的 `::`；
   - 模式（§4.4）与表达式的歧义（`Some(x)` 是构造器调用还是模式？在模式上下文解析为模式）；
   - `with config { ... } { ... }`（§13.3 局部策略）。
   这些在递归下降里只是几个分支，在 LR/PEG 里需要大量消歧与语义谓词。
3. **增量演进**：§22 预留 macro/async/trait 语法；手写 parser 加规则、加错误恢复是局部改动，组合子/文法文件的重写成本高。rustc、Zig、Gleam 等均采用手写递归下降，是语言实现的主流做法。
4. **无生成步骤**：AST 类型即代码，无 build.rs、无过程宏，利于调试与 AI 维护。

**表达式解析**：Pratt（优先级爬升）。优先级表（低 → 高）：

| 级别 | 算子 | 结合性 | 备注 |
|------|------|--------|------|
| 1 | `\|>` 管道 | 左 | 弃用（W0002），降级为调用 |
| 2 | `\|\|` | 左 | |
| 3 | `&&` | 左 | |
| 4 | `==` `!=` `<` `<=` `>` `>=` | 左 | |
| 5 | `+` `-` | 左 | |
| 6 | `*` `/` `%` `@` `@.` | 左 | `@`=矩阵乘、`@.`=广播（§11.4） |
| 7 | 一元 `-` `!` `+` | 右 | |
| 8 | `^` `**` | 右 | 幂高于一元负号（数学惯例：`-x^2 = -(x^2)`，同 Julia） |
| 9 | 后缀：调用 `()`、索引 `[]`（含切片 `..`）、路径 `::`、方法 `.name()`、字段 `.name`、`?` | — | `?` 绑定于第 9 级，优先于二元运算 |

`^` 与 `**` 在解析层归一为同一个 BinOp 节点（别名，§三）。

**Parser 错误策略**：panic-mode + 同步 token 集（`;`、`}`、`)`、文件尾），一次编译收集全部语法错误。

**语句分隔策略**（§4.2）：
- 解析时以 `;` 为语句终止符。
- 当读到换行（`\n`）而未见 `;` 时：若下一 token 能合法开始一条新语句 → 记录一条 `W0001` 警告，按旧式换行分隔继续解析（兼容模式）；否则报 `E0011` 期望 `;`。
- 块级语句（`if`/`while`/`for`/`fn`/`class`/`match`/`with config` 后的 `{}`）结束后 `;` 可省略，不触发警告。

### 2.3 渲染：手写，不用 `latex` crate

crates.io 的 `latex` crate 是「程序化生成 LaTeX 文档/报告」的排版库，与本语言的符号渲染（ExprDAG → LaTeX/Unicode/ASCII 字符串）毫无关系。渲染器与 ExprDAG 结构强耦合（§8.4 规范形、§7 内置符号的 TeX 名、`print_format` 策略切换），必须手写三个后端实现同一 `Renderer` trait（§4.9）。该部分体量约 300–500 行，是 MVP 门槛之一（§19.1 里程碑 1）。

### 2.4 JIT 选型（Phase 5，预先定方向）

`cranelift-codegen` 优先于 `inkwell`：纯 Rust（无系统 LLVM 依赖）、编译速度快、API 稳定演进中；且与「查询式增量编译」（§19.3）兼容。LLVM（inkwell）仅当 AOT 需要深度优化与链接 BLAS 静态库时再评估。

---

## 3. 工作空间与 crate 划分

本仓库是 **Prima 工具链本体**（用户项目目录结构见规范 §20，与本仓库无关）。采用 Cargo workspace，依赖方向严格单向：

```
prima-language/                        # workspace 根 = 根包（CLI 二进制，bin 名 prima）
├── Cargo.toml                         # [package] + [workspace] 成员声明
├── src/main.rs                        # prima 二进制（clap 子命令）+ src/lib.rs 再导出
├── crates/
│   ├── prima-syntax/                  # 词法、语法、AST、Span/SourceMap、语法诊断
│   ├── prima-core/                    # Number 塔、Value、ExprPool/ExprId、化简引擎、Symbol 表、渲染器
│   ├── prima-jit/                     # cranelift JIT：数值标量 ExprDAG → 字节码 → 原生码（Phase 5）
│   ├── prima-runtime/                 # 解释器、模块系统、策略系统(Config)、内置函数、类型检查、符号微分、parfor、AD、jit 注册表
│   └── prima-stdlib/                  # linalg(nalgebra 桥)/stats/io/physics/plot 等显式导入模块
├── tests/                             # 集成测试（根）：词法/解析快照、core、CLI、proptest
├── benches/                           # criterion 基准（化简、ExprPool、数值层）
└── examples/                          # .pra 样例（CLI 集成测试用）
```

依赖关系：`syntax → core → runtime → stdlib`，禁止反向；CLI 在根包依赖全部 crate。理由：rustc 同构；crate 间编译隔离（JIT 引入后尤其重要）；`prima-syntax` 可独立被 fmt/check 复用；根包承载 CLI 使 `cargo run`/`cargo test` 在仓库根直接可用，且测试统一放根 `tests/`。
> **Phase 5 定稿**：依赖序扩展为 `syntax → core → prima-jit → runtime → stdlib`（`prima-jit` 只依赖 core/syntax；runtime 触发编译并持有 `CompiledScalar`）。

> 注：原计划 `crates/prima-cli` 已改为根包承载（2026-08 落地调整）；`src/main.rs` 原 Hello world 占位已替换为 clap CLI。

---

## 4. 核心数据结构定稿（规范 → 具体 Rust 类型）

### 4.1 Span 与诊断基础设施（prima-syntax）

```rust
pub struct Span { pub start: u32, pub end: u32 }        // byte 区间（u32 紧凑）
pub struct SourceLocation { pub file: Arc<PathBuf>, pub line: usize, pub column: usize }
// SourceMap: file → 源码 + LineIndex（字节偏移 ↔ 行:列，由 codespan-reporting 或自实现）
// 诊断管线：DiagnosticCollector —— 编译期收集型（多个错误一次报全）
```

§16.4 的 `error[E00xx]: ...` / `warning[W00xx]: ...` 格式由 `codespan-reporting` 渲染；错误码（§16.4/附录 C）作为诊断标题前缀（`E`/`R`/`W` + 四位编号）。`Error` 枚举（§16.1）用 `thiserror` derive，其中 `location` 字段在解释器抛错时由当前执行帧自动填充。

**警告收集**：`DiagnosticCollector` 同时收集错误与警告；警告不阻止编译，`prima check --deny W0001` 可将指定警告升级为错误（工具层）。

### 4.2 AST（prima-syntax）

**单一 AST 覆盖全部语法**，三区顺序（config → import → statement）在解析期校验：

```rust
pub struct Program { pub config: Option<ConfigBlock>, pub imports: Vec<Import>, pub stmts: Vec<Stmt> }

pub enum Stmt {
    Let { pat: Pattern, type_ann: Option<Type>, value: Expr, annotation: Option<Annotation> }, // let (a, b) = ...
    Const { name: Ident, type_ann: Type, value: Expr },
    FnDef { name: Ident, params: Vec<Param>, ret: Option<Type>, annotation: Option<Annotation>, body: Block },
    MathDef { name: Ident, params: Vec<Param>, ret: Option<Type>, annotation: Option<Annotation>, body: Expr },
    ClassDef { name: Ident, vis: Visibility, members: Vec<ClassMember> },
    Impl { op: ImplOp, target: Ident, members: Vec<FnDef> },            // ops::Add for T（§18.5）
    Expr(Expr),
    For { var: Ident, range: (Expr, Expr), step: Option<Expr>, body: Block },
    ParFor { /* 同 For */ },
    While { cond: Expr, body: Block },
    If { cond: Expr, then: Block, elifs: Vec<(Expr, Block)>, else_: Option<Block> },
    IfLet { pat: Pattern, value: Expr, then: Block, else_: Option<Block> },   // if let（§4.4）
    WhileLet { pat: Pattern, value: Expr, body: Block },                      // while let（§4.4）
    Match { scrutinee: Expr, arms: Vec<MatchArm> },                           // match（表达式；语句形态同）
    Return(Option<Expr>),
    WithConfig { entries: Vec<ConfigEntry>, body: Block },
    Pub(Box<Stmt>),
}

pub enum Visibility { Private, Module, Public }   // 无 / pub(mod) / pub（§15.2）

pub struct ClassMember {                          // §4.5
    pub vis: Visibility,
    pub kind: ClassMemberKind,
}
pub enum ClassMemberKind {
    Field { name: Ident, ty: Type },              // 字段
    Method { name: Ident, params: Vec<Param>, ret: Option<Type>, body: Block }, // 方法
    // 关联函数与普通方法同构；首个参数为 self 即方法
}

pub enum Param {
    Normal { name: Ident, ty: Option<Type> },
    Self_ { ty: Option<Type> },                   // self 参数（方法）
    MutSelf { ty: Option<Type> },                 // mut self（保留扩展）
}

pub enum Pattern {                                // §4.4 全模式
    Wildcard,                                     // _
    Binding { name: Ident },                      // x
    Literal(Literal),                             // 0 / "s" / true / \pi
    Tuple(Vec<Pattern>, bool /* .. 尾 */),        // (a, b, ..)
    Array(Vec<Pattern>, bool /* .. */),           // [x, ..]
    Struct { name: Ident, fields: Vec<FieldPattern>, rest: bool }, // Point { x, y: 0, .. }
    Variant { name: Ident, inner: Option<Box<Pattern>> },          // Some(x) / Ok(v) / None
    Range { lo: Literal, hi: Literal, inclusive: bool },           // 0..9 / 1..=5
    Or(Vec<Pattern>),                             // pat1 | pat2
    Group(Box<Pattern>),
}
pub struct FieldPattern { pub name: Ident, pub pat: Option<Pattern> }

pub struct MatchArm { pub pat: Pattern, pub guard: Option<Expr>, pub body: Expr }

pub enum Annotation {                             // @parallel / @jit / @gpu / @builtin / @c_api::extern
    Parallel, Jit, Gpu, Builtin, CApiExtern,
}

pub enum Expr {
    Literal(Literal),            // Integer/Float/Hex/Bin/String/Char/Bool/TexString
    Symbol(Ident),               // 含 \pi 形式的 TeX 名
    Self_,                       // self
    SelfType,                    // Self（类型位置出现于方法返回/字段）
    Path(Vec<Ident>),            // a::b::c（模块路径 / 限定访问）
    Call { f: Box<Expr>, args: Vec<Expr> },
    MethodCall { receiver: Box<Expr>, name: Ident, args: Vec<Expr> }, // obj.method(...)（§4.5）
    Index { base: Box<Expr>, index: Index },          // Index::Elem / Index::Slice(RangeExpr)
    Field { base: Box<Expr>, name: Ident },           // obj.field（类字段访问）
    StructLiteral { name: Ident, fields: Vec<FieldValue>, base: Option<Box<Expr>> }, // T { a, b } / T { ..base }
    Binary { op: BinOp, lhs: Box<Expr>, rhs: Box<Expr> },  // op 携带优先级、幂右结合
    Unary { op: UnOp, e: Box<Expr> },
    Try(Box<Expr>),              // expr?（§16.3）
    Array(Vec<Expr>),            // 可变长数组字面量（v2.1 元素任意值，§11.3）
    Dict(Vec<(Expr, Expr)>),     // v2.1 Dict 字面量 { k: v, ... }（§4.6）
    Set(Vec<Expr>),              // v2.1 Set 字面量 { a, b, ... }（§4.6）
    Comprehension {              // v2.1 推导式（§11.7）：
        frame: CompFrame,        //   Array/Dict/Set/Tuple 外框
        clauses: Vec<CompClause>,//   for x in iterable [if cond] 链
        body: Expr,              //   元素表达式（Dict 推导式元素为 (k, v) 对）
    },
    Tuple(Vec<Expr>),
    Lambda { params: Vec<Param>, body: Box<Expr> },   // |x| expr
    Match { scrutinee: Box<Expr>, arms: Vec<MatchArm> },  // match 表达式
    Pipeline { lhs: Box<Expr>, rhs: Box<Expr> },      // a |> f —— 弃用（W0002），降级时改写为 Call
}
pub struct FieldValue { pub name: Ident, pub value: Option<Expr> }
// CompFrame: 产出容器（Array | Dict | Set | Tuple）；CompClause: For{var, iter} | If{cond}（§4.6/11.7）
// 每个节点携带 Span；Block = Vec<Stmt>
```

**关键设计：数学表达式与宿主表达式共用同一 AST**（规范 §4.3：`math_expr := expr`）。「符号世界 / 数值世界」的区分**不在解析层**，而在**降级（lowering）层**：同一棵 AST 子树按上下文走两条路（§4.8）——这是「三世界架构」的落地缝隙，规范 §二 的示意图落位于此。

### 4.3 Number 塔与 Value（prima-core）

```rust
pub enum Number {
    Integer(BigInt),             // §6.1 精确；溢出升级由 num_to_big 策略决定
    Rational(BigRational),       // 自动约分、分母为正（§6.4 规则 3）
    Real(Real),                  // F32(f32) | F64(f64)；NaN/Inf 仅存在于此（§6.2）
    Complex(Box<Complex<Number>>), // 递归；re/im 归一到 Rational 或 Real
}

// v2.0：定宽坍缩数值类型（§6.1）——实现策略：折叠进 Number::Real 的 F32/F64 变体 + 新增定宽整型变体
pub enum Number {                // v2.0 定稿
    Integer(BigInt),             // 符号/精确层
    Rational(BigRational),
    Real(Real),                  // F32 | F64
    Complex(Box<Complex<Number>>),
    // —— 坍缩层（§6.1 与 Rust 一一对应）——
    I8(i8), I16(i16), I32(i32), I64(i64), I128(i128),
    U8(u8), U16(u16), U32(u32), U64(u64), U128(u128),
    Isize(isize), Usize(usize),
    BigFloat(BigFloat),
}

pub struct Complex<T> { pub re: T, pub im: T }   // 不直接依赖 num-complex 的类型，
                                                 // 但复用其 trait 实现（T: Num）辅助泛型运算
pub enum Value {  // §5 逐字落地
    Number(Number), Bool(bool), Char(char), String(String),
    Array(Array),            // v2.1：可变长序列，`Vec<Value>`（§11.3，见下方 v2.1 注）
    Dict(Dict),              // v2.1：`HashMap<ValueKey, Value>`（§11.6）
    Set(Set),                // v2.1：`HashSet<ValueKey>`（§11.6）
    Matrix(Matrix), Function(Function),
    Class(ClassId),            // §5 类实例句柄（§4.7）
    Expr(ExprId), Symbol(SymbolId),
    Option(Option<Box<Value>>), // §5 Option<T>：Some(T)/None
    Indeterminate(IndeterminateForm), Undefined, Error(Error), Nil,
    Tuple(Vec<Value>), Result(Result<Box<Value>, Error>),
}
```

> **v2.1 落地注**：`Value::Array` 由 v1.x 的 `Vec<Number>` 改为 `Vec<Value>`（元素可为任意值）；`Dict`/`Set` 为新增变体，键/元素用 `ValueKey` 包装（`Number`/`String`/`Char`/`Bool`/`Expr`/`Symbol` 的可哈希形式）。广播/矩阵接口在**调用点**校验数组元素为数值（`R0009`），不再于字面量构造层强制同构。`grad` 等返回多表达式的接口在 `Vec<Value>` 化完成前暂以 `Value::Tuple` 承载。

**提升（promotion）规则实现**（§6.4 定稿）：`promote(a, b) -> (Number, Number)` 把两个数抬到公共层——序列 `Integer < Rational < Complex<Rational> < F64 < Complex<F64>`；遇 `Real` 即整个 Complex 提升为 Complex<Real>。约分/规范化在 `Rational` 构造时完成（`num-rational` 原生支持）。

**定宽坍缩类型**：`I8/U8/.../F64` 只在**显式坍缩后**存在（§6.1），`promote` 不参与（坍缩后数值不参与隐式提升，定宽间转换需显式 `to_*`，§6.3）。数值算术在定宽层按 Rust 原生语义进行（溢出按 `checked_*` 报 `R0001`，`+` 等运算符在定宽层直接溢出为 Rust 语义）。

> 性能注记：MVP 直接 `BigInt`；`num-bigint` 在值较小时内部已有小整数优化，无需自定义小整数快速路径。若基准显示热点，再做 `i64` 内联标签。

### 4.4 ExprPool：hash-consing DAG（prima-core）

规范 §8.1 直接落地，改动仅一处：`ExprData` 中的大整数用 `Box` 装（规范示例已如此）：

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
    global: DashMap<u64, ExprId>,           // 内容哈希 → 已存在节点
    store: RwLock<Vec<ExprData>>,           // 中心存储（追加式，无删除：符号层无环常驻）
}
thread_local! { static LOCAL_CACHE: RefCell<HashMap<u64, ExprId>> }   // §8.1 线程本地缓存
```

**intern 流程**：计算节点内容哈希 `h = hash(ExprData)` → 查 `LOCAL_CACHE` → 未中查 `global` → 仍未中则分配新 `ExprId`、`store` 追加、写回两级缓存。相等即 `ExprId ==`（O(1)，§8.2）。

**规范形**（§8.4）：`Add`/`Mul` 子项按固定序（数字/常数 → 符号 → 复合节点，再按 (kind, id)）排序后 intern，保证 `x+1 ≡ 1+x`。

**化简分级**（§8.3）：
- intern 时只做 **等级 0/1** 的本地规则：`0*x→0`、`1*x→x`、`x+0→x`、同层数字合并（`2+3→5`）；
- **等级 2** 于数值上下文 / `simplify()` 触发（`sin(0)→0`、`2*5→10`）；
- **等级 3** 仅显式 `simplify()`（有理化、三角恒等、因式分解，Phase 3+ 逐步扩充规则库）；
- 打印/渲染始终用**等级 0**（保留原形态，除非先 `simplify()`，§8.3 默认策略）。

### 4.5 符号体系（prima-core）

```rust
pub struct SymbolId(u32);
enum SymbolKey { TeX(&'static str), Ident(String), Physics(&'static str) }  // 登记表 + 全局表
```

- **内置符号**（§7）：数学常数（`\e` `\pi` `\i` `\tau` `\infty` `\gamma` `\phi`）、算子（`\log` `\ln` `\exp` `\sqrt` `\sin` `\cos` `\tan` `\sigma` `\prod` `\int` `\partial`）、物理常数（§7.3 CODATA 2022，高精度存储，默认不坍缩）。物理常数值以 `BigRational` 或 `num-bigfloat` 存储，需 `to_f64(...)` 等显式坍缩。
- **名字解析与冲突**（§15.4）在 `prima-runtime` 模块表完成：`import` 将公开项登记进当前作用域，同名冲突报编译期错误。

### 4.6 策略系统 Config（prima-runtime）

```rust
pub struct Config {
    pub domain: Domain,                       // §6.5
    pub undefined_handling: UndefinedHandling,// strict | custom(HashMap<ExprKey, Value>)
    pub fraction: bool,                       // true
    pub broadcast: bool,                      // true
    pub loop_optimization: bool,              // true
    pub simplify_level: u8,                   // 0..=3，默认 2
    pub num_to_big: bool,                     // true
    pub print_format: PrintFormat,            // latex | unicode | ascii
    pub overload_policy: OverloadPolicy,      // warn | allow | deny（v2.0 新增，§13.2/18.5）
}
```

- **存储**：`thread_local! { RefCell<ConfigStack> }` —— 栈式模型：全局（入口文件）→ 模块（压栈）→ 局部 `with config`（临时压栈），出块弹栈，天然实现「局部 > 模块 > 全局」（§13.1）。
- **并行传播**：`parfor`/`@parallel` 任务创建时**快照 Config 传入 rayon 闭包**（工作线程的 thread_local 不继承），任务内局部策略在快照上再叠加。
- **污染性检查**（§13.2）：`domain`/`undefined_handling` 出现在非入口文件 → 编译期报错。

### 4.7 模块系统（prima-runtime）

- `FileResolver`：根模块 `src/main.pra`；`import` 解析按 §15.3 文件映射（`physics.pra` → 模块 `physics`；目录 → 子模块，`main.pra` 为入口）；import 环检测。
- `Module`：`{ items: HashMap<String, (Visibility, Value)>, path: Vec<Ident>, config: Config }`；**默认私有，`pub`/`pub(mod)` 公开**（§15.2）；模块间变量不互通。
- **类注册表**：`ClassRegistry`（进程级 `OnceLock`）：`ClassId → ClassDef { name, fields: Vec<(Ident, Type)>, methods: HashMap<Ident, MethodDef>, vis }`。类实例 `Value::Class(ClassId)` + 运行时 `Rc<RefCell<ClassInstance>>`（§12.3 引用计数）。
- 预导入 `core`（§15.5）：启动时把 core 全部公开项注入根作用域。
- 求值序：解析时**预扫描**所有可达模块（两遍：先收集符号表，再求值），使 `pub` 项在 import 后立即可解析；模块体按 import 依赖序求值。

### 4.8 求值模型（prima-runtime 解释器）

统一 AST 的**双重降级**（§4.2 的落点）：

```
expr_ast
 ├─ Symbolic 上下文（MFn 体、let 右值默认为符号）→ lower_to_expr() → ExprDAG → 化简 → Value::Expr(ExprId)
 └─ Host 上下文（fn 体、控制流条件/循环变量）      → eval() → Value（数值栈值 / 宿主对象）
```

- **MFn**（`let f(x) = body`）：闭包持有 body AST；调用时 `substitute(参数 → 实参 ExprId)` 得实例化 DAG → 化简 → 返回 `Expr`。实参是数值时按需坍缩（§10 例：`f(3.0) → 15.0`）。
- **Fn**（`fn`）：宿主闭包，解释执行 Block；可有副作用（§11.2）；可返回 `Result`（§16.3）。
- **广播**（§11.4）：调用点检查——参数为 `Array` 且 `broadcast := true` 时逐元素；**数组元素须为数值**（非数值 → `R0009`），**拒绝空数组**（`R0014`，§16 诊断）；`@.` 是显式广播算子；`broadcast := false` 时提供 `map`/`@.`。
- **集合与推导式**（v2.1，§4.6/11.6/11.7）：`Array`/`Dict`/`Set` 字面量求值为对应 `Value`；`for`/`parfor`/`in`/推导式统一迭代协议（`iter_values(v) -> Vec<Value>`：Array 元素、Dict 键、Set 元素、range、String 字符）；推导式求值 = 嵌套循环 + 过滤 + 收集到外框容器；`in` 二元运算在 Array 线性查找、Dict/Set O(1)。
- **可变集合方法**（v2.1，§11.3/11.6）：`v.push`/`pop`/`append`/`extend`/`insert`/`remove`/`clear`、`d[k]=v`/`get`/`insert`/`remove`/`keys`/`values`/`items`、`s.add`/`remove`/`discard`/`union`/`intersection`/`difference` 由 `eval_method_call` 对 `Value::Array/Dict/Set` 特判分发；切片赋值 `v[a..b] = [...]` 改写为 splice。
- **模式与解构**（§4.4）：`match`/`if let`/`while let`/`let` 统一走 `match_pattern(pattern, value) -> Result<Bindings, MatchFail>`；构造器模式（`Some`/`Ok`/`Err`）按内建变体匹配；`..` 通配其余。
- **类**（§4.5/12.3）：`Test::new(...)` 查类注册表关联函数；`obj.method(...)` 依 `Value::Class` 的 `ClassId` 查方法表；`self` 绑定为实例的 `Rc` 浅拷贝；字段访问 `obj.x` 按可见性检查；`T { a, b }` 构造新实例（缺字段 `E0061`，未知字段 `E0060`）。
- **`?` 运算符**（§16.3）：`expr?` 在返回 `Result` 的函数内：`Err(e)` → 提前返回 `Err(e)`；返回 `Option` 的函数内：`None` → 提前返回 `None`；上下文不匹配 → 编译期 `E0054`。
- **`|>` 管道**（§9.7）：弃用（W0002），降级改写为嵌套调用。
- **循环优化**（§10）：`loop_optimization := true` 时，`for i in a..b { acc += i }` 形态识别为闭式公式（Phase 2 实现，先 `0..n` 与 `1..n` 等差模式）。
- **parfor**（§17.2，v2.1 落地）：`rayon::par_iter`；迭代体**副作用静态检查**——仅允许对索引槽 `A[i]`/`A[i] +=` 赋值与纯函数调用，违规报 `E0082`；各槽位独立求值（每线程块一个独立 Evaluator，共享进程级 `ExprPool`/`SymbolTable`），结束后整数组回写绑定。
- **@parallel 广播并行**（§17.1/17.4，v2.1 落地）：`Function::User` 增加 `parallel: bool`（`MathDef` 带 `Annotation::Parallel` 时为真）；广播路径中当数组长度 ≥ 阈值（默认 1024）时按 `rayon` 线程数分块，每块一个**独立 Evaluator**（快照当前 Config，输出 sink 丢弃），块内对 `@parallel` MFn 的形参环境求值（要求函数体**自包含**，不引用自由变量）；小数组走顺序路径。
- **符号微分**（§19.4，v2.1 落地）：`crates/prima-runtime/src/diff.rs` 基于 `ExprPool` DAG 实现 `derivative`/`partial`/`grad`/`limit`；`eval_call` 拦截这四个名字，`derivative(f, x)`/`grad(f)` 接受 MFn 名（经 `resolve_func` 取函数体并绑定形参为符号）或符号表达式；`limit` 先直接代入，遇 0/0 用洛必达（最多 8 轮）。
- **控制台**（§18.1b，v2.1 落地）：`print` 不追加换行、`println` 追加换行（`Builtin::Print`/`Println` 分派差异）；`input`/`read_line` 读 stdin（EOF/错误返回空串）。
- **错误流**（§16.2/16.3）：可恢复错误以 `Value::Result` 表达；`to_*`/`unwrap`/`expect` 在解释器内转为**终止性 panic**（`panic!`，§16.2 第 3 类）。**不存在 `try/catch`**；解析 `try` 关键字即报 `E0010` 并提示改用 `Result`。
- **Undefined 严格性**（§6.2）：`Undefined` 参与任何一元/二元运算即抛 `UndefinedError`（不传播运算）；`Indeterminate` 只在符号层存在，坍缩失败时转 `Undefined`。

### 4.9 渲染器（prima-core）

```rust
trait Renderer { fn render_expr(&self, pool: &ExprPool, id: ExprId, out: &mut String); }
// 实现：LatexRenderer / UnicodeRenderer / AsciiRenderer，由 print_format 策略选择
```

- LaTeX 是 MVP 门槛（§19.1 里程碑 1）：`\sqrt{2} + \pi` 级输出；`tex"..."` 字面量解析为渲染树（内嵌小型 TeX 解析器，Phase 1 先支持 MVP 子集，逐步扩充）。
- 规范 §7 强调「内置符号独立于 TeX，TeX 仅是视图」：`ExprDAG → (LaTeX|Unicode|ASCII)` 是纯视图转换，反向（`tex"..."` → DAG）才是解析，二者解耦。
- `format`（§18.1）：模板解析 + 按 `print_format` 渲染参数，复用 `Renderer` 的 `render_to_string`。

### 4.10 错误模型汇总

| 类别 | 时机 | 形态 |
|------|------|------|
| 语法 / 类型 / 导入 / 可见性 / 污染性 | 编译期 | 收集型诊断（§16.2），编号 `E####`，codespan-reporting 渲染 |
| `Undefined` 参与运算 | 可静态判定则编译期，否则运行时 | `R0006`，以 `Result` 返回（不 panic） |
| 越界 / 维度 / I/O / 坍缩失败（`try_*`） | 运行时 | `R####`，以 `Result` 返回 |
| 显式放弃错误（`to_*` / `unwrap` / `expect`） | 运行时 | 终止性 panic（带跨语言堆栈） |
| 内部不可恢复错误 | 运行时 | panic（兜底） |

---

## 5. 实施路线图（Phase 0 → 5）

每个 Phase 结束都有可运行的验收命令。Phase 0–2 已按 v1.x 落地；v2.0 变更分布在各 Phase 的增量任务中，见各 Phase 的「v2.0 增量」小节。

### Phase 0：工程骨架 + 前端（syntax crate）

- workspace 建 4 个 lib crate + 根 CLI 包；CLI 用 clap 搭 `run`（其余子命令占位报「未实现」）。
- `SourceMap` / `Span` / 词法器（§2.1 全 token 集）→ 单测覆盖每个 token 类型与错误分支。
- 递归下降 Parser + Pratt（§2.2 优先级表），**覆盖附录 A BNF 全部产生式**。
- 诊断收集器：一处源码可报多个错误。
- **验收**：`cargo test` 通过；`insta` 快照测试（`tests/parsing/*.pra` → AST dump）；`proptest` 随机 token 序列不 panic 不挂起。

**v2.0 增量**：
- token：`;`、`?`、`..=`、`class`/`self`/`Self`/`impl`/`match`、注解 `@builtin`/`@c_api::extern`。
- 模式解析器（§4.4 全模式）+ `match`/`if let`/`while let`/`let` 解构。
- 语句分隔：`;` 规范 + 换行兼容（`W0001` 警告，§2.2）。
- Class 语法（字段/方法/`Self`/可见性）+ `impl ops::X for T`。
- 字符串转义全量（含 `\u{XXXX}`）+ `format` 函数签名（求值在 core/runtime）。

### Phase 1：MVP 符号引擎（core crate，对应 §19.1 里程碑 1–3）

- `Number`/`Value`/提升规则；`ExprPool` + 化简等级 0/1 + 规范形。
- 内置符号表（§7.1–7.2 常数与算子）；LaTeX 渲染器（等级 0 原形态）。
- 解释器最小集：`let`/`const`、MFn（符号替换求值）、数组字面量、`print`、一元/二元运算、`\pi` 等符号参与化简（`simplify(tex"\e^{i\pi}+1") → 0` 目标）。
- **验收**：
  ```
  prima run 里程碑样例：f(v) 广播 → [1, 4, 9]；
  tex"\sqrt{2}+\pi" 打印出 LaTeX；
  simplify(tex"\e^{i\pi}+1") → 0
  ```

> **Phase 1 落地记录（2026-08）**：全部完成，验收样例见 `examples/phase1.pra`。与本文档的偏差/定稿：
> - `ExprData` 增加了 `Real(Real)` 变体（规范 v1.0 §8.1 未列，为使浮点可进入符号 DAG）；`ExprData::Symbol(SymbolId)` 用 `core::symbol::SymbolId` 新类型。
> - 化简规则实现在 `core::simplify::simplify(pool, builtins, id)`（无 level 参数，MVP 全量应用）：intern 层（`ExprPool::add2/mul2/pow2/sub2/div2` + `add_n/mul_n` 扁平化/常量合并）做 0/1 级；`simplify` 做 2/3 级。
> - TeX 字面量解析器放在 `prima-syntax::tex`（MVP 子集），产出与普通语法相同的 AST。
> - 解释器在 `prima-runtime::eval`；广播在调用点对纯函数逐元素，拒空/嵌套数组；`a |> f` 管道改写为调用。
> - `print`/`println` 当时都换行；**v2.1 起区分**：`print` 不换行、`println` 换行（§4.8 控制台）；默认 LaTeX 输出。

### Phase 2：策略、数值层与错误处理（对应里程碑 4、5、7）

- Config 三级策略（§4.6）；`fraction := false` 生效；F64 不精确传染（§6.4）。
- 坍缩函数族（v1.0 规模）含 Result 包装、`unwrap` 家族。
- `fn` 宿主函数、`if`/`while`/`for step`/`return`（v1.0 含 `try-catch`，**v2.0 移除**）。
- 循环优化（等差闭式）；`Undefined` 严格性检查（§6.2）。
- 模块系统（§4.7）+ `pub`/`import`/冲突检测 + 预导入 core。
- **验收**：里程碑 4/5/7 样例通过；`prima check` 报 §16.4 格式的类型错误。

> **Phase 2 落地记录（2026-08，v1.x）**：`examples/config_fraction.pra`、`examples/loop_optimization.pra`、`examples/try_catch.pra`。与本文档的偏差/定稿：
> - 作用域实现为 `Rc<RefCell<Env>>` 共享链（`EnvRef`）。
> - `Value::Result`/`Value::Error` 以消息字符串承载错误（结构化 `Error` 枚举在 v2.0 补齐）。
> - `to_bigfloat` 为退化实现；`print_format` 仅 latex 渲染器可用。
> - `prima check` 先做字面量-注解级静态检查。
> - 循环优化先覆盖 `0..n` 与 `1..n`。

**v2.0 增量**：
- **移除 `try/catch`**：解析器删除 `try` 语句产生式；`examples/try_catch.pra` 改写为 `Result` + `match` 版本；`E0010` 对 `try` 关键字给出「改用 Result」提示。
- **语句分隔**：全仓示例/测试改用 `;`；新增 `W0001` 警告通道。
- **`?` 运算符**：求值器 `eval_try` 实现（§4.8）。
- **`Result`/`Option` 一等待遇**：`match`/`if let`/`unwrap` 家族、`Some/None/Ok/Err` 构造器模式。
- **坍缩族扩展**：`i8…u128/isize/usize` 全部 `to_*`/`try_*`/`checked_*`/`clamped_*`（§九全量）。
- **编号诊断**：`E`/`R`/`W` 码接入 `DiagnosticCollector` 与 `codespan-reporting` 标题（§16.4/附录 C）。

### Phase 3：并行与符号微分（对应里程碑 6 + §19.4 MVP）

- `@parallel` 注解 + `parfor`（rayon，Config 快照传播，§4.6）；副作用静态检查。
- 符号微分引擎：`derivative`/`partial`/`grad` 递归求导规则（§19.4 MVP）；`limit`（泰勒展开/洛必达起步）。
- **验收**：`derivative(f, x)` 对 `x^2 + sin(x)` 输出 `2x + cos(x)`；百万级 `@parallel` 广播验证加速（criterion 基准）。

**v2.0 增量**：
- 自动内联（§10.2）：`InlinePass` 在类型检查后、求值/代码生成前运行；启发式（MFn/无副作用 fn、规模阈值、非递归）由编译器内部判定，不暴露注解。
- 常量折叠/CSE/循环不变量提升：作为化简/求值旁路增量实现，与 `simplify_level` 协同。

> **Phase 3 落地记录（2026-08，v2.1）**：全部完成。与本文档的偏差/定稿：
> - `@parallel` 以 `Function::User.parallel` 标记；广播路径对 `parallel && len ≥ 1024` 走 rayon 分块（每块一个独立 `Evaluator`，快照 Config、丢弃输出）。要求 `@parallel` 函数体**自包含**（不引用自由变量）。
> - `parfor` 副作用静态检查在 `eval_stmt` 的 `ParFor` 分支执行（报 `E0082`）；允许索引槽赋值（`A[i] = …`/`+=`）与纯函数调用；各槽位独立求值后整数组回写绑定。
> - 符号微分在 `crates/prima-runtime/src/diff.rs`：`derivative`/`partial`（同一求导，一个变量）、`grad`（自动收集自由符号逐偏导，`Vec<Value>` 化完成前返回 `Value::Tuple`）、`limit`（直接代入 → 洛必达 ≤8 轮）；`eval_call` 拦截四个名字，接受 MFn 名或符号表达式。
> - `print`/`println` 区分（print 不换行）、`input`/`read_line` 一并落地。

### Phase 4：标准库与工具链

- `prima-stdlib`：`linalg`（nalgebra：Matrix 构造/运算/分解/求解）、`stats`、`io`（JSON/CSV）、`physics`（§7.3 常数）、`plot`（SVG）。
- CLI 补齐：`repl`（rustyline，续行检测）、`fmt`（AST printer 复用渲染器）、`check`（纯类型检查，支持 `--deny W####`）、`test`、`doc`。
- **验收**：附录 B 函数速查表逐项可用（对应模块 import 后 golden 测试）。

**v2.0 增量**：
- **`String` 类**（`@builtin`，§18.1）：全方法集（`push/insert/len/...`）+ `format`/`to_string`。
- **`sys`**（`path`/`env`/`os`）、**`time`**、**`num`** 模块（§18.2/18.3，附录 B.5）。
- **`ops`**（§18.5）：`impl ops::Add for T` 注册到运算符分派表；调用点按 `overload_policy` 决定 `W0005`/放行/报错。
- **`@builtin`**（§18.4）：内置注册表（`BuiltinRegistry`），签名绑定 + 可见性校验（`E0055`/`E0056`）。
- **`@c_api::extern`**（§18.4）：AST 级标注 + 类型校验（`E0071`/`E0072`）；MVP 先产出「导出清单」+ ABI 头文件骨架，实际二进制导出在 Phase 5 AOT 落地。

**v2.1 增量（基础类型可用性，排期于 Phase 4 + 后续增量）**：
- **`Array` 可变长化**：`Value::Array` → `Vec<Value>`；`push/pop/append/extend/insert/remove/clear`、切片赋值、负索引、`+`/`+=` 拼接、`in` 成员测试（§4.3/4.8）。
- **`Dict`/`Set` 变体**：`ValueKey` 可哈希包装；字面量/索引/方法/集合代数（`∪`/`∩`/`\`）；`R0012`/`R0013`/`R0014` 错误码接入。
- **推导式**：`ExprKind::Comprehension` 求值 + 统一迭代协议；BNF 见规范附录 A。
- **便捷函数**：`len/enumerate/zip/sorted/reversed/sum/prod/min/max/all/any/join/count/index/first/last`（core 预导入）。
- **控制台**：`print`（不换行）/`println`（换行）区分已随 Phase 3 落地；`input`/`read_line` 随 `print` 分派落地。

> **Phase 4 落地记录（2026-08，v2.1）**：全部完成。与本文档的偏差/定稿：
> - **stdlib 采用「嵌入式 `.pra` 签名模块 + `@builtin` 实现注册表」**（对齐规范 §18.4 的设计意图，ADR 见 §7）：每个模块是一份内嵌进二进制的 `.pra`，只声明带类型的 `@builtin pub fn` 签名（如 `linalg::determinant(M: Matrix<F64>) -> F64`）；Rust 侧按 `"模块::函数"` 键注册实现（`register_impl`），`.pra` 经 `register_module_source` 内嵌。`import <module>` 解析到内嵌源码并**按普通模块求值**（`collect_pub` 导出绑定到实现的 `Function::Native`），API 表面单一来源。
> - **模块解析优先级**：内嵌 stdlib 源码 → 宿主命名空间 → 本地文件；**注册的 stdlib 路径名保留**（类 Rust `std`，本地 `linalg.pra` 不能遮蔽 `import linalg`）。物理常数（纯数据，无逻辑）保持宿主命名空间 `NamespaceItem::Val`，是唯一不经 `.pra` 的模块。
> - **`@builtin` 绑定**：根模块按 core builtin 名（`Builtin::from_name`）；stdlib 模块内**先查实现注册表**（`"模块::名"`），未注册 → `E0055`（core builtin 不遮蔽模块内同名实现）。`@builtin` 函数名支持 `::` 路径（`Matrix::zeros`、`Duration::from_secs`）。
> - **`prima check` 调用点类型检查**：从内嵌签名表校验 stdlib 调用（`E0050` 参数个数/类型），重载（如 `stats::quantile` 数组/分布双形态）任一签名匹配即通过；`Value` 类型名作通配；未知类型不误报。参数少于声明的个数允许（可选尾参）。
> - **`@c_api::extern`**：`E0071`/`E0072` 静态校验；`prima compile --emit-headers` 从导出清单生成 C 头（`crate::capi`）。
> - **CLI 补齐**：`repl`（rustyline，括号续行）、`fmt`（AST printer，`-w`/`--check`）、`test`（默认跑 `examples/` 全部样例）、`doc`（定义清单 + `///` 注释）、`check --deny W####`（警告提升为错误）。
> - **偏差记录**：物理常数以 `physics::planck_const`（裸名）访问，规范 §7.3 的 `physics::\planck_const` TeX 名不作模块键；`import sys::path` 绑定**全路径**（§15.3 约定，`sys::path::join`），规范 §18.2 示例的 `path::join` 简写不支持；`String.split` 返回 `Array<String>`（§18.1 v2.1）；`linalg::norm/solve/lstsq` 首参或 RHS 用 `Value` 通配以覆盖向量/矩阵双形态。

### Phase 5：JIT（§19.2）

- `cranelift-codegen` 热点编译：调用计数阈值触发（默认 100）或 `@jit` 注解；`ExprDAG → 字节码 → cranelift IR → 原生码`；符号层保持解释。
- AD 前向（Dual）与反向（Tape）模式（§19.4 第二、三阶段）；`jit(grad(f))` 组合。
- 优化管道接入（§10.2 全量）：常量折叠、CSE、循环优化、自动内联、TCO、DCE。
- C ABI 导出（§18.4）：`--emit-c-abi` 生成动态库 + 头文件。
- **验收**：`f(to_f64(101))` 走原生路径；criterion 对比解释/编译耗时，阈值调优。

> **Phase 5 落地记录（2026-08）**：全部完成。与本文档的偏差/定稿：
> - **新 crate `prima-jit`**（依赖方向 `syntax → core → prima-jit → runtime`）：`ExprDAG → Bytecode → cranelift IR → 原生码`。字节码是纯 `f64` 栈机（`Const`/`Param`/四则/`Pow`/超越函数）；**超越函数不依赖 cranelift 的 libcall 符号名解析**，而是经 `#[unsafe(no_mangle)] extern "C"` trampoline（`pj_sin/pj_cos/...`）在 `JITBuilder::symbol` 注册后由生成码直接 `call`。`CompiledScalar::call` 无锁线程安全；编译在进程级 `OnceLock<Mutex<JitEngine>>` 下串行。cranelift 0.135 注意：无 `frem`（`Rem` 走 `pj_rem`）、`MemFlagsData`/`Offset32` 路径、`JITModule::new` 需 `is_pic=false`。
> - **自动热点编译**：`Function::User` 增加 `hot: Arc<HotState>`（`force` + `AtomicU64` 调用计数 + `OnceLock<Option<Arc<CompiledScalar>>>`，克隆共享）。实参全为非复数 `Number` 时走热点：`@jit`（`MathDef` 注解）首调用即编译；否则第 `JIT_CALL_THRESHOLD`（默认 100）次调用触发编译并原生返回，失败结果缓存（不重试）；非数值实参回落到解释路径。语义不变（原生与解释结果一致）。
> - **`jit(...)` 内建**（`Builtin::Jit`，`eval_call` 拦截）：接受 MFn 名 → 编译前向标量；`jit(grad(f))` → **反向模式**（`ad::Tape`）多变量梯度（`Value::Array`）；裸符号表达式 → 以自由符号为参数编译；`grad` 的符号元组 → 逐分量数值求值。产物是 **`Value::JitFunction(u32)`**（`prima-core` 新变体，句柄指向 `runtime::jit` 进程级注册表，含 `compiled`/`tape`/`expressions`/`fallback` 多形态，编译不可用时自动回落）；调用点在 `eval_call` 支持该值作被调对象。
> - **AD**（`crates/prima-runtime/src/ad.rs`）：前向 `Dual`（`forward_derivative`）+ 反向 `Tape`（post-order DFS + memo 建图，`grad(inputs)` 一次反向传播得全偏导，支持 `Pow` 对数导数链式、内置常数）。反向 Tape 是 `jit(grad(f))` 的运行引擎。
> - **优化管道**（§10.2）：`core::opt`（`const_fold`=simplify、`cse`=hash-consing 天然共享、`optimize`）；`runtime::opt`（`tail_call_of` 纯 AST 尾调用分析：末语句直接 `return f(args)` 且前置语句 effect-free）；解释器 `Function::Host` 分支以 trampoline 实现 **TCO**（常量栈空间跑 10 万层尾递归；前置 `if` 内提前 `return` 正确退出）。自动内联=MFn 替换天然内联；循环优化沿用 Phase 2；标量字节码无分支故 DCE 为空（常量折叠已消死）。
> - **C ABI 导出**（`--emit-c-abi`）：按维护者决策采用 **cdylib 壳工程**——解析后收集 `@c_api::extern` 导出清单，生成头文件（`-o` 基名 + `.h`），并在临时目录生成 `cdylib` 壳 crate（绝对路径依赖 `prima-runtime`/`prima-core`，内嵌源文件绝对路径，每个导出一个 `#[unsafe(no_mangle)] extern "C"` 包装函数，`call_file_export` 线程本地缓存求值后按 C 类型转换），`cargo build --release` 产出 `.so`/`.dylib`/`.dll`。运行时需 cargo；`--emit-headers` 仍为离线路径。ctypes 实测 `add(2.5,3.0)=5.5`、`hello("world")` 往返成功。
> - **criterion 基准**（`benches/bench_jit.rs`）：同一 `x^4 + sin(x)·x + exp(x)` DAG，解释递归 ~373ns vs 原生 ~34ns（约 11× 加速），`f(101)` 双路结果一致——即验收 `f(to_f64(101))` 走原生路径。

> AOT（§19.3，WASM/独立可执行）不在本路线图内，待 Phase 5 完成后再立项评估。

---

## 6. 风险与备选方案

| 风险 | 缓解 |
|------|------|
| 手写 parser 覆盖不全 | 附录 A BNF 是验收清单；insta 快照 + proptest 持续补盲 |
| 模式解析歧义（构造器 vs 调用） | 模式上下文单独解析器函数 `parse_pattern`，与表达式解析隔离；快照覆盖 `Some(x)`/`Ok(v)`/嵌套 |
| 换行兼容解析的误判 | `W0001` 触发条件 = 换行后 token 可合法开始新语句；proptest 断言不误报 `E0011` |
| 化简规则库膨胀（等级 3） | 规则表驱动（`Vec<(Pattern, Rewrite)>`），不写进控制流；等级 3 推迟到 Phase 3+ |
| 类实例所有权（浅/深拷贝）语义复杂 | 统一 `Rc<RefCell<ClassInstance>>` + 字段值按基本值/类实例分派拷贝（§12.3）；方法参数/返回的拷贝语义做专门集成测试 |
| `?` 传播的上下文校验遗漏 | 静态检查 `?` 所在函数返回类型；`E0054` 在 check 阶段全量覆盖 |
| `num-bigint` 性能不达标 | `rug`（GMP）feature flag 替换底层，`Number` 封装层已隔离（§21 决策 30） |
| `nalgebra` 性能不足 | `faer` 0.24 作为替换后端，stdlib 层 trait 化（`MatrixBackend`） |
| dashmap 7.0 RC 不稳 | 锁 6.x；7 正式发布后评估升级 |
| ExprId 不可跨进程序列化 | hash-consing 的 Id 依赖进程内创建顺序 → **禁止缓存/序列化 ExprId**；查询式增量编译（§19.3）只缓存「可重放输入」，文档化约束 |
| rayon 工作线程的 thread_local 策略漂移 | 任务创建时快照 Config（§4.6），集成测试覆盖 parfor 内策略 |

---

## 7. 与规范冲突的决策记录（ADR 摘要）

| 规范条款 | 规范建议 | 本方案 | 理由 |
|---------|---------|--------|------|
| §19.1 | 词法用 `logos` | 手写 lexer | §2.1：token 形状特殊，错误定位优先 |
| §19.1 | 语法用 `chumsky` | 手写递归下降 + Pratt | §2.2：诊断精确性、上下文敏感语法、增量演进 |
| §19.1 | LaTeX 输出用 `latex` crate | 手写渲染器（LaTeX/Unicode/ASCII） | §2.3：该 crate 是文档排版库，与符号渲染无关 |
| §19.1 | nalgebra 或 faer | MVP 用 nalgebra，faer 为替换后端 | API 成熟度与文档，§6 风险表留切换口 |
| §19.2 | inkwell（LLVM）或 cranelift | 优先 cranelift | 纯 Rust、无系统依赖、编译快（§2.4） |
| §19.2 | 阈值触发 JIT | 默认阈值 100 次 + `@jit` 注解 | 与规范一致，仅将「如 100 次」定为默认值 |

**v2.0 ADR 新增**：

| 规范条款 | 规范建议 | 本方案 | 理由 |
|---------|---------|--------|------|
| §16.3（v1.0） | `try/catch` 错误处理 | **移除**，`Result` + `?` + `match` | 规范 §16.3（v2.0）定稿：错误是值；`?` 传播、`unwrap` 家族显式兜底；避免隐式异常流对符号求值的干扰 |
| §4.2（v1.0） | 换行分隔语句 | **`;` 规范，换行弃用（W0001）** | 规范 §4.2（v2.0）定稿：与 Rust 对齐、消除跨行歧义；过渡期警告并逐步移除 |
| §9.7（v1.0） | `\|>` 管道组合 | **弃用（W0002），由类方法链取代** | 规范 §9.7/4.5（v2.0）定稿：方法与链式调用表达力更强，避免多层管道可读性下降 |
| §6.1（v1.0） | 坍缩类型 I32/F32/F64 | **扩展为 i8…u128/isize/usize/f32/f64 全量** | 规范 §6.1（v2.0）定稿：与 Rust 基本数值一一对应，互操作与数值控制更细 |
| §18（v1.0） | stdlib 模块集 | **新增 sys/time/num/ops/c_api** | 规范 §十八（v2.0）定稿：系统层/时间/数值扩展/运算符重载/互操作 |

**v2.1 ADR 新增**：

| 规范条款 | 规范建议 | 本方案 | 理由 |
|---------|---------|--------|------|
| §5/§11.3（v2.0） | `Value::Array(Vec<Number>)`，同构、拒嵌套 | **`Vec<Value>` 可变长，可嵌套作数据；广播在调用点校验数值同质（R0009）** | 规范 §11.3（v2.1）定稿：Python 式可用性优先；符号层/数值层不变量仍在广播与矩阵接口处保持 |
| §5（v2.0） | 无 Dict/Set | **新增 `Dict`/`Set` 变体与 `ValueKey`** | 规范 §4.6/11.6（v2.1）定稿：映射/集合是科学计算高频需求 |
| §17.1（v2.0） | `@parallel` 无自包含要求 | **要求函数体自包含（不引用自由变量）** | 并行子任务各自求值、无共享环境；违反在求值时以未定义名报错，文档化约束 |
| §17.2（v2.0） | parfor 副作用检查时机未定 | **求值期静态检查，报 `E0082`** | 与 `prima check` 的增量检查并存；Phase 4 后移入编译期 |
| §19.4（v2.0） | `derivative(f, var)` 需函数值 | **`eval_call` 拦截 + 接受 MFn 名/表达式** | 当前 `Value` 无 `Function` 变体、函数不可作值；拦截方案支持 `derivative(f, x)` 且不扩张值系统 |
| §18（v2.1） | stdlib 模块为 Rust 命名空间 | **嵌入式 `.pra` 签名模块 + `@builtin` 实现注册表** | API 表面单一来源、`prima check` 调用点类型检查（E0050）、错误反馈与文档一体；物理常数（纯数据）例外保留 `Val` 命名空间 |
| §18.4（v2.1） | `@builtin` 按名称绑定内置 | **模块内先查实现注册表（`"模块::名"`），core builtin 只在根模块按名绑定** | 避免 `sys::path::join` 等被同名 core 便捷函数遮蔽 |
| §15.3（v2.1） | 模块解析仅文件映射 | **解析顺序：内嵌 stdlib → 宿主命名空间 → 本地文件；stdlib 路径名保留** | 确定性、类 Rust `std`；本地同名文件不再遮蔽内建模块 |
| §18.2（v2.1） | `import sys::path` 后以 `path::join` 访问 | **绑定全路径 `sys::path::join`** | 与 §15.3 嵌套导入约定一致（`import linalg::fft` → `linalg::fft::double`）；§18.2 示例的简写不支持 |
| §7.3（v2.1） | 物理常数以 `\planck_const` 符号名访问 | **模块键用裸名 `physics::planck_const`** | 注册表键是纯字符串；TeX 名仅作显示层概念 |

**v2.1 Phase 5 ADR 新增**：

| 规范条款 | 规范建议 | 本方案 | 理由 |
|---------|---------|--------|------|
| §19.3（v2.0） | C ABI 导出（`--emit-c-abi`）直接生成动态库 | **cdylib 壳工程**：生成内嵌源文件绝对路径的 `cdylib` crate（`#[no_mangle] extern "C"` 包装经 `call_file_export` 走解释器），用 cargo 编译产出 `.so/.dylib/.dll` | 导出目标是任意控制流的 `pub fn`（`print`/字符串/分支），纯 cranelift 无法直接编译；壳工程复用完整语言语义、跨平台可靠；要求运行时 cargo，`--emit-headers` 保留离线路径 |
| §19.4（v2.0） | `grad`/`jit` 组合需函数作值 | **新增 `Value::JitFunction(u32)` 句柄 + runtime `jit` 进程级注册表** | `jit(grad(f))` 返回可调用值；句柄模式与 `Value::Class` 一致，不引入 `Function` 值变体；编译不可用时回落解释 |
| §19.2（v2.0） | JIT 触发阈值「如 100 次」 | **默认 `JIT_CALL_THRESHOLD = 100`，第 100 次数值调用触发编译** | 与规范 §19.2 示例对齐：`for i in 1..100` 预热后 `f(to_f64(101))` 走原生路径 |

其余所有设计（三世界架构、Number 塔、ExprPool、策略三级、模块系统、错误模型、并行哲学、类所有权）与规范完全一致。

---

*实现方案 Prima v2.1 · 与 SPECIFICATIONS-zh_CN.md v2.1 配套 · 实现工作的唯一依据*