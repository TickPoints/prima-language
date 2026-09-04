//! Value application (spec §10/§11): indexing, function application (host + host TCO), broadcasting
//! (parallel and scalar-with-array), array binary ops, and the SIMD vector fast paths.

use super::helpers::{
    PARALLEL_BROADCAST_THRESHOLD, normalize_index, numeric_args, overload_key, path_key,
    require_numeric_array,
};
use super::*;

impl Evaluator {
    pub(crate) fn eval_index(
        &mut self,
        env: &EnvRef,
        base: &Expr,
        index: &prima_syntax::ast::Index,
    ) -> Result<Value, RuntimeError> {
        let arr_v = self.eval_expr(env, base)?;
        // Operator overload (spec §18.5): `Index` on a class instance.
        if let Value::Class(id) = &arr_v {
            let class = self.instances.get(id).map(|i| i.class.clone());
            if let Some(class) = class {
                if self
                    .overloads
                    .contains_key(&overload_key(&class, ImplOp::Index))
                {
                    if index.items.len() != 1 {
                        return crate::error::err(
                            "multi-dimensional indexing is not supported yet",
                        );
                    }
                    let idx_v = match &index.items[0] {
                        IndexItem::Elem(e) => self.eval_expr(env, e)?,
                        IndexItem::Slice { .. } => {
                            return crate::error::err(
                                "slice indexing is not supported for overloads",
                            );
                        }
                    };
                    return self.overload_dispatch(&class, ImplOp::Index, arr_v, vec![idx_v]);
                }
                return crate::error::err("cannot index a class instance");
            }
        }
        match arr_v {
            // Array indexing with negative indices and clamped slices (spec §11.3).
            Value::Array(a) => {
                if index.items.len() != 1 {
                    return crate::error::err("multi-dimensional indexing is not supported yet");
                }
                match &index.items[0] {
                    IndexItem::Elem(e) => {
                        let raw = self.eval_index_i64(env, e)?;
                        let idx = normalize_index(raw, a.len()).ok_or_else(|| {
                            RuntimeError::IndexOutOfBounds(format!(
                                "index {raw} (length {})",
                                a.len()
                            ))
                        })?;
                        Ok(a[idx].clone())
                    }
                    IndexItem::Slice { start, end } => {
                        let (lo, hi) =
                            self.slice_bounds(env, start.as_ref(), end.as_ref(), a.len())?;
                        Ok(Value::Array(a[lo..hi].to_vec()))
                    }
                }
            }
            // Dict indexing by key (spec §11.6): a missing key is `R0012`.
            Value::Dict(d) => {
                if index.items.len() != 1 {
                    return crate::error::err("multi-dimensional indexing is not supported yet");
                }
                match &index.items[0] {
                    IndexItem::Elem(e) => {
                        let k = self.eval_expr(env, e)?;
                        let key = ValueKey::from_value(&k).ok_or_else(|| {
                            RuntimeError::Message("dict key must be a hashable value".into())
                        })?;
                        d.get(&key)
                            .cloned()
                            .ok_or_else(|| RuntimeError::Message("key not found".into()))
                    }
                    IndexItem::Slice { .. } => crate::error::err("cannot slice a dict"),
                }
            }
            other => crate::error::err(format!(
                "indexing requires an array or dict, got {}",
                value_type_name(&other)
            )),
        }
    }

    /// Evaluate an index expression to a raw `i64` (negative allowed; normalized by the caller).
    pub(crate) fn eval_index_i64(&mut self, env: &EnvRef, e: &Expr) -> Result<i64, RuntimeError> {
        match self.eval_expr(env, e)? {
            Value::Number(n) => n
                .as_i64()
                .ok_or_else(|| RuntimeError::Message("array index must be an integer".into())),
            _ => crate::error::err("array index must be an integer"),
        }
    }

