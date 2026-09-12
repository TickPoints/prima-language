//! Statement and control-flow evaluation (spec §4.4/§4.8/§14): block scoping, statements,
//! collection lvalues + write-back, parallel `for` loops, guards, and the arithmetic-sum loop
//! optimization. Ground-expression evaluation lives in `expr.rs`; pattern matching in `pattern.rs`.

use super::helpers::{
    ParforStep, ParforWriteVec, MAX_RANGE_ELEMS, check_parfor_body, collect_read_names,
    expr_is_side_effect_free, normalize_index, overload_key, pattern_is_refutable,
    unalias_array_cycles, value_contains_array_buffer,
};
use super::*;

impl Evaluator {
    pub(crate) fn eval_block(&mut self, env: &EnvRef, block: &Block) -> Result<Flow, RuntimeError> {
        let scope = Env::child(env);
        self.eval_block_stmts(&scope, block)
    }

    pub(crate) fn eval_block_stmts(
        &mut self,
        env: &EnvRef,
        block: &Block,
    ) -> Result<Flow, RuntimeError> {
        for stmt in &block.stmts {
            match self.eval_stmt(env, stmt)? {
                Flow::Continue => {}
                flow @ Flow::Return(_) => return Ok(flow),
            }
        }
        Ok(Flow::Continue)
    }

    /// Evaluate a function/method body with implicit tail-return of the last expression statement
    /// (spec §4.5 method examples such as `get_a`/`new` end in a bare expression).
    pub(crate) fn eval_block_tail(
        &mut self,
        env: &EnvRef,
        block: &Block,
    ) -> Result<Value, RuntimeError> {
        let n = block.stmts.len();
        for (i, stmt) in block.stmts.iter().enumerate() {
            if i == n - 1
                && let Stmt::Expr(e) = stmt
            {
                return self.eval_expr(env, e);
            }
            match self.eval_stmt(env, stmt)? {
                Flow::Continue => {}
                Flow::Return(v) => return Ok(v),
            }
        }
        Ok(Value::Nil)
    }

    /// Evaluate one statement, attaching its source span to any error (spec §16.4).
    pub(crate) fn eval_stmt(&mut self, env: &EnvRef, stmt: &Stmt) -> Result<Flow, RuntimeError> {
        let span = stmt_span(stmt);
        // Cancellation boundary (host interruption): one relaxed flag load per statement keeps
        // runaway statement streams and non-looping recursion responsive.
        self.check_cancelled()
            .map_err(|e| crate::error::attach_span(e, span))?;
        self.eval_stmt_inner(env, stmt)
            .map_err(|e| crate::error::attach_span(e, span))
    }

