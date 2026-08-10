use std::collections::HashMap;

use num_bigint::BigInt;
use num_traits::ToPrimitive;
use prima_core::render::{render_latex, render_number};
use prima_core::simplify::simplify;
use prima_core::{BuiltinSymbols, ExprId, ExprPool, Number, SymbolTable, Value};
use prima_syntax::ast::{AssignOp, BinOp, Expr, ExprKind, IndexItem, Literal, Program, Stmt, UnOp};
use prima_syntax::error::SyntaxError;

use crate::builtins::Builtin;
use crate::error::RuntimeError;

#[derive(Clone)]
pub enum Function {
    Builtin(Builtin),
    User { params: Vec<prima_syntax::ast::Param>, body: Expr, env: Env },
}

impl Function {
    pub fn is_pure(&self) -> bool {
        match self {
            Function::Builtin(b) => b.is_pure(),
            Function::User { .. } => true,
        }
    }
}

#[derive(Clone, Default)]
pub struct Env {
    values: HashMap<String, Value>,
    funcs: HashMap<String, Function>,
    parent: Option<Box<Env>>,
}

impl Env {
    pub fn new() -> Env {
        let mut env = Env::default();
        for name in [
            "print",
            "println",
            "simplify",
            "sqrt",
            "exp",
            "log",
            "ln",
            "sin",
            "cos",
            "tan",
            "abs",
        ] {
            if let Some(b) = Builtin::from_name(name) {
                env.set_func(name, Function::Builtin(b));
            }
        }
        env
    }

    fn child(parent: Env) -> Env {
        Env { values: HashMap::new(), funcs: HashMap::new(), parent: Some(Box::new(parent)) }
    }

    pub fn get_value(&self, name: &str) -> Option<Value> {
        if let Some(v) = self.values.get(name) {
            return Some(v.clone());
        }
        self.parent.as_ref().and_then(|p| p.get_value(name))
    }

    fn set_value(&mut self, name: &str, v: Value) {
        self.values.insert(name.to_string(), v);
    }

    fn get_func(&self, name: &str) -> Option<Function> {
        if let Some(f) = self.funcs.get(name) {
            return Some(f.clone());
        }
        self.parent.as_ref().and_then(|p| p.get_func(name))
    }

    fn set_func(&mut self, name: &str, f: Function) {
        self.funcs.insert(name.to_string(), f);
    }
}

pub struct Evaluator {
    pool: &'static ExprPool,
    symbols: &'static SymbolTable,
    builtins: &'static BuiltinSymbols,
    output: Box<dyn FnMut(String)>,
}

impl Default for Evaluator {
    fn default() -> Self {
        Self::new()
    }
}

impl Evaluator {
    pub fn new() -> Evaluator {
        Evaluator {
            pool: ExprPool::global(),
            symbols: SymbolTable::global(),
            builtins: BuiltinSymbols::global(),
            output: Box::new(|s| print!("{s}")),
        }
    }

    pub fn with_sink(output: impl FnMut(String) + 'static) -> Evaluator {
        Evaluator {
            pool: ExprPool::global(),
            symbols: SymbolTable::global(),
            builtins: BuiltinSymbols::global(),
            output: Box::new(output),
        }
    }

