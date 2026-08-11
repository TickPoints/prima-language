use crate::ast::*;
use crate::error::{SyntaxError, SyntaxWarning};
use crate::lexer::lex;
use crate::span::Span;
use crate::token::{describe, Token, TokenKind};

// Unary operator binding power: lower than power `^` (8), higher than mul/div (6/7), implementing `-x^2 == -(x^2)` (same as Julia, spec §2.2).
const UNARY_BP: u8 = 7;

/// Hand-written recursive-descent + Pratt climbing parser (implementation plan §2.2), covering all appendix A BNF productions.
pub fn parse(src: &str) -> Result<Program, Vec<SyntaxError>> {
    let (program, errors, _) = parse_checked(src);
    if errors.is_empty() { Ok(program) } else { Err(errors) }
}

/// Parse and return the program plus all collected errors and warnings (spec §16.4/§16.5).
pub fn parse_checked(src: &str) -> (Program, Vec<SyntaxError>, Vec<SyntaxWarning>) {
    let tokens = match lex(src) {
        Ok(t) => t,
        Err(errors) => return (Program { config: None, imports: Vec::new(), stmts: Vec::new() }, errors, Vec::new()),
    };
    let mut parser = Parser::new(tokens);
    match parser.parse_program_inner() {
        Ok(program) => (program, Vec::new(), parser.warnings),
        Err(e) => (Program { config: None, imports: Vec::new(), stmts: Vec::new() }, vec![e], parser.warnings),
    }
}

// Parser: errors use panic-mode recovery with sync tokens (`;`, `}`, `)`, end of file), collecting all syntax errors in one compilation (spec §2.2).
pub(crate) struct Parser {
    tokens: Vec<Token>,
    pos: usize,
    /// Deprecated newline-as-statement-separator span seen most recently (spec §4.2), cleared by `end_statement`.
    pending_newline: Option<Span>,
    /// Collected warnings (spec §16.5), e.g. `W0001` newline statement separation.
    warnings: Vec<SyntaxWarning>,
    /// Disables struct-literal parsing in control-flow conditions (`if x {` must stay a block, not `x { ... }`).
    no_struct_literal: bool,
}

