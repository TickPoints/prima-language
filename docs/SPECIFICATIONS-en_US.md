# **Prima** — Language Specification v2.2

> **Translation note**: this is the English counterpart of the authoritative Chinese specification [`SPECIFICATIONS-zh_CN.md`](./SPECIFICATIONS-zh_CN.md) (v2.2). The Chinese original is the final authority; this translation may lag or differ on the margins.
> **Notice**: This specification is the official language specification v2.2 of the **Prima language** and is the final authority for unified design and implementation.
> **v2.0 change summary**: ① error handling switched to Rust-style `Result`/`?`/`match` (`try/catch` removed); ② statements uniformly separated by `;` (newline separation enters the deprecation process, to be removed gradually); ③ Rust-style patterns and destructuring introduced (`if let`/`while let`/`match` full patterns); ④ Class and ownership semantics introduced; ⑤ numbered error/warning code table established (English, Appendix C); ⑥ full string support and `format`; ⑦ post-collapse numeric types correspond one-to-one with Rust basic numeric types; ⑧ interop (`@c_api::extern` exporting C ABI, `@builtin` Rust implementations); ⑨ standard library extended with `sys`/`time`/`num`/`ops`.
> **v2.1 change summary (base-type usability enhancements, Python-flavored)**: ⑩`Array` changed to **variable-length** sequence, supporting `push`/`pop`/`append`/`insert`/`remove`/`extend`/slice assignment/concatenation/membership testing (`in`)/negative indices/nestable (as data); ⑪new **mapping type `Dict`** and **set type `Set`** (literals, indexing, methods, iteration); ⑫common collection convenience functions: `len`/`enumerate`/`sorted`/`reversed`/`sum`/`prod`/`min`/`max`/`all`/`any`/`join`/`count`, etc.; ⑬`print` and `println` **distinguished** (the former does not add a newline, the latter does); ⑭console input `input`/`read_line`; ⑮list/dict/set **comprehensions** (`[x^2 for x in v if x > 0]`); ⑯symbolic differentiation primitives `derivative`/`partial`/`grad`/`limit` added to core (§19).
> **v2.2 change summary**: ⑰**`format` removed, replaced by Python-style f-strings** `f"a={a}"`; strings also support the `"..."`/`'...'` delimiters and raw strings `r"..."` (§18.1); ⑱**doc comments stabilized**: `///`/`//!` become normative comments and are incorporated into the AST, `prima doc` covers the built-in standard library, and when a method call fails the diagnostic note carries the method definition and documentation (§4.1/16.4); ⑲**`@builtin(O1)` layered optimization**: switching between a Rust implementation and the `.pra` original implementation according to the optimization level (§18.4); ⑳**optimization-level system**: a new `opt_level` policy (`O0`–`O3`), each level corresponding to a set of optimization passes (§10.2/13.2); ㉑**built-in method system**: the method sets of common classes such as `String` reference the stable Python methods, with the list and documentation uniformly maintained in the doc comments of the embedded `.pra` modules (§18.1); ㉒**standard library expansion**: `math` numeric tools (factorization, Taylor expansion), `physics` common formulas (Rust implementations), system interaction, and `plot`/`render` plotting and formula rendering (§18); ㉓**host-layer memory switched to GC**, with the standard library providing `mem::Arc` for explicit reference counting (§12.3/12.4).

## Identification

| Item | Value | Description |
|------|-------|-------------|
| **Language name** | **Prima** | Latin for "first / fundamental", echoing the "mathematical truth first" philosophy |
| **File extension** | **`.pra`** | Abbreviation of `Prima`; short, no major conflicts, directly corresponds to the language name |
| **Entry file** | **`src/main.pra`** | Project root module |
| **Package manager / tool name** | `prima` | Provides subcommands such as `run` / `compile` / `repl` |

---

## Glossary

| Term (Chinese) | Term (English) | Definition | First appearance |
|----------------|----------------|-----------|-----------------|
| 符号世界 | Symbol World (W_symbol) | The exact mathematical layer where expressions, symbols, and simplification live | §2 |
| 数值世界 | Numeric World (W_numeric) | The high-performance computing layer of floats, matrices, etc. | §2 |
| 宿主世界 | Host World (W_host) | The functional layer of control flow, objects, I/O, etc. | §2 |
| 坍缩 | Collapse | The explicit conversion from the symbol world to the numeric world | §2 |
| 不定式 | Indeterminate | An indeterminate form at the symbol layer (e.g. 0/0), further simplifiable | §6.2 |
| 未定义 | Undefined | The erroneous state at the numeric layer; must not participate in computation | §6.2 |
| 提升 | Promotion | Automatic conversion of a numeric type to a higher-precision type | §6.4 |
| 域标注 | Domain Annotation | The domain constraint on an expression (Real/Complex/etc.) | §6.5 |
| **hash-consing** | Hash-consing | An immutable data structure achieving structural sharing via hash-based deduplication | §8 |
| 策略 | Config/Policy | Module-level or global behavioral configuration | §13 |
| 编译单元 | Compilation Unit | An independently compiled module (corresponding to one .pra file or directory) | §15 |
| 类 | Class | A data structure of fields + methods (semantically close to Rust struct + impl) | §4/12 |
| 关联函数 | Associated Function | A class member function that does not take `self` and is called as `Type::name(...)` | §4.5 |
| 方法 | Method | A class member function that takes `self` and is called as `obj.name(...)` | §4.5 |
| 模式 | Pattern | A structure for matching/destructuring values (`if let`/`while let`/`match`/`let` destructuring) | §4.4 |
| 错误码 | Error Code | Numbered identifier of compile-time/runtime diagnostics (`E####`/`R####`) | §16/Appendix C |
| 警告码 | Warning Code | Numbered identifier of non-fatal diagnostics (`W####`) | §16/Appendix C |
| 内置实现 | Builtin | Functions/classes implemented by the Rust host (`@builtin` annotation) | §18 |
| C ABI 导出 | C ABI Export | Exporting a binary interface in the C calling convention (`@c_api::extern`) | §18 |
| 运算符重载 | Operator Overload | Customizing operator semantics for classes via the `ops` module | §18.5 |

---

## 1. Positioning

**Prima** is a **symbol-first** scientific computing language. By default it is exact, by default it preserves expressions, and by default it renders results in LaTeX; it safely descends into the numeric world through a **rich family of explicit collapse functions**; all behavioral customization is uniformly managed by a **module-level policy system**; parallelism is fully explicit; error handling adopts the **Rust-style `Result`/`?`** model.

**Design philosophy**:

- Mathematical "truth" takes precedence over the machine's "speed";
- Performance and precision are **explicit choices**; the default preserves mathematical authenticity;
- All configurable items belong to **modules**; contaminating configuration must be declared at the project entry;
- **Errors are values**: fallible operations return `Result`, handled explicitly by the caller via `match`/`?`/`unwrap`; the language provides no implicit exception-swallowing mechanism;
- Subsequent design decisions are governed by **implementability + user convenience + ease of learning**.

**Frames of reference**: Julia (numeric/multiple dispatch/promotion rules) + Mathematica/SymPy (symbol-first) + Rust (types/modules/memory/ownership/error handling) + Python (import syntax; since v2.1, base-type usability: variable-length `Array`, `Dict`/`Set`, comprehensions, `print`/`println`/`input`).

---

## 2. Overall Architecture and Execution Model

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

### The three "worlds"

| World | Name | Contents | Value form | Memory policy | Characteristics |
|-------|------|----------|------------|---------------|-----------------|
| **W_symbol** | Symbol World | Expressions, symbols, simplification | `ExprId` (hash-consed DAG, immutable) | hash-consing interner + thread-local cache | exact, simplifiable, thread-safe |
| **W_numeric** | Numeric World | Basic numerics (i8…f64, BigInt, complex), matrices, arrays | stack value types | stack + linear memory (BLAS) | native speed |
| **W_host** | Host World | Control flow, Class objects, I/O | user objects (class instances) | GC (tracing, shallow-copy sharing) + value semantics (deep copy); `mem::Arc` for explicit reference counting (§12.4) | functional |

**Core rule**: for an expression to leave the symbol world and enter the numeric/host world, it **must** pass through an explicit collapse function (§9); **no implicit conversion** (except for the exceptions allowed by §13 policies). **Fallible operations do not panic (unless explicitly `unwrap`/`to_*`); they always propagate as a `Result` return value.**

---

## 3. Lexical Structure

- **Identifiers**: `[a-zA-Z_][a-zA-Z0-9_]*` (extensible to Unicode letters, including Greek letters).
- **Numeric literals**: `123`, `3.14`, `1e-9`, `0x1F`, `0b1010`.
- **Strings**:
  - Ordinary strings: the `"..."` and `'...'` delimiters are **equivalent**, both supporting escapes (including `\u{XXXX}`);
  - Raw strings: `r"..."` / `r'...'` — escapes are not processed (`\n` is a literal backslash + `n`), `\u{XXXX}` is not expanded;
  - **Interpolated strings (f-strings)**: `f"..."` / `f'...'` — `{expr}` interpolation, `{:spec}` format refinements, `{{`/`}}` escapes (§18.1);
  - The prefixes above can be combined (`rf"..."` raw + interpolated).
- **TeX literals**: ``tex"..."``.
- **Operators**: `+ - * / ^ ** @ % == != < <= > >= && || ! = += -= ?`. Both `^` and `**` denote exponentiation (mutual aliases); `?` is the **try operator** (error propagation, §16.3); `in` is a **membership test** in **expression position** (§11.3) and the iteration keyword in `for`/`parfor`.
- **Comments**: `//` line comments, `/* */` block comments, **`///` doc comments** (on the item immediately following, §4.1), **`//!` module doc comments** (§4.1).
- **Reserved keywords** (for future extension): `async`, `yield`, `macro`, `trait`.
- **Active keywords**: `let`, `const`, `fn`, `class`, `pub`, `self`, `Self`, `if`, `else`, `while`, `for`, `in`, `step`, `parfor`, `return`, `match`, `impl`, `with`, `config`, `import`, `from`, `as`, `true`, `false`.
- **Annotations**: `@parallel`, `@jit`, `@gpu`, `@builtin` (may take an optimization-level argument `@builtin(O1)`, §18.4), `@c_api::extern`.
- **Statement separator**: `;` (normative, §4.2); newline separation is a **deprecated form** (§16.5 W0001).
- **Collection literals**: `{ ... }` is disambiguated by context into `Dict`/`Set` literals (§4.6) and code blocks; comprehensions reuse the `[ ... ]`/`{ ... }`/`( ... )` brackets (§11.7).

---

## 4. Syntax

### 4.1 File Structure

A Prima source file (`.pra`) consists of three sections in order:

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

#### Doc comments (v2.2)

- **`///`**: a doc comment attached to the **item** immediately following it (`fn`/`let` (MFn)/`class`/method/field/`const`/`import` binding) — multiple consecutive `///` lines are merged into that item's documentation text.
- **`//!`**: a module doc comment placed at the top of the module file (before the `config`/`import` sections), describing the module as a whole.
- Doc comments are **part of the language semantics**: they are parsed into the AST and preserved together with the item; `prima doc` (§20) generates documentation from them, and the diagnostics note of `prima check`/the interpreter (§16.4) reads them.
- Doc comments support simple markup (such as `` `code` ``, heading lines, `# Examples`, etc.); the rendering rules are defined by `prima doc` (§Implementation Plan).
- Example:

  ```prima
  //! The `String` type and its method set.

  /// Returns the number of Unicode scalar values in `self`.
  pub fn len(self) -> Integer
  ```

### 4.2 Statement Separation

- **Normative form**: every statement ends with `;` (`;` is the only normative statement separator).
- **Block-level statements** (`if`/`while`/`for`/`parfor`/`fn`/`class`/`match`/`with config` followed by `{}`) may omit the trailing `;`, consistent with Rust.
- **Deprecated form**: separating statements by newline (the newline immediately following a statement serves as the separator) is still accepted, but produces warning `W0001` (§16.5) and will be **removed** in a later version. New code must use `;`.
- **Empty statements**: a standalone `;` is legal (no-op).

```prima
let a = 1;                 // ✓ 规范：以 ; 分隔
let b = 2                  // ⚠ W0001：换行分隔（弃用）
let c = 3;                 // 上一条语句在 b = 2 的换行处结束

if a > 0 {                 // ✓ 块级语句省略末尾 ;
    print(a);
}
```

### 4.3 Grammar Skeleton

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
              | "@builtin" ("(" opt_level ")")?    // defaults to @builtin(O0) (§18.4)
              | "@c_api" "::" "extern"
