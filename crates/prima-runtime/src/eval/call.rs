//! Call evaluation (spec §10/§17/§18.4): function-call syntax, `calc` calls, JIT compilation and
//! fallback, higher-order functions, symbolic lowering, and name/module resolution.

use super::helpers::path_key;
use super::*;

impl Evaluator {
    pub(crate) fn eval_call(
        &mut self,
        env: &EnvRef,
        callee: &Expr,
        args: &[Expr],
    ) -> Result<Value, RuntimeError> {
        // Symbolic differentiation (spec §19.4): `derivative`/`partial`/`grad`/`limit` are intercepted
        // before generic argument evaluation, so the first argument may be an MFn *name* (functions are
        // not first-class values) as well as a symbolic expression.
        if let ExprKind::Path { segments } = &callee.kind
            && segments.len() == 1
            && let Some(Function::Builtin(b)) = self.resolve_func(env, segments)
            && matches!(
                b,
                Builtin::Derivative | Builtin::Partial | Builtin::Grad | Builtin::Limit
            )
        {
            return self.eval_calc_call(env, b, args);
        }
        // JIT compilation (spec §19.2): `jit(f)`/`jit(expr)`/`jit(grad(f))` are intercepted before generic
        // argument evaluation, so the argument may be an MFn *name* or a symbolic expression.
        if let ExprKind::Path { segments } = &callee.kind
            && segments.len() == 1
            && let Some(Function::Builtin(b)) = self.resolve_func(env, segments)
            && b == Builtin::Jit
        {
            return self.eval_jit_call(env, args);
        }
        // Higher-order convenience functions (spec appendix B.1): `map`/`filter`/`reduce` receive the
        // function as an un-evaluated expression (a name or a lambda), so they are intercepted before
        // generic argument evaluation, mirroring `derivative`.
        if let ExprKind::Path { segments } = &callee.kind
            && segments.len() == 1
            && let Some(Function::Builtin(b)) = self.resolve_func(env, segments)
            && matches!(b, Builtin::Map | Builtin::Filter | Builtin::Reduce)
        {
            return self.eval_higher_order(env, b, args);
        }
        // Class associated functions `T::name(args)` and `mod::T::name(args)` (spec §4.5).
        if let ExprKind::Path { segments } = &callee.kind {
            if let Some(v) = self.try_string_associated(env, segments, args)? {
                return Ok(v);
            }
            if segments.len() >= 2 {
                let (class_segs, method_seg) = segments.split_at(segments.len() - 1);
                if let Some(def) = self.resolve_class(env, class_segs) {
                    let mut arg_values = Vec::with_capacity(args.len());
                    for a in args {
                        arg_values.push(self.eval_expr(env, a)?);
                    }
                    return self.call_associated(&def, &method_seg[0].value, arg_values);
                }
            }
        }
        let mut arg_values = Vec::with_capacity(args.len());
        for a in args {
            arg_values.push(self.eval_expr(env, a)?);
        }
        let func = match &callee.kind {
            ExprKind::Path { segments } => {
                if let Some(f) = self.resolve_func(env, segments) {
                    f
                } else if segments.len() == 1
                    && let Some(Value::JitFunction(id)) = env.borrow().get_value(&segments[0].value)
                {
                    // `jit(...)` handle used as a callable (spec §19.2): dispatch through the registry.
                    return self.call_jit_function(id, arg_values);
                } else {
                    return Err(RuntimeError::Message(format!(
                        "unknown function `{}`",
                        path_key(segments)
                    )));
                }
            }
            _ => return crate::error::err("invalid function call"),
        };
        self.apply_function(&func, arg_values)
    }

