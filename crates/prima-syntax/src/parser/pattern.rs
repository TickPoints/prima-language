//! Pattern and destructuring parsing (spec §4.4).
//!
//! This module owns the full pattern grammar used by `let`, `match` arms, and `if let`/`while let`: wildcards, bindings, literals, range patterns, or-patterns, and tuple/array/struct/constructor forms.

use super::Parser;
use crate::ast::*;
use crate::error::SyntaxError;
use crate::span::Span;
use crate::token::{TokenKind, describe};

impl Parser {
    /// Full pattern grammar (spec §4.4): `_`, bindings, literals, tuple/array/struct/constructor patterns,
    /// range patterns, or-patterns, and grouping.
    pub(crate) fn parse_pattern(&mut self) -> Result<Pattern, SyntaxError> {
        self.skip_newlines();
        let first = self.parse_pattern_simple()?;
        self.skip_newlines();
        // Or-pattern: `pat1 | pat2` (spec §4.4).
        if self.at(&TokenKind::Pipe) {
            let mut pats = vec![first];
            while self.eat(&TokenKind::Pipe).is_some() {
                self.skip_newlines();
                pats.push(self.parse_pattern_simple()?);
                self.skip_newlines();
            }
            return Ok(Pattern::Or(pats));
        }
        Ok(first)
    }

    pub(crate) fn parse_pattern_simple(&mut self) -> Result<Pattern, SyntaxError> {
        self.skip_newlines();
        // Leading `-` for negative literal patterns.
        if self.at(&TokenKind::Minus) {
            let m = self.bump();
            self.skip_newlines();
            let t = self.bump();
            let lit = match t.kind {
                TokenKind::Integer(s) => Literal::Integer(format!("-{s}")),
                TokenKind::Float(s) => Literal::Float(format!("-{s}")),
                _ => {
                    return Err(self.err(
                        t.span,
                        "expected a numeric literal after `-` in a pattern".into(),
                    ));
                }
            };
            let _ = m;
            return Ok(Pattern::Literal(lit));
        }
        let tok = self.bump();
        match tok.kind {
            TokenKind::Underscore => Ok(Pattern::Wildcard(tok.span)),
            TokenKind::Integer(s) => {
                self.parse_pattern_range(Pattern::Literal(Literal::Integer(s)), tok.span)
            }
            TokenKind::Float(s) => {
                self.parse_pattern_range(Pattern::Literal(Literal::Float(s)), tok.span)
            }
            TokenKind::Str { value, raw, single } => Ok(Pattern::Literal(Literal::String {
                value,
                quote: if single {
                    StringQuote::Single
                } else {
                    StringQuote::Double
                },
                raw,
            })),
            TokenKind::Char(c) => {
                self.parse_pattern_range(Pattern::Literal(Literal::Char(c)), tok.span)
            }
            TokenKind::KwTrue => Ok(Pattern::Literal(Literal::Bool(true))),
            TokenKind::KwFalse => Ok(Pattern::Literal(Literal::Bool(false))),
            TokenKind::Symbol(s) => Ok(Pattern::Binding(Spanned {
                value: s,
                span: tok.span,
            })),
            TokenKind::LParen => self.parse_tuple_pattern(tok.span),
            TokenKind::LBracket => self.parse_array_pattern(tok.span),
            TokenKind::Ident(s) => {
                // `Some(x)`/`Ok(v)` constructor pattern or `Type { ... }` struct pattern (spec §4.4).
                let name = Spanned {
                    value: s,
                    span: tok.span,
                };
                self.skip_newlines();
                if self.at(&TokenKind::LParen) {
                    self.bump();
                    let mut args = Vec::new();
                    self.skip_newlines();
                    if !self.at(&TokenKind::RParen) {
                        loop {
                            args.push(self.parse_pattern()?);
                            self.skip_newlines();
                            if self.eat(&TokenKind::Comma).is_some() {
                                self.skip_newlines();
                                continue;
                            }
                            break;
                        }
                    }
                    let end = self.expect(&TokenKind::RParen, "`)`")?.span;
                    return Ok(Pattern::Variant {
                        name,
                        args,
                        span: Span::merge(tok.span, end),
                    });
                }
                if self.at(&TokenKind::LBrace) {
                    self.bump();
                    let mut fields = Vec::new();
                    let mut rest = false;
                    loop {
                        self.skip_newlines();
                        if self.at(&TokenKind::RBrace) {
                            self.bump();
                            break;
                        }
                        if self.eat(&TokenKind::DotDot).is_some() {
                            rest = true;
                            self.skip_newlines();
                            if self.eat(&TokenKind::RBrace).is_none() {
                                return Err(self.err(
                                    self.span(),
                                    "expected `}` after `..` in a struct pattern".into(),
                                ));
                            }
                            break;
                        }
                        let fname = self.parse_ident("field name")?;
                        self.skip_newlines();
                        let pat = if self.eat(&TokenKind::Colon).is_some() {
                            self.skip_newlines();
                            Some(self.parse_pattern()?)
                        } else {
                            None
                        };
                        fields.push(FieldPattern { name: fname, pat });
                        self.skip_newlines();
                        if self.eat(&TokenKind::Comma).is_some() {
                            continue;
                        }
                        self.expect(&TokenKind::RBrace, "`}`")?;
                        break;
                    }
                    return Ok(Pattern::Struct { name, fields, rest });
                }
                Ok(Pattern::Binding(name))
            }
            _ => Err(self.err(
                tok.span,
                format!("expected pattern, found {}", describe(&tok.kind)),
            )),
        }
    }

