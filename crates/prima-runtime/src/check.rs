//! Static source checking (spec §6.3 / §16.2 compile-time errors): collect type errors from a
//! program. The walkers live in child modules (`signature`/`error`/`collect`/`infer`).

use prima_syntax::Span;
use prima_syntax::parse;

mod collect;
mod error;
mod infer;
mod signature;

/// A statically decidable type error (located via `--> file:line:col` per spec §16.4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeError {
    pub line: usize,
    pub column: usize,
    /// Source span of the offending value, for caret rendering (spec §16.4).
    pub span: Span,
    pub message: String,
    /// Diagnostic notes attached to the error (spec §16.4), e.g. the stdlib `@builtin` signature
    /// and definition location for `E0050` call-site violations.
    pub notes: Vec<String>,
}

/// Static-check context: whether `?` (spec §16.3 `E0054`) is allowed — i.e. inside a `fn`/method
/// whose return type is `Result<..>`/`Option<..>`.
#[derive(Debug, Clone, Copy)]
struct Ctx {
    allow_try: bool,
}

/// Statically check source code (spec §6.3 / §16.2 compile-time errors): return all type errors (collecting, not fail-fast).
pub fn check_src(src: &str) -> Vec<TypeError> {
    let program = match parse(src) {
        Ok(program) => program,
        Err(errors) => {
            return errors
                .iter()
                .map(|e| {
                    let (line, column) = line_col(src, e.span.start);
                    TypeError {
                        line,
                        column,
                        span: e.span,
                        message: e.message.clone(),
                        notes: Vec::new(),
                    }
                })
                .collect();
        }
    };

    let sigs = signature::build_signature_table(&program);
    let mut errors = Vec::new();
    let ctx = Ctx { allow_try: false };
    for stmt in &program.stmts {
        collect::collect_stmt_errors(src, stmt, &mut errors, ctx, false, &sigs);
    }
    // Statement order is source order; sorting stably by (line, column) keeps it consistent with span.start.
    errors.sort_by_key(|e| (e.line, e.column));
    errors
}

