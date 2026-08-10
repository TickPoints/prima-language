# Prima 语言 —— 实现方案（Implementation Plan）v1.0

> **定位**：本文档是 [`SPECIFICATIONS-zh_CN.md`](./SPECIFICATIONS-zh_CN.md) 的实现落地决策。
> 规范未覆盖处，以本文档为准；规范 §19.1 的若干**初步建议**（logos/chumsky/latex crate）经评估后**不采纳**，理由见 §2 与 §7。
> 本文档的读者：实现者（含 AI 代理）。后续所有开发工作按本文档的分工与顺序推进。

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
 诊断渲染 | `codespan-reporting` | 0.13 | `miette` | §16.4 即 rustc 风格（`--> file:line:col` + 脱字符），与它逐字匹配 |
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

Prima 的 token 集很小（约 30 类），但形状特殊：

- TeX 符号字面量 `\pi`、`\speed_of_light`（§7），与反斜杠转义易混淆；
- `tex"..."` 字面量（§三）内含任意 TeX 文本，不能按普通字符串切词；
- `@`（矩阵乘法）、`@.`（广播算子，§11.4）、`|>`（管道，§9.7）这类多字符/复合算子；
- `..`（区间与切片）、`^`/`**` 别名、保留关键字（async/yield/macro/trait/impl）需保留为 token 供未来使用。

手写 lexer 约 300–400 行，能对上述每一项给出**精确的 token 级错误与 span**（如未闭合字符串/TeX 字面量定位），并天然产出 `Token { kind, span }` 流。logos 的派生宏对自定义字面量与错误恢复控制较弱，收益（速度）在当前规模下无意义。

### 2.2 语法：手写递归下降 + Pratt 优先级爬升

**结论：Parser 手写**，理由：

1. **精确诊断是硬需求**。§16.4 要求「文件:行:列 + 相关表达式 + 提示」，且编译期错误要**收集多个**而非 fail-fast。手写解析器对 span 与错误同步点（`;`、`}`、文件尾）有完全控制；chumsky 的恢复机制与自定义诊断格式对接成本高。
2. **语法有上下文敏感结构**，表驱动文法（lalrpop/pest）处理别扭：
   - `let f(x) = expr`（数学定义 §4.2）vs `let x = v`（变量绑定）——`let` 后跟 `ident (` 时是函数定义；
   - `config {}` / `import` 必须位于文件顶部（三区顺序），违反即报错；
   - 注解后置：`let f(x) @parallel = x^2`（§17.1）；
   - `with config { ... } { ... }`（§13.3 局部策略）。
   这些在递归下降里只是几个分支，在 LR/PEG 里需要大量消歧与语义谓词。
3. **增量演进**：§22 预留 macro/async/trait/impl 语法；手写 parser 加规则、加错误恢复是局部改动，组合子/文法文件的重写成本高。rustc、Zig、Gleam 等均采用手写递归下降，是语言实现的主流做法。
4. **无生成步骤**：AST 类型即代码，无 build.rs、无过程宏，利于调试与 AI 维护。

**表达式解析**：Pratt（优先级爬升）。优先级表（低 → 高）：

| 级别 | 算子 | 结合性 | 备注 |
|------|------|--------|------|
| 1 | `\|>` 管道 | 左 | `a \|> f \|> g` |
| 2 | `\|\|` | 左 | |
| 3 | `&&` | 左 | |
| 4 | `==` `!=` `<` `<=` `>` `>=` | 左 | |
| 5 | `+` `-` | 左 | |
| 6 | `*` `/` `%` `@` `@.` | 左 | `@`=矩阵乘、`@.`=广播（§11.4） |
| 7 | 一元 `-` `!` `+` | 右 | |
| 8 | `^` `**` | 右 | 幂高于一元负号（数学惯例：`-x^2 = -(x^2)`，同 Julia） |
| 9 | 后缀：调用 `()`、索引 `[]`（含切片 `..`）、路径 `::` | — | |

`^` 与 `**` 在解析层归一为同一个 BinOp 节点（别名，§三）。

