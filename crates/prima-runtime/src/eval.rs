use std::cell::RefCell;
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::Ordering as AtomicOrdering;

use num_bigint::BigInt;
use prima_core::simplify::simplify;
use prima_core::{
    BuiltinSymbols, ExprData, ExprId, ExprPool, Number, Real, SymbolId, SymbolTable, Value,
    ValueKey,
};
use prima_syntax::ast::{
    Annotation, AssignOp, BinOp, Block, ClassMemberKind, CompKind, ComprehensionClause, DocComment,
    Expr, ExprKind, FStringPart, FieldValue, ImplOp, IndexItem, Literal, MatchArm, Param, Pattern,
    Spanned, Stmt, UnOp, Visibility,
};
use prima_syntax::{Span, SyntaxWarning};
use rayon::prelude::*;

use crate::builtins::Builtin;
use crate::class::{ClassDef, ClassInstance, FieldDef, MethodDef, MethodNature};
use crate::config::{Config, Domain, OptLevel, OverloadPolicy, UndefinedHandling};
use crate::error::RuntimeError;

mod apply;
mod builtin;
mod call;
mod class;
mod entry;
mod env;
mod expr;
mod helpers;
mod pattern;
mod stmt;
pub use helpers::value_type_name;
pub(crate) use helpers::{stmt_span, syntax_err};

use env::BuiltinBackend;
pub(crate) use env::BuiltinBackend as EvalBackend;
pub use env::{Env, EnvRef, Function, HotState, JIT_CALL_THRESHOLD, NamespaceItem, NativeCall};

/// The `core` builtins pre-imported into the root environment (spec §15.5), in declaration order.
pub(crate) const CORE_BUILTIN_NAMES: &[&str] = &[
    "print",
    "println",
    "input",
    "read_line",
    "simplify",
    "derivative",
    "partial",
    "grad",
    "limit",
    "jit",
    "range",
    "sqrt",
    "exp",
    "log",
    "ln",
    "sin",
    "cos",
    "tan",
    "abs",
    "to_i8",
    "to_i16",
    "to_i32",
    "to_i64",
    "to_i128",
    "to_u8",
    "to_u16",
    "to_u32",
    "to_u64",
    "to_u128",
    "to_isize",
    "to_usize",
    "to_f32",
    "to_f64",
    "to_bigint",
    "to_rational",
    "to_bigfloat",
    "to_complex",
    "try_i8",
    "try_i16",
    "try_i32",
    "try_i64",
    "try_i128",
    "try_u8",
    "try_u16",
    "try_u32",
    "try_u64",
    "try_u128",
    "try_isize",
    "try_usize",
    "try_f32",
    "try_f64",
    "try_bigint",
    "try_rational",
    "try_complex",
    "checked_i8",
    "checked_i16",
    "checked_i32",
    "checked_i64",
    "checked_i128",
    "checked_u8",
    "checked_u16",
    "checked_u32",
    "checked_u64",
    "checked_u128",
    "checked_add",
    "checked_mul",
    "clamped_i8",
    "clamped_i16",
    "clamped_i32",
    "clamped_i64",
    "clamped_i128",
    "clamped_u8",
    "clamped_u16",
    "clamped_u32",
    "clamped_u64",
    "clamped_u128",
    "clamped_f32",
    "clamped_f64",
    "rounded_f64",
    "rounded_f32",
    "rounded_i32",
    "truncated_i32",
    "unwrap",
    "unwrap_or",
    "expect",
    "to_string",
    "concat",
    "len",
    "enumerate",
    "zip",
    "sorted",
    "reversed",
    "sum",
    "prod",
    "min",
    "max",
    "all",
    "any",
    "join",
    "count",
    "index",
    "first",
    "last",
    "linspace",
    "map",
    "filter",
    "reduce",
    "Some",
    "None",
    "Ok",
    "Err",
];

/// Statement evaluation result: `return` exits non-locally up the call chain via `Flow::Return` (spec §14).
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Flow {
    Continue,
    Return(Value),
}

