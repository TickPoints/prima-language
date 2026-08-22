# **Prima** —— 语言规范 v2.2

> **声明**：本规范为 **Prima 语言** 的正式语言规范 v2.2，是设计与实现统一的最终依据。
> **v2.0 变更摘要**：①错误处理改为 Rust 式 `Result`/`?`/`match`（移除 `try/catch`）；②语句统一以 `;` 划分（换行分隔进入弃用流程，逐步移除）；③引入 Rust 式模式与解构（`if let`/`while let`/`match` 全模式）；④引入 Class（类）与所有权语义；⑤建立编号错误/警告码表（英文，附录 C）；⑥完整字符串支持与 `format`；⑦坍缩后数值类型与 Rust 基本数值类型一一对应；⑧互操作（`@c_api::extern` 导出 C ABI、`@builtin` Rust 实现）；⑨标准库扩充 `sys`/`time`/`num`/`ops`。
> **v2.1 变更摘要（基础类型可用性增强，偏向 Python 风格）**：⑩`Array` 改为**可变长度**序列，支持 `push`/`pop`/`append`/`insert`/`remove`/`extend`/切片赋值/拼接/成员测试（`in`）/负索引/可嵌套（作为数据）；⑪新增**映射类型 `Dict`** 与**集合类型 `Set`**（字面量、索引、方法、迭代）；⑫常用集合便捷函数：`len`/`enumerate`/`sorted`/`reversed`/`sum`/`prod`/`min`/`max`/`all`/`any`/`join`/`count` 等；⑬`print` 与 `println` **区分**（前者不换行，后者换行）；⑭控制台输入 `input`/`read_line`；⑮列表/字典/集合**推导式**（`[x^2 for x in v if x > 0]`）；⑯符号微分原语 `derivative`/`partial`/`grad`/`limit` 纳入 core（§十九）。
> **v2.2 变更摘要**：⑰**`format` 移除，改用 Python 式 f-string** `f"a={a}"`；字符串同时支持 `"..."` / `'...'` 定界与原始字符串 `r"..."`（§18.1）；⑱**文档注释稳定化**：`///`/`//!` 成为规范注释并纳入 AST，`prima doc` 覆盖内置标准库，方法调用出错时在诊断 note 中附带方法定义与文档（§4.1/16.4）；⑲**`@builtin(O1)` 分层优化**：按优化等级在 Rust 实现与 `.pra` 原实现之间切换（§18.4）；⑳**优化等级体系**：新增 `opt_level` 策略（`O0`–`O3`），各等级对应优化通道（§10.2/13.2）；㉑**内置方法体系**：`String` 等常用类方法集以 Python 稳定方法为参照，清单与文档统一维护于内嵌 `.pra` 模块的文档注释（§18.1）；㉒**标准库扩充**：`math` 数值工具（因式分解、泰勒展开）、`physics` 常用公式（Rust 实现）、系统交互、`plot`/`render` 绘图与公式渲染（§十八）；㉓**宿主层内存改为 GC**，标准库提供 `mem::Arc` 显式引用计数（§12.3/12.4）。

## 标识

 项 | 值 | 说明 |
----|-----|------|
 **语言名** | **Prima** | 拉丁语「第一 / 根本」，呼应「数学真优先」的哲学 |
 **文件后缀** | **`.pra`** | `Prima` 缩写；短、无主流冲突、与语言名直接对应 |
 **入口文件** | **`src/main.pra`** | 项目根模块 |
 **包管理器/工具名** | `prima` | 提供 `run` / `compile` / `repl` 等子命令 |

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
| **类** | Class | 字段 + 方法的数据结构（语义近 Rust struct + impl） | §四/十二 |
| **关联函数** | Associated Function | 不接收 `self`、以 `Type::name(...)` 调用的类成员函数 | §4.5 |
| **方法** | Method | 接收 `self`、以 `obj.name(...)` 调用的类成员函数 | §4.5 |
| **模式** | Pattern | 匹配/解构值的结构（`if let`/`while let`/`match`/`let` 解构） | §4.4 |
| **错误码** | Error Code | 编译期/运行时诊断的编号标识（`E####`/`R####`） | §十六/附录C |
| **警告码** | Warning Code | 非致命诊断的编号标识（`W####`） | §十六/附录C |
| **内置实现** | Builtin | 由 Rust 宿主实现的函数/类（`@builtin` 标注） | §十八 |
| **C ABI 导出** | C ABI Export | 以 C 调用约定导出二进制接口（`@c_api::extern`） | §十八 |
| **运算符重载** | Operator Overload | 通过 `ops` 模块为类自定义运算符语义 | §18.5 |

---

## 一、总体定位

**Prima** 是一门**符号优先**的科学计算语言。它默认精确、默认保留表达式、默认以 LaTeX 渲染结果；通过**丰富的显式坍缩函数族**安全地下降到数值世界；一切行为定制统一由**模块级策略系统**管理；并行完全显式；错误处理采用 **Rust 式 `Result`/`?`** 模型。

**设计哲学**：

- 数学的「真」优先于机器的「快」；
- 性能与精度是**显式选择**，缺省值守恒数学本真；
- 一切可配置项归属**模块**，污染性配置必须声明于项目入口；
- **错误是值**：可失败的运算返回 `Result`，由调用方用 `match`/`?`/`unwrap` 显式处理，语言不提供隐式异常吞并机制；
- 后续设计决策以**实现可行性 + 用户便捷性 + 上手难度**为准。

**参考系**：Julia（数值/多重分派/提升规则）+ Mathematica/SymPy（符号优先）+ Rust（类型/模块/内存/所有权/错误处理）+ Python（import 语法；v2.1 起基本类型可用性：可变长 `Array`、`Dict`/`Set`、推导式、`print`/`println`/`input`）。

---

## 二、总体架构与执行模型

```text
源码(.pra)
 ↓ Lexer → Parser
文件（模块主体：策略区 + import 区 + 代码区）
 ↓
AST
 ├─ 数学表达式子树（符号世界）→ ExprDAG → 化简 → LaTeX渲染 / 惰性求值
 │                                             ↕ 显式坍缩函数族(§九)
 │                                             数值求值 → 基本数值/矩阵/复数
 ├─ 宿主代码子树（功能世界）→ 类型检查 → 执行 → Result 传播 / 兜底 panic
 └─ 诊断通道（错误/警告，编号化 §十六）
```

### 三种「世界」

 世界 | 名称 | 承载内容 | 值形态 | 内存策略 | 特征 |
------|------|---------|--------|---------|------|
 **W_symbol** | 符号世界 | 表达式、符号、化简 | `ExprId`（hash-consing DAG，不可变） | hash-consing interner + 线程本地缓存 | 精确、可化简、线程安全 |
 **W_numeric** | 数值世界 | 基本数值（i8…f64、BigInt、复数）、矩阵、数组 | 栈上值类型 | 栈 + 线性内存（BLAS） | 原生速度 |
 **W_host** | 宿主世界 | 控制流、Class 对象、I/O | 用户对象（类实例） | GC（追踪式，浅拷贝共享）+ 值语义（深拷贝）；`mem::Arc` 显式引用计数（§12.4） | 功能性 |

**核心规则**：表达式离开符号世界进入数值/宿主世界，**必须**经过显式坍缩函数（§九）；**无隐式转换**（除 §十三 策略允许的例外）。**可失败的运算不 panic（除非显式 `unwrap`/`to_*`），一律以 `Result` 返回值形式传播。**

---

## 三、词法

- **标识符**：`[a-zA-Z_][a-zA-Z0-9_]*`（可扩展 Unicode 字母，含希腊字母）。
- **数字字面量**：`123`、`3.14`、`1e-9`、`0x1F`、`0b1010`。
- **字符串**：
  - 普通字符串：`"..."` 与 `'...'` 两种定界**等价**，均支持转义（含 `\u{XXXX}`）；
  - 原始字符串：`r"..."` / `r'...'`——不处理转义（`\n` 即字面反斜杠 + `n`），`\u{XXXX}` 不展开；
  - **插值字符串（f-string）**：`f"..."` / `f'...'`——`{expr}` 插值、`{:spec}` 格式精化、`{{`/`}}` 转义（§18.1）；
  - 上述前缀可组合（`rf"..."` 原始 + 插值）。
- **TeX 字面量**：``tex"..."``。
- **运算符**：`+ - * / ^ ** @ % == != < <= > >= && || ! = += -= ?`。其中 `^` 与 `**` 均表示幂运算（互为别名）；`?` 为 **try 运算符**（错误传播，§16.3）；`in` 在**表达式位置**为**成员测试**（§11.3），在 `for`/`parfor` 中为迭代关键字。
- **注释**：`//` 行注释、`/* */` 块注释、**`///` 文档注释**（紧跟其后项，§4.1）、**`//!` 模块文档注释**（§4.1）。
- **保留关键字**（未来扩展）：`async`、`yield`、`macro`、`trait`。
- **生效关键字**：`let`、`const`、`fn`、`class`、`pub`、`self`、`Self`、`if`、`else`、`while`、`for`、`in`、`step`、`parfor`、`return`、`match`、`impl`、`with`、`config`、`import`、`from`、`as`、`true`、`false`。
- **注解**：`@parallel`、`@jit`、`@gpu`、`@builtin`（可带优化等级参数 `@builtin(O1)`，§18.4）、`@c_api::extern`。
- **语句分隔符**：`;`（规范、§4.2）；换行分隔为**弃用形式**（§16.5 W0001）。
- **集合字面量**：`{ ... }` 按上下文区分为 `Dict`/`Set` 字面量（§4.6）与代码块；推导式复用 `[ ... ]`/`{ ... }`/`( ... )` 外框（§11.7）。

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
from stats import mean, std;

                              // ③ 代码区
let f(x) = x^2 + 6;
print(f(3));
```

#### 文档注释（v2.2）

- **`///`**：文档注释，紧跟其后的**项**（`fn`/`let`（MFn）/`class`/方法/字段/`const`/`import` 绑定）——多个连续 `///` 行合并为该项的文档文本。
- **`//!`**：模块文档注释，置于模块文件顶部（`config`/`import` 区之前），描述整个模块。
- 文档注释属于**语言语义的一部分**：随项一起被解析进 AST 并保留；`prima doc`（§二十）生成文档、`prima check`/解释器的诊断 note（§16.4）读取它。
- 文档注释支持简单标记（如 `` `code` ``、标题行、`# Examples` 等），渲染规则由 `prima doc` 定义（§实现方案）。
- 例：

  ```prima
  //! The `String` type and its method set.

  /// Returns the number of Unicode scalar values in `self`.
  pub fn len(self) -> Integer
  ```

### 4.2 语句划分

- **规范形式**：每条语句以 `;` 结尾（`;` 是唯一规范语句分隔符）。
- **块级语句**（`if`/`while`/`for`/`parfor`/`fn`/`class`/`match`/`with config` 后跟 `{}` 的语句）末尾 `;` 可省略，与 Rust 一致。
- **弃用形式**：以换行分隔语句（紧跟语句末尾的换行充当分隔）仍被接受，但产生警告 `W0001`（§16.5），并将在后续版本中**移除**。新代码必须使用 `;`。
- **空语句**：单独的 `;` 合法（no-op）。

```prima
let a = 1;                 // ✓ 规范：以 ; 分隔
let b = 2                  // ⚠ W0001：换行分隔（弃用）
let c = 3;                 // 上一条语句在 b = 2 的换行处结束

if a > 0 {                 // ✓ 块级语句省略末尾 ;
    print(a);
}
```

### 4.3 文法骨架

```text
program      := config? import* item*
item         := pub_item | statement
pub_item     := "pub" statement
statement    := let_stmt | const_stmt | fn_def | math_def | class_def
              | expr_stmt | control_stmt | impl_stmt
let_stmt     := "let" "mut"? pattern type_ann? "=" expr
const_stmt   := "const" ident ":" type "=" expr
fn_def       := "fn" ident "(" params ")" type_ann? annotation* block
math_def     := "let" ident "(" params ")" type_ann? annotation* "=" expr
class_def    := "class" ident "{" class_member* "}"
expr_stmt    := expr
control_stmt := for_stmt | while_stmt | if_stmt | if_let_stmt
              | while_let_stmt | match_stmt | return_stmt | parfor_stmt
              | with_config_stmt
math_expr    := expr               // 纯数学函数体，默认符号世界
type_ann     := ":" type
annotation   := "@parallel" | "@jit" | "@gpu"
              | "@builtin" ("(" opt_level ")")?    // 默认 @builtin(O0)（§18.4）
              | "@c_api" "::" "extern"
opt_level    := "O0" | "O1" | "O2" | "O3"           // §10.2
```

> 语句分隔：`;` 规范；换行弃用（§4.2）。`pub` 可修饰 `let`/`const`/`fn`/`math_def`/`class_def`。

### 4.4 模式与解构（Rust 式）

模式用于 `let` 解构、`if let`、`while let`、`match` 分支：

```text
pattern        := pattern_alt
pattern_alt    := pattern_simple ("|" pattern_simple)*      // 或模式
pattern_simple := "_" | ident | literal | "-"? literal
                | tuple_pattern | array_pattern | class_pattern
                | variant_pattern | range_pattern | grouped_pattern
tuple_pattern  := "(" pattern ("," pattern)* ("," "..")? ")"
array_pattern  := "[" pattern ("," pattern)* ("..")? "]"
class_pattern  := ident "{" field_pattern ("," field_pattern)* ("..")? "}"
field_pattern  := ident (":" pattern)?                      // 字段简写：name 等价 name: name
variant_pattern:= ident pattern_simple?                     // 构造器模式：Some(x)、Ok(v)、None
range_pattern  := literal ".." literal | literal "..=" literal
grouped_pattern:= "(" pattern ")"
guard          := pattern_alt "if" expr                     // match 守卫
```

**规则**：

- `_` 通配；`ident` 绑定；字面量匹配；`-` 用于负数字面量。
- 元组/数组模式支持 `..` 省略其余元素。
- 类模式 `Point { x, y: 0, .. }` 匹配字段。
- 构造器模式用于内建 `Option`（`Some`/`None`）与 `Result`（`Ok`/`Err`）。
- 范围模式仅对可比较字面量（数字/字符）可用。
- `let` 只接受**不可反驳模式**（可反驳模式如 `Some(x)` 必须用 `if let`/`match`）。
- `match` 分支守卫 `pattern if cond => ...`。

**示例**：

```prima
let v = [1, 2, 3];
let (first, ..) = v;                 // 元组/数组解构

match v {
    [x, ..] if x > 0 => print("positive head"),
    [..]             => print("empty or other")
}

if let Some(x) = v.get(0) {          // 安全索引返回 Option
    print(x);
}

while let Some(x) = iter.next() {    // 迭代
    print(x);
}

match try_f64("3.14") {
    Ok(x)  => print(f"parsed {x}"),
    Err(e) => print(f"failed: {e}")
}
```

