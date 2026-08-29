//! Layered native fast paths for the `Array`/`Dict`/`Set` class methods (spec §11.3/§11.6/§18.4),
//! registered under `"<Type>::<name>"` keys and bound to the `@builtin(ON)` declarations in
//! `modules/{array,dict,set}.pra`. The `.pra` fallback bodies are the semantic authority; these
//! Rust implementations must match them (the O0/O2 consistency tests enforce this).

use prima_core::Value;
use prima_runtime::builtin;
use prima_runtime::{Evaluator, RuntimeError, value_type_name};

fn array_copy(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    match args.first() {
        Some(Value::Array(a)) => Ok(Value::Array(a.clone())),
        Some(other) => Err(RuntimeError::Message(format!(
            "`Array.copy` expects an array receiver, got {}",
            value_type_name(other)
        ))),
        None => Err(RuntimeError::Message(
            "`Array.copy` missing receiver".into(),
        )),
    }
}

fn dict_copy(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    match args.first() {
        Some(Value::Dict(d)) => Ok(Value::Dict(d.clone())),
        Some(other) => Err(RuntimeError::Message(format!(
            "`Dict.copy` expects a dict receiver, got {}",
            value_type_name(other)
        ))),
        None => Err(RuntimeError::Message("`Dict.copy` missing receiver".into())),
    }
}

fn set_symmetric_difference(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    match (args.first(), args.get(1)) {
        (Some(Value::Set(a)), Some(Value::Set(b))) => {
            Ok(Value::Set(a.symmetric_difference(b).cloned().collect()))
        }
        _ => Err(RuntimeError::Message(
            "`Set.symmetric_difference` expects a set receiver and a set argument".into(),
        )),
    }
}

fn char_is_digit(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    match args.first() {
        Some(Value::Char(c)) => Ok(Value::Bool(c.is_ascii_digit())),
        Some(other) => Err(RuntimeError::Message(format!(
            "`Char.is_digit` expects a char receiver, got {}",
            value_type_name(other)
        ))),
        None => Err(RuntimeError::Message(
            "`Char.is_digit` missing receiver".into(),
        )),
    }
}

fn tuple_count(ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    match args.first() {
        Some(Value::Tuple(t)) => {
            let value = args
                .get(1)
                .cloned()
                .ok_or_else(|| RuntimeError::Message("`Tuple.count` missing argument 1".into()))?;
            Ok(Value::Number(prima_core::Number::from(
                t.iter().filter(|e| ev.value_eq(e, &value)).count() as i64,
            )))
        }
        Some(other) => Err(RuntimeError::Message(format!(
            "`Tuple.count` expects a tuple receiver, got {}",
            value_type_name(other)
        ))),
        None => Err(RuntimeError::Message(
            "`Tuple.count` missing receiver".into(),
        )),
    }
}

/// Register every layered collection/char/tuple method implementation (spec §18.4).
pub fn register() {
    builtin!("Array::copy", array_copy, O2);
    builtin!("Dict::copy", dict_copy, O2);
    builtin!("Set::symmetric_difference", set_symmetric_difference, O2);
    builtin!("Char::is_digit", char_is_digit, O2);
    builtin!("Tuple::count", tuple_count, O2);
}
