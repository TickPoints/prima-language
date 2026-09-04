//! Top-level program, config, import, doc-comment, and low-level token helpers (spec §4.1/§15/§18.1).
//!
//! This module owns the program/import/config parsers, doc-comment collection, and the low-level token cursor helpers (`peek`/`bump`/`expect`/`end_statement`/...) shared by the sibling parsing modules.

use super::Parser;
use crate::ast::*;
use crate::error::{SyntaxError, SyntaxWarning};
use crate::span::Span;
use crate::token::{Token, TokenKind, describe};

impl Parser {
    pub(crate) fn peek(&self) -> &TokenKind {
        &self.tokens[self.pos].kind
    }

    pub(crate) fn peek_at(&self, n: usize) -> &TokenKind {
        self.tokens
            .get(self.pos + n)
            .map(|t| &t.kind)
            .unwrap_or(&TokenKind::Eof)
    }

    pub(crate) fn span(&self) -> Span {
        self.tokens[self.pos].span
    }

    pub(crate) fn bump(&mut self) -> Token {
        let t = self.tokens[self.pos].clone();
        if self.pos < self.tokens.len() - 1 {
            self.pos += 1;
        }
        t
    }

    /// Skip newlines between tokens; statement separation is now enforced by `end_statement` (spec §4.2).
    pub(crate) fn skip_newlines(&mut self) {
        while matches!(self.peek(), TokenKind::Newline) {
            self.bump();
        }
    }

    pub(crate) fn at(&self, kind: &TokenKind) -> bool {
        self.peek() == kind
    }

    pub(crate) fn eat(&mut self, kind: &TokenKind) -> Option<Token> {
        if self.at(kind) {
            Some(self.bump())
        } else {
            None
        }
    }

    pub(crate) fn expect(&mut self, kind: &TokenKind, what: &str) -> Result<Token, SyntaxError> {
        if self.at(kind) {
            Ok(self.bump())
        } else {
            Err(SyntaxError {
                span: self.span(),
                message: format!("expected {what}, found {}", describe(self.peek())),
            })
        }
    }

    pub(crate) fn err(&self, span: Span, message: String) -> SyntaxError {
        SyntaxError { span, message }
    }

    /// Record a non-fatal warning (spec §16.5), e.g. the `W0006` `format` deprecation hint.
    pub(crate) fn push_warning(&mut self, code: &'static str, span: Span, message: String) {
        self.warnings.push(SyntaxWarning {
            span,
            code,
            message,
        });
    }

    /// Consume a run of consecutive doc-comment tokens (`///` or `//!`, per `module`), merging
    /// their lines into a `DocComment`. Blank lines between doc lines do not break the run (spec
    /// §4.1: consecutive lines merge for the following item). Returns `None` at the first non-doc token.
    pub(crate) fn take_docs(&mut self, module: bool) -> Option<DocComment> {
        let mut lines = Vec::new();
        let mut start = None;
        let mut end = None;
        loop {
            match self.peek() {
                TokenKind::Doc { module: m, .. } if *m == module => {}
                TokenKind::Newline => {
                    self.bump();
                    continue;
                }
                _ => break,
            }
            let TokenKind::Doc { text, .. } = self.peek().clone() else {
                unreachable!()
            };
            let span = self.span();
            lines.push((text, span));
            if start.is_none() {
                start = Some(span);
            }
            end = Some(span);
            self.bump();
        }
        if lines.is_empty() {
            None
        } else {
            Some(DocComment {
                lines,
                span: Span::merge(start.unwrap(), end.unwrap()),
            })
        }
    }

