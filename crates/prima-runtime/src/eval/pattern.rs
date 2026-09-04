//! Pattern matching (spec §4.4/§16.3): `match`/`if let`/`while let` arm evaluation, pattern
//! decomposition, and structural equality for pattern operands.

use super::*;

impl Evaluator {
    /// `match`/`if let`/`while let` arm evaluation (spec §4.4/§16.3): first matching pattern (with optional guard) wins.
    pub(crate) fn eval_match(
        &mut self,
        env: &EnvRef,
        scrutinee: &Expr,
        arms: &[MatchArm],
    ) -> Result<Value, RuntimeError> {
        let sv = self.eval_expr(env, scrutinee)?;
        for arm in arms {
            if let Some(bindings) = self.match_pattern(env, &sv, &arm.pattern) {
                let scope = Env::child(env);
                for (name, val) in bindings {
                    scope.borrow_mut().set_value(&name, val);
                }
                if let Some(g) = &arm.guard {
                    match self.eval_expr(&scope, g)? {
                        Value::Bool(true) => return self.eval_expr(&scope, &arm.body),
                        Value::Bool(false) => continue,
                        _ => return crate::error::err("match guard must be a boolean"),
                    }
                }
                return self.eval_expr(&scope, &arm.body);
            }
        }
        crate::error::err("match is non-exhaustive")
    }

    /// Full pattern matching (spec §4.4): returns the bindings the pattern produces, or `None` on mismatch.
    pub(crate) fn match_pattern(
        &mut self,
        env: &EnvRef,
        v: &Value,
        p: &Pattern,
    ) -> Option<Vec<(String, Value)>> {
        match p {
            Pattern::Wildcard(_) => Some(vec![]),
            Pattern::Binding(name) => Some(vec![(name.value.clone(), v.clone())]),
            Pattern::Literal(lit) => {
                let lv = self.eval_literal(env, lit).ok()?;
                if self.pattern_values_equal(&lv, v) {
                    Some(vec![])
                } else {
                    None
                }
            }
            Pattern::Tuple(pats, rest) => {
                let Value::Tuple(items) = v else { return None };
                if !*rest && items.len() != pats.len() {
                    return None;
                }
                if *rest && items.len() < pats.len() {
                    return None;
                }
                let mut out = Vec::new();
                for (pat, val) in pats.iter().zip(items) {
                    out.extend(self.match_pattern(env, val, pat)?);
                }
                Some(out)
            }
            Pattern::Array(pats, rest) => {
                let Value::Array(elems) = v else { return None };
                if !*rest && elems.len() != pats.len() {
                    return None;
                }
                if *rest && elems.len() < pats.len() {
                    return None;
                }
                let mut out = Vec::new();
                for (pat, e) in pats.iter().zip(elems) {
                    out.extend(self.match_pattern(env, e, pat)?);
                }
                Some(out)
            }
            Pattern::Struct { name, fields, .. } => {
                let Value::Class(id) = v else { return None };
                let inst = self.instances.get(id)?.clone();
                if inst.class != name.value {
                    return None;
                }
                let mut out = Vec::new();
                for fp in fields {
                    let fv = inst.fields.get(&fp.name.value)?;
                    match &fp.pat {
                        None => out.push((fp.name.value.clone(), fv.clone())),
                        Some(p) => out.extend(self.match_pattern(env, fv, p)?),
                    }
                }
                Some(out)
            }
            Pattern::Variant { name, args, .. } => match name.value.as_str() {
                "Some" => match v {
                    Value::Option(Some(inner)) if args.len() == 1 => {
                        self.match_pattern(env, inner, &args[0])
                    }
                    _ => None,
                },
                "None" => {
                    if args.is_empty() && matches!(v, Value::Option(None)) {
                        Some(vec![])
                    } else {
                        None
                    }
                }
                "Ok" => match v {
                    Value::Result(Ok(inner)) if args.len() == 1 => {
                        self.match_pattern(env, inner, &args[0])
                    }
                    _ => None,
                },
                "Err" => match v {
                    Value::Result(Err(msg)) if args.len() == 1 => {
                        self.match_pattern(env, &Value::String(msg.clone()), &args[0])
                    }
                    _ => None,
                },
                _ => None,
            },
            Pattern::Range { lo, hi, inclusive } => {
                let Value::Number(vn) = v else { return None };
                let lo_n = self.eval_literal(env, lo).ok().and_then(|v| match v {
                    Value::Number(n) => Some(n),
                    _ => None,
                })?;
                let hi_n = self.eval_literal(env, hi).ok().and_then(|v| match v {
                    Value::Number(n) => Some(n),
                    _ => None,
                })?;
                let ge = self
                    .number_cmp(vn, &lo_n)
                    .is_some_and(|o| o != Ordering::Less);
                let hi_ok = if *inclusive {
                    self.number_cmp(vn, &hi_n)
                        .is_some_and(|o| o != Ordering::Greater)
                } else {
                    self.number_cmp(vn, &hi_n)
                        .is_some_and(|o| o == Ordering::Less)
                };
                if ge && hi_ok { Some(vec![]) } else { None }
            }
            Pattern::Or(pats) => {
                for p in pats {
                    if let Some(b) = self.match_pattern(env, v, p) {
                        return Some(b);
                    }
                }
                None
            }
            Pattern::Group(inner) => self.match_pattern(env, v, inner),
        }
    }

    /// Promote-and-compare two numbers (spec §6.4); `None` when they cannot be compared.
    pub(crate) fn number_cmp(&self, a: &Number, b: &Number) -> Option<Ordering> {
        let (x, y) = prima_core::number::promote(a, b);
        match (x, y) {
            (Number::Integer(x), Number::Integer(y)) => Some(x.cmp(&y)),
            (Number::Rational(x), Number::Rational(y)) => Some(x.cmp(&y)),
            (Number::Real(Real::F32(x)), Number::Real(Real::F32(y))) => x.partial_cmp(&y),
            (Number::Real(Real::F64(x)), Number::Real(Real::F64(y))) => x.partial_cmp(&y),
            _ => None,
        }
    }

    /// Equality used by literal patterns (spec §4.4): numbers compare through the promotion tower.
    pub(crate) fn pattern_values_equal(&self, a: &Value, b: &Value) -> bool {
        match (a, b) {
            (Value::Number(x), Value::Number(y)) => self.number_cmp(x, y) == Some(Ordering::Equal),
            (Value::String(x), Value::String(y)) => x == y,
            (Value::Char(x), Value::Char(y)) => x == y,
            (Value::Bool(x), Value::Bool(y)) => x == y,
            _ => a == b,
        }
    }
}
