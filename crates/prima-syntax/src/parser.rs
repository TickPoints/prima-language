use crate::ast::*;
use crate::error::SyntaxError;
use crate::lexer::lex;
use crate::span::Span;
use crate::token::{describe, Token, TokenKind};

// Unary operator binding power: lower than power `^` (8), higher than mul/div (6/7), implementing `-x^2 == -(x^2)` (same as Julia, spec §2.2).
const UNARY_BP: u8 = 7;

/// Hand-written recursive-descent + Pratt climbing parser (implementation plan §2.2), covering all appendix A BNF productions.
pub fn parse(src: &str) -> Result<Program, Vec<SyntaxError>> {
    let tokens = lex(src)?;
    Parser::new(tokens).parse_program()
}

// Parser: errors use panic-mode recovery with sync tokens (`;`, `}`, `)`, end of file), collecting all syntax errors in one compilation (spec §2.2).
pub(crate) struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    pub(crate) fn new(tokens: Vec<Token>) -> Parser {
        Parser { tokens, pos: 0 }
    }

    fn peek(&self) -> &TokenKind {
        &self.tokens[self.pos].kind
    }

    fn peek_at(&self, n: usize) -> &TokenKind {
        self.tokens.get(self.pos + n).map(|t| &t.kind).unwrap_or(&TokenKind::Eof)
    }

    fn span(&self) -> Span {
        self.tokens[self.pos].span
    }

    fn bump(&mut self) -> Token {
        let t = self.tokens[self.pos].clone();
        if self.pos < self.tokens.len() - 1 {
            self.pos += 1;
        }
        t
    }

    fn skip_newlines(&mut self) {
        while matches!(self.peek(), TokenKind::Newline) {
            self.bump();
        }
    }

    fn at(&self, kind: &TokenKind) -> bool {
        self.peek() == kind
    }

    fn eat(&mut self, kind: &TokenKind) -> Option<Token> {
        if self.at(kind) {
            Some(self.bump())
        } else {
            None
        }
    }

    fn expect(&mut self, kind: &TokenKind, what: &str) -> Result<Token, SyntaxError> {
        if self.at(kind) {
            Ok(self.bump())
        } else {
            Err(SyntaxError { span: self.span(), message: format!("expected {what}, found {}", describe(self.peek())) })
        }
    }

    fn err(&self, span: Span, message: String) -> SyntaxError {
        SyntaxError { span, message }
    }

    fn parse_ident(&mut self, what: &str) -> Result<Spanned<String>, SyntaxError> {
        self.skip_newlines();
        let t = self.bump();
        match t.kind {
            TokenKind::Ident(s) => Ok(Spanned { value: s, span: t.span }),
            _ => Err(SyntaxError { span: t.span, message: format!("expected {what}, found {}", describe(&t.kind)) }),
        }
    }

    fn parse_module_segment(&mut self) -> Result<Spanned<String>, SyntaxError> {
        self.skip_newlines();
        let t = self.bump();
        match t.kind {
            TokenKind::Ident(s) | TokenKind::Symbol(s) => Ok(Spanned { value: s, span: t.span }),
            _ => Err(SyntaxError { span: t.span, message: format!("expected module path segment, found {}", describe(&t.kind)) }),
        }
    }

    fn parse_module_path(&mut self) -> Result<Vec<Spanned<String>>, SyntaxError> {
        let mut segs = vec![self.parse_module_segment()?];
        while self.eat(&TokenKind::ColonColon).is_some() {
            segs.push(self.parse_module_segment()?);
        }
        Ok(segs)
    }

    fn end_statement(&mut self) {
        match self.peek() {
            TokenKind::Newline | TokenKind::Semicolon => {
                self.bump();
            }
            _ => {}
        }
    }

    pub(crate) fn parse_program(&mut self) -> Result<Program, Vec<SyntaxError>> {
        match self.parse_program_inner() {
            Ok(p) => Ok(p),
            Err(e) => Err(vec![e]),
        }
    }

    fn parse_program_inner(&mut self) -> Result<Program, SyntaxError> {
        let mut config = None;
        let mut imports = Vec::new();
        let mut stmts = Vec::new();
        loop {
            self.skip_newlines();
            match self.peek().clone() {
                TokenKind::Eof => break,
                TokenKind::KwConfig => {
                    if config.is_some() {
                        return Err(self.err(self.span(), "duplicate `config` block".into()));
                    }
                    if !imports.is_empty() || !stmts.is_empty() {
                        return Err(self.err(self.span(), "`config` must appear before `import` and statements".into()));
                    }
                    config = Some(self.parse_config_block()?);
                }
                TokenKind::KwImport | TokenKind::KwFrom => {
                    if !stmts.is_empty() {
                        return Err(self.err(self.span(), "`import` must appear before statements".into()));
                    }
                    imports.push(self.parse_import()?);
                }
                _ => {
                    stmts.push(self.parse_stmt()?);
                }
            }
        }
        Ok(Program { config, imports, stmts })
    }

    fn parse_config_block(&mut self) -> Result<ConfigBlock, SyntaxError> {
        self.skip_newlines();
        let start = self.expect(&TokenKind::KwConfig, "`config`")?.span;
        self.skip_newlines();
        let entries = self.parse_config_entries()?;
        let end = self.tokens[self.pos.saturating_sub(1)].span;
        Ok(ConfigBlock { entries, span: Span::merge(start, end) })
    }

    fn parse_config_entries(&mut self) -> Result<Vec<ConfigEntry>, SyntaxError> {
        self.expect(&TokenKind::LBrace, "`{`")?;
        let mut entries = Vec::new();
        loop {
            self.skip_newlines();
            if self.at(&TokenKind::RBrace) {
                self.bump();
                break;
            }
            entries.push(self.parse_config_entry()?);
            self.skip_newlines();
            self.eat(&TokenKind::Comma);
        }
        Ok(entries)
    }

    // config entries accept both forms: `ident := value` (spec §4.1 examples) and `ident : type? = value` (appendix BNF).
    fn parse_config_entry(&mut self) -> Result<ConfigEntry, SyntaxError> {
        self.skip_newlines();
        let name = self.parse_ident("config key")?;
        let mut type_ann = None;
        if self.eat(&TokenKind::ColonEq).is_none() {
            if self.at(&TokenKind::Colon) {
                self.bump();
                if !self.at(&TokenKind::Eq) {
                    type_ann = Some(self.parse_type()?);
                }
                self.expect(&TokenKind::Eq, "`=`")?;
            } else {
                self.expect(&TokenKind::Eq, "`=` or `:=`")?;
            }
        }
        let value = self.parse_config_value()?;
        let span = Span::merge(name.span, value.span);
        Ok(ConfigEntry { name, type_ann, value, span })
    }

    fn parse_config_value(&mut self) -> Result<Expr, SyntaxError> {
        self.skip_newlines();
        let is_custom = matches!(self.peek(), TokenKind::Ident(s) if s.as_str() == "custom")
            && self.peek_at(1) == &TokenKind::LBrace;
        if is_custom {
            let start = self.bump().span;
            self.skip_newlines();
            self.expect(&TokenKind::LBrace, "`{`")?;
            let mut items = Vec::new();
            loop {
                self.skip_newlines();
                if self.at(&TokenKind::RBrace) {
                    self.bump();
                    break;
                }
                let pattern = self.parse_expr()?;
                self.skip_newlines();
                if self.eat(&TokenKind::ColonEq).is_none() {
                    self.expect(&TokenKind::Eq, "`:=`")?;
                }
                self.skip_newlines();
                let value = self.parse_expr()?;
                items.push((pattern, value));
                self.skip_newlines();
                self.eat(&TokenKind::Comma);
            }
            let end = self.tokens[self.pos.saturating_sub(1)].span;
            Ok(Expr { kind: ExprKind::Custom(items), span: Span::merge(start, end) })
        } else {
            self.parse_expr()
        }
    }

    fn parse_import(&mut self) -> Result<Import, SyntaxError> {
        self.skip_newlines();
        let start = self.bump().span;
        let start_kind = self.tokens[self.pos.saturating_sub(1)].kind.clone();
        let kind = match start_kind {
            TokenKind::KwImport => {
                let path = self.parse_module_path()?;
                let alias = if self.eat(&TokenKind::KwAs).is_some() {
                    Some(self.parse_ident("alias")?)
                } else {
                    None
                };
                ImportKind::Namespace { path, alias }
            }
            TokenKind::KwFrom => {
                let path = self.parse_module_path()?;
                self.skip_newlines();
                self.expect(&TokenKind::KwImport, "`import`")?;
                self.skip_newlines();
                let mut items = Vec::new();
                if self.at(&TokenKind::Star) {
                    self.bump();
                    items.push(ImportItem::Star);
                } else {
                    loop {
                        let name = self.parse_ident("name")?;
                        self.skip_newlines();
                        let alias = if self.eat(&TokenKind::KwAs).is_some() {
                            Some(self.parse_ident("alias")?)
                        } else {
                            None
                        };
                        items.push(ImportItem::Name { name, alias });
                        self.skip_newlines();
                        if self.eat(&TokenKind::Comma).is_some() {
                            self.skip_newlines();
                            continue;
                        }
                        break;
                    }
                }
                ImportKind::From { path, items }
            }
            _ => unreachable!(),
        };
        let end = self.tokens[self.pos.saturating_sub(1)].span;
        Ok(Import { kind, span: Span::merge(start, end) })
    }

    fn parse_stmt(&mut self) -> Result<Stmt, SyntaxError> {
        self.skip_newlines();
        match self.peek().clone() {
            TokenKind::KwLet => self.parse_let_stmt(),
            TokenKind::KwConst => self.parse_const_stmt(),
            TokenKind::KwFn => self.parse_fn_stmt(),
            TokenKind::KwFor => self.parse_for_stmt(false),
            TokenKind::KwParFor => self.parse_for_stmt(true),
            TokenKind::KwWhile => self.parse_while_stmt(),
            TokenKind::KwIf => self.parse_if_stmt(),
            TokenKind::KwReturn => self.parse_return_stmt(),
            TokenKind::KwTry => self.parse_try_stmt(),
            TokenKind::KwWith => self.parse_with_stmt(),
            TokenKind::KwPub => self.parse_pub_stmt(),
            _ => self.parse_expr_or_assign_stmt(),
        }
    }

    fn parse_let_stmt(&mut self) -> Result<Stmt, SyntaxError> {
        let start = self.bump().span;
        self.skip_newlines();
        let mut_ = self.eat(&TokenKind::KwMut).is_some();
        let name = self.parse_ident("variable name")?;
        self.skip_newlines();
        if self.at(&TokenKind::LParen) {
            let params = self.parse_params()?;
            self.skip_newlines();
            let ret = if self.eat(&TokenKind::Colon).is_some() {
                Some(self.parse_type()?)
            } else {
                None
            };
            let annotations = self.parse_annotations()?;
            self.expect(&TokenKind::Eq, "`=`")?;
            self.skip_newlines();
            let body = self.parse_expr()?;
            self.end_statement();
            let span = Span::merge(start, body.span);
            Ok(Stmt::MathDef { name, params, ret, annotations, body, span })
        } else {
            let type_ann = if self.eat(&TokenKind::Colon).is_some() {
                Some(self.parse_type()?)
            } else {
                None
            };
            self.expect(&TokenKind::Eq, "`=`")?;
            self.skip_newlines();
            let value = self.parse_expr()?;
            self.end_statement();
            let span = Span::merge(start, value.span);
            Ok(Stmt::Let { name, mut_, type_ann, value, span })
        }
    }

    fn parse_const_stmt(&mut self) -> Result<Stmt, SyntaxError> {
        let start = self.bump().span;
        self.skip_newlines();
        let name = self.parse_ident("constant name")?;
        self.skip_newlines();
        self.expect(&TokenKind::Colon, "`:`")?;
        let type_ann = self.parse_type()?;
        self.skip_newlines();
        self.expect(&TokenKind::Eq, "`=`")?;
        self.skip_newlines();
        let value = self.parse_expr()?;
        self.end_statement();
        let span = Span::merge(start, value.span);
        Ok(Stmt::Const { name, type_ann, value, span })
    }

    fn parse_fn_stmt(&mut self) -> Result<Stmt, SyntaxError> {
        let start = self.bump().span;
        self.skip_newlines();
        let name = self.parse_ident("function name")?;
        let params = self.parse_params()?;
        self.skip_newlines();
        let ret = if self.at(&TokenKind::Arrow) {
            self.bump();
            Some(self.parse_type()?)
        } else if self.eat(&TokenKind::Colon).is_some() {
            Some(self.parse_type()?)
        } else {
            None
        };
        let annotations = self.parse_annotations()?;
        let body = self.parse_block()?;
        let span = Span::merge(start, body.span);
        Ok(Stmt::FnDef { name, params, ret, annotations, body, span })
    }

    fn parse_params(&mut self) -> Result<Vec<Param>, SyntaxError> {
        self.expect(&TokenKind::LParen, "`(`")?;
        let mut params = Vec::new();
        self.skip_newlines();
        if !self.at(&TokenKind::RParen) {
            loop {
                let name = self.parse_ident("parameter name")?;
                self.skip_newlines();
                let type_ann = if self.eat(&TokenKind::Colon).is_some() {
                    Some(self.parse_type()?)
                } else {
                    None
                };
                params.push(Param { name, type_ann });
                self.skip_newlines();
                if self.eat(&TokenKind::Comma).is_some() {
                    self.skip_newlines();
                    continue;
                }
                break;
            }
        }
        self.skip_newlines();
        self.expect(&TokenKind::RParen, "`)`")?;
        Ok(params)
    }

    fn parse_annotations(&mut self) -> Result<Vec<Annotation>, SyntaxError> {
        let mut anns = Vec::new();
        loop {
            self.skip_newlines();
            if !self.at(&TokenKind::At) {
                break;
            }
            self.bump();
            self.skip_newlines();
            let t = self.bump();
            let ann = match t.kind {
                TokenKind::Ident(s) => match s.as_str() {
                    "parallel" => Annotation::Parallel,
                    "jit" => Annotation::Jit,
                    "gpu" => Annotation::Gpu,
                    _ => return Err(self.err(t.span, format!("unknown annotation `@{s}`"))),
                },
                _ => return Err(self.err(t.span, "expected annotation name after `@`".into())),
            };
            anns.push(ann);
        }
        Ok(anns)
    }

    fn parse_type(&mut self) -> Result<Type, SyntaxError> {
        self.skip_newlines();
        let t = self.bump();
        match t.kind {
            TokenKind::Ident(s) => match s.as_str() {
                "Number" => Ok(Type::Number),
                "Integer" => Ok(Type::Integer),
                "Rational" => Ok(Type::Rational),
                "F64" => Ok(Type::F64),
                "F32" => Ok(Type::F32),
                "I32" => Ok(Type::I32),
                "Complex" => Ok(Type::Complex),
                "Expr" => Ok(Type::Expr),
                "Symbol" => Ok(Type::Symbol),
                "Bool" => Ok(Type::Bool),
                "String" => Ok(Type::String),
                "Char" => Ok(Type::Char),
                "Array" => {
                    self.expect(&TokenKind::Lt, "`<`")?;
                    let inner = self.parse_type()?;
                    self.expect(&TokenKind::Gt, "`>`")?;
                    Ok(Type::Array(Box::new(inner)))
                }
                "Matrix" => {
                    self.expect(&TokenKind::Lt, "`<`")?;
                    let inner = self.parse_type()?;
                    self.expect(&TokenKind::Gt, "`>`")?;
                    Ok(Type::Matrix(Box::new(inner)))
                }
                "Tuple" => {
                    self.expect(&TokenKind::Lt, "`<`")?;
                    let params = self.parse_type_list()?;
                    self.expect(&TokenKind::Gt, "`>`")?;
                    Ok(Type::Tuple(params))
                }
                "Fn" | "MFn" => {
                    let is_mfn = s == "MFn";
                    self.expect(&TokenKind::LParen, "`(`")?;
                    let params = self.parse_type_list()?;
                    self.expect(&TokenKind::RParen, "`)`")?;
                    self.expect(&TokenKind::Arrow, "`->`")?;
                    let ret = self.parse_type()?;
                    if is_mfn {
                        Ok(Type::MFn { params, ret: Box::new(ret) })
                    } else {
                        Ok(Type::Fn { params, ret: Box::new(ret) })
                    }
                }
                _ => {
                    let mut segs = vec![s];
                    while self.eat(&TokenKind::ColonColon).is_some() {
                        let seg = self.parse_module_segment()?;
                        segs.push(seg.value);
                    }
                    Ok(Type::User(Spanned { value: segs.join("::"), span: t.span }))
                }
            },
            _ => Err(self.err(t.span, format!("expected type, found {}", describe(&t.kind)))),
        }
    }

    fn parse_type_list(&mut self) -> Result<Vec<Type>, SyntaxError> {
        let mut types = Vec::new();
        self.skip_newlines();
        if !self.at(&TokenKind::RParen) && !self.at(&TokenKind::Gt) {
            loop {
                types.push(self.parse_type()?);
                self.skip_newlines();
                if self.eat(&TokenKind::Comma).is_some() {
                    self.skip_newlines();
                    continue;
                }
                break;
            }
        }
        Ok(types)
    }

    fn parse_block(&mut self) -> Result<Block, SyntaxError> {
        self.skip_newlines();
        let start = self.expect(&TokenKind::LBrace, "`{`")?.span;
        let mut stmts = Vec::new();
        loop {
            self.skip_newlines();
            if self.at(&TokenKind::RBrace) {
                let end = self.bump().span;
                return Ok(Block { stmts, span: Span::merge(start, end) });
            }
            stmts.push(self.parse_stmt()?);
        }
    }

    fn parse_while_stmt(&mut self) -> Result<Stmt, SyntaxError> {
        let start = self.bump().span;
        self.skip_newlines();
        let cond = self.parse_expr()?;
        let body = self.parse_block()?;
        let span = Span::merge(start, body.span);
        Ok(Stmt::While { cond, body, span })
    }

    fn parse_if_stmt(&mut self) -> Result<Stmt, SyntaxError> {
        let start = self.bump().span;
        self.skip_newlines();
        let cond = self.parse_expr()?;
        let then = self.parse_block()?;
        let mut elifs = Vec::new();
        let mut else_ = None;
        loop {
            self.skip_newlines();
            if self.eat(&TokenKind::KwElse).is_none() {
                break;
            }
            self.skip_newlines();
            if self.eat(&TokenKind::KwIf).is_some() {
                let c = self.parse_expr()?;
                let b = self.parse_block()?;
                elifs.push((c, b));
            } else {
                else_ = Some(self.parse_block()?);
                break;
            }
        }
        let end = else_.as_ref().map(|b| b.span).unwrap_or_else(|| then.span);
        let span = Span::merge(start, end);
        Ok(Stmt::If { cond, then, elifs, else_, span })
    }

    fn parse_return_stmt(&mut self) -> Result<Stmt, SyntaxError> {
        let start = self.bump().span;
        let value = if matches!(self.peek(), TokenKind::Newline | TokenKind::Semicolon | TokenKind::RBrace) || self.at(&TokenKind::Eof) {
            None
        } else {
            Some(self.parse_expr()?)
        };
        self.end_statement();
        let span = match &value {
            Some(v) => Span::merge(start, v.span),
            None => start,
        };
        Ok(Stmt::Return { value, span })
    }

    fn parse_for_stmt(&mut self, is_par: bool) -> Result<Stmt, SyntaxError> {
        let start = self.bump().span;
        self.skip_newlines();
        let var = self.parse_ident("loop variable")?;
        self.skip_newlines();
        self.expect(&TokenKind::KwIn, "`in`")?;
        self.skip_newlines();
        let range_start = self.parse_expr()?;
        self.expect(&TokenKind::DotDot, "`..`")?;
        self.skip_newlines();
        let range_end = self.parse_expr()?;
        let step = if self.eat(&TokenKind::KwStep).is_some() {
            self.skip_newlines();
            Some(self.parse_expr()?)
        } else {
            None
        };
        let body = self.parse_block()?;
        let range = (range_start, range_end);
        let span = Span::merge(start, body.span);
        if is_par {
            Ok(Stmt::ParFor { var, range, step, body, span })
        } else {
            Ok(Stmt::For { var, range, step, body, span })
        }
    }

    fn parse_try_stmt(&mut self) -> Result<Stmt, SyntaxError> {
        let start = self.bump().span;
        let body = self.parse_block()?;
        let mut catches = Vec::new();
        loop {
            self.skip_newlines();
            if !self.at(&TokenKind::KwCatch) {
                break;
            }
            self.bump();
            self.skip_newlines();
            let var = self.parse_ident("catch variable")?;
            self.skip_newlines();
            let ty = if self.eat(&TokenKind::Colon).is_some() {
                Some(self.parse_type()?)
            } else {
                None
            };
            let block = self.parse_block()?;
            catches.push(Catch { var, ty, block });
        }
        if catches.is_empty() {
            return Err(self.err(start, "`try` requires at least one `catch` block".into()));
        }
        let end = catches.last().map(|c| c.block.span).unwrap_or(body.span);
        let span = Span::merge(start, end);
        Ok(Stmt::Try { body, catches, span })
    }

    fn parse_with_stmt(&mut self) -> Result<Stmt, SyntaxError> {
        let start = self.bump().span;
        self.skip_newlines();
        self.expect(&TokenKind::KwConfig, "`config`")?;
        self.skip_newlines();
        let entries = self.parse_config_entries()?;
        let body = self.parse_block()?;
        let span = Span::merge(start, body.span);
        Ok(Stmt::WithConfig { entries, body, span })
    }

    fn parse_pub_stmt(&mut self) -> Result<Stmt, SyntaxError> {
        let start = self.bump().span;
        self.skip_newlines();
        let inner = match self.peek().clone() {
            TokenKind::KwLet | TokenKind::KwConst | TokenKind::KwFn => self.parse_stmt()?,
            _ => return Err(self.err(self.span(), "expected `let`, `const`, or `fn` after `pub`".into())),
        };
        let _ = start;
        Ok(Stmt::Pub(Box::new(inner)))
    }

    fn parse_expr_or_assign_stmt(&mut self) -> Result<Stmt, SyntaxError> {
        let start = self.span();
        let lhs = self.parse_expr()?;
        self.skip_newlines();
        let op = match self.peek() {
            TokenKind::Eq => Some(AssignOp::Assign),
            TokenKind::PlusEq => Some(AssignOp::AddAssign),
            TokenKind::MinusEq => Some(AssignOp::SubAssign),
            _ => None,
        };
        if let Some(op) = op {
            self.bump();
            self.skip_newlines();
            let value = self.parse_expr()?;
            self.end_statement();
            let span = Span::merge(start, value.span);
            Ok(Stmt::Assign { target: lhs, op, value, span })
        } else {
            self.end_statement();
            Ok(Stmt::Expr(lhs))
        }
    }

    fn parse_expr(&mut self) -> Result<Expr, SyntaxError> {
        self.parse_expr_bp(0)
    }

    // Pratt climbing (implementation plan §2.2 precedence table): `|>` lowest, `^`/`**` highest and right-associative.
    fn parse_expr_bp(&mut self, min_bp: u8) -> Result<Expr, SyntaxError> {
        let mut lhs = self.parse_prefix()?;
        loop {
            self.skip_newlines();
            let Some((op, lbp, rbp)) = binop_bp(self.peek()) else { break };
            if lbp < min_bp {
                break;
            }
            self.bump();
            let rhs = self.parse_expr_bp(rbp)?;
            let span = Span::merge(lhs.span, rhs.span);
            lhs = if op == BinOp::Pipeline {
                Expr { kind: ExprKind::Pipeline { lhs: Box::new(lhs), rhs: Box::new(rhs) }, span }
            } else {
                Expr { kind: ExprKind::Binary { op, lhs: Box::new(lhs), rhs: Box::new(rhs) }, span }
            };
        }
        Ok(lhs)
    }

    fn parse_prefix(&mut self) -> Result<Expr, SyntaxError> {
        self.skip_newlines();
        let tok = self.bump();
        match tok.kind {
            TokenKind::Minus => {
                let operand = self.parse_expr_bp(UNARY_BP)?;
                let span = Span::merge(tok.span, operand.span);
                Ok(Expr { kind: ExprKind::Unary { op: UnOp::Neg, operand: Box::new(operand) }, span })
            }
            TokenKind::Bang => {
                let operand = self.parse_expr_bp(UNARY_BP)?;
                let span = Span::merge(tok.span, operand.span);
                Ok(Expr { kind: ExprKind::Unary { op: UnOp::Not, operand: Box::new(operand) }, span })
            }
            TokenKind::Plus => {
                let operand = self.parse_expr_bp(UNARY_BP)?;
                let span = Span::merge(tok.span, operand.span);
                Ok(Expr { kind: ExprKind::Unary { op: UnOp::Pos, operand: Box::new(operand) }, span })
            }
            _ => {
                let atom = self.parse_atom(tok)?;
                self.parse_postfix(atom)
            }
        }
    }

    fn parse_atom(&mut self, tok: Token) -> Result<Expr, SyntaxError> {
        let span = tok.span;
        let kind = match tok.kind {
            TokenKind::Integer(s) => ExprKind::Literal(Literal::Integer(s)),
            TokenKind::Float(s) => ExprKind::Literal(Literal::Float(s)),
            TokenKind::Hex(s) => ExprKind::Literal(Literal::Hex(s)),
            TokenKind::Binary(s) => ExprKind::Literal(Literal::Binary(s)),
            TokenKind::Str(s) => ExprKind::Literal(Literal::Str(s)),
            TokenKind::Char(c) => ExprKind::Literal(Literal::Char(c)),
            TokenKind::TexStr(s) => ExprKind::Literal(Literal::Tex(s)),
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
            TokenKind::Pipe => return self.parse_lambda(span),
            TokenKind::KwMatch => return self.parse_match(span),
            _ => return Err(SyntaxError { span, message: format!("expected expression, found {}", describe(&tok.kind)) }),
        };
        Ok(Expr { kind, span })
    }

    fn parse_postfix(&mut self, mut e: Expr) -> Result<Expr, SyntaxError> {
        loop {
            self.skip_newlines();
            match self.peek().clone() {
                TokenKind::LParen => {
                    self.bump();
                    let args = self.parse_args()?;
                    let end = self.tokens[self.pos.saturating_sub(1)].span;
                    let span = Span::merge(e.span, end);
                    e = Expr { kind: ExprKind::Call { callee: Box::new(e), args }, span };
                }
                TokenKind::LBracket => {
                    self.bump();
                    let index = self.parse_index()?;
                    let end = self.tokens[self.pos.saturating_sub(1)].span;
                    let span = Span::merge(e.span, end);
                    e = Expr { kind: ExprKind::Index { base: Box::new(e), index }, span };
                }
                _ => break,
            }
        }
        Ok(e)
    }

    fn parse_args(&mut self) -> Result<Vec<Expr>, SyntaxError> {
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

    fn parse_paren_or_tuple(&mut self, start: Span) -> Result<Expr, SyntaxError> {
        self.skip_newlines();
        if self.at(&TokenKind::RParen) {
            let end = self.bump().span;
            return Ok(Expr { kind: ExprKind::Tuple(vec![]), span: Span::merge(start, end) });
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
            Ok(Expr { kind: ExprKind::Tuple(items), span: Span::merge(start, end) })
        } else {
            let end = self.expect(&TokenKind::RParen, "`)`")?.span;
            Ok(Expr { kind: first.kind, span: Span::merge(start, end) })
        }
    }

    fn parse_array(&mut self, start: Span) -> Result<Expr, SyntaxError> {
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
        let end = self.expect(&TokenKind::RBracket, "`]`")?.span;
        Ok(Expr { kind: ExprKind::Array(items), span: Span::merge(start, end) })
    }

    fn parse_index(&mut self) -> Result<Index, SyntaxError> {
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
                    IndexItem::Slice { start: Some(start), end }
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

    fn parse_lambda(&mut self, start: Span) -> Result<Expr, SyntaxError> {
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
                params.push(Param { name, type_ann });
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
        Ok(Expr { kind: ExprKind::Lambda { params, body: Box::new(body) }, span })
    }

    fn parse_match(&mut self, start: Span) -> Result<Expr, SyntaxError> {
        self.skip_newlines();
        let scrutinee = self.parse_expr()?;
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
            self.expect(&TokenKind::FatArrow, "`=>`")?;
            self.skip_newlines();
            let body = self.parse_expr()?;
            arms.push(MatchArm { pattern, body });
            self.skip_newlines();
            self.eat(&TokenKind::Comma);
        }
        let end = self.tokens[self.pos.saturating_sub(1)].span;
        Ok(Expr { kind: ExprKind::Match { scrutinee: Box::new(scrutinee), arms }, span: Span::merge(start, end) })
    }

    fn parse_pattern(&mut self) -> Result<Pattern, SyntaxError> {
        self.skip_newlines();
        let tok = self.bump();
        match tok.kind {
            TokenKind::Underscore => Ok(Pattern::Wildcard(tok.span)),
            TokenKind::Integer(s) => Ok(Pattern::Literal(Literal::Integer(s))),
            TokenKind::Float(s) => Ok(Pattern::Literal(Literal::Float(s))),
            TokenKind::Str(s) => Ok(Pattern::Literal(Literal::Str(s))),
            TokenKind::KwTrue => Ok(Pattern::Literal(Literal::Bool(true))),
            TokenKind::KwFalse => Ok(Pattern::Literal(Literal::Bool(false))),
            TokenKind::Symbol(s) => Ok(Pattern::Binding(Spanned { value: s, span: tok.span })),
            TokenKind::Ident(s) => {
                let mut segs = vec![Spanned { value: s, span: tok.span }];
                while self.eat(&TokenKind::ColonColon).is_some() {
                    segs.push(self.parse_module_segment()?);
                }
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
                    Ok(Pattern::Ctor { name: segs, args, span: Span::merge(tok.span, end) })
                } else if segs.len() == 1 {
                    Ok(Pattern::Binding(segs.pop().unwrap()))
                } else {
                    Ok(Pattern::Path(segs))
                }
            }
            _ => Err(self.err(tok.span, format!("expected pattern, found {}", describe(&tok.kind)))),
        }
    }
}

