pub mod ast;
pub mod error;
pub mod lexer;
pub mod parser;
pub mod span;
pub mod token;

pub use ast::{Expr, ExprKind, Program, Stmt};
pub use error::SyntaxError;
pub use span::{SourceLocation, Span};
pub use token::{Token, TokenKind};

pub fn parse(src: &str) -> Result<Program, Vec<SyntaxError>> {
    parser::parse(src)
}