### 4.5 Class（类）定义

**Class 是字段 + 方法的聚合类型**（语义近 Rust `struct` + `impl` 的组合）。语法：

```prima
pub class Test {                      // pub：跨模块可见；省略则默认私有（§15.2）
    a: Expr,                          // 字段（默认类内私有）
    b: Expr,

    pub fn new(a: Expr, b: Expr) -> Self {   // 关联函数；Self = 本类类型
        Test { a, b }                 // 结构字面量（字段简写）
    }

    pub(mod) fn get_a(self) -> Expr { // pub(mod)：当前模块可见（§15.2）
        self.a                        // 返回基本值 → 深拷贝（§12.3）
    }
}

let test1 = Test::new(1, 2);          // 关联函数调用
print(test1.get_a());                 // 方法调用
```

**规则**：

1. **可见性**（成员）：
   - 无修饰 → 类内私有（仅本类方法可访问）。
   - `pub(mod)` → 当前模块可见（模块内可调用/访问）。
   - `pub` → 公开，可跨模块（类自身需为 `pub` 或 `pub(mod)`）。
2. **字段**：`ident : type`；字面量构造 `Test { a: expr, ... }`，`Test { a }` 为简写。字段默认只读，类内方法可读。
3. **关联函数**：不接收 `self`，经 `Type::name(args)` 调用；典型用途为构造器（约定名为 `new`，返回 `Self`）。结构字面量本身也是构造手段。
4. **方法**：首个参数为 `self`，经 `obj.name(args)` 调用；`self` 是**对象本身的浅拷贝**（共享底层，§12.3）。
5. **所有权**（§12.3）：`self` 浅拷贝（引用计数共享）；方法**返回基本值**（`Number`/`Expr`/`String` 等）时深拷贝后传出，返回本类实例时保持共享。
6. **`Self`**：类体内的类型别名，指代当前类。
7. Class 不设继承。组合与 trait 式接口经 `ops` 模块（§18.5）实现运算符语义。
8. **管道弃用**：`|>` 管道（§9.7）为弃用语法（`W0002`），其职责逐步由「类方法 + 方法链」取代。

**示例（方法链取代管道）**：

```prima
// 弃用：a |> to_f64 |> rounded_f64(3)
// 规范：通过类方法组合
let result = Float(a) |> to_f64;      // ⚠ W0002 弃用

class Float {
    pub fn new(x) -> Self { Float { v: x } }
    pub fn to_f64(self) -> F64 { to_f64(self.v) }
    pub fn rounded(self, digits) -> F64 { rounded_f64(self.v, digits) }
}
let r = Float::new(sqrt(2) + \pi).to_f64().rounded(3);   // 方法链
```

### 4.6 集合字面量与推导式（v2.1）

`Dict` 与 `Set` 使用花括号字面量；推导式复用 `[ ]`/`{ }`/`( )` 外框，由 `for` 从句区分：

```prima
// Dict 字面量：{ key: value, ... }（键可为数字/字符串/布尔/符号等不可变值）
let d = { "a": 1, "b": 2, "c": 3 };
let d2 = Dict::new();                 // 空字典（类型可变长）

// Set 字面量：{ value, ... }（元素必须可哈希，默认数字/字符串/布尔）
let s = {1, 2, 3, 2};                 // 重复元素去重 → {1, 2, 3}
let s2 = Set::new();                  // 空集合

// 空花括号 {} 默认是空 Dict（与 Rust 字面量习惯一致）
let e = {};

// 推导式（§11.7）：外框决定产出类型
let squares = [x^2 for x in range(0, 10) if x % 2 == 0];   // Array
let lookup  = {x: x^2 for x in range(0, 5)};               // Dict
let odds    = {x for x in range(0, 10) if x % 2 == 1};     // Set
let pairs   = ((x, x+1) for x in range(0, 3));             // Tuple（惰性生成器）
```

**规则**：

1. `{ k: v }` 判定为 Dict 字面量（键值对形式）；`{ a, b }` 判定为 Set 字面量；`{}` 为空 Dict。
2. Dict 键必须是**不可变且可哈希**的值（`Number`/`String`/`Char`/`Bool`/`Expr`/`Symbol`）；Set 元素同理。
3. 集合字面量在**表达式位置**才有效；`{` 紧跟控制流关键字时仍是代码块。
4. 推导式语法：`<外框> <元素表达式> for <变量> in <可迭代> [if <条件>]`，可多重 `for`（笛卡尔积）。`Dict` 推导式元素为 `key: value` 对；`Set` 推导式元素为单值。
5. 花括号在 `Dict`/`Set` 字面量与 `match`/`class`/`impl`/`config`/`with config` 后代码块之间按位置消歧（详见附录 A BNF）。

---

## 五、值系统（Value）

```rust
enum Value {
    Number(Number),
    Bool(bool), Char(char), String(String),
    Array(Array),           // 可变长序列（v2.1：元素可为任意值，§11.3）
    Dict(Dict),             // v2.1：映射类型，键不可变可哈希，§11.6
    Set(Set),               // v2.1：集合类型，元素不可变可哈希，§11.6
    Matrix(Matrix),
    Function(Function),
    Expr(ExprId),           // hash-consed 表达式句柄
    Symbol(SymbolId),       // 内置/用户符号
    Class(ClassId),         // 类实例（§4.5/十二）
    Option(Option<Box<Value>>),  // Option<T>：Some(T) / None
    Indeterminate(IndeterminateForm),  // 不定式（0/0 等），仅符号层
    Undefined,              // 未定义（数值层错误状态）
    Error(Error),
    Nil,                    // 单元/无返回值
    Tuple(Vec<Value>),      // 坍缩函数可返回多值
    Result(Result<Box<Value>, Error>), // 安全坍缩/可失败运算的 Result 包装
}
```

**不可变性**：数学值（`Number`/`Expr`/`Symbol`）默认不可变；`Array`/`Dict`/`Set` 是**可变宿主值**（长度/内容可变，§12.1）；`W_host` 对象（类实例）按 §12.3 的浅拷贝/深拷贝语义管理。

---

## 六、数值塔与类型系统

### 6.1 数值类型层次

```text
Number
 ├── 表达式形式（默认，精确）
 │    ├── Expr(ExprId)
 │    └── Symbol(SymbolId)          // \e, \pi, \i 等（§七）
 ├── 精确数值
 │    ├── Integer(BigInt)           // 溢出行为由策略 num_to_big 决定（§十三）
 │    ├── Rational(BigRat)          // 精确分数，默认偏好
 │    └── Complex{re, im}           // 精确复数（§6.4）
 ├── 坍缩后数值（§九，与 Rust 基本数值一一对应）
 │    ├── I8(i8)  I16(i16)  I32(i32)  I64(i64)  I128(i128)
 │    ├── U8(u8)  U16(u16)  U32(u32)  U64(u64)  U128(u128)
 │    ├── Isize(isize)  Usize(usize)
 │    ├── F32(f32)  F64(f64)
 │    └── BigFloat                // 任意精度浮点
 └── 特殊值
      ├── Indeterminate(form)       // 不定式（0/0, ∞-∞），仅符号层
      ├── Undefined                 // 未定义，数值层错误状态
      ├── PlusInf / MinusInf        // ±∞
      └── NaN                       // 仅坍缩后存在
```

**坍缩后数值类型与 Rust 基本数值类型一一对应**：`i8/i16/i32/i64/i128/u8/u16/u32/u64/u128/isize/usize/f32/f64`。类型名即大写形式（`I8`、`U32`、`F64`、`Isize`、`Usize`…）。

### 6.2 不定式与未定义的严格区分

#### 符号层：`Indeterminate`

- **定义**：数学上的不定式（indeterminate form），如 `0/0`、`∞/∞`、`0*∞`、`∞-∞`。
- **行为**：
  - 保留为符号节点 `Indeterminate(form_type)`，**不立即报错**。
  - 可参与后续符号化简、极限计算、洛必达法则。
  - 示例：

    ```prima
    let expr = (sin(x) - x) / x^3;   // 在 x=0 处形成 0/0，保留为 Indeterminate
    limit(expr, x, 0);               // → -1/6（通过泰勒展开或洛必达）
    simplify(expr);                  // 尝试化简不定式
    ```

#### 数值层：`Undefined`

- **定义**：无法给出有意义数值的错误状态。
- **产生时机**：
  - 不定式**坍缩到数值层**时，若无法化简 → `Undefined`。
  - 实数域下的非法操作：`log(-1)` 在 `domain := real` 策略下 → `Undefined`。
- **严格规则**：
  - **`Undefined` 不得参与任何运算**：任何一元/二元算子输入含 `Undefined` 即**报错**（可静态判定则编译期，否则运行时 `R0006`），**不传播**。
  - 示例：

    ```prima
    let a = 0/0;                     // 符号层 → Indeterminate
    let b = to_f64(a);               // 坍缩失败 → panic（to_* 家族）
    let c = try_f64(a);              // → Err(Error::UndefinedError)
    ```

#### 特殊数值：`NaN` 和 `Inf`

- `0.0/0.0` → `NaN`（浮点运算规则）；`1.0/0.0` → `PlusInf`。
- **`NaN` / `Inf` 不允许在符号层显式存在**，仅显式坍缩到数值层后才出现。

### 6.3 类型系统

#### 类型语法

```text
type :=
    // 基础数值类型
    | "Number" | "Integer" | "Rational" | "F64" | "F32"
    | "I8" | "I16" | "I32" | "I64" | "I128"
    | "U8" | "U16" | "U32" | "U64" | "U128" | "Isize" | "Usize"
    | "Complex" | "Expr" | "Symbol"
    // 复合类型
    | "Array" "<" type ">"
    | "Matrix" "<" type ">"
    | "Tuple" "<" type_list ">"
    | "Option" "<" type ">"
    // 函数类型
    | "Fn" "(" type_list ")" "->" type
    | "MFn" "(" type_list ")" "->" type   // 纯数学函数
    | "Result" "<" type "," type ">"
    // 其他
    | "Bool" | "String" | "Char"
    | ident                               // 用户自定义类型（含类）
    | "Self"                              // 类体内自指（§4.5）
```

#### 类型推断规则（效仿 Rust）

**字面量推断**：

```prima
let x = 1;          // → Integer（整数字面量）
let y = 1.0;        // → F64（浮点字面量，有小数点或科学记数法）
let z = 0x1F;       // → Integer（十六进制）
let s = "hello";    // → String
let b = true;       // → Bool
```

**表达式推断**：

```prima
let a = sqrt(2);           // → Expr（符号函数，未坍缩）
let b = 1 + 2;             // → Integer（精确整数运算）
let c = 1/3;               // → Rational（fraction := true 默认）
let d = 1.0 + 2;           // → F64（不精确传染）
let e = [1, 2, 3];         // → Array<Integer>
let f = [[1, 2], [3, 4]];  // 错误：拒绝嵌套数组
```

**函数推断**：

```prima
let f(x) = x^2;           // → MFn(Expr) -> Expr（纯数学函数）
fn g(x: F64) -> F64 {     // → Fn(F64) -> F64（功能函数）
    return x * 2.0;
}
```

**显式类型注解**：

```prima
let x: F64 = sqrt(2);     // 类型错误：sqrt(2) 是 Expr，需显式坍缩
let y: F64 = to_f64(sqrt(2));  // 正确
let z: Integer = 3.14;    // 类型错误
```

**类型兼容性**：

- 精确类型可隐式提升（§6.4）：`Integer → Rational → Complex`。
- 不精确类型传染：`Integer + F64 → F64`。
- 符号类型不自动坍缩：`Expr` 需显式转换才能进入数值计算。
- 坍缩后定宽类型之间**不隐式转换**：`I32 → I64` 需显式 `to_i64`（防静默溢出，§九）。

### 6.4 精确复数运算（内置固定规则）

采用 Julia 的**提升（promotion）/转换（convert）** 思想，但实现为**内置固定规则**，不暴露用户扩展点：

**提升序列**：

```text
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
   let a = 1/3;            // → Rational(1/3)
   let b = to_f64(a);      // → F64(0.333...)
   let c = Complex(0, 1);  // → Complex<Rational>(0, 1)
   b + c;                  // → Complex<F64>(0.333..., 1.0)
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
  let x: Real = -1;
  let y = x^(1/2);     // 化简时：最高域 = Complex → y 内部表示为 Complex(\i)
  ```

**域的继承与传播**：

1. **赋值时的域继承**（外部优先性）：

   ```prima
   let x: Real = -1;
   let y = x;           // y 继承 Real 域标注
   let z = y^(1/2);     // 错误：Real 域下负数开方非法
   ```

2. **显式域转换**（型变能力）：

   ```prima
   let x: Real = -1;
   let y = with_domain(x, Complex);  // 显式放宽为 Complex 域
   let z = y^(1/2);                  // 正确 → \i
   ```

3. **函数参数的域继承**：

   ```prima
   let f(x: Real): Real = x^2;    // 函数内 x 受 Real 约束
   f(-1);                         // 正确 → 1

   let g(x: Real): Complex = x^(1/2);  // 返回类型放宽
   g(-1);                         // 错误：输入域为 Real，内部无法开方

   let h(x): Complex = x^(1/2);   // x 无显式域约束，采用默认（Complex）
   h(-1);                         // 正确 → \i
   ```

4. **混合运算的域提升**：

   ```prima
   let a: Real = 2;
   let b: Complex = \i;
   let c = a + b;       // c 的域 = Complex（提升到更宽松的域）
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
import physics;              // 仅导入模块命名空间

let E = physics::\planck_const * physics::\speed_of_light;  // 限定访问

// 或选择性导入
from physics import \planck_const as h, \speed_of_light as c;
let E = h * c;
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

### 9.1 坍缩函数命名体系

**设计原则**：

- **基础形式** `to_<type>(x)`：失败则 **panic**，适合受信输入。
- **尝试形式** `try_<type>(x)`：返回 `Result<T, Error>`，适合非受信输入。
- **检查形式** `checked_<type>(x)`：检查溢出/边界，返回 `Result<T, Error>`。
- **钳制形式** `clamped_<type>(x, min, max)`：强制钳制到范围。
- **舍入形式** `rounded_<type>(x, digits)`：按指定位数舍入。

**类型覆盖**：所有坍缩函数族覆盖与 Rust 基本数值一一对应的全部类型：`i8/i16/i32/i64/i128/u8/u16/u32/u64/u128/isize/usize/f32/f64`，外加 `bigint/rational/bigfloat/complex`。

### 9.2 基础坍缩（可能 panic）

```prima
to_i8(x)   to_i16(x)  to_i32(x)  to_i64(x)  to_i128(x)
to_u8(x)   to_u16(x)  to_u32(x)  to_u64(x)  to_u128(x)
to_isize(x) to_usize(x)
to_f32(x)  to_f64(x)                          // f64 最常用
to_bigint(x) to_rational(x) to_bigfloat(x) to_complex(x)
```

**示例**：

```prima
let a = sqrt(2);
let b = to_f64(a);          // 1.414...

