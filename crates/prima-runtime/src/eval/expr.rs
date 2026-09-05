//! Expression evaluation (spec §4.3/§6/§7/§9/§11): ground literals and f-strings, binary/unary
//! operators, membership and set algebra, broadcasting, comprehensions, comparison, and operator
//! overload dispatch. Statement evaluation lives in `stmt.rs`; calls in `call.rs`.

use super::helpers::{
    apply_spec, is_zero_literal, literal_value, number_mod, overload_key, path_key,
};
use super::*;

impl Evaluator {
    /// Evaluate one expression, attaching its source span to any error (spec §16.4).
    pub(crate) fn eval_expr(&mut self, env: &EnvRef, expr: &Expr) -> Result<Value, RuntimeError> {
        let span = expr.span;
        self.eval_expr_inner(env, expr)
            .map_err(|e| crate::error::attach_span(e, span))
    }

    pub(crate) fn eval_expr_inner(
        &mut self,
        env: &EnvRef,
        expr: &Expr,
    ) -> Result<Value, RuntimeError> {
        match &expr.kind {
            ExprKind::Literal(lit) => self.eval_literal(env, lit),
            ExprKind::FString(parts) => self.eval_fstring(env, parts),
            ExprKind::Symbol(s) => Ok(Value::Expr(self.pool.symbol(self.symbols.intern(&s.value)))),
            ExprKind::Path { segments } => {
                if segments.len() == 1 {
                    let name = &segments[0].value;
                    let env_r = env.borrow();
                    if let Some(v) = env_r.get_value(name) {
                        Ok(v)
                    } else if env_r.get_func(name).is_some() {
                        crate::error::err(format!("function `{name}` cannot be used as a value"))
                    } else {
                        Ok(Value::Expr(self.pool.symbol(self.symbols.intern(name))))
                    }
                } else {
                    // Module-qualified access (spec §15.2): `module::item`
                    let ns = path_key(&segments[..segments.len() - 1]);
                    let item = &segments[segments.len() - 1].value;
                    match env.borrow().lookup_module_item(&ns, item) {
                        Some(NamespaceItem::Val(v)) => Ok(v),
                        Some(NamespaceItem::Func(_)) => crate::error::err(format!(
                            "function `{item}` cannot be used as a value"
                        )),
                        Some(NamespaceItem::Class(_)) => {
                            crate::error::err(format!("class `{item}` cannot be used as a value"))
                        }
                        None => crate::error::err(format!("unknown module item `{ns}::{item}`")),
                    }
                }
            }
            ExprKind::Self_ => {
                // `self` resolves to the enclosing method's receiver: a class instance (spec §12.3)
                // for user classes, or a builtin-class value (`Value::String`/`Array`/..., spec §18.1).
                if let Some(id) = self.self_stack.last() {
                    Ok(Value::Class(*id))
                } else if let Some(v) = self.self_values.last() {
                    Ok(v.clone())
                } else {
                    Err(RuntimeError::Message("`self` outside of a method".into()))
                }
            }
            ExprKind::Call { callee, args } => self.eval_call(env, callee, args),
            ExprKind::MethodCall {
                receiver,
                name,
                args,
            } => self.eval_method_call(env, receiver, name, args),
            ExprKind::Field { receiver, name } => self.eval_field(env, receiver, name),
            ExprKind::StructLiteral { name, fields, base } => {
                self.eval_struct_literal(env, name, fields, base.as_deref())
            }
            ExprKind::Binary {
                op: BinOp::Broadcast,
                lhs,
                rhs,
            } => self.eval_broadcast_op(env, lhs, rhs),
            ExprKind::Binary { op, lhs, rhs } => {
                let a = self.eval_expr(env, lhs)?;
                let b = self.eval_expr(env, rhs)?;
                self.eval_binary(*op, a, b)
            }
            ExprKind::Unary { op, operand } => {
                let v = self.eval_expr(env, operand)?;
                self.eval_unary(*op, v)
            }
            ExprKind::Index { base, index } => self.eval_index(env, base, index),
            ExprKind::Try(inner) => {
                // `?` operator (spec §16.3): propagates `Err`/`None` as a runtime error (checked statically in `check`).
                let v = self.eval_expr(env, inner)?;
                match v {
                    Value::Result(Ok(v)) => Ok(*v),
                    Value::Result(Err(m)) => Err(RuntimeError::Message(m)),
                    Value::Option(Some(v)) => Ok(*v),
                    Value::Option(None) => {
                        Err(RuntimeError::Message("`?` on a `None` value".into()))
                    }
                    other => crate::error::err(format!(
                        "`?` expects a `Result` or `Option`, got {}",
                        value_type_name(&other)
                    )),
                }
            }
            ExprKind::Array(items) => {
                let elems: Result<Vec<Value>, RuntimeError> =
                    items.iter().map(|it| self.eval_expr(env, it)).collect();
                Ok(Value::Array(elems?))
            }
            ExprKind::Dict(entries) => {
                let mut d: HashMap<ValueKey, Value> = HashMap::new();
                for (k, v) in entries {
                    let kv = self.eval_expr(env, k)?;
                    let key = ValueKey::from_value(&kv).ok_or_else(|| {
                        RuntimeError::Message("dict key must be a hashable value".into())
                    })?;
                    d.insert(key, self.eval_expr(env, v)?);
                }
                Ok(Value::Dict(d))
            }
            ExprKind::Set(items) => {
                let mut s: HashSet<ValueKey> = HashSet::new();
                for it in items {
                    let v = self.eval_expr(env, it)?;
                    let key = ValueKey::from_value(&v).ok_or_else(|| {
                        RuntimeError::Message("set element must be a hashable value".into())
                    })?;
                    s.insert(key);
                }
                Ok(Value::Set(s))
            }
            ExprKind::Comprehension {
                kind,
                output,
                clauses,
            } => self.eval_comprehension(env, *kind, output, clauses),
            ExprKind::KeyValue { .. } => crate::error::err("internal error: stray key-value node"),
            ExprKind::Tuple(items) => {
                let vals: Result<Vec<Value>, RuntimeError> =
                    items.iter().map(|it| self.eval_expr(env, it)).collect();
                Ok(Value::Tuple(vals?))
            }
            ExprKind::Lambda { .. } => {
                crate::error::err("lambda must be assigned to a variable to be callable")
            }
            ExprKind::Match { scrutinee, arms } => self.eval_match(env, scrutinee, arms),
            ExprKind::Custom(_) => crate::error::err("`custom` config block is not valid here"),
        }
    }

