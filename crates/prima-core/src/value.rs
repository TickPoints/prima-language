use crate::error::CoreError;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum IndeterminateForm {
    ZeroOverZero,
    InfOverInf,
    ZeroTimesInf,
    InfMinusInf,
}

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
    Error(CoreError),
    Tuple(Vec<Value>),
    Result(std::result::Result<Box<Value>, CoreError>),
}
