//! Expression-level parsing (Pratt climbing, spec §2.2).
//!
//! This module owns the prefix/atom/postfix expression parsers, the precedence-climbing core, and the collection/comprehension/lambda/match expression forms; statements live in the sibling `stmt` module.

use super::{Parser, UNARY_BP, binop_bp};
use crate::ast::*;
use crate::error::SyntaxError;
use crate::span::Span;
use crate::token::{Token, TokenKind, describe};

impl Parser {
    /// `match <expr> {` — parse the scrutinee with struct literals disabled so `match x { ... }` treats `{` as the arms block.
    pub(crate) fn parse_scrutinee(&mut self) -> Result<Expr, SyntaxError> {
        let prev = self.no_struct_literal;
        self.no_struct_literal = true;
        let r = self.parse_expr();
        self.no_struct_literal = prev;
        r
    }

    pub(crate) fn parse_expr(&mut self) -> Result<Expr, SyntaxError> {
        self.parse_expr_bp(0)
    }

    // Pratt climbing (implementation plan §2.2 precedence table): `^`/`**` highest and right-associative.
    pub(crate) fn parse_expr_bp(&mut self, min_bp: u8) -> Result<Expr, SyntaxError> {
        let mut lhs = self.parse_prefix()?;
        loop {
            self.skip_newlines();
            // `|>` was removed in v2.3 (spec §9.7); report it at the point of use instead of a generic `expected expression`.
            if self.at(&TokenKind::PipeArrow) {
                let span = self.span();
                self.skip_to_statement_boundary();
                return Err(self.err(
                    span,
                    "`|>` pipeline was removed in v2.3 (E0010); use class methods or a direct function call (spec §9.7)".into(),
                ));
            }
            let Some((op, lbp, rbp)) = binop_bp(self.peek()) else {
                break;
            };
            if lbp < min_bp {
                break;
            }
            self.bump();
            let rhs = self.parse_expr_bp(rbp)?;
            let span = Span::merge(lhs.span, rhs.span);
            lhs = Expr {
                kind: ExprKind::Binary {
                    op,
                    lhs: Box::new(lhs),
                    rhs: Box::new(rhs),
                },
                span,
            };
        }
        Ok(lhs)
    }

    pub(crate) fn parse_prefix(&mut self) -> Result<Expr, SyntaxError> {
        self.skip_newlines();
        let tok = self.bump();
        match tok.kind {
            TokenKind::Minus => {
                let operand = self.parse_expr_bp(UNARY_BP)?;
                let span = Span::merge(tok.span, operand.span);
                Ok(Expr {
                    kind: ExprKind::Unary {
                        op: UnOp::Neg,
                        operand: Box::new(operand),
                    },
                    span,
                })
            }
            TokenKind::Bang => {
                let operand = self.parse_expr_bp(UNARY_BP)?;
                let span = Span::merge(tok.span, operand.span);
                Ok(Expr {
                    kind: ExprKind::Unary {
                        op: UnOp::Not,
                        operand: Box::new(operand),
                    },
                    span,
                })
            }
            TokenKind::Plus => {
                let operand = self.parse_expr_bp(UNARY_BP)?;
                let span = Span::merge(tok.span, operand.span);
                Ok(Expr {
                    kind: ExprKind::Unary {
                        op: UnOp::Pos,
                        operand: Box::new(operand),
                    },
                    span,
                })
            }
            TokenKind::KwSelf => {
                let e = Expr {
                    kind: ExprKind::Self_,
                    span: tok.span,
                };
                self.parse_postfix(e)
            }
            _ => {
                let atom = self.parse_atom(tok)?;
                self.parse_postfix(atom)
            }
        }
    }

