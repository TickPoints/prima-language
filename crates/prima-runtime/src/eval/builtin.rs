//! Builtin intrinsic calls (spec §18.1): the pre-imported `core` builtin bodies, the `input`/
//! `read_line` IO functions, and symbol/`Expr` conversions.

use super::helpers::{check_arity, value_type_name};
use super::*;

impl Evaluator {
    pub(crate) fn call_builtin(
        &mut self,
        b: Builtin,
        args: Vec<Value>,
    ) -> Result<Value, RuntimeError> {
        match b {
            // `print` outputs without a trailing newline; `println` appends one (v2.1, spec §18.1b).
            Builtin::Print | Builtin::Println => {
                let mut s = String::new();
                for (i, v) in args.iter().enumerate() {
                    if i > 0 {
                        s.push(' ');
                    }
                    s.push_str(&self.format_value(v));
                }
                if matches!(b, Builtin::Println) {
                    s.push('\n');
                }
                (self.output)(s);
                Ok(Value::Nil)
            }
            Builtin::Input | Builtin::ReadLine => self.call_input(b, args),
            // Symbolic differentiation is intercepted in `eval_call` (spec §19.4); reaching here means
            // it was used indirectly (e.g. as a value), which the interpreter does not support.
            Builtin::Derivative | Builtin::Partial | Builtin::Grad | Builtin::Limit => {
                crate::error::err(
                    "`derivative`/`partial`/`grad`/`limit` must be called directly with a variable",
                )
            }
            Builtin::Simplify => {
                let arg = args
                    .first()
                    .ok_or_else(|| RuntimeError::Message("simplify expects one argument".into()))?;
                let id = self.to_expr_id(arg)?;
                let simp = simplify(self.pool, self.builtins, id);
                Ok(self.value_from_expr(simp))
            }
            Builtin::Range => {
                if args.len() < 2 || args.len() > 3 {
                    return crate::error::err("`range` expects (start, end, step?)");
                }
                let start = self.scalar_value(args[0].clone())?;
                let end = self.scalar_value(args[1].clone())?;
                let step = match args.get(2) {
                    Some(s) => self.scalar_value(s.clone())?,
                    None => Number::from(1),
                };
                let start_i = start.as_i64().ok_or_else(|| {
                    RuntimeError::Type(format!("range bounds must be integers, got {start}"))
                })?;
                let end_i = end.as_i64().ok_or_else(|| {
                    RuntimeError::Type(format!("range bounds must be integers, got {end}"))
                })?;
                let step_i = step.as_i64().ok_or_else(|| {
                    RuntimeError::Type(format!("range step must be an integer, got {step}"))
                })?;
                if step_i == 0 {
                    return crate::error::err("`range` step cannot be zero");
                }
                let mut out = Vec::new();
                let mut i = start_i;
                while if step_i > 0 { i < end_i } else { i > end_i } {
                    out.push(Value::Number(Number::from(i)));
                    i += step_i;
                }
                Ok(Value::Array(out))
            }
            // ---- collection convenience functions (spec appendix B.1) ----
            Builtin::Len => {
                check_arity("len", &args, 1)?;
                let n = match &args[0] {
                    Value::Array(a) => a.len(),
                    Value::Dict(d) => d.len(),
                    Value::Set(s) => s.len(),
                    Value::String(s) => s.chars().count(),
                    Value::Tuple(t) => t.len(),
                    other => {
                        return crate::error::err(format!(
                            "`len` expects an array, dict, set, string, or tuple, got {}",
                            value_type_name(other)
                        ));
                    }
                };
                Ok(Value::Number(Number::from(n as i64)))
            }
            Builtin::Enumerate => {
                check_arity("enumerate", &args, 1)?;
                let Value::Array(a) = &args[0] else {
                    return crate::error::err("`enumerate` expects an array");
                };
                Ok(Value::Array(
                    a.iter()
                        .enumerate()
                        .map(|(i, e)| {
                            Value::Tuple(vec![Value::Number(Number::from(i as i64)), e.clone()])
                        })
                        .collect(),
                ))
            }
            Builtin::Zip => {
                check_arity("zip", &args, 2)?;
                let (Value::Array(x), Value::Array(y)) = (&args[0], &args[1]) else {
                    return crate::error::err("`zip` expects two arrays");
                };
                Ok(Value::Array(
                    x.iter()
                        .zip(y)
                        .map(|(a, b)| Value::Tuple(vec![a.clone(), b.clone()]))
                        .collect(),
                ))
            }
            Builtin::Sorted => {
                check_arity("sorted", &args, 1)?;
                let Value::Array(a) = &args[0] else {
                    return crate::error::err("`sorted` expects an array");
                };
                let mut nums = Vec::with_capacity(a.len());
                for e in a {
                    match e {
                        Value::Number(n) => nums.push(n.clone()),
                        _ => return crate::error::err("`sorted` requires an array of numbers"),
                    }
                }
                nums.sort_by(|x, y| self.number_cmp(x, y).unwrap_or(Ordering::Equal));
                Ok(Value::Array(nums.into_iter().map(Value::Number).collect()))
            }
            Builtin::Reversed => {
                check_arity("reversed", &args, 1)?;
                let Value::Array(mut a) = args[0].clone() else {
                    return crate::error::err("`reversed` expects an array");
                };
                a.reverse();
                Ok(Value::Array(a))
            }
            Builtin::Sum | Builtin::Prod => {
                let name = if matches!(b, Builtin::Sum) {
                    "sum"
                } else {
                    "prod"
                };
                check_arity(name, &args, 1)?;
                let Value::Array(a) = &args[0] else {
                    return crate::error::err(format!("`{name}` expects an array"));
                };
                if a.is_empty() {
                    return crate::error::err("empty collection");
                }
                let op = if matches!(b, Builtin::Sum) {
                    BinOp::Add
                } else {
                    BinOp::Mul
                };
                let mut acc = match &a[0] {
                    Value::Number(n) => n.clone(),
                    _ => {
                        return crate::error::err(format!("`{name}` requires an array of numbers"));
                    }
                };
                for e in &a[1..] {
                    let n = match e {
                        Value::Number(n) => n.clone(),
                        _ => {
                            return crate::error::err(format!(
                                "`{name}` requires an array of numbers"
                            ));
                        }
                    };
                    match self.eval_number_binary(op, acc, n)? {
                        Value::Number(n) => acc = n,
                        _ => return crate::error::err(format!("`{name}` result must be numeric")),
                    }
                }
                Ok(Value::Number(acc))
            }
            Builtin::Min | Builtin::Max => {
                let name = if matches!(b, Builtin::Min) {
                    "min"
                } else {
                    "max"
                };
                check_arity(name, &args, 1)?;
                let Value::Array(a) = &args[0] else {
                    return crate::error::err(format!("`{name}` expects an array"));
                };
                if a.is_empty() {
                    return crate::error::err("empty collection");
                }
                let mut best = match &a[0] {
                    Value::Number(n) => n.clone(),
                    _ => {
                        return crate::error::err(format!("`{name}` requires an array of numbers"));
                    }
                };
                for e in &a[1..] {
                    let n = match e {
                        Value::Number(n) => n.clone(),
                        _ => {
                            return crate::error::err(format!(
                                "`{name}` requires an array of numbers"
                            ));
                        }
                    };
                    let ord = self.number_cmp(&n, &best).ok_or_else(|| {
                        RuntimeError::Message("cannot compare these numbers".into())
                    })?;
                    let better = if matches!(b, Builtin::Min) {
                        ord == Ordering::Less
                    } else {
                        ord == Ordering::Greater
                    };
                    if better {
                        best = n;
                    }
                }
                Ok(Value::Number(best))
            }
            Builtin::All | Builtin::Any => {
                let name = if matches!(b, Builtin::All) {
                    "all"
                } else {
                    "any"
                };
                check_arity(name, &args, 1)?;
                let Value::Array(a) = &args[0] else {
                    return crate::error::err(format!("`{name}` expects an array"));
                };
                let is_all = matches!(b, Builtin::All);
                let mut result = is_all;
                for e in a {
                    let ok = match e {
                        Value::Bool(x) => *x,
                        _ => {
                            return crate::error::err(format!(
                                "`{name}` requires an array of booleans"
                            ));
                        }
                    };
                    if is_all {
                        result = result && ok;
                        if !result {
                            break;
                        }
                    } else {
                        result = result || ok;
                        if result {
                            break;
                        }
                    }
                }
                Ok(Value::Bool(result))
            }
            Builtin::Join => {
                check_arity("join", &args, 2)?;
                let Value::Array(parts) = &args[0] else {
                    return crate::error::err("`join` expects an array of strings");
                };
                let Value::String(sep) = &args[1] else {
                    return crate::error::err("`join` separator must be a string");
                };
                let mut out = String::new();
                for (i, p) in parts.iter().enumerate() {
                    if i > 0 {
                        out.push_str(sep);
                    }
                    match p {
                        Value::String(s) => out.push_str(s),
                        _ => return crate::error::err("`join` requires an array of strings"),
                    }
                }
                Ok(Value::String(out))
            }
            Builtin::Count => {
                check_arity("count", &args, 2)?;
                let Value::Array(a) = &args[0] else {
                    return crate::error::err("`count` expects an array");
                };
                Ok(Value::Number(Number::from(
                    a.iter().filter(|e| self.value_eq(e, &args[1])).count() as i64,
                )))
            }
            Builtin::Index => {
                check_arity("index", &args, 2)?;
                let Value::Array(a) = &args[0] else {
                    return crate::error::err("`index` expects an array");
                };
                match a.iter().position(|e| self.value_eq(e, &args[1])) {
                    Some(i) => Ok(Value::Number(Number::from(i as i64))),
                    None => crate::error::err("element not found"),
                }
            }
            Builtin::First | Builtin::Last => {
                let name = if matches!(b, Builtin::First) {
                    "first"
                } else {
                    "last"
                };
                check_arity(name, &args, 1)?;
                let Value::Array(a) = &args[0] else {
                    return crate::error::err(format!("`{name}` expects an array"));
                };
                let elem = if matches!(b, Builtin::First) {
                    a.first()
                } else {
                    a.last()
                };
                Ok(elem
                    .map(|v| Value::Option(Some(Box::new(v.clone()))))
                    .unwrap_or(Value::Option(None)))
            }
            Builtin::Linspace => {
                check_arity("linspace", &args, 3)?;
                let start = self.scalar_value(args[0].clone())?;
                let end = self.scalar_value(args[1].clone())?;
                let n = match &args[2] {
                    Value::Number(n) => n.as_i64().ok_or_else(|| {
                        RuntimeError::Type("`linspace` count must be an integer".into())
                    })?,
                    _ => return crate::error::err("`linspace` count must be an integer"),
                };
                if n < 0 {
                    return crate::error::err("`linspace` count must be non-negative");
                }
                let n = n as usize;
                if n == 0 {
                    return Ok(Value::Array(vec![]));
                }
                let (start_f, end_f) = (start.to_f64_lossy(), end.to_f64_lossy());
                if n == 1 {
                    return Ok(Value::Array(vec![Value::Number(Number::Real(Real::F64(
                        start_f,
                    )))]));
                }
                let step = (end_f - start_f) / (n - 1) as f64;
                let mut out = Vec::with_capacity(n);
                for i in 0..n {
                    out.push(Value::Number(Number::Real(Real::F64(
                        start_f + step * i as f64,
                    ))));
                }
                Ok(Value::Array(out))
            }
            // `map`/`filter`/`reduce` are intercepted in `eval_call` (the function argument is an
            // un-evaluated expression); reaching here means the name was used as a value.
            Builtin::Map | Builtin::Filter | Builtin::Reduce => {
                crate::error::err("`map`/`filter`/`reduce` must be called directly with a function")
            }
            // `jit` is intercepted in `eval_call` (the argument is an un-evaluated function/expression);
            // reaching here means the name was used as a value.
            Builtin::Jit => {
                crate::error::err("`jit` must be called directly with a function or expression")
            }
            Builtin::Collapse(name) => crate::collapse::call(name, &args, self.pool, self.builtins),
            // Math operators: build an `Apply` node, then simplify the whole thing (spec §8.3 level 2 constant folding).
            _ => {
                let f_id = self.pool.symbol(match b {
                    Builtin::Sqrt => self.builtins.sqrt,
                    Builtin::Exp => self.builtins.exp,
                    Builtin::Log => self.builtins.log,
                    Builtin::Ln => self.builtins.ln,
                    Builtin::Sin => self.builtins.sin,
                    Builtin::Cos => self.builtins.cos,
                    Builtin::Tan => self.builtins.tan,
                    Builtin::Abs => self.builtins.abs,
                    _ => unreachable!(),
                });
                if args.len() != 1 {
                    return crate::error::err("math function expects one argument");
                }
                let arg_id = self.to_expr_id(&args[0])?;
                let app = self.pool.apply(f_id, &[arg_id]);
                let simp = self.simplify_current(app);
                Ok(self.value_from_expr(simp))
            }
        }
    }

