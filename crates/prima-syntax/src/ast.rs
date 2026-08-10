use crate::span::Span;

#[derive(Debug, Clone, PartialEq)]
pub struct Spanned<T> {
    pub value: T,
    pub span: Span,
}

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

#[derive(Debug, Clone, PartialEq)]
pub enum Stmt {
    Let {
        name: Spanned<String>,
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
    Return {
        value: Option<Expr>,
        span: Span,
    },
    Try {
        body: Block,
        catches: Vec<Catch>,
        span: Span,
    },
    WithConfig {
        entries: Vec<ConfigEntry>,
        body: Block,
        span: Span,
    },
    Pub(Box<Stmt>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Catch {
    pub var: Spanned<String>,
    pub ty: Option<Type>,
    pub block: Block,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Param {
    pub name: Spanned<String>,
    pub type_ann: Option<Type>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Annotation {
    Parallel,
    Jit,
    Gpu,
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
    I32,
    Complex,
    Expr,
    Symbol,
    Bool,
    String,
    Char,
    Array(Box<Type>),
    Matrix(Box<Type>),
    Tuple(Vec<Type>),
    Fn { params: Vec<Type>, ret: Box<Type> },
    MFn { params: Vec<Type>, ret: Box<Type> },
    User(Spanned<String>),
}

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
    Call { callee: Box<Expr>, args: Vec<Expr> },
    Index { base: Box<Expr>, index: Index },
    Binary { op: BinOp, lhs: Box<Expr>, rhs: Box<Expr> },
    Unary { op: UnOp, operand: Box<Expr> },
    Array(Vec<Expr>),
    Tuple(Vec<Expr>),
    Lambda { params: Vec<Param>, body: Box<Expr> },
    Match { scrutinee: Box<Expr>, arms: Vec<MatchArm> },
    Pipeline { lhs: Box<Expr>, rhs: Box<Expr> },
    Custom(Vec<(Expr, Expr)>),
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
    Broadcast,
    Pipeline,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnOp {
    Neg,
    Not,
    Pos,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MatchArm {
    pub pattern: Pattern,
    pub body: Expr,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Pattern {
    Literal(Literal),
    Binding(Spanned<String>),
    Wildcard(Span),
    Path(Vec<Spanned<String>>),
    Ctor {
        name: Vec<Spanned<String>>,
        args: Vec<Pattern>,
        span: Span,
    },
}