    pub(crate) fn apply_function(
        &mut self,
        func: &Function,
        args: Vec<Value>,
    ) -> Result<Value, RuntimeError> {
        // Collection convenience functions receive their array argument whole (spec appendix B.1):
        // they are not subject to the implicit-broadcast path (spec §11.4).
        let takes_array_whole = matches!(func, Function::Builtin(b) if b.is_collection());
        let array_positions: Vec<usize> = if takes_array_whole {
            Vec::new()
        } else {
            args.iter()
                .enumerate()
                .filter(|(_, v)| matches!(v, Value::Array(_)))
                .map(|(i, _)| i)
                .collect()
        };
        if !array_positions.is_empty() && func.is_pure() {
            if self.current_config().broadcast {
                return self.broadcast_call(func, args, &array_positions);
            }
            return crate::error::err(
                "implicit broadcast is disabled (`broadcast := false`); use `@.`",
            );
        }
        match func {
            Function::Builtin(b) => self.call_builtin(*b, args),
            Function::NativeGet => {
                if args.len() != 2 {
                    return crate::error::err("`get` expects 2 arguments");
                }
                self.call_array_get(args[0].clone(), args[1].clone())
            }
            Function::Native { call, .. } => call(self, &args),
            Function::User {
                params,
                body,
                env: f_env,
                hot,
                ..
            } => {
                if args.len() != params.len() {
                    return crate::error::err(format!(
                        "expected {} arguments, got {}",
                        params.len(),
                        args.len()
                    ));
                }
                // JIT hot path (spec §19.2): when every argument is a non-complex number, run the body
                // natively. `@jit` forces compilation on the first call; otherwise the body is compiled
                // once after `JIT_CALL_THRESHOLD` numeric calls. Failed compilations are cached so a
                // non-numeric body always falls through to the interpreted path below.
                if let Some(inputs) = numeric_args(&args) {
                    if let Some(Some(f)) = hot.compiled.get() {
                        return Ok(Value::Number(Number::Real(Real::F64(f.call(&inputs)))));
                    }
                    if hot.compiled.get().is_none() {
                        // Auto hot-path compilation (spec §19.2, gated at `opt_level >= O2` per §10.2):
                        // compile on the call that makes the count reach `JIT_CALL_THRESHOLD` (spec §19.2
                        // default 100), so `for i in 1..100 { f(to_f64(i)) }` warms up and the next call
                        // (`f(to_f64(101))`) runs native. `@jit` (an execution-model annotation, §10.2)
                        // forces compilation on the first numeric call at any tier.
                        let c = hot.calls.fetch_add(1, AtomicOrdering::Relaxed);
                        let attempt = hot.force
                            || (self.current_config().opt_level >= OptLevel::O2
                                && c + 1 >= JIT_CALL_THRESHOLD);
                        if attempt {
                            let compiled = self.try_compile_body(params, body, f_env);
                            let _ = hot.compiled.set(compiled);
                            if let Some(Some(f)) = hot.compiled.get() {
                                return Ok(Value::Number(Number::Real(Real::F64(f.call(&inputs)))));
                            }
                        }
                    }
                }
                let call_env = Env::child(f_env);
                for (p, a) in params.iter().zip(args) {
                    call_env.borrow_mut().set_value(&p.name.value, a);
                }
                self.eval_expr(&call_env, body)
            }
            Function::Host {
                params,
                ret: _,
                body,
                env: f_env,
            } => self.apply_host(params, body, f_env, args),
            // A `@builtin(ON)` layered fn (spec §18.4): the Rust implementation is used when the
            // active `opt_level` is at least the declared tier and it is registered; otherwise the
            // `.pra` fallback body is evaluated (host semantics, so TCO applies at `O2`).
            Function::Layered {
                params,
                body,
                env: f_env,
                native,
                level,
                ..
            } => {
                if args.len() != params.len() {
                    return crate::error::err(format!(
                        "expected {} arguments, got {}",
                        params.len(),
                        args.len()
                    ));
                }
                if let Some(call) = native {
                    let cfg_level = self.current_config().opt_level.tier();
                    if cfg_level >= *level {
                        return call(self, &args);
                    }
                }
                let tco = self.current_config().opt_level >= OptLevel::O2;
                self.apply_host_tco(params, body, f_env, args, tco)
            }
        }
    }