    pub(crate) fn parse_atom(&mut self, tok: Token) -> Result<Expr, SyntaxError> {
        let span = tok.span;
        let kind = match tok.kind {
            TokenKind::Integer(s) => ExprKind::Literal(Literal::Integer(s)),
            TokenKind::Float(s) => ExprKind::Literal(Literal::Float(s)),
            TokenKind::Hex(s) => ExprKind::Literal(Literal::Hex(s)),
            TokenKind::Binary(s) => ExprKind::Literal(Literal::Binary(s)),
            TokenKind::Str { value, raw, single } => ExprKind::Literal(Literal::String {
                value,
                quote: if single {
                    StringQuote::Single
                } else {
                    StringQuote::Double
                },
                raw,
            }),
            TokenKind::Char(c) => ExprKind::Literal(Literal::Char(c)),
            TokenKind::TexStr(s) => ExprKind::Literal(Literal::Tex(s)),
            TokenKind::FStr(parts) => {
                let mut out = Vec::with_capacity(parts.len());
                for p in parts {
                    match p {
                        crate::token::FStringToken::Lit(s) => out.push(FStringPart::Literal(s)),
                        crate::token::FStringToken::Interp { expr, spec } => {
                            let e = self.parse_fstring_interp(&expr, span)?;
                            out.push(FStringPart::Interp {
                                expr: Box::new(e),
                                spec,
                            });
                        }
                    }
                }
                ExprKind::FString(out)
            }
            TokenKind::KwTrue => ExprKind::Literal(Literal::Bool(true)),
            TokenKind::KwFalse => ExprKind::Literal(Literal::Bool(false)),
            TokenKind::Ident(s) => {
                let mut segments = vec![Spanned { value: s, span }];
                while self.eat(&TokenKind::ColonColon).is_some() {
                    let seg = self.parse_module_segment()?;
                    segments.push(seg);
                }
                ExprKind::Path { segments }
            }
            TokenKind::Symbol(s) => ExprKind::Symbol(Spanned { value: s, span }),
            TokenKind::LParen => return self.parse_paren_or_tuple(span),
            TokenKind::LBracket => return self.parse_array(span),
            // A `{` at the start of an expression is always a Dict/Set literal (spec §4.6 rule 3);
            // struct literals `T { ... }` and block-context braces are consumed by their dedicated parsers.
            TokenKind::LBrace => return self.parse_brace_literal(span),
            TokenKind::Pipe => return self.parse_lambda(span),
            TokenKind::KwMatch => return self.parse_match_expr(span),
            _ => {
                return Err(SyntaxError {
                    span,
                    message: format!("expected expression, found {}", describe(&tok.kind)),
                });
            }
        };
        Ok(Expr { kind, span })
    }

    /// Parse the already-lexed body of an f-string interpolation into an `Expr` (spec §18.1).
    /// The body is a normal Prima expression; a leftover token is a parse error.
    pub(crate) fn parse_fstring_interp(
        &mut self,
        tokens: &[Token],
        fstring_span: Span,
    ) -> Result<Expr, SyntaxError> {
        let mut sub = Parser::new(tokens.to_vec());
        let e = sub.parse_expr()?;
        sub.skip_newlines();
        if !sub.at(&TokenKind::Eof) {
            return Err(self.err(
                fstring_span,
                "invalid expression in f-string interpolation".into(),
            ));
        }
        self.warnings.extend(sub.warnings);
        Ok(e)
    }