let c = 1e20;
let d = to_i32(c);          // panic: 值超出 i32 范围
```

### 9.3 安全坍缩（返回 `Result<T, Error>`，不 panic）

```prima
try_i8(x)  try_i16(x)  try_i32(x)  try_i64(x)  try_i128(x)
try_u8(x)  try_u16(x)  try_u32(x)  try_u64(x)  try_u128(x)
try_isize(x) try_usize(x)
try_f32(x) try_f64(x)
try_bigint(x) try_rational(x) try_complex(x)
```

**示例**（与 `match`/`?` 组合）：

```prima
let a = sqrt(2) + \pi;
match try_i32(a) {
    Ok(n)  => print(f"converted {n}"),
    Err(e) => print(f"failed: {e}")
}

// 或使用 ? 传播（仅可在返回 Result 的函数内）
fn parse(x) -> Result<F64, Error> {
    let v = try_f64(x)?;      // Err 则提前返回
    return Ok(v * 2.0);
}
```

### 9.4 检查坍缩（检查溢出/范围）

```prima
checked_i8(x)  checked_i16(x)  checked_i32(x)  checked_i64(x)  checked_i128(x)
checked_u8(x)  checked_u16(x)  checked_u32(x)  checked_u64(x)  checked_u128(x)
checked_add(a, b)    // 检查加法溢出
checked_mul(a, b)    // 检查乘法溢出
```

**示例**：

```prima
let a = 2^31 - 1;
let b = checked_i32(a);     // Ok(2147483647)
let c = checked_i32(a + 1); // Err(Error::Overflow)
```

### 9.5 钳制坍缩

```prima
clamped_i32(x, min, max)   // 钳制到 [min, max]
clamped_u8(x, min, max)
clamped_u64(x)             // 钳制到 [0, u64::MAX]
clamped_f64(x, min, max)   // 钳制浮点范围
```

**示例**：

```prima
let a = 1000;
let b = clamped_i32(a, 0, 255);  // → 255（钳制到上界）
```

### 9.6 舍入坍缩

```prima
rounded_f64(x, digits)       // 舍入到指定小数位
rounded_i32(x)               // 四舍五入到最近整数
truncated_i32(x)             // 截断小数部分
```

**示例**：

```prima
let a = \pi;
let b = rounded_f64(a, 3);    // → 3.142
let c = truncated_i32(a);     // → 3
```

### 9.7 组合坍缩

**规范形式**：基于 `Result` 的链式处理（`?` + `match` + `unwrap` 家族）与类方法（§4.5）。

```prima
let a = sqrt(2) + \pi;
let b = try_f64(a)?.unwrap_or(0.0);   // 先 ? 传播，再兜底默认值
let c = try_f64(a).unwrap();          // 失败则 panic
let d = try_f64(a).expect("convert pi");  // 自定义 panic 消息
```

**弃用管道**：`|>` 管道（`a |> f`）为弃用语法（§16.5 `W0002`），逐步被方法链取代（§4.5 示例）。

**多值返回**：

```prima
complex_to_parts(z)          // → Tuple<(re, im)> 两个独立值
polar_form(z)                // → Tuple<(r, theta)>
```

**示例**（`let` 元组解构）：

```prima
let z = Complex(3, 4);
let (r, theta) = polar_form(z);  // r = 5, theta = arctan(4/3)
```

### 9.8 无隐式坍缩 + 不提示精度

- 表达式不自动抬入浮点运算。
- **坍缩 = 用户自决 → 不产生精度告警**（语言不支持精度提示，用户显式选择即主动接受）。
- 仅当坍缩结果是**错误**时（`to_i32()` 遇非整数 → panic；`checked_i32` 溢出 → `Err`）按 §九 处理。

### 9.9 幂与定义域

- `sqrt(-1)` 在 `domain := complex` → `\i`。
- 分式指数（如 `(-1)^0.5`）由 `Domain` 元数据（§6.5）决定：
  - `domain := complex` → 允许，得 `\i`。
  - `domain := real` → 报错或产生 `Undefined`。

### 9.10 算子的惰性求值

`\sigma`（求和）、`\prod`（连乘）、`\int`（积分）等算子**默认惰性保留**，直到遇到强制求值函数（显式坍缩或 `loop_optimization` 触发的闭式优化）才数值化。

**示例**：

```prima
let s = sum(i, 1, n);          // 保持符号形式 Σ(i, 1, n)
print(s);                      // LaTeX 输出：\sum_{i=1}^{n} i
let s_eval = to_f64(s);        // 此时才数值求值（需 n 已绑定具体值）
```

## 十、求值语义与优化

### 10.1 求值语义

- **符号求值**：`f(x) = x^2 + 6; f(0)` → 化简后精确结果，不自动数值化。
- **数值求值**：经 §九 坍缩后。
- **循环公式优化**（`loop_optimization := true` 默认开）：`sum(1..n) i → n(n+1)/2`。

**示例**：

```prima
let f(x) = x^2 + 6;
let a = f(sqrt(2));       // → Expr: (sqrt(2))^2 + 6 → 2 + 6 → 8（符号化简）
let b = f(3.0);           // → F64: 15.0（数值计算）

// 循环优化
config { loop_optimization := true }
let s = 0;
for i in 1..100 {
    s += i;               // 编译器识别模式，转换为 s = 100*101/2
}
```

### 10.2 优化系统（现代语言优化）

**原则**：优化全部**自动**发生，不向开发者暴露逐函数优化指令（无 `#[inline]` 之类注解）；全局/局部优化强度由 **`opt_level` 策略**控制（§13.2）。`@parallel`/`@jit`/`@gpu` 是**并行/执行模型**注解，不是优化指令；`@builtin(O1)`（§18.4）是**实现分层**注解，受 `opt_level` 影响但不替代它。

**优化等级（v2.2，策略 `opt_level`）**：`O0`–`O3`，默认 `O2`。每个等级是**递增通道集合**，`opt_level := On` 启用等级 `≤ n` 的全部通道：

| 等级 | 启用的优化通道 |
|------|----------------|
| `O0` | 无优化管道：逐条解释、保留循环原语义；`simplify_level`/`fraction` 等**符号与数值语义**策略仍生效；`loop_optimization`/JIT 自动编译不生效 |
| `O1` | 基础通道：常量折叠/传播、死代码消除（DCE）、循环闭式公式（§10.1）；`@builtin(O1)` Rust 实现启用 |
| `O2`（默认） | `O1` + 公共子表达式消除（CSE）、自动内联（启发式）、尾调用优化（TCO）、JIT 自动热点编译（§19.2） |
| `O3` | `O2` + 激进通道：SIMD 识别与向量化、循环展开、无条件内联小函数、`@builtin(O3)` 实现启用（§18.4） |

**优化通道详述**：

1. **常量折叠/传播**（constant folding/propagation）：字面量与 `const` 的编译期求值。
2. **死代码消除**（dead code elimination）：不可达分支、无用赋值剔除。
3. **公共子表达式消除**（CSE）：重复子表达式只算一次。
4. **循环优化**：闭式公式（§10.1）、循环不变量提升（hoisting）、向量化预判。
5. **自动内联**（automatic inlining）：对**满足启发式的纯/小函数**自动内联展开——阈值由编译器内部判定（如调用次数、函数体规模、无副作用），**开发者不可干预**。内联不改变可观察语义（包括错误时机与 `Result` 传播）。
6. **尾调用优化**（TCO）：尾部递归/尾调用栈复用（纯函数与 `fn` 均可）。
7. **SIMD 识别与向量化**（`O3`）：对稠密数值数组的逐元素运算（广播、循环内元素操作）识别为可向量化模式，映射到 SIMD 指令或按块并行；仅在可证明数值语义不变时应用（含 IEEE 舍入例外按 `print_format`/精度策略保守处理）。
8. **化简等级**：与 §8.3 的化简系统协同，`simplify_level` 策略控制符号化简深度，数值优化在其后。

**内联规则**：

- 内联对象：纯数学函数（MFn）与无副作用宿主函数。
- 禁止内联：含 `@parallel` 副作用、递归函数、体积超阈值函数。
- 内联在**类型检查之后**、代码生成之前进行（不影响错误诊断定位，诊断仍以源码位置为准）。

**与既有策略的协同**：`loop_optimization := false` 在任意等级下都显式关闭循环闭式公式；`broadcast`/`fraction` 等语义策略优先于优化等级；`opt_level` 只决定**编译器自动施加的优化**，不改变可观察语义（结果、错误时机、`Result` 传播）。

---

## 十一、函数、数组与广播

### 11.1 纯数学函数（MFn）

```prima
let f(x) = x^2 + 1;       // 纯函数，默认符号世界
let g(x): F64 = to_f64(x^2);  // 显式声明返回类型
```

**特性**：纯、无副作用、可化简、可组合、一等公民、支持自动微分（§19.4）。可 `@parallel` 注解（§十七）。自动内联优先对象（§10.2）。

### 11.2 功能函数（fn）

```prima
fn process(x: F64) -> F64 {
    print(f"Processing: {x}");
    return x * 2.0;
}
```

**特性**：可有副作用、控制流、I/O。可返回 `Result`（§16.3）。

### 11.3 数组（Array，v2.1 可变长序列）

**`Array` 是可变长度、可变的同质或异质序列**（v2.1，效仿 Python `list`）：长度可增长/收缩，元素可为任意值（数字/字符串/布尔/`Expr`/类实例/嵌套数组等）。广播与矩阵接口仍要求**同质数值数组**（§11.4 调用点校验）。

#### 字面量与构造

```prima
let v = [1, 2, 3];            // Array：可含任意值
let w = [1.0, 2.0, 3.0];
let m = ["a", "b"];           // Array<String>
let nested = [[1, 2], [3, 4]]; // v2.1 合法：作为数据的嵌套数组（广播仍拒绝，§11.4）
let e = Array::new();         // 空数组（可变长）
let f = [x^2 for x in range(0, 5)];  // 推导式（§11.7）
```

#### 索引与切片（含负索引）

```prima
let v = [10, 20, 30, 40];
let a = v[0];                 // → 10
let b = v[-1];                // → 40（负索引从末尾数，越界报 R0003）
let c = v[1..3];              // → [20, 30]（切片，左闭右开）
let d = v[..2];               // → [10, 20]
let e = v[2..];               // → [30, 40]
let f = v[-2..];              // → [30, 40]
```

#### 切片赋值（v2.1）

```prima
let v = [1, 2, 3, 4];
v[1..3] = [20, 30];           // v == [1, 20, 30, 4]
v[0..1] = [];                 // 删除元素：v == [20, 30, 4]
```

#### 拼接与成员测试

```prima
let a = [1, 2];
let b = [3, 4];
let c = a + b;                // → [1, 2, 3, 4]（拼接）
a += b;                       // a == [1, 2, 3, 4]（原地扩展，等价 extend）
let has2 = 2 in c;            // → true（成员测试，`in` 运算符）
let has5 = 5 in c;            // → false
```

#### 可变方法（在持有者可变绑定上调用）

```prima
let mut v = [1, 2, 3];
v.push(4);                    // [1, 2, 3, 4]
let last = v.pop();           // → 4（Some）；v == [1, 2, 3]
v.append(5);                  // 追加单个元素
v.extend([6, 7]);             // 追加序列
v.insert(0, 0);               // 头部插入：v == [0, 1, 2, 3, 5, 6, 7]
let removed = v.remove(0);    // → 0（删除并返回）；v 前移
v.clear();                    // v == []
```

#### 只读方法与便捷函数

```prima
let v = [3, 1, 2];
v.len()                       // → 3
v.is_empty()                  // → false
v.get(1)                      // → Some(1)（安全索引，§4.4）
v.contains(2)                 // → true（等价 `2 in v`）
v.index(2)                    // → 1（元素下标，找不到报 R0013）
v.count(2)                    // → 1（出现次数）
v.first()                     // → Some(3)
v.last()                      // → Some(2)
let s = sorted(v);            // → [1, 2, 3]（新数组）
let r = reversed(v);          // → [2, 1, 3]
let total = sum(v);           // → 6
let prod_v = prod(v);         // → 6
let m = min(v);               // → 1
let M = max(v);               // → 3
```

#### 越界处理

```prima
let v = [1, 2, 3];
let x = v[10];                // 运行时错误 R0003：索引越界
let y = v.get(10);            // → None（安全访问，Option）
let z = v.get(1);             // → Some(2)
```

#### 矩阵构造

```prima
let M = Matrix::from_rows([[1, 2], [3, 4]]);  // 2×2 矩阵
let N = Matrix::zeros(3, 3);                  // 3×3 零矩阵
let I = Matrix::identity(4);                  // 4×4 单位矩阵

// 矩阵索引
let A = Matrix::from_rows([[1, 2, 3], [4, 5, 6], [7, 8, 9]]);
let e = A[0, 1];              // → 2（单元素）
let f = A[0, ..];             // → [1, 2, 3]（第 0 行）
let g = A[.., 1];             // → [2, 5, 8]（第 1 列）
let h = A[0..2, 1..3];        // → [[2, 3], [5, 6]]（子矩阵）
```

### 11.4 广播（Broadcast）

**规则**（v2.1 收紧为「数值同质数组」）：

- **仅作用于同质数值数组**：广播要求数组元素为 `Number`；元素含非数值（字符串/数组/类等）即**报错**（`R0009`），不静默降级。
- **拒绝嵌套数值数组**：广播遇到「数组的数组」**报错**，不递归；一般嵌套数组仅作数据（§11.3）不参与广播。
- **空数组报错**：广播遇到空数组即报错（`R0014`），不产生静默空结果。
- **默认广播** `broadcast := true`（默认）：纯函数传数组逐元素；`false` 时需显式 `map` 或广播算子。
- **并行广播**：`@parallel` 纯函数 + 大数组自动走 rayon 并行（§17.4）。

**示例**：

```prima
config { broadcast := true }

let f(x) = x^2;
let v = [1, 2, 3];
let w = f(v);                // → [1, 4, 9]（自动广播）

// 二元运算广播
let a = [1, 2, 3];
let b = [10, 20, 30];
let c = a + b;               // → [11, 22, 33]

// 标量广播
let d = a * 10;              // → [10, 20, 30]

// 错误示例
let g = f(["a", "b"]);       // 错误 R0009：数组元素非数值
let h = [];
let i = f(h);                // 错误 R0014：空数组
```

**显式控制**（`broadcast := false` 时）：

```prima
config { broadcast := false }

let f(x) = x^2;
let v = [1, 2, 3];
let w = map(f, v);           // 显式 map
let x = v @. f;              // 广播算子（语法糖）
```

