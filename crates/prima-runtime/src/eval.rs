use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::rc::Rc;

use num_bigint::BigInt;
use num_traits::ToPrimitive;
use prima_core::render::{render_latex, render_number};
use prima_core::simplify::simplify;
use prima_core::{BuiltinSymbols, ExprId, ExprPool, Number, Real, SymbolTable, Value};
use prima_syntax::ast::{
    AssignOp, BinOp, Block, ConfigBlock, Expr, ExprKind, ImportItem, ImportKind, IndexItem, Literal, MatchArm, Pattern, Program, Spanned, Stmt, Type, UnOp,
};
use prima_syntax::error::SyntaxError;

use crate::builtins::Builtin;
use crate::config::{Config, Domain, UndefinedHandling};
use crate::error::RuntimeError;
use crate::module::{ModuleGraph, ModuleUnit, ResolvedImport};

/// Function value (spec §11): builtins, pure math functions (MFn/closures), and host functions (`fn`); closures carry their defining environment.
#[derive(Clone)]
pub enum Function {
    Builtin(Builtin),
    User { params: Vec<prima_syntax::ast::Param>, body: Expr, env: EnvRef },
    Host { params: Vec<prima_syntax::ast::Param>, ret: Option<Type>, body: Block, env: EnvRef },
}

impl Function {
    pub fn is_pure(&self) -> bool {
        match self {
            Function::Builtin(b) => b.is_pure(),
            // MFn/closures are pure math; `fn` may have side effects (spec §11.2) and does not participate in implicit broadcast.
            Function::User { .. } => true,
            Function::Host { .. } => false,
        }
    }
}

/// Module namespace item (spec §15.2): a public function or value exported by a module.
#[derive(Clone)]
pub enum NamespaceItem {
    Func(Function),
    Val(Value),
}

/// Shared handle for an evaluation environment: an `Rc<RefCell>` shared chain makes block-scope shadowing (spec §12.2)
/// and cross-scope assignment (updating outer variables inside `while`/`for` bodies) both work.
pub type EnvRef = Rc<RefCell<Env>>;

/// Evaluation environment: dual value/function namespaces plus a module namespace plus a shared parent-environment chain.
#[derive(Clone, Default)]
pub struct Env {
    values: HashMap<String, Value>,
    funcs: HashMap<String, Function>,
    modules: HashMap<String, HashMap<String, NamespaceItem>>,
    parent: Option<EnvRef>,
}

impl Env {
    /// Root environment: pre-imports the common builtins of `core` (spec §15.5, currently the available subset).
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
            "to_i32",
            "to_i64",
            "to_f32",
            "to_f64",
            "to_bigint",
            "to_rational",
            "to_bigfloat",
            "to_complex",
            "try_i32",
            "try_i64",
            "try_f64",
            "try_bigint",
            "try_rational",
            "try_complex",
            "checked_i32",
            "checked_u64",
            "checked_add",
            "checked_mul",
            "clamped_i32",
            "clamped_u64",
            "clamped_f64",
            "rounded_f64",
            "rounded_i32",
            "truncated_i32",
            "unwrap",
            "unwrap_or",
            "expect",
        ] {
            if let Some(b) = Builtin::from_name(name) {
                env.set_func(name, Function::Builtin(b));
            }
        }
        env
    }

    /// Wrap into a shared handle (root environment).
    pub fn into_ref(self) -> EnvRef {
        Rc::new(RefCell::new(self))
    }

    /// Create a child scope: empty local tables plus a shared parent handle.
    fn child(parent: &EnvRef) -> EnvRef {
        Rc::new(RefCell::new(Env {
            values: HashMap::new(),
            funcs: HashMap::new(),
            modules: HashMap::new(),
            parent: Some(Rc::clone(parent)),
        }))
    }

    pub fn get_value(&self, name: &str) -> Option<Value> {
        if let Some(v) = self.values.get(name) {
            return Some(v.clone());
        }
        self.parent.as_ref().and_then(|p| p.borrow().get_value(name))
    }

    fn set_value(&mut self, name: &str, v: Value) {
        self.values.insert(name.to_string(), v);
    }

    /// Update an existing binding in place (along the shared chain, spec §12.2 shadowing); return `false` if undefined.
    fn set_existing(&mut self, name: &str, v: Value) -> bool {
        if self.values.contains_key(name) {
            self.values.insert(name.to_string(), v);
            return true;
        }
        if let Some(p) = &self.parent {
            return p.borrow_mut().set_existing(name, v);
        }
        false
    }

    fn get_func(&self, name: &str) -> Option<Function> {
        if let Some(f) = self.funcs.get(name) {
            return Some(f.clone());
        }
        self.parent.as_ref().and_then(|p| p.borrow().get_func(name))
    }

    fn set_func(&mut self, name: &str, f: Function) {
        self.funcs.insert(name.to_string(), f);
    }

    /// Register a module namespace (key is the module path or alias, spec §15.1). Returns `true` if it already existed.
    fn set_module(&mut self, name: &str, items: HashMap<String, NamespaceItem>) -> bool {
        self.modules.insert(name.to_string(), items).is_some()
    }

    /// Resolve `module::item` (spec §15.2 qualified access).
    fn lookup_module_item(&self, ns_key: &str, item: &str) -> Option<NamespaceItem> {
        if let Some(m) = self.modules.get(ns_key)
            && let Some(it) = m.get(item)
        {
            return Some(it.clone());
        }
        self.parent.as_ref().and_then(|p| p.borrow().lookup_module_item(ns_key, item))
    }

    fn bind_item(&mut self, name: &str, item: NamespaceItem) {
        match item {
            NamespaceItem::Func(f) => self.set_func(name, f),
            NamespaceItem::Val(v) => self.set_value(name, v),
        }
    }
}

