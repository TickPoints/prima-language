//! JIT function registry and call dispatch (spec §19.2/§19.4): `jit(...)` produces a
//! `Value::JitFunction(id)` whose id addresses a process-global registry of [`JitCallable`]s. A callable
//! is a numeric scalar function in one of several interchangeable forms — a compiled native function,
//! a reverse-mode gradient tape, a list of symbolic gradient expressions, or an interpreted fallback —
//! so a `JitFunction` keeps working even when native compilation is unavailable.
//!
//! Ids are process-local (like `Value::Class` handles, spec §5) and are never recycled; entries
//! themselves are evicted oldest-first once the registry capacity is reached (a resource limit —
//! see [`REGISTRY_CAPACITY`]).

use std::cell::Cell;
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

/// Registry capacity (resource limit): every `jit(...)` call registers a fresh callable, so an
/// ever-growing registry leaks memory for loops like `while … { f = jit(x^2); f(1.0); }`. The
/// registry keeps at most this many callables, evicting the oldest (smallest) ids; 1024 far
/// exceeds the number of JIT functions a working program registers. An evicted handle reports
/// "unknown JIT function handle" at call time.
const REGISTRY_CAPACITY: usize = 1024;

/// Process-global registry: `Value::JitFunction(id)` ids are never recycled (like class
/// instances); entries are evicted oldest-first beyond [`REGISTRY_CAPACITY`].
///
/// Callables are created and invoked from the interpreter's evaluating thread (`jit(...)` and
/// `JitFunction` calls resolve through `eval_call` on that thread). Rayon `parfor` tasks may
/// *look up* a callable and run its compiled/tape/symbolic forms (all thread-safe reads), but
/// never execute the `fallback` form: `ParforTaskGuard` marks worker threads and [`call`] refuses
/// the fallback there, because it dereferences the registering thread's `EnvRef`
/// (`Rc<RefCell<Env>>`, spec §5). `JitCallable` is therefore not `Send`/`Sync` and the wrapper
/// claims both bounds explicitly.
struct Registry(Mutex<HashMap<u32, Arc<JitCallable>>>);

// SAFETY: all registry map access is guarded by the mutex; the non-`Send` `EnvRef` inside a
// callable is only ever *dereferenced* on the evaluating thread (worker threads are refused by
// the `ParforTaskGuard` check in `call`). Cross-thread `Arc` clones are safe: the registry
// always retains one strong reference while the entry is live, and eviction happens only in
// `register` on the evaluating thread — which is blocked for the duration of a `parfor` — so an
// `Arc` dropped on a worker thread is never the last reference.
#[allow(clippy::arc_with_non_send_sync)]
unsafe impl Send for Registry {}
// SAFETY: see above — the mutex serializes access; the fallback `EnvRef` is dereferenced only on
// the evaluating thread (worker access to the compiled/tape/symbolic forms is read-only).
#[allow(clippy::arc_with_non_send_sync)]
unsafe impl Sync for Registry {}

static REGISTRY: OnceLock<Registry> = OnceLock::new();
/// Monotonic id allocator (starts at 1).
static NEXT_ID: AtomicU32 = AtomicU32::new(1);

fn registry() -> &'static Registry {
    REGISTRY.get_or_init(|| Registry(Mutex::new(HashMap::new())))
}

thread_local! {
    /// Marks rayon threads running `parfor` tasks (spec §17.2): set by [`ParforTaskGuard`] at the
    /// worker closure entry, checked in [`call`] before the fallback dispatch.
    static PARFOR_TASK: Cell<bool> = const { Cell::new(false) };
}

/// Marks the current thread as a `parfor` worker task for the guard's lifetime: JIT fallback
/// execution is refused there (the fallback's `EnvRef` is not thread-safe).
pub(crate) struct ParforTaskGuard;

impl ParforTaskGuard {
    pub(crate) fn new() -> ParforTaskGuard {
        PARFOR_TASK.with(|f| f.set(true));
        ParforTaskGuard
    }
}

impl Drop for ParforTaskGuard {
    fn drop(&mut self) {
        PARFOR_TASK.with(|f| f.set(false));
    }
}

/// Whether the current thread is running a `parfor` worker task (spec §17.2).
pub(crate) fn in_parfor_task() -> bool {
    PARFOR_TASK.with(Cell::get)
}

