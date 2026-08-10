use crate::ast::{BinOp, Expr, ExprKind, Literal, Spanned, UnOp};
use crate::error::SyntaxError;
use crate::span::Span;

pub fn parse_tex(src: &str) -> Result<Expr, SyntaxError> {
    TexParser { chars: src.chars().collect(), pos: 0 }.parse_expr()
}

struct TexParser {
    chars: Vec<char>,
    pos: usize,
}

impl TexParser {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek();
        if c.is_some() {
            self.pos += 1;
        }
        c
    }

    fn skip_ws(&mut self) {
        while self.peek() == Some(' ') {
            self.pos += 1;
        }
    }

    fn span(&self) -> Span {
        Span::new(self.pos as u32, self.pos as u32)
    }

    fn err(&self, message: &str) -> SyntaxError {
        SyntaxError { span: self.span(), message: message.to_string() }
    }

    fn binary(&self, op: BinOp, lhs: Expr, rhs: Expr) -> Expr {
        let span = lhs.span.merge(rhs.span);
        Expr { kind: ExprKind::Binary { op, lhs: Box::new(lhs), rhs: Box::new(rhs) }, span }
    }

    fn unary(&self, op: UnOp, e: Expr) -> Expr {
        let span = e.span;
        Expr { kind: ExprKind::Unary { op, operand: Box::new(e) }, span }
    }

    fn call(&self, name: String, args: Vec<Expr>) -> Expr {
        let span = self.span();
        let path = Expr { kind: ExprKind::Path { segments: vec![Spanned { value: name, span }] }, span };
        Expr { kind: ExprKind::Call { callee: Box::new(path), args }, span }
    }

    fn parse_expr(&mut self) -> Result<Expr, SyntaxError> {
        self.skip_ws();
        let mut lhs = self.parse_term()?;
        loop {
            self.skip_ws();
            match self.peek() {
                Some('+') => {
                    self.bump();
                    let rhs = self.parse_term()?;
                    lhs = self.binary(BinOp::Add, lhs, rhs);
                }
                Some('-') => {
                    self.bump();
                    let rhs = self.parse_term()?;
                    lhs = self.binary(BinOp::Sub, lhs, rhs);
                }
                _ => break,
            }
        }
        Ok(lhs)
    }

    fn starts_factor(&self) -> bool {
        matches!(self.peek(), Some(c) if c.is_ascii_digit() || c.is_ascii_alphabetic() || c == '\\' || c == '{' || c == '(')
    }

    fn parse_term(&mut self) -> Result<Expr, SyntaxError> {
        self.skip_ws();
        let neg = self.peek() == Some('-');
        if neg {
            self.bump();
        }
        let mut factors = Vec::new();
        loop {
            self.skip_ws();
            if self.starts_factor() {
                factors.push(self.parse_factor()?);
            } else {
                break;
            }
        }
        if factors.is_empty() {
            return Err(self.err("expected a TeX expression"));
        }
        let mut e = factors.remove(0);
        for f in factors {
            e = self.binary(BinOp::Mul, e, f);
        }
        if neg {
            e = self.unary(UnOp::Neg, e);
        }
        Ok(e)
    }

    fn parse_factor(&mut self) -> Result<Expr, SyntaxError> {
        self.skip_ws();
        let base = self.parse_atom()?;
        self.skip_ws();
        if self.peek() == Some('^') {
            self.bump();
            let sup = self.parse_group_or_atom()?;
            return Ok(self.binary(BinOp::Pow, base, sup));
        }
        if self.peek() == Some('_') {
            self.bump();
            self.parse_group_or_atom()?;
        }
        Ok(base)
    }

    fn parse_group_or_atom(&mut self) -> Result<Expr, SyntaxError> {
        self.skip_ws();
        if self.peek() == Some('{') {
            self.parse_group()
        } else {
            self.parse_atom()
        }
    }

    fn parse_group(&mut self) -> Result<Expr, SyntaxError> {
        if self.bump() != Some('{') {
            return Err(self.err("expected `{`"));
        }
        let e = self.parse_expr()?;
        self.skip_ws();
        if self.bump() != Some('}') {
            return Err(self.err("expected `}`"));
        }
        Ok(e)
    }

    fn parse_paren(&mut self) -> Result<Expr, SyntaxError> {
        if self.bump() != Some('(') {
            return Err(self.err("expected `(`"));
        }
        let e = self.parse_expr()?;
        self.skip_ws();
        if self.bump() != Some(')') {
            return Err(self.err("expected `)`"));
        }
        Ok(e)
    }

    fn parse_atom(&mut self) -> Result<Expr, SyntaxError> {
        self.skip_ws();
        let c = self.peek().ok_or_else(|| self.err("expected an expression"))?;
        if c.is_ascii_digit() {
            let mut s = String::new();
            while let Some(d) = self.peek() {
                if d.is_ascii_digit() {
                    s.push(d);
                    self.pos += 1;
                } else {
                    break;
                }
            }
            Ok(Expr { kind: ExprKind::Literal(Literal::Integer(s)), span: self.span() })
        } else if c.is_ascii_alphabetic() {
            let mut s = String::new();
            while let Some(l) = self.peek() {
                if l.is_ascii_alphabetic() {
                    s.push(l);
                    self.pos += 1;
                } else {
                    break;
                }
            }
            Ok(Expr { kind: ExprKind::Symbol(Spanned { value: s, span: self.span() }), span: self.span() })
        } else if c == '\\' {
            self.bump();
            let mut name = String::new();
            while let Some(l) = self.peek() {
                if l.is_ascii_alphabetic() || l == '_' {
                    name.push(l);
                    self.pos += 1;
                } else {
                    break;
                }
            }
            if name.is_empty() {
                return Err(self.err("invalid TeX command"));
            }
            self.skip_ws();
            if name == "frac" {
                let num = self.parse_group()?;
                let den = self.parse_group()?;
                return Ok(self.binary(BinOp::Div, num, den));
            }
            if self.peek() == Some('{') {
                let arg = self.parse_group()?;
                return Ok(self.call(name, vec![arg]));
            }
            if self.peek() == Some('(') {
                let arg = self.parse_paren()?;
                return Ok(self.call(name, vec![arg]));
            }
            Ok(Expr { kind: ExprKind::Symbol(Spanned { value: name, span: self.span() }), span: self.span() })
        } else if c == '{' {
            self.parse_group()
        } else if c == '(' {
            self.parse_paren()
        } else {
            Err(self.err("unexpected character in TeX expression"))
        }
    }
}