**Parser 错误策略**：panic-mode + 同步 token 集（`;`、`}`、`)`、文件尾），一次编译收集全部语法错误。

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
│   ├── prima-runtime/                 # 解释器、模块系统、策略系统(Config)、内置函数、类型检查、符号微分、parfor
│   └── prima-stdlib/                  # linalg(nalgebra 桥)/stats/io/physics/plot 等显式导入模块
├── tests/                             # 集成测试（根）：词法/解析快照、core、CLI、proptest
├── benches/                           # criterion 基准（化简、ExprPool、数值层）
└── examples/                          # .pra 样例（CLI 集成测试用）
```

依赖关系：`syntax → core → runtime → stdlib`，禁止反向；CLI 在根包依赖全部 crate。理由：rustc 同构；crate 间编译隔离（JIT 引入后尤其重要）；`prima-syntax` 可独立被 fmt/check 复用；根包承载 CLI 使 `cargo run`/`cargo test` 在仓库根直接可用，且测试统一放根 `tests/`。

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

§16.4 的 `--> src/main.pra:15:9` 格式由 `codespan-reporting` 渲染；`Error` 枚举（§16.1）用 `thiserror` derive，其中 `location` 字段在解释器抛错时由当前执行帧自动填充。

### 4.2 AST（prima-syntax）

**单一 AST 覆盖全部语法**，三区顺序（config → import → statement）在解析期校验：

```rust
pub struct Program { pub config: Option<ConfigBlock>, pub imports: Vec<Import>, pub stmts: Vec<Stmt> }

pub enum Stmt {
    Let { name: Ident, type_ann: Option<Type>, value: Expr, annotation: Option<Annotation> },
    Const { name: Ident, type_ann: Type, value: Expr },
    FnDef { name: Ident, params: Vec<Param>, ret: Option<Type>, body: Block },     // fn
    MathDef { name: Ident, params: Vec<Param>, ret: Option<Type>, body: Expr },    // let f(x) = ...
    Expr(Expr),
    For { var: Ident, range: (Expr, Expr), step: Option<Expr>, body: Block },
    ParFor { /* 同 For */ },
    While { cond: Expr, body: Block },
    If { cond: Expr, then: Block, elifs: Vec<(Expr, Block)>, else_: Option<Block> },
    Return(Option<Expr>),
    Try { body: Block, catches: Vec<(Ident, Option<Type>, Block)> },
    WithConfig { entries: Vec<ConfigEntry>, body: Block },
    Pub(Box<Stmt>),
}

pub enum Expr {
    Literal(Literal),            // Integer/Float/Hex/Bin/String/Char/Bool/TexString
    Symbol(Ident),               // 含 \pi 形式的 TeX 名
    Path(Vec<Ident>),            // a::b::c（模块路径 / 限定访问）
    Call { f: Box<Expr>, args: Vec<Expr> },
    Index { base: Box<Expr>, index: Index },          // Index::Elem / Index::Slice(RangeExpr)
    Binary { op: BinOp, lhs: Box<Expr>, rhs: Box<Expr> },  // op 携带优先级、幂右结合
    Unary { op: UnOp, e: Box<Expr> },
    Array(Vec<Expr>),            // 嵌套数组在类型/求值层拒绝（§11.4）
    Tuple(Vec<Expr>),
    Lambda { params: Vec<Param>, body: Box<Expr> },   // |x| expr
    Match { scrutinee: Box<Expr>, arms: Vec<(Pattern, Expr)> },
    Pipeline { lhs: Box<Expr>, rhs: Box<Expr> },      // a |> f —— 解析为 AST 节点，降级时改写为 Call
}
// 每个节点携带 Span；Block = Vec<Stmt>
```

**关键设计：数学表达式与宿主表达式共用同一 AST**（规范 §4.2：`math_expr := expr`）。「符号世界 / 数值世界」的区分**不在解析层**，而在**降级（lowering）层**：同一棵 AST 子树按上下文走两条路（§4.8）——这是「三世界架构」的落地缝隙，规范 §二 的示意图落位于此。

### 4.3 Number 塔与 Value（prima-core）

```rust
pub enum Number {
    Integer(BigInt),             // §6.1 精确；溢出升级由 num_to_big 策略决定
    Rational(BigRational),       // 自动约分、分母为正（§6.4 规则 3）
    Real(Real),                  // F32(f32) | F64(f64)；NaN/Inf 仅存在于此（§6.2）
    Complex(Box<Complex<Number>>), // 递归；re/im 归一到 Rational 或 Real
}

