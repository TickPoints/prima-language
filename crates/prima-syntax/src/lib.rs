pub mod ast;
pub mod error;
pub mod lexer;
pub mod parser;
pub mod span;
pub mod tex;
pub mod token;

pub use ast::{Expr, ExprKind, Program, Stmt};
pub use error::{SyntaxError, SyntaxWarning};
pub use span::{SourceLocation, Span};
pub use token::{Token, TokenKind};

/// Parse `.pra` source into a `Program` (spec §4). Returns all collected syntax errors.
pub fn parse(src: &str) -> Result<Program, Vec<SyntaxError>> {
    parser::parse(src)
}

/// Parse `.pra` source, returning the program plus all collected errors and warnings (spec §16.4/§16.5).
pub fn parse_checked(src: &str) -> (Program, Vec<SyntaxError>, Vec<SyntaxWarning>) {
    parser::parse_checked(src)
}
