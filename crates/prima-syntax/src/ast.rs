use crate::span::Span;

#[derive(Debug, Clone, PartialEq)]
pub struct Spanned<T> {
    pub value: T,
    pub span: Span,
}

/// A single AST covers the entire grammar (spec §4). The three-section order `config → import → statement`
/// is validated at parse time (spec §4.1, appendix A BNF `program` production).
#[derive(Debug, Clone, PartialEq)]
pub struct Program {
    pub config: Option<ConfigBlock>,
    pub imports: Vec<Import>,
    pub stmts: Vec<Stmt>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ConfigBlock {
    pub entries: Vec<ConfigEntry>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ConfigEntry {
    pub name: Spanned<String>,
    pub type_ann: Option<Type>,
    pub value: Expr,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Import {
    pub kind: ImportKind,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ImportKind {
    Namespace { path: Vec<Spanned<String>>, alias: Option<Spanned<String>> },
    From { path: Vec<Spanned<String>>, items: Vec<ImportItem> },
}

#[derive(Debug, Clone, PartialEq)]
pub enum ImportItem {
    Star,
    Name { name: Spanned<String>, alias: Option<Spanned<String>> },
}

/// Visibility modifier (spec §15.2): default private / `pub(mod)` / `pub`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Visibility {
    Private,
    Module,
    Public,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Stmt {
    Let {
        pat: Pattern,
        mut_: bool,
        type_ann: Option<Type>,
        value: Expr,
        span: Span,
    },
    Const {
        name: Spanned<String>,
        type_ann: Type,
        value: Expr,
        span: Span,
    },
    FnDef {
        name: Spanned<String>,
        params: Vec<Param>,
        ret: Option<Type>,
        annotations: Vec<Annotation>,
        body: Block,
        span: Span,
    },
    MathDef {
        name: Spanned<String>,
        params: Vec<Param>,
        ret: Option<Type>,
        annotations: Vec<Annotation>,
        body: Expr,
        span: Span,
    },
    /// Class definition (spec §4.5): fields + methods. Statement-level annotations (`@builtin`,
    /// spec §18.4) are recorded so the checker can reject unregistered builtin classes.
    ClassDef {
        name: Spanned<String>,
        annotations: Vec<Annotation>,
        members: Vec<ClassMember>,
        span: Span,
    },
    /// Operator overload via `impl ops::Add for T` (spec §18.5).
    Impl {
        op: ImplOp,
        target: Spanned<String>,
        members: Vec<Box<Stmt>>,
        span: Span,
    },
    Assign {
        target: Expr,
        op: AssignOp,
        value: Expr,
        span: Span,
    },
    Expr(Expr),
    For {
        var: Spanned<String>,
        range: (Expr, Expr),
        step: Option<Expr>,
        body: Block,
        span: Span,
    },
    ParFor {
        var: Spanned<String>,
        range: (Expr, Expr),
        step: Option<Expr>,
        body: Block,
        span: Span,
    },
    While {
        cond: Expr,
        body: Block,
        span: Span,
    },
    If {
        cond: Expr,
        then: Block,
        elifs: Vec<(Expr, Block)>,
        else_: Option<Block>,
        span: Span,
    },
    /// `if let pattern = expr { ... } else { ... }` (spec §4.4).
    IfLet {
        pat: Pattern,
        value: Expr,
        then: Block,
        else_: Option<Block>,
        span: Span,
    },
    /// `while let pattern = expr { ... }` (spec §4.4).
    WhileLet {
        pat: Pattern,
        value: Expr,
        body: Block,
        span: Span,
    },
    /// `match` used as a statement (spec §4.4); the expression form is `ExprKind::Match`.
    Match {
        scrutinee: Expr,
        arms: Vec<MatchArm>,
        span: Span,
    },
    Return {
        value: Option<Expr>,
        span: Span,
    },
    WithConfig {
        entries: Vec<ConfigEntry>,
        body: Block,
        span: Span,
    },
    Pub(Box<Stmt>),
}

/// Class member (spec §4.5): a field or a method. The outer visibility modifier comes from the parse layer.
#[derive(Debug, Clone, PartialEq)]
pub struct ClassMember {
    pub vis: Visibility,
    pub kind: ClassMemberKind,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ClassMemberKind {
    Field { name: Spanned<String>, ty: Type },
    /// Method with an optional body: `@builtin` classes declare signature-only methods (spec §18.4).
    Method { name: Spanned<String>, params: Vec<Param>, ret: Option<Type>, annotations: Vec<Annotation>, body: Option<Block> },
}

/// Operator overload target of `impl ops::X for T` (spec §18.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImplOp {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Neg,
    Eq,
    Cmp,
    Index,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Param {
    pub name: Spanned<String>,
    pub type_ann: Option<Type>,
    /// `self` receiver of a method (spec §4.5): a shallow copy of the object.
    pub is_self: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Annotation {
    Parallel,
    Jit,
    Gpu,
    /// `@builtin`: implementation provided by the Rust host (spec §18.4).
    Builtin,
    /// `@c_api::extern`: export a C ABI interface (spec §18.4).
    CApiExtern,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssignOp {
    Assign,
    AddAssign,
    SubAssign,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Block {
    pub stmts: Vec<Stmt>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Type {
    Number,
    Integer,
    Rational,
    F64,
    F32,
    I8,
    I16,
    I32,
    I64,
    I128,
    U8,
    U16,
    U32,
    U64,
    U128,
    Isize,
    Usize,
    Complex,
    Expr,
    Symbol,
    Bool,
    String,
    Char,
    Array(Box<Type>),
    Matrix(Box<Type>),
    Tuple(Vec<Type>),
    Option(Box<Type>),
    Result(Box<Type>, Box<Type>),
    Fn { params: Vec<Type>, ret: Box<Type> },
    MFn { params: Vec<Type>, ret: Box<Type> },
    /// `Self` inside a class body (spec §4.5).
    SelfType,
    User(Spanned<String>),
}

/// Match arm (spec §4.4): pattern with an optional guard, `pattern [if cond] => expr`.
#[derive(Debug, Clone, PartialEq)]
pub struct MatchArm {
    pub pattern: Pattern,
    pub guard: Option<Expr>,
    pub body: Expr,
}

/// Rust-style patterns (spec §4.4) for `let` destructuring, `if let`/`while let`, and `match` arms.
#[derive(Debug, Clone, PartialEq)]
pub enum Pattern {
    Wildcard(Span),
    Binding(Spanned<String>),
    Literal(Literal),
    /// `(a, b, ..)` — trailing `..` is allowed.
    Tuple(Vec<Pattern>, bool),
    /// `[x, y, ..]` — trailing `..` is allowed.
    Array(Vec<Pattern>, bool),
    /// `Type { x, y: pat, .. }`.
    Struct {
        name: Spanned<String>,
        fields: Vec<FieldPattern>,
        rest: bool,
    },
    /// `Some(x)` / `Ok(v)` / `Err(e)` / `None`.
    Variant { name: Spanned<String>, args: Vec<Pattern>, span: Span },
    /// `0..9` / `1..=5` (inclusive range).
    Range { lo: Literal, hi: Literal, inclusive: bool },
    /// `pat1 | pat2` (or-pattern).
    Or(Vec<Pattern>),
    /// `(pat)`.
    Group(Box<Pattern>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct FieldPattern {
    pub name: Spanned<String>,
    pub pat: Option<Pattern>,
}

/// Math and host expressions **share the same AST** (spec §4.3: `math_expr := expr`).
/// The "symbol world / numeric world" distinction lives not at the parse layer but in the runtime demotion layer (implementation plan §4.8).
#[derive(Debug, Clone, PartialEq)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ExprKind {
    Literal(Literal),
    Symbol(Spanned<String>),
    Path { segments: Vec<Spanned<String>> },
    /// `self` expression (spec §4.5).
    Self_,
    Call { callee: Box<Expr>, args: Vec<Expr> },
    /// `obj.method(args)` (spec §4.5).
    MethodCall { receiver: Box<Expr>, name: Spanned<String>, args: Vec<Expr> },
    /// `obj.field` (spec §4.5 field access).
    Field { receiver: Box<Expr>, name: Spanned<String> },
    /// `T { a, b, ..base }` struct literal (spec §4.5).
    StructLiteral { name: Spanned<String>, fields: Vec<FieldValue>, base: Option<Box<Expr>> },
    Index { base: Box<Expr>, index: Index },
    Binary { op: BinOp, lhs: Box<Expr>, rhs: Box<Expr> },
    Unary { op: UnOp, operand: Box<Expr> },
    /// `expr?` try operator (spec §16.3): propagates `Err`/`None` in a `Result`/`Option`-returning function.
    Try(Box<Expr>),
    Array(Vec<Expr>),
    Tuple(Vec<Expr>),
    /// `{ key: value, ... }` dict literal (spec §4.6): an ordered list of key/value pairs.
    Dict(Vec<(Expr, Expr)>),
    /// `{ a, b, ... }` set literal (spec §4.6). Duplicate elements are kept by the parser; dedup happens at runtime.
    Set(Vec<Expr>),
    /// Comprehension `[...]`/`{...}`/`(...)` (spec §4.6/§11.7): the frame kind + output expression + `for`/`if` clauses.
    Comprehension { kind: CompKind, output: Box<Expr>, clauses: Vec<ComprehensionClause> },
    /// `key: value` — internal node used only as the `output` of a Dict comprehension; never appears in normal expressions.
    KeyValue { key: Box<Expr>, value: Box<Expr> },
    Lambda { params: Vec<Param>, body: Box<Expr> },
    Match { scrutinee: Box<Expr>, arms: Vec<MatchArm> },
    Pipeline { lhs: Box<Expr>, rhs: Box<Expr> },
    Custom(Vec<(Expr, Expr)>),
}

/// Frame kind of a comprehension (spec §4.6 rule 4): the enclosing bracket decides the produced collection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompKind {
    Array,
    Dict,
    Set,
    Tuple,
}

/// One clause of a comprehension (spec §11.7): `for <var> in <iter>` or `if <cond>`, in any order and any count.
#[derive(Debug, Clone, PartialEq)]
pub enum ComprehensionClause {
    For { var: Spanned<String>, iter: Expr },
    If { cond: Expr },
}

#[derive(Debug, Clone, PartialEq)]
pub struct FieldValue {
    pub name: Spanned<String>,
    pub value: Option<Expr>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Literal {
    Integer(String),
    Float(String),
    Hex(String),
    Binary(String),
    Str(String),
    Char(char),
    Bool(bool),
    Tex(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Index {
    pub items: Vec<IndexItem>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum IndexItem {
    Elem(Expr),
    Slice { start: Option<Expr>, end: Option<Expr> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Pow,
    MatMul,
    Mod,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
    /// Membership test `x in c` (spec §11.3/§11.6).
    In,
    /// Set union `∪` (spec §11.6).
    Union,
    /// Set intersection `∩` (spec §11.6).
    Intersect,
    /// Set difference `\` (spec §11.6).
    Difference,
    Broadcast,
    Pipeline,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnOp {
    Neg,
    Not,
    Pos,
}