    /// `@.` explicit broadcast operator (spec §11.4): not disabled by `broadcast := false`.
    pub(crate) fn eval_broadcast_op(
        &mut self,
        env: &EnvRef,
        lhs: &Expr,
        rhs: &Expr,
    ) -> Result<Value, RuntimeError> {
        let v = self.eval_expr(env, lhs)?;
        let func = match &rhs.kind {
            ExprKind::Path { segments } => self.resolve_func(env, segments).ok_or_else(|| {
                RuntimeError::Message(format!("unknown function `{}`", path_key(segments)))
            })?,
            _ => return crate::error::err("`@.` right-hand side must be a function"),
        };
        if matches!(v, Value::Array(_)) {
            self.broadcast_call(&func, vec![v], &[0])
        } else {
            self.apply_function(&func, vec![v])
        }
    }

    /// Comprehension evaluation (spec §11.7): iterate the clauses in order — `For` binds the variable
    /// in a child scope and iterates, `If` filters on a boolean condition — and accumulate the output
    /// expression at the deepest level. The frame kind decides the produced collection.
    pub(crate) fn eval_comprehension(
        &mut self,
        env: &EnvRef,
        kind: CompKind,
        output: &Expr,
        clauses: &[ComprehensionClause],
    ) -> Result<Value, RuntimeError> {
        let mut values: Vec<Value> = Vec::new();
        self.comprehension_clauses(env, clauses, kind, output, &mut values)?;
        match kind {
            CompKind::Array => Ok(Value::Array(values)),
            // Tuple comprehension is eager here (documented deviation from the spec's lazy generator).
            CompKind::Tuple => Ok(Value::Tuple(values)),
            CompKind::Set => {
                let mut s: HashSet<ValueKey> = HashSet::new();
                for v in values {
                    let key = ValueKey::from_value(&v).ok_or_else(|| {
                        RuntimeError::Message("set element must be a hashable value".into())
                    })?;
                    s.insert(key);
                }
                Ok(Value::Set(s))
            }
            CompKind::Dict => {
                let mut d: HashMap<ValueKey, Value> = HashMap::new();
                for v in values {
                    let Value::Tuple(pair) = v else {
                        unreachable!("dict comprehension leaf emits a key/value pair")
                    };
                    let key = ValueKey::from_value(&pair[0]).ok_or_else(|| {
                        RuntimeError::Message("dict key must be a hashable value".into())
                    })?;
                    d.insert(key, pair[1].clone());
                }
                Ok(Value::Dict(d))
            }
        }
    }

