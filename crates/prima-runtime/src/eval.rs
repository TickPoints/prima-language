use std::cell::RefCell;
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::rc::Rc;

use num_bigint::BigInt;
use prima_core::render::{render_latex, render_number};
use prima_core::simplify::simplify;
use prima_core::{BuiltinSymbols, ExprData, ExprId, ExprPool, Number, Real, SymbolId, SymbolTable, Value};
use prima_syntax::ast::{
    Annotation, AssignOp, BinOp, Block, ClassMemberKind, ConfigBlock, Expr, ExprKind, FieldValue,
    ImportItem, ImportKind, ImplOp, IndexItem, Literal, MatchArm, Param, Pattern, Program, Spanned,
    Stmt, Type, UnOp, Visibility,
};
use prima_syntax::error::SyntaxError;
use prima_syntax::{Span, SyntaxWarning};
use rayon::prelude::*;

use crate::builtins::Builtin;
use crate::class::{ClassDef, ClassInstance, FieldDef, MethodDef};
use crate::config::{Config, Domain, OverloadPolicy, UndefinedHandling};
use crate::error::RuntimeError;
use crate::module::{ModuleGraph, ModuleUnit, ResolvedImport};

/// Function value (spec §11): builtins, pure math functions (MFn/closures), host functions (`fn`),
/// and a small set of native host functions (`get`). Closures carry their defining environment.
/// `parallel` (v2.1, spec §17.1) marks `@parallel` MFn: their bodies must be self-contained
/// (parameters + builtin symbols only), so a broadcast call can be split across rayon threads.
#[derive(Clone)]
pub enum Function {
    Builtin(Builtin),
    User { params: Vec<Param>, body: Expr, env: EnvRef, parallel: bool },
    Host { params: Vec<Param>, ret: Option<Type>, body: Block, env: EnvRef },
    /// `get(array, index) -> Option<Number>`: safe array access returning `None` out of range (spec §11.3).
    NativeGet,
}

impl Function {
    pub fn is_pure(&self) -> bool {
        match self {
            Function::Builtin(b) => b.is_pure(),
            // MFn/closures are pure math; `fn` may have side effects (spec §11.2) and does not participate in implicit broadcast.
            Function::User { .. } => true,
            Function::Host { .. } => false,
            // `get` returns an `Option`, which broadcast would misinterpret; keep it out of the elementwise path.
            Function::NativeGet => false,
        }
    }
}

