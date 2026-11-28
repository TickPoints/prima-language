//! Mutation method support for user classes and builtin values (spec §4.5/§11/§18).

use super::*;

impl Evaluator {
    /// Mutating array methods (spec §11.3): the receiver must be a single-segment path (a variable
    /// binding); the binding's buffer is mutated in place through the chain (copy-on-write when
    /// the handle is aliased, spec §11.3 value semantics).
    pub(crate) fn mutate_array(
        &self,
        env: &EnvRef,
        receiver: &Expr,
        name: &str,
        args: Vec<Value>,
    ) -> Result<Value, RuntimeError> {
        let var = match &receiver.kind {
            ExprKind::Path { segments } if segments.len() == 1 => segments[0].value.clone(),
            _ => return crate::error::type_err("cannot mutate a temporary value"),
        };
        match env.borrow().get_value(&var) {
            Some(Value::Array(_)) => {}
            Some(_) => return crate::error::type_err("expected an array binding"),
            None => return Err(RuntimeError::UnknownVariable { name: var.clone() }),
        }
        let mut out: Option<Result<Value, RuntimeError>> = None;
        env.borrow_mut().update_value(&var, |slot| {
            if let Value::Array(a) = slot {
                out = Some(self.mutate_array_handle(a, name, args));
            }
        });
        out.unwrap_or_else(|| Err(RuntimeError::UnknownVariable { name: var.clone() }))
    }

    /// Apply a mutating `Array` method to an owned array handle in place (spec §11.3): the mutation
    /// runs through `with_mut` (copy-on-write when the handle is aliased). Shared by the AST
    /// receiver path (mutating through the binding) and the VM local-slot path. Payloads that
    /// reference the target buffer itself are replaced by snapshot copies so no reference cycle
    /// can form.
    pub(crate) fn mutate_array_handle(
        &self,
        arr: &mut ArrayVal,
        name: &str,
        args: Vec<Value>,
    ) -> Result<Value, RuntimeError> {
        let mut args = args;
        for a in &mut args {
            if value_contains_array_buffer(a, arr) {
                unalias_array_cycles(a, arr);
            }
        }
        arr.with_mut(|items| self.apply_array_mutation(items, name, &args))
    }

    /// The item-level application of a mutating array method over the element buffer (spec §11.3).
    /// Arity and index errors match the whole-value path exactly.
    pub(crate) fn apply_array_mutation(
        &self,
        arr: &mut Vec<Value>,
        name: &str,
        args: &[Value],
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
        let index = |v: &Value| -> Result<i64, RuntimeError> {
            match v {
                Value::Number(n) => n.as_i64().ok_or_else(|| {
                    RuntimeError::Type(format!("`Array.{name}` index must be an integer"))
                }),
                _ => crate::error::type_err(format!("`Array.{name}` index must be an integer")),
            }
        };
        match name {
            "push" => {
                arity(1)?;
                arr.push(args[0].clone());
                Ok(Value::Nil)
            }
            "pop" => {
                arity(0)?;
                Ok(arr
                    .pop()
                    .map(|v| Value::Option(Some(Box::new(v))))
                    .unwrap_or(Value::Option(None)))
            }
            "append" => {
                arity(1)?;
                arr.push(args[0].clone());
                Ok(Value::Nil)
            }
            "extend" => {
                arity(1)?;
                match &args[0] {
                    Value::Array(elems) => {
                        arr.extend(elems.to_vec());
                        Ok(Value::Nil)
                    }
                    other => crate::error::type_err(format!(
                        "`Array.extend` expects an array, got {}",
                        value_type_name(other)
                    )),
                }
            }
            "insert" => {
                arity(2)?;
                let i = index(&args[0])?;
                let i = normalize_insert(i, arr.len()).ok_or_else(|| {
                    RuntimeError::IndexOutOfBounds(format!("index {i} (length {})", arr.len()))
                })?;
                arr.insert(i, args[1].clone());
                Ok(Value::Nil)
            }
            "remove" => {
                arity(1)?;
                let i = index(&args[0])?;
                let i = normalize_index(i, arr.len()).ok_or_else(|| {
                    RuntimeError::IndexOutOfBounds(format!("index {i} (length {})", arr.len()))
                })?;
                Ok(arr.remove(i))
            }
            "clear" => {
                arity(0)?;
                arr.clear();
                Ok(Value::Nil)
            }
            // `sort` orders numeric elements only (spec §11.3); `reverse` works on any elements.
            "sort" => {
                arity(0)?;
                let mut nums = Vec::with_capacity(arr.len());
                for e in arr.iter() {
                    match e {
                        Value::Number(n) => nums.push(n.clone()),
                        _ => {
                            return crate::error::type_err(
                                "`Array.sort` requires an array of numbers",
                            );
                        }
                    }
                }
                nums.sort_by(|x, y| self.number_cmp(x, y).unwrap_or(Ordering::Equal));
                *arr = nums.into_iter().map(Value::Number).collect();
                Ok(Value::Nil)
            }
            "reverse" => {
                arity(0)?;
                arr.reverse();
                Ok(Value::Nil)
            }
            _ => crate::error::type_err(format!("unknown `Array` method `{name}`")),
        }
    }

    /// Mutating dict methods (spec §11.6): receiver must be a single-segment path; write-back pattern
    /// mirrors `mutate_array`.
    pub(crate) fn mutate_dict(
        &mut self,
        env: &EnvRef,
        receiver: &Expr,
        name: &str,
        args: Vec<Value>,
    ) -> Result<Value, RuntimeError> {
        let var = match &receiver.kind {
            ExprKind::Path { segments } if segments.len() == 1 => segments[0].value.clone(),
            _ => return crate::error::type_err("cannot mutate a temporary value"),
        };
        let cur = env
            .borrow()
            .get_value(&var)
            .ok_or_else(|| RuntimeError::UnknownVariable { name: var.clone() })?;
        let Value::Dict(mut d) = cur else {
            return crate::error::type_err("expected a dict binding");
        };
        let out = self.mutate_dict_handle(&mut d, name, args)?;
        self.write_back(env, &var, Value::Dict(d));
        Ok(out)
    }

