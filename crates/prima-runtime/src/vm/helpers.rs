//! Value-level VM helpers on the `Evaluator` (spec §19.5, Milestone B).
//!
//! These mirror the AST interpreter's scalar value operations but operate on already-evaluated
//! `Value`s, so the VM can call them directly from its dispatch loop. They reuse the same machinery
//! (index reads/writes, class/builtin method dispatch, unary negation) and therefore produce
//! identical results to the interpreter.

use prima_core::Value;
use prima_syntax::ast::{Expr, ExprKind, Literal, Spanned, StringQuote, UnOp};

use crate::eval::{EnvRef, EvalBackend, Evaluator};

/// The current `self` receiver value for the executing method (class instance id or builtin-class
/// value), mirroring `ExprKind::Self_` resolution.
pub(crate) fn current_self_value(
    eval: &mut Evaluator,
) -> Result<Value, crate::error::RuntimeError> {
    if let Some(v) = eval.self_values.last().cloned() {
        return Ok(v);
    }
    if let Some(stack_id) = eval.self_stack.last().copied() {
        return Ok(Value::Class(stack_id));
    }
    Err(crate::error::RuntimeError::Message(
        "`self` outside of a method".into(),
    ))
}

/// Unary negation of a value, mirroring `eval_unary(UnOp::Neg, ...)`.
pub(crate) fn vm_neg(eval: &mut Evaluator, v: Value) -> Result<Value, crate::error::RuntimeError> {
    eval.eval_unary(UnOp::Neg, v)
}

/// Index read `base[idx]` on a `Value` (single-element index only) using the interpreter's array/dict
/// semantics. A class-instance receiver is rejected (no overload support in this milestone subset).
pub(crate) fn vm_index(
    eval: &mut Evaluator,
    base: Value,
    idx: Value,
) -> Result<Value, crate::error::RuntimeError> {
    match base {
        Value::Array(a) => {
            let i = index_to_usize(eval, &idx, "array index", a.len())?;
            a.get(i).cloned().ok_or_else(|| {
                crate::error::RuntimeError::IndexOutOfBounds(format!(
                    "index {i} (length {})",
                    a.len()
                ))
            })
        }
        Value::Dict(d) => {
            let key = prima_core::ValueKey::from_value(&idx).ok_or_else(|| {
                crate::error::RuntimeError::Message("dict key must be hashable".into())
            })?;
            d.get(&key)
                .cloned()
                .ok_or_else(|| crate::error::RuntimeError::Message("missing dict key".into()))
        }
        Value::String(s) => {
            let nchars = s.chars().count();
            let i = index_to_usize(eval, &idx, "string index", nchars)?;
            s.chars().nth(i).map(Value::Char).ok_or_else(|| {
                crate::error::RuntimeError::IndexOutOfBounds(format!("index {i} (length {nchars})"))
            })
        }
        Value::Tuple(t) => {
            let i = index_to_usize(eval, &idx, "tuple index", t.len())?;
            t.get(i).cloned().ok_or_else(|| {
                crate::error::RuntimeError::IndexOutOfBounds(format!(
                    "index {i} (length {})",
                    t.len()
                ))
            })
        }
        other => Err(crate::error::RuntimeError::Message(format!(
            "cannot index {}",
            crate::eval::value_type_name(&other)
        ))),
    }
}

/// Index write `base[idx] = value` for a mutable array (in place).
pub(crate) fn vm_index_store(
    eval: &mut Evaluator,
    base: Value,
    idx: Value,
    value: Value,
) -> Result<(), crate::error::RuntimeError> {
    match base {
        Value::Array(mut a) => {
            let len = a.len();
            let i = index_to_usize(eval, &idx, "array index", len)?;
            let slot = a.get_mut(i).ok_or_else(|| {
                crate::error::RuntimeError::IndexOutOfBounds(format!("index {i} (length {len})"))
            })?;
            *slot = value;
            Ok(())
        }
        other => Err(crate::error::RuntimeError::Message(format!(
            "cannot index-assign {}",
            crate::eval::value_type_name(&other)
        ))),
    }
}

/// Convert a `Value` index to a non-negative `usize`, normalizing negative indices from `len`.
fn index_to_usize(
    _eval: &mut Evaluator,
    idx: &Value,
    what: &str,
    len: usize,
) -> Result<usize, crate::error::RuntimeError> {
    match idx {
        Value::Number(n) => {
            let i = n.as_i64().ok_or_else(|| {
                crate::error::RuntimeError::Message(format!("{what} must be an integer"))
            })?;
            Ok(if i < 0 {
                len.checked_sub(i.unsigned_abs() as usize).ok_or_else(|| {
                    crate::error::RuntimeError::IndexOutOfBounds(format!(
                        "index {i} (length {len})"
                    ))
                })?
            } else {
                i as usize
            })
        }
        _ => Err(crate::error::RuntimeError::Message(format!(
            "{what} must be an integer"
        ))),
    }
}