fn binop_bp(kind: &TokenKind) -> Option<(BinOp, u8, u8)> {
    let (op, lbp, rbp) = match kind {
        TokenKind::PipeArrow => (BinOp::Pipeline, 1, 2),
        TokenKind::PipePipe => (BinOp::Or, 2, 3),
        TokenKind::AmpAmp => (BinOp::And, 3, 4),
        TokenKind::EqEq => (BinOp::Eq, 4, 5),
        TokenKind::BangEq => (BinOp::Ne, 4, 5),
        TokenKind::Lt => (BinOp::Lt, 4, 5),
        TokenKind::LtEq => (BinOp::Le, 4, 5),
        TokenKind::Gt => (BinOp::Gt, 4, 5),
        TokenKind::GtEq => (BinOp::Ge, 4, 5),
        TokenKind::Plus => (BinOp::Add, 5, 6),
        TokenKind::Minus => (BinOp::Sub, 5, 6),
        TokenKind::Star => (BinOp::Mul, 6, 7),
        TokenKind::Slash => (BinOp::Div, 6, 7),
        TokenKind::Percent => (BinOp::Mod, 6, 7),
        TokenKind::At => (BinOp::MatMul, 6, 7),
        TokenKind::AtDot => (BinOp::Broadcast, 6, 7),
        TokenKind::Caret | TokenKind::DoubleStar => (BinOp::Pow, 8, 8),
        _ => return None,
    };
    Some((op, lbp, rbp))
}
