//! Evaluation environment and function values (spec §11/§12.2/§15.2/§19.2).
//!
//! This module owns the value/function/module namespaces of one environment scope, the shared
//! parent-chain handle `EnvRef`, the runtime `Function` values, and the per-MFn hot-path JIT state.
//! It is a leaf of the evaluator: `Evaluator` (in `super::super`) is referenced only through the
//! `NativeCall` type alias, so this module carries no interpreter state.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, OnceLock};

use prima_core::Value;
use prima_syntax::ast::{Block, Param, Type};

use crate::builtins::Builtin;
use crate::error::RuntimeError;

/// A Rust-hosted standard-library function registered by `prima-stdlib` (spec §18): called with the
/// evaluator (for access to pool/symbols/output) and the already-evaluated arguments.
pub type NativeCall = fn(&mut super::Evaluator, &[Value]) -> Result<Value, RuntimeError>;

/// Default call count before an MFn body is JIT-compiled (spec §19.2); `@jit` functions skip the countdown.
pub const JIT_CALL_THRESHOLD: u64 = 100;

/// Per-MFn hot-path state (spec §19.2): a monotonic call counter and the compiled artifact, guarded by a
/// `OnceLock` so the body is compiled at most once per `Function::User` instance. Compilation failure is
/// cached as `None` so a non-numeric body is never retried.
pub struct HotState {
    /// `@jit` annotation: compile on the first numeric call regardless of count.
    pub force: bool,
    pub(crate) calls: AtomicU64,
    pub(crate) compiled: OnceLock<Option<Arc<prima_jit::CompiledScalar>>>,
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

// `Function::User`/`Host`/`Layered` carry `EnvRef` (an `Rc`), so `Function` is not `Send`/`Sync`.
// The stdlib registry wraps items in `SyncItem`; `NativeCall` is a plain `fn` pointer, which is `Send`.

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

use prima_syntax::ast::Expr;

/// Module namespace item (spec §15.2): a public function, value, or class exported by a module.
#[derive(Clone)]
pub enum NamespaceItem {
    Func(Function),
    Val(Value),
    Class(crate::class::ClassDef),
}

/// Shared handle for an evaluation environment: an `Rc<RefCell>` shared chain makes block-scope shadowing (spec §12.2)
/// and cross-scope assignment (updating outer variables inside `while`/`for` bodies) both work.
pub type EnvRef = Rc<RefCell<Env>>;

/// Evaluation environment: dual value/function namespaces plus a module namespace plus a shared parent-environment chain.
#[derive(Clone, Default)]
pub struct Env {
    pub(crate) values: HashMap<String, Value>,
    pub(crate) funcs: HashMap<String, Function>,
    pub(crate) modules: HashMap<String, HashMap<String, NamespaceItem>>,
    pub(crate) parent: Option<EnvRef>,
}

impl Env {
    /// Root environment: pre-imports the common builtins of `core` (spec §15.5) plus `get` (spec §11.3).
    pub fn new() -> Env {
        let mut env = Env::default();
        for name in crate::eval::CORE_BUILTIN_NAMES {
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
    pub(crate) fn child(parent: &EnvRef) -> EnvRef {
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

    pub(crate) fn set_value(&mut self, name: &str, v: Value) {
        self.values.insert(name.to_string(), v);
    }

    /// Update an existing binding in place (along the shared chain, spec §12.2 shadowing); return `false` if undefined.
    pub(crate) fn set_existing(&mut self, name: &str, v: Value) -> bool {
        if self.values.contains_key(name) {
            self.values.insert(name.to_string(), v);
            return true;
        }
        if let Some(p) = &self.parent {
            return p.borrow_mut().set_existing(name, v);
        }
        false
    }

    pub(crate) fn get_func(&self, name: &str) -> Option<Function> {
        if let Some(f) = self.funcs.get(name) {
            return Some(f.clone());
        }
        self.parent.as_ref().and_then(|p| p.borrow().get_func(name))
    }

    pub(crate) fn set_func(&mut self, name: &str, f: Function) {
        self.funcs.insert(name.to_string(), f);
    }

    /// Register a module namespace (key is the module path or alias, spec §15.1). Returns `true` if it already existed.
    pub(crate) fn set_module(&mut self, name: &str, items: HashMap<String, NamespaceItem>) -> bool {
        self.modules.insert(name.to_string(), items).is_some()
    }

    /// Resolve `module::item` (spec §15.2 qualified access).
    pub(crate) fn lookup_module_item(&self, ns_key: &str, item: &str) -> Option<NamespaceItem> {
        if let Some(m) = self.modules.get(ns_key)
            && let Some(it) = m.get(item)
        {
            return Some(it.clone());
        }
        self.parent
            .as_ref()
            .and_then(|p| p.borrow().lookup_module_item(ns_key, item))
    }

    pub(crate) fn bind_item(&mut self, name: &str, item: NamespaceItem) {
        match item {
            NamespaceItem::Func(f) => self.set_func(name, f),
            NamespaceItem::Val(v) => self.set_value(name, v),
            // Classes live in the evaluator's class registry; `bind_imports` registers them there directly.
            NamespaceItem::Class(_) => {}
        }
    }
}

/// The broadcast backend a builtin-class method dispatches through (spec §18.1): read-only collection
/// and `Char`/`Tuple` backends, receiver write-back via `mutate_*` (spec §11.3/§11.6), or the collapse
/// family (`collapse::call`, spec §9) for `Number`/`Option`/`Result`.
#[derive(Debug, Clone, Copy)]
pub(crate) enum BuiltinBackend {
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
