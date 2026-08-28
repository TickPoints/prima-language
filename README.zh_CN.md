# Prima

**符号优先的科学计算语言** —— 默认精确，显式可控。

![license](https://img.shields.io/badge/license-MIT-blue.svg)
![CI](https://github.com/TickPoints/prima-language/actions/workflows/ci.yml/badge.svg)
![release](https://img.shields.io/github/v/release/TickPoints/prima-language)
![stars](https://img.shields.io/github/stars/TickPoints/prima-language?style=social)
![built with Rust](https://img.shields.io/badge/built%20with-Rust-orange.svg)
![PRs welcome](https://img.shields.io/badge/PRs-welcome-brightgreen.svg)

Prima 是一门面向科学计算的语言与工具链，把**符号层放在第一位**：默认所有表达式都保持精确（整数、有理数与符号 `Expr` DAG），直到你显式坍缩；而浮点、矩阵与并行则通过显式策略系统按需开启。

它融合了前人的最佳设计：

| 来源 | Prima 继承了 |
|---|---|
| **Julia** | 数值塔与提升：`Integer < Rational < Complex < F64 < Complex<F64>` |
| **Mathematica / SymPy** | 符号优先：`1/3` 保持 `\frac{1}{3}`，`sqrt(2)` 保持 `\sqrt{2}` |
| **Rust** | 类型、模块、所有权、`Result`/`Option` 与 `?`、带可见性的类 |
| **Python** | 易用性：异质 `Array`、`Dict`/`Set`、推导式、`print`/`println` |

## 为什么选择 Prima？

- **精确的数学，绝不静默失真。** `1/3 + 1/6 = 1/2`，分毫不差。没有 `0.333...` 的漂移，没有比较的意外。`simplify(\e^{i\pi} + 1)` → `0`。浮点只在你要的时候出现。
- **显式优于隐式。** 没有隐式浮点转换、没有隐式并行、没有隐式精度损失。坍缩类型（`i8…u128/isize/usize/f32/f64`）只会在显式 `to_*` / `try_*` / `checked_*` / `clamped_*` 之后出现。`NaN`/`Inf`/`Undefined` 无法溜进精确计算。
- **策略系统，而非预处理器。** `config {}` —— `fraction`、`domain`、`broadcast`、`undefined_handling`、`opt_level` —— 按全局 → 模块 → `with config` 块分层作用，一个文件里可以有意识地混合精确与数值两种模式。
- **可组合的优化管线。** `derivative`、`partial`、`grad`、`limit` 内置于 core；JIT（cranelift）编译热点数值函数，`jit(grad(f))` 把反向自动微分与原生速度串成一条链。
- **真正的工具链。** rustc 风格彩色诊断、静态检查 `prima check`、格式化器、REPL、测试运行器、文档生成，以及 C-ABI 导出（`--emit-c-abi` / `--emit-headers`）。

## 特性

- **默认精确符号计算** —— `1/3` 保持为 `\frac{1}{3}`；`simplify(\e^{i\pi} + 1)` → `0`；`sqrt(2)` 渲染为 `\sqrt{2}` 并保持符号形态，直到显式坍缩。
- **数值塔** —— `Integer < Rational < Complex < F64 < Complex<F64>` 提升、基于 `num-bigint` 的任意精度，以及一组与 Rust 基本数值一一对应的定宽坍缩类型（`i8…u128/isize/usize/f32/f64`），仅经显式坍缩后出现。
- **策略系统**（`config {}`）—— `fraction`、`domain`、`broadcast`、`overload_policy`、`undefined_handling`、`opt_level`（O0–O3）等，按全局 → 模块 → `with config` 块分层作用。
- **Python 风格集合** —— 可变长异质 `Array`、`Dict`/`Set`（`ValueKey` 哈希）、推导式（`[x^2 for x in v if x > 0]`）、负索引与切片赋值、`in`、便捷函数（`len`/`enumerate`/`zip`/`sorted`/`sum`/…）。
- **Rust 式语义** —— `Result`/`Option` 与 `?`、`match`/`if let`/`while let`、完整模式与解构、带 `pub`/`pub(mod)` 可见性的类、`;` 语句分隔。
- **符号微分** —— `derivative`/`partial`/`grad`/`limit` 内置于 core；前向与反向模式自动微分，可直接进入编译代码。
- **并行** —— `@parallel` 广播与 `parfor`（rayon），带静态副作用检查。
- **带类型签名的 `@builtin` 标准库** —— 每个 stdlib 模块是内嵌的 `.pra` 签名文件（`linalg`、`stats`、`io`、`plot`、`physics`、`sys`、`time`、`num`），`@builtin` 声明绑定到 Rust 实现；`prima check` 依据这些签名校验调用点（`E0050`）。
- **工具链** —— `prima run` / `check` / `parse` / `compile` / `repl` / `fmt` / `test` / `doc`，rustc 风格彩色诊断（`error[E00xx]: --> file:line:col`）。

## 安装

获取与你平台匹配的预编译二进制（Linux / macOS / Windows）：

```bash
# Linux / macOS / git-bash / WSL
curl -fsSL https://raw.githubusercontent.com/TickPoints/prima-language/main/install.sh | bash

# Windows（PowerShell）
irm https://raw.githubusercontent.com/TickPoints/prima-language/main/install.ps1 | iex
```

脚本会探测你的操作系统/架构，下载对应版本的二进制，校验其 SHA-256 校验和，并安装到 `~/.local/bin`（Windows 为 `~\.local\bin`）——如有提示请将该目录加入 `PATH`。可用环境变量覆盖：`PRIMA_VERSION`、`PRIMA_TARGET`、`PRIMA_LIBC`（Linux musl）、`PRIMA_INSTALL_DIR`，或命令行参数 `--version`、`--target`、`--dir`。

也可以从源码构建：

```bash
cargo install --git https://github.com/TickPoints/prima-language prima-language
```

## 快速开始

```bash
cargo build --release
cargo run --release -- run examples/simple.pra     # → 9
cargo run --release -- run examples/linear_algebra.pra   # 通过 import 使用标准库
```

语言初体验（`examples/comprehension.pra`）：

```prima
let squares = [x^2 for x in range(0, 10)];
println(squares);                    // → [0, 1, 4, 9, ..., 81]

let d = { "a": 1, "b": 2 };
println(d["b"]);                     // → 2

let s = {1, 2, 3, 2};
println(s ∪ {5, 6});                 // → {1, 2, 3, 5, 6}

let f(x) = x^2;
println(f([1, 2, 3]));               // → [1, 4, 9]（广播）
println(derivative(x^2 + sin(x), x));// → 2 x + \cos(x)
```

## 工具链

```text
prima run    <file.pra>            解释执行程序（文件即根模块）
prima check  [--deny W####] file   静态检查（含 stdlib 调用点类型）
prima parse  <file.pra>            dump AST
prima compile --emit-headers file  为 @c_api::extern 导出生成 C 头文件
prima compile --emit-c-abi file    构建含 C-ABI 导出的共享库
prima repl                         交互式会话（rustyline）
prima fmt    [-w|--check] file     源码格式化
prima test   [dir]                 运行目录下所有 *.pra（默认 examples/）
prima doc    <file.pra> | --stdlib 依据 /// 注释生成 Markdown 文档
```

## 文档

全部文档提供中文（权威）与英文（对照）两个版本：

| 文档 | 中文（权威） | English |
|---|---|---|
| 语言规范 v2.3 | [`SPECIFICATIONS-zh_CN.md`](docs/SPECIFICATIONS-zh_CN.md) | [`SPECIFICATIONS-en_US.md`](docs/SPECIFICATIONS-en_US.md) |
| 实现方案 v2.3 | [`IMPLEMENTATION-zh_CN.md`](docs/IMPLEMENTATION-zh_CN.md) | [`IMPLEMENTATION-en_US.md`](docs/IMPLEMENTATION-en_US.md) |
| 更新日志 | — | [`CHANGELOG.md`](docs/CHANGELOG.md) |

规范与实现方案冲突时以实现方案为准（其 §7 记录 ADR）。

## 开发

```bash
cargo test --workspace       # 完整测试套件（insta 快照、assert_cmd CLI 测试、proptest）
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
INSTA_UPDATE=always cargo test   # 刷新 insta 快照
```

CI（`.github/workflows/ci.yml`）在每次推送与 PR 上，于 Linux 与 Windows 强制格式化、clippy（warnings denied）与完整测试套件。

提交使用 Conventional Commits（`feat:` / `fix:` / `test:` / `docs:` / `refactor:` / `chore:`），一条一主题。

## 许可证

[MIT](./LICENSE)