    pub(crate) fn eval_stmt_inner(
        &mut self,
        env: &EnvRef,
        stmt: &Stmt,
    ) -> Result<Flow, RuntimeError> {
        match stmt {
            Stmt::Let { pat, value, .. } => {
                // `let name = lambda` binds a function (spec §11.1); other patterns destructure the value (spec §4.4).
                if let Pattern::Binding(name) = pat
                    && let ExprKind::Lambda { params, body } = &value.kind
                {
                    let f = Function::User {
                        params: params.clone(),
                        body: (**body).clone(),
                        env: Rc::clone(env),
                        parallel: false,
                        hot: Arc::new(HotState::new(false)),
                    };
                    env.borrow_mut().set_func(&name.value, f);
                    return Ok(Flow::Continue);
                }
                let v = self.eval_expr(env, value)?;
                // `let` accepts only irrefutable patterns (spec §4.4; `E0053`).
                if pattern_is_refutable(pat) {
                    return crate::error::err("refutable pattern in `let`");
                }
                let bindings = self
                    .match_pattern(env, &v, pat)
                    .ok_or_else(|| RuntimeError::Message("refutable pattern in `let`".into()))?;
                let mut e = env.borrow_mut();
                for (name, val) in bindings {
                    e.set_value(&name, val);
                }
                Ok(Flow::Continue)
            }
            Stmt::Const { name, value, .. } => {
                let v = self.eval_expr(env, value)?;
                env.borrow_mut().set_value(&name.value, v);
                Ok(Flow::Continue)
            }
            Stmt::MathDef {
                name,
                params,
                annotations,
                body,
                ..
            } => {
                let parallel = annotations.contains(&Annotation::Parallel);
                let force = annotations.contains(&Annotation::Jit);
                let f = Function::User {
                    params: params.clone(),
                    body: body.clone(),
                    env: Rc::clone(env),
                    parallel,
                    hot: Arc::new(HotState::new(force)),
                };
                env.borrow_mut().set_func(&name.value, f);
                Ok(Flow::Continue)
            }
            Stmt::FnDef {
                name,
                params,
                ret,
                annotations,
                body,
                ..
            } => {
                // `@builtin fn` (spec §18.4): bind, in order, to the core builtin of the same name,
                // then to a registered stdlib implementation keyed `"<module>::<name>"`; unregistered → E0055.
                // A `@builtin(ON)` tier produces a layered function (native fast path + `.pra` fallback).
                if annotations.iter().any(|a| a.is_builtin()) {
                    let level = annotations
                        .iter()
                        .map(|a| a.builtin_level())
                        .max()
                        .unwrap_or(0);
                    let f =
                        self.bind_builtin_annotated(&name.value, level, params, ret, body, env)?;
                    env.borrow_mut().set_func(&name.value, f);
                    Ok(Flow::Continue)
                } else {
                    let f = Function::Host {
                        params: params.clone(),
                        ret: ret.clone(),
                        body: body.clone(),
                        env: Rc::clone(env),
                        vm: Rc::new(std::sync::OnceLock::new()),
                    };
                    env.borrow_mut().set_func(&name.value, f);
                    Ok(Flow::Continue)
                }
            }
            Stmt::ClassDef {
                name,
                members,
                docs,
                ..
            } => {
                let def = self.build_class_def(name, members, docs.as_ref(), env)?;
                self.register_class(def);
                Ok(Flow::Continue)
            }
            Stmt::Impl {
                op,
                target,
                members,
                ..
            } => {
                // Operator overload methods (spec §18.5): `impl ops::Add for T { fn add(self, ...) { ... } }`.
                for m in members {
                    match m.as_ref() {
                        Stmt::FnDef {
                            params, ret, body, ..
                        } => {
                            let def = MethodDef {
                                params: params.clone(),
                                ret: ret.clone(),
                                body: Some(body.clone()),
                                native: None,
                                level: 0,
                                nature: MethodNature::Plain,
                                vis: Visibility::Public,
                                env: Rc::clone(env),
                                docs: None,
                            };
                            self.overloads.insert(overload_key(&target.value, *op), def);
                        }
                        Stmt::MathDef {
                            params, ret, body, ..
                        } => {
                            let block = Block {
                                stmts: vec![Stmt::Expr(body.clone())],
                                span: body.span,
                            };
                            let def = MethodDef {
                                params: params.clone(),
                                ret: ret.clone(),
                                body: Some(block),
                                native: None,
                                level: 0,
                                nature: MethodNature::Plain,
                                vis: Visibility::Public,
                                env: Rc::clone(env),
                                docs: None,
                            };
                            self.overloads.insert(overload_key(&target.value, *op), def);
                        }
                        _ => {
                            return crate::error::err(
                                "`impl` body must contain function definitions",
                            );
                        }
                    }
                }
                Ok(Flow::Continue)
            }
            Stmt::Expr(e) => {
                self.eval_expr(env, e)?;
                Ok(Flow::Continue)
            }
            Stmt::Assign {
                target, op, value, ..
            } => {
                let v = self.eval_expr(env, value)?;
                // Collection element/slice assignment `A[i] = v` / `d[k] = v` / `A[lo..hi] = v`
                // (spec §11.3/§11.6): array slots mutate the binding's buffer in place
                // (copy-on-write when the handle is aliased, spec §11.3 value semantics); dicts
                // write back the whole value.
                if let ExprKind::Index { base, index } = &target.kind {
                    let name = match &base.kind {
                        ExprKind::Path { segments } if segments.len() == 1 => {
                            segments[0].value.clone()
                        }
                        _ => return crate::error::err("assignment target must be a variable"),
                    };
                    let base_v = self.eval_expr(env, base)?;
                    match base_v {
                        Value::Dict(mut d) => {
                            if index.items.len() != 1 {
                                return crate::error::err(
                                    "multi-dimensional indexing is not supported yet",
                                );
                            }
                            let k = match &index.items[0] {
                                IndexItem::Elem(e) => self.eval_expr(env, e)?,
                                IndexItem::Slice { .. } => {
                                    return crate::error::err("cannot slice-assign a dict");
                                }
                            };
                            let key = ValueKey::from_value(&k).ok_or_else(|| {
                                RuntimeError::Message(
                                    "dict key must be a hashable value".into(),
                                )
                            })?;
                            let merged = match op {
                                AssignOp::Assign => v,
                                AssignOp::AddAssign => self.eval_binary(
                                    BinOp::Add,
                                    d.get(&key)
                                        .cloned()
                                        .unwrap_or(Value::Number(Number::from(0))),
                                    v,
                                )?,
                                AssignOp::SubAssign => self.eval_binary(
                                    BinOp::Sub,
                                    d.get(&key)
                                        .cloned()
                                        .unwrap_or(Value::Number(Number::from(0))),
                                    v,
                                )?,
                            };
                            d.insert(key, merged);
                            self.write_back(env, &name, Value::Dict(d));
                            return Ok(Flow::Continue);
                        }
                        Value::Array(a) => {
                            if index.items.len() != 1 {
                                return crate::error::err(
                                    "multi-dimensional indexing is not supported yet",
                                );
                            }
                            match &index.items[0] {
                                IndexItem::Elem(e) => {
                                    let raw = self.eval_index_i64(env, e)?;
                                    let (len, idx, old) = {
                                        let idx = normalize_index(raw, a.len()).ok_or_else(
                                            || {
                                                RuntimeError::IndexOutOfBounds(format!(
                                                    "index {raw} (length {})",
                                                    a.len()
                                                ))
                                            },
                                        )?;
                                        let old = a.get(idx);
                                        (a.len(), idx, old)
                                    };
                                    // A class operand can dispatch an operator overload (spec
                                    // §18.5) whose user code could re-enter the interpreter.
                                    let merged_is_pure = match op {
                                        AssignOp::Assign => true,
                                        _ => {
                                            !matches!(old, Some(Value::Class(_)))
                                                && !matches!(v, Value::Class(_))
                                        }
                                    };
                                    let merged = match op {
                                        AssignOp::Assign => v,
                                        AssignOp::AddAssign => self.eval_binary(
                                            BinOp::Add,
                                            old.ok_or_else(|| {
                                                RuntimeError::IndexOutOfBounds(format!(
                                                    "index {raw} (length {len})"
                                                ))
                                            })?,
                                            v,
                                        )?,
                                        AssignOp::SubAssign => self.eval_binary(
                                            BinOp::Sub,
                                            old.ok_or_else(|| {
                                                RuntimeError::IndexOutOfBounds(format!(
                                                    "index {raw} (length {len})"
                                                ))
                                            })?,
                                            v,
                                        )?,
                                    };
                                    // Storing an array that references the target buffer itself
                                    // would create a reference cycle; store a snapshot copy
                                    // instead (the whole-value write-back semantics).
                                    let mut merged = merged;
                                    if value_contains_array_buffer(&merged, &a) {
                                        unalias_array_cycles(&mut merged, &a);
                                    }
                                    if expr_is_side_effect_free(e) && merged_is_pure {
                                        // In-place fast path: neither the index expression nor the
                                        // merged-value computation can run user code, so the
                                        // binding still holds this array; mutate its buffer
                                        // through the chain (CoW when aliased).
                                        drop(a);
                                        let mut ok = false;
                                        env.borrow_mut().update_value(&name, |slot| {
                                            if let Value::Array(buf) = slot {
                                                buf.with_mut(|items| items[idx] = merged);
                                                ok = true;
                                            }
                                        });
                                        if !ok {
                                            return crate::error::err(
                                                "assignment target must be an array or dict",
                                            );
                                        }
                                    } else {
                                        // Conservative path: the index expression or the
                                        // merged-value computation may have rebound the binding;
                                        // restore the array read above, like the whole-value
                                        // write-back.
                                        let mut arr = a;
                                        arr.with_mut(|items| items[idx] = merged);
                                        self.write_back(env, &name, Value::Array(arr));
                                    }
                                    return Ok(Flow::Continue);
                                }
                                IndexItem::Slice { start, end } => {
                                    if !matches!(op, AssignOp::Assign) {
                                        return crate::error::err(
                                            "slice assignment only supports `=`",
                                        );
                                    }
                                    let Value::Array(rhs) = v else {
                                        return crate::error::err(
                                            "slice assignment right-hand side must be an array",
                                        );
                                    };
                                    let (lo, hi) =
                                        self.slice_bounds(env, start.as_ref(), end.as_ref(), a.len())?;
                                    let items = rhs.to_vec();
                                    let mut arr = a;
                                    arr.with_mut(|buf| {
                                        buf.splice(lo..hi, items);
                                    });
                                    self.write_back(env, &name, Value::Array(arr));
                                    return Ok(Flow::Continue);
                                }
                            }
                        }
                        other => {
                            return crate::error::err(format!(
                                "assignment target must be an array or dict, got {}",
                                value_type_name(&other)
                            ));
                        }
                    }
                }
                // Simple variable assignment (spec §4.2 examples: `s = 0`, `s += i`).
                let name = match &target.kind {
                    ExprKind::Path { segments } if segments.len() == 1 => &segments[0].value,
                    _ => return crate::error::err("assignment target must be a variable"),
                };
                let merged = {
                    let prev = env.borrow().get_value(name);
                    match op {
                        AssignOp::Assign => v,
                        AssignOp::AddAssign => self.eval_binary(
                            BinOp::Add,
                            prev.unwrap_or(Value::Number(Number::from(0))),
                            v,
                        )?,
                        AssignOp::SubAssign => self.eval_binary(
                            BinOp::Sub,
                            prev.unwrap_or(Value::Number(Number::from(0))),
                            v,
                        )?,
                    }
                };
                // Update in place along the shared chain (spec §12.2 shadowing); create locally if undefined.
                let mut e = env.borrow_mut();
                if !e.set_existing(name, merged.clone()) {
                    e.set_value(name, merged);
                }
                Ok(Flow::Continue)
            }
            Stmt::If {
                cond,
                then,
                elifs,
                else_,
                ..
            } => {
                if self.eval_cond(env, cond)? {
                    return self.eval_block(env, then);
                }
                for (c, b) in elifs {
                    if self.eval_cond(env, c)? {
                        return self.eval_block(env, b);
                    }
                }
                match else_ {
                    Some(b) => self.eval_block(env, b),
                    None => Ok(Flow::Continue),
                }
            }
            Stmt::IfLet {
                pat,
                value,
                then,
                else_,
                ..
            } => {
                let v = self.eval_expr(env, value)?;
                if let Some(bindings) = self.match_pattern(env, &v, pat) {
                    let scope = Env::child(env);
                    for (name, val) in bindings {
                        scope.borrow_mut().set_value(&name, val);
                    }
                    return self.eval_block_stmts(&scope, then);
                }
                match else_ {
                    Some(b) => self.eval_block(env, b),
                    None => Ok(Flow::Continue),
                }
            }
            Stmt::While { cond, body, .. } => {
                loop {
                    // Cancellation (host interruption) at the loop back-edge (spec §16).
                    self.check_cancelled()?;
                    if !self.eval_cond(env, cond)? {
                        break;
                    }
                    if let flow @ Flow::Return(_) = self.eval_block(env, body)? {
                        return Ok(flow);
                    }
                }
                Ok(Flow::Continue)
            }
            Stmt::WhileLet {
                pat, value, body, ..
            } => {
                loop {
                    self.check_cancelled()?;
                    let v = self.eval_expr(env, value)?;
                    let Some(bindings) = self.match_pattern(env, &v, pat) else {
                        break;
                    };
                    let scope = Env::child(env);
                    for (name, val) in bindings {
                        scope.borrow_mut().set_value(&name, val);
                    }
                    if let flow @ Flow::Return(_) = self.eval_block_stmts(&scope, body)? {
                        return Ok(flow);
                    }
                }
                Ok(Flow::Continue)
            }
            Stmt::Match {
                scrutinee, arms, ..
            } => {
                self.eval_match(env, scrutinee, arms)?;
                Ok(Flow::Continue)
            }
            Stmt::For {
                var,
                range,
                step,
                body,
                ..
            } => {
                // Loop formula optimization (spec §10/§19.1): closed form for the arithmetic series `for i in 0..n`/`1..n { acc += i }`.
                // Gated at `opt_level >= O1` (spec §10.2); `loop_optimization := false` disables it at any tier (spec §10.2).
                if step.is_none()
                    && self.current_config().loop_optimization
                    && self.current_config().opt_level >= OptLevel::O1
                    && let Some(()) = self.try_arithmetic_sum(env, var, range, body)?
                {
                    return Ok(Flow::Continue);
                }
                let start = self.eval_to_i64(env, &range.0)?;
                let end = self.eval_to_i64(env, &range.1)?;
                let step_v = match step {
                    Some(s) => self.eval_to_i64(env, s)?,
                    None => 1,
                };
                let mut i = start;
                while if step_v > 0 { i < end } else { i > end } {
                    // Cancellation (host interruption) at the loop back-edge (spec §16).
                    self.check_cancelled()?;
                    let scope = Env::child(env);
                    scope
                        .borrow_mut()
                        .set_value(&var.value, Value::Number(Number::from(i)));
                    if let flow @ Flow::Return(_) = self.eval_block_stmts(&scope, body)? {
                        return Ok(flow);
                    }
                    // Checked step (spec §16.1 R0001): an overflowing loop index is an error, not
                    // a silent wraparound.
                    i = i.checked_add(step_v).ok_or_else(|| {
                        RuntimeError::Overflow(format!(
                            "for-loop index overflow: {i} + {step_v}"
                        ))
                    })?;
                }
                Ok(Flow::Continue)
            }
            Stmt::Return { value, .. } => {
                let v = match value {
                    Some(e) => self.eval_expr(env, e)?,
                    None => Value::Nil,
                };
                Ok(Flow::Return(v))
            }
            Stmt::WithConfig { entries, body, .. } => {
                let mut cfg = self.current_config().clone();
                cfg.apply(entries)?;
                self.config.push(cfg);
                let r = self.eval_block(env, body);
                self.config.pop();
                r
            }
            Stmt::Pub(inner) => self.eval_stmt(env, inner),
            Stmt::ParFor {
                var,
                range,
                step,
                body,
                ..
            } => self.eval_parfor(env, var, range, step, body),
        }
    }

