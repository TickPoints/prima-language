use std::cell::RefCell;
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use std::sync::{Arc, OnceLock};

use num_bigint::BigInt;
use prima_core::render::{render_latex, render_number};
use prima_core::simplify::{simplify, simplify_at};
use prima_core::{
    BuiltinSymbols, ExprData, ExprId, ExprPool, Number, Real, SymbolId, SymbolTable, Value,
    ValueKey,
};
use prima_syntax::ast::{
    Annotation, AssignOp, BinOp, Block, ClassMemberKind, CompKind, ComprehensionClause,
    ConfigBlock, DocComment, Expr, ExprKind, FStringPart, FieldValue, ImplOp, ImportItem,
    ImportKind, IndexItem, Literal, MatchArm, Param, Pattern, Program, Spanned, Stmt, Type, UnOp,
    Visibility,
};
use prima_syntax::error::SyntaxError;
use prima_syntax::{Span, SyntaxWarning};
use rayon::prelude::*;

use crate::builtins::Builtin;
use crate::class::{ClassDef, ClassInstance, FieldDef, MethodDef, MethodNature};
use crate::config::{Config, Domain, OptLevel, OverloadPolicy, UndefinedHandling};
use crate::error::RuntimeError;
use crate::module::{ModuleGraph, ModuleUnit, ResolvedImport};

/// A Rust-hosted standard-library function registered by `prima-stdlib` (spec §18): called with the
/// evaluator (for access to pool/symbols/output) and the already-evaluated arguments.
pub type NativeCall = fn(&mut Evaluator, &[Value]) -> Result<Value, RuntimeError>;

/// Which runtime backend executes a builtin-class method (spec §18.1): read-only collection and
/// `Char`/`Tuple` backends, receiver write-back via `mutate_*` (spec §11.3/§11.6), or the collapse
/// family (`collapse::call`, spec §9) for `Number`/`Option`/`Result`.
#[derive(Debug, Clone, Copy)]
enum BuiltinBackend {
    Array,
    Dict,
    Set,
    MutateArray,
    MutateDict,
    MutateSet,
    Char,
    Tuple,
    Collapse,
    CollapseNumber,
}

/// Default call count before an MFn body is JIT-compiled (spec §19.2); `@jit` functions skip the countdown.
pub const JIT_CALL_THRESHOLD: u64 = 100;

/// Per-MFn hot-path state (spec §19.2): a monotonic call counter and the compiled artifact, guarded by a
/// `OnceLock` so the body is compiled at most once per `Function::User` instance. Compilation failure is
/// cached as `None` so a non-numeric body is never retried.
pub struct HotState {
    /// `@jit` annotation: compile on the first numeric call regardless of count.
    pub force: bool,
    calls: AtomicU64,
    compiled: OnceLock<Option<Arc<prima_jit::CompiledScalar>>>,
}

impl HotState {
    pub fn new(force: bool) -> HotState {
        HotState {
            force,
            calls: AtomicU64::new(0),
            compiled: OnceLock::new(),
        }
    }
}

/// Function value (spec §11): builtins, pure math functions (MFn/closures), host functions (`fn`),
/// and a small set of native host functions (`get`, plus the Rust-hosted stdlib `Native`). Closures
/// carry their defining environment. `parallel` (v2.1, spec §17.1) marks `@parallel` MFn: their bodies
/// must be self-contained (parameters + builtin symbols only), so a broadcast call can be split across rayon threads.
#[derive(Clone)]
pub enum Function {
    Builtin(Builtin),
    User {
        params: Vec<Param>,
        body: Expr,
        env: EnvRef,
        parallel: bool,
        /// Hot-path JIT state (spec §19.2): `@jit` forces compilation; otherwise it compiles after
        /// `JIT_CALL_THRESHOLD` numeric calls. Shared by every cloned copy of the function.
        hot: Arc<HotState>,
    },
    Host {
        params: Vec<Param>,
        ret: Option<Type>,
        body: Block,
        env: EnvRef,
    },
    /// `get(array, index) -> Option<Number>`: safe array access returning `None` out of range (spec §11.3).
    NativeGet,
    /// A Rust-hosted stdlib function (spec §18); see [`NativeCall`].
    Native {
        name: &'static str,
        call: NativeCall,
    },
    /// A `@builtin(ON)` layered function (spec §18.4): carries a `.pra` fallback body plus an
    /// optional Rust implementation. The native path is used when `opt_level >= level` and the Rust
    /// implementation is registered; otherwise the `.pra` body is evaluated. The `.pra` body is the
    /// sole observable-semantics source (spec §18.4).
    Layered {
        params: Vec<Param>,
        ret: Option<Type>,
        body: Block,
        env: EnvRef,
        native: Option<NativeCall>,
        level: u8,
    },
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
            // Native stdlib functions (matrix/vector ops, sys/time/num) never participate in implicit broadcast.
            Function::Native { .. } => false,
            // A layered function may be a host body with side effects; keep it out of the elementwise path.
            Function::Layered { .. } => false,
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
            "jit",
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
            // `format` was removed in v2.2 (spec §18.1): it is not pre-imported and not a builtin.
            "to_string",
            "concat",
            "len",
            "enumerate",
            "zip",
            "sorted",
            "reversed",
            "sum",
            "prod",
            "min",
            "max",
            "all",
            "any",
            "join",
            "count",
            "index",
            "first",
            "last",
            "linspace",
            "map",
            "filter",
            "reduce",
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
        self.parent
            .as_ref()
            .and_then(|p| p.borrow().get_value(name))
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
        self.parent
            .as_ref()
            .and_then(|p| p.borrow().lookup_module_item(ns_key, item))
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
    /// Collected warnings (spec §16.5): only the operator-overload `W0005` is emitted by the evaluator.
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
    /// Stack of the `self` receiver *values* of builtin-class methods currently executing (spec
    /// §18.1): `Value::String`/`Array`/... receivers are not class instances, so their `.pra`
    /// bodies resolve `self` through this stack instead of `self_stack`.
    self_values: Vec<Value>,
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
            self_values: Vec::new(),
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
            self_values: Vec::new(),
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

    /// Warnings collected since the last parse entry point (spec §16.5): parse-time warnings no
    /// longer exist; only the operator-overload `W0005` is emitted during evaluation.
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

    /// Simplify a symbolic `ExprId` at the depth requested by the active `simplify_level` policy
    /// (spec §8.3/§13.2). Semantics (not just polish) never change: lowering the level only reduces
    /// which rules fire, never the mathematical value.
    fn simplify_current(&self, id: ExprId) -> ExprId {
        simplify_at(
            self.pool,
            self.builtins,
            id,
            self.current_config().simplify_level,
        )
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
        self.warnings.push(SyntaxWarning {
            span,
            code,
            message,
        });
    }

    /// Fully-qualified registry key for an `@builtin` declared in the module currently being
    /// evaluated (spec §18.4): `"<module>::<name>"` in a stdlib module, plain `<name>` at the root.
    fn builtin_key(&self, name: &str) -> String {
        if self.current_module.is_empty() {
            name.to_string()
        } else {
            format!("{}::{name}", self.current_module)
        }
    }

    /// Bind an `@builtin` function declaration to its implementation (spec §18.4).
    /// At the root a core builtin of the same name wins; inside a module the registered stdlib
    /// implementation (`"<module>::<name>"`) takes precedence — core builtins must NOT shadow a
    /// module's own `@builtin` (e.g. `sys::path::join` must not become the core `join`). Otherwise E0055.
    fn bind_builtin(&self, name: &str) -> Result<Function, RuntimeError> {
        if self.current_module.is_empty()
            && let Some(b) = Builtin::from_name(name)
        {
            return Ok(Function::Builtin(b));
        }
        let key = self.builtin_key(name);
        if let Some(call) = crate::stdlib::get_impl(&key) {
            let leaked: &'static str = Box::leak(key.into_boxed_str());
            return Ok(Function::Native { name: leaked, call });
        }
        crate::error::err(format!("unregistered `@builtin` function `{name}` (E0055)"))
    }

