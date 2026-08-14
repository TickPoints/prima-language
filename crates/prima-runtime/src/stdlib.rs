//! Process-global registry of Rust-hosted stdlib namespaces, `@builtin` implementations, and
//! embedded stdlib signature modules (spec §18 / §18.4).
//!
//! Three registries, all process-global `OnceLock`s:
//!
//! - **Namespaces** (`register_namespace`): Rust-hosted stdlib modules (e.g. `"linalg"`,
//!   `"sys::env"`, `"time"`) whose items are `NamespaceItem`s with no backing file. This is the
//!   legacy form (spec §18); the physics constants remain a plain host namespace of `NamespaceItem::Val`.
//! - **`@builtin` implementations** (`register_impl`): keyed by fully-qualified `"module::name"`
//!   (e.g. `"linalg::Matrix::zeros"`, `"time::Duration::from_secs"`). Embedded stdlib signature
//!   modules bind their `@builtin pub fn` declarations to these at evaluation time (spec §18.4).
//! - **Embedded stdlib sources** (`register_module_source`): the `.pra` signature files, keyed by
//!   module path (`"linalg"`, `"sys::path"`, `"time"`). The module loader resolves `import`s to
//!   these when no file is on disk.
//!
//! The `prima-stdlib` crate registers all three at startup; the module loader (`module.rs`) and
//! `bind_imports` (`eval.rs`) consult them when an `import` does not resolve to a `.pra` file.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use crate::eval::{NamespaceItem, NativeCall};

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

/// Implementation for an `@builtin` stdlib function, keyed by fully-qualified `"module::name"`
/// (e.g. `"linalg::transpose"`, `"linalg::Matrix::zeros"`, `"time::now"`). `fn` pointers are
/// `Send`/`Sync`, so no `SyncItem` wrapper is needed (unlike `NamespaceItem`). The `Mutex` keeps
/// concurrent registrations (e.g. parallel tests) from losing a write.
static IMPLS: OnceLock<Mutex<HashMap<String, NativeCall>>> = OnceLock::new();

fn impls() -> &'static Mutex<HashMap<String, NativeCall>> {
    IMPLS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Register one `@builtin` implementation (spec §18.4). Idempotent: the first registration wins,
/// so a shared key cannot be silently overwritten by a later `init`/test.
pub fn register_impl(key: impl Into<String>, call: NativeCall) {
    let key = key.into();
    let mut map = impls().lock().unwrap_or_else(|e| e.into_inner());
    map.entry(key).or_insert(call);
}

/// Look up a registered `@builtin` implementation by its fully-qualified key.
pub fn get_impl(key: &str) -> Option<NativeCall> {
    let map = impls().lock().unwrap_or_else(|e| e.into_inner());
    map.get(key).copied()
}

/// Embedded stdlib module source (the `.pra` signature file), keyed by module path
/// (e.g. `"linalg"`, `"sys::path"`, `"time"`). Static strings are trivially `Sync`; the `Mutex`
/// again protects concurrent registration.
static SOURCES: OnceLock<Mutex<HashMap<String, &'static str>>> = OnceLock::new();

fn sources() -> &'static Mutex<HashMap<String, &'static str>> {
    SOURCES.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Register an embedded stdlib signature module (spec §18.4). Idempotent: the first registration wins.
pub fn register_module_source(path: impl Into<String>, src: &'static str) {
    let path = path.into();
    let mut map = sources().lock().unwrap_or_else(|e| e.into_inner());
    map.entry(path).or_insert(src);
}

/// Look up the embedded stdlib source for a module path.
pub fn get_module_source(path: &str) -> Option<&'static str> {
    let map = sources().lock().unwrap_or_else(|e| e.into_inner());
    map.get(path).copied()
}