    /// Write a collection value back to its binding along the shared chain (spec §12.2 shadowing),
    /// creating the binding locally if it is undefined.
    pub(crate) fn write_back(&mut self, env: &EnvRef, name: &str, v: Value) {
        let mut e = env.borrow_mut();
        if !e.set_existing(name, v.clone()) {
            e.set_value(name, v);
        }
    }

    /// Compute the clamped `[lo, hi)` slice bounds (spec §11.3): both bounds may be negative and are
    /// clamped to `[0, len]`; `lo > hi` is an error.
    pub(crate) fn slice_bounds(
        &mut self,
        env: &EnvRef,
        start: Option<&Expr>,
        end: Option<&Expr>,
        len: usize,
    ) -> Result<(usize, usize), RuntimeError> {
        let len_i = len as i64;
        let raw_lo = match start {
            Some(e) => self.eval_index_i64(env, e)?,
            None => 0,
        };
        let raw_hi = match end {
            Some(e) => self.eval_index_i64(env, e)?,
            None => len_i,
        };
        let lo = if raw_lo < 0 {
            (len_i + raw_lo).max(0)
        } else {
            raw_lo.min(len_i)
        };
        let hi = if raw_hi < 0 {
            (len_i + raw_hi).max(0)
        } else {
            raw_hi.min(len_i)
        };
        if lo > hi {
            return crate::error::err(format!("invalid slice range {lo}..{hi} (length {len})"));
        }
        Ok((lo as usize, hi as usize))
    }