    /// Recurse over comprehension clauses (spec §11.7), in order; `For` and `If` may appear any number
    /// of times and interleave.
    pub(crate) fn comprehension_clauses(
        &mut self,
        env: &EnvRef,
        clauses: &[ComprehensionClause],
        kind: CompKind,
        output: &Expr,
        values: &mut Vec<Value>,
    ) -> Result<(), RuntimeError> {
        match clauses.split_first() {
            None => {
                if kind == CompKind::Dict {
                    // A Dict comprehension's output is the internal `key: value` node (spec §4.6).
                    let ExprKind::KeyValue { key, value } = &output.kind else {
                        return crate::error::err(
                            "dict comprehension output must be a `key: value` pair",
                        );
                    };
                    let k = self.eval_expr(env, key)?;
                    let v = self.eval_expr(env, value)?;
                    values.push(Value::Tuple(vec![k, v]));
                } else {
                    values.push(self.eval_expr(env, output)?);
                }
                Ok(())
            }
            Some((clause, rest)) => match clause {
                ComprehensionClause::For { var, iter } => {
                    let iter_v = self.eval_expr(env, iter)?;
                    let items = self.iter_values(&iter_v)?;
                    for item in items {
                        let scope = Env::child(env);
                        scope.borrow_mut().set_value(&var.value, item);
                        self.comprehension_clauses(&scope, rest, kind, output, values)?;
                    }
                    Ok(())
                }
                ComprehensionClause::If { cond } => {
                    let ok = match self.eval_expr(env, cond)? {
                        Value::Bool(b) => b,
                        _ => {
                            return crate::error::err(
                                "comprehension `if` condition must be a boolean",
                            );
                        }
                    };
                    if ok {
                        self.comprehension_clauses(env, rest, kind, output, values)
                    } else {
                        Ok(())
                    }
                }
            },
        }
    }

    /// Iteration protocol (spec §11.7): `Array` → elements, `Dict` → keys (deterministic order),
    /// `Set` → elements, `String` → `Char` per character, `Tuple` → elements.
    pub(crate) fn iter_values(&self, v: &Value) -> Result<Vec<Value>, RuntimeError> {
        match v {
            Value::Array(elems) => Ok(elems.clone()),
            Value::Dict(d) => Ok(self
                .sorted_dict_keys(d)
                .iter()
                .map(|k| k.to_value())
                .collect()),
            Value::Set(s) => Ok(self.sorted_set_values(s)),
            Value::String(s) => Ok(s.chars().map(Value::Char).collect()),
            Value::Tuple(items) => Ok(items.clone()),
            other => crate::error::err(format!("not iterable: {}", value_type_name(other))),
        }
    }

    pub(crate) fn eval_literal(
        &mut self,
        env: &EnvRef,
        lit: &Literal,
    ) -> Result<Value, RuntimeError> {
        match lit {
            Literal::Integer(s) => {
                let i = s
                    .parse::<BigInt>()
                    .map_err(|_| RuntimeError::Message("invalid integer literal".into()))?;
                Ok(Value::Number(Number::Integer(i)))
            }
            Literal::Hex(s) => {
                let i = BigInt::parse_bytes(&s.as_bytes()[2..], 16)
                    .ok_or_else(|| RuntimeError::Message("invalid hex literal".into()))?;
                Ok(Value::Number(Number::Integer(i)))
            }
            Literal::Binary(s) => {
                let i = BigInt::parse_bytes(&s.as_bytes()[2..], 2)
                    .ok_or_else(|| RuntimeError::Message("invalid binary literal".into()))?;
                Ok(Value::Number(Number::Integer(i)))
            }
            Literal::Float(s) => {
                let f = s
                    .parse::<f64>()
                    .map_err(|_| RuntimeError::Message("invalid float literal".into()))?;
                Ok(Value::Number(Number::from(f)))
            }
            Literal::String { value, .. } => Ok(Value::String(value.clone())),
            Literal::Char(c) => Ok(Value::Char(*c)),
            Literal::Bool(b) => Ok(Value::Bool(*b)),
            Literal::Tex(s) => {
                // A TeX literal is parsed into the same AST as ordinary syntax and evaluated uniformly (implementation plan §4.9).
                let tex_ast = prima_syntax::tex::parse_tex(s).map_err(syntax_err)?;
                self.eval_expr(env, &tex_ast)
            }
        }
    }