    /// `derivative`/`partial`/`grad`/`limit` (spec §19.4): lower the argument expressions to the symbolic
    /// DAG, resolve the variable symbol, and delegate to `crate::diff`.
    pub(crate) fn eval_calc_call(
        &mut self,
        env: &EnvRef,
        b: Builtin,
        args: &[Expr],
    ) -> Result<Value, RuntimeError> {
        match b {
            Builtin::Derivative | Builtin::Partial => {
                if args.len() != 2 {
                    return crate::error::err("`derivative`/`partial` expect (expr, var)");
                }
                let expr = self.lower_symbolic(env, &args[0])?;
                let x = self.eval_var_symbol(env, &args[1])?;
                let d = crate::diff::derivative(self.pool, self.builtins, expr, x);
                Ok(self.value_from_expr(self.simplify_current(d)))
            }
            Builtin::Grad => {
                if args.len() != 1 {
                    return crate::error::err("`grad` expects (expr)");
                }
                let expr = self.lower_symbolic(env, &args[0])?;
                let grads = crate::diff::grad(self.pool, self.builtins, expr);
                let vals: Vec<Value> = grads
                    .into_iter()
                    .map(|g| self.value_from_expr(self.simplify_current(g)))
                    .collect();
                Ok(Value::Tuple(vals))
            }
            Builtin::Limit => {
                if args.len() != 3 {
                    return crate::error::err("`limit` expects (expr, var, value)");
                }
                let expr = self.lower_symbolic(env, &args[0])?;
                let x = self.eval_var_symbol(env, &args[1])?;
                let a_val = self.eval_expr(env, &args[2])?;
                let a = self.to_expr_id(&a_val)?;
                let lim = crate::diff::limit(self.pool, self.builtins, expr, x, a);
                Ok(self.value_from_expr(lim))
            }
            _ => unreachable!("eval_calc_call only handles the calc builtins"),
        }
    }

    /// `jit(...)` (spec §19.2/§19.4): compile a numeric scalar function or expression and return a
    /// `Value::JitFunction` handle. Accepts, in order: an MFn name (`jit(f)`), a gradient composition
    /// (`jit(grad(f))` with an MFn name → reverse-mode tape), a symbolic expression, or the symbolic
    /// tuple `grad(expr)` returns. Native compilation is opportunistic — a callable is registered with
    /// an interpreted fallback so it still works when `prima-jit` cannot compile the body.
    pub(crate) fn eval_jit_call(
        &mut self,
        env: &EnvRef,
        args: &[Expr],
    ) -> Result<Value, RuntimeError> {
        if args.len() != 1 {
            return crate::error::err("`jit` expects a single function or expression");
        }
        // `jit(f)` where `f` is an MFn name.
        if let ExprKind::Path { segments } = &args[0].kind
            && segments.len() == 1
            && let Some(Function::User {
                params,
                body,
                env: f_env,
                ..
            }) = self.resolve_func(env, segments)
        {
            let (dag, names) = self.body_dag(&params, &body, &f_env)?;
            let compiled = prima_jit::compile_scalar(self.pool, self.builtins, dag, &names);
            let id = crate::jit::register(crate::jit::JitCallable {
                params: names,
                n_out: 1,
                compiled,
                tape: None,
                fallback: Some((params, body, f_env)),
                expressions: None,
            });
            return Ok(Value::JitFunction(id));
        }
        // `jit(grad(f))` with an MFn name → reverse-mode gradient composition (spec §19.2/§19.4 stage 3).
        if let ExprKind::Call {
            callee,
            args: inner,
        } = &args[0].kind
            && inner.len() == 1
            && let ExprKind::Path {
                segments: callee_segs,
            } = &callee.kind
            && callee_segs.len() == 1
            && let Some(Function::Builtin(Builtin::Grad)) = self.resolve_func(env, callee_segs)
            && let ExprKind::Path {
                segments: inner_segs,
            } = &inner[0].kind
            && inner_segs.len() == 1
            && let Some(Function::User {
                params,
                body,
                env: f_env,
                ..
            }) = self.resolve_func(env, inner_segs)
        {
            let (dag, names) = self.body_dag(&params, &body, &f_env)?;
            let tape =
                crate::ad::Tape::build(self.pool, self.builtins, dag, &names).ok_or_else(|| {
                    RuntimeError::Message(
                        "`jit(grad(f))` requires a numeric-scalar function body".into(),
                    )
                })?;
            let id = crate::jit::register(crate::jit::JitCallable {
                params: names,
                n_out: params.len(),
                compiled: None,
                tape: Some(tape),
                fallback: None,
                expressions: None,
            });
            return Ok(Value::JitFunction(id));
        }
        // Otherwise evaluate the argument and dispatch on the resulting value.
        let v = self.eval_expr(env, &args[0])?;
        match v {
            Value::Expr(id) => {
                let syms = crate::diff::free_symbols(self.pool, self.builtins, id);
                let names: Vec<String> = syms
                    .iter()
                    .map(|s| self.symbols.name(*s).unwrap_or_default())
                    .collect();
                let compiled = prima_jit::compile_scalar(self.pool, self.builtins, id, &names);
                let n = crate::jit::register(crate::jit::JitCallable {
                    params: names.clone(),
                    n_out: 1,
                    compiled,
                    tape: None,
                    fallback: None,
                    // Symbolic fallback: the DAG itself, evaluated numerically per call when compilation is unavailable.
                    expressions: Some((vec![id], names)),
                });
                Ok(Value::JitFunction(n))
            }
            Value::Tuple(items)
                if !items.is_empty()
                    && items
                        .iter()
                        .all(|it| matches!(it, Value::Expr(_) | Value::Number(_))) =>
            {
                // `grad(expr)` returns a symbolic tuple (spec §19.4): register each component as an output.
                let ids: Vec<ExprId> = items
                    .iter()
                    .map(|it| self.to_expr_id(it))
                    .collect::<Result<_, _>>()?;
                let syms = crate::diff::free_symbols(self.pool, self.builtins, ids[0]);
                let names: Vec<String> = syms
                    .iter()
                    .map(|s| self.symbols.name(*s).unwrap_or_default())
                    .collect();
                let n_out = items.len();
                let n = crate::jit::register(crate::jit::JitCallable {
                    params: names.clone(),
                    n_out,
                    compiled: None,
                    tape: None,
                    fallback: None,
                    expressions: Some((ids, names)),
                });
                Ok(Value::JitFunction(n))
            }
            _ => crate::error::err(
                "`jit` argument must be a function, a symbolic expression, or a `grad(...)` result",
            ),
        }
    }