opt_level    := "O0" | "O1" | "O2" | "O3"           // §10.2
```

> Statement separation: `;` is normative; newline is deprecated (§4.2). `pub` can modify `let`/`const`/`fn`/`math_def`/`class_def`.

### 4.4 Patterns and Destructuring (Rust-style)

Patterns are used in `let` destructuring, `if let`, `while let`, and `match` arms:

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

**Rules**:

- `_` wildcard; `ident` binding; literal matching; `-` for negative numeric literals.
- Tuple/array patterns support `..` to elide the remaining elements.
- The class pattern `Point { x, y: 0, .. }` matches fields.
- Constructor patterns are used for the built-in `Option` (`Some`/`None`) and `Result` (`Ok`/`Err`).
- Range patterns are only available for comparable literals (numbers/characters).
- `let` accepts only **irrefutable patterns** (refutable patterns such as `Some(x)` must use `if let`/`match`).
- `match` arm guards: `pattern if cond => ...`.

**Examples**:

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

### 4.5 Class Definition

**A Class is an aggregate type of fields + methods** (semantically close to a combination of Rust `struct` + `impl`). Syntax:

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

**Rules**:

1. **Visibility** (members):
   - No modifier → private to the class (only the class's own methods can access).
   - `pub(mod)` → visible to the current module (callable/accessible within the module).
   - `pub` → public, usable across modules (the class itself must be `pub` or `pub(mod)`).
2. **Fields**: `ident : type`; literal construction `Test { a: expr, ... }`, with `Test { a }` as shorthand. Fields are read-only by default; methods within the class can read them.
3. **Associated functions**: they do not take `self` and are called via `Type::name(args)`; the typical use is constructors (conventionally named `new`, returning `Self`). Struct literals are also a means of construction.
4. **Methods**: the first parameter is `self`, called via `obj.name(args)`; `self` is a **shallow copy of the object itself** (sharing the underlying data, §12.3).
5. **Ownership** (§12.3): `self` is a shallow copy (reference-counted sharing); when a method **returns a basic value** (`Number`/`Expr`/`String`, etc.), a deep copy is made before handing it out; returning an instance of this class keeps sharing.
6. **`Self`**: a type alias inside the class body referring to the current class.
7. Class does not support inheritance. Composition and trait-like interfaces implement operator semantics via the `ops` module (§18.5).
8. **Pipe deprecation**: the `|>` pipe (§9.7) is deprecated syntax (`W0002`); its role is gradually replaced by "class methods + method chaining".

**Example (method chaining replaces the pipe)**:

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

### 4.6 Collection Literals and Comprehensions (v2.1)

`Dict` and `Set` use curly-brace literals; comprehensions reuse the `[ ]`/`{ }`/`( )` brackets, distinguished by the `for` clause:

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

**Rules**:

1. `{ k: v }` is judged a Dict literal (key-value pair form); `{ a, b }` is judged a Set literal; `{}` is an empty Dict.
2. Dict keys must be **immutable and hashable** values (`Number`/`String`/`Char`/`Bool`/`Expr`/`Symbol`); Set elements likewise.
3. Collection literals are only valid in **expression position**; `{` directly following a control-flow keyword is still a code block.
4. Comprehension syntax: `<bracket> <element expression> for <variable> in <iterable> [if <condition>]`, allowing multiple `for` clauses (Cartesian product). The element of a `Dict` comprehension is a `key: value` pair; the element of a `Set` comprehension is a single value.
5. Curly braces are disambiguated by position between `Dict`/`Set` literals and code blocks following `match`/`class`/`impl`/`config`/`with config` (see Appendix A BNF for details).

---

## 5. Value System (Value)

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

**Immutability**: mathematical values (`Number`/`Expr`/`Symbol`) are immutable by default; `Array`/`Dict`/`Set` are **mutable host values** (length/content variable, §12.1); `W_host` objects (class instances) are managed under the shallow-copy/deep-copy semantics of §12.3.

---

## 6. Numeric Tower and Type System

### 6.1 Numeric Type Hierarchy

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

**Post-collapse numeric types correspond one-to-one with Rust basic numeric types**: `i8/i16/i32/i64/i128/u8/u16/u32/u64/u128/isize/usize/f32/f64`. The type names are the uppercase forms (`I8`, `U32`, `F64`, `Isize`, `Usize`…).

### 6.2 Strict Distinction Between Indeterminate and Undefined

#### Symbol layer: `Indeterminate`

- **Definition**: mathematically indeterminate forms, such as `0/0`, `∞/∞`, `0*∞`, `∞-∞`.
- **Behavior**:
  - Preserved as the symbol node `Indeterminate(form_type)`, **not an immediate error**.
  - May participate in later symbolic simplification, limit computation, and l'Hôpital's rule.
  - Example:

    ```prima
    let expr = (sin(x) - x) / x^3;   // 在 x=0 处形成 0/0，保留为 Indeterminate
    limit(expr, x, 0);               // → -1/6（通过泰勒展开或洛必达）
    simplify(expr);                  // 尝试化简不定式
    ```

#### Numeric layer: `Undefined`

- **Definition**: an erroneous state that cannot yield a meaningful numeric value.
- **When it arises**:
  - When an indeterminate form **collapses to the numeric layer** and cannot be simplified → `Undefined`.
  - Illegal operations in the real domain: `log(-1)` under the `domain := real` policy → `Undefined`.
- **Strict rules**:
  - **`Undefined` must not participate in any computation**: any unary/binary operator whose input contains `Undefined` is an **error** (compile-time if statically determinable, otherwise runtime `R0006`), **not propagated**.
  - Example:

    ```prima
    let a = 0/0;                     // 符号层 → Indeterminate
    let b = to_f64(a);               // 坍缩失败 → panic（to_* 家族）
    let c = try_f64(a);              // → Err(Error::UndefinedError)
    ```

#### Special numerics: `NaN` and `Inf`

- `0.0/0.0` → `NaN` (float arithmetic rules); `1.0/0.0` → `PlusInf`.
- **`NaN` / `Inf` must not explicitly exist at the symbol layer**; they appear only after explicit collapse to the numeric layer.

### 6.3 Type System

#### Type Syntax

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

#### Type Inference Rules (modeled after Rust)

**Literal inference**:

```prima
let x = 1;          // → Integer（整数字面量）
let y = 1.0;        // → F64（浮点字面量，有小数点或科学记数法）
let z = 0x1F;       // → Integer（十六进制）
let s = "hello";    // → String
let b = true;       // → Bool
```

**Expression inference**:

```prima
let a = sqrt(2);           // → Expr（符号函数，未坍缩）
let b = 1 + 2;             // → Integer（精确整数运算）
let c = 1/3;               // → Rational（fraction := true 默认）
let d = 1.0 + 2;           // → F64（不精确传染）
let e = [1, 2, 3];         // → Array<Integer>
let f = [[1, 2], [3, 4]];  // 错误：拒绝嵌套数组
```

**Function inference**:

```prima
let f(x) = x^2;           // → MFn(Expr) -> Expr（纯数学函数）
fn g(x: F64) -> F64 {     // → Fn(F64) -> F64（功能函数）
    return x * 2.0;
}
```

**Explicit type annotations**:

```prima
let x: F64 = sqrt(2);     // 类型错误：sqrt(2) 是 Expr，需显式坍缩
let y: F64 = to_f64(sqrt(2));  // 正确
let z: Integer = 3.14;    // 类型错误
```

**Type compatibility**:

- Exact types promote implicitly (§6.4): `Integer → Rational → Complex`.
- Inexact types are contagious: `Integer + F64 → F64`.
- Symbolic types do not auto-collapse: `Expr` needs an explicit conversion to enter numeric computation.
- No **implicit conversion between post-collapse fixed-width types**: `I32 → I64` requires an explicit `to_i64` (to prevent silent overflow, §9).

### 6.4 Exact Complex Arithmetic (built-in fixed rules)

Adopts Julia's **promotion/convert** ideas, but implemented as **built-in fixed rules**, exposing no user extension points:

**Promotion sequence**:

```text
Integer < Rational < Complex<Rational> < F64 < Complex<F64>
```

**Promotion rules**:

1. **Same-kind exact operations stay exact**:

   ```prima
   1 + 2                  // → Integer(3)
   1/3 + 2/5              // → Rational(11/15)
   Complex(1, 2) + 3      // → Complex(4, 2)（提升 3 → Complex(3, 0)）
   ```

2. **Inexactness contagion**:

   ```prima
   let a = 1/3;            // → Rational(1/3)
   let b = to_f64(a);      // → F64(0.333...)
   let c = Complex(0, 1);  // → Complex<Rational>(0, 1)
   b + c;                  // → Complex<F64>(0.333..., 1.0)
   ```

   **Rule**: upon encountering an `F64`, the entire `Complex` is promoted to `Complex<F64>`.

3. **Automatic reduction and normalization**:

   ```prima
   2/4                    // → Rational(1/2)（自动约分）
   Rational(6, -9)        // → Rational(-2/3)（分母为正）
   ```

**Complex functions**:

- `real(z)`, `imag(z)`, `conj(z)`, `abs(z)`, `abs2(z)` (avoids the square root), `angle(z)`.
- Exact exponentiation: `(-1)^(1/2)` in the complex domain → `\i` (§6.5).

**Implementation choices**:

- Base layer: `num-complex` + `num-rational` (pure Rust, MIT/Apache-2.0).
- Optional acceleration: `rug` (GMP, LGPL) as a feature flag (`--features=rug-backend`).

### 6.5 Domain Annotation

**Domain types**:

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

**Default behavior**:

- Global policy `domain := complex` (default) or `domain := real`.
- When simplifying symbols, the **highest domain** (most permissive domain) is used:

  ```prima
  let x: Real = -1;
  let y = x^(1/2);     // 化简时：最高域 = Complex → y 内部表示为 Complex(\i)
  ```

**Domain inheritance and propagation**:

1. **Domain inheritance on assignment** (outer constraint takes precedence):

   ```prima
   let x: Real = -1;
   let y = x;           // y 继承 Real 域标注
   let z = y^(1/2);     // 错误：Real 域下负数开方非法
   ```

2. **Explicit domain conversion** (variance capability):

   ```prima
   let x: Real = -1;
   let y = with_domain(x, Complex);  // 显式放宽为 Complex 域
   let z = y^(1/2);                  // 正确 → \i
   ```

3. **Domain inheritance for function parameters**:

   ```prima
   let f(x: Real): Real = x^2;    // 函数内 x 受 Real 约束
   f(-1);                         // 正确 → 1

   let g(x: Real): Complex = x^(1/2);  // 返回类型放宽
   g(-1);                         // 错误：输入域为 Real，内部无法开方

   let h(x): Complex = x^(1/2);   // x 无显式域约束，采用默认（Complex）
   h(-1);                         // 正确 → \i
   ```

4. **Domain promotion in mixed operations**:

   ```prima
   let a: Real = 2;
   let b: Complex = \i;
   let c = a + b;       // c 的域 = Complex（提升到更宽松的域）
   ```

**Intuitive principles**:

- Be permissive when simplifying/computing (allow intermediate steps to use a higher domain).
- Be strict when assigning/binding (outer type constraints take precedence).
- Provide explicit tools (`with_domain`) to break the constraints.

### 6.6 Default Rules

- **Exact by default**: the literal `2` is an `Integer`; `sqrt(2)` stays an `Expr`.
- **Inexactness contagion**: mixed operations after collapsing to `F64` tend to be inexact.
- **Fractions by default**: the default `fraction := true` keeps rationals as `Rational` (configurable).

---

## 7. Built-in Symbol System

**Built-in symbols are an inherent part of the language, independent of TeX** (TeX is only a view). Physical constant values are based on **CODATA 2022**.

### 7.1 Mathematical Constants

`\e` (Euler's number), `\pi`, `\i` (the imaginary unit), `\tau`, `\infty`, `\gamma` (the Euler–Mascheroni constant), `\phi` (the golden ratio).

### 7.2 Operators (expression-structure entities)

`\log` (logarithm), `\ln`, `\exp` (exponential), `\sqrt`, `\sin`, `\cos`, `\tan`, `\sigma` (summation), `\prod` (product), `\int` (integral), `\partial` (partial derivative).

**Key point**: **logarithms, exponentials, etc. are not just function names — they are expression-structure entities** (e.g. `Apply(Log, [x])`, `Apply(Exp, [x])`) that can enter simplification, differentiation, and integration.

### 7.3 Physical Constants (built-in, CODATA 2022)

**Naming strategy** (following Julia's `PhysicalConstants.jl`): **short names are not auto-exported by default** (e.g. `c`/`h`/`e` easily clash with variables and pollute the namespace); access is primarily via qualified long names.

| Category | Built-in long name | Access |
|----------|--------------------|--------|
| Basic | `\speed_of_light`, `\planck_const`, `\boltzmann_const`, `\gravitational_const` | `physics::\speed_of_light` or `import physics::\speed_of_light` |
| Electromagnetism | `\elementary_charge`, `\vacuum_permittivity`, `\vacuum_permeability`, `\fine_structure` | same as above |
| Chemistry | `\avogadro_const`, `\gas_const`, `\atomic_mass_unit` | same as above |
| Mass | `\electron_mass`, `\proton_mass`, `\neutron_mass` | same as above |
| Quantum | `\reduced_planck`, `\rydberg`, `\bohr_radius`, `\bohr_magneton` | same as above |
| Other | `\standard_gravity`, `\stefan_boltzmann`, `\standard_atmosphere` | same as above |

**Usage example**:

```prima
import physics;              // 仅导入模块命名空间

let E = physics::\planck_const * physics::\speed_of_light;  // 限定访问

// 或选择性导入
from physics import \planck_const as h, \speed_of_light as c;
let E = h * c;
```

> Physical constants are stored at **high precision** (not auto-collapsed by default; explicit `to_f64(\planck_const)` etc. is required).

### 7.4 Symbol Properties

- Built-in symbols can be simplified: `\e^{i\pi}` = `Pow(\e, Mul(\i, \pi))`.
- `sqrt(-1)` in the complex domain → `\i`; `(-1)^0.5` is determined by the `Domain` metadata carried by the symbol (§6.5).

---

## 8. Expression Representation: hash-consing DAG and Simplification

### 8.1 Representation: `ExprId` + `ExprPool`

```rust
#[derive(Copy, Clone, Hash, Eq, PartialEq)]
pub struct ExprId(u32);                       // compact opaque handle

pub enum ExprData {
    Symbol(SymbolId),
    Integer(Box<BigInt>),                     // Box avoids bloating the enum
    Rational(Box<BigRat>),
    Add(Box<[ExprId]>),                       // slice pointer, compact
    Mul(Box<[ExprId]>),
    Pow { base: ExprId, exp: ExprId },
    Apply { f: ExprId, args: Box<[ExprId]> },
    Indeterminate(IndeterminateForm),
    // extensible: LaTeX special nodes, etc.
}

pub struct ExprPool {
    global: DashMap<u64, ExprId>,             // global interner (sharded locks)
    store: RwLock<Vec<ExprData>>,             // central store
}

// thread-local cache (optimizes high-frequency symbol construction)
thread_local! {
    static LOCAL_CACHE: RefCell<HashMap<u64, ExprId>> = RefCell::new(HashMap::new());
}
```

**Implementation strategy**:

1. **Thread-local cache first**: every intern first checks `LOCAL_CACHE`; on a hit it returns directly.
2. **On a miss, consult the global**: look up `global: DashMap`, and on a hit write back to the local cache.
3. **Deferred globalization** (optional optimization): during symbolic computation, first accumulate in a local DAG and batch-intern to the global pool once computation completes.

### 8.2 Performance Gains (JuliaSymbolics measured reference)

- Structural sharing deduplication (eliminates expression swell): memory ↓2×, symbolic operations up to 3.2× faster.
- Equality = integer/pointer comparison (O(1)).
- Interner cache acceleration: numeric evaluation up to 100×; codegen/compilation 5–10×.
- Immutable + read-only sharing: naturally thread-safe (§17).

### 8.3 Simplification Levels

  Level | Trigger | Example |
------|------|------|
  0 | Just parsed, LaTeX rendering (shows original form) | `x + x` |
  1 | At assignment | `0*x→0`, `1*x→x` |
  2 | Numeric context / `simplify()` | `sin(0)→0`, `2*5→10` |
  3 | Explicit `simplify()` | rationalization, trigonometric identities, factorization |

**Default strategy**: printing/rendering = level 0 (preserves the original form unless `simplify()` is called first); entering numeric collapse = equivalent to level 2. Simplification never changes mathematical truth.

### 8.4 Canonical Form

`Add`/`Mul` are stored as n-ary ordered lists and normalized/sorted → `x+1 ≡ 1+x`, the same `ExprId`, hashable, usable as a `HashMap` key.

---

## 9. Collapse Library

Collapse is a **family of functions** whose naming conventions express safety properties; the user picks one per need.

### 9.1 Collapse Function Naming System

**Design principles**:

- **Basic form** `to_<type>(x)`: **panics** on failure; suitable for trusted input.
- **Attempt form** `try_<type>(x)`: returns `Result<T, Error>`; suitable for untrusted input.
- **Checked form** `checked_<type>(x)`: checks overflow/bounds, returns `Result<T, Error>`.
- **Clamped form** `clamped_<type>(x, min, max)`: forcibly clamps into a range.
- **Rounded form** `rounded_<type>(x, digits)`: rounds to the specified number of digits.

**Type coverage**: every collapse function family covers all types in one-to-one correspondence with Rust primitive numerics: `i8/i16/i32/i64/i128/u8/u16/u32/u64/u128/isize/usize/f32/f64`, plus `bigint/rational/bigfloat/complex`.

### 9.2 Basic Collapse (may panic)

```prima
to_i8(x)   to_i16(x)  to_i32(x)  to_i64(x)  to_i128(x)
to_u8(x)   to_u16(x)  to_u32(x)  to_u64(x)  to_u128(x)
to_isize(x) to_usize(x)
to_f32(x)  to_f64(x)                          // f64 is the most common
to_bigint(x) to_rational(x) to_bigfloat(x) to_complex(x)
```

**Example**:

```prima
let a = sqrt(2);
let b = to_f64(a);          // 1.414...

let c = 1e20;
let d = to_i32(c);          // panic: value out of i32 range
```

### 9.3 Safe Collapse (returns `Result<T, Error>`, no panic)

```prima
try_i8(x)  try_i16(x)  try_i32(x)  try_i64(x)  try_i128(x)
try_u8(x)  try_u16(x)  try_u32(x)  try_u64(x)  try_u128(x)
try_isize(x) try_usize(x)
try_f32(x) try_f64(x)
try_bigint(x) try_rational(x) try_complex(x)
```

**Example** (combined with `match`/`?`):

```prima
let a = sqrt(2) + \pi;
match try_i32(a) {
    Ok(n)  => print(f"converted {n}"),
    Err(e) => print(f"failed: {e}")
}

// or propagate with ? (only inside functions that return Result)
fn parse(x) -> Result<F64, Error> {
    let v = try_f64(x)?;      // early return on Err
    return Ok(v * 2.0);
}
```

### 9.4 Checked Collapse (checks overflow/range)

```prima
checked_i8(x)  checked_i16(x)  checked_i32(x)  checked_i64(x)  checked_i128(x)
checked_u8(x)  checked_u16(x)  checked_u32(x)  checked_u64(x)  checked_u128(x)
checked_add(a, b)    // checks addition overflow
checked_mul(a, b)    // checks multiplication overflow
```

**Example**:

```prima
let a = 2^31 - 1;
let b = checked_i32(a);     // Ok(2147483647)
let c = checked_i32(a + 1); // Err(Error::Overflow)
```

### 9.5 Clamped Collapse

```prima
clamped_i32(x, min, max)   // clamp into [min, max]
clamped_u8(x, min, max)
clamped_u64(x)             // clamp into [0, u64::MAX]
clamped_f64(x, min, max)   // clamp a floating-point range
```

**Example**:

```prima
let a = 1000;
let b = clamped_i32(a, 0, 255);  // → 255 (clamped to the upper bound)
```

### 9.6 Rounded Collapse

```prima
rounded_f64(x, digits)       // round to the specified number of decimal places
rounded_i32(x)               // round to the nearest integer
truncated_i32(x)             // truncate the fractional part
```

**Example**:

```prima
let a = \pi;
let b = rounded_f64(a, 3);    // → 3.142
let c = truncated_i32(a);     // → 3
```

### 9.7 Combined Collapse

**Canonical form**: `Result`-based chained handling (`?` + `match` + the `unwrap` family) and class methods (§4.5).

```prima
let a = sqrt(2) + \pi;
let b = try_f64(a)?.unwrap_or(0.0);   // first propagate with ?, then fall back to a default
let c = try_f64(a).unwrap();          // panics on failure
let d = try_f64(a).expect("convert pi");  // custom panic message
```

**Deprecated pipes**: the `|>` pipe (`a |> f`) is deprecated syntax (§16.5 `W0002`), progressively replaced by method chains (§4.5 examples).

**Multiple return values**:

```prima
complex_to_parts(z)          // → Tuple<(re, im)> two independent values
polar_form(z)                // → Tuple<(r, theta)>
```

**Example** (`let` tuple destructuring):

```prima
let z = Complex(3, 4);
let (r, theta) = polar_form(z);  // r = 5, theta = arctan(4/3)
```

### 9.8 No Implicit Collapse + No Precision Hints

- Expressions are not automatically lifted into floating-point arithmetic.
- **Collapse = user's own decision → no precision warnings** (the language does not support precision hints; an explicit user choice is active acceptance).
- Only when a collapse result is an **error** (`to_i32()` hitting a non-integer → panic; `checked_i32` overflow → `Err`) is it handled per §9.

### 9.9 Powers and Domain

- `sqrt(-1)` under `domain := complex` → `\i`.
- Fractional exponents (e.g. `(-1)^0.5`) are decided by the `Domain` metadata (§6.5):
  - `domain := complex` → allowed, yielding `\i`.
  - `domain := real` → error or `Undefined`.

### 9.10 Lazy Evaluation of Operators

Operators such as `\sigma` (sum), `\prod` (product), `\int` (integral) are **lazily preserved by default** and only become numeric when a force-evaluation function is encountered (explicit collapse or closed-form optimization triggered by `loop_optimization`).

**Example**:

```prima
let s = sum(i, 1, n);          // stays symbolic as Σ(i, 1, n)
print(s);                      // LaTeX output: \sum_{i=1}^{n} i
let s_eval = to_f64(s);        // only now is it numerically evaluated (requires n bound to a concrete value)
```

## 10. Evaluation Semantics and Optimization

### 10.1 Evaluation Semantics

- **Symbolic evaluation**: `f(x) = x^2 + 6; f(0)` → exact result after simplification, no automatic numeric conversion.
- **Numeric evaluation**: after collapse per §9.
- **Loop formula optimization** (`loop_optimization := true` on by default): `sum(1..n) i → n(n+1)/2`.

**Example**:

```prima
let f(x) = x^2 + 6;
let a = f(sqrt(2));       // → Expr: (sqrt(2))^2 + 6 → 2 + 6 → 8 (symbolic simplification)
let b = f(3.0);           // → F64: 15.0 (numeric computation)

// loop optimization
config { loop_optimization := true }
let s = 0;
for i in 1..100 {
    s += i;               // the compiler recognizes the pattern and converts it to s = 100*101/2
}
```

### 10.2 Optimization System (modern-language optimization)

**Principle**: all optimizations happen **automatically**; no per-function optimization directives are exposed to the developer (no annotations like `#[inline]`); global/local optimization intensity is controlled by the **`opt_level` policy** (§13.2). `@parallel`/`@jit`/`@gpu` are **parallel/execution-model** annotations, not optimization directives; `@builtin(O1)` (§18.4) is an **implementation-layering** annotation, affected by `opt_level` but not a substitute for it.

**Optimization levels (v2.2, the `opt_level` policy)**: `O0`–`O3`, defaulting to `O2`. Each level is an **incremental set of passes**: `opt_level := On` enables all passes of levels `≤ n`:

| Level | Enabled optimization passes |
|-------|-----------------------------|
| `O0` | No optimization pipeline: statement-by-statement interpretation, preserving the original loop semantics; **symbolic and numeric semantics** policies such as `simplify_level`/`fraction` still take effect; `loop_optimization`/automatic JIT compilation do not take effect |
| `O1` | Basic passes: constant folding/propagation, dead code elimination (DCE), closed-form loop formulas (§10.1); `@builtin(O1)` Rust implementations enabled |
| `O2` (default) | `O1` + common subexpression elimination (CSE), automatic inlining (heuristic), tail call optimization (TCO), automatic JIT hot-spot compilation (§19.2) |
| `O3` | `O2` + aggressive passes: SIMD recognition and vectorization, loop unrolling, unconditional inlining of small functions, `@builtin(O3)` implementations enabled (§18.4) |

**Optimization passes in detail**:

1. **Constant folding/propagation**: compile-time evaluation of literals and `const`.
2. **Dead code elimination**: removal of unreachable branches and unused assignments.
3. **Common subexpression elimination (CSE)**: duplicate subexpressions are computed only once.
4. **Loop optimization**: closed-form formulas (§10.1), loop-invariant hoisting, vectorization heuristics.
5. **Automatic inlining**: automatically inlines **pure/small functions that meet the heuristics** — thresholds are decided internally by the compiler (e.g. call count, function-body size, absence of side effects), **not controllable by the developer**. Inlining never changes observable semantics (including error timing and `Result` propagation).
6. **Tail call optimization (TCO)**: tail-recursive/tail-call stack reuse (applicable to both pure functions and `fn`).
7. **SIMD recognition and vectorization** (`O3`): element-wise operations on dense numeric arrays (broadcast, in-loop element operations) are recognized as vectorizable patterns and mapped to SIMD instructions or parallelized by block; applied only when numeric semantics can be proven unchanged (IEEE rounding exceptions are handled conservatively per the `print_format`/precision policies).
8. **Simplification level**: coordinates with the simplification system of §8.3; the `simplify_level` policy controls symbolic-simplification depth, with numeric optimization following after.

**Inlining rules**:

- Inlining targets: pure mathematical functions (MFn) and side-effect-free host functions.
- Never inlined: functions with `@parallel` side effects, recursive functions, and functions exceeding the size threshold.
- Inlining happens **after type checking** and before code generation (it does not affect error-diagnostic positioning; diagnostics still use source locations).

**Interplay with existing policies**: `loop_optimization := false` explicitly disables closed-form loop formulas at any level; semantic policies such as `broadcast`/`fraction` take precedence over the optimization level; `opt_level` only decides the **optimizations applied automatically by the compiler** and never changes observable semantics (results, error timing, `Result` propagation).

---

## 11. Functions, Arrays and Broadcast

### 11.1 Pure Mathematical Functions (MFn)

```prima
let f(x) = x^2 + 1;       // pure function, symbolic world by default
let g(x): F64 = to_f64(x^2);  // explicit return-type declaration
```

**Properties**: pure, side-effect-free, simplifiable, composable, first-class citizens, support automatic differentiation (§19.4). Can be annotated `@parallel` (§17). Preferred targets for automatic inlining (§10.2).

### 11.2 Imperative Functions (fn)

```prima
fn process(x: F64) -> F64 {
    print(f"Processing: {x}");
    return x * 2.0;
}
```

**Properties**: may have side effects, control flow, I/O. Can return `Result` (§16.3).

### 11.3 Arrays (Array, v2.1 variable-length sequences)

**`Array` is a variable-length, mutable sequence that may be homogeneous or heterogeneous** (v2.1, modeled after Python `list`): length can grow/shrink, elements can be arbitrary values (numbers/strings/booleans/`Expr`/class instances/nested arrays, etc.). The broadcast and matrix interfaces still require **homogeneous numeric arrays** (validated at the call site in §11.4).

#### Literals and Construction

```prima
let v = [1, 2, 3];            // Array: may hold arbitrary values
let w = [1.0, 2.0, 3.0];
let m = ["a", "b"];           // Array<String>
let nested = [[1, 2], [3, 4]]; // legal in v2.1: nested arrays as data (broadcast still rejects them, §11.4)
let e = Array::new();         // empty array (variable length)
let f = [x^2 for x in range(0, 5)];  // comprehension (§11.7)
```

#### Indexing and Slicing (including negative indices)

```prima
let v = [10, 20, 30, 40];
let a = v[0];                 // → 10
let b = v[-1];                // → 40 (negative indices count from the end; out of bounds reports R0003)
let c = v[1..3];              // → [20, 30] (slice, half-open interval)
let d = v[..2];               // → [10, 20]
let e = v[2..];               // → [30, 40]
let f = v[-2..];              // → [30, 40]
```

#### Slice Assignment (v2.1)

```prima
let v = [1, 2, 3, 4];
v[1..3] = [20, 30];           // v == [1, 20, 30, 4]
v[0..1] = [];                 // delete elements: v == [20, 30, 4]
```

#### Concatenation and Membership Testing

```prima
let a = [1, 2];
let b = [3, 4];
let c = a + b;                // → [1, 2, 3, 4] (concatenation)
a += b;                       // a == [1, 2, 3, 4] (in-place extension, equivalent to extend)
let has2 = 2 in c;            // → true (membership test, the `in` operator)
let has5 = 5 in c;            // → false
```

#### Mutating Methods (called on a mutable binding of the holder)

```prima
let mut v = [1, 2, 3];
v.push(4);                    // [1, 2, 3, 4]
let last = v.pop();           // → 4 (Some); v == [1, 2, 3]
v.append(5);                  // append a single element
v.extend([6, 7]);             // append a sequence
v.insert(0, 0);               // insert at the head: v == [0, 1, 2, 3, 5, 6, 7]
let removed = v.remove(0);    // → 0 (removes and returns); v shifts forward
v.clear();                    // v == []
```

#### Read-only Methods and Convenience Functions

```prima
let v = [3, 1, 2];
v.len()                       // → 3
v.is_empty()                  // → false
v.get(1)                      // → Some(1) (safe indexing, §4.4)
v.contains(2)                 // → true (equivalent to `2 in v`)
v.index(2)                    // → 1 (element index; reports R0013 if not found)
v.count(2)                    // → 1 (number of occurrences)
v.first()                     // → Some(3)
v.last()                      // → Some(2)
let s = sorted(v);            // → [1, 2, 3] (new array)
let r = reversed(v);          // → [2, 1, 3]
let total = sum(v);           // → 6
let prod_v = prod(v);         // → 6
let m = min(v);               // → 1
let M = max(v);               // → 3
```

#### Out-of-bounds Handling

```prima
let v = [1, 2, 3];
let x = v[10];                // runtime error R0003: index out of bounds
let y = v.get(10);            // → None (safe access, Option)
let z = v.get(1);             // → Some(2)
```

#### Matrix Construction

```prima
let M = Matrix::from_rows([[1, 2], [3, 4]]);  // 2×2 matrix
let N = Matrix::zeros(3, 3);                  // 3×3 zero matrix
let I = Matrix::identity(4);                  // 4×4 identity matrix

// matrix indexing
let A = Matrix::from_rows([[1, 2, 3], [4, 5, 6], [7, 8, 9]]);
let e = A[0, 1];              // → 2 (single element)
let f = A[0, ..];             // → [1, 2, 3] (row 0)
let g = A[.., 1];             // → [2, 5, 8] (column 1)
let h = A[0..2, 1..3];        // → [[2, 3], [5, 6]] (submatrix)
```

### 11.4 Broadcast

**Rules** (tightened in v2.1 to "homogeneous numeric arrays"):

- **Applies only to homogeneous numeric arrays**: broadcast requires array elements to be `Number`; elements containing non-numeric values (strings/arrays/classes, etc.) **error** (`R0009`) rather than silently degrading.
- **Rejects nested numeric arrays**: broadcast **errors** on "arrays of arrays" and does not recurse; ordinary nested arrays are data only (§11.3) and do not participate in broadcast.
- **Empty arrays error**: broadcast on an empty array errors (`R0014`), producing no silent empty result.
- **Default broadcast** `broadcast := true` (default): pure functions applied to arrays are element-wise; when `false`, an explicit `map` or a broadcast operator is required.
- **Parallel broadcast**: `@parallel` pure functions + large arrays automatically go parallel via rayon (§17.4).

**Example**:

```prima
config { broadcast := true }

let f(x) = x^2;
let v = [1, 2, 3];
let w = f(v);                // → [1, 4, 9] (automatic broadcast)

// binary-operation broadcast
let a = [1, 2, 3];
let b = [10, 20, 30];
let c = a + b;               // → [11, 22, 33]

// scalar broadcast
let d = a * 10;              // → [10, 20, 30]

// error examples
let g = f(["a", "b"]);       // error R0009: array element not numeric
let h = [];
let i = f(h);                // error R0014: empty array
```

**Explicit control** (when `broadcast := false`):

```prima
config { broadcast := false }

let f(x) = x^2;
let v = [1, 2, 3];
let w = map(f, v);           // explicit map
let x = v @. f;              // broadcast operator (syntactic sugar)
```

### 11.5 Function Context

- Pure function body → W_symbol (expression form, stays exact).
- Imperative function body → numeric by default, with explicit collapse as needed.

### 11.6 Mappings `Dict` and Sets `Set` (v2.1)

`Dict` is a mutable key → value mapping (modeled after Python `dict`), `Set` is a deduplicating mutable collection (modeled after Python `set`). Both require keys/elements to be **immutable and hashable** (`Number`/`String`/`Char`/`Bool`/`Expr`/`Symbol`).

#### Literals and Construction

```prima
let d = { "a": 1, "b": 2 };   // Dict: { key: value }
let d0 = Dict::new();         // empty Dict
let s = {1, 2, 3, 2};         // Set: deduplicated → {1, 2, 3}
let s0 = Set::new();          // empty Set
let t = {};                   // empty braces → empty Dict
```

#### Dict Indexing and Membership Testing

```prima
let d = { "a": 1, "b": 2 };
let a = d["a"];               // → 1
let missing = d["x"];         // runtime error R0012: key does not exist
let m = d.get("x");           // → None (safe access)
let m2 = d.get("a");          // → Some(1)
let has = "a" in d;           // → true (membership test)
let n = d.len();              // → 2 (number of entries)
```

#### Dict Methods

```prima
let d = { "a": 1 };
d["b"] = 2;                   // insert/update a key (element assignment)
d.insert("c", 3);             // equivalent to d["c"] = 3
let v = d.remove("a");        // → Some(1); if the key does not exist → None
d.clear();
d["a"] = 1;
let keys = d.keys();          // → ["a"] (Array, arbitrary order)
let vals = d.values();        // → [1]
let items = d.items();        // → [("a", 1)] (array of Tuples)
let dd = d.update({ "x": 9 }); // merge: d + { "x": 9 } (the latter overrides the former)
```

#### Set Methods and Set Algebra

```prima
let s = {1, 2, 3};
s.add(4);                     // add an element
s.remove(2);                  // remove an element; reports R0013 if absent
s.discard(99);                // remove an element; silently ignored if absent
let c = s.contains(1);        // → true (equivalent to `1 in s`)
let n = s.len();              // → number of elements
let u = s ∪ {5, 6};           // union (∪ is a Set-specific operator)
let i = s ∩ {2, 3};           // intersection
let diff = s \ {3};           // difference
```

#### Iteration and Generic Convenience

```prima
for k in d.keys() { print(k); }      // iterate keys
for (k, v) in d.items() { ... }      // iterate key-value pairs
for x in s { ... }                   // iterate the set

let n = len(d);                      // equivalent to d.len()
let m = len("hello");                // → 5 (`len` is a polymorphic convenience function, §18.1)
let e = enumerate(["a", "b"]);       // → [(0, "a"), (1, "b")] (array of Tuples)
let z = zip([1, 2], ["a", "b"]);     // → [(1, "a"), (2, "b")]
let all = all([true, true]);         // → true
let any = any([false, true]);        // → true
```

### 11.7 Comprehensions and the Iteration Protocol (v2.1)

A comprehension expresses "construction + filtering + mapping" as a single expression (modeled after Python). Syntax: `<frame> <element expression> for <variable> in <iterable> [if <condition>]`, with multiple `for` clauses allowed (Cartesian product).

```prima
let squares = [x^2 for x in range(0, 10)];              // Array: [0, 1, 4, ..., 81]
let evens   = [x for x in range(0, 10) if x % 2 == 0];  // with filtering
let pairs   = [(x, y) for x in range(0, 2) for y in range(0, 2)];  // nested
let table   = {x: x^2 for x in range(0, 5)};            // Dict comprehension
let odds    = {x for x in range(0, 10) if x % 2 == 1};  // Set comprehension

let n = len(squares);         // → 10
```

**Iterables**: `Array`, `Dict` (keys), `Set`, `range`, `String` (character sequence), `Tuple`. `for`/`parfor`/`in`/comprehensions all use the unified iteration protocol.

---

## 12. Variables, Scoping, and Ownership

### 12.1 Variables and Constants

```prima
let a = sqrt(2);              // variable: symbol preserved by default; scalar mutable
let mut b = 0;                // explicitly mutable (requires the mut keyword)
const c: Expr = \e^{i\pi};    // constant: type annotation required; immutable, inlineable
let d: Number = 0;            // explicit type annotation
```

**Mutability rules** (v2.1):

- **Numeric scalars**: mutable within `let mut` scope.
- **Collections** (`Array`/`Dict`/`Set`): **mutable host values** — length/content are mutable (`push`/`pop`/`d[k]=v`/`add`/…, §11.3/11.6); mutable methods require the binding to be `let mut` (`let` bindings may still call read-only methods in place).
- **Composite mathematical values** (`Expr`/`Matrix`/`Symbol`): immutable by default, shared references.
- **Constants**: globally immutable, inlineable at compile time.

### 12.2 Scoping and Visibility

- Block scope (`{}`) shadows same-named variables in outer scopes.
- **Irrefutable pattern destructuring** such as `let (a, b) = tuple;` creates multiple bindings (§4.4).
- Variables are not shared between modules (§15): using a module's public item requires `import`.

### 12.3 Ownership (Class semantics)

**Class instance ownership** (GC-handle semantics; use `mem::Arc` (§12.4) when a deterministic lifetime is needed):

- **Default value semantics**: class instances are managed by the host-layer **GC**; **assignment, argument passing, and returning an instance** are all **shallow copies** (handles sharing the underlying object, with no counting overhead).
- **`self` parameter**: a method receiving `self` receives a **shallow copy of the object itself**; reading `self` fields inside the method is a shared read.
- **Deep copy**: when a method **returns a primitive-value field** (`Expr`/`Number`/`String` and other scalar/immutable values), the value passed out is held independently (these primitive values are themselves immutable, so a deep copy just duplicates a handle/buffer). Returning a class instance keeps sharing.
- **Struct literal**: `Test { a, b }` creates a new instance (owning new field values).
- **No manual ownership syntax**: Rust-style borrowing syntax such as `&`/`move` is not exposed (`let mut` is kept to mark mutable bindings). Mutating class fields is done by explicitly constructing a new instance or via `mut self` methods.
- **GC is semantically transparent**: the GC automatically reclaims unreachable instances (including **reference cycles**); no destructor timing is exposed. For **deterministic release/destructor hooks** or keeping instances alive across FFI, use the standard library `mem::Arc` (explicit reference counting, `mem::Arc::new(x)`/`x.strong_count()`).
- **`ExprId` is a `Copy` value handle** and is naturally shared; the underlying `ExprPool` is read-only and concurrency-safe.

### 12.4 Memory Strategy

  Layer | Strategy |
----|------|
  W_symbol | hash-consing `ExprPool` (thread-local cache + global `DashMap`, shareable, acyclic, O(1) equality) |
  W_numeric | stack values + nalgebra + batched algorithms (BLAS) |
  W_host | **GC (tracing, mark-sweep/generational)** manages class instances and host values (shallow-copy sharing + deep copy for primitive values); `String` buffers inline/SSO; `mem::Arc` for explicit reference counting |

**GC design points (v2.2, §Implementation Plan Phase 12)**:

- GC scope: class instances and collection buffers in the **host world (W_host)**; the symbolic layer (`ExprPool`) and the numeric layer (stack values) are not involved.
- Trigger: collection is triggered by watermark at safe collection points inside the evaluator (block/function/loop boundaries); no asynchronous scanning thread is introduced (single-threaded GC, prioritizing determinism).
- Root set: the environment chain `EnvRef` + the current evaluation stack + the module table; instances reachable from the roots through the `Value` graph are traced.
- No destructor trap: GC-managed instances do not provide destructors; use `mem::Arc` for deterministic release.
- Parallel evaluation (`parfor`/`@parallel`) gives each task its own GC heap, reclaiming the whole heap when the task finishes.

---

## 13. Strategy System (Config)

### 13.1 Three-Level Strategy Hierarchy

Prima uses a **three-level strategy system** that layers from global to local:

1. **Global strategy** (contagious): the `config {}` in the project entry `src/main.pra`, affecting all modules.
2. **Module strategy**: the `config {}` at the top of each module file, affecting only that module.
3. **Local strategy**: function/block-level `with config { ... } { code }`, affecting only a specific block of code.

**Merge rule**: precedence **local > module > global**. Submodules inherit the parent strategy and may override locally.

### 13.2 Strategy Table (finalized)

#### Global (contagious) strategy

**Must** be declared at the very top of `src/main.pra`; declaring it in a non-entry file is a **compile error** (`E0021`).

  Strategy | Type | Default | Description |
------|------|------|------|
  `domain` | enum | `complex` | Default domain (`complex` / `real`), affects exponentiation and other operations |
  `undefined_handling` | enum | `strict` | `Undefined` behavior (`strict` errors / `custom { 0/0 := 1 }` black magic) |

#### Module/local strategy

May be declared at the module top or in a local `with config`.

  Strategy | Type | Default | Description |
------|------|------|------|
  `fraction` | bool | `true` | Preference for rational fractions vs floats |
  `broadcast` | bool | `true` | Pure functions are automatically element-wise (v2.1: only for numeric homogeneous arrays; rejects nested/empty arrays, §11.4) |
  `loop_optimization` | bool | `true` | Closed-form loop optimization |
  `opt_level` | enum | `O2` | Optimization level (`O0`/`O1`/`O2`/`O3`, v2.2, §10.2): the set of optimization passes applied automatically by the compiler |
  `simplify_level` | int 0-3 | `2` | Default simplification level (symbolic layer, independent of `opt_level`) |
  `num_to_big` | bool | `true` | Integer overflow automatically upgrades to BigInt (otherwise errors) |
  `print_format` | enum | `latex` | Print format (`latex` / `unicode` / `ascii`) |
  `overload_policy` | enum | `warn` | Operator overloading policy: `warn` (default, with `W0005` warning) / `allow` (released) / `deny` (error), §18.5 |

### 13.3 Strategy Usage Examples

#### Global strategy (entry file)

```prima
// src/main.pra
config {
    domain := complex              // global default: complex domain
    undefined_handling := strict   // strict mode
}

import mymath;

let a = (-1)^0.5;                  // correct → \i (global complex domain)
```

#### Module strategy

```prima
// src/numerical.pra
config {
    fraction := false              // this module prefers floats
    simplify_level := 3            // this module uses advanced simplification
}

let compute(x) = x / 3;            // → F64(x / 3.0)
```

#### Local strategy

```prima
config { domain := complex }       // module level: complex domain

let f(x) = x^2;

with config { domain := real } {   // locally switch to the real domain
    let y = (-1)^0.5;              // error: square root of a negative is illegal in the real domain
}

let z = (-1)^0.5;                  // correct → \i (back to the module-level complex domain)
```

### 13.4 The Special-Value "Black Magic" (experimental)

```prima
// src/main.pra (must be in the entry)
config {
    undefined_handling := custom {
        0/0 := 1,                  // define 0/0 = 1 (dangerous!)
        log(0) := -\infty          // define log(0) = -∞
    }
}
```

**Warning**: this feature breaks mathematical consistency and is only for specific domains (e.g., certain limit-computation conventions).

---

## 14. Control Flow

```prima
// for loop (range)
for i in 0..10 {
    total += i;
}

// with step
for i in 0..10 step 2 {
    print(i);                      // 0, 2, 4, 6, 8
}

// explicit parallel loop
parfor i in 0..n {
    A[i] = compute(i);             // the iteration body must be side-effect free
}

// while loop
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

**Pattern-destructuring control flow** (§4.4):

```prima
// if let
if let Some(x) = v.get(0) {
    print(f"first: {x}");
}

// while let
while let Some(x) = iter.next() {
    print(x);
}

// match (expression, full patterns)
let kind = match x {
    0        => "zero",
    1 | 2    => "small",
    3..=9    => "medium",
    n if n > 100 => "large",
    _        => "other"
};
```

**`?` operator** (§16.3): propagates errors only inside functions returning `Result`.

**Rules**:

- Control-flow variables are **numeric by default**, **symbols allowed** (via strategy/explicit annotation).
- Closed-form loop optimization is on by default (`loop_optimization := true`).
- `match` is an expression (usable as an rvalue); `if`/`while` are statements.
- `try/catch` **does not exist** (removed in v2.0, §16.3).

---

## 15. Modules and the Import System

**Philosophy**: `import` syntax (Python style) + compilation unit/visibility/paths (Rust style) + no variable sharing between modules.

### 15.1 Import Syntax (unified `import`)

```prima
import core;                    // bring in a namespace
import linalg as la;            // alias
from stats import mean, std;    // selective import
from mymath import *;           // wildcard (not recommended)
```

### 15.2 Modules / Compilation Units (Rust structure)

- **A module is a compilation unit** with its own scope.
- **Private by default**: all items are private by default; crossing modules requires explicit `pub`.
- **No variable sharing**: `import` brings **public items** into the namespace; internal state is not shared.
- **Nested module paths** `a::b::c`.
- **Visibility modifiers**:
  - `pub`: public, visible across modules.
  - `pub(mod)`: visible in the current module (module-level, equivalent to Rust `pub(crate)` semantics: visible to all items **within this module**, invisible outside).
  - No modifier: private to the enclosing **class**/scope.

**Example**:

```prima
// src/math_utils.pra
pub let square(x) = x^2;        // public function
let helper(x) = x + 1;          // private function

pub const PHI: Rational = (1 + sqrt(5)) / 2;  // public constant

pub class Vec2 {                // public class
    x: F64,
    y: F64,
    pub fn new(x: F64, y: F64) -> Self {
        Vec2 { x, y }
    }
    pub(mod) fn norm(self) -> F64 {      // visible only within this module
        sqrt(self.x^2 + self.y^2)
    }
}
```

```prima
// src/main.pra
import math_utils;

let a = math_utils::square(3);  // correct
let b = math_utils::helper(3);  // error: helper is private (E0032)
let c = math_utils::PHI;        // correct
let v = math_utils::Vec2::new(3.0, 4.0);
let n = v.norm();               // error: norm is pub(mod), invisible to the main module (E0032)
```

### 15.3 File Mapping

- **One `.pra` file = one module body**.
- **One directory = one submodule**, whose `main.pra` is the directory module's entry (modeled after Rust `mod.rs`).
- **Project entry = `src/main.pra`** (root module).

**Example directory structure**:

```text
src/
├── main.pra               // root module
├── physics.pra            // module physics
└── linalg/                // submodule linalg
    ├── main.pra           // linalg module entry
    └── fft.pra            // linalg::fft submodule
```

```prima
// src/main.pra
import physics;
import linalg;
import linalg::fft;
```

### 15.4 Import Conflicts

Same-named symbols imported from multiple modules: importing them conflicts and errors; resolve with an alias or `::` qualification.

```prima
from math import sin;
from custom_math import sin;    // error: sin conflicts (E0031)

// solution 1: alias
from math import sin as std_sin;
from custom_math import sin as my_sin;

// solution 2: qualified access
import math;
import custom_math;
let a = math::sin(x);
let b = custom_math::sin(x);
```

### 15.5 Pre-imports

- **`core` is pre-imported**, and **all common `core` features (built-in symbols, the collapse-function family, basic operators, f-strings (§18.1), `Option`/`Result` variants) are fully exposed**.
- All other modules (`linalg`/`stats`/`plot`/`render`/`math`/`io`/`parallel`/`physics`/`sys`/`time`/`num`/`ops`/`c_api`/`mem`) must be explicitly `import`ed.

**`core` pre-imported contents**:

- Numeric types: `Integer`, `Rational`, `F64`, `Complex`, `Expr`, `Symbol`, fixed-width type names
- Collapse functions: the full `to_*`/`try_*`/`checked_*`/`clamped_*`/`rounded_*` families (§9)
- Built-in symbols: `\e`, `\pi`, `\i`, `\infty`, `\gamma`, `\phi`
- Basic operators: `sqrt`, `sin`, `cos`, `log`, `exp`, etc.
- Simplification functions: `simplify`, `limit`, `derivative`, `partial`, `grad` (§19.4)
- Collections: `Array`/`Dict`/`Set` and their methods (§11), plus convenience functions `len`/`enumerate`/`sorted`/`reversed`/`sum`/`prod`/`min`/`max`/`all`/`any`/`zip`/`join`
- Console: `print` (no newline), `println` (newline), `input`, `read_line` (§18.1b)
- Utility functions: `range`, `map`, `filter`, `Some`, `None`, `Ok`, `Err`; string formatting uses **f-strings** (§18.1), and the `format` function has been removed (calling it yields `W0006`)

---

## 16. Error Handling and Warning System

### 16.1 Error Type Definition

```rust
pub enum Error {
    // type error
    TypeError {
        expected: Type,
        got: Type,
        location: SourceLocation,
    },

    // numerical domain error
    DomainError {
        expr: Expr,
        reason: String,
        location: SourceLocation,
    },

    // overflow error
    Overflow {
        value: Number,
        target_type: Type,
        location: SourceLocation,
    },

    // underflow error
    Underflow {
        value: Number,
        location: SourceLocation,
    },

    // undefined error
    UndefinedError {
        expr: Expr,
        reason: String,
        location: SourceLocation,
    },

    // index out of bounds
    IndexOutOfBounds {
        index: usize,
        length: usize,
        location: SourceLocation,
    },

    // key not found (Dict/Set, v2.1, §11.6)
    KeyNotFound {
        key: String,
        location: SourceLocation,
    },

    // element/key not found (v2.1, §11.3/11.6)
    NotFound {
        value: String,
        location: SourceLocation,
    },

    // dimension mismatch
    DimensionMismatch {
        expected: Vec<usize>,
        got: Vec<usize>,
        location: SourceLocation,
    },

    // I/O error
    IoError {
        kind: IoErrorKind,
        message: String,
        location: SourceLocation,
    },

    // import error
    ImportError {
        module: String,
        reason: String,
        location: SourceLocation,
    },

    // syntax error
    SyntaxError {
        message: String,
        location: SourceLocation,
    },

    // custom error
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

### 16.2 Error Classification

  Category | Semantics | Handling |
------|------|---------|
  **Compile-time errors** | syntax, types, imports, visibility, statically decidable `Undefined` | compilation fails, numbered `E####`, detailed diagnostics provided (§16.4) |
  **Recoverable errors** | overflow, out of bounds, I/O, collapse failures, `Undefined` in operations | returned as `Result<T, Error>` (numbered `R####`), handled by the caller via `match`/`?`/`unwrap` |
  **Fallback panic** | triggered by explicit `to_*`/`unwrap`/`expect`; unrecoverable internal errors | carries a cross-language stack trace, terminates the program |

### 16.3 Error Handling Syntax (Rust style, no try/catch)

**`Result<T, Error>`** is the only first-class error representation; `Option<T>` represents possibly-missing values.

#### match handling

```prima
let result = try_i32(1e20);
match result {
    Ok(n) => print(f"success: {n}"),
    Err(e) => print(f"failed: {e}")
}
```

#### The `?` operator (error propagation)

```prima
// ? may only be used inside functions returning Result/Option
fn parse_and_double(s: String) -> Result<F64, Error> {
    let v = try_f64(s)?;          // on Err, immediately return Err(...)
    return Ok(v * 2.0);
}

fn first(list: Array) -> Option<Integer> {
    let x = list.get(0)?;         // on None, immediately return None
    return Some(x);
}
```

#### The unwrap family (explicitly discard errors, panic as fallback)

```prima
let a = try_i32(100).unwrap();              // panics on failure
let b = try_i32(1e20).unwrap_or(0);         // returns a default value on failure
let c = try_i32(1e20).expect("conversion failed");  // custom panic message
```

#### Safe access (Option)

```prima
let v = [1, 2, 3];
if let Some(x) = v.get(1) {
    print(x);                    // 2
}
let y = v.get(10).unwrap_or(0);  // 0
```

**Rules**:

- `?` is only allowed inside functions returning `Result`/`Option`; otherwise a compile-time error (`E0054`).
- The `to_*` family panics directly and does not return `Result`; use `try_*` when error handling is needed.
- **`try/catch` syntax was removed in v2.0**: it is a compile-time error at parse time (`E0010` syntax error, pointing users to `Result`).

### 16.4 Diagnostic Format

**Format**: **number + source location (file:line:col) + relevant expression (LaTeX) + recovery suggestion**.

**Error example**:

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

**Warning example** (§16.5):

```text
warning[W0001]: statements separated by newlines are deprecated
  --> src/main.pra:8:12
   |
 8 |     let b = 2
   |              ^ use `;` to terminate statements; newline separation will be removed
   |
   = help: replace the trailing newline with `;`
```

**Doc notes on method-call errors (v2.2)**: when a **method call** (`obj.method(...)`) fails — whether the cause is compile-time (unknown method, argument count/type mismatch) or runtime (an error thrown inside the method) — the diagnostic must attach **the relevant definition and doc comments of that method** (§4.1) in a note:

- **Method definition**: the full signature (including parameter types and return type) and the definition location (`file:line:col`).
- **Doc comments**: the method's `///` documentation text; if absent, the doc text of the enclosing class (or module) is attached instead.
- Documentation for standard-library methods (§18.1/§18.4) likewise comes from the `///` comments in the embedded `.pra` modules and can be viewed offline (`prima doc`, §20).

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

### 16.5 Warning System

**Principle**: warnings do not block compilation/execution, but mark usage that is "non-spec-compliant/deprecated". Each warning has a unique number `W####`, an English short name plus a description, recorded in **Appendix C**.

**Existing warnings**:

  Number | Name | Meaning |
------|------|------|
  `W0001` | `newline_statement_separator` | using newlines to separate statements (deprecated; use `;`, §4.2) |
  `W0002` | `deprecated_pipeline` | using the `\|>` pipeline (deprecated; use class methods, §9.7) |
  `W0003` | `unused_binding` | `let` binding is never used |
  `W0004` | `unreachable_code` | unreachable code |
  `W0005` | `overloaded_operator` | using operator overloading (§18.5; triggered by default with `overload_policy := warn`, released by `allow`) |
  `W0006` | `deprecated_format` | calling the removed `format` function (use f-strings instead, §18.1; transitional warning, removed in the target version) |

**Rules**:

- Warnings are emitted on the diagnostic channel and do not affect the exit code; `prima check` may use `--deny W0001` to upgrade a warning to an error (tool level).
- Warnings can be released via strategy (e.g., `overload_policy := allow` releases `W0005`); **no per-warning `allow` annotation is provided** (to avoid noise).
- Deprecation warnings (`W0001`/`W0002`/`W0006`) are removed along with the corresponding syntax in the target version.

## 17. Parallelism and Multithreading

**Philosophy**: **no implicit parallelism; parallelism must be explicit**. The language never uses threads/`rayon` implicitly; whether to parallelize is decided explicitly by the user.

### 17.1 Syntax: the `@parallel` annotation

```prima
let f(x): MFn @parallel = x^2;          // 纯函数并行（安全）

// 实验性特性（暂不转正）
// fn process(x) @parallel { ... }     // 功能函数并行（需手动保证线程安全）
```

**Rules** (finalized in v2.1):

- `@parallel` may annotate **pure mathematical functions only** (safe; the compiler verifies the absence of side effects).
- A `@parallel` function body must be **self-contained**: it may depend only on parameters and built-in math symbols/constants, and must not reference external free variables (parallel subtasks each evaluate independently, with no shared environment).
- Call sites parallelize in a **broadcast context** (array arguments): when the array length is ≥ the threshold (default 1024), work is chunked by thread count and handed to rayon; small arrays take the sequential path (to avoid overhead).
- Parallelism for imperative functions is marked `[EXPERIMENTAL]`; the user must ensure thread safety manually.

### 17.2 `parfor` Explicit Parallel Loops

```prima
parfor i in 0..n {
    A[i] = compute(i);              // 迭代体必须无副作用
}

// 带步长
parfor i in 0..n step 2 {
    B[i] = heavy_computation(i);
}
```

**Rules** (finalized in v2.1):

- The loop body must be **side-effect-free**, otherwise a compile error is raised (`E0082`). Allowed statement forms: assignments to **index slots** (`A[i] = …`/`A[i] += …`, where `i` is the loop variable or a pure function of it; out of bounds raises `R0003`) and pure function calls; `print`, assignments to external variables, `let` bindings, class instance mutation, etc. are forbidden.
- Result writes: each array slot is computed independently; when finished, the whole array is written back to the binding (parallel via rayon).
- Underneath it uses the rayon thread pool, with granularity tuned automatically.

### 17.3 Thread-Safety Guarantees

- **Immutable math values** (`ExprId` + `ExprPool`) are shared read-only and are inherently thread-safe.
- Parallel use of **mutable `W_host` state** requires the user to explicitly manage primitives (such as `Mutex`, `Atomic`).
- **Class instances** may be shared read-only safely in parallel contexts; write operations require explicit synchronization.
- **Module boundaries isolate** mutable state, reducing concurrency risk.

### 17.4 Parallelism Examples

#### Parallel pure-function broadcast

```prima
let f(x) @parallel = x^2 + sin(x);
let v = range(0, 1000000);
let w = f(v);                       // 自动并行广播（broadcast + @parallel）
```

#### Parallel matrix operations

```prima
parfor i in 0..rows {
    for j in 0..cols {
        C[i, j] = dot(A[i, ..], B[.., j]);
    }
}
```

---

## 18. Standard Library Plan

> **Management principle for method-level lists (v2.2)**: **which functions/methods/Classes a module concretely implements is governed by the `///` doc comments in that module's embedded `.pra` source**; the spec and implementation docs no longer enumerate them one by one. This table maintains the **module catalog and responsibilities**; §18.1/Appendix B keep core illustrative lists.

  Module | Content | Import |
------|------|------|
 **core** | Number tower, ExprDAG, simplification, built-in symbols, the collapse function family, basic operators, f-strings (§18.1), `Result`/`Option`, the `String` class | **Pre-imported; all common names exposed** |
 **linalg** | Matrices, linear algebra (nalgebra/faer), solvers | Explicit |
 **stats** | Basic statistics (mean, variance, quantiles, distributions) | Explicit |
 **plot** | Plotting (SVG to start, optional plotly backend); scientific plots (line/scatter/bar/contour/heatmap) | Explicit |
 **render** | **Formula rendering (v2.2)**: TeX/LaTeX expressions → SVG/PNG/terminal text (reusing the ExprDAG renderer, §7); optional terminal rendering of formulas output by `print` is provided via a cargo feature | Explicit |
 **math** | Special functions (Bessel, gamma, hypergeometric), numerical integration, ODE, FFT, **numeric tools (v2.2): factorization, prime sieve, Taylor expansion, polynomial operations, etc.** | Explicit |
 **io** | File I/O, serialization (JSON/CSV/HDF5), formatted output | Explicit |
 **parallel** | Parallel primitives (`parfor` helpers, thread-pool configuration, task scheduling) | Explicit |
 **physics** | Physical constants (CODATA 2022), unit system (optional), **common formulas (v2.2: Rust implementations, for rapid optimization, §18.6)** | Explicit |
 **symbolic** | Advanced symbolic operations (differentiation, integration, series expansion, equation solving) | Explicit |
 **optimize** | Optimization algorithms (gradient descent, Newton's method, BFGS, constrained optimization) | Explicit |
 **sys** | Low-level system operations: `sys::path` (cross-platform paths), `sys::env` (environment), `sys::os` (platform-specific), **v2.2 extensions: process, filesystem metadata, terminal** | Explicit |
 **time** | Time system: `now`, `Duration`, formatting, clocks | Explicit |
 **num** | More complex numeric types (`BigInt` algorithmic extensions, `Complex` utilities) and additional numeric operations | Explicit |
 **ops** | Operator overloading interface (`impl Add for T` etc., §18.5) | Explicit |
 **mem** | **Memory (v2.2)**: `mem::Arc` explicit reference counting (§12.3/12.4); GC control (`collect`) | Explicit |
 **c_api** | C ABI types (`int`/`uint`/`float`/`double`/`bool`/`char`/`ptr`…) and `@c_api::extern` export support (§18.4) | Explicit |

### 18.1 Strings (the core-preimported `String` class and f-strings)

**Literals (v2.2)**:

- **Ordinary strings**: `"..."` and `'...'` are **equivalent**, both supporting escapes (including `\u{XXXX}` Unicode escapes).
- **Raw strings**: `r"..."` / `r'...'` — **escapes are not processed** (`\n` is a literal backslash + `n`, `\u{XXXX}` is not expanded).
- **f-strings**: `f"..."` / `f'...'` — `{expr}` interpolation, `{:spec}` format refinements, `{{`/`}}` escapes; combinable with raw strings (`rf"..."`).

**Escape sequences**: `\n` `\t` `\r` `\\` `\"` `\'` `\0` `\a` `\b` `\f` `\v` `\u{XXXX}` (any Unicode code point).

**f-string rules** (replacing the `format` function of v2.1):

```prima
let s = f"a is {a}";                    // expression interpolation
let t = f"{x} + {y} = {x + y}";
print(f"value = {v}");                  // any printable value can be interpolated
let u = f"{{literal braces}}";          // {{ → {, }} → }
let w = f"{pi:0.2}";                    // format refinement: float precision 3.14
```

- Inside `{...}` is any Prima expression (**nested f-string literals are not allowed in v2.2**; nesting is a compile-time error).
- Interpolated expressions are rendered according to the `print_format` policy (default LaTeX); `Result`/`Option` arguments show their success/failure summary.
- Format refinements `{:spec}` are being extended gradually (float precision `{x:0.2}`, alignment, padding, etc.); the syntax and supported items are governed by the `.pra` module doc comments.
- **The `format` function has been removed**: calling a function named `format` produces the transitional warning `W0006` pointing to f-strings (§16.5); after the warning is removed in the target version, it is reported as an undefined name.

**The `String` class** (embedded `core/string.pra`, v2.2): its method set **references Python 3's stable `str` methods**, adapted to Prima's conventions (case conversion, find/replace, split/join, padding/alignment, Unicode, slicing/iteration, serialization conversion, etc.). **The concrete method list and usage documentation are governed by the `///` doc comments of the embedded `.pra` module** (viewable via `prima doc` or the diagnostic note, §4.1/16.4); the spec no longer enumerates them one by one, listing only an illustration here:

```prima
pub class String {
    /// Returns the number of Unicode scalar values in `self`.
    pub fn len(self) -> Integer
    pub fn from(value) -> Self
    pub fn split(self, sep: Self) -> Array<String>
    pub fn to_upper(self) -> Self
    // ... the full method set is in core/string.pra's doc comments
}
```

- **Performance layering**: hot methods (`split`/`replace`/`to_upper`/`to_lower`, etc.) are provided as Rust implementations via `@builtin(O1)`/`@builtin(O2)`; low-frequency methods are written directly in `.pra` (§18.4's layered-optimization mechanism).

### 18.1b Console Output and Input (core-preimported, v2.1)

**Semantic distinction** between `print` and `println` (finalized in v2.1):

```prima
print("hello");             // 输出 "hello"，不追加换行
println("hello");           // 输出 "hello" 并换行
print("a", "b");            // 多参数以空格分隔：a b（不换行）
println("x =", x);          // 同上但末尾换行
```

**Rules**:

- `print(args...)`: formats and outputs each argument in turn, with arguments separated by a **single space** and **no trailing newline** (`print("\n")` can be used to emit a newline manually).
- `println(args...)`: same as `print`, but appends a **newline at the end**.
- Both render arguments according to the `print_format` policy (default LaTeX).

**Input (v2.1)**:

```prima
let name = input("Name: ");        // 打印提示（可选）并读取一行，返回 String（去掉末尾换行）
let n = read_line();               // 无提示读取一行
let v = input("n = ").try_f64();   // 读取并坍缩（配合 try_* 家族，§九）
```

**Rules**:

- `input(prompt?) -> String`: optionally prints a prompt to stdout (no newline), then reads one line from stdin (stripping a trailing `\r\n`/`\n`); at EOF it returns the empty string.
- `read_line() -> String`: equivalent to `input()` without a prompt.
- When unavailable in the interactive CLI/REPL it is treated as the empty string (I/O errors do not panic).

### 18.2 The `sys` Module (low-level system operations)

**`sys::path` (cross-platform paths)**:

```prima
import sys::path;
let p = path::join("a", "b");           // "a/b"（Linux/macOS）或 "a\\b"（Windows）
let n = path::file_name(p);             // Option<String>
let ext = path::extension(p);           // Option<String>
let parent = path::parent(p);           // Option<String>
let abs = path::is_absolute(p);         // Bool
```

**`sys::env` (cross-platform environment)**:

```prima
import sys::env;
let home = env::home_dir();             // Option<String>
let path_var = env::get("PATH");        // Option<String>
let args = env::args();                 // Array<String>（命令行参数）
let cwd = env::current_dir();           // String
```

**`sys::os` (platform-specific features)**:

```prima
import sys::os;
let name = os::name();                  // "linux" / "macos" / "windows" / ...
let arch = os::arch();                  // "x86_64" / "aarch64" / ...
os::exit(0);                            // 立即退出进程
```

### 18.3 The `time` Module (time system)

```prima
import time;
let now = time::now();                  // 当前时间戳
let d = time::Duration::from_secs(5);
time::sleep(d);
let ts = time::unix_timestamp(now);     // I64
let s = time::format(now, "%Y-%m-%d");  // 格式化
let parsed = time::parse("2024-01-01", "%Y-%m-%d");  // Result
```

### 18.4 Interoperability: `@builtin` and `@c_api::extern`

#### `@builtin` (implemented in Rust, for writing the real standard library)

Applied to `fn`/`class` declarations, indicating the implementation is provided by the Rust host; at runtime the name is bound to a registered builtin implementation. **v2.2 supports the layered-optimization form `@builtin(ON)`** (`O0`–`O3`, §10.2), letting the same function have both a Rust implementation and a `.pra` original implementation, with the optimization level deciding which one is used.

**Two forms**:

1. **`@builtin(O0)`** (equivalent to bare `@builtin`, the original v2.1 semantics):
   - **A function body is not allowed** (a body raises `E0056`);
   - The Rust implementation **must** be registered; unregistered raises `E0055`;
   - The Rust implementation is used at any `opt_level`.

2. **`@builtin(ON)` (`N = 1..3`, layered optimization)**:
   - **A function body is required** (the `.pra` original implementation, serving as the fallback/baseline);
   - The Rust implementation is **optional** (unregistered does not error);
   - At evaluation: if the **current `opt_level` policy is ≥ `N`** and a Rust implementation is registered → call the Rust implementation; otherwise evaluate the `.pra` original implementation in the body.
   - The two implementations must agree semantically (the `.pra` version is the only source of observable semantics; the Rust implementation is its performance layer).
   - Parameter/return types are identical across the two implementations, constrained by the single signature.

```prima
// O0: must have no body; the Rust implementation must be registered
@builtin
pub fn print(args...)

// O1+: Rust implementation optional; when opt_level < 1 the .pra body is evaluated
@builtin(O1)
pub fn to_upper(self: String) -> String {
    // .pra original implementation (the path below O0/O1)
    ...
}

@builtin(O2)
pub fn split(self: String, sep: String) -> Array<String> {
    ...
}
```

**Rules**:

- An illegal level argument to `@builtin(OX)` (not `O0`–`O3`) → compile-time error `E0057`.
- The registration mechanism (on the Rust side) is finalized in §Implementation Plan (key `"module::function"`, preferring **declarative macros** to simplify registration and avoid hand-written string keys).
- Standard-library method docs continue to be maintained as usual in the `.pra` `///` comments (§18.1).

#### `@c_api::extern` (exporting a C ABI interface)

Applied to `pub fn`, exporting the function with the C calling convention into a binary (`.so`/`.dylib`/`.dll`/executable) so it can be called from C/Rust/other languages.

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

**Rules**:

- Only `pub fn` (host functions) can be exported; parameters/return values must be C-compatible types under `c_api::*` (§Appendix B.6).
- The compiled artifact includes a C header (`prima compile --emit-headers`).
- Strings cross the boundary via `c_api::cstring` (converted by the host into a Prima `String` when passed in, and vice versa).

### 18.5 The `ops` Module (operator overloading)

**`ops` provides the operator-overloading interface**, defining operator semantics for classes via `impl <Op> for <Class>`.

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

**Overloadable operators** (provided by `ops`): `Add`, `Sub`, `Mul`, `Div`, `Rem`, `Neg`, `Eq`, `Cmp`, `Index`.

**Policy control** (§13.2):

```prima
with config { overload_policy := allow } {   // 解除 W0005 警告
    let c = a + b;
}
with config { overload_policy := deny } {    // 使用即报错
    let c = a + b;            // 错误
}
```

**Rules**:

- Overloading does **not affect** operators on the built-in numeric types (`Integer + Integer` and so on always use the built-in semantics).
- Call sites of overloaded operators are governed by `overload_policy`, which decides between warning/passing/erroring.
- Overloading `Eq`/`Cmp` changes the semantics of comparisons such as `==`/`<`; `Index` overloads `obj[i]`.

### 18.6 Standard Library Extensions (v2.2)

> The **method-level lists** of the modules below **are governed by the `///` doc comments of their respective embedded `.pra` modules** (§18 management principle); this section gives the responsibility scope and key implementation choices.

**`math` numeric tools**:

- Integer algorithms: **factorization** (trial division / Pollard rho), **prime sieve**, `gcd`/`lcm`/CRT (Chinese remainder theorem), modular integer exponentiation.
- Polynomials and series: **Taylor expansion** (`taylor(f, x, x0, n)` → a truncated power series), polynomial add/sub/mul/div, evaluation, root-finding, continued fractions.
- Generally written in `.pra`, with hot spots (such as the core of large-number factorization) layered to Rust via `@builtin(ON)` (§18.4).

**`physics` common formulas and Classes**:

- **Common formulas are implemented directly in Rust** (`@builtin`/`@builtin(ON)`) for rapid optimization of such computations; example scope: kinematics (uniform/uniformly accelerated linear motion, projectiles), mechanics (Newton's laws, energy/momentum), simple harmonic motion, basic thermodynamics, basic electromagnetism (Coulomb, Ohm), etc.
- Provides **Classes** (such as `Vector3`, `Unit`, etc.) and methods; the `physics` module also keeps the CODATA physical constants (§7.3).
- Teaching/engineering orientation: focused on elementary physics formulas with units; the unit system (§22.4) remains a future extension.

**`sys` system-interaction extensions**:

- Process: submodules such as `sys::process` (run commands, exit codes, output capture), `sys::fs` (file metadata, directory traversal), `sys::term` (terminal size, raw mode), following the full-path binding convention of §18.2.

**`plot` plotting**: line/scatter/bar/contour/heatmap; SVG is the default backend, PNG optional (`savefig` format parameter). **`render` formula rendering**:

- `render::to_svg(expr)` / `render::to_png(expr)`: render an expression (`Expr`/f-string result) into a formula image (reusing the §7 ExprDAG renderer).
- `render::to_terminal(expr)`: a terminal-text formula (ASCII/Unicode math); **the optional terminal formula rendering when `print` outputs formulas is provided via a cargo feature** (§Implementation Plan §1).
- Both use the `print_format` policy as the default style.

**`mem`**: `mem::Arc` explicit reference counting (§12.3/12.4), `mem::collect()` to trigger the GC manually.

---

## 19. Compiler and Runtime Implementation Path

### 19.1 MVP (interpreter + symbolic engine)

**Core components**:

- **Lexing**: hand-written lexer (§Implementation Plan).
- **Parsing**: hand-written recursive descent + Pratt.
- **Symbolic layer**: `ExprPool` (hash-consing) + `Number`/`Value` + simplification engine.
- **Rendering**: LaTeX output + Unicode/ASCII alternatives.
- **Numeric layer**:
  - Arbitrary precision: `num-bigint` + `num-rational` + `num-complex` (pure Rust, MIT/Apache-2.0).
  - Optional acceleration: `rug` (GMP bindings, LGPL) as a feature flag (`--features=rug-backend`).
  - Matrices: `nalgebra` (general purpose) or `faer` (high performance, optimized memory layout).
- **Policy system**: `config {}` parsed at compile time, stored as `ThreadLocal<Config>` at runtime.
- **Module system**: filesystem mapping + `pub`/`pub(mod)` visibility + `import` resolution.
- **Explicit collapse**: §9's collapse function family, with type checking.
- **Diagnostics**: numbered errors/warnings (§16).

**MVP milestones**:

1. **Basic symbolic computation**:

   ```prima
   import core;
   let a = tex"\sqrt{2}+\pi";
   print(a);                    // LaTeX 输出：\sqrt{2} + \pi
   ```

2. **Simplification and evaluation**:

   ```prima
   const b = tex"\e^{i\pi}+1";
   let c = simplify(b);         // → 0
   print(c);                    // 0
   ```

3. **Functions and broadcasting**:

   ```prima
   let f(x) = x^2;
   let v = [1, 2, 3];
   print(f(v));                 // [1, 4, 9]
   ```

4. **Policies take effect**:

   ```prima
   config { fraction := false }
   let x = 1/3;
   print(x);                    // 0.333... (F64)
   ```

5. **Loop optimization**:

   ```prima
   let s = 0;
   for i in 1..100 { s += i; }  // 编译器优化为 s = 100*101/2
   print(s);                    // 5050
   ```

6. **Parallel annotations**:

   ```prima
   let f(x) @parallel = x^2;
   let v = range(0, 1000000);
   let w = f(v);                // 自动并行广播
   ```

7. **Error handling (Result + ?)**:

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

8. **Class and method chaining**:

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

9. **Strings and f-strings**:

   ```prima
   let s = "e = \u{03B5}";          // ordinary string: Unicode escape
   let t = s.to_upper();
   print(t);                    // "E = Ε"
   let msg = f"parsed {x}";     // f-string: expression interpolation
   ```

### 19.2 Phase Two: Performance Optimization and JIT

**JIT compilation**:

- **Technology choice**: `inkwell` (LLVM bindings) or `cranelift-codegen` (lightweight code generation, prioritizing compile speed).
- **Hybrid execution**: the symbolic layer is interpreted; numeric hot spots are JIT-compiled to native code.
- **Compilation triggers**:
  - A function is called more than a threshold number of times (e.g. 100).
  - An explicit `@jit` annotation: `let f(x) @jit = x^2 + sin(x)`.
- **Batched algorithms**: integrate with BLAS (OpenBLAS/MKL) and LAPACK via `nalgebra` or `faer`.

**Optimization pipeline rollout** (§10.2): the JIT phase applies constant folding, CSE, loop optimization, and **automatic inlining**; the AOT phase additionally performs dead-code elimination and module-level optimization.

**Example flow**:

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

**Composable optimization**:

```prima
let f(x) = x^2 + 1;
let g = jit(grad(f));         // 组合：自动微分 + JIT 编译
print(g(3.0));                // 原生速度的梯度计算
```

### 19.3 Phase Three: AOT Compilation

**Goals**:

- Generate standalone executables (no runtime dependencies).
- Support WebAssembly (browser/edge computing).

**Technical path**:

1. **Query-based compilation** (modeled on rustc):
   - module dependency graph → incremental compilation.
   - cache `ExprId` simplification results and type-inference information.
2. **LLVM backend**:
   - whole-program analysis → inlining, dead-code elimination (the full §10.2 pipeline).
   - link BLAS/LAPACK static libraries.
3. **WASM backend**:
   - export JS interfaces via `wasm-bindgen`.
   - the symbolic layer compiles to WASM; the numeric layer uses WASM SIMD.
4. **C ABI export** (§18.4): `prima compile --emit-c-abi` produces a `.so`/header.

**Commands**:

```bash
prima compile src/main.pra -o outputs/build/myapp       # 本机可执行文件
prima compile src/main.pra --target wasm32 -o app.wasm  # WebAssembly
prima compile src/main.pra --emit-c-abi -o libhello     # C ABI 动态库 + 头文件
```

### 19.4 Automatic Differentiation (a differentiator; implement it early)

**Implementation is staged**:

#### MVP stage: symbolic differentiation (included in core preimports in v2.1)

- **ExprDAG-based symbolic differentiation**: recursively applies the differentiation rules (sum/difference/product/quotient, power, chain, `sin/cos/tan/exp/ln/log/sqrt/abs`).
- **Interface (core-preimported)**: `derivative(expr, var)` / `partial(expr, var)` / `grad(expr)` / `limit(expr, var, a)`, accepting a **symbolic expression** or an **MFn name** (`derivative(f, x)` is equivalent to `derivative(f's function body, x)`).
- **Examples**:

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

#### Phase two: forward-mode AD (numeric)

- **Dual-number implementation**:

  ```rust
  struct Dual {
      val: f64,
      grad: f64,
  }
  ```

- **Overloading arithmetic operations**:

  ```prima
  fn eval_dual(f: MFn, x: Dual) -> Dual {
      // 自动传播梯度
  }
  ```

- **Use case**: gradient computation (few inputs, many outputs).

#### Phase three: reverse-mode AD (deep-learning style)

- **Computation graph + tape**: records the forward computation and back-propagates gradients.
- **Memory management**: arena allocator + lifetime management.
- **Composability**:

  ```prima
  let loss(w) = sum((y - predict(X, w))^2);
  let grad_loss = grad(loss);   // 反向模式自动微分
  let jit_grad = jit(grad_loss);  // JIT 编译梯度函数
  ```

**Reference implementations**:

- `alkahest` (Julia; combines symbolic + AD + JIT).
- `enzyme` (LLVM plugin for automatic differentiation).
- `zygote.jl` (Julia reverse-mode AD).

---

## 20. Project Directory Layout

**Design rule**: **all code (entry point / import section / modules) goes in `src/`**; **configuration and the README stay in the project root**; run artifacts go into `outputs/`.

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

### Rules

1. **`src/main.pra` = the project entry point / root module**. Polluting policies (§13.1) may only appear here.
2. **One `.pra` file = one module body**; **one directory = one submodule**, whose `main.pra` is the directory-module entry point.
3. **`config {}` plus all `import`s must be at the top of the file**, followed by the code.
4. **Modules are linked via `import`**; variables are not shared across modules (§15).
5. **Configuration** (`config.toml` / `prima.toml`) and the **README** live in the project root, not in `src/`.
6. **Run artifacts** all go into `outputs/`; `src/` stays pure source.

### CLI Commands

Provided by the `prima` CLI:

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

# Documentation generation (v2.2: parses the `///`/`//!` doc comments, covering the project and the built-in standard library)
prima doc                        # outputs Markdown to stdout
prima doc -o docs/api.md         # writes to a file
prima doc --stdlib               # outputs only the built-in standard library (including core/string.pra, etc.) documentation
```

---

## 21. Summary of Decided Decisions

## | Decision | Conclusion |

---|------|------|
 1 | Default exactness | Expressions/fractions take priority; `sqrt(2)` is kept as an Expr (§6) |
 2 | Indeterminate and undefined | `Indeterminate` is reducible in the symbolic layer; `Undefined` raises an error in the numeric layer (§6.2) |
 3 | Result output | LaTeX by default, preserving exactness (§10) |
 4 | Forced evaluation | The collapse function family: `to_<type>()`/`try_<type>()`/`checked_<type>()`, etc. (§9) |
 5 | Memory | Hash-consed symbolic layer (thread-local cache + global pool) + stack-value numeric layer + RC/GC host layer (§8.1/12.4) |
 6 | Compilation | Interpreter + symbolic engine → LLVM JIT → optional AOT (§19) |
 7 | Execution | Divided into three layers: W_symbol/W_numeric/W_host (§2) |
 8 | Special values | `Indeterminate` is reducible in the symbolic layer; `Undefined` cannot enter operations; `NaN/Inf` exist only after collapse (§6.2) |
 9 | LaTeX | Two-way bridge + built-in symbols independent of TeX (§7) |
 10 | Loop optimization | On by default (§10) |
 11 | Broadcasting | On by default; rejects nested/empty arrays (§11.4) |
 12 | Import syntax | Python-style `import` (§15.1) |
 13 | Module visibility | Private by default; `pub`/`pub(mod)` make public (§15.2) |
 14 | Parallel syntax | `@parallel` (+`parfor`), no implicit parallelism (§17) |
 15 | Exact complex | Built-in fixed rules (Julia's promotion idea); inexactness contagion is explicit (§6.4) |
 16 | Built-in symbols | Independent of TeX; math constants + operators + physical constants (CODATA) (§7) |
 17 | Operator evaluation | Lazily preserved by default; numerified only when a forced-evaluation function is encountered (§9.10) |
 18 | Preimports | Only core, with all common names exposed (§15.5) |
 19 | Error handling | **Rust-style `Result`/`?`/`match`, no `try/catch`** (§16) |
 20 | Overflow | Configured via the `num_to_big` policy (§13.2) |
 21 | Physical constant naming | Short names not exported by default; access via qualified long names (§7.3) |
 22 | Collapse function naming | `to_<type>` / `try_<type>` / `checked_<type>` / `clamped_<type>` / `rounded_<type>` (§9.1-9.6) |
 23 | Name/extension | **Prima / `.pra`**, entry point `src/main.pra` (§Naming & 20) |
 24 | Directories | Code into `src/`, config/README in the root, artifacts into `outputs/` (§20) |
 25 | Type system | Rust-style inference + explicit annotations; strict separation of symbolic/numeric (§6.3) |
 26 | Domain propagation | Highest domain at simplification; outer domain takes priority at assignment; `with_domain` for explicit conversion (§6.5) |
 27 | Policy system | Three-level hierarchy: global (polluting) > module > local (§13) |
 28 | Indexing syntax | Rust-style: `v[i]`, `M[i, j]`, `v[1..3]`, `M[.., j]` (§11.3) |
 29 | Automatic differentiation | MVP symbolic differentiation → forward AD (dual numbers) → reverse AD (tape) (§19.4) |
 30 | Implementation choices | `num-*` (pure Rust) as the base; `rug` (GMP) as optional acceleration (§19.1) |
 31 | Statement separation | **`;` per the spec; newline separation deprecated (W0001) and being gradually removed** (§4.2) |
 32 | Patterns/destructuring | **Rust-style full patterns: `if let`/`while let`/`match` + tuple/array/class/constructor/range patterns** (§4.4) |
 33 | Class | **Aggregate type of fields + methods, `Self`/`new`/`self`; shallow-copy sharing + deep copy for primitive values** (§4.5/12.3) |
 34 | Pipeline | **`\|>` deprecated (W0002), replaced by class method chains** (§9.7) |
 35 | Warning system | **Numbered `W####`, English code list in Appendix C; removable via policy** (§16.5) |
 36 | Strings (v2.2) | **`format` removed, replaced by f-strings `f"..."`; the `"..."`/`'...'` double delimiters + raw strings `r"..."`; the `String` class method set references Python `str`, with the list in the `.pra` doc comments** (§18.1) |
 37 | Collapse types | **One-to-one correspondence with Rust primitive numerics: i8…u128/isize/usize/f32/f64** (§6.1/9) |
 38 | Interoperability | **`@builtin` Rust implementations + `@c_api::extern` C ABI export** (§18.4) |
 39 | Optimization | **The `opt_level` graded optimization pipeline (`O0`–`O3`) + automatic inlining (no developer control) + constant folding/CSE/loop optimization/TCO/SIMD (O3)** (§10.2) |
 40 | Operator overloading | **`impl` in the `ops` module; `overload_policy` defaults to `warn` (W0005)** (§18.5) |
 41 | Standard library extensions | **`sys` (path/env/os/process/fs/term), `time`, `num`, `ops`, `c_api`, `mem`, `render`; `math`/`physics`/`plot` extended per v2.2** (§18) |
 42 | Array semantics (v2.1) | **`Array` is a variable-length mutable sequence whose elements may be any value, with a full method set, and may be nested as data**; broadcasting limited to numeric homogeneous arrays (§11.3/11.4) |
 43 | Collection types (v2.1) | **`Dict`/`Set` are primitive types**: literals, indexing, methods, membership tests, set algebra (§4.6/11.6) |
 44 | Comprehensions (v2.1) | **`[x for ...]`/`{k: v for ...}`/`{x for ...}`** unified iteration protocol (§11.7) |
 45 | Console (v2.1) | **`print` without newline / `println` with newline**; `input`/`read_line` read from stdin (§18.1b) |
 46 | Convenience functions (v2.1) | **`len`/`enumerate`/`sorted`/`reversed`/`sum`/`prod`/`min`/`max`/`all`/`any`/`zip`/`join`, etc.** core-preimported (Appendix B) |
 47 | Parallelism details (v2.1) | **`@parallel` broadcasting parallelizes above a threshold; `parfor` allows only index-slot writes (E0082)** (§17) |
 48 | Symbolic differentiation (v2.1) | **`derivative`/`partial`/`grad`/`limit` in core**, ExprDAG-based symbolic differentiation (§19.4) |
 49 | Doc comments (v2.2) | **`///`/`//!` normative doc comments are preserved in the AST; `prima doc` covers the project and the built-in standard library; on a method-call error the note attaches the method definition and docs** (§4.1/16.4) |
 50 | Optimization levels (v2.2) | **The `opt_level` policy (`O0`–`O3`, default `O2`) controls the set of optimization passes; aggressive passes such as SIMD recognition live at `O3`** (§10.2/13.2) |
 51 | `@builtin(O1)` (v2.2) | **Layered optimization: when `opt_level ≥ N`, the Rust implementation is used; otherwise the `.pra` original implementation is evaluated; the original `@builtin(O0)` semantics are retained** (§18.4) |
 52 | Standard-library list (v2.2) | **A module's method-level list has the `.pra` `///` doc comments as its sole source; the spec/implementation docs maintain only the module catalog** (§18) |
 53 | Memory (v2.2) | **Host-layer GC replaces reference counting; `mem::Arc` provides explicit reference counting, `mem::collect()` for manual GC** (§12.3/12.4) |

---

### 22. Future Extension Directions

#### 22.1 Macro System (reserved)

- The keyword `macro` is reserved for future compile-time code generation.
- Modeled on Rust's declarative macros + procedural macros.

#### 22.2 Async Support (reserved)

- The keywords `async`/`await` are reserved for future asynchronous I/O.
- Modeled on the Rust `async-std`/`tokio` ecosystem.

#### 22.3 Trait System (experimental)

- The keyword `trait` is reserved for future generic constraints.
- The `ops` module's operator overloading (§18.5) is a precursor to the trait system; the `impl Add for T` syntax connects with it.

#### 22.4 Unit System (optional module)

- Physical quantities carry units: `let v = 3.0 * meter / second;`.
- Compile-time unit checking, to avoid a Mars Climate Orbiter-style disaster.

#### 22.5 GPU Acceleration (phase three)

- The `@gpu` annotation: auto-generates CUDA/OpenCL/WGSL code.
- Matrix operations and parallel loops are automatically offloaded to the GPU.

---

### Appendix A: Full Grammar BNF

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
                   | "@builtin" ("(" opt_level ")")?     // defaults to O0 (§18.4)
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
// v2.2: ordinary strings `"..."`/`'...'` (equivalent escapes); raw strings `r"..."`/`r'...'` (no escapes)
string           ::= "\"" char* "\"" | "'" char* "'"
                   | "r" "\"" char* "\"" | "r" "'" char* "'"
// v2.2: f-strings `f"..."`/`f'...'` (`{expr}` interpolation, `{:spec}` refinements, `{{`/`}}` escapes); `rf"..."` combination
f_string         ::= "f" string_tpl | "rf" string_tpl
string_tpl       ::= "\"" string_part* "\"" | "'" string_part* "'"
string_part      ::= tpl_char | "{{" | "}}" | "{" tpl_expr (":" tpl_spec)? "}"
tpl_expr         ::= expr        // v2.2: nested f-string literals are not allowed
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

> Note: after the `=>` of a `match_arm` comes an expression; statements within a `block` end with `;` (block-level statements may omit the trailing `;`, §4.2).

---

### Appendix B: Standard Library Function Reference

#### B.1 Core (preimported)

##### Collapse functions

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

##### Mathematical functions

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

##### Simplification and symbolic operations

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

##### Strings and formatting

```prima
format(fmt, args...)           // 格式化生成 String（§18.1）
to_string(x)                   // 任意值转 String
String::new(), String::from(v)
s.push(t), s.insert(i, t), s.len(), s.is_empty()
s.char_at(i), s.substring(a, b), s.contains(p), s.starts_with(p), s.ends_with(p)
s.replace(f, t), s.trim(), s.strip(p), s.split(sep), s.join(parts)
s.find(p), s.to_upper(), s.to_lower(), s.repeat(n)
```

##### Console (v2.1, §18.1b)

```prima
print(args...)                 // 格式化输出，参数空格分隔，不追加换行（v2.1）
println(args...)               // 同 print，末尾追加换行
input(prompt?)                 // 打印提示（可选）并读取一行 → String
read_line()                    // 无提示读取一行 → String
```

##### Collection convenience functions (v2.1, core-preimported)

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

##### Utility functions

```prima
map(f, array)                  // 映射
filter(pred, array)            // 过滤
reduce(f, array, init)         // 归约
```

##### Built-in variant constructors (core-preimported)

```prima
Some(x)     // Option 的 Some
None        // Option 的 None
Ok(x)       // Result 的 Ok
Err(e)      // Result 的 Err
```

#### B.2 Linalg (explicitly imported)

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

#### B.3 Stats (explicitly imported)

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

#### B.4 Plot (explicitly imported)

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

#### B.5 Sys / Time / Num / Ops (explicitly imported)

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

#### B.6 C ABI types (`sys::c_api`, explicitly imported)

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

---

### Appendix C: Error & Warning Codes

> The code list is in **English**. Compile-time errors are `E####`; runtime errors are `R####`; warnings are `W####`. All codes appear in diagnostic output as `error[CODE]`/`warning[CODE]` (§16.4).

#### C.1 Compile-time Errors (E)

 Code | Name | Meaning |
----|------|------|
 `E0001` | `lex_error` | Lexical error (illegal character/unclosed literal) |
 `E0010` | `syntax_error` | Syntax error (including hints about removed syntax, such as `try/catch`) |
 `E0011` | `expected_separator` | Expected a `;` statement separator |
 `E0020` | `config_position` | `config {}` is not at the top of the file |
 `E0021` | `polluting_config` | Polluting policy declared in a non-entry file |
 `E0022` | `unknown_config` | Unknown policy key |
 `E0030` | `module_not_found` | Module does not exist |
 `E0031` | `import_conflict` | Conflict among imported symbols |
 `E0032` | `private_item` | Access to a private item |
 `E0040` | `undefined_name` | Undefined name |
 `E0041` | `duplicate_definition` | Duplicate definition |
 `E0050` | `type_mismatch` | Type mismatch |
 `E0051` | `missing_type_ann` | Missing type annotation |
 `E0052` | `unknown_type` | Unknown type |
 `E0053` | `irrefutable_pattern` | `let` used a refutable pattern |
 `E0054` | `try_operator_context` | `?` used outside a function returning `Result`/`Option` |
 `E0055` | `unregistered_builtin` | `@builtin` (`O0`) without a registered implementation |
 `E0056` | `builtin_body` | `@builtin(O0)` functions/classes must not have a body |
 `E0057` | `invalid_opt_level` | `@builtin(OX)` has an invalid optimization-level argument (v2.2; must be `O0`–`O3`, §18.4) |
 `E0060` | `unknown_field` | Struct literal/class pattern references an unknown field |
 `E0061` | `missing_field` | Struct literal is missing a field |
 `E0062` | `self_outside_method` | `self`/`Self` used outside a class method |
 `E0063` | `self_not_first` | `self` is not the first parameter of a method |
 `E0070` | `unknown_annotation` | Unknown annotation |
 `E0071` | `c_api_type` | `@c_api::extern` parameter/return is not a C-compatible type |
 `E0072` | `c_api_visibility` | `@c_api::extern` function is not `pub` |
  `E0080` | `return_outside_fn` | `return` used outside a function |
  `E0081` | `op_overload_bad_arity` | Illegal operator-overload function signature |
  `E0082` | `parfor_side_effect` | `parfor` loop body contains side effects (v2.1; only index-slot assignment/pure function calls allowed, §17.2) |

#### C.2 Runtime Errors (R)

 Code | Name | Meaning |
----|------|------|
  `R0001` | `overflow` | Overflow (`checked_*` returns Err) |
  `R0002` | `underflow` | Underflow |
  `R0003` | `index_out_of_bounds` | Index out of bounds (including negative-index out of bounds, v2.1 §11.3) |
  `R0004` | `dimension_mismatch` | Dimension mismatch |
  `R0005` | `domain_error` | Domain error |
  `R0006` | `undefined_error` | `Undefined` participates in an operation |
  `R0007` | `io_error` | I/O error |
  `R0008` | `import_error` | Runtime module loading error |
  `R0009` | `type_error` | Runtime type mismatch (including broadcasting encountering a non-numeric element, v2.1 §11.4) |
  `R0010` | `cast_error` | Collapse failure (`to_*`/`try_*`) |
  `R0011` | `custom_error` | Custom error (`panic`/`Err`) |
  `R0012` | `key_not_found` | `Dict` key does not exist (v2.1 §11.6) |
  `R0013` | `not_found` | `Array.index`/`Set.remove` target not found (v2.1 §11.3/11.6) |
  `R0014` | `empty_collection` | Broadcasting or reducing over an empty array/collection (v2.1 §11.4) |

#### C.3 Warnings (W)

 Code | Name | Meaning |
----|------|------|
 `W0001` | `newline_statement_separator` | Newline-separated statements (deprecated; use `;`) |
 `W0002` | `deprecated_pipeline` | The `\|>` pipeline is deprecated; use class methods |
 `W0003` | `unused_binding` | Binding is unused |
 `W0004` | `unreachable_code` | Unreachable code |
 `W0005` | `overloaded_operator` | Operator overloading in use (removed via `overload_policy := allow`) |

---

*Prima language specification v2.2 · the authoritative basis for the design and implementation of Prima*