    pub fn pool(&self) -> &'static ExprPool {
        self.pool
    }

    pub fn symbols(&self) -> &'static SymbolTable {
        self.symbols
    }

    pub fn builtins(&self) -> &'static BuiltinSymbols {
        self.builtins
    }

    pub fn eval_program(&mut self, program: &Program) -> Result<(), RuntimeError> {
        let mut env = Env::new();
        for stmt in &program.stmts {
            self.eval_stmt(&mut env, stmt)?;
        }
        Ok(())
    }

    pub fn eval_src(&mut self, src: &str) -> Result<(), RuntimeError> {
        let program = prima_syntax::parse(src).map_err(syntax_errors)?;
        self.eval_program(&program)
    }

    pub fn eval_value(&mut self, src: &str) -> Result<Value, RuntimeError> {
        let program = prima_syntax::parse(src).map_err(syntax_errors)?;
        let mut env = Env::new();
        let mut last = Value::Nil;
        for stmt in &program.stmts {
            if let Stmt::Expr(e) = stmt {
                last = self.eval_expr(&mut env, e)?;
            } else {
                self.eval_stmt(&mut env, stmt)?;
            }
        }
        Ok(last)
    }

    pub fn format_value(&self, v: &Value) -> String {
        match v {
            Value::Nil => "nil".into(),
            Value::Number(n) => render_number(n),
            Value::Bool(b) => b.to_string(),
            Value::Char(c) => c.to_string(),
            Value::String(s) => s.clone(),
            Value::Array(elems) => {
                let inner: Vec<String> = elems.iter().map(render_number).collect();
                format!("[{}]", inner.join(", "))
            }
            Value::Expr(id) => render_latex(self.pool, self.symbols, *id),
            Value::Symbol(_) => "symbol".into(),
            Value::Indeterminate(_) => "indeterminate".into(),
            Value::Undefined => "undefined".into(),
            Value::Error(_) => "error".into(),
            Value::Tuple(_) => "tuple".into(),
            Value::Result(_) => "result".into(),
        }
    }

    fn eval_stmt(&mut self, env: &mut Env, stmt: &Stmt) -> Result<(), RuntimeError> {
        match stmt {
            Stmt::Let { name, value, .. } => {
                if let ExprKind::Lambda { params, body } = &value.kind {
                    env.set_func(&name.value, Function::User { params: params.clone(), body: (**body).clone(), env: env.clone() });
                } else {
                    let v = self.eval_expr(env, value)?;
                    env.set_value(&name.value, v);
                }
                Ok(())
            }
            Stmt::Const { name, value, .. } => {
                let v = self.eval_expr(env, value)?;
                env.set_value(&name.value, v);
                Ok(())
            }
            Stmt::MathDef { name, params, body, .. } => {
                env.set_func(&name.value, Function::User { params: params.clone(), body: body.clone(), env: env.clone() });
                Ok(())
            }
            Stmt::Expr(e) => {
                self.eval_expr(env, e)?;
                Ok(())
            }
            Stmt::Assign { target, op, value, .. } => {
                let v = self.eval_expr(env, value)?;
                let name = match &target.kind {
                    ExprKind::Path { segments } if segments.len() == 1 => &segments[0].value,
                    _ => return crate::error::err("assignment target must be a variable"),
                };
                let prev = env.get_value(name);
                let merged = match op {
                    AssignOp::Assign => v,
                    AssignOp::AddAssign => self.eval_binary(BinOp::Add, prev.unwrap_or(Value::Number(Number::from(0))), v)?,
                    AssignOp::SubAssign => self.eval_binary(BinOp::Sub, prev.unwrap_or(Value::Number(Number::from(0))), v)?,
                };
                env.set_value(name, merged);
                Ok(())
            }
            Stmt::Pub(inner) => self.eval_stmt(env, inner),
            other => crate::error::err(format!("statement not supported yet: {}", stmt_kind_name(other))),
        }
    }

    fn eval_expr(&mut self, env: &mut Env, expr: &Expr) -> Result<Value, RuntimeError> {
        match &expr.kind {
            ExprKind::Literal(lit) => self.eval_literal(env, lit),
            ExprKind::Symbol(s) => Ok(Value::Expr(self.pool.symbol(self.symbols.intern(&s.value)))),
            ExprKind::Path { segments } => {
                if segments.len() == 1 {
                    let name = &segments[0].value;
                    if let Some(v) = env.get_value(name) {
                        Ok(v)
                    } else if env.get_func(name).is_some() {
                        crate::error::err(format!("function `{name}` cannot be used as a value"))
                    } else {
                        Ok(Value::Expr(self.pool.symbol(self.symbols.intern(name))))
                    }
                } else {
                    crate::error::err("module-qualified access is not supported yet")
                }
            }
            ExprKind::Binary { op, lhs, rhs } => {
                let a = self.eval_expr(env, lhs)?;
                let b = self.eval_expr(env, rhs)?;
                self.eval_binary(*op, a, b)
            }
            ExprKind::Unary { op, operand } => {
                let v = self.eval_expr(env, operand)?;
                self.eval_unary(*op, v)
            }
            ExprKind::Call { callee, args } => self.eval_call(env, callee, args),
            ExprKind::Index { base, index } => self.eval_index(env, base, index),
            ExprKind::Array(items) => {
                let mut elems = Vec::new();
                for item in items {
                    match self.eval_expr(env, item)? {
                        Value::Number(n) => elems.push(n),
                        Value::Array(_) => return crate::error::err("nested arrays are not allowed"),
                        _ => return crate::error::err("array elements must be numbers"),
                    }
                }
                Ok(Value::Array(elems))
            }
            ExprKind::Tuple(_) => crate::error::err("tuples are not supported yet"),
            ExprKind::Lambda { .. } => crate::error::err("lambda must be assigned to a variable to be callable"),
            ExprKind::Match { .. } => crate::error::err("`match` is not supported yet"),
            ExprKind::Pipeline { lhs, rhs } => self.eval_pipeline(env, lhs, rhs),
            ExprKind::Custom(_) => crate::error::err("`custom` config block is not valid here"),
        }
    }

    fn eval_literal(&mut self, env: &mut Env, lit: &Literal) -> Result<Value, RuntimeError> {
        match lit {
            Literal::Integer(s) => {
                let i = s.parse::<BigInt>().map_err(|_| RuntimeError::Message("invalid integer literal".into()))?;
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
                let f = s.parse::<f64>().map_err(|_| RuntimeError::Message("invalid float literal".into()))?;
                Ok(Value::Number(Number::from(f)))
            }
            Literal::Str(s) => Ok(Value::String(s.clone())),
            Literal::Char(c) => Ok(Value::Char(*c)),
            Literal::Bool(b) => Ok(Value::Bool(*b)),
            Literal::Tex(s) => {
                let tex_ast = prima_syntax::tex::parse_tex(s).map_err(syntax_err)?;
                self.eval_expr(env, &tex_ast)
            }
        }
    }

    fn eval_binary(&mut self, op: BinOp, a: Value, b: Value) -> Result<Value, RuntimeError> {
        if matches!(a, Value::Array(_)) || matches!(b, Value::Array(_)) {
            return self.eval_binary_array(op, a, b);
        }
        match op {
            BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Pow => match (a, b) {
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
                        _ => unreachable!(),
                    };
                    let simp = simplify(self.pool, self.builtins, node);
                    Ok(self.value_from_expr(simp))
                }
            },
            BinOp::And | BinOp::Or => match (a, b) {
                (Value::Bool(x), Value::Bool(y)) => Ok(Value::Bool(if op == BinOp::And { x && y } else { x || y })),
                _ => crate::error::err("`&&`/`||` require boolean operands"),
            },
            BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => self.eval_compare(op, a, b),
            _ => crate::error::err("operator not supported"),
        }
    }

    fn eval_number_binary(&mut self, op: BinOp, x: Number, y: Number) -> Result<Value, RuntimeError> {
        match op {
            BinOp::Add => Ok(Value::Number(x + y)),
            BinOp::Sub => Ok(Value::Number(x - y)),
            BinOp::Mul => Ok(Value::Number(x * y)),
            BinOp::Div => {
                if y.is_zero() && !matches!(y, Number::Real(_)) {
                    return crate::error::err("division by zero");
                }
                Ok(Value::Number(x / y))
            }
            BinOp::Pow => match x.pow(&y) {
                Some(r) => Ok(Value::Number(r)),
                None => {
                    let a = self.pool.number(&x);
                    let b = self.pool.number(&y);
                    let node = self.pool.pow2(a, b);
                    let simp = simplify(self.pool, self.builtins, node);
                    Ok(self.value_from_expr(simp))
                }
            },
            _ => crate::error::err("arithmetic operator required"),
        }
    }

    fn eval_compare(&mut self, op: BinOp, a: Value, b: Value) -> Result<Value, RuntimeError> {
        use std::cmp::Ordering;
        let ord = match (a, b) {
            (Value::Number(x), Value::Number(y)) => {
                let (x, y) = prima_core::number::promote(&x, &y);
                match (x, y) {
                    (Number::Integer(x), Number::Integer(y)) => Some(x.cmp(&y)),
                    (Number::Rational(x), Number::Rational(y)) => Some(x.cmp(&y)),
                    (Number::Real(prima_core::number::Real::F32(x)), Number::Real(prima_core::number::Real::F32(y))) => {
                        x.partial_cmp(&y)
                    }
                    (Number::Real(prima_core::number::Real::F64(x)), Number::Real(prima_core::number::Real::F64(y))) => {
                        x.partial_cmp(&y)
                    }
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
                }))
            }
            (Value::Bool(x), Value::Bool(y)) => {
                return Ok(Value::Bool(match op {
                    BinOp::Eq => x == y,
                    BinOp::Ne => x != y,
                    _ => false,
                }))
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

    fn eval_unary(&mut self, op: UnOp, v: Value) -> Result<Value, RuntimeError> {
        match op {
            UnOp::Neg => match v {
                Value::Number(n) => Ok(Value::Number(-n)),
                Value::Array(elems) => Ok(Value::Array(elems.into_iter().map(|n| -n).collect())),
                Value::Expr(id) => {
                    let node = self.pool.mul2(self.pool.integer(-1), id);
                    let simp = simplify(self.pool, self.builtins, node);
                    Ok(self.value_from_expr(simp))
                }
                _ => crate::error::err("cannot negate this value"),
            },
            UnOp::Not => match v {
                Value::Bool(b) => Ok(Value::Bool(!b)),
                _ => crate::error::err("`!` requires a boolean"),
            },
            UnOp::Pos => Ok(v),
        }
    }

    fn eval_call(&mut self, env: &mut Env, callee: &Expr, args: &[Expr]) -> Result<Value, RuntimeError> {
        let mut arg_values = Vec::with_capacity(args.len());
        for a in args {
            arg_values.push(self.eval_expr(env, a)?);
        }
        let func = match &callee.kind {
            ExprKind::Path { segments } if segments.len() == 1 => env
                .get_func(&segments[0].value)
                .ok_or_else(|| RuntimeError::Message(format!("unknown function `{}`", segments[0].value)))?,
            _ => return crate::error::err("invalid function call"),
        };
        self.apply_function(&func, arg_values)
    }

    fn eval_index(&mut self, env: &mut Env, base: &Expr, index: &prima_syntax::ast::Index) -> Result<Value, RuntimeError> {
        let arr = self.eval_expr(env, base)?;
        let arr = match arr {
            Value::Array(a) => a,
            _ => return crate::error::err("indexing requires an array"),
        };
        if index.items.len() != 1 {
            return crate::error::err("multi-dimensional indexing is not supported yet");
        }
        match &index.items[0] {
            IndexItem::Elem(e) => {
                let idx = self.eval_expr(env, e)?;
                let idx = match idx {
                    Value::Number(Number::Integer(i)) => i.to_usize().ok_or_else(|| RuntimeError::Message("array index out of range".into()))?,
                    _ => return crate::error::err("array index must be an integer"),
                };
                arr.get(idx)
                    .cloned()
                    .map(Value::Number)
                    .ok_or_else(|| RuntimeError::Message(format!("index out of bounds: {idx}")))
            }
            IndexItem::Slice { .. } => crate::error::err("array slicing is not supported yet"),
        }
    }

    fn eval_pipeline(&mut self, env: &mut Env, lhs: &Expr, rhs: &Expr) -> Result<Value, RuntimeError> {
        let v = self.eval_expr(env, lhs)?;
        match &rhs.kind {
            ExprKind::Path { segments } if segments.len() == 1 => {
                let func = env
                    .get_func(&segments[0].value)
                    .ok_or_else(|| RuntimeError::Message(format!("unknown function `{}`", segments[0].value)))?;
                self.apply_function(&func, vec![v])
            }
            ExprKind::Call { callee, args } => {
                let func = match &callee.kind {
                    ExprKind::Path { segments } if segments.len() == 1 => env
                        .get_func(&segments[0].value)
                        .ok_or_else(|| RuntimeError::Message(format!("unknown function `{}`", segments[0].value)))?,
                    _ => return crate::error::err("invalid pipeline target"),
                };
                let mut cargs = vec![v];
                for a in args {
                    cargs.push(self.eval_expr(env, a)?);
                }
                self.apply_function(&func, cargs)
            }
            ExprKind::Lambda { params, body } => {
                let func = Function::User { params: params.clone(), body: (**body).clone(), env: env.clone() };
                self.apply_function(&func, vec![v])
            }
            _ => crate::error::err("pipeline right-hand side must be a function"),
        }
    }

    fn apply_function(&mut self, func: &Function, args: Vec<Value>) -> Result<Value, RuntimeError> {
        let array_positions: Vec<usize> = args
            .iter()
            .enumerate()
            .filter(|(_, v)| matches!(v, Value::Array(_)))
            .map(|(i, _)| i)
            .collect();
        if !array_positions.is_empty() && func.is_pure() {
            return self.broadcast_call(func, args, &array_positions);
        }
        match func {
            Function::Builtin(b) => self.call_builtin(*b, args),
            Function::User { params, body, env } => {
                if args.len() != params.len() {
                    return crate::error::err(format!("expected {} arguments, got {}", params.len(), args.len()));
                }
                let mut call_env = Env::child(env.clone());
                for (p, a) in params.iter().zip(args) {
                    call_env.set_value(&p.name.value, a);
                }
                self.eval_expr(&mut call_env, body)
            }
        }
    }

    fn broadcast_call(&mut self, func: &Function, args: Vec<Value>, positions: &[usize]) -> Result<Value, RuntimeError> {
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
        let mut results = Vec::with_capacity(len);
        for i in 0..len {
            let mut cargs = Vec::with_capacity(args.len());
            for (j, v) in args.iter().enumerate() {
                if positions.contains(&j) {
                    if let Value::Array(a) = v {
                        cargs.push(Value::Number(a[i].clone()));
                    }
                } else {
                    match v {
                        Value::Number(_) => cargs.push(v.clone()),
                        _ => return crate::error::err("cannot broadcast a non-numeric scalar"),
                    }
                }
            }
            match self.apply_function(func, cargs)? {
                Value::Number(n) => results.push(n),
                _ => return crate::error::err("broadcast result must be numeric"),
            }
        }
        Ok(Value::Array(results))
    }

    fn eval_binary_array(&mut self, op: BinOp, a: Value, b: Value) -> Result<Value, RuntimeError> {
        match (a, b) {
            (Value::Array(av), Value::Array(bv)) => {
                if av.len() != bv.len() {
                    return crate::error::err("dimension mismatch in array operation");
                }
                if av.is_empty() {
                    return crate::error::err("cannot operate on an empty array");
                }
                let mut out = Vec::with_capacity(av.len());
                for (x, y) in av.into_iter().zip(bv) {
                    match self.eval_number_binary(op, x, y)? {
                        Value::Number(n) => out.push(n),
                        _ => return crate::error::err("array operation result must be numeric"),
                    }
                }
                Ok(Value::Array(out))
            }
            (Value::Array(av), other) => {
                let scalar = self.scalar_for_broadcast(other)?;
                if av.is_empty() {
                    return crate::error::err("cannot operate on an empty array");
                }
                let mut out = Vec::with_capacity(av.len());
                for x in av {
                    match self.eval_number_binary(op, x, scalar.clone())? {
                        Value::Number(n) => out.push(n),
                        _ => return crate::error::err("array operation result must be numeric"),
                    }
                }
                Ok(Value::Array(out))
            }
            (other, Value::Array(bv)) => {
                let scalar = self.scalar_for_broadcast(other)?;
                if bv.is_empty() {
                    return crate::error::err("cannot operate on an empty array");
                }
                let mut out = Vec::with_capacity(bv.len());
                for y in bv {
                    match self.eval_number_binary(op, scalar.clone(), y)? {
                        Value::Number(n) => out.push(n),
                        _ => return crate::error::err("array operation result must be numeric"),
                    }
                }
                Ok(Value::Array(out))
            }
            _ => crate::error::err("invalid array operation"),
        }
    }

    fn scalar_for_broadcast(&self, v: Value) -> Result<Number, RuntimeError> {
        match v {
            Value::Number(n) => Ok(n),
            _ => crate::error::err("cannot broadcast with a non-numeric scalar"),
        }
    }

    fn call_builtin(&mut self, b: Builtin, args: Vec<Value>) -> Result<Value, RuntimeError> {
        match b {
            Builtin::Print | Builtin::Println => {
                let mut s = String::new();
                for (i, v) in args.iter().enumerate() {
                    if i > 0 {
                        s.push(' ');
                    }
                    s.push_str(&self.format_value(v));
                }
                s.push('\n');
                (self.output)(s);
                Ok(Value::Nil)
            }
            Builtin::Simplify => {
                let arg = args.first().ok_or_else(|| RuntimeError::Message("simplify expects one argument".into()))?;
                let id = self.to_expr_id(arg)?;
                let simp = simplify(self.pool, self.builtins, id);
                Ok(self.value_from_expr(simp))
            }
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
                let simp = simplify(self.pool, self.builtins, app);
                Ok(self.value_from_expr(simp))
            }
        }
    }

    fn to_expr_id(&self, v: &Value) -> Result<ExprId, RuntimeError> {
        match v {
            Value::Number(n) => Ok(self.pool.number(n)),
            Value::Expr(id) => Ok(*id),
            _ => crate::error::err("expected a numeric or symbolic expression"),
        }
    }

    fn value_from_expr(&self, id: ExprId) -> Value {
        if let Some(n) = self.pool.const_number(id) {
            Value::Number(n)
        } else {
            Value::Expr(id)
        }
    }
}

fn syntax_err(e: SyntaxError) -> RuntimeError {
    RuntimeError::Message(format!("syntax error: {}", e.message))
}

fn syntax_errors(errors: Vec<SyntaxError>) -> RuntimeError {
    match errors.first() {
        Some(e) => syntax_err(e.clone()),
        None => RuntimeError::Message("syntax error".into()),
    }
}

fn stmt_kind_name(stmt: &Stmt) -> &'static str {
    match stmt {
        Stmt::For { .. } => "`for`",
        Stmt::ParFor { .. } => "`parfor`",
        Stmt::While { .. } => "`while`",
        Stmt::If { .. } => "`if`",
        Stmt::Return { .. } => "`return`",
        Stmt::Try { .. } => "`try`",
        Stmt::WithConfig { .. } => "`with config`",
        Stmt::FnDef { .. } => "`fn`",
        _ => "statement",
    }
}
