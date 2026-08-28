use crate::error::SyntaxError;
use crate::span::Span;
use crate::token::{FStringToken, Token, TokenKind, describe};

/// Lexing (spec §3): produces a token stream including `Newline`; errors are returned in collection form.
/// Numeric literals keep their **raw text** (`TokenKind::Integer("0x1F")`); numeric parsing happens in the core layer.
pub fn lex(src: &str) -> Result<Vec<Token>, Vec<SyntaxError>> {
    Lexer {
        src: src.as_bytes(),
        pos: 0,
    }
    .run()
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
    FString,
    RawFString,
    Char,
    Symbol,
    /// Backslash used as the binary set-difference operator (spec §11.6), e.g. `s \ {3}`.
    SetMinus,
    /// `///`/`//!` doc comment (spec §4.1), lexed as a `Doc` token so the parser can attach it.
    Doc,
    Comment,
    Punct,
}

enum NumKind {
    Int,
    Float,
    Hex,
    Bin,
}

/// Result of a single-quoted literal (spec §3): one character is a `Char`, otherwise a `Str`.
enum Quoted {
    Char(char),
    Str(String),
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
        self.src
            .get(self.pos..)
            .is_some_and(|r| r.starts_with(s.as_bytes()))
    }

    fn classify(&self) -> Option<Class> {
        let c = self.cur()?;
        Some(match c {
            ' ' | '\t' => Class::Space,
            '\n' | '\r' => Class::Newline,
            '0'..='9' => Class::Number,
            '"' => Class::String,
            '\'' => Class::Char,
            // String prefixes (spec §3/§18.1): `f"`/`f'` f-string, `r"`/`r'` raw, `rf"`/`rf'` raw f-string.
            // Only an immediate quote makes a prefix; `r`/`f`/`rf` used as identifiers keep `Class::Ident`.
            'r' => {
                let p1 = self.peek(1);
                if p1 == Some(b'f') && matches!(self.peek(2), Some(b'"') | Some(b'\'')) {
                    Class::RawFString
                } else if matches!(p1, Some(b'"') | Some(b'\'')) {
                    Class::RawString
                } else {
                    Class::Ident
                }
            }
            'f' => {
                if matches!(self.peek(1), Some(b'"') | Some(b'\'')) {
                    Class::FString
                } else {
                    Class::Ident
                }
            }
            'a'..='z' | 'A'..='Z' | '_' => Class::Ident,
            // `\` followed by an identifier start is a TeX symbol (`\pi`); otherwise it is the set-difference operator (spec §11.6).
            '\\' => {
                if self.peek_char(1).is_some_and(unicode_ident::is_xid_start) {
                    Class::Symbol
                } else {
                    Class::SetMinus
                }
            }
            '/' if self.peek(1) == Some(b'/')
                && matches!(self.peek(2), Some(b'/') | Some(b'!')) =>
            {
                Class::Doc
            }
            '/' if self.peek(1) == Some(b'/') || self.peek(1) == Some(b'*') => Class::Comment,
            _ if unicode_ident::is_xid_start(c) => Class::Ident,
            _ => Class::Punct,
        })
    }

    fn run(&mut self) -> Result<Vec<Token>, Vec<SyntaxError>> {
        let mut errors = Vec::new();
        let mut tokens = self.lex_all(&mut errors);
        tokens.push(Token {
            kind: TokenKind::Eof,
            span: Span::new(self.pos as u32, self.pos as u32),
        });
        if errors.is_empty() {
            Ok(tokens)
        } else {
            Err(errors)
        }
    }

    /// Tokenize the whole input into `tokens`, pushing any lexical errors into `errors`
    /// (collection-based, spec §16.2). `lex_range` reuses this on a sub-slice for f-string
    /// interpolation bodies while keeping absolute byte offsets.
    fn lex_all(&mut self, errors: &mut Vec<SyntaxError>) -> Vec<Token> {
        let mut tokens = Vec::new();
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
                    tokens.push(Token {
                        kind: TokenKind::Newline,
                        span: Span::new(start as u32, self.pos as u32),
                    });
                }
                Class::Ident => {
                    let ident = self.read_ident();
                    if ident == "_" {
                        tokens.push(Token {
                            kind: TokenKind::Underscore,
                            span: Span::new(start as u32, self.pos as u32),
                        });
                    } else if ident == "tex" && self.cur() == Some('"') {
                        self.bump();
                        match self.read_quoted(b'"', false) {
                            Ok(v) => tokens.push(Token {
                                kind: TokenKind::TexStr(v),
                                span: Span::new(start as u32, self.pos as u32),
                            }),
                            Err(message) => errors.push(SyntaxError {
                                span: Span::new(start as u32, self.pos as u32),
                                message,
                            }),
                        }
                    } else {
                        tokens.push(Token {
                            kind: keyword_or_ident(&ident),
                            span: Span::new(start as u32, self.pos as u32),
                        });
                    }
                }
                Class::Number => match self.read_number() {
                    Ok(kind) => {
                        let text = std::str::from_utf8(&self.src[start..self.pos])
                            .unwrap_or_default()
                            .to_string();
                        let kind = match kind {
                            NumKind::Int => TokenKind::Integer(text),
                            NumKind::Float => TokenKind::Float(text),
                            NumKind::Hex => TokenKind::Hex(text),
                            NumKind::Bin => TokenKind::Binary(text),
                        };
                        tokens.push(Token {
                            kind,
                            span: Span::new(start as u32, self.pos as u32),
                        });
                    }
                    Err(message) => errors.push(SyntaxError {
                        span: Span::new(start as u32, self.pos as u32),
                        message,
                    }),
                },
                Class::String => {
                    self.bump();
                    match self.read_quoted(b'"', true) {
                        Ok(value) => tokens.push(Token {
                            kind: TokenKind::Str {
                                value,
                                raw: false,
                                single: false,
                            },
                            span: Span::new(start as u32, self.pos as u32),
                        }),
                        Err(message) => errors.push(SyntaxError {
                            span: Span::new(start as u32, self.pos as u32),
                            message,
                        }),
                    }
                }
                Class::RawString => {
                    let quote = self.peek(1).expect("raw string quote");
                    self.advance(2);
                    match self.read_quoted(quote, false) {
                        Ok(value) => tokens.push(Token {
                            kind: TokenKind::Str {
                                value,
                                raw: true,
                                single: quote == b'\'',
                            },
                            span: Span::new(start as u32, self.pos as u32),
                        }),
                        Err(message) => errors.push(SyntaxError {
                            span: Span::new(start as u32, self.pos as u32),
                            message,
                        }),
                    }
                }
                Class::FString => {
                    let quote = self.peek(1).expect("f-string quote");
                    self.advance(2);
                    match self.read_fstring(quote, false) {
                        Ok(parts) => tokens.push(Token {
                            kind: TokenKind::FStr(parts),
                            span: Span::new(start as u32, self.pos as u32),
                        }),
                        Err(message) => errors.push(SyntaxError {
                            span: Span::new(start as u32, self.pos as u32),
                            message,
                        }),
                    }
                }
                Class::RawFString => {
                    let quote = self.peek(2).expect("raw f-string quote");
                    self.advance(3);
                    match self.read_fstring(quote, true) {
                        Ok(parts) => tokens.push(Token {
                            kind: TokenKind::FStr(parts),
                            span: Span::new(start as u32, self.pos as u32),
                        }),
                        Err(message) => errors.push(SyntaxError {
                            span: Span::new(start as u32, self.pos as u32),
                            message,
                        }),
                    }
                }
                Class::Char => match self.read_single_quoted() {
                    Ok(Quoted::Char(c)) => tokens.push(Token {
                        kind: TokenKind::Char(c),
                        span: Span::new(start as u32, self.pos as u32),
                    }),
                    Ok(Quoted::Str(value)) => tokens.push(Token {
                        kind: TokenKind::Str {
                            value,
                            raw: false,
                            single: true,
                        },
                        span: Span::new(start as u32, self.pos as u32),
                    }),
                    Err(message) => errors.push(SyntaxError {
                        span: Span::new(start as u32, self.pos as u32),
                        message,
                    }),
                },
                Class::Symbol => {
                    self.bump();
                    if let Some(name) = self.read_ident_after_symbol() {
                        tokens.push(Token {
                            kind: TokenKind::Symbol(name),
                            span: Span::new(start as u32, self.pos as u32),
                        });
                    } else {
                        errors.push(SyntaxError {
                            span: Span::new(start as u32, self.pos as u32),
                            message: "expected an identifier after `\\`".into(),
                        });
                    }
                }
                Class::SetMinus => {
                    self.bump();
                    tokens.push(Token {
                        kind: TokenKind::SetMinus,
                        span: Span::new(start as u32, self.pos as u32),
                    });
                }
                Class::Doc => {
                    let start = self.pos;
                    self.advance(3);
                    let module = self.src[start + 2] == b'!';
                    // Read to end of line; the text is the raw doc line minus the `///`/`//!` marker.
                    let text_start = self.pos;
                    while let Some(c) = self.cur() {
                        if c == '\n' {
                            break;
                        }
                        self.bump();
                    }
                    let text =
                        std::str::from_utf8(&self.src[text_start..self.pos]).unwrap_or_default();
                    // One optional leading space after the marker is stripped (Rust convention).
                    let text = text.strip_prefix(' ').unwrap_or(text).to_string();
                    tokens.push(Token {
                        kind: TokenKind::Doc { text, module },
                        span: Span::new(start as u32, self.pos as u32),
                    });
                }
                Class::Comment => {
                    if let Some(message) = self.skip_comment() {
                        errors.push(SyntaxError {
                            span: Span::new(start as u32, self.pos as u32),
                            message,
                        });
                    }
                }
                Class::Punct => match self.operator() {
                    Ok(kind) => tokens.push(Token {
                        kind,
                        span: Span::new(start as u32, self.pos as u32),
                    }),
                    Err(message) => {
                        self.bump();
                        errors.push(SyntaxError {
                            span: Span::new(start as u32, self.pos as u32),
                            message,
                        });
                    }
                },
            }
        }
        tokens
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

    /// Read a delimited string body after the opening quote has been consumed (spec §3/§18.1):
    /// returns the content with escapes processed when `escapes` is true. Works for both `"..."`
    /// and `'...'`, and for nested strings inside f-string interpolations.
    fn read_quoted(&mut self, quote: u8, escapes: bool) -> Result<String, String> {
        let mut value = String::new();
        loop {
            let Some(c) = self.bump() else {
                return Err("unterminated string literal".into());
            };
            if c as u8 == quote {
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
            value.push(self.read_escape()?);
        }
    }

    /// `'...'` (spec §3/§18.1): a single escaped character lexes as a `Char`, anything else
    /// (including the empty string) lexes as a single-quoted `Str`. The opening `'` is consumed.
    fn read_single_quoted(&mut self) -> Result<Quoted, String> {
        self.bump();
        let content = self.read_quoted(b'\'', true)?;
        let mut chars = content.chars();
        match (chars.next(), chars.next()) {
            (Some(c), None) => Ok(Quoted::Char(c)),
            _ => Ok(Quoted::Str(content)),
        }
    }

    /// One escape sequence after the backslash (spec §18.1): `\n` `\t` `\r` `\\` `\"` `\'`
    /// `\0` `\a` `\b` `\f` `\v` `\u{XXXX}`.
    fn read_escape(&mut self) -> Result<char, String> {
        let Some(ec) = self.bump() else {
            return Err("unterminated escape sequence".into());
        };
        match ec {
            'n' => Ok('\n'),
            't' => Ok('\t'),
            'r' => Ok('\r'),
            '0' => Ok('\0'),
            'a' => Ok('\x07'),
            'b' => Ok('\x08'),
            'f' => Ok('\x0c'),
            'v' => Ok('\x0b'),
            '\\' => Ok('\\'),
            '"' => Ok('"'),
            '\'' => Ok('\''),
            'u' => self.read_unicode_escape(),
            _ => Err(format!("invalid escape sequence `\\{ec}`")),
        }
    }

    /// `\u{XXXX}` Unicode escape (spec §18.1): any valid Unicode scalar value.
    fn read_unicode_escape(&mut self) -> Result<char, String> {
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
        char::from_u32(cp).ok_or_else(|| "invalid unicode escape".to_string())
    }

    /// Scan an f-string template (spec §18.1) after the `f`/`rf` prefix and the opening quote:
    /// literal segments with `{{`/`}}` escapes (and, unless `raw`, backslash escapes) alternate
    /// with `{expr}` interpolations whose bodies are lexed into a token stream.
    fn read_fstring(&mut self, quote: u8, raw: bool) -> Result<Vec<FStringToken>, String> {
        let mut parts = Vec::new();
        let mut lit = String::new();
        loop {
            let Some(c) = self.bump() else {
                return Err("unterminated f-string literal".into());
            };
            if c as u8 == quote {
                if !lit.is_empty() {
                    parts.push(FStringToken::Lit(lit));
                }
                return Ok(parts);
            }
            match c {
                '{' if self.cur() == Some('{') => {
                    self.bump();
                    lit.push('{');
                }
                '{' => {
                    if !lit.is_empty() {
                        parts.push(FStringToken::Lit(std::mem::take(&mut lit)));
                    }
                    let (expr, spec) = self.read_fstring_interp()?;
                    parts.push(FStringToken::Interp { expr, spec });
                }
                '}' if self.cur() == Some('}') => {
                    self.bump();
                    lit.push('}');
                }
                '}' => return Err("unmatched `}` in f-string literal".into()),
                '\\' if !raw => lit.push(self.read_escape()?),
                other => lit.push(other),
            }
        }
    }

    /// Scan one `{expr}` / `{expr:spec}` interpolation body (positioned just after the `{`):
    /// bracket and string nesting is tracked, a `:` at brace depth 0 (outside `::`) separates the
    /// format spec, and the expression body is lexed into a token stream with absolute spans.
    fn read_fstring_interp(&mut self) -> Result<(Vec<Token>, Option<String>), String> {
        let expr_start = self.pos;
        let mut depth = 0i32;
        let mut spec_colon: Option<usize> = None;
        loop {
            let Some(c) = self.bump() else {
                return Err("unterminated f-string interpolation".into());
            };
            if c == '"' || c == '\'' {
                // A nested string/char literal inside the expression must not confuse `{`/`}`.
                self.read_quoted(c as u8, true)?;
                continue;
            }
            if c == '{' {
                depth += 1;
                continue;
            }
            if c == '}' {
                if depth == 0 {
                    let interp_end = self.pos - 1;
                    let expr_end = spec_colon.unwrap_or(interp_end);
                    let spec = spec_colon.map(|s| self.slice_str(s + 1, interp_end));
                    let tokens = self.lex_range(expr_start, expr_end)?;
                    if tokens.iter().any(|t| matches!(t.kind, TokenKind::FStr(_))) {
                        return Err("nested f-string literals are not allowed (spec §18.1)".into());
                    }
                    return Ok((tokens, spec));
                }
                depth -= 1;
                continue;
            }
            if depth == 0 && c == ':' && self.peek(0) != Some(b':') && spec_colon.is_none() {
                spec_colon = Some(self.pos - 1);
                continue;
            }
            // The spec is `[^{}]+` (spec appendix A `tpl_spec`); an opening brace inside it is malformed.
            if spec_colon.is_some() && c == '{' {
                return Err("invalid character `{` in f-string format spec".into());
            }
        }
    }

    /// Slice the source bytes `[start, end)` back into a `String` (for the `:spec` text).
    fn slice_str(&self, start: usize, end: usize) -> String {
        std::str::from_utf8(&self.src[start..end])
            .unwrap_or_default()
            .to_string()
    }

    /// Lex the source bytes in `[start, end)` into a token stream with **absolute** spans
    /// (used for f-string interpolation bodies). Errors propagate as a message.
    fn lex_range(&mut self, start: usize, end: usize) -> Result<Vec<Token>, String> {
        // A sub-slice that ends at `end` makes the sub-lexer see EOF there while keeping
        // absolute byte positions (spans stay aligned with the whole source).
        let mut sub = Lexer {
            src: &self.src[..end],
            pos: start,
        };
        let mut errors = Vec::new();
        let mut tokens = sub.lex_all(&mut errors);
        tokens.push(Token {
            kind: TokenKind::Eof,
            span: Span::new(sub.pos as u32, sub.pos as u32),
        });
        if let Some(e) = errors.into_iter().next() {
            Err(e.message)
        } else {
            Ok(tokens)
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
            _ => {
                return Err(format!(
                    "unexpected character `{c}` ({})",
                    describe(&TokenKind::Ident(c.to_string()))
                ));
            }
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
        lex(src)
            .expect("lex should succeed")
            .into_iter()
            .map(|t| t.kind)
            .collect()
    }

    #[test]
    fn set_minus_backslash() {
        assert_eq!(
            kinds("s \\ {3}"),
            vec![
                TokenKind::Ident("s".into()),
                TokenKind::SetMinus,
                TokenKind::LBrace,
                TokenKind::Integer("3".into()),
                TokenKind::RBrace,
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn string_prefixes() {
        // Ordinary `"..."`, single-quoted string `'...'`, raw `r"..."`/`r'...'` (no escapes).
        assert_eq!(
            kinds(r#""a\nb""#),
            vec![
                TokenKind::Str {
                    value: "a\nb".into(),
                    raw: false,
                    single: false
                },
                TokenKind::Eof
            ]
        );
        assert_eq!(
            kinds(r#"'hello'"#),
            vec![
                TokenKind::Str {
                    value: "hello".into(),
                    raw: false,
                    single: true
                },
                TokenKind::Eof
            ]
        );
        assert_eq!(
            kinds(r#"r"a\nb""#),
            vec![
                TokenKind::Str {
                    value: "a\\nb".into(),
                    raw: true,
                    single: false
                },
                TokenKind::Eof
            ]
        );
        assert_eq!(
            kinds(r#"r'a\nb'"#),
            vec![
                TokenKind::Str {
                    value: "a\\nb".into(),
                    raw: true,
                    single: true
                },
                TokenKind::Eof
            ]
        );
    }

    #[test]
    fn single_quote_char_vs_string() {
        // A single character is a `Char`, a longer (or empty) literal is a string (spec appendix A).
        assert_eq!(kinds("'a'"), vec![TokenKind::Char('a'), TokenKind::Eof]);
        assert_eq!(
            kinds("'ab'"),
            vec![
                TokenKind::Str {
                    value: "ab".into(),
                    raw: false,
                    single: true
                },
                TokenKind::Eof
            ]
        );
        assert_eq!(
            kinds("''"),
            vec![
                TokenKind::Str {
                    value: String::new(),
                    raw: false,
                    single: true
                },
                TokenKind::Eof
            ]
        );
        assert_eq!(kinds("'\\n'"), vec![TokenKind::Char('\n'), TokenKind::Eof]);
    }

    #[test]
    fn fstrings_split_parts() {
        let toks = lex(r#"f"a{x} b""#).unwrap();
        match &toks[0].kind {
            TokenKind::FStr(parts) => {
                assert_eq!(parts.len(), 3);
                assert_eq!(parts[0], FStringToken::Lit("a".into()));
                let TokenKind::FStr(_) = toks[0].kind else {
                    unreachable!()
                };
                match &parts[1] {
                    FStringToken::Interp { expr, spec } => {
                        assert_eq!(spec, &None);
                        let kinds: Vec<_> = expr.iter().map(|t| &t.kind).collect();
                        assert_eq!(kinds, vec![&TokenKind::Ident("x".into()), &TokenKind::Eof]);
                    }
                    _ => panic!("expected interpolation"),
                }
                assert_eq!(parts[2], FStringToken::Lit(" b".into()));
            }
            _ => panic!("expected FStr token"),
        }
    }

    #[test]
    fn fstring_escapes_and_braces() {
        let toks = lex(r#"f"{{x}} = {1 + 2}""#).unwrap();
        match &toks[0].kind {
            TokenKind::FStr(parts) => {
                assert_eq!(parts[0], FStringToken::Lit("{x} = ".into()));
                match &parts[1] {
                    FStringToken::Interp { expr, spec } => {
                        assert_eq!(spec, &None);
                        let kinds: Vec<_> = expr.iter().map(|t| &t.kind).collect();
                        assert_eq!(
                            kinds,
                            vec![
                                &TokenKind::Integer("1".into()),
                                &TokenKind::Plus,
                                &TokenKind::Integer("2".into()),
                                &TokenKind::Eof
                            ]
                        );
                    }
                    _ => panic!("expected interpolation"),
                }
            }
            _ => panic!("expected FStr token"),
        }
    }

    #[test]
    fn fstring_spec_and_raw() {
        let toks = lex(r#"f"{pi:0.2}""#).unwrap();
        match &toks[0].kind {
            TokenKind::FStr(parts) => match &parts[0] {
                FStringToken::Interp { expr, spec } => {
                    assert_eq!(spec.as_deref(), Some("0.2"));
                    let kinds: Vec<_> = expr.iter().map(|t| &t.kind).collect();
                    assert_eq!(kinds, vec![&TokenKind::Ident("pi".into()), &TokenKind::Eof]);
                }
                _ => panic!("expected interpolation"),
            },
            _ => panic!("expected FStr token"),
        }
        // Raw f-string keeps literal backslashes outside interpolations.
        let toks = lex(r#"rf"a\nb{x}""#).unwrap();
        match &toks[0].kind {
            TokenKind::FStr(parts) => {
                assert_eq!(parts[0], FStringToken::Lit("a\\nb".into()));
            }
            _ => panic!("expected FStr token"),
        }
    }

    #[test]
    fn fstring_nested_is_error() {
        let err = lex(r#"f"a { f"b" } c""#).unwrap_err();
        assert!(
            err[0].message.contains("nested f-string"),
            "message = {}",
            err[0].message
        );
    }

    #[test]
    fn fstring_unterminated_is_error() {
        assert!(lex(r#"f"a {x"#).is_err());
        assert!(lex(r#"f"unterminated"#).is_err());
    }

    #[test]
    fn backslash_symbol_still_works() {
        assert_eq!(
            kinds("\\pi"),
            vec![TokenKind::Symbol("pi".into()), TokenKind::Eof]
        );
    }

    #[test]
    fn union_and_intersect() {
        assert_eq!(
            kinds("a ∪ b ∩ c"),
            vec![
                TokenKind::Ident("a".into()),
                TokenKind::Union,
                TokenKind::Ident("b".into()),
                TokenKind::Intersect,
                TokenKind::Ident("c".into()),
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn backslash_before_operator_is_set_minus() {
        assert_eq!(
            kinds("s \\ ∩ {1}"),
            vec![
                TokenKind::Ident("s".into()),
                TokenKind::SetMinus,
                TokenKind::Intersect,
                TokenKind::LBrace,
                TokenKind::Integer("1".into()),
                TokenKind::RBrace,
                TokenKind::Eof,
            ]
        );
    }
}
