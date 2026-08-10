# **Prima** —— 语言规范 v1.0

> **声明**：本规范为 **Prima 语言** 的正式语言规范 v1.0，是设计与实现统一的最终依据。

## 标识

 项 | 值 | 说明 |
----|-----|------|
 **语言名** | **Prima** | 拉丁语「第一 / 根本」，呼应「数学真优先」的哲学 |
 **文件后缀** | **`.pra`** | `Prima` 缩写；短、无主流冲突、与语言名直接对应 |
 **入口文件** | **`src/main.pra`** | 项目根模块 |
 **包管理器/工具名** | `prima` | 提供 `run` / `compile` / `repl` 子命令 |

---

## 术语表

| 术语（中文） | 术语（英文） | 定义 | 首次出现 |
|------------|------------|------|---------|
| **符号世界** | Symbol World (W_symbol) | 表达式、符号、化简所在的精确数学层 | §二 |
| **数值世界** | Numeric World (W_numeric) | 浮点数、矩阵等高性能计算层 | §二 |
| **宿主世界** | Host World (W_host) | 控制流、对象、I/O 等功能性层 | §二 |
| **坍缩** | Collapse | 从符号世界向数值世界的显式转换 | §二 |
| **不定式** | Indeterminate | 符号层的未定形式（如 0/0），可进一步化简 | §6.2 |
| **未定义** | Undefined | 数值层的错误状态，不可参与运算 | §6.2 |
| **提升** | Promotion | 数值类型自动向更高精度的转换 | §6.4 |
| **域标注** | Domain Annotation | 表达式的定义域约束（Real/Complex/等） | §6.5 |
| **hash-consing** | Hash-consing | 通过哈希去重实现结构共享的不可变数据结构 | §八 |
| **策略** | Config/Policy | 模块级或全局级的行为配置 | §十三 |
| **编译单元** | Compilation Unit | 一个独立编译的模块（对应一个 .pra 文件或目录） | §十五 |

---

## 一、总体定位

**Prima** 是一门**符号优先**的科学计算语言。它默认精确、默认保留表达式、默认以 LaTeX 渲染结果；通过**丰富的显式坍缩函数族**安全地下降到数值世界；一切行为定制统一由**模块级策略系统**管理；并行完全显式。

**设计哲学**：
- 数学的「真」优先于机器的「快」；
- 性能与精度是**显式选择**，缺省值守恒数学本真；
- 一切可配置项归属**模块**，污染性配置必须声明于项目入口；
- 后续设计决策以**实现可行性 + 用户便捷性 + 上手难度**为准。

**参考系**：Julia（数值/多重分派/提升规则）+ Mathematica/SymPy（符号优先）+ Rust（类型/模块/内存/所有权）+ Python（import 语法）。

---

## 二、总体架构与执行模型

```
源码(.pra)
 ↓ Lexer → Parser
文件（模块主体：策略区 + import 区 + 代码区）
 ↓
AST
 ├─ 数学表达式子树（符号世界）→ ExprDAG → 化简 → LaTeX渲染 / 惰性求值
 │                                             ↕ 显式坍缩函数族(§九)
 │                                             数值求值 → f64/矩阵/复数
 └─ 宿主代码子树（功能世界）→ 类型检查 → 执行 → 结构化错误/panic
```

### 三种「世界」

 世界 | 名称 | 承载内容 | 值形态 | 内存策略 | 特征 |
------|------|---------|--------|---------|------|
 **W_symbol** | 符号世界 | 表达式、符号、化简 | `ExprId`（hash-consing DAG，不可变） | hash-consing interner + 线程本地缓存 | 精确、可化简、线程安全 |
 **W_numeric** | 数值世界 | f64/i32、矩阵、复数、数组 | 栈上值类型 | 栈 + 线性内存（BLAS） | 原生速度 |
 **W_host** | 宿主世界 | 控制流、对象、I/O | 用户对象 | 引用计数 / 可后置 GC | 功能性 |

**核心规则**：表达式离开符号世界进入数值/宿主世界，**必须**经过显式坍缩函数（§九）；**无隐式转换**（除 §十三 策略允许的例外）。

---

## 三、词法

- **标识符**：`[a-zA-Z_][a-zA-Z0-9_]*`（可扩展 Unicode 字母，含希腊字母）。
- **数字字面量**：`123`、`3.14`、`1e-9`、`0x1F`、`0b1010`。
- **字符串**：`"..."`（转义）+ 原始字符串 `r"..."`。
- **TeX 字面量**：``tex"..."``。
- **运算符**：`+ - * / ^ ** @ % == != < <= > >= && || ! = += -=`。其中 `^` 与 `**` 均表示幂运算（互为别名）。
- **注释**：`//` 行注释、`/* */` 块注释。
- **保留关键字**（未来扩展）：`async`、`yield`、`macro`、`trait`、`impl`。

---

## 四、语法

### 4.1 文件结构

Prima 源文件（`.pra`）按顺序由三区组成：

```prima
config {                      // ① 策略区（可选，须在文件顶部；污染性须在项目入口）
    fraction := true
    broadcast := true
    loop_optimization := true
}

import linalg as la           // ② import 区（可选；core 已预导入，无需重复）
from stats import mean, std

                              // ③ 代码区
let f(x) = x^2 + 6
print(f(3))
```

### 4.2 文法骨架

```
program      := config? import* statement*
config       := "config" "{" config_entry* "}"
config_entry := ident ":" type? "=" value
import       := "import" module_path ("as" ident)?
              | "from" module_path "import" (item_list | "*")
statement    := let_stmt | const_stmt | fn_def | math_def | expr_stmt | control_stmt
let_stmt     := "let" mut? ident type_ann? "=" expr
const_stmt   := "const" ident ":" type "=" expr
fn_def       := "fn" ident "(" params ")" type_ann? block
math_def     := "let" ident "(" params ")" type_ann? "=" math_expr
control_stmt := for_stmt | while_stmt | if_stmt | return_stmt | try_stmt | parfor_stmt
math_expr    := expr               // 纯数学函数体，默认符号世界
type_ann     := ":" type
```

---

## 五、值系统（Value）

```rust
enum Value {
    Number(Number),
    Bool(bool), Char(char), String(String),
    Array(Array),           // 同构数值数组，拒绝嵌套（§十一）
    Matrix(Matrix),
    Function(Function),
    Expr(ExprId),           // hash-consed 表达式句柄
    Symbol(SymbolId),       // 内置/用户符号
    Indeterminate(IndeterminateForm),  // 不定式（0/0 等），仅符号层
    Undefined,              // 未定义（数值层错误状态）
    Error(Error),
    Nil,
    Tuple(Vec<Value>),      // 坍缩函数可返回多值
    Result(Result<Box<Value>, Error>), // 安全坍缩的 Result 包装
}
```

**不可变性**：数学值（`Number`/`Expr`/`Symbol`）默认不可变；`W_host` 对象按需可变。

---

## 六、数值塔与类型系统

### 6.1 数值类型层次

```
Number
 ├── 表达式形式（默认，精确）
 │    ├── Expr(ExprId)
 │    └── Symbol(SymbolId)          // \e, \pi, \i 等（§七）
 ├── 精确数值
 │    ├── Integer(i128 / BigInt)    // 溢出行为由策略 num_to_big 决定（§十三）
 │    ├── Rational(BigRat)          // 精确分数，默认偏好
 │    └── Complex{re, im}           // 精确复数（§6.4）
 ├── 不精确数值（坍缩产物，§九）
 │    ├── I32(i32)、F32(f32)、F64(f64)、BigFloat
 └── 特殊值
      ├── Indeterminate(form)       // 不定式（0/0, ∞-∞），仅符号层
      ├── Undefined                 // 未定义，数值层错误状态
      ├── PlusInf / MinusInf        // ±∞
      └── NaN                       // 仅坍缩后存在
```

### 6.2 不定式与未定义的严格区分