    /// Attach `docs` to a definition statement; a doc comment in front of any other statement
    /// kind is a dangling comment and warns `W0007` (spec §4.1, spec §16.5).
    pub(crate) fn attach_docs(
        &mut self,
        stmt: Stmt,
        docs: Option<DocComment>,
    ) -> Result<Stmt, SyntaxError> {
        let Some(docs) = docs else { return Ok(stmt) };
        match stmt {
            Stmt::Let {
                pat,
                mut_,
                type_ann,
                value,
                span,
                ..
            } => Ok(Stmt::Let {
                pat,
                mut_,
                type_ann,
                value,
                span,
                docs: Some(docs),
            }),
            Stmt::Const {
                name,
                type_ann,
                value,
                span,
                ..
            } => Ok(Stmt::Const {
                name,
                type_ann,
                value,
                span,
                docs: Some(docs),
            }),
            Stmt::FnDef {
                name,
                params,
                ret,
                annotations,
                body,
                span,
                ..
            } => Ok(Stmt::FnDef {
                name,
                params,
                ret,
                annotations,
                body,
                span,
                docs: Some(docs),
            }),
            Stmt::MathDef {
                name,
                params,
                ret,
                annotations,
                body,
                span,
                ..
            } => Ok(Stmt::MathDef {
                name,
                params,
                ret,
                annotations,
                body,
                span,
                docs: Some(docs),
            }),
            Stmt::ClassDef {
                name,
                annotations,
                members,
                span,
                ..
            } => Ok(Stmt::ClassDef {
                name,
                annotations,
                members,
                span,
                docs: Some(docs),
            }),
            other => {
                self.push_warning(
                    "W0007",
                    docs.span,
                    "doc comment has no following definition (W0007); `///` must precede an item (spec §4.1)".into(),
                );
                Ok(other)
            }
        }
    }

    /// First token kind at or after `self.pos`, skipping any `Newline` tokens (spec §4.2).
    pub(crate) fn peek_non_newline(&self) -> &TokenKind {
        let mut i = self.pos;
        while i < self.tokens.len() && matches!(self.tokens[i].kind, TokenKind::Newline) {
            i += 1;
        }
        self.tokens
            .get(i)
            .map(|t| &t.kind)
            .unwrap_or(&TokenKind::Eof)
    }

    /// Statement terminator (spec §4.2): `;` is the only separator; a trailing statement at
    /// end-of-input or before a block end `}` may omit it. Any other following token is E0011.
    pub(crate) fn end_statement(&mut self) -> Result<(), SyntaxError> {
        if self.eat(&TokenKind::Semicolon).is_some() {
            return Ok(());
        }
        if matches!(self.peek_non_newline(), TokenKind::Eof | TokenKind::RBrace) {
            return Ok(());
        }
        Err(self.err(
            self.span(),
            "expected `;` to separate statements (E0011); newline statement separation was removed in v2.3 (spec §4.2)".into(),
        ))
    }

    /// Terminator for block-level statements (spec §4.2): the trailing `;` is optional.
    pub(crate) fn finish_block_statement(&mut self) {
        self.eat(&TokenKind::Semicolon);
    }

    pub(crate) fn parse_ident(&mut self, what: &str) -> Result<Spanned<String>, SyntaxError> {
        self.skip_newlines();
        let t = self.bump();
        match t.kind {
            TokenKind::Ident(s) => Ok(Spanned {
                value: s,
                span: t.span,
            }),
            _ => Err(SyntaxError {
                span: t.span,
                message: format!("expected {what}, found {}", describe(&t.kind)),
            }),
        }
    }

    pub(crate) fn parse_module_segment(&mut self) -> Result<Spanned<String>, SyntaxError> {
        self.skip_newlines();
        let t = self.bump();
        match t.kind {
            TokenKind::Ident(s) | TokenKind::Symbol(s) => Ok(Spanned {
                value: s,
                span: t.span,
            }),
            _ => Err(SyntaxError {
                span: t.span,
                message: format!("expected module path segment, found {}", describe(&t.kind)),
            }),
        }
    }

    pub(crate) fn parse_module_path(&mut self) -> Result<Vec<Spanned<String>>, SyntaxError> {
        let mut segs = vec![self.parse_module_segment()?];
        while self.eat(&TokenKind::ColonColon).is_some() {
            segs.push(self.parse_module_segment()?);
        }
        Ok(segs)
    }

