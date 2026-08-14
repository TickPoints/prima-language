use crate::error::SyntaxError;
use crate::span::Span;
use crate::token::{describe, Token, TokenKind};

/// Lexing (spec §3): produces a token stream including `Newline`; errors are returned in collection form.
/// Numeric literals keep their **raw text** (`TokenKind::Integer("0x1F")`); numeric parsing happens in the core layer.
pub fn lex(src: &str) -> Result<Vec<Token>, Vec<SyntaxError>> {
    Lexer { src: src.as_bytes(), pos: 0 }.run()
}

// Hand-written lexer (implementation plan §2.1): advances by character class, giving exact token-level errors and spans per literal kind.
struct Lexer<'a> {
    src: &'a [u8],
    pos: usize,
}

enum Class {
    Space,
    Newline,
    Ident,
    Number,
    String,
    RawString,
    Char,
    Symbol,
    /// Backslash used as the binary set-difference operator (spec §11.6), e.g. `s \ {3}`.
    SetMinus,
    Comment,
    Punct,
}

enum NumKind {
    Int,
    Float,
    Hex,
    Bin,
}

impl<'a> Lexer<'a> {
    fn cur(&self) -> Option<char> {
        self.src
            .get(self.pos..)
            .and_then(|r| std::str::from_utf8(r).ok())
            .and_then(|s| s.chars().next())
    }

    fn peek(&self, n: usize) -> Option<u8> {
        self.src.get(self.pos + n).copied()
    }

    fn advance(&mut self, n: usize) {
        self.pos = (self.pos + n).min(self.src.len());
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.cur()?;
        self.advance(c.len_utf8());
        Some(c)
    }

    /// Character `n` bytes ahead of the current position (used for lookahead over multi-byte chars).
    fn peek_char(&self, n: usize) -> Option<char> {
        self.src
            .get(self.pos + n..)
            .and_then(|r| std::str::from_utf8(r).ok())
            .and_then(|s| s.chars().next())
    }

    fn starts_with(&self, s: &str) -> bool {
        self.src.get(self.pos..).is_some_and(|r| r.starts_with(s.as_bytes()))
    }

    fn classify(&self) -> Option<Class> {
        let c = self.cur()?;
        Some(match c {
            ' ' | '\t' => Class::Space,
            '\n' | '\r' => Class::Newline,
            '0'..='9' => Class::Number,
            '"' => Class::String,
            '\'' => Class::Char,
            'r' if self.peek(1) == Some(b'"') => Class::RawString,
            'a'..='z' | 'A'..='Z' | '_' => Class::Ident,
            // `\` followed by an identifier start is a TeX symbol (`\pi`); otherwise it is the set-difference operator (spec §11.6).
            '\\' => {
                if self.peek_char(1).is_some_and(unicode_ident::is_xid_start) {
                    Class::Symbol
                } else {
                    Class::SetMinus
                }
            }
            '/' if self.peek(1) == Some(b'/') || self.peek(1) == Some(b'*') => Class::Comment,
            _ if unicode_ident::is_xid_start(c) => Class::Ident,
            _ => Class::Punct,
        })
    }