    /// Bind a `@builtin(ON)` fn declaration (spec §18.4) to its implementation:
    /// - tier `O0` (bare `@builtin`): signature-only — a body is an error (`E0056`) and the impl must be
    ///   registered (`E0055`, via [`Self::bind_builtin`]);
    /// - tier `O1..=O3` (layered): requires a `.pra` fallback body (`E0056` if absent); the Rust impl is
    ///   optional and is used at call time when `opt_level >= level` (spec §18.4).
    fn bind_builtin_annotated(
        &self,
        name: &str,
        level: u8,
        params: &[Param],
        ret: &Option<Type>,
        body: &Block,
        env: &EnvRef,
    ) -> Result<Function, RuntimeError> {
        if level == 0 {
            if !body.stmts.is_empty() {
                return crate::error::err(format!(
                    "`@builtin` function `{name}` must not have a body (E0056)"
                ));
            }
            return self.bind_builtin(name);
        }
        if body.stmts.is_empty() {
            return crate::error::err(format!(
                "`@builtin(O{level})` function `{name}` must have a body (E0056)"
            ));
        }
        let native = crate::stdlib::get_impl(&self.builtin_key(name));
        Ok(Function::Layered {
            params: params.to_vec(),
            ret: ret.clone(),
            body: body.clone(),
            env: Rc::clone(env),
            native,
            level,
        })
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

    /// Like [`Evaluator::eval_file`] but retains the root environment, so a named function can be
    /// invoked afterwards (used by the C ABI export path, spec §18.4).
    pub fn eval_file_keep_env(&mut self, path: &Path) -> Result<EnvRef, RuntimeError> {
        let graph = ModuleGraph::load(path).map_err(RuntimeError::Message)?;
        self.reset_config();
        self.module_items.clear();
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
        result?;
        Ok(env)
    }

    /// Invoke a function bound in `env` by name with already-evaluated arguments (spec §15.1).
    /// Public so the C ABI export wrappers can call into a loaded module (spec §18.4).
    pub fn call_function(
        &mut self,
        env: &EnvRef,
        name: &str,
        args: Vec<Value>,
    ) -> Result<Value, RuntimeError> {
        let func = env
            .borrow()
            .get_func(name)
            .ok_or_else(|| RuntimeError::Message(format!("unknown function `{name}`")))?;
        self.apply_function(&func, args)
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

    fn eval_module_inner(
        &mut self,
        env: &EnvRef,
        unit: &ModuleUnit,
    ) -> Result<HashMap<String, NamespaceItem>, RuntimeError> {
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

    fn collect_pub(
        &mut self,
        env: &EnvRef,
        inner: &Stmt,
        items: &mut HashMap<String, NamespaceItem>,
    ) -> Result<(), RuntimeError> {
        match inner {
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
                env.borrow_mut().set_func(&name.value, f.clone());
                items.insert(name.value.clone(), NamespaceItem::Func(f));
                Ok(())
            }
            Stmt::FnDef {
                name,
                params,
                ret,
                annotations,
                body,
                ..
            } => {
                // `@builtin pub fn` (spec §18.4): the exported item binds to the core builtin or the
                // registered stdlib implementation (keyed `"<module>::<name>"`), keeping the typed
                // signature for later call-site checking. Path names like `Matrix::zeros` are exported
                // under the joined key so module-qualified calls resolve. A `@builtin(ON)` tier
                // produces a layered function (native fast path + `.pra` fallback).
                let f = if annotations.iter().any(|a| a.is_builtin()) {
                    let level = annotations
                        .iter()
                        .map(|a| a.builtin_level())
                        .max()
                        .unwrap_or(0);
                    self.bind_builtin_annotated(&name.value, level, params, ret, body, env)?
                } else {
                    Function::Host {
                        params: params.clone(),
                        ret: ret.clone(),
                        body: body.clone(),
                        env: Rc::clone(env),
                    }
                };
                env.borrow_mut().set_func(&name.value, f.clone());
                items.insert(name.value.clone(), NamespaceItem::Func(f));
                Ok(())
            }
            Stmt::Let {
                pat: Pattern::Binding(name),
                value,
                ..
            }
            | Stmt::Const { name, value, .. } => {
                let v = self.eval_expr(env, value)?;
                env.borrow_mut().set_value(&name.value, v.clone());
                items.insert(name.value.clone(), NamespaceItem::Val(v));
                Ok(())
            }
            Stmt::ClassDef {
                name,
                members,
                docs,
                ..
            } => {
                let def = self.build_class_def(name, members, docs.as_ref(), env)?;
                self.register_class(def.clone());
                items.insert(def.name.clone(), NamespaceItem::Class(def));
                Ok(())
            }
            _ => crate::error::err("`pub` only applies to `let`/`const`/`fn`/`class`"),
        }
    }

    /// Bind a module's public items into the current environment (spec §15.1/§15.4): namespaces, selective imports, and conflict detection.
    fn bind_imports(
        &mut self,
        env: &EnvRef,
        imports: &[ResolvedImport],
    ) -> Result<(), RuntimeError> {
        let mut bound: HashSet<String> = HashSet::new();
        for ri in imports {
            let key = ri.path.join("::");
            match &ri.kind {
                ImportKind::Namespace { alias, .. } => {
                    // File-loaded modules come from `module_items`; host modules resolve from the Rust
                    // stdlib registry (spec §18).
                    let items = self
                        .module_items
                        .get(&key)
                        .cloned()
                        .or_else(|| crate::stdlib::get_namespace(&key))
                        .ok_or_else(|| {
                            RuntimeError::Message(format!("module `{key}` is not loaded"))
                        })?;
                    for item in items.values() {
                        if let NamespaceItem::Class(def) = item {
                            self.register_class(def.clone());
                        }
                    }
                    let ns = alias
                        .as_ref()
                        .map(|a| a.value.clone())
                        .unwrap_or_else(|| key.clone());
                    if env.borrow_mut().set_module(&ns, items) {
                        return crate::error::err(format!("conflicting import: module `{ns}`"));
                    }
                }
                ImportKind::From {
                    items: from_items, ..
                } => {
                    let module = self
                        .module_items
                        .get(&key)
                        .cloned()
                        .or_else(|| crate::stdlib::get_namespace(&key))
                        .ok_or_else(|| {
                            RuntimeError::Message(format!("module `{key}` is not loaded"))
                        })?;
                    for it in from_items {
                        match it {
                            ImportItem::Star => {
                                for (name, item) in &module {
                                    if !bound.insert(name.clone()) {
                                        return crate::error::err(format!(
                                            "conflicting import: `{name}`"
                                        ));
                                    }
                                    self.bind_imported_item(env, name, item);
                                }
                            }
                            ImportItem::Name { name, alias } => {
                                let item = module.get(&name.value).cloned().ok_or_else(|| {
                                    RuntimeError::Message(format!(
                                        "module `{key}` has no public item `{}`",
                                        name.value
                                    ))
                                })?;
                                let target = alias
                                    .as_ref()
                                    .map(|a| a.value.clone())
                                    .unwrap_or_else(|| name.value.clone());
                                if !bound.insert(target.clone()) {
                                    return crate::error::err(format!(
                                        "conflicting import: `{target}`"
                                    ));
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
            // In-memory evaluation supports embedded stdlib modules (spec §18.4) and Rust-hosted
            // namespaces (spec §18); file modules require the module graph (`eval_file`).
            let env = Env::new().into_ref();
            self.bind_host_imports(&env, &program.imports)?;
            let r = self.eval_program_in(&env, program);
            self.reset_config();
            return r;
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
        let env = Env::new().into_ref();
        if !program.imports.is_empty() {
            // In-memory evaluation only supports Rust-hosted stdlib namespaces (spec §18): file modules
            // require the module graph, so they go through `eval_file` / `prima run`.
            self.bind_host_imports(&env, &program.imports)?;
        }
        let r = self.eval_value_in(&env, &program);
        self.reset_config();
        r
    }

    /// Bind in-memory imports that resolve to embedded stdlib modules (spec §18.4) or Rust-hosted
    /// stdlib namespaces (spec §18). Embedded modules are evaluated first (like `eval_module` for a
    /// dependency), populating `module_items` so `bind_imports` finds their `@builtin` items. Any
    /// other import (a file module) is rejected as before — file modules need `eval_file`.
    fn bind_host_imports(
        &mut self,
        env: &EnvRef,
        imports: &[prima_syntax::ast::Import],
    ) -> Result<(), RuntimeError> {
        let mut resolved = Vec::with_capacity(imports.len());
        for imp in imports {
            let segments = match &imp.kind {
                ImportKind::Namespace { path, .. } | ImportKind::From { path, .. } => path,
            };
            let key = segments
                .iter()
                .map(|s| s.value.as_str())
                .collect::<Vec<_>>()
                .join("::");
            let path: Vec<String> = segments.iter().map(|s| s.value.clone()).collect();
            if let Some(src) = crate::stdlib::get_module_source(&key) {
                if !self.module_items.contains_key(&key) {
                    let program = prima_syntax::parse(src).map_err(|errs| {
                        let details = errs
                            .iter()
                            .map(|e| e.to_string())
                            .collect::<Vec<_>>()
                            .join(", ");
                        RuntimeError::Message(format!(
                            "embedded stdlib module `{key}` failed to parse: {details}"
                        ))
                    })?;
                    let unit = ModuleUnit {
                        path: path.clone(),
                        file: crate::module::embedded_file(&path),
                        program,
                        imports: Vec::new(),
                    };
                    self.eval_module(&unit)?;
                }
                let file = crate::module::embedded_file(&path);
                resolved.push(ResolvedImport {
                    path,
                    file,
                    kind: imp.kind.clone(),
                    host: false,
                    embedded: true,
                });
            } else if crate::stdlib::has_namespace(&key) {
                resolved.push(ResolvedImport {
                    path,
                    file: PathBuf::new(),
                    kind: imp.kind.clone(),
                    host: true,
                    embedded: false,
                });
            } else {
                return crate::error::err(
                    "`import` requires running from a file (`prima run <file>`)",
                );
            }
        }
        self.bind_imports(env, &resolved)
    }

    fn eval_value_in(&mut self, env: &EnvRef, program: &Program) -> Result<Value, RuntimeError> {
        self.push_module_config(program.config.as_ref())?;
        let mut last = Value::Nil;
        for stmt in &program.stmts {
            if let Stmt::Expr(e) = stmt {
                last = self.eval_expr(env, e)?;
            } else if let Stmt::Match {
                scrutinee, arms, ..
            } = stmt
            {
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
                let inner: Vec<String> = elems.iter().map(|e| self.format_value(e)).collect();
                format!("[{}]", inner.join(", "))
            }
            Value::Dict(d) => {
                let keys = self.sorted_dict_keys(d);
                let inner: Vec<String> = keys
                    .iter()
                    .map(|k| {
                        format!(
                            "{}: {}",
                            self.format_value(&k.to_value()),
                            self.format_value(&d[k])
                        )
                    })
                    .collect();
                format!("{{{}}}", inner.join(", "))
            }
            Value::Set(s) => {
                let elems = self.sorted_set_values(s);
                let inner: Vec<String> = elems.iter().map(|e| self.format_value(e)).collect();
                format!("{{{}}}", inner.join(", "))
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
            Value::JitFunction(id) => format!("jit function {id}"),
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
    fn eval_stmt(&mut self, env: &EnvRef, stmt: &Stmt) -> Result<Flow, RuntimeError> {
        let span = stmt_span(stmt);
        self.eval_stmt_inner(env, stmt)
            .map_err(|e| crate::error::attach_span(e, span))
    }

    fn eval_stmt_inner(&mut self, env: &EnvRef, stmt: &Stmt) -> Result<Flow, RuntimeError> {
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
                // (spec §11.3/§11.6): writes back through the collection binding.
                if let ExprKind::Index { base, index } = &target.kind {
                    let (name, base_v) = self.eval_collection_lvalue(env, base)?;
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
                                RuntimeError::Message("dict key must be a hashable value".into())
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
                        Value::Array(mut arr) => {
                            if index.items.len() != 1 {
                                return crate::error::err(
                                    "multi-dimensional indexing is not supported yet",
                                );
                            }
                            match &index.items[0] {
                                IndexItem::Elem(e) => {
                                    let raw = self.eval_index_i64(env, e)?;
                                    let idx = normalize_index(raw, arr.len()).ok_or_else(|| {
                                        RuntimeError::IndexOutOfBounds(format!(
                                            "index {raw} (length {})",
                                            arr.len()
                                        ))
                                    })?;
                                    let merged = match op {
                                        AssignOp::Assign => v,
                                        AssignOp::AddAssign => {
                                            self.eval_binary(BinOp::Add, arr[idx].clone(), v)?
                                        }
                                        AssignOp::SubAssign => {
                                            self.eval_binary(BinOp::Sub, arr[idx].clone(), v)?
                                        }
                                    };
                                    arr[idx] = merged;
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
                                    let (lo, hi) = self.slice_bounds(
                                        env,
                                        start.as_ref(),
                                        end.as_ref(),
                                        arr.len(),
                                    )?;
                                    arr.splice(lo..hi, rhs);
                                }
                            }
                            self.write_back(env, &name, Value::Array(arr));
                            return Ok(Flow::Continue);
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
                    let scope = Env::child(env);
                    scope
                        .borrow_mut()
                        .set_value(&var.value, Value::Number(Number::from(i)));
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
            Stmt::ParFor {
                var,
                range,
                step,
                body,
                ..
            } => self.eval_parfor(env, var, range, step, body),
        }
    }

    /// Evaluate a collection lvalue `A` (a plain variable holding an array or dict), for `A[i] = v`
    /// / `d[k] = v` (spec §11.3/§11.6).
    fn eval_collection_lvalue(
        &mut self,
        env: &EnvRef,
        base: &Expr,
    ) -> Result<(String, Value), RuntimeError> {
        match &base.kind {
            ExprKind::Path { segments } if segments.len() == 1 => {
                let name = segments[0].value.clone();
                match self.eval_expr(env, base)? {
                    Value::Array(a) => Ok((name, Value::Array(a))),
                    Value::Dict(d) => Ok((name, Value::Dict(d))),
                    _ => crate::error::err("assignment target must be an array or dict"),
                }
            }
            _ => crate::error::err("assignment target must be a variable"),
        }
    }

    /// Write a collection value back to its binding along the shared chain (spec §12.2 shadowing),
    /// creating the binding locally if it is undefined.
    fn write_back(&mut self, env: &EnvRef, name: &str, v: Value) {
        let mut e = env.borrow_mut();
        if !e.set_existing(name, v.clone()) {
            e.set_value(name, v);
        }
    }

    /// Compute the clamped `[lo, hi)` slice bounds (spec §11.3): both bounds may be negative and are
    /// clamped to `[0, len]`; `lo > hi` is an error.
    fn slice_bounds(
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
        let mut arrays: HashMap<String, Vec<Value>> = HashMap::new();
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
        let n = if step_v > 0 {
            if start >= end {
                0
            } else {
                (end - start - 1) / step_v + 1
            }
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
                let mut out: Vec<(String, usize, Value)> = Vec::new();
                for &i in chunk {
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
    fn try_arithmetic_sum(
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
        let sum = if start == 0 && end > 0 {
            end * (end - 1) / 2
        } else if start == 1 && end >= 1 {
            end * (end + 1) / 2
        } else {
            return Ok(None);
        };
        let prev = env
            .borrow()
            .get_value(&acc)
            .unwrap_or(Value::Number(Number::from(0)));
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
        self.eval_expr_inner(env, expr)
            .map_err(|e| crate::error::attach_span(e, span))
    }

    fn eval_expr_inner(&mut self, env: &EnvRef, expr: &Expr) -> Result<Value, RuntimeError> {
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
    fn eval_broadcast_op(
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
    fn eval_comprehension(
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
    fn comprehension_clauses(
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
    fn iter_values(&self, v: &Value) -> Result<Vec<Value>, RuntimeError> {
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

    fn eval_literal(&mut self, env: &EnvRef, lit: &Literal) -> Result<Value, RuntimeError> {
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
    fn eval_fstring(&mut self, env: &EnvRef, parts: &[FStringPart]) -> Result<Value, RuntimeError> {
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
    fn ensure_defined(&self, v: &Value) -> Result<(), RuntimeError> {
        if matches!(v, Value::Undefined) {
            Err(RuntimeError::Undefined(
                "`Undefined` cannot participate in operations".into(),
            ))
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
    fn eval_in(&mut self, a: Value, b: Value) -> Result<Value, RuntimeError> {
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
    fn eval_set_algebra(&mut self, op: BinOp, a: Value, b: Value) -> Result<Value, RuntimeError> {
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
    fn try_overload_binary(
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
    fn overload_dispatch(
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

    fn eval_number_binary(
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
    fn custom_zero_div(&self) -> Option<Value> {
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

    fn eval_compare(&mut self, op: BinOp, a: Value, b: Value) -> Result<Value, RuntimeError> {
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

    fn eval_unary(&mut self, op: UnOp, v: Value) -> Result<Value, RuntimeError> {
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

    fn eval_call(
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
    fn eval_calc_call(
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
    fn eval_jit_call(&mut self, env: &EnvRef, args: &[Expr]) -> Result<Value, RuntimeError> {
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
    fn eval_higher_order(
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
    fn lower_symbolic(&mut self, env: &EnvRef, e: &Expr) -> Result<ExprId, RuntimeError> {
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
    fn body_dag(
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
    fn try_compile_body(
        &mut self,
        params: &[Param],
        body: &Expr,
        f_env: &EnvRef,
    ) -> Option<Arc<prima_jit::CompiledScalar>> {
        let (dag, names) = self.body_dag(params, body, f_env).ok()?;
        prima_jit::compile_scalar(self.pool, self.builtins, dag, &names)
    }

    /// Dispatch a `Value::JitFunction(id)` call (spec §19.2): the arguments are already evaluated.
    fn call_jit_function(&mut self, id: u32, args: Vec<Value>) -> Result<Value, RuntimeError> {
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
    fn lookup_module_item_flat(&self, env: &EnvRef, ns: &str, item: &str) -> Option<NamespaceItem> {
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
    fn expr_is_pure_call(&self, env: &EnvRef, e: &Expr) -> bool {
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
    fn call_associated(
        &mut self,
        def: &ClassDef,
        method_name: &str,
        args: Vec<Value>,
    ) -> Result<Value, RuntimeError> {
        let method = match def.methods.get(method_name) {
            Some(m) => m.clone(),
            None => {
                let err = RuntimeError::Message(format!(
                    "unknown associated function `{}::{}`",
                    def.name, method_name
                ));
                // Attach a `did you mean` help and a doc note from the nearest candidate (spec §16.4).
                let mut notes = Vec::new();
                let mut help = None;
                if let Some(cand) =
                    did_you_mean(method_name, def.methods.keys().map(|k| k.as_str()))
                {
                    if cand != method_name {
                        help = Some(format!("did you mean `{cand}`?"));
                    }
                    if let Some(m) = def.methods.get(&cand) {
                        notes.extend(method_note(&cand, m));
                    }
                }
                return Err(with_notes(err, notes, help));
            }
        };
        let note = |e: RuntimeError| with_notes(e, method_note(method_name, &method), None);
        if method.params.iter().any(|p| p.is_self) {
            return Err(note(RuntimeError::Message(format!(
                "`{}::{}` is a method; call it on an instance",
                def.name, method_name
            ))));
        }
        if method.body.is_none() {
            return Err(note(RuntimeError::Message(format!(
                "`{}::{}` is an unregistered `@builtin` method",
                def.name, method_name
            ))));
        }
        let body = method.body.as_ref().expect("body checked above");
        if args.len() != method.params.len() {
            return Err(note(RuntimeError::Message(format!(
                "`{}::{}` expects {} arguments, got {}",
                def.name,
                method_name,
                method.params.len(),
                args.len()
            ))));
        }
        let call_env = Env::child(&method.env);
        for (p, a) in method.params.iter().zip(args) {
            call_env.borrow_mut().set_value(&p.name.value, a);
        }
        self.eval_block_tail(&call_env, body).map_err(note)
    }

    /// Evaluate a method call `obj.method(args)` (spec §4.5), including the builtin `String` methods (spec §18.1).
    fn eval_method_call(
        &mut self,
        env: &EnvRef,
        receiver: &Expr,
        name: &Spanned<String>,
        args: &[Expr],
    ) -> Result<Value, RuntimeError> {
        let rcv = self.eval_expr(env, receiver)?;
        let mut arg_values = Vec::with_capacity(args.len());
        for a in args {
            arg_values.push(self.eval_expr(env, a)?);
        }
        match rcv {
            Value::Class(id) => {
                let inst = self
                    .instances
                    .get(&id)
                    .cloned()
                    .ok_or_else(|| RuntimeError::Message("unknown class instance".into()))?;
                let def = self.class_defs.get(&inst.class).cloned().ok_or_else(|| {
                    RuntimeError::Message(format!("unknown class `{}`", inst.class))
                })?;
                let method = match def.methods.get(&name.value) {
                    Some(m) => m.clone(),
                    None => {
                        // Unknown method: attach a `did you mean` help and a doc note from the
                        // nearest candidate method (spec §16.4).
                        let err = RuntimeError::Message(format!(
                            "unknown method `{}` on `{}`",
                            name.value, def.name
                        ));
                        let mut notes = Vec::new();
                        let mut help = None;
                        if let Some(cand) =
                            did_you_mean(&name.value, def.methods.keys().map(|k| k.as_str()))
                        {
                            if cand != name.value {
                                help = Some(format!("did you mean `{cand}`?"));
                            }
                            if let Some(m) = def.methods.get(&cand) {
                                notes.extend(method_note(&cand, m));
                            }
                        }
                        return Err(with_notes(err, notes, help));
                    }
                };
                if method.vis == Visibility::Private && !self.in_method_of(&def.name) {
                    return Err(with_notes(
                        RuntimeError::Message(format!(
                            "private method `{}` cannot be called",
                            name.value
                        )),
                        method_note(&name.value, &method),
                        None,
                    ));
                }
                if method.vis == Visibility::Module && self.current_module != def.module {
                    return Err(with_notes(
                        RuntimeError::Message(format!(
                            "method `{}` of `{}` is `pub(mod)` and not accessible from this module",
                            name.value, def.name
                        )),
                        method_note(&name.value, &method),
                        None,
                    ));
                }
                if !method.params.first().map(|p| p.is_self).unwrap_or(false) {
                    return Err(with_notes(
                        RuntimeError::Message(format!(
                            "`{}` on `{}` is an associated function; call it as `{}::{}(...)`",
                            name.value, def.name, def.name, name.value
                        )),
                        method_note(&name.value, &method),
                        None,
                    ));
                }
                self.call_method(&method, Value::Class(id), arg_values)
                    .map_err(|e| with_notes(e, method_note(&name.value, &method), None))
            }
            // Builtin classes (spec §18.1): the method is looked up in the embedded `core::<class>`
            // module and dispatched through its registered impl / `.pra` body / runtime backend.
            Value::String(_) => self.dispatch_builtin_method(
                env,
                receiver,
                "String",
                rcv,
                &name.value,
                arg_values,
                None,
            ),
            Value::Number(_) => self.dispatch_builtin_method(
                env,
                receiver,
                "Number",
                rcv,
                &name.value,
                arg_values,
                Some(BuiltinBackend::CollapseNumber),
            ),
            Value::Array(_) => {
                let backend = if is_mutating_array_method(&name.value) {
                    BuiltinBackend::MutateArray
                } else {
                    BuiltinBackend::Array
                };
                self.dispatch_builtin_method(
                    env,
                    receiver,
                    "Array",
                    rcv,
                    &name.value,
                    arg_values,
                    Some(backend),
                )
            }
            Value::Dict(_) => {
                let backend = if is_mutating_dict_method(&name.value) {
                    BuiltinBackend::MutateDict
                } else {
                    BuiltinBackend::Dict
                };
                self.dispatch_builtin_method(
                    env,
                    receiver,
                    "Dict",
                    rcv,
                    &name.value,
                    arg_values,
                    Some(backend),
                )
            }
            Value::Set(_) => {
                let backend = if is_mutating_set_method(&name.value) {
                    BuiltinBackend::MutateSet
                } else {
                    BuiltinBackend::Set
                };
                self.dispatch_builtin_method(
                    env,
                    receiver,
                    "Set",
                    rcv,
                    &name.value,
                    arg_values,
                    Some(backend),
                )
            }
            Value::Char(_) => self.dispatch_builtin_method(
                env,
                receiver,
                "Char",
                rcv,
                &name.value,
                arg_values,
                Some(BuiltinBackend::Char),
            ),
            Value::Tuple(_) => self.dispatch_builtin_method(
                env,
                receiver,
                "Tuple",
                rcv,
                &name.value,
                arg_values,
                Some(BuiltinBackend::Tuple),
            ),
            Value::Option(_) => self.dispatch_builtin_method(
                env,
                receiver,
                "Option",
                rcv,
                &name.value,
                arg_values,
                Some(BuiltinBackend::Collapse),
            ),
            Value::Result(_) => self.dispatch_builtin_method(
                env,
                receiver,
                "Result",
                rcv,
                &name.value,
                arg_values,
                Some(BuiltinBackend::Collapse),
            ),
            other => crate::error::err(format!(
                "cannot call method `{}` on {}",
                name.value,
                value_type_name(&other)
            )),
        }
    }

    /// Dispatch a builtin-class method call through its class definition (spec §18.1): a registered
    /// Rust fast path or `.pra` fallback body when the method declares one (layered `@builtin(ON)`,
    /// spec §18.4), otherwise the runtime backend. Errors are wrapped with the method's doc note
    /// (spec §16.4).
    #[allow(clippy::too_many_arguments)]
    fn dispatch_builtin_method(
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
            let err = RuntimeError::Message(format!("unknown `{class}` method `{name}`"));
            return Err(native_method_error(class, name, err));
        };
        // Registered impl and/or `.pra` body: `call_method` applies the `@builtin(ON)` layering.
        if method.body.is_some() || method.native.is_some() {
            return self
                .call_method(&method, rcv, args)
                .map_err(|e| native_method_error(class, name, e));
        }
        let backend = backend.ok_or_else(|| {
            RuntimeError::Message(format!("unregistered `{class}` method `{name}`"))
        })?;
        let result = match backend {
            BuiltinBackend::Array => {
                let Value::Array(a) = &rcv else {
                    return crate::error::err("expected an array receiver");
                };
                self.call_array_method(a, name, args)
            }
            BuiltinBackend::Dict => {
                let Value::Dict(d) = &rcv else {
                    return crate::error::err("expected a dict receiver");
                };
                self.call_dict_method(d, name, args)
            }
            BuiltinBackend::Set => {
                let Value::Set(s) = &rcv else {
                    return crate::error::err("expected a set receiver");
                };
                self.call_set_method(s, name, args)
            }
            BuiltinBackend::MutateArray => self.mutate_array(env, receiver, name, args),
            BuiltinBackend::MutateDict => self.mutate_dict(env, receiver, name, args),
            BuiltinBackend::MutateSet => self.mutate_set(env, receiver, name, args),
            BuiltinBackend::Char => {
                let Value::Char(c) = &rcv else {
                    return crate::error::err("expected a char receiver");
                };
                self.call_char_method(*c, name, args)
            }
            BuiltinBackend::Tuple => {
                let Value::Tuple(t) = &rcv else {
                    return crate::error::err("expected a tuple receiver");
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

    /// Call a method: `self` is bound to the receiver (a shallow copy — same instance handle, spec §12.3).
    fn call_method(
        &mut self,
        method: &MethodDef,
        receiver: Value,
        args: Vec<Value>,
    ) -> Result<Value, RuntimeError> {
        let self_param = method
            .params
            .first()
            .filter(|p| p.is_self)
            .ok_or_else(|| RuntimeError::Message("method requires a `self` receiver".into()))?;
        let expected = method.params.len() - 1;
        if args.len() != expected {
            return crate::error::err(format!(
                "method expects {expected} arguments, got {}",
                args.len()
            ));
        }
        // Layered `@builtin(ON)` method (spec §18.4): the registered Rust implementation is used when
        // the active `opt_level` is at least the declared tier; otherwise the `.pra` body is evaluated.
        if let Some(native) = method.native
            && (method.level == 0 || self.current_config().opt_level.tier() >= method.level)
        {
            let mut full = Vec::with_capacity(args.len() + 1);
            full.push(receiver);
            full.extend(args);
            return native(self, &full);
        }
        let body = method
            .body
            .as_ref()
            .ok_or_else(|| RuntimeError::Message("unregistered `@builtin` method".into()))?;
        let instance_id = match &receiver {
            Value::Class(id) => Some(*id),
            _ => None,
        };
        if let Some(id) = instance_id {
            self.self_stack.push(id);
        } else {
            // Builtin-class receiver (`Value::String`/`Array`/...): expose it as `self` in the body.
            self.self_values.push(receiver.clone());
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
        } else {
            self.self_values.pop();
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
    fn eval_field(
        &mut self,
        env: &EnvRef,
        receiver: &Expr,
        name: &Spanned<String>,
    ) -> Result<Value, RuntimeError> {
        let rcv = self.eval_expr(env, receiver)?;
        match rcv {
            Value::Class(id) => {
                let inst = self
                    .instances
                    .get(&id)
                    .cloned()
                    .ok_or_else(|| RuntimeError::Message("unknown class instance".into()))?;
                let def = self.class_defs.get(&inst.class).cloned().ok_or_else(|| {
                    RuntimeError::Message(format!("unknown class `{}`", inst.class))
                })?;
                let field = def.fields.get(&name.value).cloned().ok_or_else(|| {
                    RuntimeError::Message(format!("`{}` has no field `{}`", def.name, name.value))
                })?;
                if !self.field_accessible(&def, &field) {
                    return crate::error::err(format!(
                        "field `{}` of `{}` is not accessible here",
                        name.value, def.name
                    ));
                }
                inst.fields.get(&name.value).cloned().ok_or_else(|| {
                    RuntimeError::Message(format!("field `{}` is uninitialized", name.value))
                })
            }
            other => crate::error::err(format!(
                "field access requires a class instance, got {}",
                value_type_name(&other)
            )),
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
        let def = self
            .class_defs
            .get(&name.value)
            .cloned()
            .ok_or_else(|| RuntimeError::Message(format!("unknown class `{}`", name.value)))?;
        let mut provided: HashSet<String> = HashSet::new();
        let mut out_fields: HashMap<String, Value> = HashMap::new();
        for fv in fields {
            if !def.fields.contains_key(&fv.name.value) {
                return crate::error::err(format!(
                    "unknown field `{}` in `{}` literal",
                    fv.name.value, name.value
                ));
            }
            let v = match &fv.value {
                Some(e) => self.eval_expr(env, e)?,
                None => env.borrow().get_value(&fv.name.value).ok_or_else(|| {
                    RuntimeError::Message(format!(
                        "no value `{}` in scope for field shorthand",
                        fv.name.value
                    ))
                })?,
            };
            provided.insert(fv.name.value.clone());
            out_fields.insert(fv.name.value.clone(), v);
        }
        if let Some(b) = base {
            match self.eval_expr(env, b)? {
                Value::Class(bid) => {
                    let binst =
                        self.instances.get(&bid).cloned().ok_or_else(|| {
                            RuntimeError::Message("unknown class instance".into())
                        })?;
                    if binst.class != def.name {
                        return crate::error::err(format!(
                            "`{}` literal base must be a `{}` instance",
                            name.value, def.name
                        ));
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
                return crate::error::err(format!(
                    "missing field `{f}` in `{}` literal",
                    name.value
                ));
            }
        }
        let id = self.next_instance_id;
        self.next_instance_id += 1;
        self.instances.insert(
            id,
            ClassInstance {
                class: def.name,
                fields: out_fields,
            },
        );
        Ok(Value::Class(id))
    }

    /// Dispatch backend for a builtin-class method (spec §18.1): mutating methods write back through
    /// the receiver binding; `Number`/`Option`/`Result` go through the collapse family; everything
    /// else is a read-only backend. User classes are always `Plain`.
    fn method_nature(&self, class: &str, method: &str) -> MethodNature {
        match class {
            "Array" if is_mutating_array_method(method) => MethodNature::Mutating,
            "Dict" if is_mutating_dict_method(method) => MethodNature::Mutating,
            "Set" if is_mutating_set_method(method) => MethodNature::Mutating,
            "Array" | "Dict" | "Set" | "Char" | "Tuple" => MethodNature::Backend,
            "Number" | "Option" | "Result" => MethodNature::Collapse,
            _ => MethodNature::Plain,
        }
    }

    /// Build a `ClassDef` from a class statement (spec §4.5): fields and methods.
    ///
    /// A method's `@builtin(ON)` annotation (spec §18.4) drives validation: bare `@builtin` (O0) is
    /// signature-only and must be backed by a registered implementation or a runtime backend
    /// (`E0055`), with a body rejected as `E0056`; `@builtin(ON)` (`O1..O3`) requires a `.pra`
    /// fallback body (`E0056`) and an optional Rust fast path. Plain methods with neither a body nor
    /// an implementation are also `E0055`.
    fn build_class_def(
        &mut self,
        name: &Spanned<String>,
        members: &[prima_syntax::ast::ClassMember],
        docs: Option<&DocComment>,
        env: &EnvRef,
    ) -> Result<ClassDef, RuntimeError> {
        let mut def = ClassDef {
            name: name.value.clone(),
            module: self.current_module.clone(),
            fields: HashMap::new(),
            methods: HashMap::new(),
            docs: docs.cloned(),
        };
        for m in members {
            match &m.kind {
                ClassMemberKind::Field { name: fname, ty } => {
                    def.fields.insert(
                        fname.value.clone(),
                        FieldDef {
                            ty: ty.clone(),
                            vis: m.vis,
                        },
                    );
                }
                ClassMemberKind::Method {
                    name: mname,
                    params,
                    ret,
                    annotations,
                    body,
                } => {
                    let is_builtin = annotations.iter().any(|a| a.is_builtin());
                    let level = annotations
                        .iter()
                        .map(|a| a.builtin_level())
                        .max()
                        .unwrap_or(0);
                    let nature = self.method_nature(&def.name, &mname.value);
                    let native = if is_builtin {
                        crate::stdlib::get_impl(&format!("{}::{}", def.name, mname.value))
                    } else {
                        None
                    };
                    // Validation (spec §18.4): `E0055` unregistered `@builtin`, `E0056` wrong body.
                    if is_builtin && level == 0 && body.is_some() {
                        return crate::error::err(format!(
                            "`@builtin` method `{}` of `{}` must not have a body (E0056)",
                            mname.value, def.name
                        ));
                    }
                    if is_builtin && level > 0 && body.is_none() {
                        return crate::error::err(format!(
                            "`@builtin(O{level})` method `{}` of `{}` must have a `.pra` fallback body (E0056)",
                            mname.value, def.name
                        ));
                    }
                    if body.is_none() && native.is_none() && nature == MethodNature::Plain {
                        return crate::error::err(format!(
                            "unregistered `@builtin` method `{}` of `{}` (E0055)",
                            mname.value, def.name
                        ));
                    }
                    def.methods.insert(
                        mname.value.clone(),
                        MethodDef {
                            params: params.clone(),
                            ret: ret.clone(),
                            body: body.clone(),
                            native,
                            level,
                            nature,
                            vis: m.vis,
                            env: Rc::clone(env),
                            docs: m.docs.clone(),
                        },
                    );
                }
            }
        }
        Ok(def)
    }

    /// Register a class in the registry (spec §4.7). A later definition with the same name wins.
    fn register_class(&mut self, def: ClassDef) {
        self.class_defs.insert(def.name.clone(), def);
    }

    /// Lazily load a builtin class definition (`String`/`Array`/...) from its embedded `core::<class>`
    /// module (spec §18.1). The modules are registered by the standard library; without
    /// `prima_stdlib::init()` the class is unavailable and its methods error out.
    fn ensure_class(&mut self, class: &str) -> Result<(), RuntimeError> {
        if self.class_defs.contains_key(class) {
            return Ok(());
        }
        let module_path = format!("core::{}", class.to_ascii_lowercase());
        let src = crate::stdlib::get_module_source(&module_path).ok_or_else(|| {
            RuntimeError::Message(format!(
                "`{class}` methods are unavailable: the standard library is not initialized"
            ))
        })?;
        let program = prima_syntax::parse(src).map_err(|_| {
            RuntimeError::Message(format!(
                "internal error: embedded `{module_path}` module does not parse"
            ))
        })?;
        let env = Env::new().into_ref();
        let prev_module = std::mem::take(&mut self.current_module);
        self.current_module = module_path.clone();
        let result = (|| {
            for stmt in &program.stmts {
                if let Stmt::ClassDef {
                    name,
                    members,
                    docs,
                    ..
                } = stmt
                    && name.value == class
                {
                    let def = self.build_class_def(name, members, docs.as_ref(), &env)?;
                    self.register_class(def);
                }
            }
            if self.class_defs.contains_key(class) {
                Ok(())
            } else {
                crate::error::err(format!(
                    "internal error: `{module_path}` does not define `class {class}`"
                ))
            }
        })();
        self.current_module = prev_module;
        result
    }

    /// Look up a builtin-class method, loading the class definition first (spec §18.1). `None` for an
    /// unknown method; loading fails with a runtime error when the standard library is missing.
    fn builtin_method(
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
    fn call_char_method(
        &mut self,
        c: char,
        name: &str,
        args: Vec<Value>,
    ) -> Result<Value, RuntimeError> {
        if !args.is_empty() {
            return crate::error::err(format!(
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
            "to_string" => Ok(Value::String(c.to_string())),
            "code" => Ok(Value::Number(Number::from(u32::from(c) as i64))),
            _ => crate::error::err(format!("unknown `Char` method `{name}`")),
        }
    }

    /// Builtin `Tuple` methods (spec §18.1): `len`/`get`/`count`/`index`/`first`/`last`.
    fn call_tuple_method(
        &mut self,
        t: &[Value],
        name: &str,
        args: Vec<Value>,
    ) -> Result<Value, RuntimeError> {
        let arity = |n: usize| -> Result<(), RuntimeError> {
            if args.len() == n {
                Ok(())
            } else {
                crate::error::err(format!(
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
                    return crate::error::err("`Tuple.get` expects an integer index");
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
                    None => crate::error::err("element not found"),
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
            _ => crate::error::err(format!("unknown `Tuple` method `{name}`")),
        }
    }

    /// `String::new()` / `String::from(x)` associated functions (spec §18.1).
    fn try_string_associated(
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

    /// `get(array, index) -> Option<T>`: safe array access (spec §11.3); a negative index counts from
    /// the end, out-of-range yields `None`.
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
        match normalize_index(i, a.len()) {
            Some(i) => Ok(Value::Option(Some(Box::new(a[i].clone())))),
            None => Ok(Value::Option(None)),
        }
    }

    /// Read-only array methods (spec §11.3): operate on a value-semantic copy of the array.
    fn call_array_method(
        &mut self,
        a: &[Value],
        name: &str,
        args: Vec<Value>,
    ) -> Result<Value, RuntimeError> {
        let arity = |n: usize| -> Result<(), RuntimeError> {
            if args.len() == n {
                Ok(())
            } else {
                crate::error::err(format!(
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
                self.call_array_get(Value::Array(a.to_vec()), args[0].clone())
            }
            "contains" => {
                arity(1)?;
                Ok(Value::Bool(a.iter().any(|e| self.value_eq(e, &args[0]))))
            }
            "index" => {
                arity(1)?;
                match a.iter().position(|e| self.value_eq(e, &args[0])) {
                    Some(i) => Ok(Value::Number(Number::from(i as i64))),
                    None => crate::error::err("element not found"),
                }
            }
            "count" => {
                arity(1)?;
                Ok(Value::Number(Number::from(
                    a.iter().filter(|e| self.value_eq(e, &args[0])).count() as i64,
                )))
            }
            "first" => {
                arity(0)?;
                Ok(a.first()
                    .map(|v| Value::Option(Some(Box::new(v.clone()))))
                    .unwrap_or(Value::Option(None)))
            }
            "last" => {
                arity(0)?;
                Ok(a.last()
                    .map(|v| Value::Option(Some(Box::new(v.clone()))))
                    .unwrap_or(Value::Option(None)))
            }
            "copy" => {
                arity(0)?;
                Ok(Value::Array(a.to_vec()))
            }
            _ => crate::error::err(format!("unknown `Array` method `{name}`")),
        }
    }

    /// Mutating array methods (spec §11.3): the receiver must be a single-segment path (a variable
    /// binding); the mutated copy is written back to the binding.
    fn mutate_array(
        &mut self,
        env: &EnvRef,
        receiver: &Expr,
        name: &str,
        args: Vec<Value>,
    ) -> Result<Value, RuntimeError> {
        let var = match &receiver.kind {
            ExprKind::Path { segments } if segments.len() == 1 => segments[0].value.clone(),
            _ => return crate::error::err("cannot mutate a temporary value"),
        };
        let cur = env
            .borrow()
            .get_value(&var)
            .ok_or_else(|| RuntimeError::Message(format!("unknown variable `{var}`")))?;
        let Value::Array(mut arr) = cur else {
            return crate::error::err("expected an array binding");
        };
        let arity = |n: usize| -> Result<(), RuntimeError> {
            if args.len() == n {
                Ok(())
            } else {
                crate::error::err(format!(
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
                _ => crate::error::err(format!("`Array.{name}` index must be an integer")),
            }
        };
        let out = match name {
            "push" => {
                arity(1)?;
                arr.push(args[0].clone());
                Value::Nil
            }
            "pop" => {
                arity(0)?;
                arr.pop()
                    .map(|v| Value::Option(Some(Box::new(v))))
                    .unwrap_or(Value::Option(None))
            }
            "append" => {
                arity(1)?;
                arr.push(args[0].clone());
                Value::Nil
            }
            "extend" => {
                arity(1)?;
                match &args[0] {
                    Value::Array(elems) => arr.extend(elems.iter().cloned()),
                    other => {
                        return crate::error::err(format!(
                            "`Array.extend` expects an array, got {}",
                            value_type_name(other)
                        ));
                    }
                }
                Value::Nil
            }
            "insert" => {
                arity(2)?;
                let i = index(&args[0])?;
                let i = normalize_insert(i, arr.len()).ok_or_else(|| {
                    RuntimeError::IndexOutOfBounds(format!("index {i} (length {})", arr.len()))
                })?;
                arr.insert(i, args[1].clone());
                Value::Nil
            }
            "remove" => {
                arity(1)?;
                let i = index(&args[0])?;
                let i = normalize_index(i, arr.len()).ok_or_else(|| {
                    RuntimeError::IndexOutOfBounds(format!("index {i} (length {})", arr.len()))
                })?;
                arr.remove(i)
            }
            "clear" => {
                arity(0)?;
                arr.clear();
                Value::Nil
            }
            // `sort` orders numeric elements only (spec §11.3); `reverse` works on any elements.
            "sort" => {
                arity(0)?;
                let mut nums = Vec::with_capacity(arr.len());
                for e in &arr {
                    match e {
                        Value::Number(n) => nums.push(n.clone()),
                        _ => return crate::error::err("`Array.sort` requires an array of numbers"),
                    }
                }
                nums.sort_by(|x, y| self.number_cmp(x, y).unwrap_or(Ordering::Equal));
                arr = nums.into_iter().map(Value::Number).collect();
                Value::Nil
            }
            "reverse" => {
                arity(0)?;
                arr.reverse();
                Value::Nil
            }
            _ => return crate::error::err(format!("unknown `Array` method `{name}`")),
        };
        self.write_back(env, &var, Value::Array(arr));
        Ok(out)
    }

    /// Read-only dict methods (spec §11.6): `keys`/`values`/`items` return arrays in deterministic
    /// (canonical-key sorted) order.
    fn call_dict_method(
        &mut self,
        d: &HashMap<ValueKey, Value>,
        name: &str,
        args: Vec<Value>,
    ) -> Result<Value, RuntimeError> {
        let arity = |n: usize| -> Result<(), RuntimeError> {
            if args.len() == n {
                Ok(())
            } else {
                crate::error::err(format!(
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
                let key = ValueKey::from_value(&args[0]).ok_or_else(|| {
                    RuntimeError::Message("dict key must be a hashable value".into())
                })?;
                Ok(d.get(&key)
                    .map(|v| Value::Option(Some(Box::new(v.clone()))))
                    .unwrap_or(Value::Option(None)))
            }
            "contains" => {
                arity(1)?;
                let key = ValueKey::from_value(&args[0]).ok_or_else(|| {
                    RuntimeError::Message("dict key must be a hashable value".into())
                })?;
                Ok(Value::Bool(d.contains_key(&key)))
            }
            "keys" => {
                arity(0)?;
                Ok(Value::Array(
                    self.sorted_dict_keys(d)
                        .iter()
                        .map(|k| k.to_value())
                        .collect(),
                ))
            }
            "values" => {
                arity(0)?;
                Ok(Value::Array(
                    self.sorted_dict_keys(d)
                        .iter()
                        .map(|k| d[k].clone())
                        .collect(),
                ))
            }
            "items" => {
                arity(0)?;
                Ok(Value::Array(
                    self.sorted_dict_keys(d)
                        .iter()
                        .map(|k| Value::Tuple(vec![k.to_value(), d[k].clone()]))
                        .collect(),
                ))
            }
            _ => crate::error::err(format!("unknown `Dict` method `{name}`")),
        }
    }

    /// Mutating dict methods (spec §11.6): receiver must be a single-segment path; write-back pattern
    /// mirrors `mutate_array`.
    fn mutate_dict(
        &mut self,
        env: &EnvRef,
        receiver: &Expr,
        name: &str,
        args: Vec<Value>,
    ) -> Result<Value, RuntimeError> {
        let var = match &receiver.kind {
            ExprKind::Path { segments } if segments.len() == 1 => segments[0].value.clone(),
            _ => return crate::error::err("cannot mutate a temporary value"),
        };
        let cur = env
            .borrow()
            .get_value(&var)
            .ok_or_else(|| RuntimeError::Message(format!("unknown variable `{var}`")))?;
        let Value::Dict(mut d) = cur else {
            return crate::error::err("expected a dict binding");
        };
        let arity = |n: usize| -> Result<(), RuntimeError> {
            if args.len() == n {
                Ok(())
            } else {
                crate::error::err(format!(
                    "`Dict.{name}` expects {n} argument(s), got {}",
                    args.len()
                ))
            }
        };
        let key = |i: usize| -> Result<ValueKey, RuntimeError> {
            args.get(i)
                .and_then(ValueKey::from_value)
                .ok_or_else(|| RuntimeError::Message("dict key must be a hashable value".into()))
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
                    return crate::error::err("`Dict.update` expects a dict argument");
                };
                for (k, v) in other {
                    d.insert(k.clone(), v.clone());
                }
                // `d.update(other)` returns the merged dict (spec §11.6 example `let dd = d.update(…)`).
                Value::Dict(d.clone())
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
                let k = self
                    .sorted_dict_keys(&d)
                    .into_iter()
                    .next()
                    .ok_or_else(|| {
                        RuntimeError::Message("`Dict.popitem` on an empty dict".into())
                    })?;
                let v = d.remove(&k).unwrap();
                Value::Tuple(vec![k.to_value(), v])
            }
            _ => return crate::error::err(format!("unknown `Dict` method `{name}`")),
        };
        self.write_back(env, &var, Value::Dict(d));
        Ok(out)
    }

    /// Read-only set methods (spec §11.6): `union`/`intersection`/`difference` return new sets.
    fn call_set_method(
        &mut self,
        s: &HashSet<ValueKey>,
        name: &str,
        args: Vec<Value>,
    ) -> Result<Value, RuntimeError> {
        let arity = |n: usize| -> Result<(), RuntimeError> {
            if args.len() == n {
                Ok(())
            } else {
                crate::error::err(format!(
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
                let key = ValueKey::from_value(&args[0]).ok_or_else(|| {
                    RuntimeError::Message("set element must be a hashable value".into())
                })?;
                Ok(Value::Bool(s.contains(&key)))
            }
            "union" | "intersection" | "difference" | "symmetric_difference" => {
                arity(1)?;
                let Value::Set(other) = &args[0] else {
                    return crate::error::err("`Set.{name}` expects a set argument");
                };
                let out = match name {
                    "union" => s.union(other).cloned().collect(),
                    "intersection" => s.intersection(other).cloned().collect(),
                    "difference" => s.difference(other).cloned().collect(),
                    "symmetric_difference" => s.symmetric_difference(other).cloned().collect(),
                    _ => unreachable!(),
                };
                Ok(Value::Set(out))
            }
            "issubset" | "issuperset" | "isdisjoint" => {
                arity(1)?;
                let Value::Set(other) = &args[0] else {
                    return crate::error::err("`Set.{name}` expects a set argument");
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
                Ok(Value::Set(s.clone()))
            }
            _ => crate::error::err(format!("unknown `Set` method `{name}`")),
        }
    }

    /// Mutating set methods (spec §11.6): `remove` reports `R0013` on an absent element, `discard` is silent.
    fn mutate_set(
        &mut self,
        env: &EnvRef,
        receiver: &Expr,
        name: &str,
        args: Vec<Value>,
    ) -> Result<Value, RuntimeError> {
        let var = match &receiver.kind {
            ExprKind::Path { segments } if segments.len() == 1 => segments[0].value.clone(),
            _ => return crate::error::err("cannot mutate a temporary value"),
        };
        let cur = env
            .borrow()
            .get_value(&var)
            .ok_or_else(|| RuntimeError::Message(format!("unknown variable `{var}`")))?;
        let Value::Set(mut s) = cur else {
            return crate::error::err("expected a set binding");
        };
        let arity = |n: usize| -> Result<(), RuntimeError> {
            if args.len() == n {
                Ok(())
            } else {
                crate::error::err(format!(
                    "`Set.{name}` expects {n} argument(s), got {}",
                    args.len()
                ))
            }
        };
        let key = |i: usize| -> Result<ValueKey, RuntimeError> {
            args.get(i)
                .and_then(ValueKey::from_value)
                .ok_or_else(|| RuntimeError::Message("set element must be a hashable value".into()))
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
                    return crate::error::err("element not found");
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
                        for k in other {
                            s.insert(k.clone());
                        }
                    }
                    Value::Array(elems) => {
                        for e in elems {
                            let k = ValueKey::from_value(e).ok_or_else(|| {
                                RuntimeError::Message("set element must be a hashable value".into())
                            })?;
                            s.insert(k);
                        }
                    }
                    other => {
                        return crate::error::err(format!(
                            "`Set.update` expects a set or array, got {}",
                            value_type_name(other)
                        ));
                    }
                }
                Value::Nil
            }
            _ => return crate::error::err(format!("unknown `Set` method `{name}`")),
        };
        self.write_back(env, &var, Value::Set(s));
        Ok(out)
    }

    /// Deterministic key order for a dict (spec §11.6): sorted by the `format_value` of each key, with
    /// the key's debug rendering as a tiebreaker, so snapshots/tests are stable.
    fn sorted_dict_keys(&self, d: &HashMap<ValueKey, Value>) -> Vec<ValueKey> {
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
    fn sorted_set_values(&self, s: &HashSet<ValueKey>) -> Vec<Value> {
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
    fn dict_eq(&self, x: &HashMap<ValueKey, Value>, y: &HashMap<ValueKey, Value>) -> bool {
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

    fn eval_index(
        &mut self,
        env: &EnvRef,
        base: &Expr,
        index: &prima_syntax::ast::Index,
    ) -> Result<Value, RuntimeError> {
        let arr_v = self.eval_expr(env, base)?;
        // Operator overload (spec §18.5): `Index` on a class instance.
        if let Value::Class(id) = &arr_v {
            let class = self.instances.get(id).map(|i| i.class.clone());
            if let Some(class) = class {
                if self
                    .overloads
                    .contains_key(&overload_key(&class, ImplOp::Index))
                {
                    if index.items.len() != 1 {
                        return crate::error::err(
                            "multi-dimensional indexing is not supported yet",
                        );
                    }
                    let idx_v = match &index.items[0] {
                        IndexItem::Elem(e) => self.eval_expr(env, e)?,
                        IndexItem::Slice { .. } => {
                            return crate::error::err(
                                "slice indexing is not supported for overloads",
                            );
                        }
                    };
                    return self.overload_dispatch(&class, ImplOp::Index, arr_v, vec![idx_v]);
                }
                return crate::error::err("cannot index a class instance");
            }
        }
        match arr_v {
            // Array indexing with negative indices and clamped slices (spec §11.3).
            Value::Array(a) => {
                if index.items.len() != 1 {
                    return crate::error::err("multi-dimensional indexing is not supported yet");
                }
                match &index.items[0] {
                    IndexItem::Elem(e) => {
                        let raw = self.eval_index_i64(env, e)?;
                        let idx = normalize_index(raw, a.len()).ok_or_else(|| {
                            RuntimeError::IndexOutOfBounds(format!(
                                "index {raw} (length {})",
                                a.len()
                            ))
                        })?;
                        Ok(a[idx].clone())
                    }
                    IndexItem::Slice { start, end } => {
                        let (lo, hi) =
                            self.slice_bounds(env, start.as_ref(), end.as_ref(), a.len())?;
                        Ok(Value::Array(a[lo..hi].to_vec()))
                    }
                }
            }
            // Dict indexing by key (spec §11.6): a missing key is `R0012`.
            Value::Dict(d) => {
                if index.items.len() != 1 {
                    return crate::error::err("multi-dimensional indexing is not supported yet");
                }
                match &index.items[0] {
                    IndexItem::Elem(e) => {
                        let k = self.eval_expr(env, e)?;
                        let key = ValueKey::from_value(&k).ok_or_else(|| {
                            RuntimeError::Message("dict key must be a hashable value".into())
                        })?;
                        d.get(&key)
                            .cloned()
                            .ok_or_else(|| RuntimeError::Message("key not found".into()))
                    }
                    IndexItem::Slice { .. } => crate::error::err("cannot slice a dict"),
                }
            }
            other => crate::error::err(format!(
                "indexing requires an array or dict, got {}",
                value_type_name(&other)
            )),
        }
    }

    /// Evaluate an index expression to a raw `i64` (negative allowed; normalized by the caller).
    fn eval_index_i64(&mut self, env: &EnvRef, e: &Expr) -> Result<i64, RuntimeError> {
        match self.eval_expr(env, e)? {
            Value::Number(n) => n
                .as_i64()
                .ok_or_else(|| RuntimeError::Message("array index must be an integer".into())),
            _ => crate::error::err("array index must be an integer"),
        }
    }

    fn apply_function(&mut self, func: &Function, args: Vec<Value>) -> Result<Value, RuntimeError> {
        // Collection convenience functions receive their array argument whole (spec appendix B.1):
        // they are not subject to the implicit-broadcast path (spec §11.4).
        let takes_array_whole = matches!(func, Function::Builtin(b) if b.is_collection());
        let array_positions: Vec<usize> = if takes_array_whole {
            Vec::new()
        } else {
            args.iter()
                .enumerate()
                .filter(|(_, v)| matches!(v, Value::Array(_)))
                .map(|(i, _)| i)
                .collect()
        };
        if !array_positions.is_empty() && func.is_pure() {
            if self.current_config().broadcast {
                return self.broadcast_call(func, args, &array_positions);
            }
            return crate::error::err(
                "implicit broadcast is disabled (`broadcast := false`); use `@.`",
            );
        }
        match func {
            Function::Builtin(b) => self.call_builtin(*b, args),
            Function::NativeGet => {
                if args.len() != 2 {
                    return crate::error::err("`get` expects 2 arguments");
                }
                self.call_array_get(args[0].clone(), args[1].clone())
            }
            Function::Native { call, .. } => call(self, &args),
            Function::User {
                params,
                body,
                env: f_env,
                hot,
                ..
            } => {
                if args.len() != params.len() {
                    return crate::error::err(format!(
                        "expected {} arguments, got {}",
                        params.len(),
                        args.len()
                    ));
                }
                // JIT hot path (spec §19.2): when every argument is a non-complex number, run the body
                // natively. `@jit` forces compilation on the first call; otherwise the body is compiled
                // once after `JIT_CALL_THRESHOLD` numeric calls. Failed compilations are cached so a
                // non-numeric body always falls through to the interpreted path below.
                if let Some(inputs) = numeric_args(&args) {
                    if let Some(Some(f)) = hot.compiled.get() {
                        return Ok(Value::Number(Number::Real(Real::F64(f.call(&inputs)))));
                    }
                    if hot.compiled.get().is_none() {
                        // Auto hot-path compilation (spec §19.2, gated at `opt_level >= O2` per §10.2):
                        // compile on the call that makes the count reach `JIT_CALL_THRESHOLD` (spec §19.2
                        // default 100), so `for i in 1..100 { f(to_f64(i)) }` warms up and the next call
                        // (`f(to_f64(101))`) runs native. `@jit` (an execution-model annotation, §10.2)
                        // forces compilation on the first numeric call at any tier.
                        let c = hot.calls.fetch_add(1, AtomicOrdering::Relaxed);
                        let attempt = hot.force
                            || (self.current_config().opt_level >= OptLevel::O2
                                && c + 1 >= JIT_CALL_THRESHOLD);
                        if attempt {
                            let compiled = self.try_compile_body(params, body, f_env);
                            let _ = hot.compiled.set(compiled);
                            if let Some(Some(f)) = hot.compiled.get() {
                                return Ok(Value::Number(Number::Real(Real::F64(f.call(&inputs)))));
                            }
                        }
                    }
                }
                let call_env = Env::child(f_env);
                for (p, a) in params.iter().zip(args) {
                    call_env.borrow_mut().set_value(&p.name.value, a);
                }
                self.eval_expr(&call_env, body)
            }
            Function::Host {
                params,
                ret: _,
                body,
                env: f_env,
            } => self.apply_host(params, body, f_env, args),
            // A `@builtin(ON)` layered fn (spec §18.4): the Rust implementation is used when the
            // active `opt_level` is at least the declared tier and it is registered; otherwise the
            // `.pra` fallback body is evaluated (host semantics, so TCO applies at `O2`).
            Function::Layered {
                params,
                body,
                env: f_env,
                native,
                level,
                ..
            } => {
                if args.len() != params.len() {
                    return crate::error::err(format!(
                        "expected {} arguments, got {}",
                        params.len(),
                        args.len()
                    ));
                }
                if let Some(call) = native {
                    let cfg_level = self.current_config().opt_level.tier();
                    if cfg_level >= *level {
                        return call(self, &args);
                    }
                }
                let tco = self.current_config().opt_level >= OptLevel::O2;
                self.apply_host_tco(params, body, f_env, args, tco)
            }
        }
    }

    /// Evaluate a host `fn` body (spec §11.2). When `tco` is enabled and the body ends in a direct
    /// `return f(args)` preceded only by effect-free statements (see `crate::opt`), the call is
    /// trampolined so tail recursion runs in constant stack space; otherwise the body is evaluated
    /// normally (spec §10.2 item 6, gated by `opt_level >= O2`).
    fn apply_host(
        &mut self,
        params: &[Param],
        body: &Block,
        f_env: &EnvRef,
        args: Vec<Value>,
    ) -> Result<Value, RuntimeError> {
        let tco = self.current_config().opt_level >= OptLevel::O2;
        self.apply_host_tco(params, body, f_env, args, tco)
    }

    fn apply_host_tco(
        &mut self,
        params: &[Param],
        body: &Block,
        f_env: &EnvRef,
        args: Vec<Value>,
        tco: bool,
    ) -> Result<Value, RuntimeError> {
        if args.len() != params.len() {
            return crate::error::err(format!(
                "expected {} arguments, got {}",
                params.len(),
                args.len()
            ));
        }
        // Tail-call optimization (spec §10.2 item 6): when a host body ends in a direct
        // `return f(args)` preceded only by effect-free statements (see `crate::opt`), the
        // call is trampolined so tail recursion runs in constant stack space. Early `return`s
        // in the effect-free prefix are honored; the prefix is re-evaluated per iteration but
        // is pure, so this cannot change observable behavior.
        if !tco {
            let call_env = Env::child(f_env);
            for (p, a) in params.iter().zip(args) {
                call_env.borrow_mut().set_value(&p.name.value, a);
            }
            return self.eval_block_tail(&call_env, body);
        }
        let mut cparams: Vec<Param> = params.to_vec();
        let mut cbody: Block = (*body).clone();
        let mut cenv = Rc::clone(f_env);
        let mut cargs = args;
        loop {
            let call_env = Env::child(&cenv);
            for (p, a) in cparams.iter().zip(&cargs) {
                call_env.borrow_mut().set_value(&p.name.value, a.clone());
            }
            let Some(tc) = crate::opt::tail_call_of(&cbody) else {
                return self.eval_block_tail(&call_env, &cbody);
            };
            let n = cbody.stmts.len();
            for stmt in &cbody.stmts[..n - 1] {
                if let Flow::Return(v) = self.eval_stmt(&call_env, stmt)? {
                    return Ok(v);
                }
            }
            let ExprKind::Path { segments } = &tc.callee.kind else {
                return self.eval_block_tail(&call_env, &cbody);
            };
            let next = self.resolve_func(&call_env, segments).ok_or_else(|| {
                RuntimeError::Message(format!("unknown function `{}`", path_key(segments)))
            })?;
            let nargs: Vec<Value> = tc
                .args
                .iter()
                .map(|a| self.eval_expr(&call_env, a))
                .collect::<Result<_, _>>()?;
            match next {
                Function::Host {
                    params: np,
                    ret: _,
                    body: nb,
                    env: nenv,
                } => {
                    if nargs.len() != np.len() {
                        return crate::error::err(format!(
                            "expected {} arguments, got {}",
                            np.len(),
                            nargs.len()
                        ));
                    }
                    cparams = np;
                    cbody = nb;
                    cenv = nenv;
                    cargs = nargs;
                }
                other => return self.apply_function(&other, nargs),
            }
        }
    }

    /// Broadcast (spec §11.4): pure functions are applied elementwise to array arguments; **empty arrays are rejected** (`R0014`),
    /// non-numeric elements/scalars error (`R0009`). `@parallel` MFn (spec §17.1) over large arrays are split across rayon threads.
    fn broadcast_call(
        &mut self,
        func: &Function,
        args: Vec<Value>,
        positions: &[usize],
    ) -> Result<Value, RuntimeError> {
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
        if let Function::User {
            params,
            body,
            parallel: true,
            ..
        } = func
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
                        // Only numeric elements participate in broadcast (spec §11.4, R0009).
                        match &a[i] {
                            Value::Number(n) => cargs.push(Value::Number(n.clone())),
                            _ => {
                                return crate::error::err("cannot broadcast a non-numeric element");
                            }
                        }
                    }
                } else {
                    match v {
                        Value::Number(_) => cargs.push(v.clone()),
                        _ => return crate::error::err("cannot broadcast a non-numeric scalar"),
                    }
                }
            }
            match self.apply_function(func, cargs)? {
                Value::Number(n) => results.push(Value::Number(n)),
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
                            match &a[i] {
                                Value::Number(n) => cargs.push(Value::Number(n.clone())),
                                _ => {
                                    return Err(RuntimeError::Message(
                                        "cannot broadcast a non-numeric element".into(),
                                    ));
                                }
                            }
                        } else {
                            return Err(RuntimeError::Message(
                                "cannot broadcast a non-numeric scalar".into(),
                            ));
                        }
                    } else {
                        match v {
                            Value::Number(_) => cargs.push(v.clone()),
                            _ => {
                                return Err(RuntimeError::Message(
                                    "cannot broadcast a non-numeric scalar".into(),
                                ));
                            }
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
                    _ => Err(RuntimeError::Message(
                        "broadcast result must be numeric".into(),
                    )),
                }
            })
            .collect();
        let mut out = Vec::with_capacity(len);
        for r in results {
            out.push(Value::Number(r?));
        }
        Ok(Value::Array(out))
    }

    /// Binary array operation (spec §11.3/§11.4): `Array + Array` concatenates; `Array ∘ Array` for the
    /// other operators is elementwise (equal lengths, numeric-homogeneous), `Array ∘ scalar` broadcasts
    /// the scalar. Empty arrays error (`R0014`).
    fn eval_binary_array(&mut self, op: BinOp, a: Value, b: Value) -> Result<Value, RuntimeError> {
        // `Array + Array` concatenates (spec §11.3, v2.1) — this overrides the stale §11.4 elementwise example.
        if op == BinOp::Add && matches!(&a, Value::Array(_)) && matches!(&b, Value::Array(_)) {
            let Value::Array(mut av) = a else {
                unreachable!("checked above")
            };
            let Value::Array(bv) = b else {
                unreachable!("checked above")
            };
            av.extend(bv);
            return Ok(Value::Array(av));
        }
        let out: Vec<Value> = match (a, b) {
            (Value::Array(av), Value::Array(bv)) => {
                if av.len() != bv.len() {
                    return crate::error::err("dimension mismatch in array operation");
                }
                if av.is_empty() {
                    return crate::error::err("cannot operate on an empty array");
                }
                let av = require_numeric_array(&av)?;
                let bv = require_numeric_array(&bv)?;
                if let Some(v) = self.try_simd_arrays(op, &av, &bv) {
                    return Ok(Value::Array(v));
                }
                let mut out = Vec::with_capacity(av.len());
                for (x, y) in av.into_iter().zip(bv) {
                    match self.eval_number_binary(op, x, y)? {
                        Value::Number(n) => out.push(Value::Number(n)),
                        _ => return crate::error::err("array operation result must be numeric"),
                    }
                }
                out
            }
            (Value::Array(av), other) => {
                let scalar = self.scalar_for_broadcast(other)?;
                if av.is_empty() {
                    return crate::error::err("cannot operate on an empty array");
                }
                let av = require_numeric_array(&av)?;
                if let Some(v) = self.try_simd_scalar(op, &av, &scalar) {
                    return Ok(Value::Array(v));
                }
                let mut out = Vec::with_capacity(av.len());
                for x in av {
                    match self.eval_number_binary(op, x, scalar.clone())? {
                        Value::Number(n) => out.push(Value::Number(n)),
                        _ => return crate::error::err("array operation result must be numeric"),
                    }
                }
                out
            }
            (other, Value::Array(bv)) => {
                let scalar = self.scalar_for_broadcast(other)?;
                if bv.is_empty() {
                    return crate::error::err("cannot operate on an empty array");
                }
                let bv = require_numeric_array(&bv)?;
                if let Some(v) = self.try_simd_scalar_left(op, &scalar, &bv) {
                    return Ok(Value::Array(v));
                }
                let mut out = Vec::with_capacity(bv.len());
                for y in bv {
                    match self.eval_number_binary(op, scalar.clone(), y)? {
                        Value::Number(n) => out.push(Value::Number(n)),
                        _ => return crate::error::err("array operation result must be numeric"),
                    }
                }
                out
            }
            _ => return crate::error::err("invalid array operation"),
        };
        Ok(Value::Array(out))
    }

    fn scalar_for_broadcast(&self, v: Value) -> Result<Number, RuntimeError> {
        match v {
            Value::Number(n) => Ok(n),
            _ => crate::error::err("cannot broadcast with a non-numeric scalar"),
        }
    }

    /// SIMD-accelerated elementwise `array ⊕ array` when the active tier is `O3` and both arrays are
    /// dense `F64` (spec §10.2). Returns `None` to fall back to the scalar loop.
    fn try_simd_arrays(&self, op: BinOp, a: &[Number], b: &[Number]) -> Option<Vec<Value>> {
        if self.current_config().opt_level < OptLevel::O3 {
            return None;
        }
        crate::simd::try_f64x4_arrays(op, a, b)
            .map(|nums| nums.into_iter().map(Value::Number).collect())
    }

    /// SIMD-accelerated elementwise `array ⊕ scalar` when the active tier is `O3` (spec §10.2).
    fn try_simd_scalar(&self, op: BinOp, a: &[Number], scalar: &Number) -> Option<Vec<Value>> {
        if self.current_config().opt_level < OptLevel::O3 {
            return None;
        }
        crate::simd::try_f64x4_scalar(op, a, scalar)
            .map(|nums| nums.into_iter().map(Value::Number).collect())
    }

    /// SIMD-accelerated elementwise `scalar ⊕ array` when the active tier is `O3` (spec §10.2).
    fn try_simd_scalar_left(&self, op: BinOp, scalar: &Number, b: &[Number]) -> Option<Vec<Value>> {
        if self.current_config().opt_level < OptLevel::O3 {
            return None;
        }
        crate::simd::try_f64x4_scalar_left(op, scalar, b)
            .map(|nums| nums.into_iter().map(Value::Number).collect())
    }

    /// `match`/`if let`/`while let` arm evaluation (spec §4.4/§16.3): first matching pattern (with optional guard) wins.
    fn eval_match(
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
    fn match_pattern(
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
    opt_level: crate::config::OptLevel::O2,
    num_to_big: true,
    print_format: crate::config::PrintFormat::Latex,
    overload_policy: OverloadPolicy::Warn,
};

/// Minimum array length for which a `@parallel` MFn broadcast is split across rayon threads (spec §17.1);
/// smaller arrays keep the sequential path to avoid thread-spawn overhead.
const PARALLEL_BROADCAST_THRESHOLD: usize = 1024;

fn path_key(segments: &[Spanned<String>]) -> String {
    segments
        .iter()
        .map(|s| s.value.as_str())
        .collect::<Vec<_>>()
        .join("::")
}

/// Map arguments to `f64` when every argument is a non-complex number; otherwise `None` (spec §19.2:
/// only numeric scalars participate in the JIT hot path).
fn numeric_args(args: &[Value]) -> Option<Vec<f64>> {
    let mut out = Vec::with_capacity(args.len());
    for a in args {
        if let Value::Number(n) = a {
            if n.is_complex() {
                return None;
            }
            out.push(n.to_f64_lossy());
        } else {
            return None;
        }
    }
    Some(out)
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
        ExprKind::Binary { lhs, rhs, .. } => {
            collect_read_names(lhs, out);
            collect_read_names(rhs, out);
        }
        ExprKind::Unary { operand, .. } | ExprKind::Try(operand) => {
            collect_read_names(operand, out)
        }
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
        ExprKind::Dict(entries) => {
            for (k, v) in entries {
                collect_read_names(k, out);
                collect_read_names(v, out);
            }
        }
        ExprKind::Set(items) => {
            for it in items {
                collect_read_names(it, out);
            }
        }
        ExprKind::Comprehension {
            output, clauses, ..
        } => {
            collect_read_names(output, out);
            for c in clauses {
                match c {
                    ComprehensionClause::For { iter, .. } => collect_read_names(iter, out),
                    ComprehensionClause::If { cond } => collect_read_names(cond, out),
                }
            }
        }
        ExprKind::KeyValue { key, value } => {
            collect_read_names(key, out);
            collect_read_names(value, out);
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
type ParforWriteVec = Vec<(String, usize, Value)>;

/// Static side-effect check for a `parfor` body (spec §17.2, `E0082`): only index-slot assignments
/// (`A[i] = …`/`A[i] += …`/`A[i] -= …`) and pure function calls are allowed; anything else (external
/// variable assignment, `let`, `print`, class mutation, …) is an error.
fn check_parfor_body(body: &Block) -> Result<Vec<ParforStep>, RuntimeError> {
    let mut steps = Vec::new();
    for stmt in &body.stmts {
        match stmt {
            Stmt::Assign {
                target, op, value, ..
            } => {
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
        Pattern::Struct { fields, .. } => fields.iter().any(|f| match &f.pat {
            None => false,
            Some(sub) => pattern_is_refutable(sub),
        }),
        Pattern::Group(inner) => pattern_is_refutable(inner),
        Pattern::Or(pats) => pats.iter().any(pattern_is_refutable),
        _ => true,
    }
}

/// Short display name of a value's type, for error messages.
pub fn value_type_name(v: &Value) -> String {
    match v {
        Value::Nil => "nil".into(),
        Value::Number(_) => "number".into(),
        Value::Bool(_) => "bool".into(),
        Value::Char(_) => "char".into(),
        Value::String(_) => "string".into(),
        Value::Array(_) => "array".into(),
        Value::Dict(_) => "dict".into(),
        Value::Set(_) => "set".into(),
        Value::Expr(_) => "expr".into(),
        Value::Symbol(_) => "symbol".into(),
        Value::Class(_) => "class".into(),
        Value::Option(_) => "option".into(),
        Value::Result(_) => "result".into(),
        Value::Tuple(_) => "tuple".into(),
        Value::Indeterminate(_) => "indeterminate".into(),
        Value::Undefined => "undefined".into(),
        Value::Error(_) => "error".into(),
        Value::JitFunction(_) => "jit function".into(),
    }
}

/// Source-level name of a type for diagnostic signatures (spec §16.4); `Type::User` carries its text.
fn render_ty(t: &Type) -> String {
    match t {
        Type::Number => "Number".into(),
        Type::Integer => "Integer".into(),
        Type::Rational => "Rational".into(),
        Type::F64 => "F64".into(),
        Type::F32 => "F32".into(),
        Type::I8 => "I8".into(),
        Type::I16 => "I16".into(),
        Type::I32 => "I32".into(),
        Type::I64 => "I64".into(),
        Type::I128 => "I128".into(),
        Type::U8 => "U8".into(),
        Type::U16 => "U16".into(),
        Type::U32 => "U32".into(),
        Type::U64 => "U64".into(),
        Type::U128 => "U128".into(),
        Type::Isize => "Isize".into(),
        Type::Usize => "Usize".into(),
        Type::Complex => "Complex".into(),
        Type::Expr => "Expr".into(),
        Type::Symbol => "Symbol".into(),
        Type::Bool => "Bool".into(),
        Type::String => "String".into(),
        Type::Char => "Char".into(),
        Type::Array(inner) => format!("Array<{}>", render_ty(inner)),
        Type::Matrix(inner) => format!("Matrix<{}>", render_ty(inner)),
        Type::Tuple(ts) => format!(
            "Tuple<{}>",
            ts.iter().map(render_ty).collect::<Vec<_>>().join(", ")
        ),
        Type::Option(inner) => format!("Option<{}>", render_ty(inner)),
        Type::Result(a, b) => format!("Result<{}, {}>", render_ty(a), render_ty(b)),
        Type::Fn { params, ret } => format!(
            "Fn({}) -> {}",
            params.iter().map(render_ty).collect::<Vec<_>>().join(", "),
            render_ty(ret)
        ),
        Type::MFn { params, ret } => format!(
            "MFn({}) -> {}",
            params.iter().map(render_ty).collect::<Vec<_>>().join(", "),
            render_ty(ret)
        ),
        Type::SelfType => "Self".into(),
        Type::User(sp) => sp.value.clone(),
    }
}

/// Render a method signature as `name(self, args) -> Ret` for diagnostic notes (spec §16.4),
/// omitting the `-> Ret` suffix when the method has no return type.
fn method_signature(name: &str, m: &MethodDef) -> String {
    let params = m
        .params
        .iter()
        .map(|p| {
            if p.is_self {
                "self".to_string()
            } else {
                match &p.type_ann {
                    Some(t) => format!("{}: {}", p.name.value, render_ty(t)),
                    None => p.name.value.clone(),
                }
            }
        })
        .collect::<Vec<_>>()
        .join(", ");
    match &m.ret {
        Some(t) => format!("{name}({params}) -> {}", render_ty(t)),
        None => format!("{name}({params})"),
    }
}

/// A diagnostic note for a runtime `MethodDef` (spec §16.4): `method \`<sig>\`` plus the `///` doc
/// text when present. Only emitted when the method carries a doc comment — the runtime class model
/// records no definition location, so the `defined at` suffix is omitted here (registry/native docs
/// carry theirs; see [`native_method_error`]).
fn method_note(name: &str, m: &MethodDef) -> Option<String> {
    let docs = m.docs.as_ref()?;
    let mut note = format!("method `{}`", method_signature(name, m));
    let text = docs.text();
    if !text.is_empty() {
        note.push_str(&format!("\n{text}"));
    }
    Some(note)
}

/// A diagnostic note for a registry `MethodDoc` (spec §16.4): `method \`<sig>\` defined at <loc>`
/// plus the doc text when present.
fn method_doc_note(doc: &crate::docs::MethodDoc) -> String {
    let mut note = format!("method `{}` defined at {}", doc.sig, doc.defined_at);
    if let Some(text) = &doc.doc
        && !text.is_empty()
    {
        note.push_str(&format!("\n{text}"));
    }
    note
}

/// Attach diagnostic notes/help to an error, or return it unchanged when there is nothing to add.
fn with_notes(
    error: RuntimeError,
    notes: impl IntoIterator<Item = String>,
    help: Option<String>,
) -> RuntimeError {
    let notes: Vec<String> = notes.into_iter().collect();
    if notes.is_empty() && help.is_none() {
        error
    } else {
        RuntimeError::WithNotes {
            notes,
            help,
            error: Box::new(error),
        }
    }
}

/// Wrap a failed native-method call (`String`/`Array`/`Dict`/`Set`, spec §18.1/§11.3/§11.6) with a
/// doc note from the process-global registry (spec §16.4). A known method uses its own doc; an
/// unknown method falls back to the class-level doc plus a `did you mean` from the documented
/// methods. The registry is seeded by `prima-stdlib` at startup and empty in runtime unit tests.
fn native_method_error(class: &str, name: &str, err: RuntimeError) -> RuntimeError {
    let mut notes = Vec::new();
    let mut help = None;
    if let Some(doc) = crate::docs::get_doc(&format!("{class}::{name}")) {
        // Known method: attach its own definition + doc (spec §16.4).
        notes.push(method_doc_note(&doc));
    } else if let Some(cand) = did_you_mean(
        name,
        crate::docs::class_methods(class)
            .iter()
            .map(|d| d.name.as_str()),
    ) && cand != name
    {
        // Unknown method: point at the nearest documented method (its sig + doc) and suggest it.
        help = Some(format!("did you mean `{cand}`?"));
        if let Some(doc) = crate::docs::get_doc(&format!("{class}::{cand}")) {
            notes.push(method_doc_note(&doc));
        } else if let Some(doc) = crate::docs::get_doc(class) {
            notes.push(method_doc_note(&doc));
        }
    } else if let Some(doc) = crate::docs::get_doc(class) {
        notes.push(method_doc_note(&doc));
    }
    with_notes(err, notes, help)
}

/// Best `did you mean` candidate among `candidates` for `attempted` (spec §16.4): the closest name
/// by Levenshtein distance when it is within 2, else any name sharing a non-empty prefix with the
/// attempted one. `None` when no plausible candidate exists.
fn did_you_mean<'a>(attempted: &str, candidates: impl Iterator<Item = &'a str>) -> Option<String> {
    let mut best: Option<(usize, &str)> = None;
    let mut prefixed: Option<&str> = None;
    for c in candidates {
        if c == attempted {
            continue;
        }
        let d = levenshtein(attempted, c);
        if best.as_ref().is_none_or(|(bd, _)| d < *bd) {
            best = Some((d, c));
        }
        if common_prefix_len(attempted, c) > 0 && prefixed.is_none() {
            prefixed = Some(c);
        }
    }
    match best {
        Some((d, c)) if d <= 2 => Some(c.to_string()),
        Some((_, c)) if common_prefix_len(attempted, c) > 0 => Some(c.to_string()),
        _ => prefixed.map(str::to_string),
    }
}

/// Number of leading characters the two strings share.
fn common_prefix_len(a: &str, b: &str) -> usize {
    a.chars().zip(b.chars()).take_while(|(x, y)| x == y).count()
}

/// Classic dynamic-programming Levenshtein edit distance.
fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0usize; b.len() + 1];
    for (i, ca) in a.iter().enumerate() {
        cur[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            cur[j + 1] = (prev[j + 1] + 1)
                .min(cur[j] + 1)
                .min(prev[j] + usize::from(ca != cb));
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

/// Arity guard for builtin functions (spec §16): wrong argument counts are `Message` errors.
fn check_arity(name: &str, args: &[Value], n: usize) -> Result<(), RuntimeError> {
    if args.len() == n {
        Ok(())
    } else {
        crate::error::err(format!(
            "`{name}` expects {n} argument(s), got {}",
            args.len()
        ))
    }
}

/// Every element of an array must be numeric for elementwise operations/broadcast (spec §11.4, `R0009`).
fn require_numeric_array(elems: &[Value]) -> Result<Vec<Number>, RuntimeError> {
    let mut out = Vec::with_capacity(elems.len());
    for e in elems {
        match e {
            Value::Number(n) => out.push(n.clone()),
            _ => return crate::error::err("array elements must be numeric (R0009)"),
        }
    }
    Ok(out)
}

/// Normalize an index against a length: negative indices count from the end; out-of-range → `None`
/// (spec §11.3, `R0003`).
fn normalize_index(idx: i64, len: usize) -> Option<usize> {
    let len_i = len as i64;
    let i = if idx < 0 { len_i + idx } else { idx };
    if i < 0 || i >= len_i {
        None
    } else {
        Some(i as usize)
    }
}

/// Normalize an insert position: `idx == len` is allowed (append); out-of-range → `None`.
fn normalize_insert(idx: i64, len: usize) -> Option<usize> {
    let len_i = len as i64;
    let i = if idx < 0 { len_i + idx } else { idx };
    if i < 0 || i > len_i {
        None
    } else {
        Some(i as usize)
    }
}

/// Remainder for the `%` operator (spec §11.4 elementwise Mod): exact for integers, f64 otherwise.
fn number_mod(x: &Number, y: &Number) -> Result<Number, RuntimeError> {
    if y.is_zero() {
        return crate::error::err("modulo by zero");
    }
    if let (Some(a), Some(b)) = (x.as_bigint(), y.as_bigint()) {
        return Ok(Number::Integer(a % b));
    }
    Ok(Number::Real(Real::F64(x.to_f64_lossy() % y.to_f64_lossy())))
}

/// Mutating array methods (spec §11.3): these require the receiver to be a single-segment path.
fn is_mutating_array_method(name: &str) -> bool {
    matches!(
        name,
        "push" | "pop" | "append" | "extend" | "insert" | "remove" | "clear" | "sort" | "reverse"
    )
}

/// Mutating dict methods (spec §11.6).
fn is_mutating_dict_method(name: &str) -> bool {
    matches!(
        name,
        "insert" | "remove" | "clear" | "update" | "setdefault" | "popitem"
    )
}

/// Mutating set methods (spec §11.6).
fn is_mutating_set_method(name: &str) -> bool {
    matches!(
        name,
        "add" | "remove" | "discard" | "pop" | "clear" | "update"
    )
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

/// Apply an f-string `:spec` refinement (spec §18.1): a `.N` precision formats float values to N
/// decimal places; `[[fill]align][width]` pads/aligns the rendered text (Python `format`
/// mini-language subset, with the leading-`0` zero-pad flag). Unknown or non-numeric forms are
/// left untouched rather than breaking the interpolation.
fn apply_spec(v: &Value, text: &str, spec: Option<&str>) -> String {
    let Some(spec) = spec else {
        return text.to_owned();
    };
    let spec: Vec<char> = spec.trim().chars().collect();
    if spec.is_empty() {
        return text.to_owned();
    }
    let mut i = 0;
    let mut fill = ' ';
    let mut align = None;
    if let Some(&c) = spec.first() {
        if matches!(c, '<' | '>' | '^') {
            align = Some(c);
            i = 1;
        } else if let Some(&a) = spec.get(1)
            && matches!(a, '<' | '>' | '^')
        {
            fill = c;
            align = Some(a);
            i = 2;
        }
    }
    if spec.get(i) == Some(&'0') {
        fill = '0';
        i += 1;
    }
    let mut width = 0usize;
    while let Some(&c) = spec.get(i) {
        if c.is_ascii_digit() {
            width = width * 10 + (c as usize - b'0' as usize);
            i += 1;
        } else {
            break;
        }
    }
    let mut precision = None;
    if spec.get(i) == Some(&'.') {
        i += 1;
        let mut p = 0usize;
        let mut any = false;
        while let Some(&c) = spec.get(i) {
            if c.is_ascii_digit() {
                p = p * 10 + (c as usize - b'0' as usize);
                any = true;
                i += 1;
            } else {
                break;
            }
        }
        if any {
            precision = Some(p);
        }
    }
    let mut body = text.to_owned();
    if let Some(p) = precision
        && let Value::Number(n) = v
    {
        match n {
            Number::Real(Real::F64(f)) => body = format!("{f:.p$}"),
            Number::Real(Real::F32(f)) => body = format!("{f:.p$}"),
            _ => {}
        }
    }
    if body.len() >= width {
        return body;
    }
    let pad = width - body.len();
    let fill = fill.to_string();
    // Python `format` default alignment: right for numbers (and zero-padding implies right),
    // left otherwise.
    let is_number = matches!(v, Value::Number(_));
    let align = align
        .or(if fill == "0" || is_number {
            Some('>')
        } else {
            None
        })
        .unwrap_or('<');
    match align {
        '>' => format!("{}{}", fill.repeat(pad), body),
        '^' => {
            let left = pad / 2;
            let right = pad - left;
            format!("{}{}{}", fill.repeat(left), body, fill.repeat(right))
        }
        _ => format!("{body}{}", fill.repeat(pad)),
    }
}

fn is_zero_literal(e: &Expr) -> bool {
    matches!(&e.kind, ExprKind::Literal(Literal::Integer(s)) if s == "0")
}

fn literal_value(e: &Expr) -> Option<Value> {
    match &e.kind {
        ExprKind::Literal(Literal::Integer(s)) => s
            .parse::<BigInt>()
            .ok()
            .map(Number::Integer)
            .map(Value::Number),
        ExprKind::Literal(Literal::Bool(b)) => Some(Value::Bool(*b)),
        ExprKind::Literal(Literal::String { value, .. }) => Some(Value::String(value.clone())),
        ExprKind::Unary {
            op: UnOp::Neg,
            operand,
        } => literal_value(operand).map(|v| match v {
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
        assert_eq!(
            eval("let t = (1, 2);\nlet (a, b) = t;\na + b"),
            Value::Number(Number::from(3))
        );
    }

    #[test]
    fn array_get_returns_option() {
        assert_eq!(
            eval("let v = [1, 2, 3];\nget(v, 1)"),
            Value::Option(Some(Box::new(Value::Number(Number::from(2)))))
        );
        assert_eq!(eval("let v = [1, 2, 3];\nget(v, 10)"), Value::Option(None));
        assert_eq!(
            eval("let v = [1, 2, 3];\nget(v, 0)"),
            Value::Option(Some(Box::new(Value::Number(Number::from(1)))))
        );
    }

    #[test]
    fn if_let_and_match() {
        assert_eq!(
            eval(
                "let v = [1, 2];\nlet r = 0;\nif let Some(x) = get(v, 0) {\n    r = x * 10;\n}\nr"
            ),
            Value::Number(Number::from(10))
        );
        let r = eval(
            "let r = try_i32(1e20);\nlet label = match r {\n    Ok(n) => \"ok\",\n    Err(e) => \"fail\"\n};\nlabel",
        );
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
            eval(
                "let r = match 5 {\n    0 => \"zero\",\n    1 | 2 => \"small\",\n    3..=9 => \"medium\",\n    n if n > 100 => \"large\",\n    _ => \"other\"\n};\nr"
            ),
            Value::String("medium".into())
        );
    }

    #[test]
    fn match_is_non_exhaustive() {
        assert!(
            Evaluator::new()
                .eval_value("match 1 {\n    2 => 0\n}")
                .is_err()
        );
    }

    #[test]
    fn try_operator_unwraps_ok() {
        let v = eval(
            "fn f(x) -> Result<Integer, Error> {\n    let v = try_i32(x)?;\n    return Ok(v);\n}\nf(7)",
        );
        assert_eq!(
            v,
            Value::Result(Ok(Box::new(Value::Number(Number::I32(7)))))
        );
    }

    #[test]
    fn try_operator_propagates_none() {
        let err = Evaluator::new().eval_value("fn g() -> Option<Integer> {\n    let x = get([1], 5)?;\n    return Some(x);\n}\ng()").unwrap_err();
        assert!(err.to_string().contains("None"), "unexpected error: {err}");
    }

    #[test]
    fn class_associated_function_and_method() {
        assert_eq!(
            eval_fmt(
                "class Vec2 {\n    x: F64, y: F64,\n    pub fn new(x, y) -> Self { Vec2 { x, y } }\n    pub fn sum(self) -> F64 { self.x + self.y }\n}\nlet v = Vec2::new(1, 2);\nv.sum()"
            ),
            "3"
        );
    }

    #[test]
    fn struct_literal_base_spreads_fields() {
        assert_eq!(
            eval_fmt(
                "class P { pub x: Integer, pub y: Integer }\nlet a = P { x: 1, y: 2 };\nlet b = P { x: 9, ..a };\nb.y"
            ),
            "2"
        );
    }

    #[test]
    fn private_fields_are_not_accessible_from_outside() {
        let src = "class C {\n    secret: Integer,\n    pub fn new(s) -> Self { C { secret: s } }\n}\nlet c = C::new(1);\nc.secret";
        assert!(Evaluator::new().eval_value(src).is_err());
    }

    #[test]
    fn missing_method_attaches_doc_note_and_did_you_mean() {
        let src = "\
class Greeter {
    pub name: String,
    /// Shout a greeting.
    pub fn greet(self) -> String { \"hello\" }
}
let g = Greeter { name: \"x\" };
g.greets()";
        let err = Evaluator::new()
            .eval_value(src)
            .expect_err("expected an unknown-method error");
        assert!(
            err.to_string().contains("unknown method `greets`"),
            "unexpected error: {err}"
        );
        assert_eq!(err.help().as_deref(), Some("did you mean `greet`?"));
        let notes = err.notes();
        assert!(
            notes.iter().any(|n| n.contains("Shout a greeting.")),
            "notes: {notes:?}"
        );
        assert!(
            notes.iter().any(|n| n.contains("greet(self)")),
            "notes: {notes:?}"
        );
    }

    #[test]
    fn wrong_arity_on_documented_method_attaches_note() {
        let src = "\
class Greeter {
    pub name: String,
    /// Shout a greeting.
    pub fn greet(self) -> String { \"hello\" }
}
let g = Greeter { name: \"x\" };
g.greet(1)";
        let err = Evaluator::new()
            .eval_value(src)
            .expect_err("expected an arity error");
        assert!(
            err.to_string().contains("expects 0 arguments"),
            "unexpected error: {err}"
        );
        let notes = err.notes();
        assert!(
            notes.iter().any(|n| n.contains("greet(self)")),
            "notes: {notes:?}"
        );
        assert!(
            notes.iter().any(|n| n.contains("Shout a greeting.")),
            "notes: {notes:?}"
        );
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
    fn semicolon_statements_evaluate_without_warnings() {
        let mut ev = Evaluator::new();
        ev.eval_value("let a = 1;\nlet b = 2;\na + b")
            .expect("eval failed");
        assert!(
            ev.warnings().is_empty(),
            "`;`-separated statements emit no warnings"
        );
    }

    #[test]
    fn newline_separated_statements_fail_to_evaluate() {
        // Spec §4.2: newline statement separation was removed; the parser rejects it (E0011).
        assert!(
            Evaluator::new()
                .eval_value("let a = 1\nlet b = 2\n")
                .is_err()
        );
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
        assert_eq!(
            eval("let a = [1, 2, 3];\na[1] = 9;\na"),
            Value::Array(vec![
                Value::Number(Number::from(1)),
                Value::Number(Number::from(9)),
                Value::Number(Number::from(3)),
            ])
        );
    }

    #[test]
    fn array_slice_returns_subarray() {
        assert_eq!(
            eval("let a = [1, 2, 3, 4];\na[1..3]"),
            Value::Array(vec![
                Value::Number(Number::from(2)),
                Value::Number(Number::from(3)),
            ])
        );
    }

    #[test]
    fn host_namespace_native_function_dispatch() {
        // The registry is a process-global `OnceLock`; use a uniquely-named namespace (idempotent).
        crate::stdlib::register_namespace(
            "testns_eval",
            HashMap::from([(
                "answer".to_string(),
                NamespaceItem::Func(Function::Native {
                    name: "testns_eval::answer",
                    call: |_ev, args| {
                        if args.is_empty() {
                            Ok(Value::Number(Number::from(42)))
                        } else {
                            Err(RuntimeError::Message("`answer` takes no arguments".into()))
                        }
                    },
                }),
            )]),
        );
        assert_eq!(
            eval("import testns_eval;\ntestns_eval::answer()"),
            Value::Number(Number::from(42))
        );
    }
}