    pub(crate) fn parse_postfix(&mut self, mut e: Expr) -> Result<Expr, SyntaxError> {
        loop {
            self.skip_newlines();
            match self.peek().clone() {
                TokenKind::LParen => {
                    self.bump();
                    let args = self.parse_args()?;
                    let end = self.tokens[self.pos.saturating_sub(1)].span;
                    let span = Span::merge(e.span, end);
                    // The removed `format` function (spec §18.1): a bare `format(...)` call gets the
                    // `W0006` deprecation hint. Module functions (`time::format`) are untouched.
                    if let ExprKind::Path { segments } = &e.kind
                        && segments.len() == 1
                        && segments[0].value == "format"
                    {
                        self.push_warning(
                            "W0006",
                            span,
                            "`format` was removed in v2.2 (W0006); use an f-string `f\"...{expr}...\"` instead (spec §18.1)".into(),
                        );
                    }
                    e = Expr {
                        kind: ExprKind::Call {
                            callee: Box::new(e),
                            args,
                        },
                        span,
                    };
                }
                TokenKind::LBracket => {
                    self.bump();
                    let index = self.parse_index()?;
                    let end = self.tokens[self.pos.saturating_sub(1)].span;
                    let span = Span::merge(e.span, end);
                    e = Expr {
                        kind: ExprKind::Index {
                            base: Box::new(e),
                            index,
                        },
                        span,
                    };
                }
                TokenKind::Dot => {
                    // Method call `obj.method(...)` / field access `obj.field` (spec §4.5).
                    self.bump();
                    self.skip_newlines();
                    let name = self.parse_ident("field or method name")?;
                    self.skip_newlines();
                    if self.at(&TokenKind::LParen) {
                        self.bump();
                        let args = self.parse_args()?;
                        let end = self.tokens[self.pos.saturating_sub(1)].span;
                        let span = Span::merge(e.span, end);
                        e = Expr {
                            kind: ExprKind::MethodCall {
                                receiver: Box::new(e),
                                name,
                                args,
                            },
                            span,
                        };
                    } else {
                        let span = Span::merge(e.span, name.span);
                        e = Expr {
                            kind: ExprKind::Field {
                                receiver: Box::new(e),
                                name,
                            },
                            span,
                        };
                    }
                }
                TokenKind::Question => {
                    // `expr?` try operator (spec §16.3).
                    let q = self.bump();
                    let span = Span::merge(e.span, q.span);
                    e = Expr {
                        kind: ExprKind::Try(Box::new(e)),
                        span,
                    };
                }
                _ => break,
            }
        }
        // Struct literal `T { a, b, ..base }` (spec §4.5): a single-segment path followed by `{` in expression position.
        // Disabled in control-flow conditions (`if x {`, `match x {`) so `{` opens the body/arms block.
        if !self.no_struct_literal
            && let ExprKind::Path { segments } = &e.kind
            && segments.len() == 1
            && self.at(&TokenKind::LBrace)
        {
            return self.parse_struct_literal(segments[0].clone(), e);
        }
        Ok(e)
    }

    /// `T { a, b, ..base }` struct literal (spec §4.5); field shorthand `a` ≡ `a: a`.
    pub(crate) fn parse_struct_literal(
        &mut self,
        name: Spanned<String>,
        path: Expr,
    ) -> Result<Expr, SyntaxError> {
        self.expect(&TokenKind::LBrace, "`{`")?;
        let mut fields = Vec::new();
        let mut base = None;
        loop {
            self.skip_newlines();
            if self.at(&TokenKind::RBrace) {
                self.bump();
                break;
            }
            // Struct update syntax `..base` (copies remaining fields from an existing instance).
            if self.eat(&TokenKind::DotDot).is_some() {
                self.skip_newlines();
                base = Some(Box::new(self.parse_expr()?));
                self.skip_newlines();
                if self.eat(&TokenKind::RBrace).is_none() {
                    return Err(self.err(
                        self.span(),
                        "expected `}` after the struct update base".into(),
                    ));
                }
                break;
            }
            let fname = self.parse_ident("field name")?;
            self.skip_newlines();
            let value = if self.eat(&TokenKind::Colon).is_some() {
                self.skip_newlines();
                Some(self.parse_expr()?)
            } else {
                None
            };
            fields.push(FieldValue { name: fname, value });
            self.skip_newlines();
            if self.eat(&TokenKind::Comma).is_some() {
                self.skip_newlines();
                continue;
            }
            self.expect(&TokenKind::RBrace, "`}`")?;
            break;
        }
        let end = self.tokens[self.pos.saturating_sub(1)].span;
        let span = Span::merge(path.span, end);
        Ok(Expr {
            kind: ExprKind::StructLiteral { name, fields, base },
            span,
        })
    }