    fn run(&mut self) -> Result<Vec<Token>, Vec<SyntaxError>> {
        let mut tokens = Vec::new();
        let mut errors = Vec::new();
        loop {
            let start = self.pos;
            let Some(class) = self.classify() else { break };
            match class {
                Class::Space => {
                    self.bump();
                }
                Class::Newline => {
                    self.bump();
                    if self.cur() == Some('\n') {
                        self.bump();
                    }
                    tokens.push(Token { kind: TokenKind::Newline, span: Span::new(start as u32, self.pos as u32) });
                }
                Class::Ident => {
                    let ident = self.read_ident();
                    if ident == "_" {
                        tokens.push(Token { kind: TokenKind::Underscore, span: Span::new(start as u32, self.pos as u32) });
                    } else if ident == "tex" && self.cur() == Some('"') {
                        self.bump();
                        match self.read_until_quote(false) {
                            Ok(v) => tokens.push(Token { kind: TokenKind::TexStr(v), span: Span::new(start as u32, self.pos as u32) }),
                            Err(message) => errors.push(SyntaxError { span: Span::new(start as u32, self.pos as u32), message }),
                        }
                    } else {
                        tokens.push(Token { kind: keyword_or_ident(&ident), span: Span::new(start as u32, self.pos as u32) });
                    }
                }
                Class::Number => match self.read_number() {
                    Ok(kind) => {
                        let text = std::str::from_utf8(&self.src[start..self.pos]).unwrap_or_default().to_string();
                        let kind = match kind {
                            NumKind::Int => TokenKind::Integer(text),
                            NumKind::Float => TokenKind::Float(text),
                            NumKind::Hex => TokenKind::Hex(text),
                            NumKind::Bin => TokenKind::Binary(text),
                        };
                        tokens.push(Token { kind, span: Span::new(start as u32, self.pos as u32) });
                    }
                    Err(message) => errors.push(SyntaxError { span: Span::new(start as u32, self.pos as u32), message }),
                },
                Class::String => {
                    self.bump();
                    match self.read_until_quote(true) {
                        Ok(v) => tokens.push(Token { kind: TokenKind::Str(v), span: Span::new(start as u32, self.pos as u32) }),
                        Err(message) => errors.push(SyntaxError { span: Span::new(start as u32, self.pos as u32), message }),
                    }
                }
                Class::RawString => {
                    self.bump();
                    self.bump();
                    match self.read_until_quote(false) {
                        Ok(v) => tokens.push(Token { kind: TokenKind::Str(v), span: Span::new(start as u32, self.pos as u32) }),
                        Err(message) => errors.push(SyntaxError { span: Span::new(start as u32, self.pos as u32), message }),
                    }
                }
                Class::Char => match self.read_char() {
                    Ok(c) => tokens.push(Token { kind: TokenKind::Char(c), span: Span::new(start as u32, self.pos as u32) }),
                    Err(message) => errors.push(SyntaxError { span: Span::new(start as u32, self.pos as u32), message }),
                },
                Class::Symbol => {
                    self.bump();
                    if let Some(name) = self.read_ident_after_symbol() {
                        tokens.push(Token { kind: TokenKind::Symbol(name), span: Span::new(start as u32, self.pos as u32) });
                    } else {
                        errors.push(SyntaxError { span: Span::new(start as u32, self.pos as u32), message: "expected an identifier after `\\`".into() });
                    }
                }
                Class::SetMinus => {
                    self.bump();
                    tokens.push(Token { kind: TokenKind::SetMinus, span: Span::new(start as u32, self.pos as u32) });
                }
                Class::Comment => {
                    if let Some(message) = self.skip_comment() {
                        errors.push(SyntaxError { span: Span::new(start as u32, self.pos as u32), message });
                    }
                }
                Class::Punct => match self.operator() {
                    Ok(kind) => tokens.push(Token { kind, span: Span::new(start as u32, self.pos as u32) }),
                    Err(message) => {
                        self.bump();
                        errors.push(SyntaxError { span: Span::new(start as u32, self.pos as u32), message });
                    }
                },
            }
        }
        tokens.push(Token { kind: TokenKind::Eof, span: Span::new(self.pos as u32, self.pos as u32) });
        if errors.is_empty() { Ok(tokens) } else { Err(errors) }
    }

    fn read_ident(&mut self) -> String {
        let mut s = String::new();
        while let Some(c) = self.cur() {
            if unicode_ident::is_xid_continue(c) {
                s.push(c);
                self.bump();
            } else {
                break;
            }
        }
        s
    }

