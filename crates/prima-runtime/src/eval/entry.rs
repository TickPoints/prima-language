//! Evaluator construction, entry points, and the module system (spec §4.8/§15).
//!
//! This module owns the `Evaluator` lifecycle (construction, config snapshot, warnings) and the
//! file/in-memory module pipeline: `eval_file`/`eval_module` (dependency order, `pub` collection),
//! `bind_imports` (namespace/selective/star imports + conflict detection), and the `eval_src`/
//! `eval_value` conveniences. Statement/expression evaluation lives in sibling modules.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use prima_core::{BuiltinSymbols, ExprId, ExprPool, SymbolTable, Value};
use prima_syntax::ast::{
    Annotation, Block, ConfigBlock, ImportItem, ImportKind, Param, Pattern, Program, Stmt, Type,
};
use prima_syntax::{Span, SyntaxWarning};

use crate::builtins::Builtin;
use crate::config::Config;
use crate::error::RuntimeError;
use crate::module::{ModuleGraph, ModuleUnit, ResolvedImport};

use super::helpers::{DEFAULT_CONFIG, syntax_errors};
use super::{Env, EnvRef, Evaluator, Flow, Function, HotState, NamespaceItem};

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

    pub(crate) fn reset_config(&mut self) {
        self.config.clear();
        self.config.push(Config::default());
    }

    pub(crate) fn current_config(&self) -> &Config {
        self.config.last().unwrap_or(&DEFAULT_CONFIG)
    }

    /// Simplify a symbolic `ExprId` at the depth requested by the active `simplify_level` policy
    /// (spec §8.3/§13.2). Semantics (not just polish) never change: lowering the level only reduces
    /// which rules fire, never the mathematical value.
    pub(crate) fn simplify_current(&self, id: ExprId) -> ExprId {
        prima_core::simplify::simplify_at(
            self.pool,
            self.builtins,
            id,
            self.current_config().simplify_level,
        )
    }

    pub(crate) fn push_module_config(
        &mut self,
        block: Option<&ConfigBlock>,
    ) -> Result<(), RuntimeError> {
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
    pub(crate) fn push_warning(&mut self, code: &'static str, span: Span, message: String) {
        self.warnings.push(SyntaxWarning {
            span,
            code,
            message,
        });
    }

    /// Fully-qualified registry key for an `@builtin` declared in the module currently being
    /// evaluated (spec §18.4): `"<module>::<name>"` in a stdlib module, plain `<name>` at the root.
    pub(crate) fn builtin_key(&self, name: &str) -> String {
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
    pub(crate) fn bind_builtin(&self, name: &str) -> Result<Function, RuntimeError> {
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
    pub(crate) fn bind_builtin_annotated(
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

    /// Invoke a `fn` by name through the bytecode VM (spec §19.5), bypassing the `vm` config gate so
    /// callers (benchmarks, C-ABI) can opt into the VM explicitly. Falls back to the AST interpreter
    /// when the body is outside the compiled subset.
    pub fn vm_call_function(
        &mut self,
        env: &EnvRef,
        name: &str,
        args: Vec<Value>,
    ) -> Result<Value, RuntimeError> {
        let func = env
            .borrow()
            .get_func(name)
            .ok_or_else(|| RuntimeError::Message(format!("unknown function `{name}`")))?;
        if let Some(v) = self.try_vm_call(&func, args.clone())? {
            return Ok(v);
        }
        self.apply_function(&func, args)
    }

    pub(crate) fn eval_root(
        &mut self,
        env: &EnvRef,
        root: &ModuleUnit,
    ) -> Result<(), RuntimeError> {
        self.push_module_config(root.program.config.as_ref())?;
        for stmt in &root.program.stmts {
            if let Flow::Return(_) = self.eval_stmt(env, stmt)? {
                return crate::error::err("`return` outside of a function");
            }
        }
        Ok(())
    }

    /// Evaluate a dependency module (spec §15): apply its module policy and collect `pub` items.
    pub(crate) fn eval_module(&mut self, unit: &ModuleUnit) -> Result<(), RuntimeError> {
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

    pub(crate) fn eval_module_inner(
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

    pub(crate) fn collect_pub(
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
    pub(crate) fn bind_imports(
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
    pub(crate) fn bind_imported_item(&mut self, env: &EnvRef, name: &str, item: &NamespaceItem) {
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

    pub(crate) fn eval_program_in(
        &mut self,
        env: &EnvRef,
        program: &Program,
    ) -> Result<(), RuntimeError> {
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
    pub(crate) fn bind_host_imports(
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

    pub(crate) fn eval_value_in(
        &mut self,
        env: &EnvRef,
        program: &Program,
    ) -> Result<Value, RuntimeError> {
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
            Value::Number(n) => prima_core::render::render_number(n),
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
            Value::Expr(id) => prima_core::render::render_latex(self.pool, self.symbols, *id),
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
}