/// Byte offset → 1-based line/column (column counted in characters, spec §16.4 location).
pub(crate) fn line_col(src: &str, offset: u32) -> (usize, usize) {
    let offset = usize::try_from(offset).unwrap_or(usize::MAX).min(src.len());
    let before = &src[..offset];
    let line = before.bytes().filter(|&b| b == b'\n').count() + 1;
    let line_start = before.rfind('\n').map_or(0, |i| i + 1);
    let column = src[line_start..offset].chars().count() + 1;
    (line, column)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use prima_syntax::Span;
    use prima_syntax::ast::{Spanned, Stmt, Type};
    use prima_syntax::parse;

    use super::check_src;
    use super::infer::{assignable, infer};

    /// Embedded signature module with a `///`-documented `inverse`, for the `E0050` note test.
    /// The module path is unique to this test binary and registration is idempotent.
    const DOC_SRC: &str = "\
/// Compute the inverse of a square matrix.
@builtin pub fn inverse(M: Matrix<F64>) -> Matrix<F64>;
";

    #[test]
    fn f64_annotation_rejects_symbolic_value() {
        let errs = check_src("let x: F64 = sqrt(2);");
        assert_eq!(errs.len(), 1);
        assert!(errs[0].message.contains("F64"));
        assert!(errs[0].message.contains("Expr"));
    }

    #[test]
    fn e0050_error_carries_definition_note() {
        crate::stdlib::register_module_source("checkdoc", DOC_SRC);
        let errs = check_src("import checkdoc; let x = checkdoc::inverse(1);");
        assert_eq!(errs.len(), 1, "expected one error, got {errs:?}");
        assert!(errs[0].message.contains("E0050"), "got: {errs:?}");
        let notes = &errs[0].notes;
        assert!(
            notes.iter().any(|n| n.contains("checkdoc.pra:2:")),
            "note should mention the definition location, notes: {notes:?}"
        );
        assert!(
            notes
                .iter()
                .any(|n| n.contains("inverse(Matrix<F64>) -> Matrix<F64>")),
            "note should carry the rendered signature, notes: {notes:?}"
        );
        assert!(
            notes
                .iter()
                .any(|n| n.contains("Compute the inverse of a square matrix.")),
            "note should carry the `///` doc text, notes: {notes:?}"
        );
    }

    #[test]
    fn explicit_conversion_satisfies_annotation() {
        assert!(check_src("let y: F64 = to_f64(sqrt(2));").is_empty());
    }

    #[test]
    fn integer_annotation_rejects_float() {
        let errs = check_src("let z: Integer = 3.14;");
        assert_eq!(errs.len(), 1);
        assert!(errs[0].message.contains("Integer"));
        assert!(errs[0].message.contains("F64"));
    }

    #[test]
    fn string_annotation_rejects_integer() {
        let errs = check_src("let s: String = 5;");
        assert_eq!(errs.len(), 1);
        assert!(errs[0].message.contains("String"));
        assert!(errs[0].message.contains("Integer"));
    }

    #[test]
    fn syntax_error_surfaces_as_type_error() {
        let errs = check_src("let x: =");
        assert_eq!(errs.len(), 1);
        assert!(!errs[0].message.is_empty());
    }

    #[test]
    fn promotion_is_allowed() {
        assert!(check_src("let n: Integer = 7; let r: F64 = 1;").is_empty());
    }

    #[test]
    fn errors_are_reported_in_source_order() {
        let errs = check_src("let a: String = 1;\nlet b: Integer = 2.5;\n");
        assert_eq!(errs.len(), 2);
        assert!(errs[0].line < errs[1].line);
    }

    #[test]
    fn try_operator_rejected_outside_result_fn() {
        let errs = check_src("let x = try_f64(\"a\")?;");
        assert_eq!(errs.len(), 1);
        assert!(errs[0].message.contains("E0054"));
    }

    #[test]
    fn try_operator_allowed_in_result_fn() {
        assert!(
            check_src(
                "fn f() -> Result<F64, Error> {\n    let v = try_f64(\"a\")?;\n    return Ok(v);\n}"
            )
            .is_empty()
        );
    }

    #[test]
    fn refutable_pattern_in_let_is_flagged() {
        let errs = check_src("let 0 = x;");
        assert_eq!(errs.len(), 1);
        assert!(errs[0].message.contains("E0053"));
    }

    #[test]
    fn collapse_targets_infer_fixed_width_types() {
        assert!(check_src("let a: I8 = to_i8(7); let b: Usize = to_usize(3); let c: Option<Integer> = get([1], 0);").is_empty());
    }

    /// Infer the type of a bare expression statement (spec §6.3).
    fn inf_of(src: &str) -> String {
        let p = parse(src).unwrap();
        let Stmt::Expr(e) = &p.stmts[0] else {
            panic!("expected an expression statement, got {:?}", p.stmts[0]);
        };
        infer(e, &HashMap::new())
    }

    #[test]
    fn infer_literal_and_collection_types() {
        assert_eq!(inf_of("1"), "Integer");
        assert_eq!(inf_of("1.5"), "F64");
        assert_eq!(inf_of("0x1F"), "Integer");
        assert_eq!(inf_of("\"s\""), "String");
        assert_eq!(inf_of("true"), "Bool");
        assert_eq!(inf_of("[1, 2, 3]"), "Array<Integer>");
        assert_eq!(inf_of("[1.0, 2.0]"), "Array<F64>");
        assert_eq!(inf_of("[[1, 2], [3, 4]]"), "Array<Array<Integer>>");
        assert_eq!(inf_of("[sqrt(2), sqrt(3)]"), "Array<Expr>");
        assert_eq!(inf_of("[my_unknown_f(), my_other()]"), "Array<Expr>");
        assert_eq!(inf_of("{ \"a\": 1 }"), "dict");
        assert_eq!(inf_of("{1, 2}"), "set");
        assert_eq!(inf_of("(1, \"a\")"), "tuple");
        assert_eq!(inf_of("-5"), "Integer");
        assert_eq!(inf_of("!true"), "Bool");
    }

    #[test]
    fn assignable_promotes_numeric_layers() {
        assert!(assignable(&Type::F64, "Integer"));
        assert!(assignable(&Type::F64, "Rational"));
        assert!(assignable(&Type::F64, "F64"));
        assert!(assignable(&Type::Integer, "Integer"));
        assert!(assignable(&Type::Rational, "Integer"));
        assert!(assignable(&Type::Number, "Complex"));
        assert!(!assignable(&Type::Integer, "F64"));
        assert!(!assignable(&Type::String, "Integer"));
        assert!(!assignable(&Type::Bool, "Integer"));
    }

    #[test]
    fn assignable_wildcards_and_collections() {
        let value = Type::User(Spanned {
            value: "Value".into(),
            span: Span::new(0, 0),
        });
        assert!(assignable(&value, "anything"));
        assert!(assignable(&value, "unknown"));
        assert!(assignable(&Type::String, "unknown"));
        assert!(assignable(&Type::Array(Box::new(Type::F64)), "array"));
        assert!(assignable(
            &Type::Array(Box::new(Type::F64)),
            "Array<Integer>"
        ));
        assert!(!assignable(
            &Type::Array(Box::new(Type::String)),
            "Array<Integer>"
        ));
        assert!(assignable(
            &Type::Matrix(Box::new(Type::F64)),
            "Array<Array<Integer>>"
        ));
        assert!(!assignable(
            &Type::Matrix(Box::new(Type::F64)),
            "Array<Integer>"
        ));
        assert!(assignable(&Type::Option(Box::new(Type::Integer)), "option"));
    }
}