    fn read_ident_after_symbol(&mut self) -> Option<String> {
        let mut s = String::new();
        while let Some(c) = self.cur() {
            if unicode_ident::is_xid_continue(c) {
                s.push(c);
                self.bump();
            } else {
                break;
            }
        }
        if s.is_empty() { None } else { Some(s) }
    }

    fn read_while(&mut self, f: impl Fn(u8) -> bool) -> String {
        let mut s = String::new();
        while let Some(b) = self.peek(0) {
            if f(b) {
                s.push(b as char);
                self.pos += 1;
            } else {
                break;
            }
        }
        s
    }

    // Numeric literals (spec §3): decimal integer/float, hexadecimal, binary; `..` in `1..10` is not confused with a decimal point.
    fn read_number(&mut self) -> Result<NumKind, String> {
        let c = self.cur().unwrap();
        if c == '0' {
            match self.peek(1) {
                Some(b'x') | Some(b'X') => {
                    self.advance(2);
                    let digits = self.read_while(|b| b.is_ascii_hexdigit());
                    if digits.is_empty() {
                        return Err("invalid hexadecimal literal".into());
                    }
                    return Ok(NumKind::Hex);
                }
                Some(b'b') | Some(b'B') => {
                    self.advance(2);
                    let digits = self.read_while(|b| b == b'0' || b == b'1');
                    if digits.is_empty() {
                        return Err("invalid binary literal".into());
                    }
                    return Ok(NumKind::Bin);
                }
                _ => {}
            }
        }
        let _int = self.read_while(|b| b.is_ascii_digit());
        let mut is_float = false;
        if self.cur() == Some('.') && self.peek(1).is_some_and(|b| b.is_ascii_digit()) {
            is_float = true;
            self.bump();
            self.read_while(|b| b.is_ascii_digit());
        }
        if self.cur().is_some_and(|c| c == 'e' || c == 'E') && self.has_exp_digits() {
            is_float = true;
            self.bump();
            if matches!(self.cur(), Some('+') | Some('-')) {
                self.bump();
            }
            self.read_while(|b| b.is_ascii_digit());
        }
        if is_float {
            Ok(NumKind::Float)
        } else {
            Ok(NumKind::Int)
        }
    }

    fn has_exp_digits(&self) -> bool {
        let p1 = self.peek(1);
        if p1.is_some_and(|b| b.is_ascii_digit()) {
            return true;
        }
        matches!(p1, Some(b'+') | Some(b'-')) && self.peek(2).is_some_and(|b| b.is_ascii_digit())
    }

    fn read_until_quote(&mut self, escapes: bool) -> Result<String, String> {
        let mut value = String::new();
        loop {
            let Some(c) = self.bump() else {
                return Err("unterminated string literal".into());
            };
            if c == '"' {
                return Ok(value);
            }
            if !escapes {
                value.push(c);
                continue;
            }
            if c != '\\' {
                value.push(c);
                continue;
            }
            let Some(ec) = self.bump() else {
                return Err("unterminated string literal".into());
            };
            match ec {
                'n' => value.push('\n'),
                't' => value.push('\t'),
                'r' => value.push('\r'),
                '0' => value.push('\0'),
                '\\' => value.push('\\'),
                '"' => value.push('"'),
                '\'' => value.push('\''),
                'u' => {
                    if self.bump() != Some('{') {
                        return Err("invalid unicode escape".into());
                    }
                    let mut hex = String::new();
                    loop {
                        let Some(h) = self.bump() else {
                            return Err("invalid unicode escape".into());
                        };
                        if h == '}' {
                            break;
                        }
                        if h.is_ascii_hexdigit() {
                            hex.push(h);
                        } else {
                            return Err("invalid unicode escape".into());
                        }
                    }
                    if hex.is_empty() {
                        return Err("invalid unicode escape".into());
                    }
                    let cp = u32::from_str_radix(&hex, 16).map_err(|_| "invalid unicode escape".to_string())?;
                    let ch = char::from_u32(cp).ok_or_else(|| "invalid unicode escape".to_string())?;
                    value.push(ch);
                }
                _ => return Err(format!("invalid escape sequence `\\{ec}`")),
            }
        }
    }