#### 符号层：`Indeterminate`
- **定义**：数学上的不定式（indeterminate form），如 `0/0`、`∞/∞`、`0*∞`、`∞-∞`。
- **行为**：
  - 保留为符号节点 `Indeterminate(form_type)`，**不立即报错**。
  - 可参与后续符号化简、极限计算、洛必达法则。
  - 示例：
    ```prima
    let expr = (sin(x) - x) / x^3   // 在 x=0 处形成 0/0，保留为 Indeterminate
    limit(expr, x, 0)               // → -1/6（通过泰勒展开或洛必达）
    simplify(expr)                  // 尝试化简不定式
    ```

#### 数值层：`Undefined`
- **定义**：无法给出有意义数值的错误状态。
- **产生时机**：
  - 不定式**坍缩到数值层**时，若无法化简 → `Undefined`。
  - 实数域下的非法操作：`log(-1)` 在 `domain := real` 策略下 → `Undefined`。
- **严格规则**：
  - **`Undefined` 不得参与任何运算**：任何一元/二元算子输入含 `Undefined` 即**报错**（可静态判定则编译期，否则运行时错误），**不传播**。
  - 示例：
    ```prima
    let a = 0/0                     // 符号层 → Indeterminate
    let b = to_f64(a)               // 坍缩失败 → Undefined
    let c = b + 1                   // 错误：Undefined 不可参与运算
    ```

#### 特殊数值：`NaN` 和 `Inf`
- `0.0/0.0` → `NaN`（浮点运算规则）；`1.0/0.0` → `PlusInf`。
- **`NaN` / `Inf` 不允许在符号层显式存在**，仅显式坍缩到数值层后才出现。

### 6.3 类型系统

#### 类型语法
```
type := 
    // 基础类型
    | "Number" | "Integer" | "Rational" | "F64" | "F32" | "I32" 
    | "Complex" | "Expr" | "Symbol"
    // 复合类型
    | "Array" "<" type ">"
    | "Matrix" "<" type ">"
    | "Tuple" "<" type_list ">"
    // 函数类型
    | "Fn" "(" type_list ")" "->" type
    | "MFn" "(" type_list ")" "->" type   // 纯数学函数
    // 其他
    | "Bool" | "String" | "Char"
    | ident                               // 用户自定义类型
```

#### 类型推断规则（效仿 Rust）

**字面量推断**：
```prima
let x = 1          // → Integer（整数字面量）
let y = 1.0        // → F64（浮点字面量，有小数点或科学记数法）
let z = 0x1F       // → Integer（十六进制）
let s = "hello"    // → String
let b = true       // → Bool
```

**表达式推断**：
```prima
let a = sqrt(2)           // → Expr（符号函数，未坍缩）
let b = 1 + 2             // → Integer（精确整数运算）
let c = 1/3               // → Rational（fraction := true 默认）
let d = 1.0 + 2           // → F64（不精确传染）
let e = [1, 2, 3]         // → Array<Integer>
let f = [[1, 2], [3, 4]]  // 错误：拒绝嵌套数组
```

**函数推断**：
```prima
let f(x) = x^2           // → MFn(Expr) -> Expr（纯数学函数）
fn g(x: F64) -> F64 {    // → Fn(F64) -> F64（功能函数）
    return x * 2.0
}
```

**显式类型注解**：
```prima
let x: F64 = sqrt(2)     // 类型错误：sqrt(2) 是 Expr，需显式坍缩
let y: F64 = to_f64(sqrt(2))  // 正确
let z: Integer = 3.14    // 类型错误
```

**类型兼容性**：
- 精确类型可隐式提升（§6.4）：`Integer → Rational → Complex`。
- 不精确类型传染：`Integer + F64 → F64`。
- 符号类型不自动坍缩：`Expr` 需显式转换才能进入数值计算。

### 6.4 精确复数运算（内置固定规则）

采用 Julia 的**提升（promotion）/转换（convert）** 思想，但实现为**内置固定规则**，不暴露用户扩展点：

**提升序列**：
```
Integer < Rational < Complex<Rational> < F64 < Complex<F64>
```

**提升规则**：
1. **同类精确运算保持精确**：
   ```prima
   1 + 2                  // → Integer(3)
   1/3 + 2/5              // → Rational(11/15)
   Complex(1, 2) + 3      // → Complex(4, 2)（提升 3 → Complex(3, 0)）
   ```

2. **不精确传染**：
   ```prima
   let a = 1/3            // → Rational(1/3)
   let b = to_f64(a)      // → F64(0.333...)
   let c = Complex(0, 1)  // → Complex<Rational>(0, 1)
   b + c                  // → Complex<F64>(0.333..., 1.0)
   ```
   **规则**：遇到 `F64`，整个 `Complex` 提升为 `Complex<F64>`。

3. **自动约分与规范化**：
   ```prima
   2/4                    // → Rational(1/2)（自动约分）
   Rational(6, -9)        // → Rational(-2/3)（分母为正）
   ```

**复数函数**：
- `real(z)`、`imag(z)`、`conj(z)`、`abs(z)`、`abs2(z)`（避免开方）、`angle(z)`。
- 精确指数：`(-1)^(1/2)` 在复数域 → `\i`（§6.5）。

**实现选型**：
- 基础层：`num-complex` + `num-rational`（纯 Rust，MIT/Apache-2.0）。
- 可选加速：`rug`（GMP，LGPL）作为 feature flag（`--features=rug-backend`）。

### 6.5 域标注（Domain Annotation）

**域类型**：
```rust
enum Domain {
    Real,          // 实数域
    Complex,       // 复数域（默认）
    Integer,       // 整数域
    Positive,      // 正实数
    NonNegative,   // 非负实数
    NonZero,       // 非零
}
```

**默认行为**：
- 全局策略 `domain := complex`（默认）或 `domain := real`。
- 符号化简时，采用**最高域**（最宽松的域）：
  ```prima
  let x: Real = -1
  let y = x^(1/2)     // 化简时：最高域 = Complex → y 内部表示为 Complex(\i)
  ```

**域的继承与传播**：

1. **赋值时的域继承**（外部优先性）：
   ```prima
   let x: Real = -1
   let y = x           // y 继承 Real 域标注
   let z = y^(1/2)     // 错误：Real 域下负数开方非法
   ```

2. **显式域转换**（型变能力）：
   ```prima
   let x: Real = -1
   let y = with_domain(x, Complex)  // 显式放宽为 Complex 域
   let z = y^(1/2)                  // 正确 → \i
   ```

3. **函数参数的域继承**：
   ```prima
   let f(x: Real): Real = x^2    // 函数内 x 受 Real 约束
   f(-1)                         // 正确 → 1
   
   let g(x: Real): Complex = x^(1/2)  // 返回类型放宽
   g(-1)                         // 错误：输入域为 Real，内部无法开方
   
   let h(x): Complex = x^(1/2)   // x 无显式域约束，采用默认（Complex）
   h(-1)                         // 正确 → \i
   ```

4. **混合运算的域提升**：
   ```prima
   let a: Real = 2
   let b: Complex = \i
   let c = a + b       // c 的域 = Complex（提升到更宽松的域）
   ```

**符合直觉的原则**：
- 化简/计算时宽松（允许中间步骤使用更高域）。
- 赋值/绑定时严格（外部类型约束优先）。
- 提供显式工具（`with_domain`）打破约束。

### 6.6 默认规则
- **默认精确**：字面量 `2` 是 `Integer`；`sqrt(2)` 保持 `Expr`。
- **不精确传染**：坍缩到 `F64` 后参与的混合运算趋于不精确。
- **分数默认**：默认 `fraction := true`，有理数保留 `Rational`（可配置）。

---

## 七、内置符号体系

**内置符号是语言固有的一部分，独立于 TeX**（TeX 仅是视图）。物理常数值基于 **CODATA 2022**。

### 7.1 数学常数
`\e`（欧拉数）、`\pi`、`\i`（虚数单位）、`\tau`、`\infty`、`\gamma`（欧拉-马歇罗尼常数）、`\phi`（黄金分割）。

### 7.2 算子（表达式结构实体）
`\log`（对数）、`\ln`、`\exp`（指数）、`\sqrt`、`\sin`、`\cos`、`\tan`、`\sigma`（求和）、`\prod`（连乘）、`\int`（积分）、`\partial`（偏导）。

**关键**：**对数、指数等不仅是函数名，更是表达式结构实体**（如 `Apply(Log, [x])`、`Apply(Exp, [x])`），可进入化简、求导、积分。