/// Interpreter (spec §4.8): the unified AST is degraded in two ways by context — MFn bodies and `let` right-hand sides go through the symbolic world
/// (`ExprDAG` → simplify → `Value::Expr`), while host statements go through numeric evaluation.
pub struct Evaluator {
    pub(crate) pool: &'static ExprPool,
    pub(crate) symbols: &'static SymbolTable,
    pub(crate) builtins: &'static BuiltinSymbols,
    pub(crate) output: Box<dyn FnMut(String)>,
    /// Policy stack (spec §4.6): global defaults at the bottom; module config / `with config` push and pop per block.
    pub(crate) config: Vec<Config>,
    /// Evaluated module public items (indexed by module path), available for `import` binding (spec §15).
    pub(crate) module_items: HashMap<String, HashMap<String, NamespaceItem>>,
    /// Collected warnings (spec §16.5): only the operator-overload `W0005` is emitted by the evaluator.
    pub(crate) warnings: Vec<SyntaxWarning>,
    /// Class registry (spec §4.7): class name → definition, shared across the evaluation run.
    pub(crate) class_defs: HashMap<String, ClassDef>,
    /// Instance table (spec §5): `Value::Class(id)` → runtime object.
    pub(crate) instances: HashMap<u32, ClassInstance>,
    /// Monotonic instance-id allocator.
    pub(crate) next_instance_id: u32,
    /// Operator overloads (spec §18.5): key `"<class>::<Op>"` → method definition (`ImplOp` has no `Hash`).
    pub(crate) overloads: HashMap<String, MethodDef>,
    /// Stack of the `self` receiver instance ids of the methods currently executing (spec §4.5).
    pub(crate) self_stack: Vec<u32>,
    /// Stack of the `self` receiver *values* of builtin-class methods currently executing (spec
    /// §18.1): `Value::String`/`Array`/... receivers are not class instances, so their `.pra`
    /// bodies resolve `self` through this stack instead of `self_stack`.
    pub(crate) self_values: Vec<Value>,
    /// Module path currently being evaluated (`""` for the root module), for `pub(mod)` visibility (spec §15.2).
    pub(crate) current_module: String,
}

impl Default for Evaluator {
    fn default() -> Self {
        Self::new()
    }
}

impl Evaluator {}

#[cfg(test)]
mod tests {
    use super::*;
    use prima_core::Real;

    fn eval(src: &str) -> Value {
        Evaluator::new().eval_value(src).expect("eval failed")
    }

    fn eval_fmt(src: &str) -> String {
        let mut ev = Evaluator::new();
        let v = ev.eval_value(src).expect("eval failed");
        ev.format_value(&v)
    }

    #[test]
    fn tuple_destructuring_in_let() {
        assert_eq!(
            eval("let t = (1, 2);\nlet (a, b) = t;\na + b"),
            Value::Number(Number::from(3))
        );
    }

    #[test]
    fn array_get_returns_option() {
        assert_eq!(
            eval("let v = [1, 2, 3];\nget(v, 1)"),
            Value::Option(Some(Box::new(Value::Number(Number::from(2)))))
        );
        assert_eq!(eval("let v = [1, 2, 3];\nget(v, 10)"), Value::Option(None));
        assert_eq!(
            eval("let v = [1, 2, 3];\nget(v, 0)"),
            Value::Option(Some(Box::new(Value::Number(Number::from(1)))))
        );
    }

    #[test]
    fn if_let_and_match() {
        assert_eq!(
            eval(
                "let v = [1, 2];\nlet r = 0;\nif let Some(x) = get(v, 0) {\n    r = x * 10;\n}\nr"
            ),
            Value::Number(Number::from(10))
        );
        let r = eval(
            "let r = try_i32(1e20);\nlet label = match r {\n    Ok(n) => \"ok\",\n    Err(e) => \"fail\"\n};\nlabel",
        );
        assert_eq!(r, Value::String("fail".into()));
    }