pub struct Complex<T> { pub re: T, pub im: T }   // 不直接依赖 num-complex 的类型，
                                                 // 但复用其 trait 实现（T: Num）辅助泛型运算
pub enum Value {  // §5 逐字落地
    Number(Number), Bool(bool), Char(char), String(String),
    Array(Array), Matrix(Matrix), Function(Function),
    Expr(ExprId), Symbol(SymbolId),
    Indeterminate(IndeterminateForm), Undefined, Error(Error), Nil,
    Tuple(Vec<Value>), Result(Result<Box<Value>, Error>),
}
```

**提升（promotion）规则实现**（§6.4 定稿）：`promote(a, b) -> (Number, Number)` 把两个数抬到公共层——序列 `Integer < Rational < Complex<Rational> < F64 < Complex<F64>`；遇 `Real` 即整个 Complex 提升为 Complex<Real>。约分/规范化在 `Rational` 构造时完成（`num-rational` 原生支持）。

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
}
```

- **存储**：`thread_local! { RefCell<ConfigStack> }` —— 栈式模型：全局（入口文件）→ 模块（压栈）→ 局部 `with config`（临时压栈），出块弹栈，天然实现「局部 > 模块 > 全局」（§13.1）。
- **并行传播**：`parfor`/`@parallel` 任务创建时**快照 Config 传入 rayon 闭包**（工作线程的 thread_local 不继承），任务内局部策略在快照上再叠加。
- **污染性检查**（§13.2）：`domain`/`undefined_handling` 出现在非入口文件 → 编译期报错。

### 4.7 模块系统（prima-runtime）

- `FileResolver`：根模块 `src/main.pra`；`import` 解析按 §15.3 文件映射（`physics.pra` → 模块 `physics`；目录 → 子模块，`main.pra` 为入口）；import 环检测。
- `Module`：`{ items: HashMap<String, (Visibility, Value)>, path: Vec<Ident>, config: Config }`；**默认私有，`pub` 公开**（§15.2）；模块间变量不互通。
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
- **Fn**（`fn`）：宿主闭包，解释执行 Block；可有副作用（§11.2）。
- **广播**（§11.4）：调用点检查——参数为 `Array` 且 `broadcast := true` 时逐元素；**拒绝嵌套数组与空数组**（错误码，含 §16 诊断）；`@.` 是显式广播算子；`broadcast := false` 时提供 `map`/`@.`。
- **`|>` 管道**（§9.7）：降级改写为嵌套调用。
- **循环优化**（§10）：`loop_optimization := true` 时，`for i in a..b { acc += i }` 形态识别为闭式公式（Phase 2 实现，先 `0..n` 与 `1..n` 等差模式）。
- **parfor**（§17.2）：`rayon::par_iter`；迭代体**副作用静态检查**（仅允许对索引槽 `A[i]` 赋值、纯函数调用），违规编译期报错。
- **try/catch 与错误流**（§16.3）：运行时 `Error` 以 `Result` 在解释器帧间传播（不用 `catch_unwind` 包装——panic 只留给「兜底 panic」，§16.2 第 4 类）。`try { } catch e { }` 由解释器捕获传播的 `Error` 绑定到 `e`，支持 `catch e: Error::Overflow` 类型分支（§16.3 示例）。
- **Undefined 严格性**（§6.2）：`Undefined` 参与任何一元/二元运算即抛 `UndefinedError`（不传播运算）；`Indeterminate` 只在符号层存在，坍缩失败时转 `Undefined`。

### 4.9 渲染器（prima-core）