### 7.3 物理常数（内置，CODATA 2022）

**命名策略**（参考 Julia `PhysicalConstants.jl`）：**默认不自动导出短名**（如 `c`/`h`/`e` 易与变量冲突，污染命名空间），以限定长名访问为主。

 类别 | 内置长名 | 访问方式 |
------|---------|---------|
 基础 | `\speed_of_light`、`\planck_const`、`\boltzmann_const`、`\gravitational_const` | `physics::\speed_of_light` 或 `import physics::\speed_of_light` |
 电磁 | `\elementary_charge`、`\vacuum_permittivity`、`\vacuum_permeability`、`\fine_structure` | 同上 |
 化学 | `\avogadro_const`、`\gas_const`、`\atomic_mass_unit` | 同上 |
 质量 | `\electron_mass`、`\proton_mass`、`\neutron_mass` | 同上 |
 量子 | `\reduced_planck`、`\rydberg`、`\bohr_radius`、`\bohr_magneton` | 同上 |
 其他 | `\standard_gravity`、`\stefan_boltzmann`、`\standard_atmosphere` | 同上 |

**使用示例**：
```prima
import physics              // 仅导入模块命名空间

let E = physics::\planck_const * physics::\speed_of_light  // 限定访问

// 或选择性导入
from physics import \planck_const as h, \speed_of_light as c
let E = h * c
```

> 物理常数以**高精度**存储（默认不自动坍缩，需显式 `to_f64(\planck_const)` 等）。

### 7.4 符号性质
- 内置符号可化简：`\e^{i\pi}` = `Pow(\e, Mul(\i, \pi))`。
- `sqrt(-1)` 复数域 → `\i`；`(-1)^0.5` 由符号携带的 `Domain` 元数据决定（§6.5）。

---

## 八、表达式表示：hash-consing DAG 与化简

### 8.1 表示：`ExprId` + `ExprPool`

```rust
#[derive(Copy, Clone, Hash, Eq, PartialEq)]
pub struct ExprId(u32);                       // 紧凑不透明句柄

pub enum ExprData {
    Symbol(SymbolId),
    Integer(Box<BigInt>),                     // Box 避免 enum 体积过大
    Rational(Box<BigRat>),
    Add(Box<[ExprId]>),                       // 切片指针，紧凑
    Mul(Box<[ExprId]>),
    Pow { base: ExprId, exp: ExprId },
    Apply { f: ExprId, args: Box<[ExprId]> },
    Indeterminate(IndeterminateForm),
    // 可扩展：LaTeX 特殊节点等
}

pub struct ExprPool {
    global: DashMap<u64, ExprId>,             // 全局 interner（分片锁）
    store: RwLock<Vec<ExprData>>,             // 中心存储
}

// 线程本地缓存（优化高频符号构造）
thread_local! {
    static LOCAL_CACHE: RefCell<HashMap<u64, ExprId>> = RefCell::new(HashMap::new());
}
```

**实现策略**：
1. **线程本地缓存优先**：每次 intern 先查 `LOCAL_CACHE`，命中则直接返回。
2. **未命中查全局**：查 `global: DashMap`，查到后回写本地缓存。
3. **延迟全局化**（可选优化）：符号计算过程中先在本地 DAG 累积，计算完成后批量 intern 到全局池。

### 8.2 性能收益（JuliaSymbolics 实测参考）
- 结构共享去重（消除 expression swell）：内存 ↓2×，符号运算快达 3.2×。
- 相等 = 整数/指针比较（O(1)）。
- interner 缓存加速：数值求值快达 100×；codegen/编译 5–10×。
- 不可变 + 只读共享：天然线程安全（§十七）。

### 8.3 化简等级

 等级 | 触发 | 示例 |
------|------|------|
 0 | 刚解析、LaTeX 渲染（展示原形态） | `x + x` |
 1 | 赋值时 | `0*x→0`, `1*x→x` |
 2 | 数值上下文 / `simplify()` | `sin(0)→0`, `2*5→10` |
 3 | 显式 `simplify()` | 有理化、三角恒等、因式分解 |

**默认策略**：打印/渲染 = 等级 0（保留原形态，除非先 `simplify()`）；进入数值坍缩 = 等价于等级 2。化简不改变数学真值。

### 8.4 规范形
`Add`/`Mul` 以 n 元有序列表存储并规范化排序 → `x+1 ≡ 1+x`，同一 `ExprId`，可哈希、可作 `HashMap` 键。

---

## 九、坍缩函数族（Collapse Library）

坍缩是**一族函数**，命名约定表达安全特性，用户按需求选择。

### 9.1 坍缩函数命名体系（重构）

**设计原则**：
- **基础形式** `to_<type>(x)`：失败则 panic，适合受信输入。
- **尝试形式** `try_<type>(x)`：返回 `Result<T, Error>`，适合非受信输入。
- **检查形式** `checked_<type>(x)`：检查溢出/边界，返回 `Result<T, Error>`。
- **钳制形式** `clamped_<type>(x, min, max)`：强制钳制到范围。
- **舍入形式** `rounded_<type>(x, digits)`：按指定位数舍入。

### 9.2 基础坍缩（可能 panic）
```prima
to_i32(x)       // Integer/Rational/F64 → i32，失败 panic
to_i64(x)       // → i64
to_f32(x)       // → f32
to_f64(x)       // → f64（最常用）
to_bigint(x)    // → BigInt
to_rational(x)  // → Rational
to_bigfloat(x)  // → BigFloat（任意精度浮点）
to_complex(x)   // → Complex
```

**示例**：
```prima
let a = sqrt(2)
let b = to_f64(a)          // 1.414...

let c = 1e20
let d = to_i32(c)          // panic: 值超出 i32 范围
```

### 9.3 安全坍缩（返回 `Result<T, Error>`，不 panic）
```prima
try_i32(x)       // → Result<i32, Error>
try_i64(x)       // → Result<i64, Error>
try_f64(x)       // → Result<f64, Error>
try_bigint(x)    // → Result<BigInt, Error>
try_rational(x)  // → Result<Rational, Error>
try_complex(x)   // → Result<Complex, Error>
```

**示例**：
```prima
let a = sqrt(2) + \pi
match try_i32(a) {
    Ok(n) => print("转换成功: {}", n),
    Err(e) => print("转换失败: {}", e)
}
```

### 9.4 检查坍缩（检查溢出/范围）
```prima
checked_i32(x)       // 检查 i32 溢出，返回 Result<i32, Error>
checked_u64(x)       // 检查 u64 溢出和非负性
checked_add(a, b)    // 检查加法溢出
checked_mul(a, b)    // 检查乘法溢出
```

**示例**：
```prima
let a = 2^31 - 1
let b = checked_i32(a)     // Ok(2147483647)
let c = checked_i32(a + 1) // Err(Error::Overflow)
```

### 9.5 钳制坍缩
```prima
clamped_i32(x, min, max)   // 钳制到 [min, max]
clamped_u64(x)             // 钳制到 [0, u64::MAX]
clamped_f64(x, min, max)   // 钳制浮点范围
```

**示例**：
```prima
let a = 1000
let b = clamped_i32(a, 0, 255)  // → 255（钳制到上界）
```

### 9.6 舍入坍缩
```prima
rounded_f64(x, digits)       // 舍入到指定小数位
rounded_i32(x)               // 四舍五入到最近整数
truncated_i32(x)             // 截断小数部分
```

**示例**：
```prima
let a = \pi
let b = rounded_f64(a, 3)    // → 3.142
let c = truncated_i32(a)     // → 3
```

### 9.7 组合坍缩

**管道语法**（显式组合）：
```prima
let a = sqrt(2) + \pi
let b = a |> to_f64          // 等价于 to_f64(a)
let c = a |> try_f64 |> unwrap_or(0.0)
```

**多值返回**：
```prima
complex_to_parts(z)          // → (re, im) 两个独立值
polar_form(z)                // → (r, theta)
```

**示例**：
```prima
let z = Complex(3, 4)
let (r, theta) = polar_form(z)  // r = 5, theta = arctan(4/3)
```