    fn read_char(&mut self) -> Result<char, String> {
        self.bump();
        let Some(c) = self.bump() else {
            return Err("unterminated character literal".into());
        };
        let ch = if c == '\\' {
            let Some(ec) = self.bump() else {
                return Err("unterminated character literal".into());
            };
            match ec {
                'n' => '\n',
                't' => '\t',
                'r' => '\r',
                '0' => '\0',
                '\\' => '\\',
                '\'' => '\'',
                '"' => '"',
                _ => return Err(format!("invalid escape sequence `\\{ec}`")),
            }
        } else {
            c
        };
        if self.bump() == Some('\'') {
            Ok(ch)
        } else {
            Err("expected closing `'`".into())
        }
    }

    fn skip_comment(&mut self) -> Option<String> {
        if self.starts_with("//") {
            while let Some(c) = self.cur() {
                if c == '\n' {
                    break;
                }
                self.bump();
            }
            None
        } else {
            self.advance(2);
            while !self.starts_with("*/") {
                if self.cur().is_none() {
                    return Some("unterminated block comment".into());
                }
                self.bump();
            }
            self.advance(2);
            None
        }
    }

    fn operator(&mut self) -> Result<TokenKind, String> {
        let c = self.cur().unwrap();
        let (kind, len) = match c {
            '+' => {
                if self.starts_with("+=") {
                    (TokenKind::PlusEq, 2)
                } else {
                    (TokenKind::Plus, 1)
                }
            }
            '-' => {
                if self.starts_with("->") {
                    (TokenKind::Arrow, 2)
                } else if self.starts_with("-=") {
                    (TokenKind::MinusEq, 2)
                } else {
                    (TokenKind::Minus, 1)
                }
            }
            '*' => {
                if self.starts_with("**") {
                    (TokenKind::DoubleStar, 2)
                } else {
                    (TokenKind::Star, 1)
                }
            }
            '/' => (TokenKind::Slash, 1),
            '^' => (TokenKind::Caret, 1),
            '%' => (TokenKind::Percent, 1),
            '@' => {
                if self.starts_with("@.") {
                    (TokenKind::AtDot, 2)
                } else {
                    (TokenKind::At, 1)
                }
            }
            '=' => {
                if self.starts_with("==") {
                    (TokenKind::EqEq, 2)
                } else if self.starts_with("=>") {
                    (TokenKind::FatArrow, 2)
                } else {
                    (TokenKind::Eq, 1)
                }
            }
            '!' => {
                if self.starts_with("!=") {
                    (TokenKind::BangEq, 2)
                } else {
                    (TokenKind::Bang, 1)
                }
            }
            '<' => {
                if self.starts_with("<=") {
                    (TokenKind::LtEq, 2)
                } else {
                    (TokenKind::Lt, 1)
                }
            }
            '>' => {
                if self.starts_with(">=") {
                    (TokenKind::GtEq, 2)
                } else {
                    (TokenKind::Gt, 1)
                }
            }
            '&' => {
                if self.starts_with("&&") {
                    (TokenKind::AmpAmp, 2)
                } else {
                    return Err("unexpected `&`".into());
                }
            }
            '|' => {
                if self.starts_with("||") {
                    (TokenKind::PipePipe, 2)
                } else if self.starts_with("|>") {
                    (TokenKind::PipeArrow, 2)
                } else {
                    (TokenKind::Pipe, 1)
                }
            }
            ':' => {
                if self.starts_with("::") {
                    (TokenKind::ColonColon, 2)
                } else if self.starts_with(":=") {
                    (TokenKind::ColonEq, 2)
                } else {
                    (TokenKind::Colon, 1)
                }
            }
            ',' => (TokenKind::Comma, 1),
            ';' => (TokenKind::Semicolon, 1),
            '?' => (TokenKind::Question, 1),
            '.' => {
                if self.starts_with("..=") {
                    (TokenKind::DotDotEq, 3)
                } else if self.starts_with("..") {
                    (TokenKind::DotDot, 2)
                } else {
                    (TokenKind::Dot, 1)
                }
            }
            '(' => (TokenKind::LParen, 1),
            ')' => (TokenKind::RParen, 1),
            '∪' => (TokenKind::Union, c.len_utf8()),
            '∩' => (TokenKind::Intersect, c.len_utf8()),
            '[' => (TokenKind::LBracket, 1),
            ']' => (TokenKind::RBracket, 1),
            '{' => (TokenKind::LBrace, 1),
            '}' => (TokenKind::RBrace, 1),
            _ => return Err(format!("unexpected character `{c}` ({})", describe(&TokenKind::Ident(c.to_string())))),
        };
        self.advance(len);
        Ok(kind)
    }
}