    pub(crate) fn parse_args(&mut self) -> Result<Vec<Expr>, SyntaxError> {
        let mut args = Vec::new();
        self.skip_newlines();
        if !self.at(&TokenKind::RParen) {
            loop {
                args.push(self.parse_expr()?);
                self.skip_newlines();
                if self.eat(&TokenKind::Comma).is_some() {
                    self.skip_newlines();
                    continue;
                }
                break;
            }
        }
        self.expect(&TokenKind::RParen, "`)`")?;
        Ok(args)
    }

    pub(crate) fn parse_paren_or_tuple(&mut self, start: Span) -> Result<Expr, SyntaxError> {
        self.skip_newlines();
        if self.at(&TokenKind::RParen) {
            let end = self.bump().span;
            return Ok(Expr {
                kind: ExprKind::Tuple(vec![]),
                span: Span::merge(start, end),
            });
        }
        let first = self.parse_expr()?;
        self.skip_newlines();
        if self.eat(&TokenKind::Comma).is_some() {
            let mut items = vec![first];
            self.skip_newlines();
            if !self.at(&TokenKind::RParen) {
                loop {
                    items.push(self.parse_expr()?);
                    self.skip_newlines();
                    if self.eat(&TokenKind::Comma).is_some() {
                        self.skip_newlines();
                        continue;
                    }
                    break;
                }
            }
            let end = self.expect(&TokenKind::RParen, "`)`")?.span;
            Ok(Expr {
                kind: ExprKind::Tuple(items),
                span: Span::merge(start, end),
            })
        } else {
            // Tuple comprehension `(output for var in iter [if cond])` (spec §4.6): a single output with no trailing comma.
            if self.at(&TokenKind::KwFor) {
                let clauses = self.parse_comprehension_clauses()?;
                let end = self.expect(&TokenKind::RParen, "`)`")?.span;
                return Ok(Expr {
                    kind: ExprKind::Comprehension {
                        kind: CompKind::Tuple,
                        output: Box::new(first),
                        clauses,
                    },
                    span: Span::merge(start, end),
                });
            }
            let end = self.expect(&TokenKind::RParen, "`)`")?.span;
            Ok(Expr {
                kind: first.kind,
                span: Span::merge(start, end),
            })
        }
    }

    pub(crate) fn parse_array(&mut self, start: Span) -> Result<Expr, SyntaxError> {
        let mut items = Vec::new();
        self.skip_newlines();
        if !self.at(&TokenKind::RBracket) {
            loop {
                items.push(self.parse_expr()?);
                self.skip_newlines();
                if self.eat(&TokenKind::Comma).is_some() {
                    self.skip_newlines();
                    continue;
                }
                break;
            }
        }
        // Array comprehension `[output for var in iter [if cond]]` (spec §4.6/§11.7): a single output expression.
        if self.at(&TokenKind::KwFor) {
            if items.len() != 1 {
                return Err(self.err(
                    self.span(),
                    "comprehension output must be a single expression".into(),
                ));
            }
            let output = items.pop().unwrap();
            let clauses = self.parse_comprehension_clauses()?;
            let end = self.expect(&TokenKind::RBracket, "`]`")?.span;
            return Ok(Expr {
                kind: ExprKind::Comprehension {
                    kind: CompKind::Array,
                    output: Box::new(output),
                    clauses,
                },
                span: Span::merge(start, end),
            });
        }
        let end = self.expect(&TokenKind::RBracket, "`]`")?.span;
        Ok(Expr {
            kind: ExprKind::Array(items),
            span: Span::merge(start, end),
        })
    }