/// Register a callable and return its process-local handle id.
// The `Arc<JitCallable>` is intentionally not `Send`/`Sync` (it may hold an `EnvRef`); the `Registry`
// only ever hands callables to the evaluating thread (see the `unsafe impl` safety comments above).
#[allow(clippy::arc_with_non_send_sync)]
pub fn register(c: JitCallable) -> u32 {
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let mut entries = registry().0.lock().unwrap();
    entries.insert(id, Arc::new(c));
    // Ids are monotonic, so the smallest key is the oldest registration; evict it while over
    // capacity (resource limit — see [`REGISTRY_CAPACITY`]).
    while entries.len() > REGISTRY_CAPACITY {
        let oldest = entries
            .keys()
            .copied()
            .min()
            .expect("over-capacity registry is non-empty");
        entries.remove(&oldest);
    }
    id
}

/// Look up a registered callable by handle id; evicted (expired) handles return `None`.
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
            Value::Array(vec![number_result(out)].into())
        });
    }
    if let Some(tape) = &callable.tape {
        let grad = tape.grad(&inputs);
        return Ok(if callable.n_out == 1 {
            number_result(grad[0])
        } else {
            Value::Array(
                grad.into_iter()
                    .map(number_result)
                    .collect::<Vec<Value>>()
                    .into(),
            )
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
            Value::Array(out.into())
        });
    }
    if let Some((params, body, env)) = &callable.fallback {
        // The interpreted fallback dereferences the registering thread's `EnvRef`
        // (`Rc<RefCell<Env>>`); running it inside a `parfor` worker task would be a cross-thread
        // data race, so it is refused with an actionable message (spec §17.2).
        if in_parfor_task() {
            return crate::error::err(
                "JIT fallback is not available inside parfor; provide a compilable function",
            );
        }
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
    use std::sync::Mutex as StdMutex;

    fn setup() -> Vec<String> {
        let params = vec!["x".to_string()];
        params
    }

    /// The registry is process-global, so tests that register/look up must not interleave.
    static TEST_LOCK: StdMutex<()> = StdMutex::new(());

    #[test]
    fn register_lookup_roundtrip() {
        let _g = TEST_LOCK.lock().unwrap();
        let params = setup();
        let c = JitCallable::scalar(params, None, None);
        let id = register(c);
        assert!(id > 0, "registered ids start at 1");
        let c = lookup(id).expect("lookup finds a registered callable");
        assert_eq!(c.n_out, 1);
    }

    #[test]
    fn ids_are_monotonic() {
        let _g = TEST_LOCK.lock().unwrap();
        let params = setup();
        let a = register(JitCallable::scalar(params.clone(), None, None));
        let b = register(JitCallable::scalar(params, None, None));
        assert!(b > a);
        assert_ne!(a, b);
    }

    #[test]
    fn lookup_unknown_is_none() {
        let _g = TEST_LOCK.lock().unwrap();
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

    #[test]
    fn registry_is_bounded() {
        let _g = TEST_LOCK.lock().unwrap();
        // Register past the capacity: the map must stop growing (oldest entries evicted).
        let mut last = 0;
        for _ in 0..REGISTRY_CAPACITY + 128 {
            last = register(JitCallable::scalar(setup(), None, None));
        }
        let entries = registry().0.lock().unwrap();
        assert!(
            entries.len() <= REGISTRY_CAPACITY,
            "registry grew to {} entries",
            entries.len()
        );
        // The most recent registration is always retained.
        assert!(entries.contains_key(&last));
    }

    #[test]
    fn parfor_task_guard_marks_the_thread() {
        assert!(!in_parfor_task());
        {
            let _g = ParforTaskGuard::new();
            assert!(in_parfor_task());
        }
        assert!(!in_parfor_task(), "guard drop clears the marker");
    }

    /// The fallback must be refused on a thread marked as a `parfor` task (the guard's whole
    /// point): the call returns the actionable error instead of dereferencing the `EnvRef`.
    #[test]
    fn fallback_call_refused_inside_parfor_task() {
        use crate::eval::Env;
        use std::cell::RefCell as StdRefCell;
        use std::rc::Rc;

        std::thread::spawn(|| {
            let _lock = TEST_LOCK.lock().unwrap();
            let _task = ParforTaskGuard::new();
            assert!(in_parfor_task());
            // Build the callable on this thread (the `EnvRef` is thread-local by construction);
            // a fallback-only callable with no parameters reaches the guard before any argument
            // handling error.
            let program = prima_syntax::parse("x").expect("parse");
            let body = match &program.stmts[0] {
                prima_syntax::ast::Stmt::Expr(e) => e.clone(),
                other => panic!("expected an expression statement, got {other:?}"),
            };
            let env: EnvRef = Rc::new(StdRefCell::new(Env::new()));
            let id = register(JitCallable::scalar(vec![], None, Some((vec![], body, env))));
            let mut ev = Evaluator::new();
            let err = call(&mut ev, id, vec![]).unwrap_err();
            assert!(
                err.to_string().contains("JIT fallback"),
                "unexpected error: {err}"
            );
        })
        .join()
        .expect("worker thread panicked");
    }
}