### 9.8 无隐式坍缩 + 不提示精度
- 表达式不自动抬入浮点运算。
- **坍缩 = 用户自决 → 不产生精度告警**（语言不支持精度提示，用户显式选择即主动接受）。
- 仅当坍缩结果是**错误**（如 `to_i32()` 遇非整数，`checked_i32` 溢出）时报错/返回 Error。

### 9.9 幂与定义域
- `sqrt(-1)` 在 `domain := complex` → `\i`。
- 分式指数（如 `(-1)^0.5`）由 `Domain` 元数据（§6.5）决定：
  - `domain := complex` → 允许，得 `\i`。
  - `domain := real` → 报错或产生 `Undefined`。

### 9.10 算子的惰性求值
`\sigma`（求和）、`\prod`（连乘）、`\int`（积分）等算子**默认惰性保留**，直到遇到强制求值函数（显式坍缩或 `loop_optimization` 触发的闭式优化）才数值化。

**示例**：
```prima
let s = sum(i, 1, n)          // 保持符号形式 Σ(i, 1, n)
print(s)                      // LaTeX 输出：\sum_{i=1}^{n} i
let s_eval = to_f64(s)        // 此时才数值求值（需 n 已绑定具体值）
```

---

## 十、求值语义

- **符号求值**：`f(x) = x^2 + 6; f(0)` → 化简后精确结果，不自动数值化。
- **数值求值**：经 §九 坍缩后。
- **循环公式优化**（`loop_optimization := true` 默认开）：`sum(1..n) i → n(n+1)/2`。

**示例**：
```prima
let f(x) = x^2 + 6
let a = f(sqrt(2))       // → Expr: (sqrt(2))^2 + 6 → 2 + 6 → 8（符号化简）
let b = f(3.0)           // → F64: 15.0（数值计算）

// 循环优化
config { loop_optimization := true }
let s = 0
for i in 1..100 {
    s += i               // 编译器识别模式，转换为 s = 100*101/2
}
```

---

## 十一、函数、数组与广播

### 11.1 纯数学函数（MFn）
```prima
let f(x) = x^2 + 1       // 纯函数，默认符号世界
let g(x): F64 = to_f64(x^2)  // 显式声明返回类型
```
**特性**：纯、无副作用、可化简、可组合、一等公民、支持自动微分（§19.4）。可 `@parallel` 注解（§十七）。

### 11.2 功能函数（fn）
```prima
fn process(x: F64) -> F64 {
    print("Processing: {}", x)
    return x * 2.0
}
```
**特性**：可有副作用、控制流、I/O。

### 11.3 数组与索引

#### 数组字面量
```prima
let v = [1, 2, 3]           // Array<Integer>
let w = [1.0, 2.0, 3.0]     // Array<F64>
let x = [1, 2.0]            // Array<F64>（提升到公共类型）
let y = [[1, 2], [3, 4]]    // 错误：拒绝嵌套数组
```

#### 矩阵构造
```prima
let M = Matrix::from_rows([[1, 2], [3, 4]])  // 2×2 矩阵
let N = Matrix::zeros(3, 3)                  // 3×3 零矩阵
let I = Matrix::identity(4)                  // 4×4 单位矩阵
```

#### 索引语法（效仿 Rust）
```prima
// 数组索引
let v = [10, 20, 30, 40]
let a = v[0]                // → 10
let b = v[1..3]             // → [20, 30]（切片，左闭右开）
let c = v[..2]              // → [10, 20]
let d = v[2..]              // → [30, 40]

// 矩阵索引
let M = Matrix::from_rows([[1, 2, 3], [4, 5, 6], [7, 8, 9]])
let e = M[0, 1]             // → 2（单元素）
let f = M[0, ..]            // → [1, 2, 3]（第 0 行）
let g = M[.., 1]            // → [2, 5, 8]（第 1 列）
let h = M[0..2, 1..3]       // → [[2, 3], [5, 6]]（子矩阵）
```

#### 越界处理
```prima
let v = [1, 2, 3]
let x = v[10]               // 运行时错误：索引越界
let y = v.get(10)           // → None（安全访问）
let z = v.get(1)            // → Some(2)
```

### 11.4 广播（Broadcast）

**规则**：
- **拒绝嵌套数组**：广播仅作用于同构标量数组/矩阵的一层；遇到嵌套数组（数组的数组）**报错**，不递归。
- **空数组报错**：广播遇到空数组即报错，不产生静默空结果。
- **默认广播** `broadcast := true`（默认）：纯函数传数组逐元素；`false` 时需显式 `map` 或广播算子。

**示例**：
```prima
config { broadcast := true }

let f(x) = x^2
let v = [1, 2, 3]
let w = f(v)                // → [1, 4, 9]（自动广播）

// 二元运算广播
let a = [1, 2, 3]
let b = [10, 20, 30]
let c = a + b               // → [11, 22, 33]

// 标量广播
let d = a * 10              // → [10, 20, 30]

// 错误示例
let e = [[1, 2], [3, 4]]
let g = f(e)                // 错误：拒绝嵌套数组

let h = []
let i = f(h)                // 错误：空数组
```

**显式控制**（`broadcast := false` 时）：
```prima
config { broadcast := false }

let f(x) = x^2
let v = [1, 2, 3]
let w = map(f, v)           // 显式 map
let x = v @. f              // 广播算子（语法糖）
```

### 11.5 函数上下文
- 纯函数体 → W_symbol（表达式形态，保持精确）。
- 功能函数体 → 默认数值，显式坍缩按需。

---

## 十二、变量、作用域与所有权

### 12.1 变量与常量
```prima
let a = sqrt(2)              // 变量：符号默认保留；标量可变
let mut b = 0                // 显式可变（需 mut 关键字）
const c: Expr = \e^{i\pi}    // 常量：类型必须标注，不可变、可内联
let d: Number = 0            // 显式类型注解
```

**可变性规则**：
- **数值标量**：`let mut` 作用域内可变。
- **复合数学值**（`Expr`/`Array`/`Matrix`）：默认不可变，共享引用。
- **常量**：全局不可变、编译期可内联。

### 12.2 作用域与可见性
- 块作用域（`{}`）遮蔽外层同名变量。
- 模块间变量不互通（§十五）：使用某模块公开项需 `import`。

### 12.3 所有权（用户层不暴露）
- **无手动所有权语法**：不暴露 `&` / `mut` / `move` 等 Rust 风格语法（除了 `let mut` 表明可变绑定）。
- **`ExprId` 是 `Copy` 值句柄**，自然共享；底层 `ExprPool` 只读并发安全。
- **复合值**：浅拷贝（引用计数）+ 写时复制（COW）策略。

### 12.4 内存策略

 层 | 策略 |
----|------|
 W_symbol | hash-consing `ExprPool`（线程本地缓存 + 全局 `DashMap`，可共享、无环、O(1) 相等） |
 W_numeric | 栈值 + nalgebra + 批量算法（BLAS） |
 W_host | 引用计数（MVP）/ 可后置 GC |

---

## 十三、策略系统（Config）

### 13.1 三级策略体系

Prima 采用**三级策略系统**，从全局到局部逐层覆盖：

1. **全局策略**（污染性）：项目入口 `src/main.pra` 的 `config {}`，影响所有模块。
2. **模块策略**：各模块文件顶部 `config {}`，仅影响本模块。
3. **局部策略**：函数/块级 `with config { ... } { code }`，仅影响特定代码块。

**合并规则**：优先级 **局部 > 模块 > 全局**。子模块继承父策略，可局部覆盖。

### 13.2 策略表（定稿）

#### 全局（污染性）策略
**必须**声明于 `src/main.pra` 最上方，在非入口文件声明则**编译报错**。

 策略 | 类型 | 默认 | 说明 |
------|------|------|------|
 `domain` | enum | `complex` | 默认定义域（`complex` / `real`），影响幂运算等 |
 `undefined_handling` | enum | `strict` | `Undefined` 行为（`strict` 报错 / `custom { 0/0 := 1 }` 黑魔法） |

#### 模块/局部策略
可在模块顶部或局部 `with config` 声明。

 策略 | 类型 | 默认 | 说明 |
