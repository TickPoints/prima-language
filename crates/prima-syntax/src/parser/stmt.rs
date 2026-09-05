//! Statement, top-level item, and type parsing (spec §4/§18.4/§18.5).
//!
//! This module owns statement/definition parsers (`let`/`const`/`fn`/`class`/`impl`/control flow), annotations, visibility, and type grammar; expressions are delegated to the sibling `expr` module.

use super::{Parser, stmt_span_of};
use crate::ast::*;
use crate::error::SyntaxError;
use crate::span::Span;
use crate::token::{TokenKind, describe};

impl Parser {
    pub(crate) fn parse_stmt(&mut self, docs: Option<DocComment>) -> Result<Stmt, SyntaxError> {
        self.skip_newlines();
        // Statement-level annotations (spec §18.4): `@builtin`/`@c_api::extern` before a `pub`/`fn`/`class` item.
        let annotations = self.parse_annotations()?;
        let stmt = match self.peek().clone() {
            TokenKind::KwLet => self.parse_let_stmt()?,
            TokenKind::KwConst => self.parse_const_stmt()?,
            TokenKind::KwFn => self.parse_fn_stmt(&annotations)?,
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
            TokenKind::KwPub => self.parse_pub_stmt(&annotations, &docs)?,
            _ => self.parse_expr_or_assign_stmt()?,
        };
        // `@builtin fn` / `@builtin pub fn` fold the statement-level annotations into the `FnDef`
        // during parsing (they drive path names and the signature-only body, spec §18.4); classes
        // and other annotated items still attach them here. A `pub` wraps the item, which has
        // already handled the annotations itself.
        let stmt = match &stmt {
            Stmt::FnDef { .. } | Stmt::Pub(_) => stmt,
            _ if !annotations.is_empty() => self.apply_annotations(stmt, &annotations)?,
            _ => stmt,
        };
        // Attach the doc comment to definition statements; anything else warns `W0007` (spec §4.1).
        // `pub`-wrapped items already consumed their docs inside `parse_pub_stmt`.
        let stmt = match &stmt {
            Stmt::Pub(_) => stmt,
            _ => self.attach_docs(stmt, docs)?,
        };
        // Statement terminator (spec §4.2): block-level statements may omit `;`; the rest require `;`.
        // A `pub`-wrapped item already enforced its own terminator via the recursive `parse_stmt`
        // (or `parse_fn_stmt`) inside `parse_pub_stmt`; only block-level kinds take the optional `;`.
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
            Stmt::Pub(inner) => match &**inner {
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
                _ => {}
            },
            _ => self.end_statement()?,
        }
        Ok(stmt)
    }

    /// Attach statement-level annotations to the definition they precede (spec §18.4).
    pub(crate) fn apply_annotations(
        &mut self,
        stmt: Stmt,
        anns: &[Annotation],
    ) -> Result<Stmt, SyntaxError> {
        match stmt {
            Stmt::FnDef {
                name,
                params,
                ret,
                mut annotations,
                body,
                span,
                docs,
            } => {
                annotations.extend_from_slice(anns);
                Ok(Stmt::FnDef {
                    name,
                    params,
                    ret,
                    annotations,
                    body,
                    span,
                    docs,
                })
            }
            Stmt::MathDef {
                name,
                params,
                ret,
                mut annotations,
                body,
                span,
                docs,
            } => {
                annotations.extend_from_slice(anns);
                Ok(Stmt::MathDef {
                    name,
                    params,
                    ret,
                    annotations,
                    body,
                    span,
                    docs,
                })
            }
            Stmt::ClassDef {
                name,
                mut annotations,
                mut members,
                span,
                docs,
            } => {
                annotations.extend_from_slice(anns);
                // A `@builtin` class carries the annotation on every method (signature-only bodies are the builtin form, spec §18.4).
                if let Some(Annotation::Builtin { opt_level }) =
                    anns.iter().find(|a| a.is_builtin())
                {
                    let level = *opt_level;
                    for m in &mut members {
                        if let ClassMemberKind::Method { annotations, .. } = &mut m.kind {
                            annotations.push(Annotation::Builtin { opt_level: level });
                        }
                    }
                }
                Ok(Stmt::ClassDef {
                    name,
                    annotations,
                    members,
                    span,
                    docs,
                })
            }
            Stmt::Pub(inner) => self
                .apply_annotations(*inner, anns)
                .map(Box::new)
                .map(Stmt::Pub),
            other => {
                let span = stmt_span_of(&other);
                Err(self.err(
                    span,
                    "annotations are only allowed on `fn`/`let` definitions and classes".into(),
                ))
            }
        }
    }

    /// Skip tokens up to the next statement boundary, so a removed-construct error still recovers cleanly (spec §2.2 sync tokens).
    pub(crate) fn skip_to_statement_boundary(&mut self) {
        while !matches!(
            self.peek(),
            TokenKind::Semicolon | TokenKind::RBrace | TokenKind::Eof | TokenKind::Newline
        ) {
            self.bump();
        }
        if matches!(self.peek(), TokenKind::Newline) {
            self.skip_newlines();
        }
    }

    pub(crate) fn parse_let_stmt(&mut self) -> Result<Stmt, SyntaxError> {
        let start = self.bump().span;
        self.skip_newlines();
        let mut_ = self.eat(&TokenKind::KwMut).is_some();
        // Math definition `let f(x) = expr` (spec §4.3): an identifier followed by `(`.
        let is_mathdef = matches!(self.peek(), TokenKind::Ident(_))
            && matches!(self.peek_at(1), TokenKind::LParen);
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
            return Ok(Stmt::MathDef {
                name,
                params,
                ret,
                annotations,
                body,
                span,
                docs: None,
            });
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
        Ok(Stmt::Let {
            pat,
            mut_,
            type_ann,
            value,
            span,
            docs: None,
        })
    }

    pub(crate) fn parse_const_stmt(&mut self) -> Result<Stmt, SyntaxError> {
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
        Ok(Stmt::Const {
            name,
            type_ann,
            value,
            span,
            docs: None,
        })
    }

    pub(crate) fn parse_fn_stmt(
        &mut self,
        stmt_annotations: &[Annotation],
    ) -> Result<Stmt, SyntaxError> {
        let start = self.bump().span;
        self.skip_newlines();
        let name = self.parse_ident("function name")?;
        // A `@builtin` fn carries an optional `::`-joined path name (`Matrix::zeros`, spec §18.4),
        // which is exported under that joined key for module-qualified calls. Only `@builtin` fns
        // accept the path form; a plain `fn a::b() {}` stays an error (`expected `(``).
        let name =
            if stmt_annotations.iter().any(|a| a.is_builtin()) && self.at(&TokenKind::ColonColon) {
                let mut joined = name.value;
                while self.eat(&TokenKind::ColonColon).is_some() {
                    self.skip_newlines();
                    let seg = self.parse_ident("`@builtin` function name segment")?;
                    joined.push_str("::");
                    joined.push_str(&seg.value);
                }
                Spanned {
                    value: joined,
                    span: name.span,
                }
            } else {
                name
            };
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
        let mut annotations = self.parse_annotations()?;
        // Fold in the statement-level annotations (`@builtin fn` / `@builtin pub fn`), so the
        // signature-only body decision and the binding step see one merged set (spec §18.4).
        for a in stmt_annotations {
            if !annotations.contains(a) {
                annotations.push(*a);
            }
        }
        // Signature-only `@builtin fn` (spec §18.4): no body — the implementation is the Rust host
        // builtin of the same name. A `@builtin` before the signature (statement level) or after it
        // both mark the signature-only form. The rule that tier `O0` must be signature-only and tier
        // `O1..O3` must have a `.pra` body is enforced by `check`/the evaluator (E0056); the parser only
        // decides whether the `{ ... }` body is present.
        let is_builtin = annotations.iter().any(|a| a.is_builtin());
        let body = if is_builtin && !self.at(&TokenKind::LBrace) {
            self.end_statement()?;
            Block {
                stmts: Vec::new(),
                span: start,
            }
        } else {
            self.parse_block()?
        };
        let span = Span::merge(start, body.span);
        Ok(Stmt::FnDef {
            name,
            params,
            ret,
            annotations,
            body,
            span,
            docs: None,
        })
    }

    pub(crate) fn parse_params(&mut self) -> Result<Vec<Param>, SyntaxError> {
        self.expect(&TokenKind::LParen, "`(`")?;
        let mut params = Vec::new();
        self.skip_newlines();
        if !self.at(&TokenKind::RParen) {
            loop {
                self.skip_newlines();
                // `self` receiver of a method (spec §4.5).
                if self.at(&TokenKind::KwSelf) {
                    let t = self.bump();
                    params.push(Param {
                        name: Spanned {
                            value: "self".into(),
                            span: t.span,
                        },
                        type_ann: None,
                        is_self: true,
                    });
                } else {
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

    pub(crate) fn parse_annotations(&mut self) -> Result<Vec<Annotation>, SyntaxError> {
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
                TokenKind::Ident(s) => {
                    match s.as_str() {
                        "parallel" => Annotation::Parallel,
                        "jit" => Annotation::Jit,
                        "gpu" => Annotation::Gpu,
                        "builtin" => {
                            // `@builtin(O0)`..`@builtin(O3)` (spec §18.4): an optional tier argument;
                            // bare `@builtin` is tier `O0`. An invalid tier is a compile error (E0057).
                            let mut opt_level = 0u8;
                            if self.eat(&TokenKind::LParen).is_some() {
                                self.skip_newlines();
                                let seg = self.parse_ident("optimization level")?;
                                let level_pat: [&str; 4] = ["O0", "O1", "O2", "O3"];
                                match level_pat.iter().position(|&l| l == seg.value) {
                                    Some(idx) => opt_level = idx as u8,
                                    None => return Err(self.err(
                                        seg.span,
                                        format!(
                                            "invalid `@builtin` optimization level `{}` (E0057)",
                                            seg.value
                                        ),
                                    )),
                                }
                                self.skip_newlines();
                                self.expect(&TokenKind::RParen, "`)`")?;
                            }
                            Annotation::Builtin { opt_level }
                        }
                        // `@c_api::extern` (spec §18.4).
                        "c_api" if self.eat(&TokenKind::ColonColon).is_some() => {
                            let seg = self.parse_ident("annotation segment")?;
                            if seg.value == "extern" {
                                Annotation::CApiExtern
                            } else {
                                return Err(self.err(
                                    seg.span,
                                    format!("unknown annotation `@c_api::{}`", seg.value),
                                ));
                            }
                        }
                        _ => return Err(self.err(t.span, format!("unknown annotation `@{s}`"))),
                    }
                }
                _ => return Err(self.err(t.span, "expected annotation name after `@`".into())),
            };
            anns.push(ann);
        }
        Ok(anns)
    }

    pub(crate) fn parse_class_def(&mut self) -> Result<Stmt, SyntaxError> {
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
            let member_docs = self.take_docs(false);
            if self.at(&TokenKind::RBrace) {
                // A trailing doc comment before `}` has no member to document.
                if let Some(d) = member_docs {
                    self.push_warning(
                        "W0007",
                        d.span,
                        "doc comment has no following definition (W0007); `///` must precede a field or method (spec §4.1)".into(),
                    );
                }
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
                    kind: ClassMemberKind::Method {
                        name: mname,
                        params,
                        ret,
                        annotations,
                        body,
                    },
                    span,
                    docs: member_docs,
                });
            } else {
                let fname = self.parse_ident("field name")?;
                self.skip_newlines();
                self.expect(&TokenKind::Colon, "`:`")?;
                let ty = self.parse_type()?;
                let end = self.tokens[self.pos.saturating_sub(1)].span;
                let span = Span::merge(member_start, end);
                members.push(ClassMember {
                    vis,
                    kind: ClassMemberKind::Field { name: fname, ty },
                    span,
                    docs: member_docs,
                });
            }
            self.skip_newlines();
            self.eat(&TokenKind::Comma); // members are comma-separated (spec §4.5)
        }
        let end = self.tokens[self.pos.saturating_sub(1)].span;
        Ok(Stmt::ClassDef {
            name,
            annotations: Vec::new(),
            members,
            span: Span::merge(start, end),
            docs: None,
        })
    }

    /// Visibility modifier (spec §15.2): none / `pub` / `pub(mod)`.
    pub(crate) fn parse_vis(&mut self) -> Result<Visibility, SyntaxError> {
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

    pub(crate) fn parse_impl_stmt(&mut self) -> Result<Stmt, SyntaxError> {
        let start = self.expect(&TokenKind::KwImpl, "`impl`")?.span;
        self.skip_newlines();
        // `impl ops::Add for Vec2 { ... }` (spec §18.5).
        let ns = self.parse_module_segment()?;
        if ns.value != "ops" {
            return Err(self.err(
                ns.span,
                "`impl` must target `ops` (e.g. `impl ops::Add for T`) (spec §18.5)".into(),
            ));
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
            other => {
                return Err(self.err(
                    op_seg.span,
                    format!("unknown operator overload `ops::{other}`"),
                ));
            }
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
            members.push(Box::new(Stmt::FnDef {
                name,
                params,
                ret,
                annotations,
                body,
                span,
                docs: None,
            }));
        }
        let end = self.tokens[self.pos.saturating_sub(1)].span;
        Ok(Stmt::Impl {
            op,
            target,
            members,
            span: Span::merge(start, end),
        })
    }

    pub(crate) fn parse_type(&mut self) -> Result<Type, SyntaxError> {
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
                        Ok(Type::MFn {
                            params,
                            ret: Box::new(ret),
                        })
                    } else {
                        Ok(Type::Fn {
                            params,
                            ret: Box::new(ret),
                        })
                    }
                }
                _ => {
                    let mut segs = vec![s];
                    while self.eat(&TokenKind::ColonColon).is_some() {
                        let seg = self.parse_module_segment()?;
                        segs.push(seg.value);
                    }
                    Ok(Type::User(Spanned {
                        value: segs.join("::"),
                        span: t.span,
                    }))
                }
            },
            _ => Err(self.err(
                t.span,
                format!("expected type, found {}", describe(&t.kind)),
            )),
        }
    }

    pub(crate) fn parse_type_list(&mut self) -> Result<Vec<Type>, SyntaxError> {
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

    pub(crate) fn parse_block(&mut self) -> Result<Block, SyntaxError> {
        self.skip_newlines();
        let start = self.expect(&TokenKind::LBrace, "`{`")?.span;
        let mut stmts = Vec::new();
        loop {
            self.skip_newlines();
            if self.at(&TokenKind::RBrace) {
                let end = self.bump().span;
                return Ok(Block {
                    stmts,
                    span: Span::merge(start, end),
                });
            }
            match self.peek().clone() {
                TokenKind::Doc { module, .. } => {
                    let docs = self.take_docs(module);
                    if module {
                        // `//!` is only valid at the top of a file (spec §4.1).
                        if let Some(d) = docs {
                            self.push_warning(
                                "W0007",
                                d.span,
                                "module doc comment `//!` is only allowed at the top of a file (W0007); spec §4.1".into(),
                            );
                        }
                    } else if matches!(self.peek_non_newline(), TokenKind::RBrace | TokenKind::Eof)
                    {
                        // A trailing doc comment before the block closes has no item to document.
                        if let Some(d) = docs {
                            self.push_warning(
                                "W0007",
                                d.span,
                                "doc comment has no following definition (W0007); `///` must precede an item (spec §4.1)".into(),
                            );
                        }
                    } else {
                        stmts.push(self.parse_stmt(docs)?);
                    }
                }
                _ => stmts.push(self.parse_stmt(None)?),
            }
        }
    }

    pub(crate) fn parse_while_stmt(&mut self) -> Result<Stmt, SyntaxError> {
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
            return Ok(Stmt::WhileLet {
                pat,
                value,
                body,
                span,
            });
        }
        let cond = self.parse_scrutinee()?;
        let body = self.parse_block()?;
        let span = Span::merge(start, body.span);
        Ok(Stmt::While { cond, body, span })
    }

    pub(crate) fn parse_if_stmt(&mut self) -> Result<Stmt, SyntaxError> {
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
            return Ok(Stmt::IfLet {
                pat,
                value,
                then,
                else_,
                span,
            });
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
        Ok(Stmt::If {
            cond,
            then,
            elifs,
            else_,
            span,
        })
    }

    /// Parses the body of `else if let` — returns a nested `Stmt::IfLet` wrapped in a single-statement block.
    pub(crate) fn parse_if_let_after_else(&mut self) -> Result<Block, SyntaxError> {
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
        let stmt = Stmt::IfLet {
            pat,
            value,
            then,
            else_,
            span,
        };
        Ok(Block {
            stmts: vec![stmt.clone()],
            span: stmt_span_of(&stmt),
        })
    }

    pub(crate) fn parse_return_stmt(&mut self) -> Result<Stmt, SyntaxError> {
        let start = self.bump().span;
        let value = if matches!(
            self.peek(),
            TokenKind::Newline | TokenKind::Semicolon | TokenKind::RBrace
        ) || self.at(&TokenKind::Eof)
        {
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

    pub(crate) fn parse_for_stmt(&mut self, is_par: bool) -> Result<Stmt, SyntaxError> {
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
            Ok(Stmt::ParFor {
                var,
                range,
                step,
                body,
                span,
            })
        } else {
            Ok(Stmt::For {
                var,
                range,
                step,
                body,
                span,
            })
        }
    }

    pub(crate) fn parse_with_stmt(&mut self) -> Result<Stmt, SyntaxError> {
        let start = self.bump().span;
        self.skip_newlines();
        self.expect(&TokenKind::KwConfig, "`config`")?;
        self.skip_newlines();
        let entries = self.parse_config_entries()?;
        let body = self.parse_block()?;
        let span = Span::merge(start, body.span);
        Ok(Stmt::WithConfig {
            entries,
            body,
            span,
        })
    }

    pub(crate) fn parse_pub_stmt(
        &mut self,
        outer_annotations: &[Annotation],
        docs: &Option<DocComment>,
    ) -> Result<Stmt, SyntaxError> {
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
            // `fn` folds in the outer statement-level annotations (e.g. `@builtin pub fn`) during
            // parsing, since the signature-only body and `::`-joined names depend on them (spec §18.4).
            TokenKind::KwFn => {
                let stmt = self.parse_fn_stmt(outer_annotations)?;
                self.attach_docs(stmt, docs.clone())?
            }
            TokenKind::KwLet | TokenKind::KwConst | TokenKind::KwClass => {
                let stmt = self.parse_stmt(None)?;
                let stmt = if outer_annotations.is_empty() {
                    stmt
                } else {
                    self.apply_annotations(stmt, outer_annotations)?
                };
                self.attach_docs(stmt, docs.clone())?
            }
            _ => {
                return Err(self.err(
                    self.span(),
                    "expected `let`, `const`, `fn`, or `class` after `pub`".into(),
                ));
            }
        };
        let _ = start;
        Ok(Stmt::Pub(Box::new(inner)))
    }

    pub(crate) fn parse_match_stmt(&mut self) -> Result<Stmt, SyntaxError> {
        let start = self.expect(&TokenKind::KwMatch, "`match`")?.span;
        self.skip_newlines();
        let scrutinee = self.parse_scrutinee()?;
        let arms = self.parse_match_arms()?;
        let end = self.tokens[self.pos.saturating_sub(1)].span;
        Ok(Stmt::Match {
            scrutinee,
            arms,
            span: Span::merge(start, end),
        })
    }

    pub(crate) fn parse_expr_or_assign_stmt(&mut self) -> Result<Stmt, SyntaxError> {
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
            Ok(Stmt::Assign {
                target: lhs,
                op,
                value,
                span,
            })
        } else {
            Ok(Stmt::Expr(lhs))
        }
    }
}
