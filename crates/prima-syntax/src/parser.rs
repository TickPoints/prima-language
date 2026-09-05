use crate::ast::*;
use crate::error::{SyntaxError, SyntaxWarning};
use crate::lexer::lex;
use crate::span::Span;
use crate::token::{Token, TokenKind};

// Unary operator binding power: lower than power `^` (8), higher than mul/div (6/7), implementing `-x^2 == -(x^2)` (same as Julia, spec §2.2).
const UNARY_BP: u8 = 7;

/// Hand-written recursive-descent + Pratt climbing parser (implementation plan §2.2), covering all appendix A BNF productions.
pub fn parse(src: &str) -> Result<Program, Vec<SyntaxError>> {
    let (program, errors, _) = parse_checked(src);
    if errors.is_empty() {
        Ok(program)
    } else {
        Err(errors)
    }
}

/// Parse and return the program plus all collected errors and warnings (spec §16.4/§16.5).
pub fn parse_checked(src: &str) -> (Program, Vec<SyntaxError>, Vec<SyntaxWarning>) {
    let tokens = match lex(src) {
        Ok(t) => t,
        Err(errors) => {
            return (
                Program {
                    module_docs: None,
                    config: None,
                    imports: Vec::new(),
                    stmts: Vec::new(),
                },
                errors,
                Vec::new(),
            );
        }
    };
    let mut parser = Parser::new(tokens);
    match parser.parse_program_inner() {
        Ok(program) => (program, Vec::new(), parser.warnings),
        Err(e) => (
            Program {
                module_docs: None,
                config: None,
                imports: Vec::new(),
                stmts: Vec::new(),
            },
            vec![e],
            parser.warnings,
        ),
    }
}

// Parser: errors use panic-mode recovery with sync tokens (`;`, `}`, `)`, end of file), collecting all syntax errors in one compilation (spec §2.2).
pub(crate) struct Parser {
    tokens: Vec<Token>,
    pos: usize,
    /// Collected warnings (spec §16.5). Parse-time warnings are currently not produced.
    warnings: Vec<SyntaxWarning>,
    /// Disables struct-literal parsing in control-flow conditions (`if x {` must stay a block, not `x { ... }`).
    no_struct_literal: bool,
}

impl Parser {
    pub(crate) fn new(tokens: Vec<Token>) -> Parser {
        Parser {
            tokens,
            pos: 0,
            warnings: Vec::new(),
            no_struct_literal: false,
        }
    }
}

// The `parse`/`parse_checked` entry points above drive the parser through `Parser`, whose methods are
// split across the focused submodules below (mirroring the `Evaluator` split in `prima-runtime`).
mod expr;
mod module;
mod pattern;
mod stmt;