------|------|------|------|
 `fraction` | bool | `true` | 有理数偏好分数 vs 浮点 |
 `broadcast` | bool | `true` | 纯函数自动逐元素（拒嵌套/空数组） |
 `loop_optimization` | bool | `true` | 循环闭式公式优化 |
 `simplify_level` | int 0-3 | `2` | 默认化简等级 |
 `num_to_big` | bool | `true` | 整数溢出自动升级 BigInt（否则报错） |
 `print_format` | enum | `latex` | 打印格式（`latex` / `unicode` / `ascii`） |

### 13.3 策略使用示例

#### 全局策略（入口文件）
```prima
// src/main.pra
config {
    domain := complex              // 全局默认复数域
    undefined_handling := strict   // 严格模式
}

import mymath

let a = (-1)^0.5                   // 正确 → \i（全局 complex 域）
```

#### 模块策略
```prima
// src/numerical.pra
config {
    fraction := false              // 本模块优先浮点
    simplify_level := 3            // 本模块高级化简
}

let compute(x) = x / 3             // → F64(x / 3.0)
```

#### 局部策略
```prima
config { domain := complex }       // 模块级：复数域

let f(x) = x^2

with config { domain := real } {   // 局部切换到实数域
    let y = (-1)^0.5               // 错误：实数域下负数开方非法
}

let z = (-1)^0.5                   // 正确 → \i（回到模块级 complex 域）
```

### 13.4 特殊值「黑魔法」（实验性）
```prima
// src/main.pra（必须在入口）
config {
    undefined_handling := custom {
        0/0 := 1,                  // 定义 0/0 = 1（危险！）
        log(0) := -\infty          // 定义 log(0) = -∞
    }
}
```
**警告**：此特性破坏数学一致性，仅用于特定领域（如某些极限计算约定）。

---

## 十四、控制流

```prima
// for 循环（范围）
for i in 0..10 {
    total += i
}

// 带步长
for i in 0..10 step 2 {
    print(i)                       // 0, 2, 4, 6, 8
}

// 显式并行循环
parfor i in 0..n {
    A[i] = compute(i)              // 迭代体必须无副作用
}

// while 循环
while cond {
    // ...
}

// if / else if / else
if x > 0 {
    print("positive")
} else if x < 0 {
    print("negative")
} else {
    print("zero")
}

// return
fn f(x) -> F64 {
    if x < 0 {
        return 0.0
    }
    return x * 2.0
}

// try/catch（错误处理）
try {
    let x = to_i32(1e20)
} catch e {
    print("错误: {}", e)
}
```

**规则**：
- 控制流变量**默认数值**，**允许符号**（策略/显式标记）。
- 循环公式优化默认开（`loop_optimization := true`）。

---

## 十五、模块与导入系统

**哲学**：`import` 语法（Python 风格）+ 编译单元/可见性/路径（Rust 风格）+ 模块间变量不互通。

### 15.1 导入语法（`import` 统一）
```prima
import core                    // 引入命名空间
import linalg as la            // 别名
from stats import mean, std    // 选择性导入
from mymath import *           // 通配（不推荐）
```

### 15.2 模块 / 编译单元（Rust 结构）
- **模块是编译单元**，独立作用域。
- **默认私有**：所有项默认私有，跨模块需显式 `pub`。
- **变量不互通**：`import` 把**公开项**引入命名空间，不共享内部状态。
- **嵌套模块路径** `a::b::c`。

**示例**：
```prima
// src/math_utils.pra
pub let square(x) = x^2        // 公开函数
let helper(x) = x + 1          // 私有函数

pub const PHI: Rational = (1 + sqrt(5)) / 2  // 公开常量
```

```prima
// src/main.pra
import math_utils

let a = math_utils::square(3)  // 正确
let b = math_utils::helper(3)  // 错误：helper 是私有的
let c = math_utils::PHI         // 正确
```

### 15.3 文件映射
- **一个 `.pra` 文件 = 一个模块主体**。
- **一个目录 = 一个子模块**，其 `main.pra` 为该目录模块入口（仿 Rust `mod.rs`）。
- **项目入口 = `src/main.pra`**（根模块）。

**示例目录结构**：
```
src/
├── main.pra               // 根模块
├── physics.pra            // 模块 physics
└── linalg/                // 子模块 linalg
    ├── main.pra           // linalg 模块入口
    └── fft.pra            // linalg::fft 子模块
```

```prima
// src/main.pra
import physics
import linalg
import linalg::fft
```

### 15.4 导入冲突
同名符号从多个模块导入：冲突时报错，需别名或 `::` 限定消除。

```prima
from math import sin
from custom_math import sin    // 错误：sin 冲突

// 解决方案 1：别名
from math import sin as std_sin
from custom_math import sin as my_sin

// 解决方案 2：限定访问
import math
import custom_math
let a = math::sin(x)
let b = custom_math::sin(x)
```

### 15.5 预导入
- **预导入 `core`**，且 **`core` 常用功能（内置符号、坍缩函数族、基础算子）全部暴露**。
- 其余模块（`linalg`/`stats`/`plot`/`math`/`io`/`parallel`/`physics`）必须显式 `import`。

**`core` 预导入内容**：
- 数值类型：`Integer`、`Rational`、`F64`、`Complex`、`Expr`、`Symbol`
- 坍缩函数：`to_f64`、`try_i32`、`checked_i32` 等（§九）
- 内置符号：`\e`、`\pi`、`\i`、`\infty`、`\gamma`、`\phi`
- 基础算子：`sqrt`、`sin`、`cos`、`log`、`exp` 等
- 化简函数：`simplify`、`limit`、`derivative`
- 工具函数：`print`、`range`、`map`、`filter`

---

## 十六、错误处理

### 16.1 错误类型定义

```rust
pub enum Error {
    // 类型错误
    TypeError {
        expected: Type,
        got: Type,
        location: SourceLocation,
    },
    
    // 数值域错误
    DomainError {
        expr: Expr,
        reason: String,
        location: SourceLocation,
    },
    
    // 溢出错误
    Overflow {
        value: Number,
        target_type: Type,
        location: SourceLocation,
    },
    
    // 下溢错误
    Underflow {
        value: Number,
        location: SourceLocation,
    },
    
    // 未定义错误
    UndefinedError {
        expr: Expr,
        reason: String,
        location: SourceLocation,
    },
    
    // 索引越界
    IndexOutOfBounds {
        index: usize,
        length: usize,
        location: SourceLocation,
    },
    
    // 维度不匹配
    DimensionMismatch {
        expected: Vec<usize>,
        got: Vec<usize>,
        location: SourceLocation,
    },
    
    // I/O 错误
    IoError {
        kind: IoErrorKind,
        message: String,
        location: SourceLocation,
    },
    
    // 导入错误
    ImportError {
        module: String,
        reason: String,
        location: SourceLocation,
    },
    
    // 语法错误
    SyntaxError {
        message: String,
        location: SourceLocation,
    },
    
    // 自定义错误
    Custom {
        message: String,
        location: SourceLocation,
    },
}

pub struct SourceLocation {
    file: String,
    line: usize,
    column: usize,
}
```

### 16.2 错误分类

 类别 | 语义 | 处理方式 |
------|------|---------|
 **编译期错误** | 语法、类型、可静态判定的 `Undefined` 错误 | 编译失败，提供详细诊断信息 |
 **数值域错误** | `0/0`→`Indeterminate`（符号层）；`Undefined` 参与运算→报错 | 运行时错误，可 `try/catch` |
 **运行时异常** | 越界、维度不匹配、I/O | 结构化 `Error`，可 `try/catch` |
 **兜底 panic** | 无法恢复的内部错误 | 带跨语言堆栈，终止程序 |

### 16.3 错误处理语法

#### try/catch
```prima
try {
    let x = to_i32(1e20)
    print(x)
} catch e: Error::Overflow {
    print("溢出: 使用 BigInt")
    let x = to_bigint(1e20)
} catch e {
    print("其他错误: {}", e)
}
```

