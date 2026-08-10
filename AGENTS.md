# Prima Language — Agent Guide

## 项目定位
符号优先科学计算语言 **Prima** 的工具链实现仓库（Rust）。语言本身面向用户项目的目录约定见规范 §20，与本仓库（工具链）无关。

## 必读文档（动代码前先读）
- `docs/SPECIFICATIONS-zh_CN.md` — 语言规范 v1.0，最终依据（术语/语法/语义）
- `docs/IMPLEMENTATION-zh_CN.md` — 实现方案：依赖选型、crate 划分、数据结构定稿、Phase 0–5 路线图
- 规范与实现冲突时以实现方案为准（其 §7 有 ADR 记录）

## 工程结构
- **根包 `prima-language` = CLI 二进制**（`src/main.rs`，bin 名 `prima`）+ `src/lib.rs` 再导出
- `crates/prima-syntax` — 手写 lexer + 手写递归下降/Pratt 解析器、AST、Span、SyntaxError
- `crates/prima-core` — Number 数值塔（Integer/Rational/Real/Complex）、Value、ExprPool（hash-consing）
- `crates/prima-runtime` — 解释器、模块系统、Config 策略系统（骨架）
- `crates/prima-stdlib` — linalg/stats/io/physics/plot（骨架）
- 依赖方向单向：`syntax → core → runtime → stdlib`；CLI 在根包
- **所有测试放根 `tests/`**；insta 快照落 `tests/snapshots/`

## 常用命令
```bash
cargo build
cargo test
cargo run -- run examples/simple.pra
INSTA_UPDATE=always cargo test   # 生成/更新快照
cargo test -p prima-syntax       # 单 crate 测试
```
提交用 Conventional Commits（`feat:` / `fix:` / `test:` / `docs:` / `refactor:` / `chore:`），一条一主题。

## 语法/实现关键约定
- **语句以换行分隔**（可选 `;`、`}`、EOF），不是强制 `;`。lexer 输出 `Newline` token，parser 在语句边界消费。
- `config` 条目两种写法都接受：`fraction := true`（规范示例）与 `fraction: bool = true`（附录 BNF）。`custom { 0/0 := 1 }` 黑魔法是 `ExprKind::Custom`。
- **赋值语句**（`s = 0`、`s += i`、`A[i] = ...`）解析为 `Stmt::Assign`（附录 BNF 未列，但示例大量使用，必须支持）。
- 保留关键字 `async/yield/macro/trait/impl` 只做 token，不参与解析。
- 数字字面量在 AST 中存**原始文本**（`Literal::Integer("0x1F")`），数值解析在 core 层做。
- 裸 TeX 写法（如 `\e^{i\pi}`、`\sqrt{2}`，含 `{}` 分组与隐式乘法）**语法层不支持**，必须用 `tex"..."` 字面量（TeX 仅是视图）。
- 类型里 `Fn(...) -> T` / `MFn(...) -> T` 的 `->` 与箭头 token 复用。
- 幂 `^`/`**` 右结合且高于一元负号：`-x^2 == -(x^2)`（同 Julia）。
- `@` = 矩阵乘、`@.` = 广播、`@parallel/@jit/@gpu` = 函数注解（上下文区分）。
- 索引支持多维：`M[.., 1]` → `IndexItem::Slice{..}` 列表。

## core 关键约定
- Number 提升序列 `Integer < Rational < Complex<Rational> < F64 < Complex<F64>`；遇 Real 即 F64 传染。除法：精确层 → Rational（自动约分）。
- `ExprId` 是进程内句柄，**禁止跨进程序列化/缓存**（hash-consing 依赖创建顺序）；内容哈希仅存于内存。
- `ExprPool::global()` 是进程级共享池（OnceLock）。
- 设计红线：无隐式并行、无隐式坍缩、默认精确、`Undefined` 不得参与运算、`NaN/Inf` 仅坍缩后存在。

## 测试约定
- 词法/解析断言优先 insta 快照（Debug 输出含 Span，输入不变输出稳定）；新增语法特性至少一正一负用例。
- CLI 集成测试用 assert_cmd，样例 `.pra` 放 `examples/`（cargo 忽略非 .rs）。
- proptest 随机输入只断言「不 panic、不挂起」，不断言语义。

## Phase 1（已完成）关键约定
- `print`/`println` 都会换行；默认 LaTeX 输出（`print_format` 策略尚未接入）。
- 化简：intern 层（`ExprPool::add2/mul2/pow2/sub2/div2`）做等级 0/1（0*x、1*x、常量合并、Add/Mul 扁平化）；`core::simplify::simplify` 全量应用 2/3 级（`Pow(sqrt(x),2)→x`、欧拉 `e^{iθ}`、`sin/cos/tan/exp/log/ln/abs/sqrt` 常量折叠、`Pow(x,1/2)→\sqrt{x}`）。
- 内置符号在 `core::builtins` 注册（TeX 名作显示名，如 `pi → \pi`）；`SymbolTable::global()` 惰性初始化。
- TeX 字面量：`prima-syntax::tex` 解析 MVP 子集 → 与普通语法相同的 AST → 解释器统一求值。
- 广播（§11.4）在调用点对纯函数逐元素；二元数组运算逐元素；空/嵌套数组报错。
- `ExprData::Real` 已加入（规范 §8.1 未列的落地扩展，便于浮点进入符号 DAG）。

## 大型任务工作方式（重要）
- **大型任务必须拆分为多个并行 SubAgent 执行**（Task 工具），不要在主会话里串行做完一切。
- 划分原则：按 crate 边界 / 模块边界 / 无依赖的独立功能块切分，尽量让子任务之间无共享文件、无运行时耦合：
  - 例如：`prima-syntax`（词法/解析/AST）与 `prima-core`（数值/ExprPool/化简/渲染）可并行；runtime 解释器可在 core API 定稿后并行启动。
- 每个 SubAgent 的任务书必须包含：目标文件清单、依赖的已有 API 签名、验收命令（`cargo build`/`cargo test`/单测）、需要返回的信息。
- 合并前主会话统一 `cargo build` + `cargo test` + `cargo clippy` 收口；接口冲突由主会话裁决。
- 注意：`ExprPool`/`SymbolTable` 是进程级共享（OnceLock），SubAgent 测试若各自起进程无影响，但同一进程内共享状态勿假定顺序。

## 下一步
- Phase 2（策略、数值层与错误处理）：Config 三级策略生效、坍缩函数族（to_/try_/checked_/clamped_/rounded_）、`fn`/`if`/`while`/`for`/`try-catch`、模块系统。详见 `docs/IMPLEMENTATION-zh_CN.md` §5。