    /// Apply a mutating `Dict` method to an already-resolved map (spec §11.6). Shared by the
    /// whole-value AST path (which writes the map back to its binding) and the bytecode VM (which
    /// mutates a local slot or an environment binding in place).
    pub(crate) fn mutate_dict_handle(
        &self,
        d: &mut HashMap<ValueKey, Value>,
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
        let key = |i: usize| -> Result<ValueKey, RuntimeError> {
            args.get(i)
                .and_then(ValueKey::from_value)
                .ok_or_else(|| RuntimeError::Unhashable {
                    role: crate::error::HashRole::DictKey,
                })
        };
        let out = match name {
            "insert" => {
                arity(2)?;
                d.insert(key(0)?, args[1].clone());
                Value::Nil
            }
            "remove" => {
                arity(1)?;
                d.remove(&key(0)?)
                    .map(|v| Value::Option(Some(Box::new(v))))
                    .unwrap_or(Value::Option(None))
            }
            "clear" => {
                arity(0)?;
                d.clear();
                Value::Nil
            }
            "update" => {
                arity(1)?;
                let Value::Dict(other) = &args[0] else {
                    return crate::error::type_err("`Dict.update` expects a dict argument");
                };
                for (k, v) in other.iter() {
                    d.insert(k.clone(), v.clone());
                }
                // `d.update(other)` returns the merged dict (spec §11.6 example `let dd = d.update(…)`).
                Value::Dict(Box::new(d.clone()))
            }
            "setdefault" => {
                // Python `dict.setdefault`: return the value for `key`, inserting `default` when absent.
                arity(2)?;
                let k = key(0)?;
                if let Some(v) = d.get(&k).cloned() {
                    v
                } else {
                    let v = args[1].clone();
                    d.insert(k, v.clone());
                    v
                }
            }
            "popitem" => {
                arity(0)?;
                let k = self.sorted_dict_keys(d).into_iter().next().ok_or_else(|| {
                    RuntimeError::EmptyCollection(("`Dict.popitem` on an empty dict").into())
                })?;
                let v = d.remove(&k).unwrap();
                Value::Tuple(vec![k.to_value(), v].into())
            }
            _ => return crate::error::type_err(format!("unknown `Dict` method `{name}`")),
        };
        Ok(out)
    }

    /// Mutating set methods (spec §11.6): `remove` reports `R0013` on an absent element, `discard` is silent.
    pub(crate) fn mutate_set(
        &mut self,
        env: &EnvRef,
        receiver: &Expr,
        name: &str,
        args: Vec<Value>,
    ) -> Result<Value, RuntimeError> {
        let var = match &receiver.kind {
            ExprKind::Path { segments } if segments.len() == 1 => segments[0].value.clone(),
            _ => return crate::error::type_err("cannot mutate a temporary value"),
        };
        let cur = env
            .borrow()
            .get_value(&var)
            .ok_or_else(|| RuntimeError::UnknownVariable { name: var.clone() })?;
        let Value::Set(mut s) = cur else {
            return crate::error::type_err("expected a set binding");
        };
        let out = self.mutate_set_handle(&mut s, name, args)?;
        self.write_back(env, &var, Value::Set(s));
        Ok(out)
    }

    /// Apply a mutating `Set` method to an already-resolved set (spec §11.6). Shared by the AST
    /// write-back path and the bytecode VM's in-place slot/binding mutation.
    pub(crate) fn mutate_set_handle(
        &self,
        s: &mut HashSet<ValueKey>,
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
        let key = |i: usize| -> Result<ValueKey, RuntimeError> {
            args.get(i)
                .and_then(ValueKey::from_value)
                .ok_or_else(|| RuntimeError::Unhashable {
                    role: crate::error::HashRole::SetElement,
                })
        };
        let out = match name {
            "add" => {
                arity(1)?;
                s.insert(key(0)?);
                Value::Nil
            }
            "remove" => {
                arity(1)?;
                if !s.remove(&key(0)?) {
                    return Err(RuntimeError::NotFound(("element not found").into()));
                }
                Value::Nil
            }
            "discard" => {
                arity(1)?;
                s.remove(&key(0)?);
                Value::Nil
            }
            "pop" => {
                arity(0)?;
                if let Some(k) = s.iter().next().cloned() {
                    s.remove(&k);
                    Value::Option(Some(Box::new(k.to_value())))
                } else {
                    Value::Option(None)
                }
            }
            "clear" => {
                arity(0)?;
                s.clear();
                Value::Nil
            }
            "update" => {
                arity(1)?;
                match &args[0] {
                    Value::Set(other) => {
                        for k in other.iter() {
                            s.insert(k.clone());
                        }
                    }
                    Value::Array(elems) => {
                        for e in elems.iter() {
                            let k = ValueKey::from_value(&e).ok_or_else(|| {
                                RuntimeError::Unhashable {
                                    role: crate::error::HashRole::SetElement,
                                }
                            })?;
                            s.insert(k);
                        }
                    }
                    other => {
                        return crate::error::type_err(format!(
                            "`Set.update` expects a set or array, got {}",
                            value_type_name(other)
                        ));
                    }
                }
                Value::Nil
            }
            _ => return crate::error::type_err(format!("unknown `Set` method `{name}`")),
        };
        Ok(out)
    }
}