/// Statement evaluation result: `return` exits non-locally up the call chain via `Flow::Return` (spec §14).
#[derive(Debug, Clone, PartialEq)]
enum Flow {
    Continue,
    Return(Value),
}

/// Interpreter (spec §4.8): the unified AST is degraded in two ways by context — MFn bodies and `let` right-hand sides go through the symbolic world
/// (`ExprDAG` → simplify → `Value::Expr`), while host statements go through numeric evaluation.
pub struct Evaluator {
    pool: &'static ExprPool,
    symbols: &'static SymbolTable,
    builtins: &'static BuiltinSymbols,
    output: Box<dyn FnMut(String)>,
    /// Policy stack (spec §4.6): global defaults at the bottom; module config / `with config` push and pop per block.
    config: Vec<Config>,
    /// Evaluated module public items (indexed by module path), available for `import` binding (spec §15).
    module_items: HashMap<String, HashMap<String, NamespaceItem>>,
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
            config: vec![Config::default()],
            module_items: HashMap::new(),
        }
    }

    pub fn with_sink(output: impl FnMut(String) + 'static) -> Evaluator {
        Evaluator {
            pool: ExprPool::global(),
            symbols: SymbolTable::global(),
            builtins: BuiltinSymbols::global(),
            output: Box::new(output),
            config: vec![Config::default()],
            module_items: HashMap::new(),
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

    fn reset_config(&mut self) {
        self.config.clear();
        self.config.push(Config::default());
    }

    fn current_config(&self) -> &Config {
        self.config.last().unwrap_or(&DEFAULT_CONFIG)
    }

    fn push_module_config(&mut self, block: Option<&ConfigBlock>) -> Result<(), RuntimeError> {
        if let Some(block) = block {
            let mut cfg = self.current_config().clone();
            cfg.apply(&block.entries)?;
            self.config.push(cfg);
        }
        Ok(())
    }

    /// Interpret a file as the root module (spec §15.3 module system plus the pre-imported `core`).
    pub fn eval_file(&mut self, path: &Path) -> Result<(), RuntimeError> {
        let graph = ModuleGraph::load(path).map_err(RuntimeError::Message)?;
        self.reset_config();
        self.module_items.clear();
        // First evaluate each module in dependency order and collect its public items (spec §15.2).
        for dep in &graph.deps {
            if let Err(e) = self.eval_module(dep) {
                self.reset_config();
                return Err(e);
            }
        }
        let root = &graph.root;
        let env = Env::new().into_ref();
        let result = self
            .bind_imports(&env, &root.imports)
            .and_then(|_| self.eval_root(&env, root));
        self.reset_config();
        result
    }

    fn eval_root(&mut self, env: &EnvRef, root: &ModuleUnit) -> Result<(), RuntimeError> {
        self.push_module_config(root.program.config.as_ref())?;
        for stmt in &root.program.stmts {
            if let Flow::Return(_) = self.eval_stmt(env, stmt)? {
                return crate::error::err("`return` outside of a function");
            }
        }
        Ok(())
    }

    /// Evaluate a dependency module (spec §15): apply its module policy and collect `pub` items.
    fn eval_module(&mut self, unit: &ModuleUnit) -> Result<(), RuntimeError> {
        // Polluting policies (spec §13.2) are only allowed in the entry module.
        if let Some(cfg) = &unit.program.config {
            for e in &cfg.entries {
                if e.name.value == "domain" || e.name.value == "undefined_handling" {
                    return Err(RuntimeError::Message(format!(
                        "polluting config `{}` is only allowed in the entry module",
                        e.name.value
                    )));
                }
            }
        }
        let env = Env::new().into_ref();
        self.bind_imports(&env, &unit.imports)?;
        self.push_module_config(unit.program.config.as_ref())?;
        let result = self.eval_module_inner(&env, unit);
        self.config.pop();
        let items = result?;
        self.module_items.insert(unit.path.join("::"), items);
        Ok(())
    }

    fn eval_module_inner(&mut self, env: &EnvRef, unit: &ModuleUnit) -> Result<HashMap<String, NamespaceItem>, RuntimeError> {
        let mut items = HashMap::new();
        for stmt in &unit.program.stmts {
            match stmt {
                Stmt::Pub(inner) => self.collect_pub(env, inner, &mut items)?,
                other => {
                    self.eval_stmt(env, other)?;
                }
            }
        }
        Ok(items)
    }

    fn collect_pub(&mut self, env: &EnvRef, inner: &Stmt, items: &mut HashMap<String, NamespaceItem>) -> Result<(), RuntimeError> {
        match inner {
            Stmt::MathDef { name, params, body, .. } => {
                let f = Function::User { params: params.clone(), body: body.clone(), env: Rc::clone(env) };
                env.borrow_mut().set_func(&name.value, f.clone());
                items.insert(name.value.clone(), NamespaceItem::Func(f));
                Ok(())
            }
            Stmt::FnDef { name, params, ret, body, .. } => {
                let f = Function::Host { params: params.clone(), ret: ret.clone(), body: body.clone(), env: Rc::clone(env) };
                env.borrow_mut().set_func(&name.value, f.clone());
                items.insert(name.value.clone(), NamespaceItem::Func(f));
                Ok(())
            }
            Stmt::Let { name, value, .. } | Stmt::Const { name, value, .. } => {
                let v = self.eval_expr(env, value)?;
                env.borrow_mut().set_value(&name.value, v.clone());
                items.insert(name.value.clone(), NamespaceItem::Val(v));
                Ok(())
            }
            _ => crate::error::err("`pub` only applies to `let`/`const`/`fn`"),
        }
    }

    /// Bind a module's public items into the current environment (spec §15.1/§15.4): namespaces, selective imports, and conflict detection.
    fn bind_imports(&mut self, env: &EnvRef, imports: &[ResolvedImport]) -> Result<(), RuntimeError> {
        let mut bound: HashSet<String> = HashSet::new();
        for ri in imports {
            let key = ri.path.join("::");
            match &ri.kind {
                ImportKind::Namespace { alias, .. } => {
                    let items = self.module_items.get(&key).cloned().ok_or_else(|| {
                        RuntimeError::Message(format!("module `{key}` is not loaded"))
                    })?;
                    let ns = alias.as_ref().map(|a| a.value.clone()).unwrap_or_else(|| key.clone());
                    if env.borrow_mut().set_module(&ns, items) {
                        return crate::error::err(format!("conflicting import: module `{ns}`"));
                    }
                }
                ImportKind::From { items: from_items, .. } => {
                    let module = self.module_items.get(&key).cloned().ok_or_else(|| {
                        RuntimeError::Message(format!("module `{key}` is not loaded"))
                    })?;
                    for it in from_items {
                        match it {
                            ImportItem::Star => {
                                for (name, item) in &module {
                                    if !bound.insert(name.clone()) {
                                        return crate::error::err(format!("conflicting import: `{name}`"));
                                    }
                                    env.borrow_mut().bind_item(name, item.clone());
                                }
                            }
                            ImportItem::Name { name, alias } => {
                                let item = module.get(&name.value).cloned().ok_or_else(|| {
                                    RuntimeError::Message(format!("module `{key}` has no public item `{}`", name.value))
                                })?;
                                let target = alias.as_ref().map(|a| a.value.clone()).unwrap_or_else(|| name.value.clone());
                                if !bound.insert(target.clone()) {
                                    return crate::error::err(format!("conflicting import: `{target}`"));
                                }
                                env.borrow_mut().bind_item(&target, item);
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }

    pub fn eval_program(&mut self, program: &Program) -> Result<(), RuntimeError> {
        self.reset_config();
        if !program.imports.is_empty() {
            return crate::error::err("`import` requires running from a file (`prima run <file>`)");
        }
        let env = Env::new().into_ref();
        let r = self.eval_program_in(&env, program);
        self.reset_config();
        r
    }

    fn eval_program_in(&mut self, env: &EnvRef, program: &Program) -> Result<(), RuntimeError> {
        self.push_module_config(program.config.as_ref())?;
        for stmt in &program.stmts {
            if let Flow::Return(_) = self.eval_stmt(env, stmt)? {
                return crate::error::err("`return` outside of a function");
            }
        }
        Ok(())
    }

    pub fn eval_src(&mut self, src: &str) -> Result<(), RuntimeError> {
        let program = prima_syntax::parse(src).map_err(syntax_errors)?;
        self.eval_program(&program)
    }

    pub fn eval_value(&mut self, src: &str) -> Result<Value, RuntimeError> {
        let program = prima_syntax::parse(src).map_err(syntax_errors)?;
        self.reset_config();
        if !program.imports.is_empty() {
            return crate::error::err("`import` requires running from a file");
        }
        let env = Env::new().into_ref();
        let r = self.eval_value_in(&env, &program);
        self.reset_config();
        r
    }

    fn eval_value_in(&mut self, env: &EnvRef, program: &Program) -> Result<Value, RuntimeError> {
        self.push_module_config(program.config.as_ref())?;
        let mut last = Value::Nil;
        for stmt in &program.stmts {
            if let Stmt::Expr(e) = stmt {
                last = self.eval_expr(env, e)?;
            } else {
                match self.eval_stmt(env, stmt)? {
                    Flow::Continue => {}
                    Flow::Return(_) => return crate::error::err("`return` outside of a function"),
                }
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
            Value::Error(msg) => format!("error: {msg}"),
            Value::Tuple(items) => {
                let inner: Vec<String> = items.iter().map(|it| self.format_value(it)).collect();
                format!("({})", inner.join(", "))
            }
            Value::Result(r) => match r {
                Ok(v) => self.format_value(v),
                Err(msg) => format!("err: {msg}"),
            },
        }
    }

    fn eval_block(&mut self, env: &EnvRef, block: &Block) -> Result<Flow, RuntimeError> {
        let scope = Env::child(env);
        self.eval_block_stmts(&scope, block)
    }

    fn eval_block_stmts(&mut self, env: &EnvRef, block: &Block) -> Result<Flow, RuntimeError> {
        for stmt in &block.stmts {
            match self.eval_stmt(env, stmt)? {
                Flow::Continue => {}
                flow @ Flow::Return(_) => return Ok(flow),
            }
        }
        Ok(Flow::Continue)
    }

    /// Evaluate one statement, attaching its source span to any error (spec §16.4).
    fn eval_stmt(&mut self, env: &EnvRef, stmt: &Stmt) -> Result<Flow, RuntimeError> {
        let span = stmt_span(stmt);
        self.eval_stmt_inner(env, stmt).map_err(|e| crate::error::attach_span(e, span))
    }

    fn eval_stmt_inner(&mut self, env: &EnvRef, stmt: &Stmt) -> Result<Flow, RuntimeError> {
        match stmt {
            Stmt::Let { name, value, .. } => {
                if let ExprKind::Lambda { params, body } = &value.kind {
                    let f = Function::User { params: params.clone(), body: (**body).clone(), env: Rc::clone(env) };
                    env.borrow_mut().set_func(&name.value, f);
                } else {
                    let v = self.eval_expr(env, value)?;
                    env.borrow_mut().set_value(&name.value, v);
                }
                Ok(Flow::Continue)
            }
            Stmt::Const { name, value, .. } => {
                let v = self.eval_expr(env, value)?;
                env.borrow_mut().set_value(&name.value, v);
                Ok(Flow::Continue)
            }
            Stmt::MathDef { name, params, body, .. } => {
                let f = Function::User { params: params.clone(), body: body.clone(), env: Rc::clone(env) };
                env.borrow_mut().set_func(&name.value, f);
                Ok(Flow::Continue)
            }
            Stmt::FnDef { name, params, ret, body, .. } => {
                let f = Function::Host { params: params.clone(), ret: ret.clone(), body: body.clone(), env: Rc::clone(env) };
                env.borrow_mut().set_func(&name.value, f);
                Ok(Flow::Continue)
            }
            Stmt::Expr(e) => {
                self.eval_expr(env, e)?;
                Ok(Flow::Continue)
            }
            Stmt::Assign { target, op, value, .. } => {
                let v = self.eval_expr(env, value)?;
                let name = match &target.kind {
                    ExprKind::Path { segments } if segments.len() == 1 => &segments[0].value,
                    _ => return crate::error::err("assignment target must be a variable"),
                };
                let merged = {
                    let prev = env.borrow().get_value(name);
                    match op {
                        AssignOp::Assign => v,
                        AssignOp::AddAssign => self.eval_binary(BinOp::Add, prev.unwrap_or(Value::Number(Number::from(0))), v)?,
                        AssignOp::SubAssign => self.eval_binary(BinOp::Sub, prev.unwrap_or(Value::Number(Number::from(0))), v)?,
                    }
                };
                // Update in place along the shared chain (spec §12.2 shadowing); create locally if undefined.
                let mut e = env.borrow_mut();
                if !e.set_existing(name, merged.clone()) {
                    e.set_value(name, merged);
                }
                Ok(Flow::Continue)
            }
            Stmt::If { cond, then, elifs, else_, .. } => {
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
            Stmt::While { cond, body, .. } => {
                loop {
                    if !self.eval_cond(env, cond)? {
                        break;
                    }
                    if let flow @ Flow::Return(_) = self.eval_block(env, body)? {
                        return Ok(flow);
                    }
                }
                Ok(Flow::Continue)
            }
            Stmt::For { var, range, step, body, .. } => {
                // Loop formula optimization (spec §10/§19.1): closed form for the arithmetic series `for i in 0..n`/`1..n { acc += i }`.
                if step.is_none()
                    && self.current_config().loop_optimization
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
                    let scope = Env::child(env);
                    scope.borrow_mut().set_value(&var.value, Value::Number(Number::from(i)));
                    if let flow @ Flow::Return(_) = self.eval_block_stmts(&scope, body)? {
                        return Ok(flow);
                    }
                    i += step_v;
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
            Stmt::Try { body, catches, .. } => match self.eval_block(env, body) {
                Ok(flow) => Ok(flow),
                Err(e) => {
                    for c in catches {
                        if let Some(ty) = &c.ty
                            && !self.catch_matches(ty, &e)
                        {
                            continue;
                        }
                        let scope = Env::child(env);
                        scope.borrow_mut().set_value(&c.var.value, Value::String(e.to_string()));
                        return self.eval_block_stmts(&scope, &c.block);
                    }
                    Err(e)
                }
            },
            Stmt::WithConfig { entries, body, .. } => {
                let mut cfg = self.current_config().clone();
                cfg.apply(entries)?;
                self.config.push(cfg);
                let r = self.eval_block(env, body);
                self.config.pop();
                r
            }
            Stmt::Pub(inner) => self.eval_stmt(env, inner),
            Stmt::ParFor { .. } => crate::error::err("`parfor` is not supported yet"),
        }
    }

    /// Type filter for `catch e: Error::Overflow` (spec §16.3), matching by error category name.
    fn catch_matches(&self, ty: &Type, e: &RuntimeError) -> bool {
        let Type::User(name) = ty else { return true };
        let want = name.value.rsplit("::").next().unwrap_or("");
        want == e.kind() || want == "Error"
    }

    fn eval_cond(&mut self, env: &EnvRef, e: &Expr) -> Result<bool, RuntimeError> {
        match self.eval_expr(env, e)? {
            Value::Bool(b) => Ok(b),
            _ => crate::error::err("condition must be a boolean"),
        }
    }

    fn eval_to_i64(&mut self, env: &EnvRef, e: &Expr) -> Result<i64, RuntimeError> {
        match self.eval_expr(env, e)? {
            Value::Number(n) => n
                .as_i64()
                .ok_or_else(|| RuntimeError::Type(format!("loop range must be integers, got {n}"))),
            other => crate::error::err(format!("loop range must be integers, got {other:?}")),
        }
    }

    /// Closed form for an arithmetic sum (spec §10/§19.1): `for i in 0..n { acc += i }` → `n(n-1)/2`,
    /// `for i in 1..n { acc += i }` → `n(n+1)/2` (the 5050 result of the spec §19.1 example).
    fn try_arithmetic_sum(&mut self, env: &EnvRef, var: &Spanned<String>, range: &(Expr, Expr), body: &Block) -> Result<Option<()>, RuntimeError> {
        if body.stmts.len() != 1 {
            return Ok(None);
        }
        let addend_is_var = |e: &Expr| {
            matches!(&e.kind, ExprKind::Path { segments } if segments.len() == 1 && segments[0].value == var.value)
        };
        let acc = match &body.stmts[0] {
            Stmt::Assign { target, op: AssignOp::AddAssign, value, .. } if addend_is_var(value) => match &target.kind {
                ExprKind::Path { segments } if segments.len() == 1 => Some(segments[0].value.clone()),
                _ => None,
            },
            _ => None,
        };
        let Some(acc) = acc else { return Ok(None) };
        let start = self.eval_to_i64(env, &range.0)?;
        let end = self.eval_to_i64(env, &range.1)?;
        let sum = if start == 0 && end > 0 {
            end * (end - 1) / 2
        } else if start == 1 && end >= 1 {
            end * (end + 1) / 2
        } else {
            return Ok(None);
        };
        let prev = env.borrow().get_value(&acc).unwrap_or(Value::Number(Number::from(0)));
        let merged = self.eval_binary(BinOp::Add, prev, Value::Number(Number::from(sum)))?;
        let mut e = env.borrow_mut();
        if !e.set_existing(&acc, merged.clone()) {
            e.set_value(&acc, merged);
        }
        Ok(Some(()))
    }

    /// Evaluate one expression, attaching its source span to any error (spec §16.4).
    fn eval_expr(&mut self, env: &EnvRef, expr: &Expr) -> Result<Value, RuntimeError> {
        let span = expr.span;
        self.eval_expr_inner(env, expr).map_err(|e| crate::error::attach_span(e, span))
    }

    fn eval_expr_inner(&mut self, env: &EnvRef, expr: &Expr) -> Result<Value, RuntimeError> {
        match &expr.kind {
            ExprKind::Literal(lit) => self.eval_literal(env, lit),
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
                        Some(NamespaceItem::Func(_)) => {
                            crate::error::err(format!("function `{item}` cannot be used as a value"))
                        }
                        None => crate::error::err(format!("unknown module item `{ns}::{item}`")),
                    }
                }
            }
            ExprKind::Binary { op: BinOp::Broadcast, lhs, rhs } => self.eval_broadcast_op(env, lhs, rhs),
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
            ExprKind::Tuple(items) => {
                let vals: Result<Vec<Value>, RuntimeError> = items.iter().map(|it| self.eval_expr(env, it)).collect();
                Ok(Value::Tuple(vals?))
            }
            ExprKind::Lambda { .. } => crate::error::err("lambda must be assigned to a variable to be callable"),
            ExprKind::Match { scrutinee, arms } => self.eval_match(env, scrutinee, arms),
            ExprKind::Pipeline { lhs, rhs } => self.eval_pipeline(env, lhs, rhs),
            ExprKind::Custom(_) => crate::error::err("`custom` config block is not valid here"),
        }
    }

    /// `@.` explicit broadcast operator (spec §11.4): not disabled by `broadcast := false`.
    fn eval_broadcast_op(&mut self, env: &EnvRef, lhs: &Expr, rhs: &Expr) -> Result<Value, RuntimeError> {
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

    fn eval_literal(&mut self, env: &EnvRef, lit: &Literal) -> Result<Value, RuntimeError> {
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
                // A TeX literal is parsed into the same AST as ordinary syntax and evaluated uniformly (implementation plan §4.9).
                let tex_ast = prima_syntax::tex::parse_tex(s).map_err(syntax_err)?;
                self.eval_expr(env, &tex_ast)
            }
        }
    }

    /// `Undefined` strictness (spec §6.2): it must not participate in any operation; any input errors immediately (no propagation).
    fn ensure_defined(&self, v: &Value) -> Result<(), RuntimeError> {
        if matches!(v, Value::Undefined) {
            Err(RuntimeError::Undefined("`Undefined` cannot participate in operations".into()))
        } else {
            Ok(())
        }
    }

    fn eval_binary(&mut self, op: BinOp, a: Value, b: Value) -> Result<Value, RuntimeError> {
        self.ensure_defined(&a)?;
        self.ensure_defined(&b)?;
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
            _ => crate::error::err("arithmetic operator required"),
        }
    }

    fn eval_pow(&mut self, x: Number, y: Number) -> Result<Value, RuntimeError> {
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
                        "negative base with a fractional exponent requires `domain := complex`".into(),
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
        let simp = simplify(self.pool, self.builtins, node);
        Ok(self.value_from_expr(simp))
    }

    /// `undefined_handling := custom { 0/0 := v }` (spec §13.4): literal values are returned directly.
    fn custom_zero_div(&self) -> Option<Value> {
        let cfg = self.current_config();
        if cfg.undefined_handling != UndefinedHandling::Custom {
            return None;
        }
        for (p, v) in &cfg.custom_rules {
            if let ExprKind::Binary { op: BinOp::Div, lhs, rhs } = &p.kind
                && is_zero_literal(lhs)
                && is_zero_literal(rhs)
            {
                return literal_value(v);
            }
        }
        None
    }

    fn eval_compare(&mut self, op: BinOp, a: Value, b: Value) -> Result<Value, RuntimeError> {
        use std::cmp::Ordering;
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
        self.ensure_defined(&v)?;
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

    fn eval_call(&mut self, env: &EnvRef, callee: &Expr, args: &[Expr]) -> Result<Value, RuntimeError> {
        let mut arg_values = Vec::with_capacity(args.len());
        for a in args {
            arg_values.push(self.eval_expr(env, a)?);
        }
        let func = match &callee.kind {
            ExprKind::Path { segments } => self.resolve_func(env, segments).ok_or_else(|| {
                RuntimeError::Message(format!("unknown function `{}`", path_key(segments)))
            })?,
            _ => return crate::error::err("invalid function call"),
        };
        self.apply_function(&func, arg_values)
    }

    fn resolve_func(&self, env: &EnvRef, segments: &[Spanned<String>]) -> Option<Function> {
        if segments.len() == 1 {
            env.borrow().get_func(&segments[0].value)
        } else {
            let ns = path_key(&segments[..segments.len() - 1]);
            match env.borrow().lookup_module_item(&ns, &segments[segments.len() - 1].value) {
                Some(NamespaceItem::Func(f)) => Some(f),
                _ => None,
            }
        }
    }

    fn eval_index(&mut self, env: &EnvRef, base: &Expr, index: &prima_syntax::ast::Index) -> Result<Value, RuntimeError> {
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
                    .ok_or_else(|| RuntimeError::IndexOutOfBounds(format!("index {idx} (length {})", arr.len())))
            }
            IndexItem::Slice { .. } => crate::error::err("array slicing is not supported yet"),
        }
    }

    fn eval_pipeline(&mut self, env: &EnvRef, lhs: &Expr, rhs: &Expr) -> Result<Value, RuntimeError> {
        let v = self.eval_expr(env, lhs)?;
        match &rhs.kind {
            ExprKind::Path { segments } => {
                let func = self.resolve_func(env, segments).ok_or_else(|| {
                    RuntimeError::Message(format!("unknown function `{}`", path_key(segments)))
                })?;
                self.apply_function(&func, vec![v])
            }
            ExprKind::Call { callee, args } => {
                let func = match &callee.kind {
                    ExprKind::Path { segments } => self.resolve_func(env, segments).ok_or_else(|| {
                        RuntimeError::Message(format!("unknown function `{}`", path_key(segments)))
                    })?,
                    _ => return crate::error::err("invalid pipeline target"),
                };
                let mut cargs = vec![v];
                for a in args {
                    cargs.push(self.eval_expr(env, a)?);
                }
                self.apply_function(&func, cargs)
            }
            ExprKind::Lambda { params, body } => {
                let func = Function::User { params: params.clone(), body: (**body).clone(), env: Rc::clone(env) };
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
            if self.current_config().broadcast {
                return self.broadcast_call(func, args, &array_positions);
            }
            return crate::error::err("implicit broadcast is disabled (`broadcast := false`); use `@.`");
        }
        match func {
            Function::Builtin(b) => self.call_builtin(*b, args),
            Function::User { params, body, env: f_env } => {
                if args.len() != params.len() {
                    return crate::error::err(format!("expected {} arguments, got {}", params.len(), args.len()));
                }
                let call_env = Env::child(f_env);
                for (p, a) in params.iter().zip(args) {
                    call_env.borrow_mut().set_value(&p.name.value, a);
                }
                self.eval_expr(&call_env, body)
            }
            Function::Host { params, ret: _, body, env: f_env } => {
                if args.len() != params.len() {
                    return crate::error::err(format!("expected {} arguments, got {}", params.len(), args.len()));
                }
                let call_env = Env::child(f_env);
                for (p, a) in params.iter().zip(args) {
                    call_env.borrow_mut().set_value(&p.name.value, a);
                }
                match self.eval_block(&call_env, body)? {
                    Flow::Continue => Ok(Value::Nil),
                    Flow::Return(v) => Ok(v),
                }
            }
        }
    }

    /// Broadcast (spec §11.4): pure functions are applied elementwise to array arguments; **nested/empty arrays are rejected** (error code), scalars align automatically.
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

    /// Binary array operation broadcast (spec §11.4): array×array is elementwise (lengths must match), array×scalar broadcasts the scalar; empty arrays error.
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

    /// `match` evaluation (spec §16.3): currently supports matching `Ok`/`Err` branches of a `Result`.
    fn eval_match(&mut self, env: &EnvRef, scrutinee: &Expr, arms: &[MatchArm]) -> Result<Value, RuntimeError> {
        let sv = self.eval_expr(env, scrutinee)?;
        for arm in arms {
            if let Some(bindings) = self.try_match(&sv, &arm.pattern) {
                let scope = Env::child(env);
                for (name, v) in bindings {
                    scope.borrow_mut().set_value(&name, v);
                }
                return self.eval_expr(&scope, &arm.body);
            }
        }
        crate::error::err("`match` is non-exhaustive")
    }

    fn try_match(&self, v: &Value, p: &Pattern) -> Option<Vec<(String, Value)>> {
        match p {
            Pattern::Ctor { name, args, .. } => {
                let ctor = name.last()?.value.as_str();
                let Value::Result(r) = v else { return None };
                match ctor {
                    "Ok" => match r {
                        Ok(inner) => bind_pattern_args(inner, args),
                        Err(_) => None,
                    },
                    "Err" => match r {
                        Ok(_) => None,
                        Err(msg) => bind_pattern_args(&Value::String(msg.clone()), args),
                    },
                    _ => None,
                }
            }
            Pattern::Wildcard(_) => Some(vec![]),
            Pattern::Binding(name) => Some(vec![(name.value.clone(), v.clone())]),
            _ => None,
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

    // A pure constant node folds back to `Value::Number`; otherwise the symbolic form is preserved (spec §6.6 exact by default).
    fn value_from_expr(&self, id: ExprId) -> Value {
        if let Some(n) = self.pool.const_number(id) {
            Value::Number(n)
        } else {
            Value::Expr(id)
        }
    }
}

/// Source span of a statement, used to locate errors (spec §16.4).
fn stmt_span(stmt: &Stmt) -> prima_syntax::Span {
    match stmt {
        Stmt::Let { span, .. }
        | Stmt::Const { span, .. }
        | Stmt::FnDef { span, .. }
        | Stmt::MathDef { span, .. }
        | Stmt::Assign { span, .. }
        | Stmt::For { span, .. }
        | Stmt::ParFor { span, .. }
        | Stmt::While { span, .. }
        | Stmt::If { span, .. }
        | Stmt::Return { span, .. }
        | Stmt::Try { span, .. }
        | Stmt::WithConfig { span, .. } => *span,
        Stmt::Expr(e) => e.span,
        Stmt::Pub(inner) => stmt_span(inner),
    }
}

static DEFAULT_CONFIG: Config = Config {
    domain: Domain::Complex,
    undefined_handling: UndefinedHandling::Strict,
    custom_rules: Vec::new(),
    fraction: true,
    broadcast: true,
    loop_optimization: true,
    simplify_level: 2,
    num_to_big: true,
    print_format: crate::config::PrintFormat::Latex,
};

fn path_key(segments: &[Spanned<String>]) -> String {
    segments.iter().map(|s| s.value.as_str()).collect::<Vec<_>>().join("::")
}

fn bind_pattern_args(v: &Value, patterns: &[Pattern]) -> Option<Vec<(String, Value)>> {
    if patterns.len() != 1 {
        return None;
    }
    match &patterns[0] {
        Pattern::Binding(name) => Some(vec![(name.value.clone(), v.clone())]),
        Pattern::Wildcard(_) => Some(vec![]),
        _ => None,
    }
}

fn is_zero_literal(e: &Expr) -> bool {
    matches!(&e.kind, ExprKind::Literal(Literal::Integer(s)) if s == "0")
}

fn literal_value(e: &Expr) -> Option<Value> {
    match &e.kind {
        ExprKind::Literal(Literal::Integer(s)) => s.parse::<BigInt>().ok().map(Number::Integer).map(Value::Number),
        ExprKind::Literal(Literal::Bool(b)) => Some(Value::Bool(*b)),
        ExprKind::Literal(Literal::Str(s)) => Some(Value::String(s.clone())),
        ExprKind::Unary { op: UnOp::Neg, operand } => literal_value(operand).map(|v| match v {
            Value::Number(n) => Value::Number(-n),
            other => other,
        }),
        _ => None,
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