```rust
trait Renderer { fn render_expr(&self, pool: &ExprPool, id: ExprId, out: &mut String); }
// 实现：LatexRenderer / UnicodeRenderer / AsciiRenderer，由 print_format 策略选择
```

- LaTeX 是 MVP 门槛（§19.1 里程碑 1）：`\sqrt{2} + \pi` 级输出；`tex"..."` 字面量解析为渲染树（内嵌小型 TeX 解析器，Phase 1 先支持 MVP 子集，逐步扩充）。
- 规范 §7 强调「内置符号独立于 TeX，TeX 仅是视图」：`ExprDAG → (LaTeX|Unicode|ASCII)` 是纯视图转换，反向（`tex"..."` → DAG）才是解析，二者解耦。

### 4.10 错误模型汇总

| 类别 | 时机 | 形态 |
|------|------|------|
| 语法 / 类型 / 污染性 / 导入冲突 | 编译期 | 收集型诊断（§16.2），codespan-reporting 渲染 |
| `Undefined` 参与运算 | 可静态判定则编译期，否则运行时 | 结构化 Error |
| 越界 / 维度 / I/O | 运行时 | Error，可 try/catch |
| 基础坍缩失败（`to_i32` 溢出等） | 运行时 | panic → try/catch 捕获（Error 传播）；不可捕获 panic 仅限内部错误 |

---

## 5. 实施路线图（Phase 0 → 5）

每个 Phase 结束都有可运行的验收命令。

### Phase 0：工程骨架 + 前端（syntax crate）

- workspace 建 4 个 lib crate + 根 CLI 包；CLI 用 clap 搭 `run`（其余子命令占位报「未实现」）。
- `SourceMap` / `Span` / 词法器（§2.1 全 token 集）→ 单测覆盖每个 token 类型与错误分支。
- 递归下降 Parser + Pratt（§2.2 优先级表），**覆盖附录 A BNF 全部产生式**（含 lambda、match、pipeline、`@parallel` 注解、`with config`、range/step、切片索引）。
- 诊断收集器：一处源码可报多个错误。
- **验收**：`cargo test` 通过；`insta` 快照测试（`tests/parsing/*.pra` → AST dump）；`proptest` 随机 token 序列不 panic 不挂起。

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
> - `ExprData` 增加了 `Real(Real)` 变体（规范 §8.1 未列，为使浮点可进入符号 DAG）；`ExprData::Symbol(SymbolId)` 用 `core::symbol::SymbolId` 新类型。
> - 化简规则实现在 `core::simplify::simplify(pool, builtins, id)`（无 level 参数，MVP 全量应用）：intern 层（`ExprPool::add2/mul2/pow2/sub2/div2` + `add_n/mul_n` 扁平化/常量合并）做 0/1 级；`simplify` 做 2/3 级（`Pow(sqrt(x),2)→x`、欧拉 `e^{iθ}→cos+i·sin`、`sin/cos/tan/exp/log/ln/abs/sqrt` 常量折叠、`Pow(x,1/2)→sqrt`）。
> - TeX 字面量解析器放在 `prima-syntax::tex`（MVP 子集：数字/命令/`{}` 分组/`^`/`_` 忽略/隐式乘法/`\frac`），产出与普通语法相同的 AST，由解释器统一求值。
> - 解释器在 `prima-runtime::eval`：`Env`（值/函数双命名空间 + 闭包捕获）、MFn 调用 = 参数替换进 body 符号求值；**广播**（§11.4）在调用点对纯函数逐元素，拒空/嵌套数组；二元数组运算逐元素；`a |> f`/`a |> f(x)` 管道改写为调用。
> - `print`/`println` 当前都会换行（spec 区分 print 与 println，MVP 从简）；默认 LaTeX 输出。
> - 返回类型 `fn` 语法支持 `->` 与 `:` 两种（示例用 `->`，BNF 写 `:`）。

### Phase 2：策略、数值层与错误处理（对应里程碑 4、5、7）