    /// `map`/`filter`/`reduce` (spec appendix B.1): the first argument is the function — a single-segment
    /// path resolving to a `Function` or a `Lambda` expression (evaluated to a `Function::User`); the
    /// remaining arguments are evaluated normally. These are explicit higher-order calls, so they do NOT
    /// apply the implicit-broadcast rules (`R0009`/`R0014`) of spec §11.4.
    pub(crate) fn eval_higher_order(
        &mut self,
        env: &EnvRef,
        b: Builtin,
        args: &[Expr],
    ) -> Result<Value, RuntimeError> {
        if args.len() < 2 {
            return crate::error::err("`map`/`filter`/`reduce` expect (func, array[, init])");
        }
        let f = match &args[0].kind {
            ExprKind::Path { segments } if segments.len() == 1 => {
                self.resolve_func(env, segments).ok_or_else(|| {
                    RuntimeError::Message(format!("unknown function `{}`", segments[0].value))
                })?
            }
            ExprKind::Lambda { params, body } => Function::User {
                params: params.clone(),
                body: (**body).clone(),
                env: Rc::clone(env),
                parallel: false,
                hot: Arc::new(HotState::new(false)),
            },
            _ => {
                return crate::error::err(
                    "`map`/`filter`/`reduce` first argument must be a function",
                );
            }
        };
        let Value::Array(elems) = self.eval_expr(env, &args[1])? else {
            return crate::error::err("`map`/`filter`/`reduce` second argument must be an array");
        };
        match b {
            Builtin::Map => {
                let mut out = Vec::with_capacity(elems.len());
                for e in elems {
                    out.push(self.apply_function(&f, vec![e])?);
                }
                Ok(Value::Array(out))
            }
            Builtin::Filter => {
                let mut out = Vec::new();
                for e in elems {
                    match self.apply_function(&f, vec![e.clone()])? {
                        Value::Bool(true) => out.push(e),
                        Value::Bool(false) => {}
                        _ => return crate::error::err("`filter` predicate must return a boolean"),
                    }
                }
                Ok(Value::Array(out))
            }
            Builtin::Reduce => {
                let init = args.get(2).ok_or_else(|| {
                    RuntimeError::Message("`reduce` expects (func, array, init)".into())
                })?;
                let mut acc = self.eval_expr(env, init)?;
                for e in elems {
                    acc = self.apply_function(&f, vec![acc, e])?;
                }
                Ok(acc)
            }
            _ => unreachable!("eval_higher_order only handles map/filter/reduce"),
        }
    }