    pub(crate) fn to_expr_id(&self, v: &Value) -> Result<ExprId, RuntimeError> {
        match v {
            Value::Number(n) => Ok(self.pool.number(n)),
            Value::Expr(id) => Ok(*id),
            _ => crate::error::err("expected a numeric or symbolic expression"),
        }
    }

    // A pure constant node folds back to `Value::Number`; otherwise the symbolic form is preserved (spec §6.6 exact by default).
    pub(crate) fn value_from_expr(&self, id: ExprId) -> Value {
        if let Some(n) = self.pool.const_number(id) {
            Value::Number(n)
        } else {
            Value::Expr(id)
        }
    }

    /// `input(prompt?)` / `read_line()` (spec §18.1b): optional prompt written without a trailing newline,
    /// then one line read from stdin (trailing `\r\n`/`\n` stripped). EOF or I/O errors return "".
    pub(crate) fn call_input(
        &mut self,
        b: Builtin,
        args: Vec<Value>,
    ) -> Result<Value, RuntimeError> {
        if args.len() > 1 {
            return crate::error::err(if matches!(b, Builtin::ReadLine) {
                "`read_line` takes no arguments"
            } else {
                "`input` takes at most one (prompt) argument"
            });
        }
        if let Some(prompt) = args.first() {
            let s = self.format_value(prompt);
            (self.output)(s);
        }
        use std::io::BufRead;
        let mut line = String::new();
        match std::io::stdin().lock().read_line(&mut line) {
            Ok(_) => {
                while line.ends_with('\n') || line.ends_with('\r') {
                    line.pop();
                }
                Ok(Value::String(line))
            }
            Err(_) => Ok(Value::String(String::new())),
        }
    }
}