impl Parser {
    pub(crate) fn new(tokens: Vec<Token>) -> Parser {
        Parser { tokens, pos: 0, pending_newline: None, warnings: Vec::new(), no_struct_literal: false }
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

    /// Skip newlines; remember the last one so `end_statement` can warn about the deprecated newline separator (spec §4.2, W0001).
    fn skip_newlines(&mut self) {
        while matches!(self.peek(), TokenKind::Newline) {
            let span = self.tokens[self.pos].span;
            self.bump();
            self.pending_newline = Some(span);
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

    fn warn(&mut self, span: Span, code: &'static str, message: &str) {
        self.warnings.push(SyntaxWarning { span, code, message: message.to_string() });
    }

    /// Statement terminator (spec §4.2): `;` is the canonical separator; a deprecated newline separator is warned (W0001).
    /// EOF/`}` (block end) terminate silently. Block-level statements use `finish_block_statement` instead.
    fn end_statement(&mut self) {
        if self.eat(&TokenKind::Semicolon).is_some() {
            self.pending_newline = None;
            return;
        }
        if let Some(span) = self.pending_newline.take()
            && !matches!(self.peek(), TokenKind::Eof | TokenKind::RBrace)
        {
            self.warn(span, "W0001", "statements separated by newlines are deprecated; use `;`");
        }
    }

    /// Terminator for block-level statements (spec §4.2): the trailing `;` is optional and never warned.
    fn finish_block_statement(&mut self) {
        if self.eat(&TokenKind::Semicolon).is_some() {
            self.pending_newline = None;
        }
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

    pub(crate) fn parse_program_inner(&mut self) -> Result<Program, SyntaxError> {
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
        self.end_statement();
        let end = self.tokens[self.pos.saturating_sub(1)].span;
        Ok(Import { kind, span: Span::merge(start, end) })
    }

    fn parse_stmt(&mut self) -> Result<Stmt, SyntaxError> {
        self.skip_newlines();
        // Statement-level annotations (spec §18.4): `@builtin`/`@c_api::extern` before a `pub`/`fn`/`class` item.
        let annotations = self.parse_annotations()?;
        let stmt = match self.peek().clone() {
            TokenKind::KwLet => self.parse_let_stmt()?,
            TokenKind::KwConst => self.parse_const_stmt()?,
            TokenKind::KwFn => self.parse_fn_stmt()?,
            TokenKind::KwClass => self.parse_class_def()?,
            TokenKind::KwImpl => self.parse_impl_stmt()?,
            TokenKind::KwFor => self.parse_for_stmt(false)?,
            TokenKind::KwParFor => self.parse_for_stmt(true)?,
            TokenKind::KwWhile => self.parse_while_stmt()?,
            TokenKind::KwIf => self.parse_if_stmt()?,
            TokenKind::KwMatch => self.parse_match_stmt()?,
            TokenKind::KwReturn => self.parse_return_stmt()?,
            TokenKind::KwTry => {
                let span = self.span();
                self.skip_to_statement_boundary();
                return Err(SyntaxError {
                    span,
                    message: "`try`/`catch` was removed in v2.0 (E0010); use `Result`/`match`/`?` instead (spec §16.3)".into(),
                });
            }
            TokenKind::KwWith => self.parse_with_stmt()?,
            TokenKind::KwPub => self.parse_pub_stmt()?,
            _ => self.parse_expr_or_assign_stmt()?,
        };
        let stmt = if annotations.is_empty() { stmt } else { self.apply_annotations(stmt, &annotations)? };
        // Statement terminator (spec §4.2): block-level statements may omit `;`; the rest require `;` (newline is deprecated, W0001).
        match &stmt {
            Stmt::FnDef { .. }
            | Stmt::ClassDef { .. }
            | Stmt::Impl { .. }
            | Stmt::For { .. }
            | Stmt::ParFor { .. }
            | Stmt::While { .. }
            | Stmt::If { .. }
            | Stmt::IfLet { .. }
            | Stmt::WhileLet { .. }
            | Stmt::Match { .. }
            | Stmt::WithConfig { .. } => self.finish_block_statement(),
            _ => self.end_statement(),
        }
        Ok(stmt)
    }

    /// Attach statement-level annotations to the definition they precede (spec §18.4).
    fn apply_annotations(&mut self, stmt: Stmt, anns: &[Annotation]) -> Result<Stmt, SyntaxError> {
        match stmt {
            Stmt::FnDef { name, params, ret, mut annotations, body, span } => {
                annotations.extend_from_slice(anns);
                Ok(Stmt::FnDef { name, params, ret, annotations, body, span })
            }
            Stmt::MathDef { name, params, ret, mut annotations, body, span } => {
                annotations.extend_from_slice(anns);
                Ok(Stmt::MathDef { name, params, ret, annotations, body, span })
            }
            Stmt::ClassDef { name, mut members, span } => {
                // A `@builtin` class carries the annotation on every method (signature-only bodies are the builtin form, spec §18.4).
                if anns.contains(&Annotation::Builtin) {
                    for m in &mut members {
                        if let ClassMemberKind::Method { annotations, .. } = &mut m.kind {
                            annotations.push(Annotation::Builtin);
                        }
                    }
                }
                Ok(Stmt::ClassDef { name, members, span })
            }
            Stmt::Pub(inner) => self.apply_annotations(*inner, anns).map(Box::new).map(Stmt::Pub),
            other => {
                let span = stmt_span_of(&other);
                Err(self.err(span, "annotations are only allowed on `fn`/`let` definitions and classes".into()))
            }
        }
    }

    /// Skip tokens up to the next statement boundary, so a removed-construct error still recovers cleanly (spec §2.2 sync tokens).
    fn skip_to_statement_boundary(&mut self) {
        while !matches!(self.peek(), TokenKind::Semicolon | TokenKind::RBrace | TokenKind::Eof | TokenKind::Newline) {
            self.bump();
        }
        if matches!(self.peek(), TokenKind::Newline) {
            self.skip_newlines();
        }
    }

    fn parse_let_stmt(&mut self) -> Result<Stmt, SyntaxError> {
        let start = self.bump().span;
        self.skip_newlines();
        let mut_ = self.eat(&TokenKind::KwMut).is_some();
        // Math definition `let f(x) = expr` (spec §4.3): an identifier followed by `(`.
        let is_mathdef = matches!(self.peek(), TokenKind::Ident(_)) && matches!(self.peek_at(1), TokenKind::LParen);
        if is_mathdef {
            let name = self.parse_ident("function name")?;
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
            let span = Span::merge(start, body.span);
            return Ok(Stmt::MathDef { name, params, ret, annotations, body, span });
        }
        // Destructuring `let (a, b) = t`, `let Point { x, .. } = p`, or plain `let x = v` (spec §4.4).
        let pat = self.parse_pattern()?;
        self.skip_newlines();
        let type_ann = if self.eat(&TokenKind::Colon).is_some() {
            Some(self.parse_type()?)
        } else {
            None
        };
        self.expect(&TokenKind::Eq, "`=`")?;
        self.skip_newlines();
        let value = self.parse_expr()?;
        let span = Span::merge(start, value.span);
        Ok(Stmt::Let { pat, mut_, type_ann, value, span })
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
        // Signature-only `@builtin fn` (spec §18.4): no body — the implementation is the Rust host builtin of the same name.
        let body = if annotations.contains(&Annotation::Builtin) && !self.at(&TokenKind::LBrace) {
            self.end_statement();
            Block { stmts: Vec::new(), span: start }
        } else {
            self.parse_block()?
        };
        let span = Span::merge(start, body.span);
        Ok(Stmt::FnDef { name, params, ret, annotations, body, span })
    }

    fn parse_params(&mut self) -> Result<Vec<Param>, SyntaxError> {
        self.expect(&TokenKind::LParen, "`(`")?;
        let mut params = Vec::new();
        self.skip_newlines();
        if !self.at(&TokenKind::RParen) {
            loop {
                self.skip_newlines();
                // `self` receiver of a method (spec §4.5).
                if self.at(&TokenKind::KwSelf) {
                    let t = self.bump();
                    params.push(Param { name: Spanned { value: "self".into(), span: t.span }, type_ann: None, is_self: true });
                } else {
                    let name = self.parse_ident("parameter name")?;
                    self.skip_newlines();
                    let type_ann = if self.eat(&TokenKind::Colon).is_some() {
                        Some(self.parse_type()?)
                    } else {
                        None
                    };
                    params.push(Param { name, type_ann, is_self: false });
                }
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
                    "builtin" => Annotation::Builtin,
                    // `@c_api::extern` (spec §18.4).
                    "c_api" if self.eat(&TokenKind::ColonColon).is_some() => {
                        let seg = self.parse_ident("annotation segment")?;
                        if seg.value == "extern" {
                            Annotation::CApiExtern
                        } else {
                            return Err(self.err(seg.span, format!("unknown annotation `@c_api::{}`", seg.value)));
                        }
                    }
                    _ => return Err(self.err(t.span, format!("unknown annotation `@{s}`"))),
                },
                _ => return Err(self.err(t.span, "expected annotation name after `@`".into())),
            };
            anns.push(ann);
        }
        Ok(anns)
    }

    fn parse_class_def(&mut self) -> Result<Stmt, SyntaxError> {
        let start = self.expect(&TokenKind::KwClass, "`class`")?.span;
        self.skip_newlines();
        let name = self.parse_ident("class name")?;
        self.skip_newlines();
        self.expect(&TokenKind::LBrace, "`{`")?;
        let mut members = Vec::new();
        loop {
            self.skip_newlines();
            if self.at(&TokenKind::RBrace) {
                self.bump();
                break;
            }
            let member_start = self.span();
            let vis = self.parse_vis()?;
            self.skip_newlines();
            if self.at(&TokenKind::KwFn) {
                self.bump();
                self.skip_newlines();
                let mname = self.parse_ident("method name")?;
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
                // Signature-only method for `@builtin` classes (spec §18.4); otherwise a body is required.
                let body = if self.at(&TokenKind::LBrace) {
                    Some(self.parse_block()?)
                } else {
                    None
                };
                let end = self.tokens[self.pos.saturating_sub(1)].span;
                let span = Span::merge(member_start, end);
                members.push(ClassMember {
                    vis,
                    kind: ClassMemberKind::Method { name: mname, params, ret, annotations, body },
                    span,
                });
            } else {
                let fname = self.parse_ident("field name")?;
                self.skip_newlines();
                self.expect(&TokenKind::Colon, "`:`")?;
                let ty = self.parse_type()?;
                let end = self.tokens[self.pos.saturating_sub(1)].span;
                let span = Span::merge(member_start, end);
                members.push(ClassMember { vis, kind: ClassMemberKind::Field { name: fname, ty }, span });
            }
            self.skip_newlines();
            self.eat(&TokenKind::Comma); // members are comma-separated (spec §4.5)
        }
        let end = self.tokens[self.pos.saturating_sub(1)].span;
        Ok(Stmt::ClassDef { name, members, span: Span::merge(start, end) })
    }

    /// Visibility modifier (spec §15.2): none / `pub` / `pub(mod)`.
    fn parse_vis(&mut self) -> Result<Visibility, SyntaxError> {
        self.skip_newlines();
        if !self.at(&TokenKind::KwPub) {
            return Ok(Visibility::Private);
        }
        self.bump();
        self.skip_newlines();
        if self.eat(&TokenKind::LParen).is_some() {
            self.skip_newlines();
            let m = self.parse_ident("`mod`")?;
            if m.value != "mod" {
                return Err(self.err(m.span, "expected `mod` inside `pub(...)`".into()));
            }
            self.skip_newlines();
            self.expect(&TokenKind::RParen, "`)`")?;
            Ok(Visibility::Module)
        } else {
            Ok(Visibility::Public)
        }
    }

    fn parse_impl_stmt(&mut self) -> Result<Stmt, SyntaxError> {
        let start = self.expect(&TokenKind::KwImpl, "`impl`")?.span;
        self.skip_newlines();
        // `impl ops::Add for Vec2 { ... }` (spec §18.5).
        let ns = self.parse_module_segment()?;
        if ns.value != "ops" {
            return Err(self.err(ns.span, "`impl` must target `ops` (e.g. `impl ops::Add for T`) (spec §18.5)".into()));
        }
        self.expect(&TokenKind::ColonColon, "`::`")?;
        let op_seg = self.parse_ident("operator name")?;
        let op = match op_seg.value.as_str() {
            "Add" => ImplOp::Add,
            "Sub" => ImplOp::Sub,
            "Mul" => ImplOp::Mul,
            "Div" => ImplOp::Div,
            "Rem" => ImplOp::Rem,
            "Neg" => ImplOp::Neg,
            "Eq" => ImplOp::Eq,
            "Cmp" => ImplOp::Cmp,
            "Index" => ImplOp::Index,
            other => return Err(self.err(op_seg.span, format!("unknown operator overload `ops::{other}`"))),
        };
        self.skip_newlines();
        self.expect(&TokenKind::KwFor, "`for`")?;
        self.skip_newlines();
        let target = self.parse_ident("class name")?;
        self.skip_newlines();
        self.expect(&TokenKind::LBrace, "`{`")?;
        let mut members = Vec::new();
        loop {
            self.skip_newlines();
            if self.at(&TokenKind::RBrace) {
                self.bump();
                break;
            }
            self.expect(&TokenKind::KwFn, "`fn`")?;
            self.skip_newlines();
            let name = self.parse_ident("method name")?;
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
            let span = Span::merge(name.span, body.span);
            members.push(Box::new(Stmt::FnDef { name, params, ret, annotations, body, span }));
        }
        let end = self.tokens[self.pos.saturating_sub(1)].span;
        Ok(Stmt::Impl { op, target, members, span: Span::merge(start, end) })
    }

    fn parse_type(&mut self) -> Result<Type, SyntaxError> {
        self.skip_newlines();
        let t = self.bump();
        match t.kind {
            TokenKind::KwSelfType => Ok(Type::SelfType),
            TokenKind::Ident(s) => match s.as_str() {
                "Number" => Ok(Type::Number),
                "Integer" => Ok(Type::Integer),
                "Rational" => Ok(Type::Rational),
                "F64" => Ok(Type::F64),
                "F32" => Ok(Type::F32),
                "I8" => Ok(Type::I8),
                "I16" => Ok(Type::I16),
                "I32" => Ok(Type::I32),
                "I64" => Ok(Type::I64),
                "I128" => Ok(Type::I128),
                "U8" => Ok(Type::U8),
                "U16" => Ok(Type::U16),
                "U32" => Ok(Type::U32),
                "U64" => Ok(Type::U64),
                "U128" => Ok(Type::U128),
                "Isize" => Ok(Type::Isize),
                "Usize" => Ok(Type::Usize),
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
                "Option" => {
                    self.expect(&TokenKind::Lt, "`<`")?;
                    let inner = self.parse_type()?;
                    self.expect(&TokenKind::Gt, "`>`")?;
                    Ok(Type::Option(Box::new(inner)))
                }
                "Result" => {
                    self.expect(&TokenKind::Lt, "`<`")?;
                    let ok = self.parse_type()?;
                    self.skip_newlines();
                    self.expect(&TokenKind::Comma, "`,`")?;
                    self.skip_newlines();
                    let err = self.parse_type()?;
                    self.expect(&TokenKind::Gt, "`>`")?;
                    Ok(Type::Result(Box::new(ok), Box::new(err)))
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
        // `while let pattern = expr { ... }` (spec §4.4).
        if self.eat(&TokenKind::KwLet).is_some() {
            let pat = self.parse_pattern()?;
            self.skip_newlines();
            self.expect(&TokenKind::Eq, "`=`")?;
            self.skip_newlines();
            let value = self.parse_scrutinee()?;
            let body = self.parse_block()?;
            let span = Span::merge(start, body.span);
            return Ok(Stmt::WhileLet { pat, value, body, span });
        }
        let cond = self.parse_scrutinee()?;
        let body = self.parse_block()?;
        let span = Span::merge(start, body.span);
        Ok(Stmt::While { cond, body, span })
    }

    fn parse_if_stmt(&mut self) -> Result<Stmt, SyntaxError> {
        let start = self.bump().span;
        self.skip_newlines();
        // `if let pattern = expr { ... }` (spec §4.4).
        if self.eat(&TokenKind::KwLet).is_some() {
            let pat = self.parse_pattern()?;
            self.skip_newlines();
            self.expect(&TokenKind::Eq, "`=`")?;
            self.skip_newlines();
            let value = self.parse_scrutinee()?;
            let then = self.parse_block()?;
            let mut else_ = None;
            self.skip_newlines();
            if self.eat(&TokenKind::KwElse).is_some() {
                self.skip_newlines();
                if self.eat(&TokenKind::KwIf).is_some() {
                    // `else if let ...` — chain via a nested IfLet in the else branch.
                    let nested = self.parse_if_let_after_else()?;
                    else_ = Some(nested);
                } else {
                    else_ = Some(self.parse_block()?);
                }
            }
            let end = else_.as_ref().map(|b| b.span).unwrap_or(then.span);
            let span = Span::merge(start, end);
            return Ok(Stmt::IfLet { pat, value, then, else_, span });
        }
        let cond = self.parse_scrutinee()?;
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
                let c = self.parse_scrutinee()?;
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

    /// Parses the body of `else if let` — returns a nested `Stmt::IfLet` wrapped in a single-statement block.
    fn parse_if_let_after_else(&mut self) -> Result<Block, SyntaxError> {
        let start = self.span();
        let pat = self.parse_pattern()?;
        self.skip_newlines();
        self.expect(&TokenKind::Eq, "`=`")?;
        self.skip_newlines();
        let value = self.parse_scrutinee()?;
        let then = self.parse_block()?;
        let mut else_ = None;
        self.skip_newlines();
        if self.eat(&TokenKind::KwElse).is_some() {
            self.skip_newlines();
            if self.eat(&TokenKind::KwIf).is_some() {
                else_ = Some(self.parse_if_let_after_else()?);
            } else {
                else_ = Some(self.parse_block()?);
            }
        }
        let end = else_.as_ref().map(|b| b.span).unwrap_or(then.span);
        let span = Span::merge(start, end);
        let stmt = Stmt::IfLet { pat, value, then, else_, span };
        Ok(Block { stmts: vec![stmt.clone()], span: stmt_span_of(&stmt) })
    }

    fn parse_return_stmt(&mut self) -> Result<Stmt, SyntaxError> {
        let start = self.bump().span;
        let value = if matches!(self.peek(), TokenKind::Newline | TokenKind::Semicolon | TokenKind::RBrace) || self.at(&TokenKind::Eof) {
            None
        } else {
            Some(self.parse_expr()?)
        };
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
        // Parse the range end with struct literals disabled so `for j in 0..n {` keeps `{` as the loop body (spec §14).
        let range_end = self.parse_scrutinee()?;
        let step = if self.eat(&TokenKind::KwStep).is_some() {
            self.skip_newlines();
            Some(self.parse_scrutinee()?)
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
        // `pub(mod)` at statement level (spec §15.2): consumed and ignored for statements (visibility matters for class members).
        if self.eat(&TokenKind::LParen).is_some() {
            self.skip_newlines();
            let m = self.parse_ident("`mod`")?;
            if m.value != "mod" {
                return Err(self.err(m.span, "expected `mod` inside `pub(...)`".into()));
            }
            self.skip_newlines();
            self.expect(&TokenKind::RParen, "`)`")?;
            self.skip_newlines();
        }
        let inner = match self.peek().clone() {
            TokenKind::KwLet | TokenKind::KwConst | TokenKind::KwFn | TokenKind::KwClass => self.parse_stmt()?,
            _ => return Err(self.err(self.span(), "expected `let`, `const`, `fn`, or `class` after `pub`".into())),
        };
        let _ = start;
        Ok(Stmt::Pub(Box::new(inner)))
    }

    fn parse_match_stmt(&mut self) -> Result<Stmt, SyntaxError> {
        let start = self.expect(&TokenKind::KwMatch, "`match`")?.span;
        self.skip_newlines();
        let scrutinee = self.parse_scrutinee()?;
        let arms = self.parse_match_arms()?;
        let end = self.tokens[self.pos.saturating_sub(1)].span;
        Ok(Stmt::Match { scrutinee, arms, span: Span::merge(start, end) })
    }

    /// `match <expr> {` — parse the scrutinee with struct literals disabled so `match x { ... }` treats `{` as the arms block.
    fn parse_scrutinee(&mut self) -> Result<Expr, SyntaxError> {
        let prev = self.no_struct_literal;
        self.no_struct_literal = true;
        let r = self.parse_expr();
        self.no_struct_literal = prev;
        r
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
            let span = Span::merge(start, value.span);
            Ok(Stmt::Assign { target: lhs, op, value, span })
        } else {
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
                // Deprecated `|>` pipeline (spec §9.7): keep the AST node; the runtime emits `W0002` and rewrites it.
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
            TokenKind::KwSelf => {
                let e = Expr { kind: ExprKind::Self_, span: tok.span };
                self.parse_postfix(e)
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
            TokenKind::KwMatch => return self.parse_match_expr(span),
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
                        e = Expr { kind: ExprKind::MethodCall { receiver: Box::new(e), name, args }, span };
                    } else {
                        let span = Span::merge(e.span, name.span);
                        e = Expr { kind: ExprKind::Field { receiver: Box::new(e), name }, span };
                    }
                }
                TokenKind::Question => {
                    // `expr?` try operator (spec §16.3).
                    let q = self.bump();
                    let span = Span::merge(e.span, q.span);
                    e = Expr { kind: ExprKind::Try(Box::new(e)), span };
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
    fn parse_struct_literal(&mut self, name: Spanned<String>, path: Expr) -> Result<Expr, SyntaxError> {
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
                    return Err(self.err(self.span(), "expected `}` after the struct update base".into()));
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
        Ok(Expr { kind: ExprKind::StructLiteral { name, fields, base }, span })
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
                params.push(Param { name, type_ann, is_self: false });
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

    fn parse_match_expr(&mut self, start: Span) -> Result<Expr, SyntaxError> {
        self.skip_newlines();
        let scrutinee = self.parse_scrutinee()?;
        let arms = self.parse_match_arms()?;
        let end = self.tokens[self.pos.saturating_sub(1)].span;
        Ok(Expr { kind: ExprKind::Match { scrutinee: Box::new(scrutinee), arms }, span: Span::merge(start, end) })
    }

    /// `{ pattern [if guard] => expr, ... }` (spec §4.4).
    fn parse_match_arms(&mut self) -> Result<Vec<MatchArm>, SyntaxError> {
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
            arms.push(MatchArm { pattern, guard, body });
            self.skip_newlines();
            self.eat(&TokenKind::Comma);
            self.eat(&TokenKind::Semicolon);
        }
        Ok(arms)
    }

    /// Full pattern grammar (spec §4.4): `_`, bindings, literals, tuple/array/struct/constructor patterns,
    /// range patterns, or-patterns, and grouping.
    fn parse_pattern(&mut self) -> Result<Pattern, SyntaxError> {
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

    fn parse_pattern_simple(&mut self) -> Result<Pattern, SyntaxError> {
        self.skip_newlines();
        // Leading `-` for negative literal patterns.
        if self.at(&TokenKind::Minus) {
            let m = self.bump();
            self.skip_newlines();
            let t = self.bump();
            let lit = match t.kind {
                TokenKind::Integer(s) => Literal::Integer(format!("-{s}")),
                TokenKind::Float(s) => Literal::Float(format!("-{s}")),
                _ => return Err(self.err(t.span, "expected a numeric literal after `-` in a pattern".into())),
            };
            let _ = m;
            return Ok(Pattern::Literal(lit));
        }
        let tok = self.bump();
        match tok.kind {
            TokenKind::Underscore => Ok(Pattern::Wildcard(tok.span)),
            TokenKind::Integer(s) => self.parse_pattern_range(Pattern::Literal(Literal::Integer(s)), tok.span),
            TokenKind::Float(s) => self.parse_pattern_range(Pattern::Literal(Literal::Float(s)), tok.span),
            TokenKind::Str(s) => Ok(Pattern::Literal(Literal::Str(s))),
            TokenKind::Char(c) => self.parse_pattern_range(Pattern::Literal(Literal::Char(c)), tok.span),
            TokenKind::KwTrue => Ok(Pattern::Literal(Literal::Bool(true))),
            TokenKind::KwFalse => Ok(Pattern::Literal(Literal::Bool(false))),
            TokenKind::Symbol(s) => Ok(Pattern::Binding(Spanned { value: s, span: tok.span })),
            TokenKind::LParen => self.parse_tuple_pattern(tok.span),
            TokenKind::LBracket => self.parse_array_pattern(tok.span),
            TokenKind::Ident(s) => {
                // `Some(x)`/`Ok(v)` constructor pattern or `Type { ... }` struct pattern (spec §4.4).
                let name = Spanned { value: s, span: tok.span };
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
                    return Ok(Pattern::Variant { name, args, span: Span::merge(tok.span, end) });
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
                                return Err(self.err(self.span(), "expected `}` after `..` in a struct pattern".into()));
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
            _ => Err(self.err(tok.span, format!("expected pattern, found {}", describe(&tok.kind)))),
        }
    }

    /// Parse a range pattern continuation after a numeric/char start literal: `0..9` / `1..=5`.
    fn parse_pattern_range(&mut self, start: Pattern, span: Span) -> Result<Pattern, SyntaxError> {
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

    fn parse_tuple_pattern(&mut self, start: Span) -> Result<Pattern, SyntaxError> {
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
                        return Err(self.err(self.span(), "expected `)` after `..` in a tuple pattern".into()));
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

    fn parse_array_pattern(&mut self, start: Span) -> Result<Pattern, SyntaxError> {
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
                        return Err(self.err(self.span(), "expected `]` after `..` in an array pattern".into()));
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

fn stmt_span_of(stmt: &Stmt) -> Span {
    match stmt {
        Stmt::IfLet { span, .. }
        | Stmt::WhileLet { span, .. }
        | Stmt::Match { span, .. }
        | Stmt::ClassDef { span, .. }
        | Stmt::Impl { span, .. }
        | Stmt::Let { span, .. }
        | Stmt::Const { span, .. }
        | Stmt::FnDef { span, .. }
        | Stmt::MathDef { span, .. }
        | Stmt::Assign { span, .. }
        | Stmt::For { span, .. }
        | Stmt::ParFor { span, .. }
        | Stmt::While { span, .. }
        | Stmt::If { span, .. }
        | Stmt::Return { span, .. }
        | Stmt::WithConfig { span, .. } => *span,
        Stmt::Expr(e) => e.span,
        Stmt::Pub(inner) => stmt_span_of(inner),
    }
}
