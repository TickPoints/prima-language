/// Indeterminate form (spec §6.2): mathematically undefined forms (0/0 etc.) that exist **only in the symbolic layer**;
/// they can take part in later simplification; when collapse to the numeric layer fails they become `Undefined`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum IndeterminateForm {
    ZeroOverZero,
    InfOverInf,
    ZeroTimesInf,
    InfMinusInf,
}

/// Value type (spec §5): covers the value forms of each layer of the three-world architecture —
/// the symbolic layer (`Expr`/`Symbol`/`Indeterminate`), the numeric layer (`Number`), and the host layer (`Bool`/`String`/`Error`, etc.).
/// `Result`/`Error` carry a structured `Error` as a message string (the structured enum from spec §16.1 is deferred to a later stage).
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Nil,
    Number(crate::number::Number),
    Bool(bool),
    Char(char),
    String(String),
    Array(Vec<crate::number::Number>),
    Expr(crate::expr_pool::ExprId),
    Symbol(u32),
    Indeterminate(IndeterminateForm),
    Undefined,
    Error(String),
    Tuple(Vec<Value>),
    Result(std::result::Result<Box<Value>, String>),
}