    /// Lower an argument to a symbolic `ExprId` (spec §19.4): a single-segment path resolving to an MFn
    /// (`Function::User`) lowers the function body with each parameter bound to its symbol; anything else
    /// is evaluated normally and collapsed to the DAG.
    pub(crate) fn lower_symbolic(
        &mut self,
        env: &EnvRef,
        e: &Expr,
    ) -> Result<ExprId, RuntimeError> {
        if let ExprKind::Path { segments } = &e.kind
            && segments.len() == 1
            && let Some(Function::User {
                params,
                body,
                env: f_env,
                ..
            }) = self.resolve_func(env, segments)
        {
            let call_env = Env::child(&f_env);
            for p in params.iter() {
                let sym = self.pool.symbol(self.symbols.intern(&p.name.value));
                call_env
                    .borrow_mut()
                    .set_value(&p.name.value, Value::Expr(sym));
            }
            let v = self.eval_expr(&call_env, &body)?;
            return self.to_expr_id(&v);
        }
        let v = self.eval_expr(env, e)?;
        self.to_expr_id(&v)
    }

    /// Build the numeric-scalar DAG of an MFn body (spec §19.2): bind each parameter to its symbol in a
    /// child env of the function's defining env, evaluate the body, and return the DAG plus the
    /// parameter names. Mirrors `lower_symbolic` and is reused by the JIT hot path and `jit(...)`.
    pub(crate) fn body_dag(
        &mut self,
        params: &[Param],
        body: &Expr,
        f_env: &EnvRef,
    ) -> Result<(ExprId, Vec<String>), RuntimeError> {
        let call_env = Env::child(f_env);
        let mut names = Vec::with_capacity(params.len());
        for p in params.iter() {
            names.push(p.name.value.clone());
            let sym = self.pool.symbol(self.symbols.intern(&p.name.value));
            call_env
                .borrow_mut()
                .set_value(&p.name.value, Value::Expr(sym));
        }
        let v = self.eval_expr(&call_env, body)?;
        let dag = self.to_expr_id(&v)?;
        Ok((dag, names))
    }

    /// Attempt to compile an MFn body once (spec §19.2); `None` on any error (non-numeric body, …
    /// unknown free symbol), cached by the caller so it is never retried.
    pub(crate) fn try_compile_body(
        &mut self,
        params: &[Param],
        body: &Expr,
        f_env: &EnvRef,
    ) -> Option<Arc<prima_jit::CompiledScalar>> {
        let (dag, names) = self.body_dag(params, body, f_env).ok()?;
        prima_jit::compile_scalar(self.pool, self.builtins, dag, &names)
    }

    /// Dispatch a `Value::JitFunction(id)` call (spec §19.2): the arguments are already evaluated.
    pub(crate) fn call_jit_function(
        &mut self,
        id: u32,
        args: Vec<Value>,
    ) -> Result<Value, RuntimeError> {
        crate::jit::call(self, id, args)
    }