    /// `parfor` (spec §17.2): explicit parallel loop over a range. The body is statically checked to be
    /// side-effect free — only index-slot assignments (`A[i] = …`/`+=`) and pure function calls are allowed
    /// (`E0082`). Each iteration's new slot values are computed on rayon threads with independent evaluators,
    /// then the whole arrays are written back to their bindings in deterministic order.
    pub(crate) fn eval_parfor(
        &mut self,
        env: &EnvRef,
        var: &Spanned<String>,
        range: &(Expr, Expr),
        step: &Option<Expr>,
        body: &Block,
    ) -> Result<Flow, RuntimeError> {
        let start = self.eval_to_i64(env, &range.0)?;
        let end = self.eval_to_i64(env, &range.1)?;
        let step_v = match step {
            Some(s) => self.eval_to_i64(env, s)?,
            None => 1,
        };
        if step_v == 0 {
            return crate::error::err("parfor step cannot be zero");
        }
        let steps = check_parfor_body(body)?;
        for s in &steps {
            if let ParforStep::Eval(e) = s
                && !self.expr_is_pure_call(env, e)
            {
                return crate::error::err(
                    "parfor iteration body must only call pure functions (E0082)",
                );
            }
        }
        let cfg = self.current_config().clone();
        let var_name = var.value.clone();

        // Snapshot the arrays being written (spec §17.2): read once, write back once. The shared
        // array handles are cloned (O(1)) and read-only on the rayon threads.
        let mut arrays: HashMap<String, ArrayVal> = HashMap::new();
        let mut read_names: HashSet<String> = HashSet::new();
        read_names.insert(var_name.clone());
        for s in &steps {
            match s {
                ParforStep::Assign(w) => {
                    if !arrays.contains_key(&w.array) {
                        match env.borrow().get_value(&w.array) {
                            Some(Value::Array(a)) => {
                                arrays.insert(w.array.clone(), a);
                            }
                            _ => {
                                return crate::error::err(format!(
                                    "parfor target `{}` must be an array",
                                    w.array
                                ));
                            }
                        }
                    }
                    collect_read_names(&w.index, &mut read_names);
                    collect_read_names(&w.value, &mut read_names);
                }
                ParforStep::Eval(e) => collect_read_names(e, &mut read_names),
            }
        }
        let arrays_ro = arrays.clone();

        // Iteration count (closed form, same sequence as the sequential `for` loop, spec §17.2).
        // Computed in `i128`: `end - start` can overflow i64, and the count must never wrap or
        // become negative (spec §16.1 R0001).
        let n_i128: i128 = if step_v > 0 {
            if start >= end {
                0
            } else {
                ((end as i128 - start as i128 - 1) / step_v as i128) + 1
            }
        } else if start <= end {
            0
        } else {
            ((start as i128 - end as i128 - 1) / -(step_v as i128)) + 1
        };
        // Resource limit (OOM guard): `parfor` eagerly materializes the index sequence before the
        // rayon dispatch; a runaway count must fail here instead of exhausting memory.
        if n_i128 > MAX_RANGE_ELEMS {
            return Err(RuntimeError::Overflow(format!(
                "parfor materializes {n_i128} iterations, exceeding the {MAX_RANGE_ELEMS} iteration limit"
            )));
        }
        let n = i64::try_from(n_i128).map_err(|_| {
            RuntimeError::Overflow("parfor iteration count overflows".into())
        })?;

        // Materialize the loop-index sequence, then process it in rayon chunks so each task evaluator
        // (and its read-only array bindings) is created once per thread rather than once per element.
        let n_usize = usize::try_from(n).map_err(|_| {
            RuntimeError::Overflow("parfor iteration count overflows".into())
        })?;
        let mut indices = Vec::with_capacity(n_usize);
        if step_v > 0 {
            let mut i = start;
            while i < end {
                indices.push(i);
                i = i.checked_add(step_v).ok_or_else(|| {
                    RuntimeError::Overflow(format!("parfor index overflow: {i} + {step_v}"))
                })?;
            }
        } else {
            let mut i = start;
            while i > end {
                indices.push(i);
                i = i.checked_add(step_v).ok_or_else(|| {
                    RuntimeError::Overflow(format!("parfor index overflow: {i} + {step_v}"))
                })?;
            }
        }

        let steps_owned = steps.clone();
        let arrays_ro_c = arrays_ro.clone();
        // Cancellation (host interruption): check before the rayon dispatch so an already-cancelled
        // run never starts the parallel work.
        self.check_cancelled()?;
        let chunk = (indices.len().max(1) / rayon::current_num_threads().max(1)).max(1);
        // Pre-resolve the read-only outer values so task threads never touch the (non-`Send`) env chain.
        let outer_reads: HashMap<String, Value> = read_names
            .iter()
            .filter_map(|name| env.borrow().get_value(name).map(|v| (name.clone(), v)))
            .collect();
        let writes: Vec<Result<ParforWriteVec, RuntimeError>> = indices
            .par_chunks(chunk)
            .map(|chunk| {
                // Mark this rayon thread as a `parfor` task (spec §17.2): JIT fallback execution
                // is refused on it (the fallback dereferences the registering thread's `EnvRef`,
                // which is not thread-safe — see `prima_runtime::jit`).
                let _parfor_task = crate::jit::ParforTaskGuard::new();
                let mut ev = Evaluator::spawn_task_evaluator(&cfg);
                let call_env = Rc::new(RefCell::new(Env::new()));
                // Bind read-only outer values (including the target arrays as pre-loop snapshots) so the
                // body may read `A[j]`/outer scalars while writing independent slots.
                for (name, v) in &outer_reads {
                    call_env.borrow_mut().set_value(name, v.clone());
                }
                let mut out: Vec<(String, usize, Value)> = Vec::new();
                for &i in chunk {
                    // Cancellation is checked per index so a long `parfor` stops promptly.
                    if Evaluator::is_cancelled() {
                        return Err(RuntimeError::Message("interrupted".into()));
                    }
                    call_env
                        .borrow_mut()
                        .set_value(&var_name, Value::Number(Number::from(i)));
                    for s in &steps_owned {
                        match s {
                            ParforStep::Eval(e) => {
                                ev.eval_expr(&call_env, e)?;
                            }
                            ParforStep::Assign(w) => {
                                let idx = match ev.eval_expr(&call_env, &w.index)? {
                                    Value::Number(n) => n.as_usize().ok_or_else(|| {
                                        RuntimeError::Message(
                                            "parfor index must be a non-negative integer".into(),
                                        )
                                    })?,
                                    _ => {
                                        return Err(RuntimeError::Message(
                                            "parfor index must be an integer".into(),
                                        ));
                                    }
                                };
                                let nv = ev.eval_expr(&call_env, &w.value)?;
                                let merged = match w.op {
                                    AssignOp::Assign => nv,
                                    AssignOp::AddAssign | AssignOp::SubAssign => {
                                        let old = match arrays_ro_c
                                            .get(&w.array)
                                            .and_then(|a| a.get(idx))
                                        {
                                            Some(old) => old.clone(),
                                            None => {
                                                return Err(RuntimeError::IndexOutOfBounds(
                                                    format!(
                                                        "index {idx} (length {})",
                                                        arrays_ro_c
                                                            .get(&w.array)
                                                            .map(|a| a.len())
                                                            .unwrap_or(0)
                                                    ),
                                                ));
                                            }
                                        };
                                        let op = if w.op == AssignOp::AddAssign {
                                            BinOp::Add
                                        } else {
                                            BinOp::Sub
                                        };
                                        ev.eval_binary(op, old, nv)?
                                    }
                                };
                                out.push((w.array.clone(), idx, merged));
                            }
                        }
                    }
                }
                Ok(out)
            })
            .collect();

        // Deterministic merge: apply each index write to its array, bounds-checked (R0003).
        for r in writes {
            for (name, idx, val) in r? {
                let arr = arrays.get_mut(&name).expect("parfor arrays snapshot above");
                if idx >= arr.len() {
                    return Err(RuntimeError::IndexOutOfBounds(format!(
                        "index {idx} (length {})",
                        arr.len()
                    )));
                }
                arr.with_mut(|items| items[idx] = val);
            }
        }
        // Write the arrays back along the shared chain (spec §12.2), creating locally if undefined.
        let mut e = env.borrow_mut();
        for (name, arr) in arrays {
            if !e.set_existing(&name, Value::Array(arr.clone())) {
                e.set_value(&name, Value::Array(arr));
            }
        }
        Ok(Flow::Continue)
    }