#### Result 类型
```prima
let result = try_i32(1e20)
match result {
    Ok(n) => print("成功: {}", n),
    Err(e) => print("失败: {}", e)
}

// 或使用 unwrap 家族
let a = try_i32(100).unwrap()              // 失败则 panic
let b = try_i32(1e20).unwrap_or(0)         // 失败返回默认值
let c = try_i32(1e20).expect("转换失败")   // 自定义 panic 消息
```

### 16.4 错误消息格式

**格式**：**来源定位（文件:行:列）+ 相关表达式（LaTeX）+ 可恢复建议**。

**示例**：
```
错误: 类型不匹配
  --> src/main.pra:15:9
   |
15 |     let x: F64 = sqrt(2)
   |               ^^^^^^^^ 期望 F64，实际是 Expr
   |
   = 提示: 使用 to_f64(sqrt(2)) 显式坍缩
   = 表达式: √2
```

```
错误: 实数域下非法运算
  --> src/numerical.pra:42:18
   |
42 |     let z = (-1)^0.5
   |                  ^^^ 负数在实数域下无法开平方根
   |
   = 提示: 切换到复数域 with config { domain := complex } { ... }
        或使用 with_domain((-1)^0.5, Complex)
   = 表达式: (-1)^{1/2}
```

---

## 十七、并行与多线程

**哲学**：**无隐式并行，并行必须显式**。语言绝不隐式使用线程/`rayon`；是否并行由用户明确指定。

### 17.1 语法：`@parallel` 注解
```prima
let f(x): MFn @parallel = x^2          // 纯函数并行（安全）

// 实验性特性（暂不转正）
// fn process(x) @parallel { ... }     // 功能函数并行（需手动保证线程安全）
```

**规则**：
- `@parallel` 仅可标注**纯数学函数**（安全，编译器验证无副作用）。
- 功能函数并行标记为 `[EXPERIMENTAL]`，需用户手动保证线程安全。

### 17.2 `parfor` 显式并行循环
```prima
parfor i in 0..n {
    A[i] = compute(i)              // 迭代体必须无副作用
}

// 带步长
parfor i in 0..n step 2 {
    B[i] = heavy_computation(i)
}
```

**规则**：
- 仅限**无副作用**迭代体，否则编译报错。
- 底层使用 `rayon` 线程池，粒度自动调优。

### 17.3 线程安全保证
- **数学不可变值**（`ExprId` + `ExprPool`）只读共享，天然线程安全。
- **可变 `W_host` 状态**并行需用户显式管理原语（如 `Mutex`、`Atomic`）。
- **模块边界隔离**可变状态，降低并发风险。

### 17.4 并行示例

#### 并行纯函数广播
```prima
let f(x) @parallel = x^2 + sin(x)
let v = range(0, 1000000)
let w = f(v)                       // 自动并行广播（broadcast + @parallel）
```

#### 并行矩阵运算
```prima
parfor i in 0..rows {
    for j in 0..cols {
        C[i, j] = dot(A[i, ..], B[.., j])
    }
}
```

---

## 十八、标准库规划

 模块 | 内容 | 导入 |
------|------|------|
 **core** | 数字塔、ExprDAG、化简、内置符号、坍缩函数族、基础算子 | **预导入，常用全暴露** |
 **linalg** | 矩阵、线性代数（nalgebra/faer）、求解器 | 显式 |
 **stats** | 统计基础（均值、方差、分位数、分布） | 显式 |
 **plot** | 绘图（SVG 起步，可选 matplotlib-rs 后端） | 显式 |
 **math** | 特殊函数（贝塞尔、伽马、超几何）、数值积分、ODE、FFT | 显式 |
 **io** | 文件 I/O、序列化（JSON/CSV/HDF5）、格式化输出 | 显式 |
 **parallel** | 并行原语（`parfor` 辅助、线程池配置、任务调度） | 显式 |
 **physics** | 物理常数（CODATA 2022）、单位系统（可选） | 显式 |
 **symbolic** | 高级符号操作（求导、积分、级数展开、方程求解） | 显式 |
 **optimize** | 优化算法（梯度下降、牛顿法、BFGS、约束优化） | 显式 |

---

## 十九、编译器与运行时实现路径

### 19.1 MVP（解释器 + 符号引擎）

**核心组件**：
- **词法**：`logos`（快速词法分析器）。
- **语法**：`chumsky`（组合子解析器，错误恢复友好）。
- **符号层**：`ExprPool`（hash-consing）+ `Number`/`Value` + 化简引擎。
- **渲染**：LaTeX 输出（`latex` crate）+ Unicode 备选。
- **数值层**：
  - 任意精度：`num-bigint` + `num-rational` + `num-complex`（纯 Rust，MIT/Apache-2.0）。
  - 可选加速：`rug`（GMP 绑定，LGPL）作为 feature flag（`--features=rug-backend`）。
  - 矩阵：`nalgebra`（通用）或 `faer`（高性能，内存布局优化）。
- **策略系统**：编译期解析 `config {}`，运行时 `ThreadLocal<Config>` 存储。
- **模块系统**：文件系统映射 + `pub` 可见性 + `import` 解析。
- **显式坍缩**：§九 坍缩函数族，带类型检查。

**MVP 里程碑**：
1. **基础符号计算**：
   ```prima
   import core
   let a = tex"\sqrt{2}+\pi"
   print(a)                    // LaTeX 输出：\sqrt{2} + \pi
   ```

2. **化简与求值**：
   ```prima
   const b = tex"\e^{i\pi}+1"
   let c = simplify(b)         // → 0
   print(c)                    // 0
   ```

3. **函数与广播**：
   ```prima
   let f(x) = x^2
   let v = [1, 2, 3]
   print(f(v))                 // [1, 4, 9]
   ```

4. **策略生效**：
   ```prima
   config { fraction := false }
   let x = 1/3
   print(x)                    // 0.333... (F64)
   ```

5. **循环优化**：
   ```prima
   let s = 0
   for i in 1..100 { s += i }  // 编译器优化为 s = 100*101/2
   print(s)                    // 5050
   ```

6. **并行注解**：
   ```prima
   let f(x) @parallel = x^2
   let v = range(0, 1000000)
   let w = f(v)                // 自动并行广播
   ```

7. **错误处理**：
   ```prima
   try {
       let x = to_i32(1e20)
   } catch e {
       print("错误: {}", e)    // 错误: Overflow { value: 1e20, target_type: I32, ... }
   }
   ```

### 19.2 第二阶段：性能优化与 JIT

**JIT 编译**：
- **技术选型**：`inkwell`（LLVM 绑定）或 `cranelift-codegen`（轻量级代码生成，编译速度优先）。
- **混合执行**：符号层解释，数值热点 JIT 编译为原生码。
- **编译触发**：
  - 函数被调用超过阈值（如 100 次）。
  - 显式 `@jit` 注解：`let f(x) @jit = x^2 + sin(x)`。
- **批量算法**：对接 BLAS（OpenBLAS/MKL）、LAPACK，通过 `nalgebra` 或 `faer`。

**示例流程**：
```prima
let f(x) = x^2 + sin(x)

// 前 100 次调用：解释执行
for i in 1..100 {
    let _ = f(to_f64(i))
}

// 第 101 次：触发 JIT 编译
// ExprDAG → LLVM IR → 原生码
let result = f(to_f64(101))  // 原生速度
```

**可组合优化**：
```prima
let f(x) = x^2 + 1
let g = jit(grad(f))         // 组合：自动微分 + JIT 编译
print(g(3.0))                // 原生速度的梯度计算
```

### 19.3 第三阶段：AOT 编译

**目标**：
- 生成独立可执行文件（无运行时依赖）。
- 支持 WebAssembly（浏览器/边缘计算）。

**技术路径**：
1. **查询式编译**（仿 rustc）：
   - 模块依赖图 → 增量编译。
   - 缓存 `ExprId` 化简结果、类型推断信息。
2. **LLVM 后端**：
   - 完整程序分析 → 内联、死代码消除。
   - 链接 BLAS/LAPACK 静态库。
3. **WASM 后端**：
   - `wasm-bindgen` 导出 JS 接口。
   - 符号层编译为 WASM，数值层对接 WASM SIMD。

**命令**：
```bash
prima compile src/main.pra -o outputs/build/myapp       # 本机可执行文件
prima compile src/main.pra --target wasm32 -o app.wasm  # WebAssembly
```

