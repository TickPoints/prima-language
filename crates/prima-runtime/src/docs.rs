//! Runtime doc registry (spec §4.1 / §16.4 v2.2): method definitions and doc comments for the
//! native classes (`String`/`Array`/`Dict`/`Set`) and stdlib `@builtin` functions, so a failed
//! method call can attach a note with the method signature, definition location, and `///` doc.
//!
//! The single source of truth is the embedded `.pra` modules (`core/string.pra`, etc.); the
//! `prima-stdlib` crate parses them at startup and registers each item here, so diagnostics and
//! `prima doc` (spec §20) share the same data (implementation plan §4.8).
//!
//! Keys are `"<Class>::<method>"` (e.g. `"String::to_upper"`) and `"<Class>"` for the class-level
//! fallback doc. All access is under a `Mutex` (startup writes, diagnostic-time reads).

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

/// Documented definition of a method (or associated function).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MethodDoc {
    /// Method name, e.g. `to_upper`.
    pub name: String,
    /// Rendered signature, e.g. `to_upper(self) -> Self`.
    pub sig: String,
    /// Doc text from the embedded `.pra` `///` comment (absent when the method has no doc).
    pub doc: Option<String>,
    /// Display location of the definition, e.g. `core/string.pra:4:5`.
    pub defined_at: String,
}

static DOCS: OnceLock<Mutex<HashMap<String, MethodDoc>>> = OnceLock::new();

fn docs() -> &'static Mutex<HashMap<String, MethodDoc>> {
    DOCS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Register one method/class doc (spec §4.7). Idempotent: the first registration wins, so a shared
/// key cannot be silently overwritten by a later `init`/test.
pub fn register_doc(key: impl Into<String>, doc: MethodDoc) {
    let key = key.into();
    let mut map = docs().lock().unwrap_or_else(|e| e.into_inner());
    map.entry(key).or_insert(doc);
}

/// Look up a registered doc by fully-qualified key (e.g. `"String::to_upper"`).
pub fn get_doc(key: &str) -> Option<MethodDoc> {
    let map = docs().lock().unwrap_or_else(|e| e.into_inner());
    map.get(key).cloned()
}

/// All documented methods of a class (e.g. all `String::*` entries), for `did you mean`
/// suggestions and doc listing. Returns entries in registration order.
pub fn class_methods(class: &str) -> Vec<MethodDoc> {
    let prefix = format!("{class}::");
    let map = docs().lock().unwrap_or_else(|e| e.into_inner());
    map.iter()
        .filter(|(k, _)| k.starts_with(&prefix))
        .map(|(_, v)| v.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_and_lookup() {
        let d = MethodDoc {
            name: "to_upper".into(),
            sig: "to_upper(self) -> Self".into(),
            doc: Some("Uppercase the string.".into()),
            defined_at: "core/string.pra:4:5".into(),
        };
        register_doc("String::to_upper", d.clone());
        assert_eq!(get_doc("String::to_upper"), Some(d));
        assert_eq!(get_doc("String::nope"), None);
        assert!(class_methods("String").iter().any(|m| m.name == "to_upper"));
        // First registration wins.
        register_doc(
            "String::to_upper",
            MethodDoc { name: "to_upper".into(), sig: "x".into(), doc: None, defined_at: "y".into() },
        );
        assert_eq!(get_doc("String::to_upper").unwrap().sig, "to_upper(self) -> Self");
    }
}
