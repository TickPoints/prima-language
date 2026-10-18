//! `mem` standard-library module (spec §12.3/§12.4): explicit reference counting.
//!
//! `mem::Arc` itself is written in the embedded `modules/mem.pra`; the `@builtin` methods bind to
//! the narrow `Evaluator::mem_arc_*` host interface, which owns the instance registry.

use prima_core::Value;
use prima_runtime::{Evaluator, RuntimeError, builtin};

/// Register the `mem` `@builtin` implementations (spec §18.4). Keys are `"<Class>::<method>"`
/// for class methods (spec §18.4 method binding).
pub fn register() {
    builtin!("Arc::clone", clone);
    builtin!("Arc::get", get);
    builtin!("Arc::strong_count", strong_count);
}

/// Exactly one receiver argument.
fn receiver<'a>(args: &'a [Value], method: &str) -> Result<&'a Value, RuntimeError> {
    match args {
        [receiver] => Ok(receiver),
        _ => Err(RuntimeError::Message(format!(
            "`{method}` expects no arguments besides the receiver, got {}",
            args.len().saturating_sub(1)
        ))),
    }
}

fn clone(ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    let arc = receiver(args, "Arc.clone")?.clone();
    ev.mem_arc_clone(arc)
}

fn get(ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    ev.mem_arc_payload(receiver(args, "Arc.get")?)
}

fn strong_count(ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    ev.mem_arc_count(receiver(args, "Arc.strong_count")?)
}