fn binop_bp(kind: &TokenKind) -> Option<(BinOp, u8, u8)> {
    let (op, lbp, rbp) = match kind {
        TokenKind::PipePipe => (BinOp::Or, 2, 3),
        TokenKind::AmpAmp => (BinOp::And, 3, 4),
        TokenKind::EqEq => (BinOp::Eq, 4, 5),
        TokenKind::BangEq => (BinOp::Ne, 4, 5),
        TokenKind::Lt => (BinOp::Lt, 4, 5),
        TokenKind::LtEq => (BinOp::Le, 4, 5),
        TokenKind::Gt => (BinOp::Gt, 4, 5),
        TokenKind::GtEq => (BinOp::Ge, 4, 5),
        TokenKind::KwIn => (BinOp::In, 4, 5),
        TokenKind::Union => (BinOp::Union, 5, 6),
        TokenKind::SetMinus => (BinOp::Difference, 5, 6),
        TokenKind::Intersect => (BinOp::Intersect, 6, 7),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_first(src: &str) -> Expr {
        let program = crate::parse(src).expect("parse failed");
        let stmt = program
            .stmts
            .into_iter()
            .next()
            .expect("expected a statement");
        match stmt {
            Stmt::Expr(e) => e,
            Stmt::Let { value, .. } => value,
            other => panic!("unexpected statement: {other:?}"),
        }
    }

    fn parse_err(src: &str) -> bool {
        crate::parse(src).is_err()
    }

    fn binop(src: &str) -> (BinOp, Expr, Expr) {
        match parse_first(src).kind {
            ExprKind::Binary { op, lhs, rhs } => (op, *lhs, *rhs),
            other => panic!("expected binary expr, got {other:?}"),
        }
    }

    fn comp(src: &str) -> (CompKind, Expr, Vec<ComprehensionClause>) {
        match parse_first(src).kind {
            ExprKind::Comprehension {
                kind,
                output,
                clauses,
            } => (kind, *output, clauses),
            other => panic!("expected comprehension, got {other:?}"),
        }
    }

    fn clause_names(clauses: &[ComprehensionClause]) -> Vec<String> {
        clauses
            .iter()
            .map(|c| match c {
                ComprehensionClause::For { var, .. } => format!("for {}", var.value),
                ComprehensionClause::If { .. } => "if".into(),
            })
            .collect()
    }

    #[test]
    fn set_literal() {
        match parse_first("{1, 2, 3, 2}").kind {
            ExprKind::Set(elems) => {
                assert_eq!(
                    elems.len(),
                    4,
                    "parser keeps duplicates; dedup is a runtime concern"
                );
                assert!(matches!(
                    elems[0].kind,
                    ExprKind::Literal(Literal::Integer(_))
                ));
            }
            other => panic!("expected Set, got {other:?}"),
        }
    }

    #[test]
    fn dict_literal() {
        match parse_first("{ \"a\": 1, \"b\": 2 }").kind {
            ExprKind::Dict(entries) => {
                assert_eq!(entries.len(), 2);
                assert!(
                    matches!(entries[0].0.kind, ExprKind::Literal(Literal::String { ref value, .. }) if value == "a")
                );
                assert!(
                    matches!(entries[1].0.kind, ExprKind::Literal(Literal::String { ref value, .. }) if value == "b")
                );
                assert!(matches!(
                    entries[0].1.kind,
                    ExprKind::Literal(Literal::Integer(_))
                ));
            }
            other => panic!("expected Dict, got {other:?}"),
        }
    }

    #[test]
    fn empty_braces_are_empty_dict() {
        match parse_first("{}").kind {
            ExprKind::Dict(entries) => assert!(entries.is_empty()),
            other => panic!("expected empty Dict, got {other:?}"),
        }
    }

    #[test]
    fn array_comprehension() {
        let (kind, output, clauses) = comp("[x^2 for x in range(0, 10)]");
        assert_eq!(kind, CompKind::Array);
        assert!(matches!(
            output.kind,
            ExprKind::Binary { op: BinOp::Pow, .. }
        ));
        assert_eq!(clause_names(&clauses), vec!["for x"]);
    }

    #[test]
    fn array_comprehension_with_filter() {
        let (kind, output, clauses) = comp("[x for x in range(0, 10) if x % 2 == 0]");
        assert_eq!(kind, CompKind::Array);
        assert!(matches!(output.kind, ExprKind::Path { .. }));
        assert_eq!(clause_names(&clauses), vec!["for x", "if"]);
        match &clauses[1] {
            ComprehensionClause::If { cond } => {
                assert!(matches!(cond.kind, ExprKind::Binary { op: BinOp::Eq, .. }));
            }
            other => panic!("expected If clause, got {other:?}"),
        }
    }

    #[test]
    fn array_comprehension_nested_for() {
        let (kind, _, clauses) = comp("[(x, y) for x in range(0, 2) for y in range(0, 2)]");
        assert_eq!(kind, CompKind::Array);
        assert_eq!(clause_names(&clauses), vec!["for x", "for y"]);
        match &clauses[0] {
            ComprehensionClause::For { iter, .. } => {
                assert!(matches!(iter.kind, ExprKind::Call { .. }));
            }
            other => panic!("expected For clause, got {other:?}"),
        }
    }

    #[test]
    fn dict_comprehension() {
        let (kind, output, clauses) = comp("{x: x^2 for x in range(0, 5)}");
        assert_eq!(kind, CompKind::Dict);
        assert_eq!(clause_names(&clauses), vec!["for x"]);
        match output.kind {
            ExprKind::KeyValue { key, value } => {
                assert!(matches!(key.kind, ExprKind::Path { .. }));
                assert!(matches!(
                    value.kind,
                    ExprKind::Binary { op: BinOp::Pow, .. }
                ));
            }
            other => panic!("expected KeyValue output, got {other:?}"),
        }
    }

    #[test]
    fn set_comprehension() {
        let (kind, _, clauses) = comp("{x for x in range(0, 10) if x % 2 == 1}");
        assert_eq!(kind, CompKind::Set);
        assert_eq!(clause_names(&clauses), vec!["for x", "if"]);
    }

    #[test]
    fn tuple_comprehension() {
        let (kind, output, clauses) = comp("((x, x+1) for x in range(0, 3))");
        assert_eq!(kind, CompKind::Tuple);
        assert_eq!(clause_names(&clauses), vec!["for x"]);
        assert!(matches!(output.kind, ExprKind::Tuple(items) if items.len() == 2));
    }

    #[test]
    fn in_binop() {
        let (op, lhs, _) = binop("2 in c");
        assert_eq!(op, BinOp::In);
        assert!(matches!(lhs.kind, ExprKind::Literal(Literal::Integer(_))));
        let (op, _, _) = binop("5 in c");
        assert_eq!(op, BinOp::In);
    }

    #[test]
    fn set_algebra_operators() {
        let (op, _, rhs) = binop("s ∪ {5, 6}");
        assert_eq!(op, BinOp::Union);
        assert!(matches!(rhs.kind, ExprKind::Set(_)));
        let (op, _, _) = binop("s ∩ {2, 3}");
        assert_eq!(op, BinOp::Intersect);
        let (op, _, rhs) = binop("s \\ {3}");
        assert_eq!(op, BinOp::Difference);
        assert!(matches!(rhs.kind, ExprKind::Set(_)));
    }

    #[test]
    fn if_cond_with_set_literal() {
        let program = crate::parse("if x in {1, 2} { }").expect("parse failed");
        let stmt = program
            .stmts
            .into_iter()
            .next()
            .expect("expected an if statement");
        match stmt {
            Stmt::If { cond, .. } => match cond.kind {
                ExprKind::Binary {
                    op: BinOp::In, rhs, ..
                } => {
                    assert!(matches!(rhs.kind, ExprKind::Set(_)));
                }
                other => panic!("expected `x in {{{{1, 2}}}}` condition, got {other:?}"),
            },
            other => panic!("expected If statement, got {other:?}"),
        }
    }

    #[test]
    fn dict_literal_as_let_value() {
        let e = parse_first("let d = { \"a\": 1 };");
        assert!(matches!(e.kind, ExprKind::Dict(entries) if entries.len() == 1));
    }

    #[test]
    fn struct_literal_untouched() {
        let e = parse_first("let p = Point { x: 1, y: 2 };");
        assert!(matches!(e.kind, ExprKind::StructLiteral { .. }));
    }

    #[test]
    fn negative_multi_output_comprehension() {
        assert!(parse_err("[a, b for x in y]"));
    }

    #[test]
    fn negative_dict_literal_missing_colon() {
        assert!(parse_err("{1: 2, 3}"));
    }

    #[test]
    fn negative_comprehension_missing_in() {
        assert!(parse_err("[x for 10]"));
    }

    #[test]
    fn comprehension_single_for_no_filter_is_fine() {
        let (kind, _, clauses) = comp("[x for x in 10]");
        assert_eq!(kind, CompKind::Array);
        assert_eq!(clause_names(&clauses), vec!["for x"]);
    }

    #[test]
    fn negative_set_literal_with_colon() {
        assert!(parse_err("{1, 2: 3}"));
    }

    #[test]
    fn negative_newline_separated_statements() {
        // Spec §4.2: statements must be separated by `;`; a bare newline separator is E0011.
        assert!(
            parse_err("x = 1\ny = 2"),
            "newline-separated statements must be a parse error"
        );
        let errs = crate::parse("x = 1\ny = 2").unwrap_err();
        assert!(errs.iter().any(|e| {
            e.message.contains("E0011")
                && e.message
                    .contains("newline statement separation was removed")
        }));
        let errs = crate::parse("1\n2\n").unwrap_err();
        assert!(errs.iter().any(|e| e.message.contains("E0011")));
    }

    #[test]
    fn negative_pipeline_operator_removed() {
        // Spec §9.7: `|>` was removed in v2.3; its use is E0010.
        assert!(parse_err("a |> f"), "`|>` must be a parse error");
        let errs = crate::parse("a |> f").unwrap_err();
        assert!(
            errs.iter()
                .any(|e| e.message.contains("E0010") && e.message.contains("pipeline was removed"))
        );
        assert!(parse_err("let x = a |> f;"));
    }

    #[test]
    fn builtin_fn_accepts_path_name() {
        // Spec §18.4: a signature-only `@builtin pub fn` may carry a `::`-joined name, exported
        // under that joined key for module-qualified calls (`linalg::Matrix::zeros`).
        let program = crate::parse(
            "@builtin pub fn Matrix::zeros(rows: Integer, cols: Integer) -> Matrix<F64>;",
        )
        .expect("parse failed");
        let stmt = program
            .stmts
            .into_iter()
            .next()
            .expect("expected a statement");
        match stmt {
            Stmt::Pub(inner) => match *inner {
                Stmt::FnDef {
                    name,
                    params,
                    annotations,
                    body,
                    ret,
                    ..
                } => {
                    assert_eq!(name.value, "Matrix::zeros");
                    assert!(annotations.iter().any(|a| a.is_builtin()));
                    assert_eq!(params.len(), 2);
                    assert!(matches!(ret, Some(Type::Matrix(_))));
                    assert!(
                        body.stmts.is_empty(),
                        "signature-only builtin must have an empty body"
                    );
                }
                other => panic!("expected FnDef, got {other:?}"),
            },
            other => panic!("expected Pub, got {other:?}"),
        }
    }

    #[test]
    fn builtin_fn_accepts_path_name_without_pub() {
        let program =
            crate::parse("@builtin fn Util::twice(x: Integer) -> Integer;").expect("parse failed");
        let stmt = program
            .stmts
            .into_iter()
            .next()
            .expect("expected a statement");
        match stmt {
            Stmt::FnDef {
                name,
                annotations,
                body,
                ..
            } => {
                assert_eq!(name.value, "Util::twice");
                assert!(annotations.iter().any(|a| a.is_builtin()));
                assert!(body.stmts.is_empty());
            }
            other => panic!("expected FnDef, got {other:?}"),
        }
    }

    #[test]
    fn non_builtin_fn_rejects_path_name() {
        assert!(
            parse_err("fn a::b() {}"),
            "path names are only allowed on `@builtin` fns"
        );
    }

    #[test]
    fn builtin_tier_annotation_parses() {
        // `@builtin(O2)` carries its tier (spec §18.4); bare `@builtin` is tier `O0`.
        let p1 = crate::parse("@builtin(O2)\npub fn scale(a: Integer) -> Integer;")
            .expect("parse failed");
        let mut stmt = p1.stmts.into_iter().next().unwrap();
        if let Stmt::Pub(inner) = stmt {
            stmt = *inner;
        }
        let Stmt::FnDef {
            annotations, body, ..
        } = stmt
        else {
            panic!("expected FnDef")
        };
        assert_eq!(annotations.iter().map(|a| a.builtin_level()).max(), Some(2));
        assert!(body.stmts.is_empty());

        let p0 = crate::parse("@builtin\npub fn identity(a: Integer) -> Integer;")
            .expect("parse failed");
        let mut stmt0 = p0.stmts.into_iter().next().unwrap();
        if let Stmt::Pub(inner) = stmt0 {
            stmt0 = *inner;
        }
        let Stmt::FnDef { annotations, .. } = stmt0 else {
            panic!("expected FnDef")
        };
        assert_eq!(annotations.iter().map(|a| a.builtin_level()).max(), Some(0));

        assert!(
            parse_err("@builtin(O9) pub fn f() -> Integer;"),
            "invalid tier is E0057"
        );
    }

    #[test]
    fn fstring_parses_into_parts() {
        // `f"a{x} b{ y + 1 :0.2}"` → literal/interp/literal/interp (spec §18.1).
        let e = parse_first(r#"f"a{x} b{ y + 1 :0.2}""#);
        match e.kind {
            ExprKind::FString(parts) => {
                assert_eq!(parts.len(), 4);
                assert_eq!(parts[0], FStringPart::Literal("a".into()));
                match &parts[1] {
                    FStringPart::Interp { expr, spec } => {
                        assert_eq!(spec, &None);
                        assert!(matches!(expr.kind, ExprKind::Path { .. }));
                    }
                    other => panic!("expected interpolation, got {other:?}"),
                }
                assert_eq!(parts[2], FStringPart::Literal(" b".into()));
                match &parts[3] {
                    FStringPart::Interp { expr, spec } => {
                        assert_eq!(spec.as_deref(), Some("0.2"));
                        assert!(matches!(expr.kind, ExprKind::Binary { .. }));
                    }
                    other => panic!("expected interpolation, got {other:?}"),
                }
            }
            other => panic!("expected FString, got {other:?}"),
        }
    }

    #[test]
    fn fstring_interpolation_allows_dict_and_strings() {
        // Nested braces/strings inside the interpolation must not break the `{}` balance.
        // `r##"..."##` so the f-string's closing quote is part of the content.
        let e = parse_first(r##"f"d = { {"a": 1}["a"] } s = {"hi"}""##);
        match e.kind {
            ExprKind::FString(parts) => {
                assert_eq!(parts.len(), 4);
                let FStringPart::Interp { expr, .. } = &parts[1] else {
                    panic!("expected interpolation")
                };
                assert!(matches!(expr.kind, ExprKind::Index { .. }));
            }
            other => panic!("expected FString, got {other:?}"),
        }
    }

    #[test]
    fn fstring_empty_interpolation_is_error() {
        assert!(
            parse_err(r#"f"a {} b""#),
            "empty interpolation must be a parse error"
        );
    }

    #[test]
    fn format_call_emits_w0006_warning() {
        let (_, errors, warnings) = crate::parse_checked(r#"print(format("x = {}", 1));"#);
        assert!(errors.is_empty(), "errors = {errors:?}");
        assert!(
            warnings
                .iter()
                .any(|w| w.code == "W0006" && w.message.contains("f-string")),
            "warnings = {warnings:?}"
        );
    }

    #[test]
    fn format_module_function_is_not_warned() {
        // `time::format` is a module function (spec §18.1), not the removed core `format`.
        let (_, errors, warnings) = crate::parse_checked(r#"time::format(0, "%Y");"#);
        assert!(errors.is_empty(), "errors = {errors:?}");
        assert!(
            !warnings.iter().any(|w| w.code == "W0006"),
            "warnings = {warnings:?}"
        );
    }

    #[test]
    fn fstring_with_nested_literal_is_compile_error() {
        let errs = crate::parse(r#"f"a { f"b" } c""#).unwrap_err();
        assert!(errs.iter().any(|e| e.message.contains("nested f-string")));
    }

    #[test]
    fn builtin_pub_fn_signature_only_without_path() {
        // `@builtin pub fn` (annotation before `pub`) must also parse the signature-only form.
        let program = crate::parse("@builtin pub fn answer() -> Integer;").expect("parse failed");
        match program
            .stmts
            .into_iter()
            .next()
            .expect("expected a statement")
        {
            Stmt::Pub(inner) => match *inner {
                Stmt::FnDef {
                    name,
                    annotations,
                    body,
                    ..
                } => {
                    assert_eq!(name.value, "answer");
                    assert!(annotations.iter().any(|a| a.is_builtin()));
                    assert!(body.stmts.is_empty());
                }
                other => panic!("expected FnDef, got {other:?}"),
            },
            other => panic!("expected Pub, got {other:?}"),
        }
    }
}