    /// `{ ... }` dict/set literal or comprehension (spec §4.6): `{}` is an empty Dict; a trailing `for` after the
    /// first item/entry makes it a comprehension; `{ k: v }` is a Dict, `{ a, b }` is a Set.
    pub(crate) fn parse_brace_literal(&mut self, start: Span) -> Result<Expr, SyntaxError> {
        self.skip_newlines();
        if self.at(&TokenKind::RBrace) {
            let end = self.bump().span;
            return Ok(Expr {
                kind: ExprKind::Dict(vec![]),
                span: Span::merge(start, end),
            });
        }
        let first = self.parse_expr()?;
        self.skip_newlines();
        if self.at(&TokenKind::KwFor) {
            // Set comprehension `{ output for var in iter [if cond] }` (spec §4.6).
            let clauses = self.parse_comprehension_clauses()?;
            let end = self.expect(&TokenKind::RBrace, "`}`")?.span;
            return Ok(Expr {
                kind: ExprKind::Comprehension {
                    kind: CompKind::Set,
                    output: Box::new(first),
                    clauses,
                },
                span: Span::merge(start, end),
            });
        }
        if self.eat(&TokenKind::Colon).is_some() {
            // Dict: the first expression is a key; parse its value, then `key : value` entries.
            let value = self.parse_expr()?;
            self.skip_newlines();
            if self.at(&TokenKind::KwFor) {
                // Dict comprehension `{ key: value for var in iter [if cond] }` (spec §4.6).
                let kv_span = Span::merge(first.span, value.span);
                let output = Expr {
                    kind: ExprKind::KeyValue {
                        key: Box::new(first),
                        value: Box::new(value),
                    },
                    span: kv_span,
                };
                let clauses = self.parse_comprehension_clauses()?;
                let end = self.expect(&TokenKind::RBrace, "`}`")?.span;
                return Ok(Expr {
                    kind: ExprKind::Comprehension {
                        kind: CompKind::Dict,
                        output: Box::new(output),
                        clauses,
                    },
                    span: Span::merge(start, end),
                });
            }
            let mut entries = vec![(first, value)];
            loop {
                self.skip_newlines();
                if self.at(&TokenKind::RBrace) {
                    self.bump();
                    break;
                }
                self.expect(&TokenKind::Comma, "`,` or `}`")?;
                self.skip_newlines();
                let key = self.parse_expr()?;
                self.skip_newlines();
                if self.eat(&TokenKind::Colon).is_none() {
                    return Err(
                        self.err(self.span(), "expected `:` in a Dict literal entry".into())
                    );
                }
                let value = self.parse_expr()?;
                entries.push((key, value));
            }
            let end = self.tokens[self.pos.saturating_sub(1)].span;
            return Ok(Expr {
                kind: ExprKind::Dict(entries),
                span: Span::merge(start, end),
            });
        }
        // Set literal: comma-separated elements; a top-level `:` would mean a Dict entry, which is invalid here.
        let mut elems = vec![first];
        loop {
            self.skip_newlines();
            if self.at(&TokenKind::RBrace) {
                self.bump();
                break;
            }
            if self.at(&TokenKind::Colon) {
                return Err(self.err(
                    self.span(),
                    "expected `}` or `,` in a Set literal; `key: value` requires a Dict literal"
                        .into(),
                ));
            }
            self.expect(&TokenKind::Comma, "`,` or `}`")?;
            self.skip_newlines();
            let elem = self.parse_expr()?;
            self.skip_newlines();
            if self.at(&TokenKind::Colon) {
                return Err(self.err(
                    self.span(),
                    "expected `}` or `,` in a Set literal; `key: value` requires a Dict literal"
                        .into(),
                ));
            }
            elems.push(elem);
        }
        let end = self.tokens[self.pos.saturating_sub(1)].span;
        Ok(Expr {
            kind: ExprKind::Set(elems),
            span: Span::merge(start, end),
        })
    }

    /// Comprehension clauses after the output: any sequence of `for <var> in <iter>` / `if <cond>` (spec §11.7).
    pub(crate) fn parse_comprehension_clauses(
        &mut self,
    ) -> Result<Vec<ComprehensionClause>, SyntaxError> {
        let mut clauses = Vec::new();
        loop {
            self.skip_newlines();
            match self.peek().clone() {
                TokenKind::KwFor => {
                    self.bump();
                    self.skip_newlines();
                    let var = self.parse_ident("loop variable")?;
                    self.skip_newlines();
                    self.expect(&TokenKind::KwIn, "`in`")?;
                    self.skip_newlines();
                    let iter = self.parse_expr()?;
                    clauses.push(ComprehensionClause::For { var, iter });
                }
                TokenKind::KwIf => {
                    self.bump();
                    self.skip_newlines();
                    let cond = self.parse_expr()?;
                    clauses.push(ComprehensionClause::If { cond });
                }
                _ => break,
            }
        }
        Ok(clauses)
    }