    #[test]
    fn while_let_loops_over_get() {
        let v = eval(
            "let arr = [1, 2, 3];\nlet sum = 0;\nlet i = 0;\nwhile let Some(x) = get(arr, i) {\n    sum += x;\n    i += 1;\n}\nsum",
        );
        assert_eq!(v, Value::Number(Number::from(6)));
    }

    #[test]
    fn match_guard_and_range_patterns() {
        assert_eq!(
            eval(
                "let r = match 5 {\n    0 => \"zero\",\n    1 | 2 => \"small\",\n    3..=9 => \"medium\",\n    n if n > 100 => \"large\",\n    _ => \"other\"\n};\nr"
            ),
            Value::String("medium".into())
        );
    }

    #[test]
    fn match_is_non_exhaustive() {
        assert!(
            Evaluator::new()
                .eval_value("match 1 {\n    2 => 0\n}")
                .is_err()
        );
    }

    #[test]
    fn try_operator_unwraps_ok() {
        let v = eval(
            "fn f(x) -> Result<Integer, Error> {\n    let v = try_i32(x)?;\n    return Ok(v);\n}\nf(7)",
        );
        assert_eq!(
            v,
            Value::Result(Ok(Box::new(Value::Number(Number::I32(7)))))
        );
    }

    #[test]
    fn try_operator_propagates_none() {
        let err = Evaluator::new().eval_value("fn g() -> Option<Integer> {\n    let x = get([1], 5)?;\n    return Some(x);\n}\ng()").unwrap_err();
        assert!(err.to_string().contains("None"), "unexpected error: {err}");
    }

    #[test]
    fn class_associated_function_and_method() {
        assert_eq!(
            eval_fmt(
                "class Vec2 {\n    x: F64, y: F64,\n    pub fn new(x, y) -> Self { Vec2 { x, y } }\n    pub fn sum(self) -> F64 { self.x + self.y }\n}\nlet v = Vec2::new(1, 2);\nv.sum()"
            ),
            "3"
        );
    }

    #[test]
    fn struct_literal_base_spreads_fields() {
        assert_eq!(
            eval_fmt(
                "class P { pub x: Integer, pub y: Integer }\nlet a = P { x: 1, y: 2 };\nlet b = P { x: 9, ..a };\nb.y"
            ),
            "2"
        );
    }

    #[test]
    fn private_fields_are_not_accessible_from_outside() {
        let src = "class C {\n    secret: Integer,\n    pub fn new(s) -> Self { C { secret: s } }\n}\nlet c = C::new(1);\nc.secret";
        assert!(Evaluator::new().eval_value(src).is_err());
    }