- Config 三级策略（§4.6）；`fraction := false` 生效；F64 不精确传染（§6.4）。
- 坍缩函数族全量（§9.2–9.6）：`to_/try_/checked_/clamped_/rounded_/truncated_`，含 Result 包装、`unwrap` 家族。
- `fn` 宿主函数、`if`/`while`/`for step`/`return`/`try-catch`（Error 传播模型 §4.8）。
- 循环优化（等差闭式）；`Undefined` 严格性检查（§6.2）。
- 模块系统（§4.7）+ `pub`/`import`/冲突检测 + 预导入 core。
- **验收**：§19.1 里程碑 4/5/7 的三个样例逐字通过；`prima check` 报 §16.4 格式的类型错误。

> **Phase 2 落地记录（2026-08）**：全部完成，验收样例见 `examples/config_fraction.pra`（里程碑 4）、`examples/loop_optimization.pra`（里程碑 5）、`examples/try_catch.pra`（里程碑 7）。与本文档的偏差/定稿：
> - 作用域实现为 `Rc<RefCell<Env>>` 共享链（`EnvRef`），块级 `let` 遮蔽 + 跨作用域赋值并存（本文档 §4.8 未明示，落地为共享引用）。
> - `Value::Result`/`Value::Error` 以消息字符串承载错误（§16.1 结构化 `Error` 枚举留待后续阶段补齐）。
> - `to_bigfloat` 为退化实现（原样返回数值，任意精度浮点留待后续）；`print_format` 仅 latex 渲染器可用；`num_to_big` 因数值层全程 BigInt 而无实际分支。
> - `prima check` 先做字面量-注解级静态检查（§6.3 示例可判定）；完整表达式/函数类型推断（§6.3）留待后续阶段。
> - 循环优化先覆盖 `0..n` 与 `1..n` 两种等差模式（§4.8 既定范围）；`for i in 1..100` 按规范 §19.1 示例闭合为 `100*101/2`。

### Phase 3：并行与符号微分（对应里程碑 6 + §19.4 MVP）

- `@parallel` 注解 + `parfor`（rayon，Config 快照传播，§4.6）；副作用静态检查。
- 符号微分引擎：`derivative`/`partial`/`grad` 递归求导规则（§19.4 MVP）；`limit`（泰勒展开/洛必达起步）。
- **验收**：`derivative(f, x)` 对 `x^2 + sin(x)` 输出 `2x + cos(x)`；百万级 `@parallel` 广播验证加速（criterion 基准）。

### Phase 4：标准库与工具链

- `prima-stdlib`：`linalg`（nalgebra：Matrix 构造/运算/分解/求解）、`stats`、`io`（JSON/CSV）、`physics`（§7.3 常数）、`plot`（SVG）。
- CLI 补齐：`repl`（rustyline，续行检测）、`fmt`（AST printer 复用渲染器）、`check`（纯类型检查）、`test`、`doc`。
- **验收**：附录 B 函数速查表逐项可用（对应模块 import 后 golden 测试）。

### Phase 5：JIT（§19.2）

- `cranelift-codegen` 热点编译：调用计数阈值触发（默认 100）或 `@jit` 注解；`ExprDAG → 字节码 → cranelift IR → 原生码`；符号层保持解释。
- AD 前向（Dual）与反向（Tape）模式（§19.4 第二、三阶段）；`jit(grad(f))` 组合。
- **验收**：`f(to_f64(101))` 走原生路径；criterion 对比解释/编译耗时，阈值调优。

> AOT（§19.3，WASM/独立可执行）不在本路线图内，待 Phase 5 完成后再立项评估。

---

## 6. 风险与备选方案

| 风险 | 缓解 |
|------|------|
| 手写 parser 覆盖不全 | 附录 A BNF 是验收清单；insta 快照 + proptest 持续补盲 |
| 化简规则库膨胀（等级 3） | 规则表驱动（`Vec<(Pattern, Rewrite)>`），不写进控制流；等级 3 推迟到 Phase 3+ |
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

其余所有设计（三世界架构、Number 塔、ExprPool、策略三级、模块系统、错误模型、并行哲学）与规范完全一致。

---

*实现方案 Prima v1.0 · 与 SPECIFICATIONS-zh_CN.md v1.0 配套 · 实现工作的唯一依据*