    pub(crate) fn scalar_value(&self, v: Value) -> Result<Number, RuntimeError> {
        match v {
            Value::Number(n) => Ok(n),
            _ => crate::error::err("array elements must be numbers"),
        }
    }

    pub(crate) fn eval_cond(&mut self, env: &EnvRef, e: &Expr) -> Result<bool, RuntimeError> {
        match self.eval_expr(env, e)? {
            Value::Bool(b) => Ok(b),
            _ => crate::error::err("condition must be a boolean"),
        }
    }

    pub(crate) fn eval_to_i64(&mut self, env: &EnvRef, e: &Expr) -> Result<i64, RuntimeError> {
        match self.eval_expr(env, e)? {
            Value::Number(n) => n
                .as_i64()
                .ok_or_else(|| RuntimeError::Type(format!("loop range must be integers, got {n}"))),
            other => crate::error::err(format!("loop range must be integers, got {other:?}")),
        }
    }

    /// Closed form for an arithmetic sum (spec §10/§19.1): `for i in 0..n { acc += i }` → `n(n-1)/2`,
    /// `for i in 1..n { acc += i }` → `n(n+1)/2` (the 5050 result of the spec §19.1 example).
    pub(crate) fn try_arithmetic_sum(
        &mut self,
        env: &EnvRef,
        var: &Spanned<String>,
        range: &(Expr, Expr),
        body: &Block,
    ) -> Result<Option<()>, RuntimeError> {
        if body.stmts.len() != 1 {
            return Ok(None);
        }
        let addend_is_var = |e: &Expr| matches!(&e.kind, ExprKind::Path { segments } if segments.len() == 1 && segments[0].value == var.value);
        let acc = match &body.stmts[0] {
            Stmt::Assign {
                target,
                op: AssignOp::AddAssign,
                value,
                ..
            } if addend_is_var(value) => match &target.kind {
                ExprKind::Path { segments } if segments.len() == 1 => {
                    Some(segments[0].value.clone())
                }
                _ => None,
            },
            _ => None,
        };
        let Some(acc) = acc else { return Ok(None) };
        let start = self.eval_to_i64(env, &range.0)?;
        let end = self.eval_to_i64(env, &range.1)?;
        // Exact closed form (the loop accumulates exact `BigInt` sums, so the closed form must be
        // exact too — computed in `BigInt`, never a wrapped i64 product).
        let sum = if start == 0 && end > 0 {
            let e = BigInt::from(end);
            (&e * (&e - 1u8)) / 2u8
        } else if start == 1 && end >= 1 {
            let e = BigInt::from(end);
            (&e * (&e + 1u8)) / 2u8
        } else {
            return Ok(None);
        };
        let prev = env
            .borrow()
            .get_value(&acc)
            .unwrap_or(Value::Number(Number::from(0)));
        let merged = self.eval_binary(BinOp::Add, prev, Value::Number(Number::Integer(Box::new(sum))))?;
        let mut e = env.borrow_mut();
        if !e.set_existing(&acc, merged.clone()) {
            e.set_value(&acc, merged);
        }
        Ok(Some(()))
    }
}