    #[test]
    fn missing_method_attaches_doc_note_and_did_you_mean() {
        let src = "\
class Greeter {
    pub name: String,
    /// Shout a greeting.
    pub fn greet(self) -> String { \"hello\" }
}
let g = Greeter { name: \"x\" };
g.greets()";
        let err = Evaluator::new()
            .eval_value(src)
            .expect_err("expected an unknown-method error");
        assert!(
            err.to_string().contains("unknown method `greets`"),
            "unexpected error: {err}"
        );
        assert_eq!(err.help().as_deref(), Some("did you mean `greet`?"));
        let notes = err.notes();
        assert!(
            notes.iter().any(|n| n.contains("Shout a greeting.")),
            "notes: {notes:?}"
        );
        assert!(
            notes.iter().any(|n| n.contains("greet(self)")),
            "notes: {notes:?}"
        );
    }

    #[test]
    fn wrong_arity_on_documented_method_attaches_note() {
        let src = "\
class Greeter {
    pub name: String,
    /// Shout a greeting.
    pub fn greet(self) -> String { \"hello\" }
}
let g = Greeter { name: \"x\" };
g.greet(1)";
        let err = Evaluator::new()
            .eval_value(src)
            .expect_err("expected an arity error");
        assert!(
            err.to_string().contains("expects 0 arguments"),
            "unexpected error: {err}"
        );
        let notes = err.notes();
        assert!(
            notes.iter().any(|n| n.contains("greet(self)")),
            "notes: {notes:?}"
        );
        assert!(
            notes.iter().any(|n| n.contains("Shout a greeting.")),
            "notes: {notes:?}"
        );
    }

    #[test]
    fn operator_overload_dispatches_with_warning() {
        let mut ev = Evaluator::new();
        let v = ev
            .eval_value(
                "class Vec2 { x: F64, y: F64 }\nimpl ops::Add for Vec2 {\n    fn add(self, rhs) -> Vec2 { Vec2 { x: self.x + rhs.x, y: self.y + rhs.y } }\n}\nlet a = Vec2 { x: 1, y: 2 };\nlet b = Vec2 { x: 3, y: 4 };\na + b",
            )
            .expect("eval failed");
        assert_eq!(ev.format_value(&v), "class Vec2");
        assert!(ev.warnings().iter().any(|w| w.code == "W0005"));
    }

    #[test]
    fn overload_policy_deny_errors() {
        let mut ev = Evaluator::new();
        assert!(ev
            .eval_value(
                "class V { x: F64 }\nimpl ops::Add for V {\n    fn add(self, rhs) -> V { V { x: self.x } }\n}\nwith config { overload_policy := deny } {\n    let a = V { x: 1 };\n    a + a\n}"
            )
            .is_err());
    }

    #[test]
    fn semicolon_statements_evaluate_without_warnings() {
        let mut ev = Evaluator::new();
        ev.eval_value("let a = 1;\nlet b = 2;\na + b")
            .expect("eval failed");
        assert!(
            ev.warnings().is_empty(),
            "`;`-separated statements emit no warnings"
        );
    }

    #[test]
    fn newline_separated_statements_fail_to_evaluate() {
        // Spec §4.2: newline statement separation was removed; the parser rejects it (E0011).
        assert!(
            Evaluator::new()
                .eval_value("let a = 1\nlet b = 2\n")
                .is_err()
        );
    }

    #[test]
    fn fixed_width_numbers_render() {
        assert_eq!(eval_fmt("to_i8(7)"), "7");
        assert_eq!(eval_fmt("to_u64(42)"), "42");
        assert_eq!(eval_fmt("to_usize(3)"), "3");
        let v = eval("to_f64(3)");
        assert!(matches!(v, Value::Number(Number::Real(Real::F64(x))) if x == 3.0));
    }

    #[test]
    fn array_element_assignment_writes_through() {
        assert_eq!(
            eval("let a = [1, 2, 3];\na[1] = 9;\na"),
            Value::Array(vec![
                Value::Number(Number::from(1)),
                Value::Number(Number::from(9)),
                Value::Number(Number::from(3)),
            ])
        );
    }

    #[test]
    fn array_slice_returns_subarray() {
        assert_eq!(
            eval("let a = [1, 2, 3, 4];\na[1..3]"),
            Value::Array(vec![
                Value::Number(Number::from(2)),
                Value::Number(Number::from(3)),
            ])
        );
    }

    #[test]
    fn host_namespace_native_function_dispatch() {
        // The registry is a process-global `OnceLock`; use a uniquely-named namespace (idempotent).
        crate::stdlib::register_namespace(
            "testns_eval",
            HashMap::from([(
                "answer".to_string(),
                NamespaceItem::Func(Function::Native {
                    name: "testns_eval::answer",
                    call: |_ev, args| {
                        if args.is_empty() {
                            Ok(Value::Number(Number::from(42)))
                        } else {
                            Err(RuntimeError::Message("`answer` takes no arguments".into()))
                        }
                    },
                }),
            )]),
        );
        assert_eq!(
            eval("import testns_eval;\ntestns_eval::answer()"),
            Value::Number(Number::from(42))
        );
    }
}
