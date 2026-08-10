pub mod ast;
pub mod error;
pub mod lexer;
pub mod parser;
pub mod span;
pub mod tex;
pub mod token;

pub use ast::{Expr, ExprKind, Program, Stmt};
pub use error::SyntaxError;
pub use span::{SourceLocation, Span};
pub use token::{Token, TokenKind};

/// Parse `.pra` source into a `Program` (spec §4). Returns all collected syntax errors.
pub fn parse(src: &str) -> Result<Program, Vec<SyntaxError>> {
    parser::parse(src)
}
