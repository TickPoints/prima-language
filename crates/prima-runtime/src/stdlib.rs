//! Process-global registry of Rust-hosted stdlib namespaces (spec §18).
//!
//! The `prima-stdlib` crate registers its module namespaces (e.g. `"linalg"`, `"sys::env"`,
//! `"time"`) here at startup; the module loader (`module.rs`) and `bind_imports` (`eval.rs`) consult
//! this registry when an `import` does not resolve to a `.pra` file.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use crate::eval::NamespaceItem;

/// One stdlib module's items, keyed by item name.
type Namespace = HashMap<String, NamespaceItem>;

/// `NamespaceItem` may carry `Rc` environments (`Function::User`/`Host`), which are not `Sync`.
/// The registry is written only during startup (`prima_stdlib::init`) and read under a `Mutex`,
/// so sharing the table itself is sound: the stored `Rc`s are never dereferenced off the thread
/// that holds the lock. The stdlib registers only `Function::Native` items in practice (no `Rc`).
#[derive(Clone)]
struct SyncItem(NamespaceItem);

// SAFETY: all access to stored items happens under the registry `Mutex`; the stored `Rc` envs are
// never dereferenced across threads. No `Send`/`Sync` item is ever moved between threads either.
unsafe impl Send for SyncItem {}
unsafe impl Sync for SyncItem {}

static NAMESPACES: OnceLock<Mutex<HashMap<String, HashMap<String, SyncItem>>>> = OnceLock::new();

fn registry() -> &'static Mutex<HashMap<String, HashMap<String, SyncItem>>> {
    NAMESPACES.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Register one stdlib namespace (module path, e.g. `"linalg"`, `"sys::env"`, `"time"`).
/// Idempotent: later registrations of an existing name are ignored.
pub fn register_namespace(name: impl Into<String>, items: Namespace) {
    let name = name.into();
    let wrapped = items.into_iter().map(|(k, v)| (k, SyncItem(v))).collect();
    let mut map = registry().lock().unwrap_or_else(|e| e.into_inner());
    map.entry(name).or_insert(wrapped);
}

/// Look up a registered stdlib namespace (full module path), cloning its items.
pub fn get_namespace(name: &str) -> Option<Namespace> {
    let map = registry().lock().unwrap_or_else(|e| e.into_inner());
    map.get(name)
        .map(|items| items.iter().map(|(k, v)| (k.clone(), v.0.clone())).collect())
}

/// Whether a module path is a registered stdlib namespace.
pub fn has_namespace(name: &str) -> bool {
    let map = registry().lock().unwrap_or_else(|e| e.into_inner());
    map.contains_key(name)
}