fn keyword_or_ident(s: &str) -> TokenKind {
    match s {
        "let" => TokenKind::KwLet,
        "mut" => TokenKind::KwMut,
        "const" => TokenKind::KwConst,
        "fn" => TokenKind::KwFn,
        "return" => TokenKind::KwReturn,
        "for" => TokenKind::KwFor,
        "in" => TokenKind::KwIn,
        "while" => TokenKind::KwWhile,
        "if" => TokenKind::KwIf,
        "else" => TokenKind::KwElse,
        "step" => TokenKind::KwStep,
        "class" => TokenKind::KwClass,
        "self" => TokenKind::KwSelf,
        "Self" => TokenKind::KwSelfType,
        "try" => TokenKind::KwTry,
        "catch" => TokenKind::KwCatch,
        "match" => TokenKind::KwMatch,
        "parfor" => TokenKind::KwParFor,
        "config" => TokenKind::KwConfig,
        "with" => TokenKind::KwWith,
        "pub" => TokenKind::KwPub,
        "import" => TokenKind::KwImport,
        "from" => TokenKind::KwFrom,
        "as" => TokenKind::KwAs,
        "true" => TokenKind::KwTrue,
        "false" => TokenKind::KwFalse,
        "async" => TokenKind::KwAsync,
        "yield" => TokenKind::KwYield,
        "macro" => TokenKind::KwMacro,
        "trait" => TokenKind::KwTrait,
        "impl" => TokenKind::KwImpl,
        _ => TokenKind::Ident(s.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(src: &str) -> Vec<TokenKind> {
        lex(src).expect("lex should succeed").into_iter().map(|t| t.kind).collect()
    }

    #[test]
    fn set_minus_backslash() {
        assert_eq!(kinds("s \\ {3}"), vec![
            TokenKind::Ident("s".into()),
            TokenKind::SetMinus,
            TokenKind::LBrace,
            TokenKind::Integer("3".into()),
            TokenKind::RBrace,
            TokenKind::Eof,
        ]);
    }

    #[test]
    fn backslash_symbol_still_works() {
        assert_eq!(kinds("\\pi"), vec![TokenKind::Symbol("pi".into()), TokenKind::Eof]);
    }

    #[test]
    fn union_and_intersect() {
        assert_eq!(kinds("a ∪ b ∩ c"), vec![
            TokenKind::Ident("a".into()),
            TokenKind::Union,
            TokenKind::Ident("b".into()),
            TokenKind::Intersect,
            TokenKind::Ident("c".into()),
            TokenKind::Eof,
        ]);
    }

    #[test]
    fn backslash_before_operator_is_set_minus() {
        assert_eq!(kinds("s \\ ∩ {1}"), vec![
            TokenKind::Ident("s".into()),
            TokenKind::SetMinus,
            TokenKind::Intersect,
            TokenKind::LBrace,
            TokenKind::Integer("1".into()),
            TokenKind::RBrace,
            TokenKind::Eof,
        ]);
    }
}