    /// Evaluate a host `fn` body (spec §11.2). When `tco` is enabled and the body ends in a direct
    /// `return f(args)` preceded only by effect-free statements (see `crate::opt`), the call is
    /// trampolined so tail recursion runs in constant stack space; otherwise the body is evaluated
    /// normally (spec §10.2 item 6, gated by `opt_level >= O2`).
    pub(crate) fn apply_host(
        &mut self,
        params: &[Param],
        body: &Block,
        f_env: &EnvRef,
        args: Vec<Value>,
    ) -> Result<Value, RuntimeError> {
        let tco = self.current_config().opt_level >= OptLevel::O2;
        self.apply_host_tco(params, body, f_env, args, tco)
    }

    pub(crate) fn apply_host_tco(
        &mut self,
        params: &[Param],
        body: &Block,
        f_env: &EnvRef,
        args: Vec<Value>,
        tco: bool,
    ) -> Result<Value, RuntimeError> {
        if args.len() != params.len() {
            return crate::error::err(format!(
                "expected {} arguments, got {}",
                params.len(),
                args.len()
            ));
        }
        // Tail-call optimization (spec §10.2 item 6): when a host body ends in a direct
        // `return f(args)` preceded only by effect-free statements (see `crate::opt`), the
        // call is trampolined so tail recursion runs in constant stack space. Early `return`s
        // in the effect-free prefix are honored; the prefix is re-evaluated per iteration but
        // is pure, so this cannot change observable behavior.
        if !tco {
            let call_env = Env::child(f_env);
            for (p, a) in params.iter().zip(args) {
                call_env.borrow_mut().set_value(&p.name.value, a);
            }
            return self.eval_block_tail(&call_env, body);
        }
        let mut cparams: Vec<Param> = params.to_vec();
        let mut cbody: Block = (*body).clone();
        let mut cenv = Rc::clone(f_env);
        let mut cargs = args;
        loop {
            let call_env = Env::child(&cenv);
            for (p, a) in cparams.iter().zip(&cargs) {
                call_env.borrow_mut().set_value(&p.name.value, a.clone());
            }
            let Some(tc) = crate::opt::tail_call_of(&cbody) else {
                return self.eval_block_tail(&call_env, &cbody);
            };
            let n = cbody.stmts.len();
            for stmt in &cbody.stmts[..n - 1] {
                if let Flow::Return(v) = self.eval_stmt(&call_env, stmt)? {
                    return Ok(v);
                }
            }
            let ExprKind::Path { segments } = &tc.callee.kind else {
                return self.eval_block_tail(&call_env, &cbody);
            };
            let next = self.resolve_func(&call_env, segments).ok_or_else(|| {
                RuntimeError::Message(format!("unknown function `{}`", path_key(segments)))
            })?;
            let nargs: Vec<Value> = tc
                .args
                .iter()
                .map(|a| self.eval_expr(&call_env, a))
                .collect::<Result<_, _>>()?;
            match next {
                Function::Host {
                    params: np,
                    ret: _,
                    body: nb,
                    env: nenv,
                } => {
                    if nargs.len() != np.len() {
                        return crate::error::err(format!(
                            "expected {} arguments, got {}",
                            np.len(),
                            nargs.len()
                        ));
                    }
                    cparams = np;
                    cbody = nb;
                    cenv = nenv;
                    cargs = nargs;
                }
                other => return self.apply_function(&other, nargs),
            }
        }
    }

    /// Broadcast (spec §11.4): pure functions are applied elementwise to array arguments; **empty arrays are rejected** (`R0014`),
    /// non-numeric elements/scalars error (`R0009`). `@parallel` MFn (spec §17.1) over large arrays are split across rayon threads.
    pub(crate) fn broadcast_call(
        &mut self,
        func: &Function,
        args: Vec<Value>,
        positions: &[usize],
    ) -> Result<Value, RuntimeError> {
        let mut len = 0usize;
        let mut first = true;
        for &pos in positions {
            if let Value::Array(a) = &args[pos] {
                if first {
                    len = a.len();
                    first = false;
                } else if a.len() != len {
                    return crate::error::err("dimension mismatch in broadcast");
                }
            }
        }
        if len == 0 {
            return crate::error::err("cannot broadcast over an empty array");
        }
        if let Function::User {
            params,
            body,
            parallel: true,
            ..
        } = func
            && len >= PARALLEL_BROADCAST_THRESHOLD
            && rayon::current_num_threads() > 1
        {
            return self.broadcast_parallel(params, body, &args, positions, len);
        }
        let mut results = Vec::with_capacity(len);
        for i in 0..len {
            let mut cargs = Vec::with_capacity(args.len());
            for (j, v) in args.iter().enumerate() {
                if positions.contains(&j) {
                    if let Value::Array(a) = v {
                        // Only numeric elements participate in broadcast (spec §11.4, R0009).
                        match &a[i] {
                            Value::Number(n) => cargs.push(Value::Number(n.clone())),
                            _ => {
                                return crate::error::err("cannot broadcast a non-numeric element");
                            }
                        }
                    }
                } else {
                    match v {
                        Value::Number(_) => cargs.push(v.clone()),
                        _ => return crate::error::err("cannot broadcast a non-numeric scalar"),
                    }
                }
            }
            match self.apply_function(func, cargs)? {
                Value::Number(n) => results.push(Value::Number(n)),
                _ => return crate::error::err("broadcast result must be numeric"),
            }
        }
        Ok(Value::Array(results))
    }