    pub(crate) fn parse_index(&mut self) -> Result<Index, SyntaxError> {
        let mut items = Vec::new();
        loop {
            self.skip_newlines();
            let item = if self.at(&TokenKind::DotDot) {
                self.bump();
                let end = if self.at(&TokenKind::Comma) || self.at(&TokenKind::RBracket) {
                    None
                } else {
                    Some(self.parse_expr()?)
                };
                IndexItem::Slice { start: None, end }
            } else {
                let start = self.parse_expr()?;
                if self.at(&TokenKind::DotDot) {
                    self.bump();
                    let end = if self.at(&TokenKind::Comma) || self.at(&TokenKind::RBracket) {
                        None
                    } else {
                        Some(self.parse_expr()?)
                    };
                    IndexItem::Slice {
                        start: Some(start),
                        end,
                    }
                } else {
                    IndexItem::Elem(start)
                }
            };
            items.push(item);
            self.skip_newlines();
            if self.eat(&TokenKind::Comma).is_some() {
                self.skip_newlines();
                continue;
            }
            break;
        }
        self.expect(&TokenKind::RBracket, "`]`")?;
        Ok(Index { items })
    }

    pub(crate) fn parse_lambda(&mut self, start: Span) -> Result<Expr, SyntaxError> {
        let mut params = Vec::new();
        self.skip_newlines();
        if !self.at(&TokenKind::Pipe) {
            loop {
                let name = self.parse_ident("parameter name")?;
                self.skip_newlines();
                let type_ann = if self.eat(&TokenKind::Colon).is_some() {
                    Some(self.parse_type()?)
                } else {
                    None
                };
                params.push(Param {
                    name,
                    type_ann,
                    is_self: false,
                });
                self.skip_newlines();
                if self.eat(&TokenKind::Comma).is_some() {
                    self.skip_newlines();
                    continue;
                }
                break;
            }
        }
        self.skip_newlines();
        self.expect(&TokenKind::Pipe, "`|`")?;
        self.skip_newlines();
        let body = self.parse_expr()?;
        let span = Span::merge(start, body.span);
        Ok(Expr {
            kind: ExprKind::Lambda {
                params,
                body: Box::new(body),
            },
            span,
        })
    }

    pub(crate) fn parse_match_expr(&mut self, start: Span) -> Result<Expr, SyntaxError> {
        self.skip_newlines();
        let scrutinee = self.parse_scrutinee()?;
        let arms = self.parse_match_arms()?;
        let end = self.tokens[self.pos.saturating_sub(1)].span;
        Ok(Expr {
            kind: ExprKind::Match {
                scrutinee: Box::new(scrutinee),
                arms,
            },
            span: Span::merge(start, end),
        })
    }

    /// `{ pattern [if guard] => expr, ... }` (spec §4.4).
    pub(crate) fn parse_match_arms(&mut self) -> Result<Vec<MatchArm>, SyntaxError> {
        self.skip_newlines();
        self.expect(&TokenKind::LBrace, "`{`")?;
        let mut arms = Vec::new();
        loop {
            self.skip_newlines();
            if self.at(&TokenKind::RBrace) {
                self.bump();
                break;
            }
            let pattern = self.parse_pattern()?;
            self.skip_newlines();
            let guard = if self.eat(&TokenKind::KwIf).is_some() {
                self.skip_newlines();
                Some(self.parse_expr()?)
            } else {
                None
            };
            self.skip_newlines();
            self.expect(&TokenKind::FatArrow, "`=>`")?;
            self.skip_newlines();
            let body = self.parse_expr()?;
            arms.push(MatchArm {
                pattern,
                guard,
                body,
            });
            self.skip_newlines();
            self.eat(&TokenKind::Comma);
            self.eat(&TokenKind::Semicolon);
        }
        Ok(arms)
    }
}