    /// Interpreted fallback for a registered JIT callable (spec §19.2): bind the parameters in a child
    /// env of the function's defining env and evaluate the body — the same path `apply_function` takes
    /// for a `Function::User`, kept so a `JitFunction` works even when native compilation is unavailable.
    pub(crate) fn apply_jit_fallback(
        &mut self,
        params: &[Param],
        body: &Expr,
        f_env: &EnvRef,
        args: Vec<Value>,
    ) -> Result<Value, RuntimeError> {
        if args.len() != params.len() {
            return crate::error::err(format!(
                "expected {} arguments, got {}",
                params.len(),
                args.len()
            ));
        }
        let call_env = Env::child(f_env);
        for (p, a) in params.iter().zip(args) {
            call_env.borrow_mut().set_value(&p.name.value, a);
        }
        self.eval_expr(&call_env, body)
    }

    /// Evaluate a variable argument to a `SymbolId` (spec §19.4): accepts a symbolic expression
    /// (`Value::Expr`/`Value::Symbol`) or a `String` naming the variable.
    pub(crate) fn eval_var_symbol(
        &mut self,
        env: &EnvRef,
        e: &Expr,
    ) -> Result<SymbolId, RuntimeError> {
        let v = self.eval_expr(env, e)?;
        match v {
            Value::Expr(id) => match self.pool.get(id) {
                Some(ExprData::Symbol(s)) => Ok(s),
                _ => crate::error::err("derivative variable must be a symbol"),
            },
            Value::Symbol(s) => Ok(SymbolId(s)),
            Value::String(name) => Ok(self.symbols.intern(&name)),
            _ => crate::error::err("derivative variable must be a symbol"),
        }
    }

    pub(crate) fn resolve_func(
        &self,
        env: &EnvRef,
        segments: &[Spanned<String>],
    ) -> Option<Function> {
        if segments.len() == 1 {
            env.borrow().get_func(&segments[0].value)
        } else {
            let ns = path_key(&segments[..segments.len() - 1]);
            match self.lookup_module_item_flat(env, &ns, &segments[segments.len() - 1].value) {
                Some(NamespaceItem::Func(f)) => Some(f),
                _ => None,
            }
        }
    }

    /// Look up a module item, flattening a nested namespace of any depth (spec §18.3): the exact
    /// module `a::b` item `c` wins; otherwise every prefix is tried as the module and the remainder
    /// plus the item as the flattened key — `time::Duration::from_secs` resolves as module `time`
    /// item `Duration::from_secs`, `linalg::Matrix::zeros` as module `linalg` item `Matrix::zeros`,
    /// `sys::path::join` as module `sys` item `path::join`. Shorter module prefixes are tried first.
    pub(crate) fn lookup_module_item_flat(
        &self,
        env: &EnvRef,
        ns: &str,
        item: &str,
    ) -> Option<NamespaceItem> {
        if let Some(it) = env.borrow().lookup_module_item(ns, item) {
            return Some(it);
        }
        let segments: Vec<&str> = ns.split("::").collect();
        for i in 1..segments.len() {
            let mod_key = segments[..i].join("::");
            let item_key = format!("{}::{item}", segments[i..].join("::"));
            if let Some(it) = env.borrow().lookup_module_item(&mod_key, &item_key) {
                return Some(it);
            }
        }
        None
    }

    /// Static purity check for a `parfor` effect call (spec §17.2, E0082): the top-level must be a call
    /// to a pure builtin (`Builtin::is_pure`) or an MFn (`Function::User`); host `fn` and `print` are rejected.
    pub(crate) fn expr_is_pure_call(&self, env: &EnvRef, e: &Expr) -> bool {
        match &e.kind {
            ExprKind::Call { callee, .. } => match &callee.kind {
                ExprKind::Path { segments } if segments.len() == 1 => {
                    match env.borrow().get_func(&segments[0].value) {
                        Some(Function::Builtin(b)) => b.is_pure(),
                        Some(Function::User { .. }) => true,
                        // Rust-hosted stdlib functions (spec §18/§18.4) may have side effects; never pure.
                        Some(Function::Native { .. }) => false,
                        _ => false,
                    }
                }
                _ => false,
            },
            _ => false,
        }
    }
}