    /// Parse a range pattern continuation after a numeric/char start literal: `0..9` / `1..=5`.
    pub(crate) fn parse_pattern_range(
        &mut self,
        start: Pattern,
        span: Span,
    ) -> Result<Pattern, SyntaxError> {
        self.skip_newlines();
        if self.at(&TokenKind::DotDot) || self.at(&TokenKind::DotDotEq) {
            let inclusive = self.at(&TokenKind::DotDotEq);
            self.bump();
            self.skip_newlines();
            let t = self.bump();
            let hi = match t.kind {
                TokenKind::Integer(s) => Literal::Integer(s),
                TokenKind::Float(s) => Literal::Float(s),
                TokenKind::Char(c) => Literal::Char(c),
                _ => return Err(self.err(t.span, "expected a literal range end".into())),
            };
            let lo = match start {
                Pattern::Literal(l) => l,
                _ => unreachable!(),
            };
            let _ = span;
            return Ok(Pattern::Range { lo, hi, inclusive });
        }
        Ok(start)
    }

    pub(crate) fn parse_tuple_pattern(&mut self, start: Span) -> Result<Pattern, SyntaxError> {
        let mut pats = Vec::new();
        let mut rest = false;
        let mut closed = false;
        self.skip_newlines();
        if !self.at(&TokenKind::RParen) {
            loop {
                self.skip_newlines();
                if self.eat(&TokenKind::DotDot).is_some() {
                    rest = true;
                    self.skip_newlines();
                    if self.eat(&TokenKind::RParen).is_none() {
                        return Err(self.err(
                            self.span(),
                            "expected `)` after `..` in a tuple pattern".into(),
                        ));
                    }
                    closed = true;
                    break;
                }
                pats.push(self.parse_pattern()?);
                self.skip_newlines();
                if self.eat(&TokenKind::Comma).is_some() {
                    continue;
                }
                break;
            }
        }
        if !closed {
            self.expect(&TokenKind::RParen, "`)`")?;
        }
        let _ = start;
        Ok(Pattern::Tuple(pats, rest))
    }

    pub(crate) fn parse_array_pattern(&mut self, start: Span) -> Result<Pattern, SyntaxError> {
        let mut pats = Vec::new();
        let mut rest = false;
        let mut closed = false;
        self.skip_newlines();
        if !self.at(&TokenKind::RBracket) {
            loop {
                self.skip_newlines();
                if self.eat(&TokenKind::DotDot).is_some() {
                    rest = true;
                    self.skip_newlines();
                    if self.eat(&TokenKind::RBracket).is_none() {
                        return Err(self.err(
                            self.span(),
                            "expected `]` after `..` in an array pattern".into(),
                        ));
                    }
                    closed = true;
                    break;
                }
                pats.push(self.parse_pattern()?);
                self.skip_newlines();
                if self.eat(&TokenKind::Comma).is_some() {
                    continue;
                }
                break;
            }
        }
        if !closed {
            self.expect(&TokenKind::RBracket, "`]`")?;
        }
        let _ = start;
        Ok(Pattern::Array(pats, rest))
    }
}