### 19.4 自动微分（差异化卖点，尽早实现）

**实现分阶段**：

#### MVP 阶段：符号微分
- **基于 ExprDAG 的符号求导**：递归应用求导规则。
- **支持函数**：
  ```prima
  let f(x) = x^2 + sin(x)
  let df = derivative(f, x)    // → 2*x + cos(x)（返回 Expr）
  print(df)                    // LaTeX: 2x + \cos(x)
  ```
- **高阶导数**：
  ```prima
  let d2f = derivative(df, x)  // → 2 - sin(x)
  ```
- **偏导数**：
  ```prima
  let g(x, y) = x^2*y + y^3
  let gx = partial(g, x)       // → 2*x*y
  let gy = partial(g, y)       // → x^2 + 3*y^2
  ```

#### 第二阶段：前向模式 AD（数值）
- **双数（Dual Numbers）实现**：
  ```rust
  struct Dual {
      val: f64,
      grad: f64,
  }
  ```
- **重载算术运算**：
  ```prima
  fn eval_dual(f: MFn, x: Dual) -> Dual {
      // 自动传播梯度
  }
  ```
- **适用场景**：梯度计算（少量输入，多量输出）。

#### 第三阶段：反向模式 AD（深度学习风格）
- **计算图 + Tape**：记录前向计算，反向传播梯度。
- **内存管理**：arena 分配器 + 生命周期管理。
- **可组合性**：
  ```prima
  let loss(w) = sum((y - predict(X, w))^2)
  let grad_loss = grad(loss)   // 反向模式自动微分
  let jit_grad = jit(grad_loss)  // JIT 编译梯度函数
  ```

**参考实现**：
- `alkahest`（Julia，符号 + AD + JIT 组合）。
- `enzyme`（LLVM 插件，自动微分）。
- `zygote.jl`（Julia 反向 AD）。

---

## 二十、项目目录结构

**设计规则**：**一切代码（入口/导入区/模块）放 `src/`**；**配置与 README 留在项目根目录**；运行产物放入 `outputs/`。

```
prima_project/                      # 项目根
│
├── src/                            # ★ 所有 Prima 源码（.pra）
│   ├── main.pra                    # 项目入口 = 根模块（污染性策略必须在此顶部）
│   │                              #   含 config{} + import 区 + 代码
│   ├── modules/                    # 业务模块（可选，一 .pra 文件 = 一模块）
│   │   ├── physics.pra
│   │   └── finance.pra
│   └── linalg/                     # 子模块（目录映射）
│       ├── main.pra                #   目录模块入口（仿 Rust mod.rs）
│       └── fft.pra
│
├── config.toml                     # 项目级配置（编译目标、默认优化项、工具参数）
├── README.md
├── prima.toml                      # 项目元数据（包名、版本、依赖 Prima 标准库版本）
│
└── outputs/                        # 运行产物（不提交版本库，通常 .gitignore）
    ├── build/                      # AOT 二进制 / 中间产物（如有）
    └── figures/                    # 生成的 SVG/PNG 图表
```

### 规则说明
1. **`src/main.pra` = 项目入口/根模块**。污染性策略（§13.1）只能出现在这里。
2. **一个 `.pra` 文件 = 一个模块主体**；**一个目录 = 一个子模块**，其 `main.pra` 为目录模块入口。
3. **`config {}` + 全部 `import` 必须位于文件顶部**，之后才是代码。
4. **模块间用 `import` 关联**，变量不互通（§15）。
5. **配置**（`config.toml` / `prima.toml`）与 **README** 在根目录，不进 `src/`。
6. **运行产物**统一进 `outputs/`，`src/` 保持纯源码。

### 工具命令
由 `prima` CLI 提供：

```bash
# 解释执行
prima run src/main.pra

# AOT 编译
prima compile src/main.pra -o outputs/build/myapp

# REPL 交互
prima repl

# 格式化代码
prima fmt src/

# 类型检查（不执行）
prima check src/main.pra

# 测试
prima test

# 文档生成
prima doc
```

---

## 二十一、已定决策总表

 # | 决策 | 结论 |
---|------|------|
 1 | 默认精确 | 表达式/分数优先，`sqrt(2)` 保留为 Expr（§六） |
 2 | 不定式与未定义 | 符号层 `Indeterminate` 可化简；数值层 `Undefined` 报错（§6.2） |
 3 | 结果输出 | 默认 LaTeX，保持精确（§十） |
 4 | 强迫求值 | `to_<type>()`/`try_<type>()`/`checked_<type>()` 等坍缩函数族（§九） |
 5 | 内存 | hash-consing 符号层（线程本地缓存 + 全局池）+ 栈值数值层 + RC/GC 宿主层（§8.1/12.4） |
 6 | 编译 | 解释器+符号引擎 → LLVM JIT → 可选 AOT（§十九） |
 7 | 执行 | 分 W_symbol/W_numeric/W_host 三层（§二） |
 8 | 特殊值 | `Indeterminate` 符号层可化简；`Undefined` 不可入运算；`NaN/Inf` 仅坍缩后（§6.2） |
 9 | LaTeX | 双向桥 + 内置符号独立于 TeX（§七） |
 10 | 循环优化 | 默认开（§十） |
 11 | 广播 | 默认开，拒嵌套/空数组（§11.4） |
 12 | 导入语法 | Python 风格 `import`（§15.1） |
 13 | 模块可见性 | 默认私有，`pub` 公开（§15.2） |
 14 | 并行语法 | `@parallel`（+`parfor`），无隐式并行（§十七） |
 15 | 精确复数 | 内置固定规则（Julia 提升思想），不精确传染明确（§6.4） |
 16 | 内置符号 | 独立于 TeX；数学常数+算子+物理常数（CODATA）（§七） |
 17 | 算子求值 | 默认惰性保留，遇强制求值函数才数值化（§9.10） |
 18 | 预导入 | 仅 core，常用全暴露（§15.5） |
 19 | 错误处理 | 现代化错误分层 + 安全坍缩 Result 可选（§九/十六） |
 20 | 溢出 | `num_to_big` 策略配置（§13.2） |
 21 | 物理常数命名 | 默认不导出短名，限定长名访问（§7.3） |
 22 | 坍缩函数命名 | `to_<type>` / `try_<type>` / `checked_<type>` / `clamped_<type>` / `rounded_<type>`（§9.1-9.6） |
 23 | 名字/后缀 | **Prima / `.pra`**，入口 `src/main.pra`（§标识 & 二十） |
 24 | 目录 | 代码入 `src/`，配置/README 留根目录，产物入 `outputs/`（§二十） |
 25 | 类型系统 | Rust 风格推断 + 显式注解，符号/数值严格分离（§6.3） |
 26 | 域传播 | 化简时最高域，赋值时外部优先，提供 `with_domain` 显式转换（§6.5） |
 27 | 策略系统 | 三级体系：全局（污染）> 模块 > 局部（§十三） |
 28 | 索引语法 | Rust 风格：`v[i]`、`M[i, j]`、`v[1..3]`、`M[.., j]`（§11.3） |
 29 | 自动微分 | MVP 符号微分 → 前向 AD（双数）→ 反向 AD（Tape）（§19.4） |
 30 | 实现选型 | `num-*`（纯 Rust）为基础，`rug`（GMP）可选加速（§19.1） |

---

## 二十二、未来扩展方向

### 22.1 宏系统（保留）
- 保留关键字 `macro`，未来支持编译期代码生成。
- 参考 Rust 声明宏 + 过程宏。

### 22.2 异步支持（保留）
- 保留关键字 `async` / `await`，未来支持异步 I/O。
- 参考 Rust `async-std` / `tokio` 生态。

### 22.3 trait 系统（实验性）
- 保留关键字 `trait` / `impl`，未来支持泛型约束。
- 用于扩展坍缩函数族、自定义类型提升规则。

### 22.4 单位系统（可选模块）
- 物理量带单位：`let v = 3.0 * meter / second`。
- 编译期单位检查，避免火星气候轨道器式灾难。

### 22.5 GPU 加速（第三阶段）
- `@gpu` 注解：自动生成 CUDA/OpenCL/WGSL 代码。
- 矩阵运算、并行循环自动卸载到 GPU。