/// Module namespace item (spec §15.2): a public function, value, or class exported by a module.
#[derive(Clone)]
pub enum NamespaceItem {
    Func(Function),
    Val(Value),
    Class(ClassDef),
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
    /// Root environment: pre-imports the common builtins of `core` (spec §15.5) plus `get` (spec §11.3).
    pub fn new() -> Env {
        let mut env = Env::default();
        for name in [
            "print",
            "println",
            "input",
            "read_line",
            "simplify",
            "derivative",
            "partial",
            "grad",
            "limit",
            "range",
            "sqrt",
            "exp",
            "log",
            "ln",
            "sin",
            "cos",
            "tan",
            "abs",
            "to_i8",
            "to_i16",
            "to_i32",
            "to_i64",
            "to_i128",
            "to_u8",
            "to_u16",
            "to_u32",
            "to_u64",
            "to_u128",
            "to_isize",
            "to_usize",
            "to_f32",
            "to_f64",
            "to_bigint",
            "to_rational",
            "to_bigfloat",
            "to_complex",
            "try_i8",
            "try_i16",
            "try_i32",
            "try_i64",
            "try_i128",
            "try_u8",
            "try_u16",
            "try_u32",
            "try_u64",
            "try_u128",
            "try_isize",
            "try_usize",
            "try_f32",
            "try_f64",
            "try_bigint",
            "try_rational",
            "try_complex",
            "checked_i8",
            "checked_i16",
            "checked_i32",
            "checked_i64",
            "checked_i128",
            "checked_u8",
            "checked_u16",
            "checked_u32",
            "checked_u64",
            "checked_u128",
            "checked_add",
            "checked_mul",
            "clamped_i8",
            "clamped_i16",
            "clamped_i32",
            "clamped_i64",
            "clamped_i128",
            "clamped_u8",
            "clamped_u16",
            "clamped_u32",
            "clamped_u64",
            "clamped_u128",
            "clamped_f32",
            "clamped_f64",
            "rounded_f64",
            "rounded_f32",
            "rounded_i32",
            "truncated_i32",
            "unwrap",
            "unwrap_or",
            "expect",
            "format",
            "to_string",
            "concat",
            "Some",
            "None",
            "Ok",
            "Err",
        ] {
            if let Some(b) = Builtin::from_name(name) {
                env.set_func(name, Function::Builtin(b));
            }
        }
        env.set_func("get", Function::NativeGet);
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
            // Classes live in the evaluator's class registry; `bind_imports` registers them there directly.
            NamespaceItem::Class(_) => {}
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
    /// Collected warnings (spec §16.5): parse warnings plus `W0002`/`W0005` from the evaluator.
    warnings: Vec<SyntaxWarning>,
    /// Class registry (spec §4.7): class name → definition, shared across the evaluation run.
    class_defs: HashMap<String, ClassDef>,
    /// Instance table (spec §5): `Value::Class(id)` → runtime object.
    instances: HashMap<u32, ClassInstance>,
    /// Monotonic instance-id allocator.
    next_instance_id: u32,
    /// Operator overloads (spec §18.5): key `"<class>::<Op>"` → method definition (`ImplOp` has no `Hash`).
    overloads: HashMap<String, MethodDef>,
    /// Stack of the `self` receiver instance ids of the methods currently executing (spec §4.5).
    self_stack: Vec<u32>,
    /// Module path currently being evaluated (`""` for the root module), for `pub(mod)` visibility (spec §15.2).
    current_module: String,
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
            warnings: Vec::new(),
            class_defs: HashMap::new(),
            instances: HashMap::new(),
            next_instance_id: 0,
            overloads: HashMap::new(),
            self_stack: Vec::new(),
            current_module: String::new(),
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
            warnings: Vec::new(),
            class_defs: HashMap::new(),
            instances: HashMap::new(),
            next_instance_id: 0,
            overloads: HashMap::new(),
            self_stack: Vec::new(),
            current_module: String::new(),
        }
    }

    /// Fresh evaluator for a rayon task (spec §17): shares the process-global pool/symbols/builtins,
    /// inherits a Config snapshot, and discards output — parallel paths only run pure math.
    pub(crate) fn spawn_task_evaluator(cfg: &Config) -> Evaluator {
        let mut ev = Evaluator::with_sink(|_| {});
        ev.config.clear();
        ev.config.push(cfg.clone());
        ev
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

    /// Warnings collected since the last parse entry point (spec §16.5): parse warnings (`W0001`)
    /// plus deprecation/overload warnings emitted during evaluation (`W0002`/`W0005`).
    pub fn warnings(&self) -> &[SyntaxWarning] {
        &self.warnings
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

    /// Record a warning (spec §16.5). Spans of the offending construct are threaded where cheaply
    /// available; operator-overload warnings (`W0005`) carry a zero span (the operator values are
    /// already evaluated when the overload is dispatched).
    fn push_warning(&mut self, code: &'static str, span: Span, message: String) {
        self.warnings.push(SyntaxWarning { span, code, message });
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
        let prev_module = std::mem::take(&mut self.current_module);
        self.current_module = unit.path.join("::");
        let env = Env::new().into_ref();
        let result = (|| {
            self.bind_imports(&env, &unit.imports)?;
            self.push_module_config(unit.program.config.as_ref())?;
            let r = self.eval_module_inner(&env, unit);
            self.config.pop();
            r
        })();
        self.current_module = prev_module;
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
            Stmt::MathDef { name, params, annotations, body, .. } => {
                let parallel = annotations.contains(&Annotation::Parallel);
                let f = Function::User { params: params.clone(), body: body.clone(), env: Rc::clone(env), parallel };
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
            Stmt::Let { pat: Pattern::Binding(name), value, .. } | Stmt::Const { name, value, .. } => {
                let v = self.eval_expr(env, value)?;
                env.borrow_mut().set_value(&name.value, v.clone());
                items.insert(name.value.clone(), NamespaceItem::Val(v));
                Ok(())
            }
            Stmt::ClassDef { name, members, .. } => {
                let def = self.build_class_def(name, members, env);
                self.register_class(def.clone());
                items.insert(def.name.clone(), NamespaceItem::Class(def));
                Ok(())
            }
            _ => crate::error::err("`pub` only applies to `let`/`const`/`fn`/`class`"),
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
                    for item in items.values() {
                        if let NamespaceItem::Class(def) = item {
                            self.register_class(def.clone());
                        }
                    }
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
                                    self.bind_imported_item(env, name, item);
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
                                self.bind_imported_item(env, &target, &item);
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// Bind one imported item into the environment; classes are registered in the class registry (spec §15.2).
    fn bind_imported_item(&mut self, env: &EnvRef, name: &str, item: &NamespaceItem) {
        match item {
            NamespaceItem::Class(def) => self.register_class(def.clone()),
            other => env.borrow_mut().bind_item(name, other.clone()),
        }
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
        let (program, errors, warnings) = prima_syntax::parse_checked(src);
        if !errors.is_empty() {
            return Err(syntax_errors(errors));
        }
        self.warnings = warnings;
        self.eval_program(&program)
    }

    pub fn eval_value(&mut self, src: &str) -> Result<Value, RuntimeError> {
        let (program, errors, warnings) = prima_syntax::parse_checked(src);
        if !errors.is_empty() {
            return Err(syntax_errors(errors));
        }
        self.warnings = warnings;
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
            } else if let Stmt::Match { scrutinee, arms, .. } = stmt {
                // `match` is an expression (spec §4.4); as a statement it still yields a value when it is the last one.
                last = self.eval_match(env, scrutinee, arms)?;
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
            Value::Class(id) => match self.instances.get(id) {
                Some(inst) => format!("class {}", inst.class),
                None => format!("class {id}"),
            },
            Value::Tuple(items) => {
                let inner: Vec<String> = items.iter().map(|it| self.format_value(it)).collect();
                format!("({})", inner.join(", "))
            }
            Value::Result(r) => match r {
                Ok(v) => self.format_value(v),
                Err(msg) => format!("err: {msg}"),
            },
            Value::Option(Some(v)) => self.format_value(v),
            Value::Option(None) => "none".into(),
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

    /// Evaluate a function/method body with implicit tail-return of the last expression statement
    /// (spec §4.5 method examples such as `get_a`/`new` end in a bare expression).
    fn eval_block_tail(&mut self, env: &EnvRef, block: &Block) -> Result<Value, RuntimeError> {
        let n = block.stmts.len();
        for (i, stmt) in block.stmts.iter().enumerate() {
            if i == n - 1 && let Stmt::Expr(e) = stmt {
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
    fn eval_stmt(&mut self, env: &EnvRef, stmt: &Stmt) -> Result<Flow, RuntimeError> {
        let span = stmt_span(stmt);
        self.eval_stmt_inner(env, stmt).map_err(|e| crate::error::attach_span(e, span))
    }

    fn eval_stmt_inner(&mut self, env: &EnvRef, stmt: &Stmt) -> Result<Flow, RuntimeError> {
        match stmt {
            Stmt::Let { pat, value, .. } => {
                // `let name = lambda` binds a function (spec §11.1); other patterns destructure the value (spec §4.4).
                if let Pattern::Binding(name) = pat
                    && let ExprKind::Lambda { params, body } = &value.kind
                {
                    let f = Function::User { params: params.clone(), body: (**body).clone(), env: Rc::clone(env), parallel: false };
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
            Stmt::MathDef { name, params, annotations, body, .. } => {
                let parallel = annotations.contains(&Annotation::Parallel);
                let f = Function::User { params: params.clone(), body: body.clone(), env: Rc::clone(env), parallel };
                env.borrow_mut().set_func(&name.value, f);
                Ok(Flow::Continue)
            }
            Stmt::FnDef { name, params, ret, annotations, body, .. } => {
                // `@builtin fn` (spec §18.4): bind to the Rust host builtin of the same name; unregistered → E0055.
                if annotations.contains(&Annotation::Builtin) {
                    match Builtin::from_name(&name.value) {
                        Some(b) => {
                            env.borrow_mut().set_func(&name.value, Function::Builtin(b));
                            Ok(Flow::Continue)
                        }
                        None => crate::error::err(format!(
                            "unregistered `@builtin` function `{}` (E0055)",
                            name.value
                        )),
                    }
                } else {
                    let f = Function::Host { params: params.clone(), ret: ret.clone(), body: body.clone(), env: Rc::clone(env) };
                    env.borrow_mut().set_func(&name.value, f);
                    Ok(Flow::Continue)
                }
            }
            Stmt::ClassDef { name, members, .. } => {
                let def = self.build_class_def(name, members, env);
                self.register_class(def);
                Ok(Flow::Continue)
            }
            Stmt::Impl { op, target, members, .. } => {
                // Operator overload methods (spec §18.5): `impl ops::Add for T { fn add(self, ...) { ... } }`.
                for m in members {
                    match m.as_ref() {
                        Stmt::FnDef { params, ret, body, .. } => {
                            let def = MethodDef {
                                params: params.clone(),
                                ret: ret.clone(),
                                body: Some(body.clone()),
                                vis: Visibility::Public,
                                env: Rc::clone(env),
                            };
                            self.overloads.insert(overload_key(&target.value, *op), def);
                        }
                        Stmt::MathDef { params, ret, body, .. } => {
                            let block = Block { stmts: vec![Stmt::Expr(body.clone())], span: body.span };
                            let def = MethodDef {
                                params: params.clone(),
                                ret: ret.clone(),
                                body: Some(block),
                                vis: Visibility::Public,
                                env: Rc::clone(env),
                            };
                            self.overloads.insert(overload_key(&target.value, *op), def);
                        }
                        _ => return crate::error::err("`impl` body must contain function definitions"),
                    }
                }
                Ok(Flow::Continue)
            }
            Stmt::Expr(e) => {
                self.eval_expr(env, e)?;
                Ok(Flow::Continue)
            }
            Stmt::Assign { target, op, value, .. } => {
                let v = self.eval_expr(env, value)?;
                // Array element assignment `A[i] = v` (spec §11.3): writes back through the array binding.
                if let ExprKind::Index { base, index } = &target.kind {
                    let (name, mut arr) = self.eval_array_lvalue(env, base)?;
                    if index.items.len() != 1 {
                        return crate::error::err("multi-dimensional indexing is not supported yet");
                    }
                    let idx = match &index.items[0] {
                        IndexItem::Elem(e) => self.eval_index_number(env, e)?,
                        IndexItem::Slice { .. } => return crate::error::err("slice assignment is not supported"),
                    };
                    if idx >= arr.len() {
                        return Err(RuntimeError::IndexOutOfBounds(format!("index {idx} (length {})", arr.len())));
                    }
                    let nv = self.scalar_value(v)?;
                    let merged = match op {
                        AssignOp::Assign => nv,
                        AssignOp::AddAssign => {
                            let r = self.eval_number_binary(BinOp::Add, arr[idx].clone(), nv)?;
                            self.scalar_value(r)?
                        }
                        AssignOp::SubAssign => {
                            let r = self.eval_number_binary(BinOp::Sub, arr[idx].clone(), nv)?;
                            self.scalar_value(r)?
                        }
                    };
                    arr[idx] = merged;
                    let mut e = env.borrow_mut();
                    if !e.set_existing(&name, Value::Array(arr.clone())) {
                        e.set_value(&name, Value::Array(arr));
                    }
                    return Ok(Flow::Continue);
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
            Stmt::IfLet { pat, value, then, else_, .. } => {
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
                    if !self.eval_cond(env, cond)? {
                        break;
                    }
                    if let flow @ Flow::Return(_) = self.eval_block(env, body)? {
                        return Ok(flow);
                    }
                }
                Ok(Flow::Continue)
            }
            Stmt::WhileLet { pat, value, body, .. } => {
                loop {
                    let v = self.eval_expr(env, value)?;
                    let Some(bindings) = self.match_pattern(env, &v, pat) else { break };
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
            Stmt::Match { scrutinee, arms, .. } => {
                self.eval_match(env, scrutinee, arms)?;
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
            Stmt::WithConfig { entries, body, .. } => {
                let mut cfg = self.current_config().clone();
                cfg.apply(entries)?;
                self.config.push(cfg);
                let r = self.eval_block(env, body);
                self.config.pop();
                r
            }
            Stmt::Pub(inner) => self.eval_stmt(env, inner),
            Stmt::ParFor { var, range, step, body, .. } => self.eval_parfor(env, var, range, step, body),
        }
    }

    /// Evaluate an array lvalue `A` (a plain variable holding an array), for `A[i] = v` (spec §11.3).
    fn eval_array_lvalue(&mut self, env: &EnvRef, base: &Expr) -> Result<(String, Vec<Number>), RuntimeError> {
        match &base.kind {
            ExprKind::Path { segments } if segments.len() == 1 => {
                let name = segments[0].value.clone();
                match self.eval_expr(env, base)? {
                    Value::Array(a) => Ok((name, a)),
                    _ => crate::error::err("assignment target must be an array"),
                }
            }
            _ => crate::error::err("assignment target must be a variable"),
        }
    }

    /// `parfor` (spec §17.2): explicit parallel loop over a range. The body is statically checked to be
    /// side-effect free — only index-slot assignments (`A[i] = …`/`+=`) and pure function calls are allowed
    /// (`E0082`). Each iteration's new slot values are computed on rayon threads with independent evaluators,
    /// then the whole arrays are written back to their bindings in deterministic order.
    fn eval_parfor(
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

        // Snapshot the arrays being written (spec §17.2): read once, write back once.
        let mut arrays: HashMap<String, Vec<Number>> = HashMap::new();
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
                            _ => return crate::error::err(format!("parfor target `{}` must be an array", w.array)),
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
        let n = if step_v > 0 {
            if start >= end { 0 } else { (end - start - 1) / step_v + 1 }
        } else if start <= end {
            0
        } else {
            (start - end - 1) / (-step_v) + 1
        };

        // Materialize the loop-index sequence, then process it in rayon chunks so each task evaluator
        // (and its read-only array bindings) is created once per thread rather than once per element.
        let mut indices = Vec::with_capacity(n as usize);
        if step_v > 0 {
            let mut i = start;
            while i < end {
                indices.push(i);
                i += step_v;
            }
        } else {
            let mut i = start;
            while i > end {
                indices.push(i);
                i += step_v;
            }
        }

        let steps_owned = steps.clone();
        let arrays_ro_c = arrays_ro.clone();
        let chunk = (indices.len().max(1) / rayon::current_num_threads().max(1)).max(1);
        // Pre-resolve the read-only outer values so task threads never touch the (non-`Send`) env chain.
        let outer_reads: HashMap<String, Value> = read_names
            .iter()
            .filter_map(|name| env.borrow().get_value(name).map(|v| (name.clone(), v)))
            .collect();
        let writes: Vec<Result<ParforWriteVec, RuntimeError>> = indices
            .par_chunks(chunk)
            .map(|chunk| {
                let mut ev = Evaluator::spawn_task_evaluator(&cfg);
                let call_env = Rc::new(RefCell::new(Env::new()));
                // Bind read-only outer values (including the target arrays as pre-loop snapshots) so the
                // body may read `A[j]`/outer scalars while writing independent slots.
                for (name, v) in &outer_reads {
                    call_env.borrow_mut().set_value(name, v.clone());
                }
                let mut out: Vec<(String, usize, Number)> = Vec::new();
                for &i in chunk {
                    call_env.borrow_mut().set_value(&var_name, Value::Number(Number::from(i)));
                    for s in &steps_owned {
                        match s {
                            ParforStep::Eval(e) => {
                                ev.eval_expr(&call_env, e)?;
                            }
                            ParforStep::Assign(w) => {
                                let idx = match ev.eval_expr(&call_env, &w.index)? {
                                    Value::Number(n) => n.as_usize().ok_or_else(|| {
                                        RuntimeError::Message("parfor index must be a non-negative integer".into())
                                    })?,
                                    _ => return Err(RuntimeError::Message("parfor index must be an integer".into())),
                                };
                                let nv = match ev.eval_expr(&call_env, &w.value)? {
                                    Value::Number(n) => n,
                                    _ => return Err(RuntimeError::Message("parfor assignment value must be numeric".into())),
                                };
                                let merged = match w.op {
                                    AssignOp::Assign => nv,
                                    AssignOp::AddAssign | AssignOp::SubAssign => {
                                        let old = match arrays_ro_c.get(&w.array).and_then(|a| a.get(idx)) {
                                            Some(old) => old.clone(),
                                            None => {
                                                return Err(RuntimeError::IndexOutOfBounds(format!(
                                                    "index {idx} (length {})",
                                                    arrays_ro_c.get(&w.array).map(|a| a.len()).unwrap_or(0)
                                                )));
                                            }
                                        };
                                        let op = if w.op == AssignOp::AddAssign { BinOp::Add } else { BinOp::Sub };
                                        match ev.eval_number_binary(op, old, nv)? {
                                            Value::Number(n) => n,
                                            _ => return Err(RuntimeError::Message("parfor assignment result must be numeric".into())),
                                        }
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
                    return Err(RuntimeError::IndexOutOfBounds(format!("index {idx} (length {})", arr.len())));
                }
                arr[idx] = val;
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

    fn scalar_value(&self, v: Value) -> Result<Number, RuntimeError> {
        match v {
            Value::Number(n) => Ok(n),
            _ => crate::error::err("array elements must be numbers"),
        }
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
                        Some(NamespaceItem::Class(_)) => {
                            crate::error::err(format!("class `{item}` cannot be used as a value"))
                        }
                        None => crate::error::err(format!("unknown module item `{ns}::{item}`")),
                    }
                }
            }
            ExprKind::Self_ => {
                let id = *self
                    .self_stack
                    .last()
                    .ok_or_else(|| RuntimeError::Message("`self` outside of a method".into()))?;
                Ok(Value::Class(id))
            }
            ExprKind::Call { callee, args } => self.eval_call(env, callee, args),
            ExprKind::MethodCall { receiver, name, args } => self.eval_method_call(env, receiver, name, args),
            ExprKind::Field { receiver, name } => self.eval_field(env, receiver, name),
            ExprKind::StructLiteral { name, fields, base } => {
                self.eval_struct_literal(env, name, fields, base.as_deref())
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
            ExprKind::Index { base, index } => self.eval_index(env, base, index),
            ExprKind::Try(inner) => {
                // `?` operator (spec §16.3): propagates `Err`/`None` as a runtime error (checked statically in `check`).
                let v = self.eval_expr(env, inner)?;
                match v {
                    Value::Result(Ok(v)) => Ok(*v),
                    Value::Result(Err(m)) => Err(RuntimeError::Message(m)),
                    Value::Option(Some(v)) => Ok(*v),
                    Value::Option(None) => Err(RuntimeError::Message("`?` on a `None` value".into())),
                    other => crate::error::err(format!("`?` expects a `Result` or `Option`, got {}", value_type_name(&other))),
                }
            }
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
            ExprKind::Pipeline { lhs, rhs } => {
                // Deprecated pipeline (spec §9.7/§16.5 W0002): rewritten as a call.
                self.push_warning("W0002", expr.span, "`|>` pipeline is deprecated (spec §9.7); use class methods".into());
                self.eval_pipeline(env, lhs, rhs)
            }
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
        // Operator overload (spec §18.5): a class operand with a registered overload for this op dispatches to the method.
        if let Some(r) = self.try_overload_binary(op, &a, &b) {
            return r;
        }
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

    /// Try to dispatch a binary operator to a registered class overload (spec §18.5).
    fn try_overload_binary(&mut self, op: BinOp, a: &Value, b: &Value) -> Option<Result<Value, RuntimeError>> {
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
        let Value::Class(id) = &self_v else { return None };
        let class = self.instances.get(id).map(|i| i.class.clone())?;
        if !self.overloads.contains_key(&overload_key(&class, impl_op)) {
            return None;
        }
        Some(self.overload_dispatch(&class, impl_op, self_v, vec![other_v]))
    }

    /// Dispatch an operator overload method: policy check (spec §13.2 `overload_policy`) then a method call.
    fn overload_dispatch(&mut self, class: &str, op: ImplOp, self_v: Value, args: Vec<Value>) -> Result<Value, RuntimeError> {
        let method = self.overloads.get(&overload_key(class, op)).cloned().ok_or_else(|| {
            RuntimeError::Message(format!("no `{op:?}` overload registered for `{class}`"))
        })?;
        match self.current_config().overload_policy {
            OverloadPolicy::Deny => {
                return crate::error::err(format!(
                    "operator overload for `{class}` is denied by `overload_policy`"
                ))
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
            UnOp::Neg => {
                if let Value::Class(id) = &v
                    && let Some(class) = self.instances.get(id).map(|i| i.class.clone())
                {
                    if self.overloads.contains_key(&overload_key(&class, ImplOp::Neg)) {
                        return self.overload_dispatch(&class, ImplOp::Neg, v, vec![]);
                    }
                    return crate::error::err("cannot negate a class instance");
                }
                match v {
                    Value::Number(n) => Ok(Value::Number(-n)),
                    Value::Array(elems) => Ok(Value::Array(elems.into_iter().map(|n| -n).collect())),
                    Value::Expr(id) => {
                        let node = self.pool.mul2(self.pool.integer(-1), id);
                        let simp = simplify(self.pool, self.builtins, node);
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

    fn eval_call(&mut self, env: &EnvRef, callee: &Expr, args: &[Expr]) -> Result<Value, RuntimeError> {
        // Symbolic differentiation (spec §19.4): `derivative`/`partial`/`grad`/`limit` are intercepted
        // before generic argument evaluation, so the first argument may be an MFn *name* (functions are
        // not first-class values) as well as a symbolic expression.
        if let ExprKind::Path { segments } = &callee.kind
            && segments.len() == 1
            && let Some(Function::Builtin(b)) = self.resolve_func(env, segments)
            && matches!(b, Builtin::Derivative | Builtin::Partial | Builtin::Grad | Builtin::Limit)
        {
            return self.eval_calc_call(env, b, args);
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
            ExprKind::Path { segments } => self.resolve_func(env, segments).ok_or_else(|| {
                RuntimeError::Message(format!("unknown function `{}`", path_key(segments)))
            })?,
            _ => return crate::error::err("invalid function call"),
        };
        self.apply_function(&func, arg_values)
    }

    /// `derivative`/`partial`/`grad`/`limit` (spec §19.4): lower the argument expressions to the symbolic
    /// DAG, resolve the variable symbol, and delegate to `crate::diff`.
    fn eval_calc_call(&mut self, env: &EnvRef, b: Builtin, args: &[Expr]) -> Result<Value, RuntimeError> {
        match b {
            Builtin::Derivative | Builtin::Partial => {
                if args.len() != 2 {
                    return crate::error::err("`derivative`/`partial` expect (expr, var)");
                }
                let expr = self.lower_symbolic(env, &args[0])?;
                let x = self.eval_var_symbol(env, &args[1])?;
                let d = crate::diff::derivative(self.pool, self.builtins, expr, x);
                Ok(self.value_from_expr(simplify(self.pool, self.builtins, d)))
            }
            Builtin::Grad => {
                if args.len() != 1 {
                    return crate::error::err("`grad` expects (expr)");
                }
                let expr = self.lower_symbolic(env, &args[0])?;
                let grads = crate::diff::grad(self.pool, self.builtins, expr);
                let vals: Vec<Value> = grads
                    .into_iter()
                    .map(|g| self.value_from_expr(simplify(self.pool, self.builtins, g)))
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

    /// Lower an argument to a symbolic `ExprId` (spec §19.4): a single-segment path resolving to an MFn
    /// (`Function::User`) lowers the function body with each parameter bound to its symbol; anything else
    /// is evaluated normally and collapsed to the DAG.
    fn lower_symbolic(&mut self, env: &EnvRef, e: &Expr) -> Result<ExprId, RuntimeError> {
        if let ExprKind::Path { segments } = &e.kind
            && segments.len() == 1
            && let Some(Function::User { params, body, env: f_env, .. }) = self.resolve_func(env, segments)
        {
            let call_env = Env::child(&f_env);
            for p in params.iter() {
                let sym = self.pool.symbol(self.symbols.intern(&p.name.value));
                call_env.borrow_mut().set_value(&p.name.value, Value::Expr(sym));
            }
            let v = self.eval_expr(&call_env, &body)?;
            return self.to_expr_id(&v);
        }
        let v = self.eval_expr(env, e)?;
        self.to_expr_id(&v)
    }

    /// Evaluate a variable argument to a `SymbolId` (spec §19.4): accepts a symbolic expression
    /// (`Value::Expr`/`Value::Symbol`) or a `String` naming the variable.
    fn eval_var_symbol(&mut self, env: &EnvRef, e: &Expr) -> Result<SymbolId, RuntimeError> {
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

    /// Static purity check for a `parfor` effect call (spec §17.2, E0082): the top-level must be a call
    /// to a pure builtin (`Builtin::is_pure`) or an MFn (`Function::User`); host `fn` and `print` are rejected.
    fn expr_is_pure_call(&self, env: &EnvRef, e: &Expr) -> bool {
        match &e.kind {
            ExprKind::Call { callee, .. } => match &callee.kind {
                ExprKind::Path { segments } if segments.len() == 1 => match env.borrow().get_func(&segments[0].value) {
                    Some(Function::Builtin(b)) => b.is_pure(),
                    Some(Function::User { .. }) => true,
                    _ => false,
                },
                _ => false,
            },
            _ => false,
        }
    }

    /// Resolve a class name: `T` (local registry) or `mod::T` (module export, spec §15.2).
    fn resolve_class(&self, env: &EnvRef, segments: &[Spanned<String>]) -> Option<ClassDef> {
        if segments.is_empty() {
            return None;
        }
        let name = &segments[segments.len() - 1].value;
        if segments.len() == 1 {
            self.class_defs.get(name).cloned()
        } else {
            let ns = path_key(&segments[..segments.len() - 1]);
            match env.borrow().lookup_module_item(&ns, name) {
                Some(NamespaceItem::Class(def)) => Some(def),
                _ => self.class_defs.get(name).cloned(),
            }
        }
    }

    /// Call an associated function `T::name(args)` (spec §4.5): a method with no `self` parameter.
    fn call_associated(&mut self, def: &ClassDef, method_name: &str, args: Vec<Value>) -> Result<Value, RuntimeError> {
        let method = def.methods.get(method_name).cloned().ok_or_else(|| {
            RuntimeError::Message(format!("unknown associated function `{}::{}`", def.name, method_name))
        })?;
        if method.params.iter().any(|p| p.is_self) {
            return crate::error::err(format!("`{}::{}` is a method; call it on an instance", def.name, method_name));
        }
        if method.body.is_none() {
            return crate::error::err(format!("`{}::{}` is an unregistered `@builtin` method", def.name, method_name));
        }
        let body = method.body.as_ref().expect("body checked above");
        if args.len() != method.params.len() {
            return crate::error::err(format!(
                "`{}::{}` expects {} arguments, got {}",
                def.name,
                method_name,
                method.params.len(),
                args.len()
            ));
        }
        let call_env = Env::child(&method.env);
        for (p, a) in method.params.iter().zip(args) {
            call_env.borrow_mut().set_value(&p.name.value, a);
        }
        self.eval_block_tail(&call_env, body)
    }

    /// Evaluate a method call `obj.method(args)` (spec §4.5), including the builtin `String` methods (spec §18.1).
    fn eval_method_call(&mut self, env: &EnvRef, receiver: &Expr, name: &Spanned<String>, args: &[Expr]) -> Result<Value, RuntimeError> {
        let rcv = self.eval_expr(env, receiver)?;
        let mut arg_values = Vec::with_capacity(args.len());
        for a in args {
            arg_values.push(self.eval_expr(env, a)?);
        }
        match rcv {
            Value::Class(id) => {
                let inst = self.instances.get(&id).cloned().ok_or_else(|| {
                    RuntimeError::Message("unknown class instance".into())
                })?;
                let def = self.class_defs.get(&inst.class).cloned().ok_or_else(|| {
                    RuntimeError::Message(format!("unknown class `{}`", inst.class))
                })?;
                let method = def.methods.get(&name.value).cloned().ok_or_else(|| {
                    RuntimeError::Message(format!("unknown method `{}` on `{}`", name.value, def.name))
                })?;
                if method.vis == Visibility::Private && !self.in_method_of(&def.name) {
                    return crate::error::err(format!("private method `{}` cannot be called", name.value));
                }
                if method.vis == Visibility::Module && self.current_module != def.module {
                    return crate::error::err(format!(
                        "method `{}` of `{}` is `pub(mod)` and not accessible from this module",
                        name.value, def.name
                    ));
                }
                if !method.params.first().map(|p| p.is_self).unwrap_or(false) {
                    return crate::error::err(format!(
                        "`{}` on `{}` is an associated function; call it as `{}::{}(...)`",
                        name.value, def.name, def.name, name.value
                    ));
                }
                self.call_method(&method, Value::Class(id), arg_values)
            }
            Value::String(s) => self.call_string_method(&s, &name.value, arg_values),
            Value::Number(_) => {
                // Numeric method syntax (spec §9): `x.to_f64()`, `x.rounded(3)` etc. dispatch to the collapse family.
                let collapse_name = numeric_method_name(&name.value);
                let mut cargs = Vec::with_capacity(arg_values.len() + 1);
                cargs.push(rcv.clone());
                cargs.extend(arg_values);
                crate::collapse::call(&collapse_name, &cargs, self.pool, self.builtins)
            }
            Value::Array(a) if name.value == "get" => {
                if arg_values.len() != 1 {
                    return crate::error::err("`get` expects 1 argument");
                }
                self.call_array_get(Value::Array(a), arg_values[0].clone())
            }
            Value::Array(_) => crate::error::err("arrays have no method (use `get` for safe element access)"),
            Value::Option(_) | Value::Result(_)
                if matches!(name.value.as_str(), "unwrap" | "unwrap_or" | "expect") =>
            {
                // `v.get(10).unwrap_or(0)` method syntax on `Option`/`Result` (spec §16.3) → the collapse builtin.
                let mut cargs = Vec::with_capacity(arg_values.len() + 1);
                cargs.push(rcv.clone());
                cargs.extend(arg_values);
                crate::collapse::call(&name.value, &cargs, self.pool, self.builtins)
            }
            other => crate::error::err(format!("cannot call method `{}` on {}", name.value, value_type_name(&other))),
        }
    }

    /// Call a method: `self` is bound to the receiver (a shallow copy — same instance handle, spec §12.3).
    fn call_method(&mut self, method: &MethodDef, receiver: Value, args: Vec<Value>) -> Result<Value, RuntimeError> {
        let body = method
            .body
            .as_ref()
            .ok_or_else(|| RuntimeError::Message("unregistered `@builtin` method".into()))?;
        let self_param = method
            .params
            .first()
            .filter(|p| p.is_self)
            .ok_or_else(|| RuntimeError::Message("method requires a `self` receiver".into()))?;
        let expected = method.params.len() - 1;
        if args.len() != expected {
            return crate::error::err(format!("method expects {} arguments, got {}", expected, args.len()));
        }
        let instance_id = match &receiver {
            Value::Class(id) => Some(*id),
            _ => None,
        };
        if let Some(id) = instance_id {
            self.self_stack.push(id);
        }
        let call_env = Env::child(&method.env);
        {
            let mut e = call_env.borrow_mut();
            e.set_value(&self_param.name.value, receiver.clone());
            for (p, a) in method.params.iter().skip(1).zip(args) {
                e.set_value(&p.name.value, a);
            }
        }
        let result = self.eval_block_tail(&call_env, body);
        if instance_id.is_some() {
            self.self_stack.pop();
        }
        result
    }

    /// Whether the currently executing method belongs to `class` (used for private-field access, spec §15.2).
    fn in_method_of(&self, class: &str) -> bool {
        self.self_stack
            .last()
            .and_then(|id| self.instances.get(id))
            .map(|i| i.class == class)
            .unwrap_or(false)
    }

    /// Whether a field is accessible from the current context (spec §15.2): private fields are readable
    /// only inside methods of the same class; `pub(mod)` fields are readable only inside the defining module.
    fn field_accessible(&self, def: &ClassDef, field: &FieldDef) -> bool {
        match field.vis {
            Visibility::Public => true,
            Visibility::Module => self.current_module == def.module,
            Visibility::Private => self.in_method_of(&def.name),
        }
    }

    /// Field access `obj.field` (spec §4.5): private fields are readable only inside methods of the same class.
    fn eval_field(&mut self, env: &EnvRef, receiver: &Expr, name: &Spanned<String>) -> Result<Value, RuntimeError> {
        let rcv = self.eval_expr(env, receiver)?;
        match rcv {
            Value::Class(id) => {
                let inst = self.instances.get(&id).cloned().ok_or_else(|| {
                    RuntimeError::Message("unknown class instance".into())
                })?;
                let def = self.class_defs.get(&inst.class).cloned().ok_or_else(|| {
                    RuntimeError::Message(format!("unknown class `{}`", inst.class))
                })?;
                let field = def.fields.get(&name.value).cloned().ok_or_else(|| {
                    RuntimeError::Message(format!("`{}` has no field `{}`", def.name, name.value))
                })?;
                if !self.field_accessible(&def, &field) {
                    return crate::error::err(format!("field `{}` of `{}` is not accessible here", name.value, def.name));
                }
                inst.fields.get(&name.value).cloned().ok_or_else(|| {
                    RuntimeError::Message(format!("field `{}` is uninitialized", name.value))
                })
            }
            other => crate::error::err(format!("field access requires a class instance, got {}", value_type_name(&other))),
        }
    }

    /// Struct literal `T { a, b, ..base }` (spec §4.5): unknown field `E0060`, missing field `E0061`.
    fn eval_struct_literal(
        &mut self,
        env: &EnvRef,
        name: &Spanned<String>,
        fields: &[FieldValue],
        base: Option<&Expr>,
    ) -> Result<Value, RuntimeError> {
        let def = self.class_defs.get(&name.value).cloned().ok_or_else(|| {
            RuntimeError::Message(format!("unknown class `{}`", name.value))
        })?;
        let mut provided: HashSet<String> = HashSet::new();
        let mut out_fields: HashMap<String, Value> = HashMap::new();
        for fv in fields {
            if !def.fields.contains_key(&fv.name.value) {
                return crate::error::err(format!("unknown field `{}` in `{}` literal", fv.name.value, name.value));
            }
            let v = match &fv.value {
                Some(e) => self.eval_expr(env, e)?,
                None => env.borrow().get_value(&fv.name.value).ok_or_else(|| {
                    RuntimeError::Message(format!("no value `{}` in scope for field shorthand", fv.name.value))
                })?,
            };
            provided.insert(fv.name.value.clone());
            out_fields.insert(fv.name.value.clone(), v);
        }
        if let Some(b) = base {
            match self.eval_expr(env, b)? {
                Value::Class(bid) => {
                    let binst = self.instances.get(&bid).cloned().ok_or_else(|| {
                        RuntimeError::Message("unknown class instance".into())
                    })?;
                    if binst.class != def.name {
                        return crate::error::err(format!("`{}` literal base must be a `{}` instance", name.value, def.name));
                    }
                    for (k, v) in binst.fields {
                        if !provided.contains(&k) {
                            out_fields.insert(k, v);
                        }
                    }
                }
                _ => return crate::error::err("struct literal base must be a class instance"),
            }
        }
        for f in def.fields.keys() {
            if !out_fields.contains_key(f) {
                return crate::error::err(format!("missing field `{f}` in `{}` literal", name.value));
            }
        }
        let id = self.next_instance_id;
        self.next_instance_id += 1;
        self.instances.insert(id, ClassInstance { class: def.name, fields: out_fields });
        Ok(Value::Class(id))
    }

    /// Build a `ClassDef` from a class statement (spec §4.5): fields and methods.
    fn build_class_def(&mut self, name: &Spanned<String>, members: &[prima_syntax::ast::ClassMember], env: &EnvRef) -> ClassDef {
        let mut def = ClassDef {
            name: name.value.clone(),
            module: self.current_module.clone(),
            fields: HashMap::new(),
            methods: HashMap::new(),
        };
        for m in members {
            match &m.kind {
                ClassMemberKind::Field { name: fname, ty } => {
                    def.fields.insert(fname.value.clone(), FieldDef { ty: ty.clone(), vis: m.vis });
                }
                ClassMemberKind::Method { name: mname, params, ret, body, .. } => {
                    def.methods.insert(
                        mname.value.clone(),
                        MethodDef {
                            params: params.clone(),
                            ret: ret.clone(),
                            body: body.clone(),
                            vis: m.vis,
                            env: Rc::clone(env),
                        },
                    );
                }
            }
        }
        def
    }

    /// Register a class in the registry (spec §4.7). A later definition with the same name wins.
    fn register_class(&mut self, def: ClassDef) {
        self.class_defs.insert(def.name.clone(), def);
    }

    /// Builtin `String` methods (spec §18.1): all string methods operate on a value-semantic copy.
    fn call_string_method(&mut self, s: &str, name: &str, args: Vec<Value>) -> Result<Value, RuntimeError> {
        let expect_arity = |n: usize| -> Result<(), RuntimeError> {
            if args.len() == n {
                Ok(())
            } else {
                crate::error::err(format!("`String.{name}` expects {n} argument(s), got {}", args.len()))
            }
        };
        let str_arg = |i: usize| -> Result<String, RuntimeError> {
            match args.get(i) {
                Some(Value::String(s)) => Ok(s.clone()),
                Some(other) => crate::error::err(format!("`String.{name}` argument {i} must be a string, got {}", value_type_name(other))),
                None => crate::error::err(format!("`String.{name}` missing argument {i}")),
            }
        };
        let int_arg = |i: usize| -> Result<i64, RuntimeError> {
            match args.get(i) {
                Some(Value::Number(n)) => n
                    .as_i64()
                    .ok_or_else(|| RuntimeError::Type(format!("`String.{name}` argument {i} must be an integer, got {n}"))),
                Some(_) => crate::error::err(format!("`String.{name}` argument {i} must be an integer")),
                None => crate::error::err(format!("`String.{name}` missing argument {i}")),
            }
        };
        match name {
            "len" => {
                expect_arity(0)?;
                Ok(Value::Number(Number::from(s.chars().count() as i64)))
            }
            "is_empty" => {
                expect_arity(0)?;
                Ok(Value::Bool(s.is_empty()))
            }
            "push" => {
                expect_arity(1)?;
                Ok(Value::String(format!("{s}{}", str_arg(0)?)))
            }
            "insert" => {
                expect_arity(2)?;
                let idx = int_arg(0)?;
                let sub = str_arg(1)?;
                let len = s.chars().count() as i64;
                if idx < 0 || idx > len {
                    return Ok(Value::Result(Err(format!("insert index {idx} out of range (length {len})"))));
                }
                let idx = idx as usize;
                let mut out: String = s.chars().take(idx).collect();
                out.push_str(&sub);
                out.extend(s.chars().skip(idx));
                Ok(Value::Result(Ok(Box::new(Value::String(out)))))
            }
            "char_at" => {
                expect_arity(1)?;
                let idx = int_arg(0)?;
                if idx < 0 {
                    return Ok(Value::Option(None));
                }
                match s.chars().nth(idx as usize) {
                    Some(c) => Ok(Value::Option(Some(Box::new(Value::Char(c))))),
                    None => Ok(Value::Option(None)),
                }
            }
            "substring" => {
                expect_arity(2)?;
                let a = int_arg(0)?;
                let b = int_arg(1)?;
                if a < 0 {
                    return crate::error::err("`String.substring` start must be non-negative");
                }
                let (a, b) = (a as usize, b as usize);
                let chars: Vec<char> = s.chars().collect();
                if a > chars.len() || b > chars.len() || a > b {
                    return crate::error::err(format!("invalid substring range {a}..{b} (length {})", chars.len()));
                }
                Ok(Value::String(chars[a..b].iter().collect()))
            }
            "contains" => {
                expect_arity(1)?;
                Ok(Value::Bool(s.contains(&str_arg(0)?)))
            }
            "starts_with" => {
                expect_arity(1)?;
                Ok(Value::Bool(s.starts_with(&str_arg(0)?)))
            }
            "ends_with" => {
                expect_arity(1)?;
                Ok(Value::Bool(s.ends_with(&str_arg(0)?)))
            }
            "replace" => {
                expect_arity(2)?;
                Ok(Value::String(s.replace(&str_arg(0)?, &str_arg(1)?)))
            }
            "trim" => {
                expect_arity(0)?;
                Ok(Value::String(s.trim().to_string()))
            }
            "split" => {
                expect_arity(1)?;
                let sep = str_arg(0)?;
                let parts: Vec<Value> = s.split(&sep).map(|p| Value::String(p.to_string())).collect();
                Ok(Value::Tuple(parts))
            }
            "to_upper" => {
                expect_arity(0)?;
                Ok(Value::String(s.to_uppercase()))
            }
            "to_lower" => {
                expect_arity(0)?;
                Ok(Value::String(s.to_lowercase()))
            }
            "repeat" => {
                expect_arity(1)?;
                let n = int_arg(0)?;
                if n < 0 {
                    return crate::error::err("`String.repeat` count must be non-negative");
                }
                Ok(Value::String(s.repeat(n as usize)))
            }
            _ => crate::error::err(format!("unknown `String` method `{name}`")),
        }
    }

    /// `String::new()` / `String::from(x)` associated functions (spec §18.1).
    fn try_string_associated(&mut self, env: &EnvRef, segments: &[Spanned<String>], args: &[Expr]) -> Result<Option<Value>, RuntimeError> {
        if segments.len() != 2 || segments[0].value != "String" {
            return Ok(None);
        }
        match segments[1].value.as_str() {
            "new" => {
                if !args.is_empty() {
                    return crate::error::err("`String::new` takes no arguments");
                }
                Ok(Some(Value::String(String::new())))
            }
            "from" => {
                if args.len() != 1 {
                    return crate::error::err("`String::from` expects 1 argument");
                }
                let v = self.eval_expr(env, &args[0])?;
                Ok(Some(Value::String(self.format_value(&v))))
            }
            _ => Ok(None),
        }
    }

    /// `get(array, index) -> Option<Number>`: safe array access (spec §11.3).
    fn call_array_get(&mut self, arr: Value, idx: Value) -> Result<Value, RuntimeError> {
        let Value::Array(a) = arr else {
            return crate::error::err("`get` expects an array");
        };
        let i = match idx {
            Value::Number(n) => n.as_i64(),
            _ => None,
        };
        let Some(i) = i else {
            return crate::error::err("`get` expects an integer index");
        };
        if i < 0 {
            return Ok(Value::Option(None));
        }
        match a.get(i as usize) {
            Some(n) => Ok(Value::Option(Some(Box::new(Value::Number(n.clone()))))),
            None => Ok(Value::Option(None)),
        }
    }

    fn eval_index(&mut self, env: &EnvRef, base: &Expr, index: &prima_syntax::ast::Index) -> Result<Value, RuntimeError> {
        let arr_v = self.eval_expr(env, base)?;
        // Operator overload (spec §18.5): `Index` on a class instance.
        if let Value::Class(id) = &arr_v {
            let class = self.instances.get(id).map(|i| i.class.clone());
            if let Some(class) = class {
                if self.overloads.contains_key(&overload_key(&class, ImplOp::Index)) {
                    if index.items.len() != 1 {
                        return crate::error::err("multi-dimensional indexing is not supported yet");
                    }
                    let idx_v = match &index.items[0] {
                        IndexItem::Elem(e) => self.eval_expr(env, e)?,
                        IndexItem::Slice { .. } => return crate::error::err("slice indexing is not supported for overloads"),
                    };
                    return self.overload_dispatch(&class, ImplOp::Index, arr_v, vec![idx_v]);
                }
                return crate::error::err("cannot index a class instance");
            }
        }
        let arr = match arr_v {
            Value::Array(a) => a,
            _ => return crate::error::err("indexing requires an array"),
        };
        if index.items.len() != 1 {
            return crate::error::err("multi-dimensional indexing is not supported yet");
        }
        match &index.items[0] {
            IndexItem::Elem(e) => {
                let idx = self.eval_index_number(env, e)?;
                arr.get(idx)
                    .cloned()
                    .map(Value::Number)
                    .ok_or_else(|| RuntimeError::IndexOutOfBounds(format!("index {idx} (length {})", arr.len())))
            }
            IndexItem::Slice { start, end } => {
                let lo = match start {
                    Some(e) => self.eval_index_number(env, e)?,
                    None => 0,
                };
                let hi = match end {
                    Some(e) => self.eval_index_number(env, e)?,
                    None => arr.len(),
                };
                let lo = lo.min(arr.len());
                let hi = hi.min(arr.len());
                if lo > hi {
                    return crate::error::err("invalid slice range");
                }
                Ok(Value::Array(arr[lo..hi].to_vec()))
            }
        }
    }

    fn eval_index_number(&mut self, env: &EnvRef, e: &Expr) -> Result<usize, RuntimeError> {
        match self.eval_expr(env, e)? {
            Value::Number(n) => n
                .as_i64()
                .ok_or_else(|| RuntimeError::Message("array index must be an integer".into()))
                .and_then(|i| {
                    usize::try_from(i).map_err(|_| RuntimeError::Message("array index out of range".into()))
                }),
            _ => crate::error::err("array index must be an integer"),
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
                let func = Function::User { params: params.clone(), body: (**body).clone(), env: Rc::clone(env), parallel: false };
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
            Function::NativeGet => {
                if args.len() != 2 {
                    return crate::error::err("`get` expects 2 arguments");
                }
                self.call_array_get(args[0].clone(), args[1].clone())
            }
            Function::User { params, body, env: f_env, .. } => {
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
                self.eval_block_tail(&call_env, body)
            }
        }
    }

    /// Broadcast (spec §11.4): pure functions are applied elementwise to array arguments; **empty arrays are rejected** (`R0014`),
    /// non-numeric elements/scalars error (`R0009`). `@parallel` MFn (spec §17.1) over large arrays are split across rayon threads.
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
        if let Function::User { params, body, parallel: true, .. } = func
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

    /// Parallel broadcast of a `@parallel` MFn (spec §17.1/17.4): each rayon thread block runs an
    /// independent `Evaluator` over a chunk of the array. The body must be self-contained — parameters
    /// are bound in a fresh root environment, so no captured (non-`Send`) closure environment is shared.
    fn broadcast_parallel(
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
                            cargs.push(Value::Number(a[i].clone()));
                        } else {
                            return Err(RuntimeError::Message("cannot broadcast a non-numeric scalar".into()));
                        }
                    } else {
                        match v {
                            Value::Number(_) => cargs.push(v.clone()),
                            _ => return Err(RuntimeError::Message("cannot broadcast a non-numeric scalar".into())),
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
                    _ => Err(RuntimeError::Message("broadcast result must be numeric".into())),
                }
            })
            .collect();
        let mut out = Vec::with_capacity(len);
        for r in results {
            out.push(r?);
        }
        Ok(Value::Array(out))
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

    /// `match`/`if let`/`while let` arm evaluation (spec §4.4/§16.3): first matching pattern (with optional guard) wins.
    fn eval_match(&mut self, env: &EnvRef, scrutinee: &Expr, arms: &[MatchArm]) -> Result<Value, RuntimeError> {
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
    fn match_pattern(&mut self, env: &EnvRef, v: &Value, p: &Pattern) -> Option<Vec<(String, Value)>> {
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
                for (pat, n) in pats.iter().zip(elems) {
                    out.extend(self.match_pattern(env, &Value::Number(n.clone()), pat)?);
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
            Pattern::Variant { name, args, .. } => {
                match name.value.as_str() {
                    "Some" => match v {
                        Value::Option(Some(inner)) if args.len() == 1 => self.match_pattern(env, inner, &args[0]),
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
                        Value::Result(Ok(inner)) if args.len() == 1 => self.match_pattern(env, inner, &args[0]),
                        _ => None,
                    },
                    "Err" => match v {
                        Value::Result(Err(msg)) if args.len() == 1 => {
                            self.match_pattern(env, &Value::String(msg.clone()), &args[0])
                        }
                        _ => None,
                    },
                    _ => None,
                }
            }
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
                let ge = self.number_cmp(vn, &lo_n).is_some_and(|o| o != Ordering::Less);
                let hi_ok = if *inclusive {
                    self.number_cmp(vn, &hi_n).is_some_and(|o| o != Ordering::Greater)
                } else {
                    self.number_cmp(vn, &hi_n).is_some_and(|o| o == Ordering::Less)
                };
                if ge && hi_ok {
                    Some(vec![])
                } else {
                    None
                }
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
    fn number_cmp(&self, a: &Number, b: &Number) -> Option<Ordering> {
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
    fn pattern_values_equal(&self, a: &Value, b: &Value) -> bool {
        match (a, b) {
            (Value::Number(x), Value::Number(y)) => self.number_cmp(x, y) == Some(Ordering::Equal),
            (Value::String(x), Value::String(y)) => x == y,
            (Value::Char(x), Value::Char(y)) => x == y,
            (Value::Bool(x), Value::Bool(y)) => x == y,
            _ => a == b,
        }
    }

    fn call_builtin(&mut self, b: Builtin, args: Vec<Value>) -> Result<Value, RuntimeError> {
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
                crate::error::err("`derivative`/`partial`/`grad`/`limit` must be called directly with a variable")
            }
            Builtin::Simplify => {
                let arg = args.first().ok_or_else(|| RuntimeError::Message("simplify expects one argument".into()))?;
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
                let start_i = start.as_i64().ok_or_else(|| RuntimeError::Type(format!("range bounds must be integers, got {start}")))?;
                let end_i = end.as_i64().ok_or_else(|| RuntimeError::Type(format!("range bounds must be integers, got {end}")))?;
                let step_i = step.as_i64().ok_or_else(|| RuntimeError::Type(format!("range step must be an integer, got {step}")))?;
                if step_i == 0 {
                    return crate::error::err("`range` step cannot be zero");
                }
                let mut out = Vec::new();
                let mut i = start_i;
                while if step_i > 0 { i < end_i } else { i > end_i } {
                    out.push(Number::from(i));
                    i += step_i;
                }
                Ok(Value::Array(out))
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

    /// `input(prompt?)` / `read_line()` (spec §18.1b): optional prompt written without a trailing newline,
    /// then one line read from stdin (trailing `\r\n`/`\n` stripped). EOF or I/O errors return "".
    fn call_input(&mut self, b: Builtin, args: Vec<Value>) -> Result<Value, RuntimeError> {
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

/// Source span of a statement, used to locate errors (spec §16.4).
fn stmt_span(stmt: &Stmt) -> prima_syntax::Span {
    match stmt {
        Stmt::Let { span, .. }
        | Stmt::Const { span, .. }
        | Stmt::FnDef { span, .. }
        | Stmt::MathDef { span, .. }
        | Stmt::ClassDef { span, .. }
        | Stmt::Impl { span, .. }
        | Stmt::Assign { span, .. }
        | Stmt::For { span, .. }
        | Stmt::ParFor { span, .. }
        | Stmt::While { span, .. }
        | Stmt::If { span, .. }
        | Stmt::IfLet { span, .. }
        | Stmt::WhileLet { span, .. }
        | Stmt::Match { span, .. }
        | Stmt::Return { span, .. }
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
    overload_policy: OverloadPolicy::Warn,
};

/// Minimum array length for which a `@parallel` MFn broadcast is split across rayon threads (spec §17.1);
/// smaller arrays keep the sequential path to avoid thread-spawn overhead.
const PARALLEL_BROADCAST_THRESHOLD: usize = 1024;

fn path_key(segments: &[Spanned<String>]) -> String {
    segments.iter().map(|s| s.value.as_str()).collect::<Vec<_>>().join("::")
}

/// Collect every single-segment variable path referenced in `e` (spec §17.2 read set): used to bind
/// read-only outer values into `parfor` task environments without touching the non-`Send` env chain.
fn collect_read_names(e: &Expr, out: &mut HashSet<String>) {
    match &e.kind {
        ExprKind::Path { segments } if segments.len() == 1 => {
            out.insert(segments[0].value.clone());
        }
        ExprKind::Call { callee, args } => {
            collect_read_names(callee, out);
            for a in args {
                collect_read_names(a, out);
            }
        }
        ExprKind::MethodCall { receiver, args, .. } => {
            collect_read_names(receiver, out);
            for a in args {
                collect_read_names(a, out);
            }
        }
        ExprKind::Field { receiver, .. } => collect_read_names(receiver, out),
        ExprKind::StructLiteral { fields, base, .. } => {
            if let Some(b) = base {
                collect_read_names(b, out);
            }
            for f in fields {
                if let Some(v) = &f.value {
                    collect_read_names(v, out);
                }
            }
        }
        ExprKind::Index { base, index } => {
            collect_read_names(base, out);
            for it in &index.items {
                match it {
                    IndexItem::Elem(e) => collect_read_names(e, out),
                    IndexItem::Slice { start, end } => {
                        if let Some(s) = start {
                            collect_read_names(s, out);
                        }
                        if let Some(s) = end {
                            collect_read_names(s, out);
                        }
                    }
                }
            }
        }
        ExprKind::Binary { lhs, rhs, .. } | ExprKind::Pipeline { lhs, rhs } => {
            collect_read_names(lhs, out);
            collect_read_names(rhs, out);
        }
        ExprKind::Unary { operand, .. } | ExprKind::Try(operand) => collect_read_names(operand, out),
        ExprKind::Array(items) | ExprKind::Tuple(items) => {
            for it in items {
                collect_read_names(it, out);
            }
        }
        ExprKind::Lambda { body, .. } => collect_read_names(body, out),
        ExprKind::Match { scrutinee, arms } => {
            collect_read_names(scrutinee, out);
            for a in arms {
                if let Some(g) = &a.guard {
                    collect_read_names(g, out);
                }
                collect_read_names(&a.body, out);
            }
        }
        _ => {}
    }
}

/// One unit of work inside a `parfor` iteration body (spec §17.2): either an index-slot assignment
/// (`A[i] = …`/`A[i] += …`) or a pure function call evaluated for effect.
#[derive(Clone)]
enum ParforStep {
    Assign(ParforWrite),
    Eval(Expr),
}

#[derive(Clone)]
struct ParforWrite {
    array: String,
    index: Expr,
    op: AssignOp,
    value: Expr,
}

/// One index write produced by a `parfor` iteration: (array name, index, merged value).
type ParforWriteVec = Vec<(String, usize, Number)>;

/// Static side-effect check for a `parfor` body (spec §17.2, `E0082`): only index-slot assignments
/// (`A[i] = …`/`A[i] += …`/`A[i] -= …`) and pure function calls are allowed; anything else (external
/// variable assignment, `let`, `print`, class mutation, …) is an error.
fn check_parfor_body(body: &Block) -> Result<Vec<ParforStep>, RuntimeError> {
    let mut steps = Vec::new();
    for stmt in &body.stmts {
        match stmt {
            Stmt::Assign { target, op, value, .. } => {
                if let ExprKind::Index { base, index } = &target.kind
                    && let ExprKind::Path { segments } = &base.kind
                    && segments.len() == 1
                    && index.items.len() == 1
                    && let IndexItem::Elem(idx) = &index.items[0]
                {
                    steps.push(ParforStep::Assign(ParforWrite {
                        array: segments[0].value.clone(),
                        index: idx.clone(),
                        op: *op,
                        value: value.clone(),
                    }));
                } else {
                    return crate::error::err(
                        "parfor iteration body may only assign to index slots `A[i]` (E0082)",
                    );
                }
            }
            Stmt::Expr(e) => steps.push(ParforStep::Eval(e.clone())),
            _ => {
                return crate::error::err(
                    "parfor iteration body must be side-effect free (E0082): only index-slot assignments and pure calls allowed",
                );
            }
        }
    }
    Ok(steps)
}

/// Operator-overload registry key: `"<class>::<Op>"` (spec §18.5; `ImplOp` has no `Hash`).
fn overload_key(class: &str, op: ImplOp) -> String {
    let name = match op {
        ImplOp::Add => "Add",
        ImplOp::Sub => "Sub",
        ImplOp::Mul => "Mul",
        ImplOp::Div => "Div",
        ImplOp::Rem => "Rem",
        ImplOp::Neg => "Neg",
        ImplOp::Eq => "Eq",
        ImplOp::Cmp => "Cmp",
        ImplOp::Index => "Index",
    };
    format!("{class}::{name}")
}

/// Whether a `let` pattern can fail to match (spec §4.4): only bindings/wildcards/grouped-tuples of
/// irrefutable patterns are irrefutable; anything else requires `if let`/`match` (`E0053`).
fn pattern_is_refutable(p: &Pattern) -> bool {
    match p {
        Pattern::Wildcard(_) | Pattern::Binding(_) => false,
        Pattern::Tuple(pats, _) | Pattern::Array(pats, _) => pats.iter().any(pattern_is_refutable),
        // Struct patterns with `..` or binding-only fields never fail; a field with an explicit refutable sub-pattern does.
        Pattern::Struct { fields, .. } => {
            fields.iter().any(|f| match &f.pat {
                None => false,
                Some(sub) => pattern_is_refutable(sub),
            })
        }
        Pattern::Group(inner) => pattern_is_refutable(inner),
        Pattern::Or(pats) => pats.iter().any(pattern_is_refutable),
        _ => true,
    }
}

/// Short display name of a value's type, for error messages.
fn value_type_name(v: &Value) -> String {
    match v {
        Value::Nil => "nil".into(),
        Value::Number(_) => "number".into(),
        Value::Bool(_) => "bool".into(),
        Value::Char(_) => "char".into(),
        Value::String(_) => "string".into(),
        Value::Array(_) => "array".into(),
        Value::Expr(_) => "expr".into(),
        Value::Symbol(_) => "symbol".into(),
        Value::Class(_) => "class".into(),
        Value::Option(_) => "option".into(),
        Value::Result(_) => "result".into(),
        Value::Tuple(_) => "tuple".into(),
        Value::Indeterminate(_) => "indeterminate".into(),
        Value::Undefined => "undefined".into(),
        Value::Error(_) => "error".into(),
    }
}

/// Map numeric method names to their collapse-family builtin (spec §9): `x.to_f64()`, `x.rounded(3)`, `x.truncated()`, `x.abs()`.
fn numeric_method_name(name: &str) -> String {
    match name {
        "to_f64" => "to_f64".into(),
        "to_f32" => "to_f32".into(),
        "to_i8" => "to_i8".into(),
        "to_i16" => "to_i16".into(),
        "to_i32" => "to_i32".into(),
        "to_i64" => "to_i64".into(),
        "to_i128" => "to_i128".into(),
        "to_u8" => "to_u8".into(),
        "to_u16" => "to_u16".into(),
        "to_u32" => "to_u32".into(),
        "to_u64" => "to_u64".into(),
        "to_u128" => "to_u128".into(),
        "to_isize" => "to_isize".into(),
        "to_usize" => "to_usize".into(),
        "to_bigint" => "to_bigint".into(),
        "to_rational" => "to_rational".into(),
        "rounded" => "rounded_f64".into(),
        "truncated" => "truncated_i32".into(),
        _ => name.into(),
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

#[cfg(test)]
mod tests {
    use super::*;
    use prima_core::Real;

    fn eval(src: &str) -> Value {
        Evaluator::new().eval_value(src).expect("eval failed")
    }

    fn eval_fmt(src: &str) -> String {
        let mut ev = Evaluator::new();
        let v = ev.eval_value(src).expect("eval failed");
        ev.format_value(&v)
    }

    #[test]
    fn tuple_destructuring_in_let() {
        assert_eq!(eval("let t = (1, 2);\nlet (a, b) = t;\na + b"), Value::Number(Number::from(3)));
    }

    #[test]
    fn array_get_returns_option() {
        assert_eq!(
            eval("let v = [1, 2, 3];\nv.get(1)"),
            Value::Option(Some(Box::new(Value::Number(Number::from(2)))))
        );
        assert_eq!(eval("let v = [1, 2, 3];\nv.get(10)"), Value::Option(None));
        assert_eq!(
            eval("let v = [1, 2, 3];\nget(v, 0)"),
            Value::Option(Some(Box::new(Value::Number(Number::from(1)))))
        );
    }

    #[test]
    fn if_let_and_match() {
        assert_eq!(
            eval("let v = [1, 2];\nlet r = 0;\nif let Some(x) = v.get(0) {\n    r = x * 10;\n}\nr"),
            Value::Number(Number::from(10))
        );
        let r = eval("let r = try_i32(1e20);\nlet label = match r {\n    Ok(n) => \"ok\",\n    Err(e) => \"fail\"\n};\nlabel");
        assert_eq!(r, Value::String("fail".into()));
    }

    #[test]
    fn while_let_loops_over_get() {
        let v = eval(
            "let arr = [1, 2, 3];\nlet sum = 0;\nlet i = 0;\nwhile let Some(x) = get(arr, i) {\n    sum += x;\n    i += 1;\n}\nsum",
        );
        assert_eq!(v, Value::Number(Number::from(6)));
    }

    #[test]
    fn match_guard_and_range_patterns() {
        assert_eq!(
            eval("let r = match 5 {\n    0 => \"zero\",\n    1 | 2 => \"small\",\n    3..=9 => \"medium\",\n    n if n > 100 => \"large\",\n    _ => \"other\"\n};\nr"),
            Value::String("medium".into())
        );
    }

    #[test]
    fn match_is_non_exhaustive() {
        assert!(Evaluator::new().eval_value("match 1 {\n    2 => 0\n}").is_err());
    }

    #[test]
    fn try_operator_unwraps_ok() {
        let v = eval("fn f(x) -> Result<Integer, Error> {\n    let v = try_i32(x)?;\n    return Ok(v);\n}\nf(7)");
        assert_eq!(v, Value::Result(Ok(Box::new(Value::Number(Number::I32(7))))));
    }

    #[test]
    fn try_operator_propagates_none() {
        let err = Evaluator::new().eval_value("fn g() -> Option<Integer> {\n    let x = get([1], 5)?;\n    return Some(x);\n}\ng()").unwrap_err();
        assert!(err.to_string().contains("None"), "unexpected error: {err}");
    }

    #[test]
    fn class_associated_function_and_method() {
        assert_eq!(
            eval_fmt("class Vec2 {\n    x: F64, y: F64,\n    pub fn new(x, y) -> Self { Vec2 { x, y } }\n    pub fn sum(self) -> F64 { self.x + self.y }\n}\nlet v = Vec2::new(1, 2);\nv.sum()"),
            "3"
        );
    }

    #[test]
    fn struct_literal_base_spreads_fields() {
        assert_eq!(
            eval_fmt("class P { pub x: Integer, pub y: Integer }\nlet a = P { x: 1, y: 2 };\nlet b = P { x: 9, ..a };\nb.y"),
            "2"
        );
    }

    #[test]
    fn private_fields_are_not_accessible_from_outside() {
        let src = "class C {\n    secret: Integer,\n    pub fn new(s) -> Self { C { secret: s } }\n}\nlet c = C::new(1);\nc.secret";
        assert!(Evaluator::new().eval_value(src).is_err());
    }

    #[test]
    fn string_methods() {
        assert_eq!(eval("let s = \"hello\";\ns.len()"), Value::Number(Number::from(5)));
        assert_eq!(eval("let s = \"aXb\";\ns.to_lower()"), Value::String("axb".into()));
        assert_eq!(eval("let s = \"ab\";\ns.push(\"c\")"), Value::String("abc".into()));
        assert_eq!(
            eval("let s = \"hello world\";\ns.contains(\"world\")"),
            Value::Bool(true)
        );
        assert_eq!(eval("let s = \"a,b,c\";\ns.split(\",\")"), Value::Tuple(vec![
            Value::String("a".into()),
            Value::String("b".into()),
            Value::String("c".into()),
        ]));
        assert_eq!(
            eval("let s = \"hi\";\ns.insert(1, \"o\")"),
            Value::Result(Ok(Box::new(Value::String("hoi".into()))))
        );
        assert!(matches!(eval("let s = \"hi\";\ns.insert(9, \"o\")"), Value::Result(Err(_))));
        assert_eq!(eval("String::new()"), Value::String(String::new()));
    }

    #[test]
    fn operator_overload_dispatches_with_warning() {
        let mut ev = Evaluator::new();
        let v = ev
            .eval_value(
                "class Vec2 { x: F64, y: F64 }\nimpl ops::Add for Vec2 {\n    fn add(self, rhs) -> Vec2 { Vec2 { x: self.x + rhs.x, y: self.y + rhs.y } }\n}\nlet a = Vec2 { x: 1, y: 2 };\nlet b = Vec2 { x: 3, y: 4 };\na + b",
            )
            .expect("eval failed");
        assert_eq!(ev.format_value(&v), "class Vec2");
        assert!(ev.warnings().iter().any(|w| w.code == "W0005"));
    }

    #[test]
    fn overload_policy_deny_errors() {
        let mut ev = Evaluator::new();
        assert!(ev
            .eval_value(
                "class V { x: F64 }\nimpl ops::Add for V {\n    fn add(self, rhs) -> V { V { x: self.x } }\n}\nwith config { overload_policy := deny } {\n    let a = V { x: 1 };\n    a + a\n}"
            )
            .is_err());
    }

    #[test]
    fn pipeline_emits_w0002() {
        let mut ev = Evaluator::new();
        ev.eval_value("let x = 1;\nx |> to_f64").expect("eval failed");
        assert!(ev.warnings().iter().any(|w| w.code == "W0002"));
    }

    #[test]
    fn parse_warnings_are_surfaced() {
        let mut ev = Evaluator::new();
        ev.eval_value("let x = 1\nx + 1\n").expect("eval failed");
        assert!(ev.warnings().iter().any(|w| w.code == "W0001"));
    }

    #[test]
    fn fixed_width_numbers_render() {
        assert_eq!(eval_fmt("to_i8(7)"), "7");
        assert_eq!(eval_fmt("to_u64(42)"), "42");
        assert_eq!(eval_fmt("to_usize(3)"), "3");
        let v = eval("to_f64(3)");
        assert!(matches!(v, Value::Number(Number::Real(Real::F64(x))) if x == 3.0));
    }

    #[test]
    fn array_element_assignment_writes_through() {
        assert_eq!(eval("let a = [1, 2, 3];\na[1] = 9;\na"), Value::Array(vec![
            Number::from(1),
            Number::from(9),
            Number::from(3),
        ]));
    }

    #[test]
    fn array_slice_returns_subarray() {
        assert_eq!(eval("let a = [1, 2, 3, 4];\na[1..3]"), Value::Array(vec![
            Number::from(2),
            Number::from(3),
        ]));
    }
}