### 11.5 函数上下文

- 纯函数体 → W_symbol（表达式形态，保持精确）。
- 功能函数体 → 默认数值，显式坍缩按需。

### 11.6 映射 `Dict` 与集合 `Set`（v2.1）

`Dict` 是键 → 值的可变映射（效仿 Python `dict`），`Set` 是去重的可变集合（效仿 Python `set`）。两者均要求键/元素**不可变且可哈希**（`Number`/`String`/`Char`/`Bool`/`Expr`/`Symbol`）。

#### 字面量与构造

```prima
let d = { "a": 1, "b": 2 };   // Dict：{ key: value }
let d0 = Dict::new();         // 空 Dict
let s = {1, 2, 3, 2};         // Set：去重 → {1, 2, 3}
let s0 = Set::new();          // 空 Set
let t = {};                   // 空花括号 → 空 Dict
```

#### Dict 索引与成员测试

```prima
let d = { "a": 1, "b": 2 };
let a = d["a"];               // → 1
let missing = d["x"];         // 运行时错误 R0012：键不存在
let m = d.get("x");           // → None（安全访问）
let m2 = d.get("a");          // → Some(1)
let has = "a" in d;           // → true（成员测试）
let n = d.len();              // → 2（条目数）
```

#### Dict 方法

```prima
let d = { "a": 1 };
d["b"] = 2;                   // 插入/更新键（元素赋值）
d.insert("c", 3);             // 等价 d["c"] = 3
let v = d.remove("a");        // → Some(1)；键不存在 → None
d.clear();
d["a"] = 1;
let keys = d.keys();          // → ["a"]（Array，任意序）
let vals = d.values();        // → [1]
let items = d.items();        // → [("a", 1)]（Tuple 数组）
let dd = d.update({ "x": 9 }); // 合并：d + { "x": 9 }（后者覆盖前者）
```

#### Set 方法与集合代数

```prima
let s = {1, 2, 3};
s.add(4);                     // 添加元素
s.remove(2);                  // 删除元素；不存在则报 R0013
s.discard(99);                // 删除元素；不存在静默
let c = s.contains(1);        // → true（等价 `1 in s`）
let n = s.len();              // → 元素个数
let u = s ∪ {5, 6};           // 并集（∪ 为 Set 专属算符）
let i = s ∩ {2, 3};           // 交集
let diff = s \ {3};           // 差集
```

#### 迭代与通用便捷

```prima
for k in d.keys() { print(k); }      // 遍历键
for (k, v) in d.items() { ... }      // 遍历键值对
for x in s { ... }                   // 遍历集合

let n = len(d);                      // 等价 d.len()
let m = len("hello");                // → 5（`len` 为多态便捷函数，§18.1）
let e = enumerate(["a", "b"]);       // → [(0, "a"), (1, "b")]（Tuple 数组）
let z = zip([1, 2], ["a", "b"]);     // → [(1, "a"), (2, "b")]
let all = all([true, true]);         // → true
let any = any([false, true]);        // → true
```

### 11.7 推导式与迭代协议（v2.1）

推导式把「构造 + 过滤 + 映射」写成一个表达式（效仿 Python）。语法：`<外框> <元素表达式> for <变量> in <可迭代> [if <条件>]`，可多重 `for`（笛卡尔积）。

```prima
let squares = [x^2 for x in range(0, 10)];              // Array：[0, 1, 4, ..., 81]
let evens   = [x for x in range(0, 10) if x % 2 == 0];  // 带过滤
let pairs   = [(x, y) for x in range(0, 2) for y in range(0, 2)];  // 嵌套
let table   = {x: x^2 for x in range(0, 5)};            // Dict 推导式
let odds    = {x for x in range(0, 10) if x % 2 == 1};  // Set 推导式

let n = len(squares);         // → 10
```

**可迭代对象**：`Array`、`Dict`（键）、`Set`、`range`、`String`（字符序列）、`Tuple`。`for`/`parfor`/`in`/推导式统一使用迭代协议。

---

## 十二、变量、作用域与所有权

### 12.1 变量与常量

```prima
let a = sqrt(2);              // 变量：符号默认保留；标量可变
let mut b = 0;                // 显式可变（需 mut 关键字）
const c: Expr = \e^{i\pi};    // 常量：类型必须标注，不可变、可内联
let d: Number = 0;            // 显式类型注解
```

**可变性规则**（v2.1）：

- **数值标量**：`let mut` 作用域内可变。
- **集合**（`Array`/`Dict`/`Set`）：**可变宿主值**——长度/内容可变（`push`/`pop`/`d[k]=v`/`add`/…，§11.3/11.6）；可变方法要求绑定为 `let mut`（`let` 绑定也可就地调用只读方法）。
- **复合数学值**（`Expr`/`Matrix`/`Symbol`）：默认不可变，共享引用。
- **常量**：全局不可变、编译期可内联。

### 12.2 作用域与可见性

- 块作用域（`{}`）遮蔽外层同名变量。
- `let (a, b) = tuple;` 等**不可反驳模式解构**创建多个绑定（§4.4）。
- 模块间变量不互通（§十五）：使用某模块公开项需 `import`。

### 12.3 所有权（Class 语义）

**类实例所有权**（GC 句柄语义；需要确定性生命周期时可用 `mem::Arc`（§12.4））：

- **默认值语义**：类实例由宿主层 **GC** 管理；**赋值、传参、返回实例**均为**浅拷贝**（共享底层对象的句柄，无计数开销）。
- **`self` 参数**：方法接收 `self` 即接收**对象本身的浅拷贝**；方法内对 `self` 字段的读取为共享读。
- **深拷贝**：当方法**返回基本值字段**（`Expr`/`Number`/`String` 等标量/不可变值）时，传出的值独立持有（这些基本值本身不可变，深拷贝即复制句柄/缓冲）。返回类实例则保持共享。
- **结构字面量**：`Test { a, b }` 创建新实例（拥有新字段值）。
- **无手动所有权语法**：不暴露 `&` / `move` 等 Rust 风格借用语法（保留 `let mut` 表明可变绑定）。可变类字段修改可通过显式构造新实例或 `mut self` 方法实现。
- **GC 对语义透明**：GC 自动回收不可达实例（含**循环引用**）；不暴露析构时机。需要**确定性释放/析构钩子**或跨 FFI 保持存活时，使用标准库 `mem::Arc`（显式引用计数，`mem::Arc::new(x)`/`x.strong_count()`）。
- **`ExprId` 是 `Copy` 值句柄**，自然共享；底层 `ExprPool` 只读并发安全。

### 12.4 内存策略

 层 | 策略 |
----|------|
 W_symbol | hash-consing `ExprPool`（线程本地缓存 + 全局 `DashMap`，可共享、无环、O(1) 相等） |
 W_numeric | 栈值 + nalgebra + 批量算法（BLAS） |
 W_host | **GC（追踪式，标记-清除/分代）**管理类实例与宿主值（浅拷贝共享 + 基本值深拷贝）；`String` 缓冲内联/SSO；`mem::Arc` 供显式引用计数 |

**GC 设计要点（v2.2，§实现方案 Phase 12）**：

- GC 作用域：**宿主世界（W_host）** 的类实例与集合缓冲；符号层（`ExprPool`）与数值层（栈值）不参与。
- 触发：求值器内可安全收集点（块/函数/循环边界）按水位触发，不引入异步扫描线程（单线程 GC，确定性优先）。
- 根集：环境链 `EnvRef` + 当前求值栈 + 模块表；从根追踪 `Value` 图可达的实例。
- 无析构陷阱：GC 实例不提供析构；需要确定性释放用 `mem::Arc`。
- 并行求值（`parfor`/`@parallel`）每任务独立 GC 堆，任务结束整堆回收。

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

**必须**声明于 `src/main.pra` 最上方，在非入口文件声明则**编译报错**（`E0021`）。

 策略 | 类型 | 默认 | 说明 |
------|------|------|------|
 `domain` | enum | `complex` | 默认定义域（`complex` / `real`），影响幂运算等 |
 `undefined_handling` | enum | `strict` | `Undefined` 行为（`strict` 报错 / `custom { 0/0 := 1 }` 黑魔法） |

#### 模块/局部策略

可在模块顶部或局部 `with config` 声明。

 策略 | 类型 | 默认 | 说明 |
------|------|------|------|
 `fraction` | bool | `true` | 有理数偏好分数 vs 浮点 |
 `broadcast` | bool | `true` | 纯函数自动逐元素（v2.1：仅限数值同质数组，拒嵌套/空数组，§11.4） |
 `loop_optimization` | bool | `true` | 循环闭式公式优化 |
 `opt_level` | enum | `O2` | 优化等级（`O0`/`O1`/`O2`/`O3`，v2.2，§10.2）：编译器自动施加的优化通道集合 |
 `simplify_level` | int 0-3 | `2` | 默认化简等级（符号层，独立于 `opt_level`） |
 `num_to_big` | bool | `true` | 整数溢出自动升级 BigInt（否则报错） |
 `print_format` | enum | `latex` | 打印格式（`latex` / `unicode` / `ascii`） |
 `overload_policy` | enum | `warn` | 运算符重载使用策略：`warn`（默认，带 `W0005` 警告）/ `allow`（解除）/ `deny`（报错），§18.5 |

### 13.3 策略使用示例

#### 全局策略（入口文件）

```prima
// src/main.pra
config {
    domain := complex              // 全局默认复数域
    undefined_handling := strict   // 严格模式
}

import mymath;

let a = (-1)^0.5;                  // 正确 → \i（全局 complex 域）
```

#### 模块策略

```prima
// src/numerical.pra
config {
    fraction := false              // 本模块优先浮点
    simplify_level := 3            // 本模块高级化简
}

let compute(x) = x / 3;            // → F64(x / 3.0)
```

#### 局部策略

```prima
config { domain := complex }       // 模块级：复数域

let f(x) = x^2;

with config { domain := real } {   // 局部切换到实数域
    let y = (-1)^0.5;              // 错误：实数域下负数开方非法
}

let z = (-1)^0.5;                  // 正确 → \i（回到模块级 complex 域）
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
    total += i;
}

// 带步长
for i in 0..10 step 2 {
    print(i);                      // 0, 2, 4, 6, 8
}

// 显式并行循环
parfor i in 0..n {
    A[i] = compute(i);             // 迭代体必须无副作用
}

// while 循环
while cond {
    // ...
}

// if / else if / else
if x > 0 {
    print("positive");
} else if x < 0 {
    print("negative");
} else {
    print("zero");
}

// return
fn f(x) -> F64 {
    if x < 0 {
        return 0.0;
    }
    return x * 2.0;
}
```

**模式解构控制流**（§4.4）：

```prima
// if let
if let Some(x) = v.get(0) {
    print(f"first: {x}");
}

// while let
while let Some(x) = iter.next() {
    print(x);
}

// match（表达式，全模式）
let kind = match x {
    0        => "zero",
    1 | 2    => "small",
    3..=9    => "medium",
    n if n > 100 => "large",
    _        => "other"
};
```

**`?` 运算符**（§16.3）：仅在返回 `Result` 的函数内传播错误。

**规则**：

- 控制流变量**默认数值**，**允许符号**（策略/显式标记）。
- 循环公式优化默认开（`loop_optimization := true`）。
- `match` 是表达式（可作右值）；`if`/`while` 是语句。
- `try/catch` **不存在**（v2.0 移除，§16.3）。

---

## 十五、模块与导入系统

**哲学**：`import` 语法（Python 风格）+ 编译单元/可见性/路径（Rust 风格）+ 模块间变量不互通。

### 15.1 导入语法（`import` 统一）

```prima
import core;                    // 引入命名空间
import linalg as la;            // 别名
from stats import mean, std;    // 选择性导入
from mymath import *;           // 通配（不推荐）
```

### 15.2 模块 / 编译单元（Rust 结构）

- **模块是编译单元**，独立作用域。
- **默认私有**：所有项默认私有，跨模块需显式 `pub`。
- **变量不互通**：`import` 把**公开项**引入命名空间，不共享内部状态。
- **嵌套模块路径** `a::b::c`。
- **可见性修饰符**：
  - `pub`：公开，跨模块可见。
  - `pub(mod)`：当前模块可见（等价 Rust `pub(crate)` 语义的模块级：对**本模块**内所有项可见，对外不可见）。
  - 无修饰：所在**类**/作用域私有。

**示例**：

```prima
// src/math_utils.pra
pub let square(x) = x^2;        // 公开函数
let helper(x) = x + 1;          // 私有函数

pub const PHI: Rational = (1 + sqrt(5)) / 2;  // 公开常量

pub class Vec2 {                // 公开类
    x: F64,
    y: F64,
    pub fn new(x: F64, y: F64) -> Self {
        Vec2 { x, y }
    }
    pub(mod) fn norm(self) -> F64 {      // 仅本模块可见
        sqrt(self.x^2 + self.y^2)
    }
}
```

```prima
// src/main.pra
import math_utils;

let a = math_utils::square(3);  // 正确
let b = math_utils::helper(3);  // 错误：helper 是私有的（E0032）
let c = math_utils::PHI;        // 正确
let v = math_utils::Vec2::new(3.0, 4.0);
let n = v.norm();               // 错误：norm 是 pub(mod)，main 模块不可见（E0032）
```

### 15.3 文件映射

- **一个 `.pra` 文件 = 一个模块主体**。
- **一个目录 = 一个子模块**，其 `main.pra` 为该目录模块入口（仿 Rust `mod.rs`）。
- **项目入口 = `src/main.pra`**（根模块）。

**示例目录结构**：

```text
src/
├── main.pra               // 根模块
├── physics.pra            // 模块 physics
└── linalg/                // 子模块 linalg
    ├── main.pra           // linalg 模块入口
    └── fft.pra            // linalg::fft 子模块
```

```prima
// src/main.pra
import physics;
import linalg;
import linalg::fft;
```

### 15.4 导入冲突

同名符号从多个模块导入：冲突时报错，需别名或 `::` 限定消除。

```prima
from math import sin;
from custom_math import sin;    // 错误：sin 冲突（E0031）

// 解决方案 1：别名
from math import sin as std_sin;
from custom_math import sin as my_sin;

// 解决方案 2：限定访问
import math;
import custom_math;
let a = math::sin(x);
let b = custom_math::sin(x);
```

### 15.5 预导入

- **预导入 `core`**，且 **`core` 常用功能（内置符号、坍缩函数族、基础算子、f-string（§18.1）、`Option`/`Result` 变体）全部暴露**。
- 其余模块（`linalg`/`stats`/`plot`/`render`/`math`/`io`/`parallel`/`physics`/`sys`/`time`/`num`/`ops`/`c_api`/`mem`）必须显式 `import`。

**`core` 预导入内容**：