    /// Parallel broadcast of a `@parallel` MFn (spec §17.1/17.4): each rayon thread block runs an
    /// independent `Evaluator` over a chunk of the array. The body must be self-contained — parameters
    /// are bound in a fresh root environment, so no captured (non-`Send`) closure environment is shared.
    pub(crate) fn broadcast_parallel(
        &mut self,
        params: &[Param],
        body: &Expr,
        args: &[Value],
        positions: &[usize],
        len: usize,
    ) -> Result<Value, RuntimeError> {
        let cfg = self.current_config().clone();
        let params_owned = params.to_vec();
        let body_owned = body.clone();
        let positions_owned = positions.to_vec();
        let results: Vec<Result<Number, RuntimeError>> = (0..len)
            .into_par_iter()
            .map(|i| {
                let mut cargs = Vec::with_capacity(args.len());
                for (j, v) in args.iter().enumerate() {
                    if positions_owned.contains(&j) {
                        if let Value::Array(a) = v {
                            match &a[i] {
                                Value::Number(n) => cargs.push(Value::Number(n.clone())),
                                _ => {
                                    return Err(RuntimeError::Message(
                                        "cannot broadcast a non-numeric element".into(),
                                    ));
                                }
                            }
                        } else {
                            return Err(RuntimeError::Message(
                                "cannot broadcast a non-numeric scalar".into(),
                            ));
                        }
                    } else {
                        match v {
                            Value::Number(_) => cargs.push(v.clone()),
                            _ => {
                                return Err(RuntimeError::Message(
                                    "cannot broadcast a non-numeric scalar".into(),
                                ));
                            }
                        }
                    }
                }
                let mut ev = Evaluator::spawn_task_evaluator(&cfg);
                let call_env = Rc::new(RefCell::new(Env::new()));
                for (p, a) in params_owned.iter().zip(cargs) {
                    call_env.borrow_mut().set_value(&p.name.value, a);
                }
                match ev.eval_expr(&call_env, &body_owned)? {
                    Value::Number(n) => Ok(n),
                    _ => Err(RuntimeError::Message(
                        "broadcast result must be numeric".into(),
                    )),
                }
            })
            .collect();
        let mut out = Vec::with_capacity(len);
        for r in results {
            out.push(Value::Number(r?));
        }
        Ok(Value::Array(out))
    }