---

## 附录 A：完整语法 BNF

```bnf
program          ::= config? import* statement*

config           ::= "config" "{" config_entry* "}"
config_entry     ::= ident ":" type? "=" value

import           ::= "import" module_path ("as" ident)?
                   | "from" module_path "import" import_items
import_items     ::= "*" | ident ("," ident)* | ident "as" ident ("," ident "as" ident)*
module_path      ::= ident ("::" ident)*

statement        ::= let_stmt | const_stmt | fn_def | math_def 
                   | expr_stmt | control_stmt | pub_stmt
let_stmt         ::= "let" "mut"? ident type_ann? "=" expr
const_stmt       ::= "const" ident type_ann "=" expr
fn_def           ::= "fn" ident "(" params ")" type_ann? annotation* block
math_def         ::= "let" ident "(" params ")" type_ann? annotation* "=" expr
pub_stmt         ::= "pub" (let_stmt | const_stmt | fn_def | math_def)

annotation       ::= "@parallel" | "@jit" | "@gpu"

control_stmt     ::= for_stmt | while_stmt | if_stmt | return_stmt 
                   | try_stmt | parfor_stmt | with_config_stmt
for_stmt         ::= "for" ident "in" range ("step" expr)? block
parfor_stmt      ::= "parfor" ident "in" range ("step" expr)? block
while_stmt       ::= "while" expr block
if_stmt          ::= "if" expr block ("else" "if" expr block)* ("else" block)?
return_stmt      ::= "return" expr?
try_stmt         ::= "try" block ("catch" ident (":" type)? block)+
with_config_stmt ::= "with" "config" "{" config_entry* "}" block

range            ::= expr ".." expr

params           ::= (param ("," param)*)?
param            ::= ident type_ann?
type_ann         ::= ":" type

type             ::= "Number" | "Integer" | "Rational" | "F64" | "F32" | "I32"
                   | "Complex" | "Expr" | "Symbol" | "Bool" | "String" | "Char"
                   | "Array" "<" type ">"
                   | "Matrix" "<" type ">"
                   | "Tuple" "<" type_list ">"
                   | "Fn" "(" type_list ")" "->" type
                   | "MFn" "(" type_list ")" "->" type
                   | ident
type_list        ::= (type ("," type)*)?

expr             ::= literal | ident | call_expr | index_expr 
                   | binary_expr | unary_expr | paren_expr 
                   | array_expr | tuple_expr | lambda_expr
                   | match_expr | pipeline_expr

literal          ::= number | string | char | bool | tex_literal
number           ::= integer | float | hex | binary
integer          ::= [0-9]+
float            ::= [0-9]+ "." [0-9]+ (("e"|"E") ("+"|"-")? [0-9]+)?
hex              ::= "0x" [0-9a-fA-F]+
binary           ::= "0b" [01]+
string           ::= "\"" char* "\"" | "r\"" char* "\""
char             ::= "'" character "'"
bool             ::= "true" | "false"
tex_literal      ::= "tex\"" tex_content "\""

call_expr        ::= expr "(" args ")"
args             ::= (expr ("," expr)*)?

index_expr       ::= expr "[" index "]"
index            ::= expr | slice
slice            ::= expr? ".." expr? | ".."

binary_expr      ::= expr binary_op expr
binary_op        ::= "+" | "-" | "*" | "/" | "^" | "**" | "@" | "%"
                   | "==" | "!=" | "<" | "<=" | ">" | ">="
                   | "&&" | "||"

unary_expr       ::= unary_op expr
unary_op         ::= "-" | "!" | "+"

paren_expr       ::= "(" expr ")"

array_expr       ::= "[" (expr ("," expr)*)? "]"

tuple_expr       ::= "(" expr "," (expr ("," expr)*)? ")"

lambda_expr      ::= "|" params "|" expr

match_expr       ::= "match" expr "{" match_arm+ "}"
match_arm        ::= pattern "=>" expr ","?
pattern          ::= literal | ident | "_" | pattern "::" ident

pipeline_expr    ::= expr "|>" expr

block            ::= "{" statement* "}"

ident            ::= [a-zA-Z_] [a-zA-Z0-9_]* | "\\" [a-zA-Z_]+
```

---

## 附录 B：标准库函数速查

### B.1 Core（预导入）

#### 坍缩函数
```prima
// 基础坍缩
to_i32(x), to_i64(x), to_f32(x), to_f64(x)
to_bigint(x), to_rational(x), to_bigfloat(x), to_complex(x)

// 安全坍缩
try_i32(x), try_i64(x), try_f64(x), try_bigint(x), try_rational(x), try_complex(x)

// 检查坍缩
checked_i32(x), checked_u64(x), checked_add(a, b), checked_mul(a, b)

// 钳制坍缩
clamped_i32(x, min, max), clamped_u64(x), clamped_f64(x, min, max)

// 舍入坍缩
rounded_f64(x, digits), rounded_i32(x), truncated_i32(x)
```

#### 数学函数
```prima
// 基础算术
sqrt(x), exp(x), log(x), ln(x), log10(x), log2(x)
abs(x), sign(x), floor(x), ceil(x), round(x)

// 三角函数
sin(x), cos(x), tan(x), asin(x), acos(x), atan(x), atan2(y, x)
sinh(x), cosh(x), tanh(x), asinh(x), acosh(x), atanh(x)

// 复数函数
real(z), imag(z), conj(z), abs(z), abs2(z), angle(z)
polar_form(z), complex_to_parts(z)

// 幂与根
pow(base, exp), cbrt(x), nth_root(x, n)
```

#### 化简与符号操作
```prima
simplify(expr, level = 2)      // 化简表达式
expand(expr)                   // 展开
factor(expr)                   // 因式分解
collect(expr, var)             // 合并同类项
substitute(expr, var, value)   // 替换
limit(expr, var, value)        // 极限
derivative(f, var)             // 导数
partial(f, var)                // 偏导数
grad(f)                        // 梯度
integral(f, var)               // 不定积分
definite_integral(f, var, a, b)  // 定积分
```

#### 工具函数
```prima
print(args...)                 // 打印
println(args...)               // 打印并换行
range(start, end, step = 1)    // 生成范围
linspace(start, end, n)        // 线性等分
map(f, array)                  // 映射
filter(pred, array)            // 过滤
reduce(f, array, init)         // 归约
zip(arr1, arr2)                // 拉链
```

### B.2 Linalg（显式导入）

```prima
// 矩阵构造
Matrix::zeros(rows, cols)
Matrix::ones(rows, cols)
Matrix::identity(n)
Matrix::from_rows(data)
Matrix::from_cols(data)
Matrix::diagonal(values)

// 矩阵运算
transpose(M), inverse(M), determinant(M), trace(M)
rank(M), norm(M, p = 2), cond(M)
dot(v1, v2), cross(v1, v2)

// 矩阵分解
lu(M), qr(M), svd(M), eigen(M), cholesky(M)

// 线性求解
solve(A, b)                    // 解 Ax = b
lstsq(A, b)                    // 最小二乘
```

### B.3 Stats（显式导入）

```prima
// 描述统计
mean(data), median(data), mode(data)
variance(data), std(data)
quantile(data, q), percentile(data, p)
min(data), max(data), range(data)

// 相关性
cov(x, y), corr(x, y), spearman(x, y)

// 分布
Normal(mu, sigma), Uniform(a, b), Exponential(lambda)
Binomial(n, p), Poisson(lambda)
pdf(dist, x), cdf(dist, x), quantile(dist, p)
sample(dist, n)
```

### B.4 Plot（显式导入）

```prima
// 基础绘图
plot(x, y, label = "", color = "blue")
scatter(x, y, label = "", marker = "o")
line(x, y, label = "", linestyle = "-")
bar(x, y, label = "")

// 配置
xlabel(text), ylabel(text), title(text)
legend(location = "best")
xlim(min, max), ylim(min, max)
grid(visible = true)

// 保存
savefig(filename, format = "svg", dpi = 300)
show()
```

---

*语言规范 Prima v1.1 · 优化版 · 作为 Prima 语言设计与实现的最终依据*

---