- 数值类型：`Integer`、`Rational`、`F64`、`Complex`、`Expr`、`Symbol`、定宽类型名
- 坍缩函数：`to_*`/`try_*`/`checked_*`/`clamped_*`/`rounded_*` 全族（§九）
- 内置符号：`\e`、`\pi`、`\i`、`\infty`、`\gamma`、`\phi`
- 基础算子：`sqrt`、`sin`、`cos`、`log`、`exp` 等
- 化简函数：`simplify`、`limit`、`derivative`、`partial`、`grad`（§19.4）
- 集合：`Array`/`Dict`/`Set` 及方法（§11）、`len`/`enumerate`/`sorted`/`reversed`/`sum`/`prod`/`min`/`max`/`all`/`any`/`zip`/`join` 等便捷函数
- 控制台：`print`（不换行）、`println`（换行）、`input`、`read_line`（§18.1b）
- 工具函数：`range`、`map`、`filter`、`Some`、`None`、`Ok`、`Err`；字符串格式化使用 **f-string**（§18.1），`format` 函数已移除（调用得 `W0006`）

---

## 十六、错误处理与警告系统

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

    // 键不存在（Dict/Set，v2.1，§11.6）
    KeyNotFound {
        key: String,
        location: SourceLocation,
    },

    // 元素/键不存在（v2.1，§11.3/11.6）
    NotFound {
        value: String,
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
 **编译期错误** | 语法、类型、导入、可见性、可静态判定的 `Undefined` | 编译失败，编号 `E####`，提供详细诊断（§16.4） |
 **可恢复错误** | 溢出、越界、I/O、坍缩失败、`Undefined` 参与运算 | 以 `Result<T, Error>` 返回（编号 `R####`），由调用方 `match`/`?`/`unwrap` 处理 |
 **兜底 panic** | 显式 `to_*`/`unwrap`/`expect` 触发；无法恢复的内部错误 | 带跨语言堆栈，终止程序 |

### 16.3 错误处理语法（Rust 式，无 try/catch）

**`Result<T, Error>`** 是唯一的一等错误表示；`Option<T>` 表示可能缺失的值。

#### match 处理

```prima
let result = try_i32(1e20);
match result {
    Ok(n) => print(f"success: {n}"),
    Err(e) => print(f"failed: {e}")
}
```

#### `?` 运算符（错误传播）

```prima
// ? 只能在返回 Result/Option 的函数体内使用
fn parse_and_double(s: String) -> Result<F64, Error> {
    let v = try_f64(s)?;          // Err 则立即 return Err(...)
    return Ok(v * 2.0);
}

fn first(list: Array) -> Option<Integer> {
    let x = list.get(0)?;         // None 则立即 return None
    return Some(x);
}
```

#### unwrap 家族（显式放弃错误，panic 兜底）

```prima
let a = try_i32(100).unwrap();              // 失败则 panic
let b = try_i32(1e20).unwrap_or(0);         // 失败返回默认值
let c = try_i32(1e20).expect("conversion failed");  // 自定义 panic 消息
```

#### 安全访问（Option）

```prima
let v = [1, 2, 3];
if let Some(x) = v.get(1) {
    print(x);                    // 2
}
let y = v.get(10).unwrap_or(0);  // 0
```

**规则**：

- `?` 只允许出现在返回 `Result`/`Option` 的函数内，否则编译期错误（`E0054`）。
- `to_*` 家族直接 panic，不返回 `Result`；需要错误处理请用 `try_*`。
- **`try/catch` 语法在 v2.0 中移除**：解析即报编译期错误（`E0010` 语法错误，提示改用 `Result`）。

### 16.4 诊断格式

**格式**：**编号 + 来源定位（文件:行:列）+ 相关表达式（LaTeX）+ 可恢复建议**。

**错误示例**：

```text
error[E0050]: type mismatch
  --> src/main.pra:15:9
   |
15 |     let x: F64 = sqrt(2);
   |               ^^^^^^^^ expected F64, found Expr
   |
   = help: use to_f64(sqrt(2)) to collapse explicitly
   = expression: √2
```

```text
error[R0005]: invalid operation in real domain
  --> src/numerical.pra:42:18
   |
42 |     let z = (-1)^0.5;
   |                  ^^^ cannot take square root of a negative number in the real domain
   |
   = help: switch to complex domain with `with config { domain := complex } { ... }`
         or use with_domain((-1)^0.5, Complex)
   = expression: (-1)^{1/2}
```

**警告示例**（§16.5）：

```text
warning[W0001]: statements separated by newlines are deprecated
  --> src/main.pra:8:12
   |
 8 |     let b = 2
   |              ^ use `;` to terminate statements; newline separation will be removed
   |
   = help: replace the trailing newline with `;`
```

**方法调用错误的文档 note（v2.2）**：当**方法调用**（`obj.method(...)`）失败时——无论失败原因是编译期（未知方法、参数个数/类型不符）还是运行时（方法内抛错）——诊断必须在 note 中附带**该方法的相关定义与文档注释**（§4.1）：

- **方法定义**：完整签名（含参数类型与返回类型）与定义位置（`file:line:col`）。
- **文档注释**：该方法的 `///` 文档文本；若无则附所属类（或模块）的文档文本。
- 标准库方法（§18.1/§18.4）的文档同样来自内嵌 `.pra` 模块的 `///` 注释，可离线查看（`prima doc`，§二十）。

```text
error[E0040]: undefined name `toUpperCase`
  --> src/main.pra:20:14
   |
20 |     print(name.toUpperCase());
   |              ^^^^^^^^^^^^ no method named `toUpperCase` on `String`
   |
   = note: method `to_upper(self) -> Self` defined at core/string.pra:42:5
           /// Returns a copy of `self` with every character uppercased.
   = help: did you mean `to_upper`?
```

### 16.5 警告系统

**原则**：警告不阻止编译/执行，但标注「不符合规范/弃用」的用法。每条警告有唯一编号 `W####`，英文短名 + 说明，记录于**附录 C**。

**现有警告**：

 编号 | 名称 | 含义 |
------|------|------|
 `W0001` | `newline_statement_separator` | 使用换行分隔语句（弃用，改用 `;`，§4.2） |
 `W0002` | `deprecated_pipeline` | 使用 `\|>` 管道（弃用，改用类方法，§9.7） |
 `W0003` | `unused_binding` | `let` 绑定未被使用 |
 `W0004` | `unreachable_code` | 不可达代码 |
 `W0005` | `overloaded_operator` | 使用运算符重载（§18.5；`overload_policy := warn` 默认触发，`allow` 解除） |
 `W0006` | `deprecated_format` | 调用已移除的 `format` 函数（改用 f-string，§18.1；过渡期警告，目标版本移除） |

**规则**：

- 警告在诊断通道输出，不影响退出码；`prima check` 可选择 `--deny W0001` 将警告升级为错误（工具层）。
- 警告可通过策略解除（如 `overload_policy := allow` 解除 `W0005`），**不提供逐条 `allow` 注解**（避免噪音）。
- 弃用型警告（`W0001`/`W0002`/`W0006`）在目标版本移除对应语法后随之删除。

## 十七、并行与多线程

**哲学**：**无隐式并行，并行必须显式**。语言绝不隐式使用线程/`rayon`；是否并行由用户明确指定。

### 17.1 语法：`@parallel` 注解

```prima
let f(x): MFn @parallel = x^2;          // 纯函数并行（安全）

// 实验性特性（暂不转正）
// fn process(x) @parallel { ... }     // 功能函数并行（需手动保证线程安全）
```

**规则**（v2.1 定稿）：

- `@parallel` 仅可标注**纯数学函数**（安全，编译器验证无副作用）。
- `@parallel` 函数体必须**自包含**：只依赖形参与内置数学符号/常数，不得引用外部自由变量（并行子任务各自求值，无共享环境）。
- 调用点在**广播上下文**（数组参数）下并行：数组长度 ≥ 阈值（默认 1024）时按线程数分块交给 rayon；小数组走顺序路径（避免开销）。
- 功能函数并行标记为 `[EXPERIMENTAL]`，需用户手动保证线程安全。

### 17.2 `parfor` 显式并行循环

```prima
parfor i in 0..n {
    A[i] = compute(i);              // 迭代体必须无副作用
}

// 带步长
parfor i in 0..n step 2 {
    B[i] = heavy_computation(i);
}
```

**规则**（v2.1 定稿）：

- 仅限**无副作用**迭代体，否则编译报错（`E0082`）。允许的语句形态：对**索引槽**的赋值（`A[i] = …`/`A[i] += …`，`i` 为循环变量或其纯函数，越界报 `R0003`）与纯函数调用；禁止 `print`、外部变量赋值、`let` 绑定、类实例修改等。
- 结果写入：各数组槽位独立计算，结束后整数组回写绑定（rayon 并行）。
- 底层使用 `rayon` 线程池，粒度自动调优。

### 17.3 线程安全保证

- **数学不可变值**（`ExprId` + `ExprPool`）只读共享，天然线程安全。
- **可变 `W_host` 状态**并行需用户显式管理原语（如 `Mutex`、`Atomic`）。
- **类实例**在并行上下文中只读共享安全；写操作需显式同步。
- **模块边界隔离**可变状态，降低并发风险。

### 17.4 并行示例

#### 并行纯函数广播

```prima
let f(x) @parallel = x^2 + sin(x);
let v = range(0, 1000000);
let w = f(v);                       // 自动并行广播（broadcast + @parallel）
```

#### 并行矩阵运算

```prima
parfor i in 0..rows {
    for j in 0..cols {
        C[i, j] = dot(A[i, ..], B[.., j]);
    }
}
```

---

## 十八、标准库规划

> **方法级清单的管理原则（v2.2）**：各模块**具体实现哪些函数/方法/Class，以该模块内嵌 `.pra` 源码的 `///` 文档注释为准**，规范与实现文档不再逐条枚举；本表维护**模块目录与职责范畴**，§18.1/附录 B 保留核心示意清单。

 模块 | 内容 | 导入 |
------|------|------|
 **core** | 数字塔、ExprDAG、化简、内置符号、坍缩函数族、基础算子、f-string（§18.1）、`Result`/`Option`、`String` 类 | **预导入，常用全暴露** |
 **linalg** | 矩阵、线性代数（nalgebra/faer）、求解器 | 显式 |
 **stats** | 统计基础（均值、方差、分位数、分布） | 显式 |
 **plot** | 绘图（SVG 起步，可选 plotly 后端）；科学绘图（线/散点/柱/等高线/热图） | 显式 |
 **render** | **公式渲染（v2.2）**：TeX/LaTeX 表达式 → SVG/PNG/终端文本（复用 ExprDAG 渲染器，§7）；`print` 输出公式的可选终端渲染以 cargo feature 提供 | 显式 |
 **math** | 特殊函数（贝塞尔、伽马、超几何）、数值积分、ODE、FFT、**数值工具（v2.2）：因式分解、素数筛、Taylor 展开、多项式运算等** | 显式 |
 **io** | 文件 I/O、序列化（JSON/CSV/HDF5）、格式化输出 | 显式 |
 **parallel** | 并行原语（`parfor` 辅助、线程池配置、任务调度） | 显式 |
 **physics** | 物理常数（CODATA 2022）、单位系统（可选）、**常用公式（v2.2：Rust 实现，便于快速优化，§18.6）** | 显式 |
 **symbolic** | 高级符号操作（求导、积分、级数展开、方程求解） | 显式 |
 **optimize** | 优化算法（梯度下降、牛顿法、BFGS、约束优化） | 显式 |
 **sys** | 底层系统操作：`sys::path`（跨平台路径）、`sys::env`（环境）、`sys::os`（平台特定）、**v2.2 扩展：进程、文件系统元数据、终端** | 显式 |
 **time** | 时间系统：`now`、`Duration`、格式化、时钟 | 显式 |
 **num** | 更复杂数字类型（`BigInt` 算法扩展、`Complex` 工具）与额外数字运算 | 显式 |
 **ops** | 运算符重载接口（`impl Add for T` 等，§18.5） | 显式 |
 **mem** | **内存（v2.2）**：`mem::Arc` 显式引用计数（§12.3/12.4）；GC 控制（`collect`） | 显式 |
 **c_api** | C ABI 类型（`int`/`uint`/`float`/`double`/`bool`/`char`/`ptr`…）与 `@c_api::extern` 导出支持（§18.4） | 显式 |

### 18.1 字符串（core 预导入 `String` 类与 f-string）

**字面量（v2.2）**：

- **普通字符串**：`"..."` 与 `'...'` **等价**，均支持转义（含 `\u{XXXX}` Unicode 转义）。
- **原始字符串**：`r"..."` / `r'...'`——**不处理转义**（`\n` 是字面反斜杠 + `n`，`\u{XXXX}` 不展开）。
- **f-string**：`f"..."` / `f'...'`——`{expr}` 插值、`{:spec}` 精化、`{{`/`}}` 转义；可与原始字符串组合（`rf"..."`）。

**转义序列**：`\n` `\t` `\r` `\\` `\"` `\'` `\0` `\a` `\b` `\f` `\v` `\u{XXXX}`（任意 Unicode 码点）。

**f-string 规则**（取代 v2.1 的 `format` 函数）：

```prima
let s = f"a is {a}";                    // 表达式插值
let t = f"{x} + {y} = {x + y}";
print(f"value = {v}");                  // 所有可打印值均可插值
let u = f"{{literal braces}}";          // {{ → {，}} → }
let w = f"{pi:0.2}";                    // 格式精化：浮点精度 3.14
```

- `{...}` 内为任意 Prima 表达式（**v2.2 不允许嵌套 f-string 字面量**，嵌套即编译期错误）。
- 插值表达式按 `print_format` 策略渲染（默认 LaTeX）；参数为 `Result`/`Option` 时显示其成功/失败摘要。
- 格式精化 `{:spec}` 逐步扩充（浮点精度 `{x:0.2}`、对齐、填充等），语法与支持项以 `.pra` 模块文档注释为准。
- **`format` 函数已移除**：调用名为 `format` 的函数产生过渡警告 `W0006` 并提示改用 f-string（§16.5）；目标版本移除警告后按未定义名报错。

**`String` 类**（内嵌 `core/string.pra`，v2.2）：方法集**以 Python 3 稳定 `str` 方法为参照**，适配 Prima 习惯（大小写转换、查找/替换、拆分/连接、填充对齐、Unicode、切片/迭代、序列化转换等）。**具体方法清单与使用文档以内嵌 `.pra` 模块的 `///` 文档注释为准**（可通过 `prima doc` 或诊断 note 查看，§4.1/16.4），规范不再逐条枚举，此处仅列示意：

```prima
pub class String {
    /// Returns the number of Unicode scalar values in `self`.
    pub fn len(self) -> Integer
    pub fn from(value) -> Self
    pub fn split(self, sep: Self) -> Array<String>
    pub fn to_upper(self) -> Self
    // ... 完整方法集见 core/string.pra 的文档注释
}
```

- **性能分层**：热点方法（`split`/`replace`/`to_upper`/`to_lower` 等）以 `@builtin(O1)`/`@builtin(O2)` 提供 Rust 实现；低频方法直接以 `.pra` 书写（§18.4 分层优化机制）。

### 18.1b 控制台输出与输入（core 预导入，v2.1）

`print` 与 `println` **语义区分**（v2.1 定稿）：

```prima
print("hello");             // 输出 "hello"，不追加换行
println("hello");           // 输出 "hello" 并换行
print("a", "b");            // 多参数以空格分隔：a b（不换行）
println("x =", x);          // 同上但末尾换行
```

**规则**：

- `print(args...)`：逐一格式化并输出，参数间以**单个空格**分隔，**不追加换行**（可用 `print("\n")` 手动换行）。
- `println(args...)`：与 `print` 相同，但**末尾追加一个换行**。
- 两者都按 `print_format` 策略渲染参数（默认 LaTeX）。

**输入（v2.1）**：

```prima
let name = input("Name: ");        // 打印提示（可选）并读取一行，返回 String（去掉末尾换行）
let n = read_line();               // 无提示读取一行
let v = input("n = ").try_f64();   // 读取并坍缩（配合 try_* 家族，§九）
```

**规则**：

- `input(prompt?) -> String`：可选提示语打印到 stdout（不换行），从 stdin 读一行（去掉行尾 `\r\n`/`\n`）；EOF 返回空字符串。
- `read_line() -> String`：等价 `input()` 无提示。
- 交互式 CLI/REPL 中不可用时按空串处理（I/O 错误不 panic）。

### 18.2 `sys` 模块（底层系统操作）

**`sys::path`（跨平台路径）**：

```prima
import sys::path;
let p = path::join("a", "b");           // "a/b"（Linux/macOS）或 "a\\b"（Windows）
let n = path::file_name(p);             // Option<String>
let ext = path::extension(p);           // Option<String>
let parent = path::parent(p);           // Option<String>
let abs = path::is_absolute(p);         // Bool
```

**`sys::env`（跨平台环境）**：

```prima
import sys::env;
let home = env::home_dir();             // Option<String>
let path_var = env::get("PATH");        // Option<String>
let args = env::args();                 // Array<String>（命令行参数）
let cwd = env::current_dir();           // String
```

**`sys::os`（平台特定功能）**：

```prima
import sys::os;
let name = os::name();                  // "linux" / "macos" / "windows" / ...
let arch = os::arch();                  // "x86_64" / "aarch64" / ...
os::exit(0);                            // 立即退出进程
```

### 18.3 `time` 模块（时间系统）

```prima
import time;
let now = time::now();                  // 当前时间戳
let d = time::Duration::from_secs(5);
time::sleep(d);
let ts = time::unix_timestamp(now);     // I64
let s = time::format(now, "%Y-%m-%d");  // 格式化
let parsed = time::parse("2024-01-01", "%Y-%m-%d");  // Result
```

### 18.4 互操作：`@builtin` 与 `@c_api::extern`

#### `@builtin`（Rust 实现，用于编写真正的标准库）

标注在 `fn`/`class` 上，表示该函数的实现由 Rust 宿主提供；运行时按名称绑定到已注册的内建实现。**v2.2 支持分层优化形式 `@builtin(ON)`**（`O0`–`O3`，§10.2），使同一函数同时拥有 Rust 实现与 `.pra` 原实现，由优化等级决定启用哪一份。

**两种形态**：

1. **`@builtin(O0)`**（等价裸 `@builtin`，v2.1 原语义）：
   - **不允许函数体**（有函数体报 `E0056`）；
   - Rust 实现**必须**注册，未注册报 `E0055`；
   - 任何 `opt_level` 下都使用 Rust 实现。

2. **`@builtin(ON)`（`N = 1..3`，分层优化）**：
   - **必须有函数体**（`.pra` 原实现，作为回退/基线）；
   - Rust 实现**可选**（未注册不报错）；
   - 求值时：若**当前 `opt_level` 策略 ≥ `N`** 且 Rust 实现已注册 → 调用 Rust 实现；否则求值函数体的 `.pra` 原实现。
   - 两实现的语义必须一致（`.pra` 是唯一可观察语义来源，Rust 实现是其性能分层）。
   - 参数/返回类型在两种实现间相同，由签名统一约束。

```prima
// O0：必须无函数体，Rust 实现必须注册
@builtin
pub fn print(args...)

// O1+：Rust 实现可选；opt_level < 1 时求值 .pra 函数体
@builtin(O1)
pub fn to_upper(self: String) -> String {
    // .pra 原实现（O0/O1 以下路径）
    ...
}

@builtin(O2)
pub fn split(self: String, sep: String) -> Array<String> {
    ...
}
```

**规则**：

- `@builtin(OX)` 的等级参数非法（非 `O0`–`O3`）→ 编译期错误 `E0057`。
- 注册机制（Rust 侧）在 §实现方案中定稿（键 `"模块::函数"`，尽量**声明式宏**简化注册，避免手工字符串键）。
- 标准库方法文档照常以 `.pra` 的 `///` 注释维护（§18.1）。

#### `@c_api::extern`（导出 C ABI 接口）

标注在 `pub fn` 上，将该函数以 C 调用约定导出到二进制文件（`.so`/`.dylib`/`.dll`/可执行），供 C/Rust/其他语言调用。

```prima
import sys::c_api;

@c_api::extern
pub fn hello(a: c_api::int) {
    print(f"Hello, a is {a}");
}

@c_api::extern
pub fn add(a: c_api::double, b: c_api::double) -> c_api::double {
    return a + b;
}
```

**规则**：

- 仅 `pub fn`（宿主函数）可导出；参数/返回值必须是 `c_api::*` C 兼容类型（§附录 B.6）。
- 编译目标产出含 C 头文件（`prima compile --emit-headers`）。
- 字符串跨界用 `c_api::cstring`（由宿主转换为 Prima `String` 后传入，反之亦然）。

### 18.5 `ops` 模块（运算符重载）

**`ops` 提供运算符重载接口**，通过 `impl <Op> for <Class>` 为类定义运算符语义。

```prima
import ops;

class Vec2 {
    x: F64, y: F64,
    pub fn new(x: F64, y: F64) -> Self { Vec2 { x, y } }
}

impl ops::Add for Vec2 {
    fn add(self, rhs: Vec2) -> Vec2 { Vec2::new(self.x + rhs.x, self.y + rhs.y) }
}

let a = Vec2::new(1.0, 2.0);
let b = Vec2::new(3.0, 4.0);
let c = a + b;            // ⚠ W0005：运算符重载使用（默认警告）
```

**可重载运算符**（`ops` 提供）：`Add`、`Sub`、`Mul`、`Div`、`Rem`、`Neg`、`Eq`、`Cmp`、`Index`。

**策略控制**（§13.2）：

```prima
with config { overload_policy := allow } {   // 解除 W0005 警告
    let c = a + b;
}
with config { overload_policy := deny } {    // 使用即报错
    let c = a + b;            // 错误
}
```

**规则**：

- 重载**不影响**内建数值类型的运算符（`Integer + Integer` 等永远使用内建语义）。
- 重载运算符的调用点按 `overload_policy` 决定警告/放行/报错。
- `Eq`/`Cmp` 重载改变 `==`/`<` 等比较语义；`Index` 重载 `obj[i]`。

### 18.6 标准库扩展（v2.2）

> 以下模块**方法级清单以各自内嵌 `.pra` 模块的 `///` 文档注释为准**（§十八 管理原则）；此处给出职责范畴与关键实现取向。

**`math` 数值工具**：

- 整数算法：**因式分解**（试除/ Pollard rho）、**素数筛**、`gcd`/`lcm`/CRT（中国剩余定理）、整数幂余。
- 多项式与级数：**Taylor 展开**（`taylor(f, x, x0, n)` → 截断幂级数）、多项式加减乘除/求值/求根、连分数。
- 一般以 `.pra` 书写，热点（如大数因式分解核心）用 `@builtin(ON)` 分层到 Rust（§18.4）。

**`physics` 常用公式与 Class**：

- **常用公式直接用 Rust 实现**（`@builtin`/`@builtin(ON)`），便于对该类计算快速优化；示例范畴：运动学（匀速/匀加速直线、抛体）、力学（牛顿定律、能量/动量）、简谐运动、热学基础、电磁基础（库仑、欧姆）等。
- 提供 **Class**（如 `Vector3`、`Unit` 等）与方法；`physics` 模块同时保留 CODATA 物理常数（§7.3）。
- 面向的教学/工程取向：以带单位的初等物理公式为主，单位系统（§22.4）仍为未来扩展。

**`sys` 系统交互扩展**：

- 进程：`sys::process`（运行命令、退出码、输出捕获）、`sys::fs`（文件元数据、目录遍历）、`sys::term`（终端尺寸、raw 模式）等子模块，沿用 §18.2 的全路径绑定约定。

**`plot` 绘图**：线/散点/柱/等高线/热图；SVG 为默认后端，PNG 可选（`savefig` 格式参数）。**`render` 公式渲染**：

- `render::to_svg(expr)` / `render::to_png(expr)`：把表达式（`Expr`/f-string 结果）渲染为公式图像（复用 §7 ExprDAG 渲染器）。
- `render::to_terminal(expr)`：终端文本公式（ASCII/Unicode 数学）；**`print` 输出公式时的可选终端公式渲染以 cargo feature 提供**（§实现方案 §1）。
- 两者都以 `print_format` 策略为默认风格。

**`mem`**：`mem::Arc` 显式引用计数（§12.3/12.4）、`mem::collect()` 手动触发 GC。

---

## 十九、编译器与运行时实现路径

### 19.1 MVP（解释器 + 符号引擎）

**核心组件**：

- **词法**：手写 lexer（§实现方案）。
- **语法**：手写递归下降 + Pratt。
- **符号层**：`ExprPool`（hash-consing）+ `Number`/`Value` + 化简引擎。
- **渲染**：LaTeX 输出 + Unicode/ASCII 备选。
- **数值层**：
  - 任意精度：`num-bigint` + `num-rational` + `num-complex`（纯 Rust，MIT/Apache-2.0）。
  - 可选加速：`rug`（GMP 绑定，LGPL）作为 feature flag（`--features=rug-backend`）。
  - 矩阵：`nalgebra`（通用）或 `faer`（高性能，内存布局优化）。
- **策略系统**：编译期解析 `config {}`，运行时 `ThreadLocal<Config>` 存储。
- **模块系统**：文件系统映射 + `pub`/`pub(mod)` 可见性 + `import` 解析。
- **显式坍缩**：§九 坍缩函数族，带类型检查。
- **诊断**：编号化错误/警告（§十六）。

**MVP 里程碑**：

1. **基础符号计算**：

   ```prima
   import core;
   let a = tex"\sqrt{2}+\pi";
   print(a);                    // LaTeX 输出：\sqrt{2} + \pi
   ```

2. **化简与求值**：

   ```prima
   const b = tex"\e^{i\pi}+1";
   let c = simplify(b);         // → 0
   print(c);                    // 0
   ```

3. **函数与广播**：

   ```prima
   let f(x) = x^2;
   let v = [1, 2, 3];
   print(f(v));                 // [1, 4, 9]
   ```

4. **策略生效**：

   ```prima
   config { fraction := false }
   let x = 1/3;
   print(x);                    // 0.333... (F64)
   ```

5. **循环优化**：

   ```prima
   let s = 0;
   for i in 1..100 { s += i; }  // 编译器优化为 s = 100*101/2
   print(s);                    // 5050
   ```

6. **并行注解**：

   ```prima
   let f(x) @parallel = x^2;
   let v = range(0, 1000000);
   let w = f(v);                // 自动并行广播
   ```

7. **错误处理（Result + ?）**：

   ```prima
   fn parse_double(s: String) -> Result<F64, Error> {
       let v = try_f64(s)?;
       return Ok(v);
   }

   match parse_double("3.14") {
       Ok(x)  => print(f"parsed {x}"),
       Err(e) => print(f"failed: {e}")
   }
   ```

8. **Class 与方法链**：

   ```prima
   class Float {
       v: Expr,
       pub fn new(v: Expr) -> Self { Float { v } }
       pub fn to_f64(self) -> F64 { to_f64(self.v) }
       pub fn rounded(self, digits: Integer) -> F64 { rounded_f64(self.v, digits) }
   }
   let r = Float::new(sqrt(2) + \pi).to_f64().rounded(3);
   print(r);                    // 3.142
   ```

9. **字符串与 f-string**：

   ```prima
   let s = "e = \u{03B5}";          // 普通字符串：Unicode 转义
   let t = s.to_upper();
   print(t);                    // "E = Ε"
   let msg = f"parsed {x}";     // f-string：表达式插值
   ```

### 19.2 第二阶段：性能优化与 JIT

**JIT 编译**：

- **技术选型**：`inkwell`（LLVM 绑定）或 `cranelift-codegen`（轻量级代码生成，编译速度优先）。
- **混合执行**：符号层解释，数值热点 JIT 编译为原生码。
- **编译触发**：
  - 函数被调用超过阈值（如 100 次）。
  - 显式 `@jit` 注解：`let f(x) @jit = x^2 + sin(x)`。
- **批量算法**：对接 BLAS（OpenBLAS/MKL）、LAPACK，通过 `nalgebra` 或 `faer`。

**优化管道落地**（§10.2）：JIT 阶段接入常量折叠、CSE、循环优化与**自动内联**；AOT 阶段再做死代码消除与模块级优化。

**示例流程**：

```prima
let f(x) = x^2 + sin(x);

// 前 100 次调用：解释执行
for i in 1..100 {
    let _ = f(to_f64(i));
}

// 第 101 次：触发 JIT 编译
// ExprDAG → LLVM IR → 原生码
let result = f(to_f64(101));  // 原生速度
```

**可组合优化**：

```prima
let f(x) = x^2 + 1;
let g = jit(grad(f));         // 组合：自动微分 + JIT 编译
print(g(3.0));                // 原生速度的梯度计算
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
   - 完整程序分析 → 内联、死代码消除（§10.2 全管道）。
   - 链接 BLAS/LAPACK 静态库。
3. **WASM 后端**：
   - `wasm-bindgen` 导出 JS 接口。
   - 符号层编译为 WASM，数值层对接 WASM SIMD。
4. **C ABI 导出**（§18.4）：`prima compile --emit-c-abi` 生成 `.so`/头文件。

**命令**：

```bash
prima compile src/main.pra -o outputs/build/myapp       # 本机可执行文件
prima compile src/main.pra --target wasm32 -o app.wasm  # WebAssembly
prima compile src/main.pra --emit-c-abi -o libhello     # C ABI 动态库 + 头文件
```

### 19.4 自动微分（差异化卖点，尽早实现）

**实现分阶段**：

#### MVP 阶段：符号微分（v2.1 纳入 core 预导入）

- **基于 ExprDAG 的符号求导**：递归应用求导规则（和差积商、幂、链式、`sin/cos/tan/exp/ln/log/sqrt/abs`）。
- **接口（core 预导入）**：`derivative(expr, var)` / `partial(expr, var)` / `grad(expr)` / `limit(expr, var, a)`，接受**符号表达式**或 **MFn 名**（`derivative(f, x)` 等价 `derivative(f 的函数体, x)`）。
- **示例**：

  ```prima
  let f(x) = x^2 + sin(x);
  let df = derivative(f, x);    // → 2*x + cos(x)（返回 Expr）
  print(df);                    // LaTeX: 2x + \cos(x)

  let d2f = derivative(df, x);  // → 2 - sin(x)（高阶导数）

  let g(x, y) = x^2*y + y^3;
  let gx = partial(g, x);       // → 2*x*y
  let gy = partial(g, y);       // → x^2 + 3*y^2

  let gradv = grad(x^2 + y^2);  // → [2x, 2y]（对自由变量逐偏导）
  let lim = limit(sin(x)/x, x, 0);  // → 1（洛必达）
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
  let loss(w) = sum((y - predict(X, w))^2);
  let grad_loss = grad(loss);   // 反向模式自动微分
  let jit_grad = jit(grad_loss);  // JIT 编译梯度函数
  ```

**参考实现**：

- `alkahest`（Julia，符号 + AD + JIT 组合）。
- `enzyme`（LLVM 插件，自动微分）。
- `zygote.jl`（Julia 反向 AD）。

---

## 二十、项目目录结构

**设计规则**：**一切代码（入口/导入区/模块）放 `src/`**；**配置与 README 留在项目根目录**；运行产物放入 `outputs/`。

```text
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

# 文档生成（v2.2：解析 `///`/`//!` 文档注释，覆盖项目与内置标准库）
prima doc                        # 输出到 stdout（Markdown）
prima doc -o docs/api.md         # 写入文件
prima doc --stdlib               # 只输出内置标准库（含 core/string.pra 等）文档
```

---

## 二十一、已定决策总表

## | 决策 | 结论 |

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
 13 | 模块可见性 | 默认私有，`pub`/`pub(mod)` 公开（§15.2） |
 14 | 并行语法 | `@parallel`（+`parfor`），无隐式并行（§十七） |
 15 | 精确复数 | 内置固定规则（Julia 提升思想），不精确传染明确（§6.4） |
 16 | 内置符号 | 独立于 TeX；数学常数+算子+物理常数（CODATA）（§七） |
 17 | 算子求值 | 默认惰性保留，遇强制求值函数才数值化（§9.10） |
 18 | 预导入 | 仅 core，常用全暴露（§15.5） |
 19 | 错误处理 | **Rust 式 `Result`/`?`/`match`，无 `try/catch`**（§十六） |
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
 31 | 语句分隔 | **规范 `;`；换行分隔弃用（W0001）并逐步移除**（§4.2） |
 32 | 模式/解构 | **Rust 式全模式：`if let`/`while let`/`match` + 元组/数组/类/构造器/范围模式**（§4.4） |
 33 | Class | **字段 + 方法聚合类型，`Self`/`new`/`self`，浅拷贝共享 + 基本值深拷贝**（§4.5/12.3） |
 34 | 管道 | **`\|>` 弃用（W0002），由类方法链取代**（§9.7） |
 35 | 警告系统 | **编号 `W####`，英文码表记录于附录 C；策略可解除**（§16.5） |
 36 | 字符串（v2.2） | **`format` 移除，改用 f-string `f"..."`；`"..."`/`'...'` 双定界 + 原始字符串 `r"..."`；`String` 类方法集以 Python `str` 为参照，清单见 `.pra` 文档注释**（§18.1） |
 37 | 坍缩类型 | **与 Rust 基本数值一一对应：i8…u128/isize/usize/f32/f64**（§6.1/九） |
 38 | 互操作 | **`@builtin` Rust 实现 + `@c_api::extern` 导出 C ABI**（§18.4） |
 39 | 优化 | **`opt_level` 等级化优化管道（`O0`–`O3`）+ 自动内联（开发者不可干预）+ 常量折叠/CSE/循环优化/TCO/SIMD（O3）**（§10.2） |
 40 | 运算符重载 | **`ops` 模块 `impl`；`overload_policy` 默认 `warn`（W0005）**（§18.5） |
 41 | 标准库扩充 | **`sys`（path/env/os/process/fs/term）、`time`、`num`、`ops`、`c_api`、`mem`、`render`；`math`/`physics`/`plot` 按 v2.2 扩展**（§十八） |
 42 | 数组语义（v2.1） | **`Array` 可变长可变序列，元素任意值，方法齐全，可嵌套作为数据**；广播仅限数值同质数组（§11.3/11.4） |
 43 | 集合类型（v2.1） | **`Dict`/`Set` 为基本类型**：字面量、索引、方法、成员测试、集合代数（§4.6/11.6） |
 44 | 推导式（v2.1） | **`[x for ...]`/`{k: v for ...}`/`{x for ...}`** 统一迭代协议（§11.7） |
 45 | 控制台（v2.1） | **`print` 不换行 / `println` 换行**；`input`/`read_line` 读取 stdin（§18.1b） |
 46 | 便捷函数（v2.1） | **`len`/`enumerate`/`sorted`/`reversed`/`sum`/`prod`/`min`/`max`/`all`/`any`/`zip`/`join` 等** core 预导入（附录 B） |
 47 | 并行细节（v2.1） | **`@parallel` 广播按阈值并行；`parfor` 只允许索引槽写入（E0082）**（§十七） |
 48 | 符号微分（v2.1） | **`derivative`/`partial`/`grad`/`limit` 纳入 core**，基于 ExprDAG 符号求导（§19.4） |
 49 | 文档注释（v2.2） | **`///`/`//!` 规范文档注释随 AST 保留；`prima doc` 覆盖项目与内置标准库；方法调用出错时 note 附方法定义与文档**（§4.1/16.4） |
 50 | 优化等级（v2.2） | **`opt_level` 策略（`O0`–`O3`，默认 `O2`）控制优化通道集合；SIMD 识别等激进通道在 `O3`**（§10.2/13.2） |
 51 | `@builtin(O1)`（v2.2） | **分层优化：`opt_level ≥ N` 时用 Rust 实现，否则求值 `.pra` 原实现；`@builtin(O0)` 原语义保留**（§18.4） |
 52 | 标准库清单（v2.2） | **模块方法级清单以 `.pra` 的 `///` 文档注释为唯一来源；规范/实现文档只维护模块目录**（§十八） |
 53 | 内存（v2.2） | **宿主层 GC 取代引用计数；`mem::Arc` 提供显式引用计数，`mem::collect()` 手动 GC**（§12.3/12.4） |

---

### 二十二、未来扩展方向

#### 22.1 宏系统（保留）

- 保留关键字 `macro`，未来支持编译期代码生成。
- 参考 Rust 声明宏 + 过程宏。

#### 22.2 异步支持（保留）

- 保留关键字 `async` / `await`，未来支持异步 I/O。
- 参考 Rust `async-std` / `tokio` 生态。

#### 22.3 trait 系统（实验性）

- 保留关键字 `trait`，未来支持泛型约束。
- `ops` 模块的运算符重载（§18.5）是 trait 系统的先行者；`impl Add for T` 语法与其衔接。

#### 22.4 单位系统（可选模块）

- 物理量带单位：`let v = 3.0 * meter / second;`。
- 编译期单位检查，避免火星气候轨道器式灾难。

#### 22.5 GPU 加速（第三阶段）

- `@gpu` 注解：自动生成 CUDA/OpenCL/WGSL 代码。
- 矩阵运算、并行循环自动卸载到 GPU。

---

### 附录 A：完整语法 BNF

```bnf
program          ::= config? import* item*

config           ::= "config" "{" config_entry* "}"
config_entry     ::= ident ":" type? "=" value

import           ::= "import" module_path ("as" ident)?
                   | "from" module_path "import" import_items
import_items     ::= "*" | ident ("," ident)* | ident "as" ident ("," ident "as" ident)*
module_path      ::= ident ("::" ident)*

item             ::= statement | pub_item
pub_item         ::= "pub" (statement)
statement        ::= let_stmt | const_stmt | fn_def | math_def | class_def
                   | expr_stmt | control_stmt | impl_stmt | empty_stmt

let_stmt         ::= "let" "mut"? pattern type_ann? "=" expr ";"
const_stmt       ::= "const" ident type_ann "=" expr ";"
fn_def           ::= "fn" ident "(" params ")" type_ann? annotation* block
math_def         ::= "let" ident "(" params ")" type_ann? annotation* "=" expr ";"
class_def        ::= "class" ident "{" class_member* "}"
expr_stmt        ::= expr ";"
empty_stmt       ::= ";"
impl_stmt        ::= "impl" "ops" "::" impl_op "for" ident "{" impl_member+ "}"
impl_op          ::= "Add" | "Sub" | "Mul" | "Div" | "Rem" | "Neg"
                   | "Eq" | "Cmp" | "Index"
impl_member      ::= "fn" ident "(" params ")" type_ann? block
class_member     ::= field_decl | method_def
field_decl       ::= vis? ident ":" type ","
method_def       ::= vis? "fn" ident "(" params ")" type_ann? block
vis              ::= "pub" | "pub" "(" "mod" ")"
params           ::= (param ("," param)*)?
param            ::= ident type_ann? | "self" type_ann?
type_ann         ::= ":" type

annotation       ::= "@parallel" | "@jit" | "@gpu"
                   | "@builtin" ("(" opt_level ")")?     // 缺省 O0（§18.4）
                   | "@c_api" "::" "extern"
opt_level        ::= "O0" | "O1" | "O2" | "O3"            // §10.2

control_stmt     ::= for_stmt | parfor_stmt | while_stmt | if_stmt
                   | if_let_stmt | while_let_stmt | match_stmt
                   | return_stmt | with_config_stmt
for_stmt         ::= "for" ident "in" range ("step" expr)? block
parfor_stmt      ::= "parfor" ident "in" range ("step" expr)? block
range            ::= expr ".." expr
while_stmt       ::= "while" expr block
if_stmt          ::= "if" expr block ("else" "if" expr block)* ("else" block)?
if_let_stmt      ::= "if" "let" pattern "=" expr block ("else" block)?
while_let_stmt   ::= "while" "let" pattern "=" expr block
match_stmt       ::= "match" expr "{" match_arm+ "}"
match_arm        ::= pattern ("if" expr)? "=>" expr ("," | ";")?
return_stmt      ::= "return" expr?
with_config_stmt ::= "with" "config" "{" config_entry* "}" block

type             ::= "Number" | "Integer" | "Rational" | "F64" | "F32"
                   | "I8" | "I16" | "I32" | "I64" | "I128"
                   | "U8" | "U16" | "U32" | "U64" | "U128" | "Isize" | "Usize"
                   | "Complex" | "Expr" | "Symbol" | "Bool" | "String" | "Char"
                   | "Array" "<" type ">"
                   | "Matrix" "<" type ">"
                   | "Dict" "<" type "," type ">"
                   | "Set" "<" type ">"
                   | "Tuple" "<" type_list ">"
                   | "Option" "<" type ">"
                   | "Result" "<" type "," type ">"
                   | "Fn" "(" type_list ")" "->" type
                   | "MFn" "(" type_list ")" "->" type
                   | ident
                   | "Self"
type_list        ::= (type ("," type)*)?

pattern          ::= pattern_alt
pattern_alt      ::= pattern_simple ("|" pattern_simple)*
pattern_simple   ::= "_" | ident | literal | "-" literal
                   | tuple_pattern | array_pattern | class_pattern
                   | variant_pattern | range_pattern | grouped_pattern
tuple_pattern    ::= "(" pattern ("," pattern)* ("," "..")? ")"
array_pattern    ::= "[" pattern ("," pattern)* ".." "]" | "[" (pattern ("," pattern)*)? "]"
class_pattern    ::= ident "{" field_pattern ("," field_pattern)* ".."? "}"
field_pattern    ::= ident (":" pattern)? | ".."
variant_pattern  ::= ident pattern_simple?
range_pattern    ::= literal ".." literal | literal "..=" literal
grouped_pattern  ::= "(" pattern ")"

expr             ::= literal | ident | self_expr | call_expr | index_expr
                   | binary_expr | unary_expr | paren_expr | array_expr
                   | tuple_expr | dict_expr | set_expr | comprehension
                   | lambda_expr | match_expr | try_expr
                   | pipeline_expr | method_call | struct_literal
self_expr        ::= "self" | "Self"
method_call      ::= expr "." ident "(" args ")"
struct_literal   ::= ident "{" field_value ("," field_value)* "}"
field_value      ::= ident (":" expr)? | "..expr"         // ".." 从既有实例拷贝剩余字段
try_expr         ::= expr "?"

// v2.1 集合字面量与推导式（§4.6）
dict_expr        ::= "{" (entry ("," entry)*)? "}"
entry            ::= expr ":" expr
set_expr         ::= "{" expr ("," expr)+ "}"
comprehension    ::= comp_frame comp_for
comp_frame       ::= "[" comp_elem comp_for "]"
                   | "{" comp_entry comp_for "}"
                   | "{" comp_elem comp_for "}"
                   | "(" comp_elem comp_for ")"
comp_elem        ::= expr
comp_entry       ::= expr ":" expr
comp_for         ::= "for" ident "in" expr (comp_if | comp_for)*
comp_if          ::= "if" expr

literal          ::= number | string | f_string | char | bool | tex_literal
number           ::= integer | float | hex | binary
integer          ::= [0-9]+
float            ::= [0-9]+ "." [0-9]+ (("e"|"E") ("+"|"-")? [0-9]+)?
hex              ::= "0x" [0-9a-fA-F]+
binary           ::= "0b" [01]+
// v2.2：普通字符串 `"..."`/`'...'`（转义等价）；原始字符串 `r"..."`/`r'...'`（不转义）
string           ::= "\"" char* "\"" | "'" char* "'"
                   | "r" "\"" char* "\"" | "r" "'" char* "'"
// v2.2：f-string `f"..."`/`f'...'`（`{expr}` 插值、`{:spec}` 精化、`{{`/`}}` 转义）；`rf"..."` 组合
f_string         ::= "f" string_tpl | "rf" string_tpl
string_tpl       ::= "\"" string_part* "\"" | "'" string_part* "'"
string_part      ::= tpl_char | "{{" | "}}" | "{" tpl_expr (":" tpl_spec)? "}"
tpl_expr         ::= expr        // v2.2：不允许嵌套 f-string 字面量
tpl_spec         ::= [^{}]+
char             ::= "'" character "'"
bool             ::= "true" | "false"
tex_literal      ::= "tex\"" tex_content "\""

call_expr        ::= expr "(" args ")"
args             ::= (expr ("," expr)*)?

index_expr       ::= expr "[" index "]"
index            ::= expr | slice
slice            ::= expr? ".." expr? | ".."
// v2.1：负索引（-1 取末元素）与切片赋值（index_expr 作赋值左值，§11.3）

binary_expr      ::= expr binary_op expr
binary_op        ::= "+" | "-" | "*" | "/" | "^" | "**" | "@" | "%"
                   | "==" | "!=" | "<" | "<=" | ">" | ">="
                   | "&&" | "||"
// v2.1：`in`（成员测试，§11.3/11.6）与 `∪`/`∩`/`\`（Set 代数，§11.6）
//       追加到 binary_op 的等价优先级组（成员测试与比较同级）

unary_expr       ::= unary_op expr
unary_op         ::= "-" | "!" | "+"

paren_expr       ::= "(" expr ")"

array_expr       ::= "[" (expr ("," expr)*)? "]"

tuple_expr       ::= "(" expr "," (expr ("," expr)*)? ")"

lambda_expr      ::= "|" params "|" expr

match_expr       ::= "match" expr "{" match_arm+ "}"

pipeline_expr    ::= expr "|>" expr        // 弃用（W0002）

block            ::= "{" statement* "}"

ident            ::= [a-zA-Z_] [a-zA-Z0-9_]* | "\\" [a-zA-Z_]+
```

> 注：`match_arm` 的 `=>` 后为表达式，`block` 内语句以 `;` 结尾（块级语句可省略末尾 `;`，§4.2）。

---

### 附录 B：标准库函数速查

#### B.1 Core（预导入）

##### 坍缩函数

```prima
// 基础坍缩（panic on failure）
to_i8(x), to_i16(x), to_i32(x), to_i64(x), to_i128(x)
to_u8(x), to_u16(x), to_u32(x), to_u64(x), to_u128(x)
to_isize(x), to_usize(x)
to_f32(x), to_f64(x), to_bigint(x), to_rational(x), to_bigfloat(x), to_complex(x)

// 安全坍缩（返回 Result<T, Error>）
try_i8(x), try_i16(x), try_i32(x), try_i64(x), try_i128(x)
try_u8(x), try_u16(x), try_u32(x), try_u64(x), try_u128(x)
try_isize(x), try_usize(x)
try_f32(x), try_f64(x), try_bigint(x), try_rational(x), try_complex(x)

// 检查坍缩（检查溢出/范围）
checked_i8(x), checked_i16(x), checked_i32(x), checked_i64(x), checked_i128(x)
checked_u8(x), checked_u16(x), checked_u32(x), checked_u64(x), checked_u128(x)
checked_add(a, b), checked_mul(a, b)

// 钳制坍缩
clamped_i8(x, min, max), clamped_i16(x, min, max), clamped_i32(x, min, max)
clamped_i64(x, min, max), clamped_u8(x, min, max), clamped_u16(x, min, max)
clamped_u32(x, min, max), clamped_u64(x, min, max)
clamped_f32(x, min, max), clamped_f64(x, min, max)

// 舍入坍缩
rounded_f64(x, digits), rounded_f32(x, digits), rounded_i32(x), truncated_i32(x)
```

##### 数学函数

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

##### 化简与符号操作

```prima
simplify(expr, level = 2)      // 化简表达式
expand(expr)                   // 展开
factor(expr)                   // 因式分解
collect(expr, var)             // 合并同类项
substitute(expr, var, value)   // 替换
limit(expr, var, value)        // 极限（v2.1 实现：直接代入 + 洛必达）
derivative(expr, var)          // 导数（v2.1 实现，接受表达式或 MFn 名，§19.4）
partial(expr, var)             // 偏导数（同 derivative）
grad(expr)                     // 梯度（对自由变量逐偏导，返回 Array）
integral(f, var)               // 不定积分
definite_integral(f, var, a, b)  // 定积分
```

##### 字符串与 f-string（v2.2）

```prima
f"a = {a}"                       // f-string 插值（§18.1）；`format` 已移除（W0006）
to_string(x)                     // 任意值转 String
"..." / '...' / r"..." / r'...'  // 字面量（§三/18.1）
String::new(), String::from(v)
s.len(), s.is_empty(), s.split(sep), s.join(parts), s.to_upper(), s.to_lower()
// 完整方法集见 core/string.pra 的 /// 文档注释（§十八 管理原则）
```

##### 控制台（v2.1，§18.1b）

```prima
print(args...)                 // 格式化输出，参数空格分隔，不追加换行（v2.1）
println(args...)               // 同 print，末尾追加换行
input(prompt?)                 // 打印提示（可选）并读取一行 → String
read_line()                    // 无提示读取一行 → String
```

##### 集合便捷函数（v2.1，core 预导入）

```prima
// 多态长度与构造
len(x)                         // Array/Dict/Set/String/Tuple 的元素数
enumerate(arr)                 // → [(0, a0), (1, a1), ...]（Tuple 数组）
zip(a, b)                      // → [(a0, b0), (a1, b1), ...]（短端截断）
range(start, end, step = 1)    // 生成范围（数组或惰性迭代）
linspace(start, end, n)        // 线性等分

// 数组便捷
sorted(arr)                    // 排序 → 新 Array
reversed(arr)                  // 反转 → 新 Array
sum(arr)                       // 求和（数值）
prod(arr)                      // 求积（数值）
min(arr), max(arr)             // 最值
all(arr)                       // 全真
any(arr)                       // 任一真
arr.contains(x)                // 成员测试（等价 `x in arr`）
arr.index(x)                   // 元素下标（找不到 R0013）
arr.count(x)                   // 出现次数
arr.first(), arr.last()        // 首/末元素（Option）
arr.sort(), arr.reverse()      // 原地排序/反转

// Array 可变方法（§11.3）
v.push(x), v.pop() -> Option, v.append(x), v.extend(iterable)
v.insert(i, x), v.remove(i) -> Value, v.clear()

// Dict 方法（§11.6）
d.keys() -> Array, d.values() -> Array, d.items() -> Array<Tuple>
d.get(k) -> Option, d.insert(k, v), d.remove(k) -> Option, d.clear()
d.update(other), d.len()

// Set 方法（§11.6）
s.add(x), s.remove(x), s.discard(x), s.contains(x), s.len()
s.union(other) / s ∪ other, s.intersection(other) / s ∩ other
s.difference(other) / s \ other
```

##### 工具函数

```prima
map(f, array)                  // 映射
filter(pred, array)            // 过滤
reduce(f, array, init)         // 归约
```

##### 内建变体构造器（core 预导入）

```prima
Some(x)     // Option 的 Some
None        // Option 的 None
Ok(x)       // Result 的 Ok
Err(e)      // Result 的 Err
```

#### B.2 Linalg（显式导入）

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

#### B.3 Stats（显式导入）

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

#### B.4 Plot（显式导入）

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

#### B.5 Sys / Time / Num / Ops（显式导入）

```prima
// sys::path —— 跨平台路径
path::join(a, b), path::file_name(p), path::extension(p)
path::parent(p), path::is_absolute(p), path::canonicalize(p) -> Result

// sys::env —— 跨平台环境
env::home_dir() -> Option<String>
env::get(name) -> Option<String>
env::args() -> Array<String>
env::current_dir() -> String

// sys::os —— 平台特定
os::name() -> String, os::arch() -> String, os::exit(code)

// time —— 时间系统
time::now(), time::sleep(d), time::unix_timestamp(t) -> I64
time::format(t, fmt) -> String, time::parse(s, fmt) -> Result
Duration::from_secs(n), Duration::from_millis(n)

// num —— 额外数字类型与运算
num::gcd(a, b), num::lcm(a, b), num::is_prime(n)
num::next_prime(n), num::random_integer(a, b)
num::to_base(n, radix) -> String, num::from_base(s, radix) -> Result

// ops —— 运算符重载（§18.5）
impl ops::Add for Vec2 { fn add(self, rhs: Vec2) -> Vec2 { ... } }
impl ops::Index for Vec2 { fn index(self, i: Integer) -> F64 { ... } }
```

#### B.6 C ABI 类型（`sys::c_api`，显式导入）

```prima
c_api::int          // C int → I32
c_api::uint         // C unsigned int → U32
c_api::long         // C long → Isize
c_api::long_long    // C long long → I64
c_api::float        // C float → F32
c_api::double       // C double → F64
c_api::bool         // C bool → Bool
c_api::char         // C char → Char
c_api::cstring      // C char* → String（跨界转换）
c_api::ptr          // C void* → 不透明指针
c_api::unit         // C void
```

#### B.7 v2.2 标准库扩展（示意；完整清单见 `.pra` 模块文档注释）

```prima
// math —— 数值工具（§18.6）
math::factor(n)                  // 整数因式分解 → Array<Integer>
math::primes(limit)              // 素数筛 → Array<Integer>
math::gcd(a, b), math::lcm(a, b) // 已从 num 归并的整数算法
math::taylor(f, x, x0, n)        // Taylor 展开（MFn 名或表达式，截断 n 阶）
math::polynomial(roots, coeffs)  // 多项式构造/求值/求根

// physics —— 常用公式（Rust 实现）与 Class（§18.6）
physics::projectile_range(v0, angle, g)
physics::kinetic_energy(m, v)    // ½mv²
physics::simple_pendulum(L, g)
// 完整公式与 Class（如 Vector3）见 physics.pra 文档注释

// sys 扩展 —— 进程/文件系统/终端（§18.6）
sys::process::run(cmd) -> Result<String>    // 运行命令并捕获输出
sys::fs::metadata(p) -> Option<Dict>        // 文件元数据
sys::term::size() -> (Integer, Integer)     // (rows, cols)

// render —— 公式渲染（§18.6）
render::to_svg(expr), render::to_png(expr)  // Expr → 公式图像
render::to_terminal(expr)                   // 终端文本公式

// mem —— 显式引用计数与 GC（§12.3/12.4）
mem::Arc::new(x), mem::Arc::strong_count(x)
mem::collect()                              // 手动触发 GC
```

---

### 附录 C：错误与警告码表（Error & Warning Codes）

> 码表为**英文**。编译期错误 `E####`；运行时错误 `R####`；警告 `W####`。所有码在诊断输出中以 `error[CODE]`/`warning[CODE]` 形式呈现（§16.4）。

#### C.1 编译期错误（E）

 码 | 名称 | 含义 |
----|------|------|
 `E0001` | `lex_error` | 词法错误（非法字符/未闭合字面量） |
 `E0010` | `syntax_error` | 语法错误（含已移除语法的提示，如 `try/catch`） |
 `E0011` | `expected_separator` | 期望 `;` 语句分隔符 |
 `E0020` | `config_position` | `config {}` 未位于文件顶部 |
 `E0021` | `polluting_config` | 污染性策略声明在非入口文件 |
 `E0022` | `unknown_config` | 未知策略键 |
 `E0030` | `module_not_found` | 模块不存在 |
 `E0031` | `import_conflict` | 导入符号冲突 |
 `E0032` | `private_item` | 访问私有项 |
 `E0040` | `undefined_name` | 未定义名称 |
 `E0041` | `duplicate_definition` | 重复定义 |
 `E0050` | `type_mismatch` | 类型不匹配 |
 `E0051` | `missing_type_ann` | 缺少类型注解 |
 `E0052` | `unknown_type` | 未知类型 |
 `E0053` | `irrefutable_pattern` | `let` 使用了可反驳模式 |
 `E0054` | `try_operator_context` | `?` 用在非 `Result`/`Option` 返回函数外 |
 `E0055` | `unregistered_builtin` | `@builtin`（`O0`）未注册实现 |
 `E0056` | `builtin_body` | `@builtin(O0)` 函数/类不应有函数体 |
 `E0057` | `invalid_opt_level` | `@builtin(OX)` 优化等级参数非法（v2.2，须 `O0`–`O3`，§18.4） |
 `E0060` | `unknown_field` | 结构字面量/类模式引用了未知字段 |
 `E0061` | `missing_field` | 结构字面量缺少字段 |
 `E0062` | `self_outside_method` | `self`/`Self` 用在类方法外 |
 `E0063` | `self_not_first` | `self` 不是方法首参 |
 `E0070` | `unknown_annotation` | 未知注解 |
 `E0071` | `c_api_type` | `@c_api::extern` 参数/返回非 C 兼容类型 |
 `E0072` | `c_api_visibility` | `@c_api::extern` 函数非 `pub` |
  `E0080` | `return_outside_fn` | `return` 用在函数外 |
  `E0081` | `op_overload_bad_arity` | 运算符重载函数签名不合法 |
  `E0082` | `parfor_side_effect` | `parfor` 迭代体含副作用（v2.1，仅允许索引槽赋值/纯函数调用，§17.2） |

#### C.2 运行时错误（R）

 码 | 名称 | 含义 |
----|------|------|
  `R0001` | `overflow` | 溢出（`checked_*` 返回 Err） |
  `R0002` | `underflow` | 下溢 |
  `R0003` | `index_out_of_bounds` | 索引越界（含负索引越界，v2.1 §11.3） |
  `R0004` | `dimension_mismatch` | 维度不匹配 |
  `R0005` | `domain_error` | 定义域错误 |
  `R0006` | `undefined_error` | `Undefined` 参与运算 |
  `R0007` | `io_error` | I/O 错误 |
  `R0008` | `import_error` | 运行时模块加载错误 |
  `R0009` | `type_error` | 运行时类型不匹配（含广播遇非数值元素，v2.1 §11.4） |
  `R0010` | `cast_error` | 坍缩失败（`to_*`/`try_*`） |
  `R0011` | `custom_error` | 自定义错误（`panic`/`Err`） |
  `R0012` | `key_not_found` | `Dict` 键不存在（v2.1 §11.6） |
  `R0013` | `not_found` | `Array.index`/`Set.remove` 找不到目标（v2.1 §11.3/11.6） |
  `R0014` | `empty_collection` | 对空数组/空集合做广播或归约（v2.1 §11.4） |

#### C.3 警告（W）

 码 | 名称 | 含义 |
----|------|------|
 `W0001` | `newline_statement_separator` | 换行分隔语句（弃用，用 `;`） |
 `W0002` | `deprecated_pipeline` | `\|>` 管道弃用，用类方法 |
 `W0003` | `unused_binding` | 绑定未使用 |
 `W0004` | `unreachable_code` | 不可达代码 |
 `W0005` | `overloaded_operator` | 运算符重载使用（`overload_policy := allow` 解除） |
 `W0006` | `deprecated_format` | 调用已移除的 `format` 函数（改用 f-string，§18.1；过渡期警告） |

---

*语言规范 Prima v2.2 · 作为 Prima 语言设计与实现的最终依据*
