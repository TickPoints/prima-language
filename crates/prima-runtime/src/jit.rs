//! JIT function registry and call dispatch (spec §19.2/§19.4): `jit(...)` produces a
//! `Value::JitFunction(id)` whose id addresses a process-global registry of [`JitCallable`]s. A callable
//! is a numeric scalar function in one of several interchangeable forms — a compiled native function,
//! a reverse-mode gradient tape, a list of symbolic gradient expressions, or an interpreted fallback —
//! so a `JitFunction` keeps working even when native compilation is unavailable.
//!
//! Ids are process-local (like `Value::Class` handles, spec §5) and never die for the process lifetime.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use prima_core::expr_pool::ExprId;
use prima_core::number::{Number, Real};
use prima_core::{Value, simplify::simplify};
use prima_syntax::ast::{Expr, Param};

use crate::ad::Tape;
use crate::error::RuntimeError;
use crate::eval::{EnvRef, Evaluator};

/// A callable produced by `jit(...)` (spec §19.2/§19.4): a numeric scalar function, a compiled forward
/// scalar, or a reverse-mode gradient. `fallback` keeps an interpreted copy so a JitFunction still
/// works when compilation is unavailable; `expressions` carries a symbolic multi-output form (the tuple
/// `grad(expr)` case) evaluated numerically per call.
pub struct JitCallable {
    pub params: Vec<String>,
    pub n_out: usize,
    pub compiled: Option<Arc<prima_jit::CompiledScalar>>,
    pub tape: Option<Arc<Tape>>,
    pub fallback: Option<(Vec<Param>, Expr, EnvRef)>,
    /// Symbolic components of a multi-output function (e.g. the tuple returned by `grad(expr)`), evaluated
    /// numerically at call time when neither `compiled` nor `tape` is available.
    pub expressions: Option<(Vec<ExprId>, Vec<String>)>,
}

impl JitCallable {
    /// A scalar callable with a compiled (or fallback) scalar body.
    pub fn scalar(
        params: Vec<String>,
        compiled: Option<Arc<prima_jit::CompiledScalar>>,
        fallback: Option<(Vec<Param>, Expr, EnvRef)>,
    ) -> JitCallable {
        JitCallable {
            params,
            n_out: 1,
            compiled,
            tape: None,
            fallback,
            expressions: None,
        }
    }
}

/// Process-global registry: `Value::JitFunction(id)` ids are never recycled (like class instances).
///
/// The callables are only ever created and invoked from the interpreter's evaluating thread (`jit(...)`
/// and `JitFunction` calls resolve through `eval_call` on that thread; rayon tasks run self-contained
/// numeric bodies, never registered callables). `JitCallable` is nevertheless not `Send`/`Sync` because
/// `fallback` holds an `EnvRef` (`Rc<RefCell<Env>>`, spec §5), so the wrapper claims both bounds explicitly.
struct Registry(Mutex<HashMap<u32, Arc<JitCallable>>>);

// SAFETY: every access to the registry (`register`/`lookup`/`call`) happens on the evaluating thread
// and is guarded by the mutex; the non-`Send` `EnvRef` inside a callable is only ever dereferenced by
// that thread (the `Arc` keeps it alive for the process lifetime).
#[allow(clippy::arc_with_non_send_sync)]
unsafe impl Send for Registry {}
// SAFETY: see above — the mutex serializes access, and cross-thread sharing of callables never occurs
// (rayon tasks never touch registered callables).
#[allow(clippy::arc_with_non_send_sync)]
unsafe impl Sync for Registry {}

static REGISTRY: OnceLock<Registry> = OnceLock::new();
/// Monotonic id allocator (starts at 1).
static NEXT_ID: AtomicU32 = AtomicU32::new(1);

fn registry() -> &'static Registry {
    REGISTRY.get_or_init(|| Registry(Mutex::new(HashMap::new())))
}

/// Register a callable and return its process-local handle id.
// The `Arc<JitCallable>` is intentionally not `Send`/`Sync` (it may hold an `EnvRef`); the `Registry`
// only ever hands callables to the evaluating thread (see the `unsafe impl` safety comments above).
#[allow(clippy::arc_with_non_send_sync)]
pub fn register(c: JitCallable) -> u32 {
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    registry().0.lock().unwrap().insert(id, Arc::new(c));
    id
}

/// Look up a registered callable by handle id.
pub fn lookup(id: u32) -> Option<Arc<JitCallable>> {
    registry().0.lock().unwrap().get(&id).cloned()
}