    /// f-string evaluation (spec §18.1): literal parts concatenate verbatim; each `{expr}`
    /// interpolation evaluates the expression, renders it with the active `print_format` (default
    /// LaTeX), and applies the optional `:spec` refinement (float precision, width/alignment).
    pub(crate) fn eval_fstring(
        &mut self,
        env: &EnvRef,
        parts: &[FStringPart],
    ) -> Result<Value, RuntimeError> {
        let mut out = String::new();
        for p in parts {
            match p {
                FStringPart::Literal(s) => out.push_str(s),
                FStringPart::Interp { expr, spec } => {
                    let v = self.eval_expr(env, expr)?;
                    let rendered = self.format_value(&v);
                    out.push_str(&apply_spec(&v, &rendered, spec.as_deref()));
                }
            }
        }
        Ok(Value::String(out))
    }

    /// `Undefined` strictness (spec §6.2): it must not participate in any operation; any input errors immediately (no propagation).
    pub(crate) fn ensure_defined(&self, v: &Value) -> Result<(), RuntimeError> {
        if matches!(v, Value::Undefined) {
            Err(RuntimeError::Undefined(
                "`Undefined` cannot participate in operations".into(),
            ))
        } else {
            Ok(())
        }
    }

    pub(crate) fn eval_binary(
        &mut self,
        op: BinOp,
        a: Value,
        b: Value,
    ) -> Result<Value, RuntimeError> {
        self.ensure_defined(&a)?;
        self.ensure_defined(&b)?;
        // Operator overload (spec §18.5): a class operand with a registered overload for this op dispatches to the method.
        if let Some(r) = self.try_overload_binary(op, &a, &b) {
            return r;
        }
        // `in` membership (spec §11.3/§11.6) and set algebra (spec §11.6) treat their operands as
        // containers, so they dispatch before the elementwise array path.
        match op {
            BinOp::In => return self.eval_in(a, b),
            BinOp::Union | BinOp::Intersect | BinOp::Difference => {
                return self.eval_set_algebra(op, a, b);
            }
            _ => {}
        }
        // Array arithmetic: `Array + Array` concatenates (spec §11.3, v2.1); the other operators
        // (and `Array ± scalar`) are elementwise broadcast (spec §11.4).
        if matches!(
            op,
            BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Pow | BinOp::Mod
        ) && (matches!(a, Value::Array(_)) || matches!(b, Value::Array(_)))
        {
            return self.eval_binary_array(op, a, b);
        }
        match op {
            BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Pow | BinOp::Mod => {
                match (a, b) {
                    (Value::Number(x), Value::Number(y)) => self.eval_number_binary(op, x, y),
                    (x, y) => {
                        let a_id = self.to_expr_id(&x)?;
                        let b_id = self.to_expr_id(&y)?;
                        let node = match op {
                            BinOp::Add => self.pool.add2(a_id, b_id),
                            BinOp::Sub => self.pool.sub2(a_id, b_id),
                            BinOp::Mul => self.pool.mul2(a_id, b_id),
                            BinOp::Div => self.pool.div2(a_id, b_id),
                            BinOp::Pow => self.pool.pow2(a_id, b_id),
                            BinOp::Mod => {
                                return crate::error::err("`%` requires numeric operands");
                            }
                            _ => unreachable!(),
                        };
                        let simp = self.simplify_current(node);
                        Ok(self.value_from_expr(simp))
                    }
                }
            }
            BinOp::And | BinOp::Or => match (a, b) {
                (Value::Bool(x), Value::Bool(y)) => {
                    Ok(Value::Bool(if op == BinOp::And { x && y } else { x || y }))
                }
                _ => crate::error::err("`&&`/`||` require boolean operands"),
            },
            BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
                self.eval_compare(op, a, b)
            }
            _ => crate::error::err("operator not supported"),
        }
    }

    /// `x in c` membership test (spec §11.3/§11.6): arrays test element equality, strings substring
    /// containment, dicts key presence, sets membership.
    pub(crate) fn eval_in(&mut self, a: Value, b: Value) -> Result<Value, RuntimeError> {
        match b {
            Value::Array(elems) => Ok(Value::Bool(elems.iter().any(|e| self.value_eq(&a, e)))),
            Value::Dict(d) => {
                let key = ValueKey::from_value(&a).ok_or_else(|| {
                    RuntimeError::Message("membership key must be a hashable value".into())
                })?;
                Ok(Value::Bool(d.contains_key(&key)))
            }
            Value::Set(s) => {
                let key = ValueKey::from_value(&a).ok_or_else(|| {
                    RuntimeError::Message("membership element must be a hashable value".into())
                })?;
                Ok(Value::Bool(s.contains(&key)))
            }
            Value::String(s) => match a {
                Value::String(x) => Ok(Value::Bool(s.contains(&x))),
                _ => crate::error::err("`in` on a string requires a string operand"),
            },
            other => crate::error::err(format!(
                "`in` requires a collection, got {}",
                value_type_name(&other)
            )),
        }
    }

    /// Set-algebra operators `∪`/`∩`/`\` (spec §11.6): both operands must be `Value::Set`.
    pub(crate) fn eval_set_algebra(
        &mut self,
        op: BinOp,
        a: Value,
        b: Value,
    ) -> Result<Value, RuntimeError> {
        let (Value::Set(x), Value::Set(y)) = (a, b) else {
            return crate::error::err("set operator requires two sets");
        };
        let out = match op {
            BinOp::Union => x.union(&y).cloned().collect(),
            BinOp::Intersect => x.intersection(&y).cloned().collect(),
            BinOp::Difference => x.difference(&y).cloned().collect(),
            _ => unreachable!(),
        };
        Ok(Value::Set(out))
    }

    /// Value equality used by membership/count/index (spec §11.3): numbers compare through the
    /// promotion tower, everything else structurally.
    pub fn value_eq(&self, a: &Value, b: &Value) -> bool {
        match (a, b) {
            (Value::Number(x), Value::Number(y)) => self.number_cmp(x, y) == Some(Ordering::Equal),
            _ => a == b,
        }
    }

    /// Try to dispatch a binary operator to a registered class overload (spec §18.5).
    pub(crate) fn try_overload_binary(
        &mut self,
        op: BinOp,
        a: &Value,
        b: &Value,
    ) -> Option<Result<Value, RuntimeError>> {
        let impl_op = match op {
            BinOp::Add => ImplOp::Add,
            BinOp::Sub => ImplOp::Sub,
            BinOp::Mul => ImplOp::Mul,
            BinOp::Div => ImplOp::Div,
            BinOp::Mod => ImplOp::Rem,
            BinOp::Eq | BinOp::Ne => ImplOp::Eq,
            BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => ImplOp::Cmp,
            _ => return None,
        };
        if !matches!(a, Value::Class(_)) && !matches!(b, Value::Class(_)) {
            return None;
        }
        // The class operand is the `self` receiver; the other operand is the argument (spec §18.5).
        let (self_v, other_v) = if matches!(a, Value::Class(_)) {
            (a.clone(), b.clone())
        } else {
            (b.clone(), a.clone())
        };
        let Value::Class(id) = &self_v else {
            return None;
        };
        let class = self.instances.get(id).map(|i| i.class.clone())?;
        if !self.overloads.contains_key(&overload_key(&class, impl_op)) {
            return None;
        }
        Some(self.overload_dispatch(&class, impl_op, self_v, vec![other_v]))
    }

    /// Dispatch an operator overload method: policy check (spec §13.2 `overload_policy`) then a method call.
    pub(crate) fn overload_dispatch(
        &mut self,
        class: &str,
        op: ImplOp,
        self_v: Value,
        args: Vec<Value>,
    ) -> Result<Value, RuntimeError> {
        let method = self
            .overloads
            .get(&overload_key(class, op))
            .cloned()
            .ok_or_else(|| {
                RuntimeError::Message(format!("no `{op:?}` overload registered for `{class}`"))
            })?;
        match self.current_config().overload_policy {
            OverloadPolicy::Deny => {
                return crate::error::err(format!(
                    "operator overload for `{class}` is denied by `overload_policy`"
                ));
            }
            OverloadPolicy::Warn => {
                self.push_warning(
                    "W0005",
                    Span::new(0, 0),
                    format!("operator overload in use (`{class}` `{op:?}`); `overload_policy := allow` to silence"),
                );
            }
            OverloadPolicy::Allow => {}
        }
        self.call_method(&method, self_v, args)
    }

    pub(crate) fn eval_number_binary(
        &mut self,
        op: BinOp,
        x: Number,
        y: Number,
    ) -> Result<Value, RuntimeError> {
        match op {
            BinOp::Add => Ok(Value::Number(x + y)),
            BinOp::Sub => Ok(Value::Number(x - y)),
            BinOp::Mul => Ok(Value::Number(x * y)),
            BinOp::Div => {
                // Exact-layer division by zero: `0/0` is evaluated by black magic under the custom policy (spec §13.4), otherwise the numeric layer errors (spec §6.2).
                if y.is_zero() && !matches!(y, Number::Real(_)) {
                    if x.is_zero()
                        && let Some(v) = self.custom_zero_div()
                    {
                        return Ok(v);
                    }
                    return crate::error::err("division by zero");
                }
                let r = x / y;
                // `fraction := false` (spec §13.3): the division result drops to F64.
                if self.current_config().fraction {
                    Ok(Value::Number(r))
                } else {
                    Ok(Value::Number(Number::Real(Real::F64(r.to_f64_lossy()))))
                }
            }
            BinOp::Pow => self.eval_pow(x, y),
            BinOp::Mod => Ok(Value::Number(number_mod(&x, &y)?)),
            _ => crate::error::err("arithmetic operator required"),
        }
    }

    pub(crate) fn eval_pow(&mut self, x: Number, y: Number) -> Result<Value, RuntimeError> {
        if let Some(r) = x.pow(&y) {
            return Ok(Value::Number(r));
        }
        // Negative base × fractional exponent (spec §6.5/§9.9): errors under `domain := real`; under `complex`, `(-1)^0.5 → \i`.
        let neg_base = !x.is_complex() && x.to_f64_lossy() < 0.0;
        let frac_exp = !y.is_complex() && !y.is_integer_value();
        if neg_base && frac_exp {
            match self.current_config().domain {
                Domain::Real => {
                    return Err(RuntimeError::Domain(
                        "negative base with a fractional exponent requires `domain := complex`"
                            .into(),
                    ));
                }
                Domain::Complex if y.to_f64_lossy() == 0.5 => {
                    let m = x.to_f64_lossy().abs().sqrt();
                    return Ok(Value::Number(Number::Complex {
                        re: Box::new(Number::from(0)),
                        im: Box::new(Number::Real(Real::F64(m))),
                    }));
                }
                _ => {}
            }
        }
        // Preserve the symbolic form (spec §8.3 levels 0/2).
        let a = self.pool.number(&x);
        let b = self.pool.number(&y);
        let node = self.pool.pow2(a, b);
        let simp = self.simplify_current(node);
        Ok(self.value_from_expr(simp))
    }

    /// `undefined_handling := custom { 0/0 := v }` (spec §13.4): literal values are returned directly.
    pub(crate) fn custom_zero_div(&self) -> Option<Value> {
        let cfg = self.current_config();
        if cfg.undefined_handling != UndefinedHandling::Custom {
            return None;
        }
        for (p, v) in &cfg.custom_rules {
            if let ExprKind::Binary {
                op: BinOp::Div,
                lhs,
                rhs,
            } = &p.kind
                && is_zero_literal(lhs)
                && is_zero_literal(rhs)
            {
                return literal_value(v);
            }
        }
        None
    }

    pub(crate) fn eval_compare(
        &mut self,
        op: BinOp,
        a: Value,
        b: Value,
    ) -> Result<Value, RuntimeError> {
        use std::cmp::Ordering;
        // Collection deep equality (spec §11.3/§11.6): arrays elementwise, dicts/sets by canonical
        // key. Ordering comparisons on collections are rejected.
        match (&a, &b) {
            (Value::Array(x), Value::Array(y)) => {
                let eq = x.len() == y.len() && x.iter().zip(y).all(|(u, v)| self.value_eq(u, v));
                return Ok(Value::Bool(match op {
                    BinOp::Eq => eq,
                    BinOp::Ne => !eq,
                    _ => return crate::error::err("cannot order arrays"),
                }));
            }
            (Value::Dict(x), Value::Dict(y)) => {
                let eq = self.dict_eq(x, y);
                return Ok(Value::Bool(match op {
                    BinOp::Eq => eq,
                    BinOp::Ne => !eq,
                    _ => return crate::error::err("cannot order dicts"),
                }));
            }
            (Value::Set(_), Value::Set(_)) => {
                let eq = a == b;
                return Ok(Value::Bool(match op {
                    BinOp::Eq => eq,
                    BinOp::Ne => !eq,
                    _ => return crate::error::err("cannot order sets"),
                }));
            }
            _ => {}
        }
        let ord = match (a, b) {
            (Value::Number(x), Value::Number(y)) => {
                // Promote to a common type before comparing (spec §6.4), so `1 == 1.0` holds.
                let (x, y) = prima_core::number::promote(&x, &y);
                match (x, y) {
                    (Number::Integer(x), Number::Integer(y)) => Some(x.cmp(&y)),
                    (Number::Rational(x), Number::Rational(y)) => Some(x.cmp(&y)),
                    (Number::Real(Real::F32(x)), Number::Real(Real::F32(y))) => x.partial_cmp(&y),
                    (Number::Real(Real::F64(x)), Number::Real(Real::F64(y))) => x.partial_cmp(&y),
                    _ => None,
                }
                .ok_or_else(|| RuntimeError::Message("cannot compare these numbers".into()))?
            }
            (Value::String(x), Value::String(y)) => {
                return Ok(Value::Bool(match op {
                    BinOp::Eq => x == y,
                    BinOp::Ne => x != y,
                    BinOp::Lt => x < y,
                    BinOp::Le => x <= y,
                    BinOp::Gt => x > y,
                    BinOp::Ge => x >= y,
                    _ => false,
                }));
            }
            (Value::Bool(x), Value::Bool(y)) => {
                return Ok(Value::Bool(match op {
                    BinOp::Eq => x == y,
                    BinOp::Ne => x != y,
                    _ => false,
                }));
            }
            _ => return crate::error::err("cannot compare values"),
        };
        let b = match op {
            BinOp::Eq => ord == Ordering::Equal,
            BinOp::Ne => ord != Ordering::Equal,
            BinOp::Lt => ord == Ordering::Less,
            BinOp::Le => ord != Ordering::Greater,
            BinOp::Gt => ord == Ordering::Greater,
            BinOp::Ge => ord != Ordering::Less,
            _ => false,
        };
        Ok(Value::Bool(b))
    }

    pub(crate) fn eval_unary(&mut self, op: UnOp, v: Value) -> Result<Value, RuntimeError> {
        self.ensure_defined(&v)?;
        match op {
            UnOp::Neg => {
                if let Value::Class(id) = &v
                    && let Some(class) = self.instances.get(id).map(|i| i.class.clone())
                {
                    if self
                        .overloads
                        .contains_key(&overload_key(&class, ImplOp::Neg))
                    {
                        return self.overload_dispatch(&class, ImplOp::Neg, v, vec![]);
                    }
                    return crate::error::err("cannot negate a class instance");
                }
                match v {
                    Value::Number(n) => Ok(Value::Number(-n)),
                    // Elementwise negation (spec §11.4): every element must be numeric.
                    Value::Array(elems) => {
                        let mut out = Vec::with_capacity(elems.len());
                        for e in elems {
                            match e {
                                Value::Number(n) => out.push(Value::Number(-n)),
                                _ => {
                                    return crate::error::err(
                                        "cannot negate a non-numeric array element",
                                    );
                                }
                            }
                        }
                        Ok(Value::Array(out))
                    }
                    Value::Expr(id) => {
                        let node = self.pool.mul2(self.pool.integer(-1), id);
                        let simp = self.simplify_current(node);
                        Ok(self.value_from_expr(simp))
                    }
                    _ => crate::error::err("cannot negate this value"),
                }
            }
            UnOp::Not => match v {
                Value::Bool(b) => Ok(Value::Bool(!b)),
                _ => crate::error::err("`!` requires a boolean"),
            },
            UnOp::Pos => Ok(v),
        }
    }
}
