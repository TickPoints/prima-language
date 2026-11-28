//! Builtin method support for user classes and builtin values (spec §4.5/§11/§18).

use super::*;

impl Evaluator {
    /// Dispatch a builtin-class method call through its class definition (spec §18.1): a registered
    /// Rust fast path or `.pra` fallback body when the method declares one (layered `@builtin(ON)`,
    /// spec §18.4), otherwise the runtime backend. Errors are wrapped with the method's doc note
    /// (spec §16.4).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn dispatch_builtin_method(
        &mut self,
        env: &EnvRef,
        receiver: &Expr,
        class: &str,
        rcv: Value,
        name: &str,
        args: Vec<Value>,
        backend: Option<BuiltinBackend>,
    ) -> Result<Value, RuntimeError> {
        let Some(method) = self.builtin_method(class, name)? else {
            let err = RuntimeError::UnknownNativeMethod {
                class: class.to_string(),
                name: name.to_string(),
            };
            return Err(native_method_error(class, name, err));
        };
        // Registered impl and/or `.pra` body: `call_method` applies the `@builtin(ON)` layering.
        if method.body.is_some() || method.native.is_some() {
            return self
                .call_method(&method, rcv, args)
                .map_err(|e| native_method_error(class, name, e));
        }
        let backend = backend.ok_or_else(|| RuntimeError::UnregisteredBuiltin {
            target: format!("{class}::{name}"),
        })?;
        let result = match backend {
            BuiltinBackend::Array => {
                let Value::Array(a) = &rcv else {
                    return crate::error::type_err("expected an array receiver");
                };
                self.call_array_method(a, name, args)
            }
            BuiltinBackend::Dict => {
                let Value::Dict(d) = &rcv else {
                    return crate::error::type_err("expected a dict receiver");
                };
                self.call_dict_method(d, name, args)
            }
            BuiltinBackend::Set => {
                let Value::Set(s) = &rcv else {
                    return crate::error::type_err("expected a set receiver");
                };
                self.call_set_method(s, name, args)
            }
            BuiltinBackend::MutateArray => self.mutate_array(env, receiver, name, args),
            BuiltinBackend::MutateDict => self.mutate_dict(env, receiver, name, args),
            BuiltinBackend::MutateSet => self.mutate_set(env, receiver, name, args),
            BuiltinBackend::Char => {
                let Value::Char(c) = &rcv else {
                    return crate::error::type_err("expected a char receiver");
                };
                self.call_char_method(*c, name, args)
            }
            BuiltinBackend::Tuple => {
                let Value::Tuple(t) = &rcv else {
                    return crate::error::type_err("expected a tuple receiver");
                };
                self.call_tuple_method(t, name, args)
            }
            BuiltinBackend::Collapse | BuiltinBackend::CollapseNumber => {
                let collapse_name = if matches!(backend, BuiltinBackend::CollapseNumber) {
                    numeric_method_name(name)
                } else {
                    name.to_string()
                };
                let mut cargs = Vec::with_capacity(args.len() + 1);
                cargs.push(rcv);
                cargs.extend(args);
                crate::collapse::call(&collapse_name, &cargs, self.pool, self.builtins)
            }
        };
        result.map_err(|e| native_method_error(class, name, e))
    }

    /// Look up a builtin-class method, loading the class definition first (spec §18.1). `None` for an
    /// unknown method; loading fails with a runtime error when the standard library is missing.
    pub(crate) fn builtin_method(
        &mut self,
        class: &str,
        method: &str,
    ) -> Result<Option<MethodDef>, RuntimeError> {
        self.ensure_class(class)?;
        Ok(self
            .class_defs
            .get(class)
            .and_then(|d| d.methods.get(method))
            .cloned())
    }

    /// Builtin `Char` methods (spec §18.1): predicates, case mapping, and conversion. All are
    /// zero-argument; `to_upper`/`to_lower` map a single char (a multi-char expansion, e.g. `ß`,
    /// keeps only the first code point).
    pub(crate) fn call_char_method(
        &mut self,
        c: char,
        name: &str,
        args: Vec<Value>,
    ) -> Result<Value, RuntimeError> {
        if !args.is_empty() {
            return crate::error::type_err(format!(
                "`Char.{name}` takes no arguments, got {}",
                args.len()
            ));
        }
        match name {
            "len" => Ok(Value::Number(Number::from(1))),
            "is_digit" => Ok(Value::Bool(c.is_ascii_digit())),
            "is_alpha" => Ok(Value::Bool(c.is_alphabetic())),
            "is_alnum" => Ok(Value::Bool(c.is_alphanumeric())),
            "is_upper" => Ok(Value::Bool(c.is_uppercase())),
            "is_lower" => Ok(Value::Bool(c.is_lowercase())),
            "is_space" => Ok(Value::Bool(c.is_whitespace())),
            "is_ascii" => Ok(Value::Bool(c.is_ascii())),
            "to_upper" => Ok(Value::Char(c.to_uppercase().next().unwrap_or(c))),
            "to_lower" => Ok(Value::Char(c.to_lowercase().next().unwrap_or(c))),
            "to_string" => Ok(Value::String(c.to_string().into())),
            "code" => Ok(Value::Number(Number::from(u32::from(c) as i64))),
            _ => crate::error::type_err(format!("unknown `Char` method `{name}`")),
        }
    }

    /// Builtin `Tuple` methods (spec §18.1): `len`/`get`/`count`/`index`/`first`/`last`.
    pub(crate) fn call_tuple_method(
        &mut self,
        t: &[Value],
        name: &str,
        args: Vec<Value>,
    ) -> Result<Value, RuntimeError> {
        let arity = |n: usize| -> Result<(), RuntimeError> {
            if args.len() == n {
                Ok(())
            } else {
                crate::error::type_err(format!(
                    "`Tuple.{name}` expects {n} argument(s), got {}",
                    args.len()
                ))
            }
        };
        match name {
            "len" => {
                arity(0)?;
                Ok(Value::Number(Number::from(t.len() as i64)))
            }
            "get" => {
                arity(1)?;
                let i = match &args[0] {
                    Value::Number(n) => n.as_i64(),
                    _ => None,
                };
                let Some(i) = i else {
                    return crate::error::type_err("`Tuple.get` expects an integer index");
                };
                match normalize_index(i, t.len()) {
                    Some(i) => Ok(Value::Option(Some(Box::new(t[i].clone())))),
                    None => Ok(Value::Option(None)),
                }
            }
            "count" => {
                arity(1)?;
                Ok(Value::Number(Number::from(
                    t.iter().filter(|e| self.value_eq(e, &args[0])).count() as i64,
                )))
            }
            "index" => {
                arity(1)?;
                match t.iter().position(|e| self.value_eq(e, &args[0])) {
                    Some(i) => Ok(Value::Number(Number::from(i as i64))),
                    None => Err(RuntimeError::NotFound(("element not found").into())),
                }
            }
            "first" => {
                arity(0)?;
                Ok(t.first()
                    .map(|v| Value::Option(Some(Box::new(v.clone()))))
                    .unwrap_or(Value::Option(None)))
            }
            "last" => {
                arity(0)?;
                Ok(t.last()
                    .map(|v| Value::Option(Some(Box::new(v.clone()))))
                    .unwrap_or(Value::Option(None)))
            }
            _ => crate::error::type_err(format!("unknown `Tuple` method `{name}`")),
        }
    }

    /// `String::new()` / `String::from(x)` associated functions (spec §18.1).
    pub(crate) fn try_string_associated(
        &mut self,
        env: &EnvRef,
        segments: &[Spanned<String>],
        args: &[Expr],
    ) -> Result<Option<Value>, RuntimeError> {
        if segments.len() != 2 || segments[0].value != "String" {
            return Ok(None);
        }
        match segments[1].value.as_str() {
            "new" => {
                if !args.is_empty() {
                    return crate::error::type_err("`String::new` takes no arguments");
                }
                Ok(Some(Value::String(String::new().into())))
            }
            "from" => {
                if args.len() != 1 {
                    return crate::error::type_err("`String::from` expects 1 argument");
                }
                let v = self.eval_expr(env, &args[0])?;
                Ok(Some(Value::String(self.format_value(&v).into())))
            }
            _ => Ok(None),
        }
    }

    /// `get(array, index) -> Option<T>`: safe array access (spec §11.3); a negative index counts from
    /// the end, out-of-range yields `None`.
    pub(crate) fn call_array_get(&mut self, arr: Value, idx: Value) -> Result<Value, RuntimeError> {
        let Value::Array(a) = arr else {
            return crate::error::type_err("`get` expects an array");
        };
        let i = match idx {
            Value::Number(n) => n.as_i64(),
            _ => None,
        };
        let Some(i) = i else {
            return crate::error::type_err("`get` expects an integer index");
        };
        match normalize_index(i, a.len()) {
            Some(i) => Ok(Value::Option(Some(Box::new(
                a.get(i).unwrap_or(Value::Nil),
            )))),
            None => Ok(Value::Option(None)),
        }
    }

    /// Read-only array methods (spec §11.3): operate on the receiver's buffer through a read-only
    /// borrow; results are fresh values.
    pub(crate) fn call_array_method(
        &mut self,
        a: &ArrayVal,
        name: &str,
        args: Vec<Value>,
    ) -> Result<Value, RuntimeError> {
        let arity = |n: usize| -> Result<(), RuntimeError> {
            if args.len() == n {
                Ok(())
            } else {
                crate::error::type_err(format!(
                    "`Array.{name}` expects {n} argument(s), got {}",
                    args.len()
                ))
            }
        };
        match name {
            "len" => {
                arity(0)?;
                Ok(Value::Number(Number::from(a.len() as i64)))
            }
            "is_empty" => {
                arity(0)?;
                Ok(Value::Bool(a.is_empty()))
            }
            "get" => {
                arity(1)?;
                self.call_array_get(Value::Array(a.snapshot()), args[0].clone())
            }
            "contains" => {
                arity(1)?;
                Ok(Value::Bool(a.with(|items| {
                    items.iter().any(|e| self.value_eq(e, &args[0]))
                })))
            }
            "index" => {
                arity(1)?;
                match a.with(|items| items.iter().position(|e| self.value_eq(e, &args[0]))) {
                    Some(i) => Ok(Value::Number(Number::from(i as i64))),
                    None => Err(RuntimeError::NotFound(("element not found").into())),
                }
            }
            "count" => {
                arity(1)?;
                Ok(Value::Number(Number::from(
                    a.with(|items| items.iter().filter(|e| self.value_eq(e, &args[0])).count())
                        as i64,
                )))
            }
            "first" => {
                arity(0)?;
                Ok(a.with(|items| {
                    items
                        .first()
                        .map(|v| Value::Option(Some(Box::new(v.clone()))))
                        .unwrap_or(Value::Option(None))
                }))
            }
            "last" => {
                arity(0)?;
                Ok(a.with(|items| {
                    items
                        .last()
                        .map(|v| Value::Option(Some(Box::new(v.clone()))))
                        .unwrap_or(Value::Option(None))
                }))
            }
            "copy" => {
                arity(0)?;
                Ok(Value::Array(a.snapshot()))
            }
            _ => crate::error::type_err(format!("unknown `Array` method `{name}`")),
        }
    }

    /// Read-only dict methods (spec §11.6): `keys`/`values`/`items` return arrays in deterministic
    /// (canonical-key sorted) order.
    pub(crate) fn call_dict_method(
        &mut self,
        d: &HashMap<ValueKey, Value>,
        name: &str,
        args: Vec<Value>,
    ) -> Result<Value, RuntimeError> {
        let arity = |n: usize| -> Result<(), RuntimeError> {
            if args.len() == n {
                Ok(())
            } else {
                crate::error::type_err(format!(
                    "`Dict.{name}` expects {n} argument(s), got {}",
                    args.len()
                ))
            }
        };
        match name {
            "len" => {
                arity(0)?;
                Ok(Value::Number(Number::from(d.len() as i64)))
            }
            "get" => {
                arity(1)?;
                let key =
                    ValueKey::from_value(&args[0]).ok_or_else(|| RuntimeError::Unhashable {
                        role: crate::error::HashRole::DictKey,
                    })?;
                Ok(d.get(&key)
                    .map(|v| Value::Option(Some(Box::new(v.clone()))))
                    .unwrap_or(Value::Option(None)))
            }
            "contains" => {
                arity(1)?;
                let key =
                    ValueKey::from_value(&args[0]).ok_or_else(|| RuntimeError::Unhashable {
                        role: crate::error::HashRole::DictKey,
                    })?;
                Ok(Value::Bool(d.contains_key(&key)))
            }
            "keys" => {
                arity(0)?;
                Ok(Value::Array(
                    self.sorted_dict_keys(d)
                        .iter()
                        .map(|k| k.to_value())
                        .collect::<Vec<Value>>()
                        .into(),
                ))
            }
            "values" => {
                arity(0)?;
                Ok(Value::Array(
                    self.sorted_dict_keys(d)
                        .iter()
                        .map(|k| d[k].clone())
                        .collect::<Vec<Value>>()
                        .into(),
                ))
            }
            "items" => {
                arity(0)?;
                Ok(Value::Array(
                    self.sorted_dict_keys(d)
                        .iter()
                        .map(|k| Value::Tuple(vec![k.to_value(), d[k].clone()].into()))
                        .collect::<Vec<Value>>()
                        .into(),
                ))
            }
            _ => crate::error::type_err(format!("unknown `Dict` method `{name}`")),
        }
    }

    /// Read-only set methods (spec §11.6): `union`/`intersection`/`difference` return new sets.
    pub(crate) fn call_set_method(
        &mut self,
        s: &HashSet<ValueKey>,
        name: &str,
        args: Vec<Value>,
    ) -> Result<Value, RuntimeError> {
        let arity = |n: usize| -> Result<(), RuntimeError> {
            if args.len() == n {
                Ok(())
            } else {
                crate::error::type_err(format!(
                    "`Set.{name}` expects {n} argument(s), got {}",
                    args.len()
                ))
            }
        };
        match name {
            "len" => {
                arity(0)?;
                Ok(Value::Number(Number::from(s.len() as i64)))
            }
            "contains" => {
                arity(1)?;
                let key =
                    ValueKey::from_value(&args[0]).ok_or_else(|| RuntimeError::Unhashable {
                        role: crate::error::HashRole::SetElement,
                    })?;
                Ok(Value::Bool(s.contains(&key)))
            }
            "union" | "intersection" | "difference" | "symmetric_difference" => {
                arity(1)?;
                let Value::Set(other) = &args[0] else {
                    return crate::error::type_err("`Set.{name}` expects a set argument");
                };
                let out: HashSet<ValueKey> = match name {
                    "union" => s.union(other).cloned().collect(),
                    "intersection" => s.intersection(other).cloned().collect(),
                    "difference" => s.difference(other).cloned().collect(),
                    "symmetric_difference" => s.symmetric_difference(other).cloned().collect(),
                    _ => unreachable!(),
                };
                Ok(Value::Set(Box::new(out)))
            }
            "issubset" | "issuperset" | "isdisjoint" => {
                arity(1)?;
                let Value::Set(other) = &args[0] else {
                    return crate::error::type_err("`Set.{name}` expects a set argument");
                };
                let out = match name {
                    "issubset" => s.is_subset(other),
                    "issuperset" => s.is_superset(other),
                    "isdisjoint" => s.is_disjoint(other),
                    _ => unreachable!(),
                };
                Ok(Value::Bool(out))
            }
            "copy" => {
                arity(0)?;
                Ok(Value::Set(Box::new(s.clone())))
            }
            _ => crate::error::type_err(format!("unknown `Set` method `{name}`")),
        }
    }

    /// Deterministic key order for a dict (spec §11.6): sorted by the `format_value` of each key, with
    /// the key's debug rendering as a tiebreaker, so snapshots/tests are stable.
    pub(crate) fn sorted_dict_keys(&self, d: &HashMap<ValueKey, Value>) -> Vec<ValueKey> {
        let mut keys: Vec<ValueKey> = d.keys().cloned().collect();
        keys.sort_by(|a, b| {
            let ka = self.format_value(&a.to_value());
            let kb = self.format_value(&b.to_value());
            ka.cmp(&kb)
                .then_with(|| format!("{a:?}").cmp(&format!("{b:?}")))
        });
        keys
    }

    /// Deterministic element order for a set, sorted by `format_value` (spec §11.6).
    pub(crate) fn sorted_set_values(&self, s: &HashSet<ValueKey>) -> Vec<Value> {
        let mut elems: Vec<Value> = s.iter().map(|k| k.to_value()).collect();
        elems.sort_by(|a, b| {
            let ka = self.format_value(a);
            let kb = self.format_value(b);
            ka.cmp(&kb)
                .then_with(|| format!("{a:?}").cmp(&format!("{b:?}")))
        });
        elems
    }

    /// Structural dict equality (spec §11.6): same keys with promotion-equal values.
    pub(crate) fn dict_eq(
        &self,
        x: &HashMap<ValueKey, Value>,
        y: &HashMap<ValueKey, Value>,
    ) -> bool {
        if x.len() != y.len() {
            return false;
        }
        for (k, v) in x {
            match y.get(k) {
                Some(w) if self.value_eq(v, w) => {}
                _ => return false,
            }
        }
        true
    }
}