    pub(crate) fn parse_program_inner(&mut self) -> Result<Program, SyntaxError> {
        let mut module_docs = None;
        let mut config = None;
        let mut imports = Vec::new();
        let mut stmts = Vec::new();
        loop {
            self.skip_newlines();
            match self.peek().clone() {
                TokenKind::Eof => break,
                TokenKind::Doc { module, .. } => {
                    let docs = self.take_docs(module);
                    if module {
                        // `//!` is a module doc (spec §4.1) and is only valid at the very top of the file.
                        if module_docs.is_none()
                            && config.is_none()
                            && imports.is_empty()
                            && stmts.is_empty()
                        {
                            module_docs = docs;
                        } else if let Some(d) = docs {
                            self.push_warning(
                                "W0007",
                                d.span,
                                "module doc comment `//!` must appear at the top of the file, before `config`/`import` (W0007); spec §4.1".into(),
                            );
                        }
                    } else {
                        // `///` documents the following item; anything before `config`/`import`/a statement is a target.
                        match self.peek().clone() {
                            TokenKind::Eof | TokenKind::KwConfig => {
                                if let Some(d) = docs {
                                    self.push_warning(
                                        "W0007",
                                        d.span,
                                        "doc comment has no following definition (W0007); `///` must precede an item (spec §4.1)".into(),
                                    );
                                }
                            }
                            TokenKind::KwImport | TokenKind::KwFrom => {
                                if !stmts.is_empty() {
                                    return Err(self.err(
                                        self.span(),
                                        "`import` must appear before statements".into(),
                                    ));
                                }
                                let mut imp = self.parse_import()?;
                                imp.docs = docs;
                                imports.push(imp);
                            }
                            _ => stmts.push(self.parse_stmt(docs)?),
                        }
                    }
                }
                TokenKind::KwConfig => {
                    if config.is_some() {
                        return Err(self.err(self.span(), "duplicate `config` block".into()));
                    }
                    if !imports.is_empty() || !stmts.is_empty() {
                        return Err(self.err(
                            self.span(),
                            "`config` must appear before `import` and statements".into(),
                        ));
                    }
                    config = Some(self.parse_config_block()?);
                }
                TokenKind::KwImport | TokenKind::KwFrom => {
                    if !stmts.is_empty() {
                        return Err(
                            self.err(self.span(), "`import` must appear before statements".into())
                        );
                    }
                    imports.push(self.parse_import()?);
                }
                _ => {
                    stmts.push(self.parse_stmt(None)?);
                }
            }
        }
        Ok(Program {
            module_docs,
            config,
            imports,
            stmts,
        })
    }

    pub(crate) fn parse_config_block(&mut self) -> Result<ConfigBlock, SyntaxError> {
        self.skip_newlines();
        let start = self.expect(&TokenKind::KwConfig, "`config`")?.span;
        self.skip_newlines();
        let entries = self.parse_config_entries()?;
        let end = self.tokens[self.pos.saturating_sub(1)].span;
        Ok(ConfigBlock {
            entries,
            span: Span::merge(start, end),
        })
    }

    pub(crate) fn parse_config_entries(&mut self) -> Result<Vec<ConfigEntry>, SyntaxError> {
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
    pub(crate) fn parse_config_entry(&mut self) -> Result<ConfigEntry, SyntaxError> {
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
        Ok(ConfigEntry {
            name,
            type_ann,
            value,
            span,
        })
    }

    pub(crate) fn parse_config_value(&mut self) -> Result<Expr, SyntaxError> {
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
            Ok(Expr {
                kind: ExprKind::Custom(items),
                span: Span::merge(start, end),
            })
        } else {
            self.parse_expr()
        }
    }

    pub(crate) fn parse_import(&mut self) -> Result<Import, SyntaxError> {
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
        self.end_statement()?;
        let end = self.tokens[self.pos.saturating_sub(1)].span;
        Ok(Import {
            kind,
            docs: None,
            span: Span::merge(start, end),
        })
    }
}