    /// Binary array operation (spec §11.3/§11.4): `Array + Array` concatenates; `Array ∘ Array` for the
    /// other operators is elementwise (equal lengths, numeric-homogeneous), `Array ∘ scalar` broadcasts
    /// the scalar. Empty arrays error (`R0014`).
    pub(crate) fn eval_binary_array(
        &mut self,
        op: BinOp,
        a: Value,
        b: Value,
    ) -> Result<Value, RuntimeError> {
        // `Array + Array` concatenates (spec §11.3, v2.1) — this overrides the stale §11.4 elementwise example.
        if op == BinOp::Add && matches!(&a, Value::Array(_)) && matches!(&b, Value::Array(_)) {
            let Value::Array(mut av) = a else {
                unreachable!("checked above")
            };
            let Value::Array(bv) = b else {
                unreachable!("checked above")
            };
            av.extend(bv);
            return Ok(Value::Array(av));
        }
        let out: Vec<Value> = match (a, b) {
            (Value::Array(av), Value::Array(bv)) => {
                if av.len() != bv.len() {
                    return crate::error::err("dimension mismatch in array operation");
                }
                if av.is_empty() {
                    return crate::error::err("cannot operate on an empty array");
                }
                let av = require_numeric_array(&av)?;
                let bv = require_numeric_array(&bv)?;
                if let Some(v) = self.try_simd_arrays(op, &av, &bv) {
                    return Ok(Value::Array(v));
                }
                let mut out = Vec::with_capacity(av.len());
                for (x, y) in av.into_iter().zip(bv) {
                    match self.eval_number_binary(op, x, y)? {
                        Value::Number(n) => out.push(Value::Number(n)),
                        _ => return crate::error::err("array operation result must be numeric"),
                    }
                }
                out
            }
            (Value::Array(av), other) => {
                let scalar = self.scalar_for_broadcast(other)?;
                if av.is_empty() {
                    return crate::error::err("cannot operate on an empty array");
                }
                let av = require_numeric_array(&av)?;
                if let Some(v) = self.try_simd_scalar(op, &av, &scalar) {
                    return Ok(Value::Array(v));
                }
                let mut out = Vec::with_capacity(av.len());
                for x in av {
                    match self.eval_number_binary(op, x, scalar.clone())? {
                        Value::Number(n) => out.push(Value::Number(n)),
                        _ => return crate::error::err("array operation result must be numeric"),
                    }
                }
                out
            }
            (other, Value::Array(bv)) => {
                let scalar = self.scalar_for_broadcast(other)?;
                if bv.is_empty() {
                    return crate::error::err("cannot operate on an empty array");
                }
                let bv = require_numeric_array(&bv)?;
                if let Some(v) = self.try_simd_scalar_left(op, &scalar, &bv) {
                    return Ok(Value::Array(v));
                }
                let mut out = Vec::with_capacity(bv.len());
                for y in bv {
                    match self.eval_number_binary(op, scalar.clone(), y)? {
                        Value::Number(n) => out.push(Value::Number(n)),
                        _ => return crate::error::err("array operation result must be numeric"),
                    }
                }
                out
            }
            _ => return crate::error::err("invalid array operation"),
        };
        Ok(Value::Array(out))
    }

    pub(crate) fn scalar_for_broadcast(&self, v: Value) -> Result<Number, RuntimeError> {
        match v {
            Value::Number(n) => Ok(n),
            _ => crate::error::err("cannot broadcast with a non-numeric scalar"),
        }
    }

    /// SIMD-accelerated elementwise `array ⊕ array` when the active tier is `O3` and both arrays are
    /// dense `F64` (spec §10.2). Returns `None` to fall back to the scalar loop.
    pub(crate) fn try_simd_arrays(
        &self,
        op: BinOp,
        a: &[Number],
        b: &[Number],
    ) -> Option<Vec<Value>> {
        if self.current_config().opt_level < OptLevel::O3 {
            return None;
        }
        crate::simd::try_f64x4_arrays(op, a, b)
            .map(|nums| nums.into_iter().map(Value::Number).collect())
    }

    /// SIMD-accelerated elementwise `array ⊕ scalar` when the active tier is `O3` (spec §10.2).
    pub(crate) fn try_simd_scalar(
        &self,
        op: BinOp,
        a: &[Number],
        scalar: &Number,
    ) -> Option<Vec<Value>> {
        if self.current_config().opt_level < OptLevel::O3 {
            return None;
        }
        crate::simd::try_f64x4_scalar(op, a, scalar)
            .map(|nums| nums.into_iter().map(Value::Number).collect())
    }

    /// SIMD-accelerated elementwise `scalar ⊕ array` when the active tier is `O3` (spec §10.2).
    pub(crate) fn try_simd_scalar_left(
        &self,
        op: BinOp,
        scalar: &Number,
        b: &[Number],
    ) -> Option<Vec<Value>> {
        if self.current_config().opt_level < OptLevel::O3 {
            return None;
        }
        crate::simd::try_f64x4_scalar_left(op, scalar, b)
            .map(|nums| nums.into_iter().map(Value::Number).collect())
    }
}