/// Dispatch a method call on an evaluated receiver value. Class instances dispatch through the class
/// model; builtin classes go through `dispatch_builtin_method` with a synthetic receiver expression
/// that carries a span.
pub(crate) fn vm_method_value(
    eval: &mut Evaluator,
    env: &EnvRef,
    receiver: Value,
    name: &str,
    args: Vec<Value>,
) -> Result<Value, crate::error::RuntimeError> {
    let span = prima_syntax::Span::new(0, 0);
    match &receiver {
        Value::Class(id) => {
            let inst = eval.instances.get(id).cloned().ok_or_else(|| {
                crate::error::RuntimeError::Message("unknown class instance".into())
            })?;
            let def = eval.class_defs.get(&inst.class).cloned().ok_or_else(|| {
                crate::error::RuntimeError::Message(format!("unknown class `{}`", inst.class))
            })?;
            let method = def.methods.get(name).cloned().ok_or_else(|| {
                crate::error::RuntimeError::Message(format!(
                    "unknown method `{name}` on `{}`",
                    def.name
                ))
            })?;
            eval.call_method(&method, Value::Class(*id), args)
        }
        Value::String(_)
        | Value::Number(_)
        | Value::Array(_)
        | Value::Dict(_)
        | Value::Set(_)
        | Value::Char(_)
        | Value::Tuple(_)
        | Value::Option(_)
        | Value::Result(_) => {
            let backend = backend_for(&receiver, name);
            let synth = synthetic_receiver_expr(&receiver, span);
            eval.dispatch_builtin_method(
                env,
                &synth,
                builtin_class(&receiver),
                receiver,
                name,
                args,
                backend,
            )
        }
        other => Err(crate::error::RuntimeError::Message(format!(
            "cannot call method `{name}` on {}",
            crate::eval::value_type_name(other)
        ))),
    }
}

/// The appropriate runtime backend for a builtin-class method (mirrors `eval_method_call`).
fn backend_for(v: &Value, name: &str) -> Option<EvalBackend> {
    match v {
        Value::Number(_) => Some(EvalBackend::CollapseNumber),
        Value::Array(_) => Some(if is_mutating(name) {
            EvalBackend::MutateArray
        } else {
            EvalBackend::Array
        }),
        Value::Dict(_) => Some(if is_dict_mutating(name) {
            EvalBackend::MutateDict
        } else {
            EvalBackend::Dict
        }),
        Value::Set(_) => Some(if is_set_mutating(name) {
            EvalBackend::MutateSet
        } else {
            EvalBackend::Set
        }),
        Value::Char(_) => Some(EvalBackend::Char),
        Value::Tuple(_) => Some(EvalBackend::Tuple),
        Value::Option(_) | Value::Result(_) => Some(EvalBackend::Collapse),
        _ => None,
    }
}

/// Whether an array method mutates the receiver (spec §11.3).
fn is_mutating(name: &str) -> bool {
    matches!(
        name,
        "push" | "pop" | "append" | "extend" | "insert" | "remove" | "clear"
    )
}

/// Whether a dict method mutates the receiver (spec §11.6).
fn is_dict_mutating(name: &str) -> bool {
    matches!(name, "insert" | "remove" | "clear" | "update")
}

/// Whether a set method mutates the receiver (spec §11.6).
fn is_set_mutating(name: &str) -> bool {
    matches!(name, "add" | "remove" | "clear" | "update")
}

/// The builtin class name for a receiver value (spec §18.1).
fn builtin_class(v: &Value) -> &'static str {
    match v {
        Value::String(_) => "String",
        Value::Number(_) => "Number",
        Value::Array(_) => "Array",
        Value::Dict(_) => "Dict",
        Value::Set(_) => "Set",
        Value::Char(_) => "Char",
        Value::Tuple(_) => "Tuple",
        Value::Option(_) => "Option",
        Value::Result(_) => "Result",
        _ => "",
    }
}

/// Build a synthetic receiver expression carrying `span` for `dispatch_builtin_method` (which uses the
/// expression for span-annotated diagnostics and env-consuming mutating methods).
fn synthetic_receiver_expr(receiver: &Value, span: prima_syntax::Span) -> Expr {
    let kind = match receiver {
        Value::String(s) => ExprKind::Literal(Literal::String {
            value: s.clone(),
            quote: StringQuote::Double,
            raw: false,
        }),
        Value::Bool(b) => ExprKind::Literal(Literal::Bool(*b)),
        Value::Char(c) => ExprKind::Literal(Literal::Char(*c)),
        _ => ExprKind::Path {
            segments: vec![Spanned {
                value: "_vm_rcv".into(),
                span,
            }],
        },
    };
    Expr { kind, span }
}
