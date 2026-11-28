//! Layered native fast paths for the `Array`/`Dict`/`Set` class methods (spec §11.3/§11.6/§18.4),
//! registered under `"<Type>::<name>"` keys and bound to the `@builtin(ON)` declarations in
//! `modules/{array,dict,set}.pra`. The `.pra` fallback bodies are the semantic authority; these
//! Rust implementations must match them (the O0/O2 consistency tests enforce this).

use prima_core::{Value, ValueKey};
use prima_runtime::builtin;
use prima_runtime::{Evaluator, RuntimeError, value_type_name};

fn array_copy(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    match args.first() {
        Some(Value::Array(a)) => Ok(Value::Array(a.clone())),
        Some(other) => Err(RuntimeError::receiver_type(
            "Array.copy".into(),
            "an array",
            value_type_name(other),
        )),
        None => Err(RuntimeError::MissingReceiver {
            callable: "Array.copy".into(),
        }),
    }
}

fn dict_copy(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    match args.first() {
        Some(Value::Dict(d)) => Ok(Value::Dict(d.clone())),
        Some(other) => Err(RuntimeError::receiver_type(
            "Dict.copy".into(),
            "a dict",
            value_type_name(other),
        )),
        None => Err(RuntimeError::MissingReceiver {
            callable: "Dict.copy".into(),
        }),
    }
}

fn set_symmetric_difference(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    match (args.first(), args.get(1)) {
        (Some(Value::Set(a)), Some(Value::Set(b))) => {
            let out: std::collections::HashSet<ValueKey> =
                a.symmetric_difference(b).cloned().collect();
            Ok(Value::Set(Box::new(out)))
        }
        (Some(Value::Set(_)), Some(other)) => Err(RuntimeError::argument_type(
            "Set.symmetric_difference".into(),
            1,
            "a set",
            value_type_name(other),
        )),
        (Some(other), _) => Err(RuntimeError::receiver_type(
            "Set.symmetric_difference".into(),
            "a set",
            value_type_name(other),
        )),
        (None, _) => Err(RuntimeError::MissingReceiver {
            callable: "Set.symmetric_difference".into(),
        }),
    }
}

fn char_is_digit(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    match args.first() {
        Some(Value::Char(c)) => Ok(Value::Bool(c.is_ascii_digit())),
        Some(other) => Err(RuntimeError::receiver_type(
            "Char.is_digit".into(),
            "a char",
            value_type_name(other),
        )),
        None => Err(RuntimeError::MissingReceiver {
            callable: "Char.is_digit".into(),
        }),
    }
}

/// Register every layered collection/char method implementation (spec §18.4).
pub fn register() {
    builtin!("Array::copy", array_copy, O2);
    builtin!("Dict::copy", dict_copy, O2);
    builtin!("Set::symmetric_difference", set_symmetric_difference, O2);
    builtin!("Char::is_digit", char_is_digit, O2);
}
