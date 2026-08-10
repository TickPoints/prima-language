use std::sync::{OnceLock, RwLock};

use dashmap::DashMap;

/// Symbol identifier (spec §7): built-in symbols and user symbols share the same registry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SymbolId(pub u32);

/// Symbol registry: name → `SymbolId`, with the display name taken from the TeX name (e.g. `pi → \pi`, spec §7).
/// Lazily initialized; built-in symbols are registered on the first access to `SymbolTable::global()`.
pub struct SymbolTable {
    names: RwLock<Vec<String>>,
    map: DashMap<String, SymbolId>,
}

impl SymbolTable {
    pub fn new() -> SymbolTable {
        SymbolTable {
            names: RwLock::new(Vec::new()),
            map: DashMap::new(),
        }
    }

    pub fn global() -> &'static SymbolTable {
        static TABLE: OnceLock<SymbolTable> = OnceLock::new();
        TABLE.get_or_init(|| {
            let t = SymbolTable::new();
            crate::builtins::register(&t);
            t
        })
    }

    pub fn intern(&self, name: &str) -> SymbolId {
        if let Some(id) = self.map.get(name) {
            return *id;
        }
        let mut names = self.names.write().unwrap();
        if let Some(id) = self.map.get(name) {
            return *id;
        }
        let id = SymbolId(names.len() as u32);
        names.push(name.to_string());
        self.map.insert(name.to_string(), id);
        id
    }

    /// Register a symbol with an explicit display name (TeX name; spec §7: built-in symbols are independent of TeX, TeX is only a view).
    pub fn intern_display(&self, name: &str, display: &str) -> SymbolId {
        if let Some(id) = self.map.get(name) {
            return *id;
        }
        let mut names = self.names.write().unwrap();
        if let Some(id) = self.map.get(name) {
            return *id;
        }
        let id = SymbolId(names.len() as u32);
        names.push(display.to_string());
        self.map.insert(name.to_string(), id);
        id
    }

    pub fn name(&self, id: SymbolId) -> Option<String> {
        self.names.read().unwrap().get(id.0 as usize).cloned()
    }
}

impl Default for SymbolTable {
    fn default() -> Self {
        Self::new()
    }
}