/// Every argument must be a numeric (non-complex) `Value` → `f64`; otherwise `None`.
fn numeric_inputs(args: &[Value]) -> Option<Vec<f64>> {
    let mut out = Vec::with_capacity(args.len());
    for a in args {
        if let Value::Number(n) = a {
            if n.is_complex() {
                return None;
            }
            out.push(n.to_f64_lossy());
        } else {
            return None;
        }
    }
    Some(out)
}

fn number_result(x: f64) -> Value {
    Value::Number(Number::Real(Real::F64(x)))
}

/// Call a registered `JitFunction` handle with already-evaluated numeric arguments. Dispatch order:
/// compiled native function → reverse-mode tape → symbolic `expressions` → interpreted `fallback`.
pub fn call(ev: &mut Evaluator, id: u32, args: Vec<Value>) -> Result<Value, RuntimeError> {
    let callable = lookup(id)
        .ok_or_else(|| RuntimeError::Message(format!("unknown JIT function handle `{id}`")))?;
    let inputs = numeric_inputs(&args).ok_or_else(|| {
        RuntimeError::Message("JIT function arguments must be numeric (non-complex) values".into())
    })?;
    if inputs.len() != callable.params.len() {
        return crate::error::err(format!(
            "expected {} arguments, got {}",
            callable.params.len(),
            inputs.len()
        ));
    }
    if let Some(f) = &callable.compiled {
        let out = f.call(&inputs);
        return Ok(if callable.n_out == 1 {
            number_result(out)
        } else {
            Value::Array(vec![number_result(out)])
        });
    }
    if let Some(tape) = &callable.tape {
        let grad = tape.grad(&inputs);
        return Ok(if callable.n_out == 1 {
            number_result(grad[0])
        } else {
            Value::Array(grad.into_iter().map(number_result).collect())
        });
    }
    if let Some((ids, params)) = &callable.expressions {
        let mut out = Vec::with_capacity(ids.len());
        for &id in ids {
            let x = eval_symbolic(ev, id, params, &inputs)?;
            out.push(number_result(x));
        }
        return Ok(if callable.n_out == 1 {
            out.pop().unwrap_or_else(|| number_result(0.0))
        } else {
            Value::Array(out)
        });
    }
    if let Some((params, body, env)) = &callable.fallback {
        return ev.apply_jit_fallback(params, body, env, args);
    }
    crate::error::err(format!("JIT function `{}` has no executable form", id))
}

/// Evaluate a symbolic gradient/expression DAG numerically at `inputs` by substituting each parameter
/// symbol with its input value and collapsing the simplified result (spec §19.4).
fn eval_symbolic(
    ev: &Evaluator,
    id: ExprId,
    params: &[String],
    inputs: &[f64],
) -> Result<f64, RuntimeError> {
    let pool = ev.pool();
    let builtins = ev.builtins();
    let symbols = ev.symbols();
    let mut cur = id;
    for (i, name) in params.iter().enumerate() {
        let sym = symbols.intern(name);
        let val = pool.number(&Number::Real(Real::F64(inputs[i])));
        cur = crate::diff::substitute(pool, cur, sym, val);
    }
    let simp = simplify(pool, builtins, cur);
    prima_core::collapse::collapse_value(pool, builtins, &Value::Expr(simp))
        .map(|n| n.to_f64_lossy())
        .ok_or_else(|| RuntimeError::Message("JIT expression did not collapse to a number".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> Vec<String> {
        let params = vec!["x".to_string()];
        params
    }

    #[test]
    fn register_lookup_roundtrip() {
        let params = setup();
        let c = JitCallable::scalar(params, None, None);
        let id = register(c);
        assert!(id > 0, "registered ids start at 1");
        let c = lookup(id).expect("lookup finds a registered callable");
        assert_eq!(c.n_out, 1);
    }

    #[test]
    fn ids_are_monotonic() {
        let params = setup();
        let a = register(JitCallable::scalar(params.clone(), None, None));
        let b = register(JitCallable::scalar(params, None, None));
        assert!(b > a);
        assert_ne!(a, b);
    }

    #[test]
    fn lookup_unknown_is_none() {
        assert!(lookup(999_999).is_none());
    }

    #[test]
    fn scalar_helper_sets_n_out_one() {
        let params = setup();
        let c = JitCallable::scalar(params, None, None);
        assert_eq!(c.n_out, 1);
        assert!(c.tape.is_none());
        assert!(c.expressions.is_none());
    }
}
