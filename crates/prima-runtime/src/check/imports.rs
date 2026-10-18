//! Import resolution checks (spec §15.3/§15.4, appendix C.1 `E0030`/`E0032`).
//!
//! `prima check` resolves the root file's imports the same way the module loader does:
//! embedded stdlib source → Rust-hosted namespace → file on disk. A path that resolves to
//! nothing is `E0030`; a `from`-import naming an item that exists but is not `pub` is `E0032`.
//! The public item sets are also returned so name checking can flag qualified access to private
//! items (`module::private_name`).
//!
//! Dependencies are not recursively checked here: each module reports its own errors when it is
//! checked as an entry point.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;

use prima_core::suggest::did_you_mean_help;
use prima_syntax::ast::{ImportItem, ImportKind, Pattern, Program, Stmt};
use prima_syntax::parse;

use super::TypeError;
use super::error::push_err_with_help;

use crate::module::{Resolution, resolve_import};

/// Top-level items of one imported module: `public` are exported, `all` includes private items
/// (so a private item can be distinguished from a missing one).
#[derive(Debug, Default, Clone)]
pub(crate) struct ModuleExports {
    pub public: HashSet<String>,
    pub all: HashSet<String>,
}

impl ModuleExports {
    /// Whether `name` was declared at all (public or private).
    fn declares(&self, name: &str) -> bool {
        self.all.contains(name)
    }
}

/// Import bindings that expose a module path, keyed by the prefix used in qualified access
/// (`import sys::path;` → `["sys", "path"]`).
pub(crate) type ModuleMap = HashMap<Vec<String>, ModuleExports>;

/// Resolve every import of the root program, report `E0030`/`E0032`, and return the module
/// prefix → exports map used by the qualified-access check.
pub(crate) fn check_imports(
    src: &str,
    file: &Path,
    program: &Program,
    errors: &mut Vec<TypeError>,
) -> ModuleMap {
    let dir = file.parent().unwrap_or_else(|| Path::new("."));
    let mut modules: ModuleMap = HashMap::new();

    for import in &program.imports {
        let segments: Vec<String> = match &import.kind {
            ImportKind::Namespace { path, .. } | ImportKind::From { path, .. } => {
                path.iter().map(|s| s.value.clone()).collect()
            }
        };
        let key = segments.join("::");
        let exports = match resolve_import(dir, &segments) {
            Resolution::None => {
                push_err_with_help(
                    src,
                    errors,
                    import.span,
                    "E0030",
                    format!(
                        "module `{key}` not found (tried `{}.pra` and `{}/main.pra`)",
                        segments.join("/"),
                        segments.join("/")
                    ),
                    None,
                    Some("check the module path and the file layout (spec §15.3)".into()),
                );
                continue;
            }
            Resolution::File(path) => match read_module(&path) {
                Some(exports) => exports,
                // A dependency that fails to parse is reported when it is checked on its own;
                // skip it here so one broken file does not cascade into every importer.
                None => continue,
            },
            Resolution::Embedded { .. } => match crate::stdlib::get_module_source(&key) {
                Some(source) => match parse(source) {
                    Ok(program) => collect_exports(&program),
                    Err(_) => continue,
                },
                None => continue,
            },
            Resolution::Host { .. } => continue,
        };

        if let ImportKind::From { items, .. } = &import.kind {
            for item in items {
                let ImportItem::Name { name, .. } = item else {
                    continue;
                };
                if exports.public.contains(&name.value) {
                    continue;
                }
                if exports.declares(&name.value) {
                    push_err_with_help(
                        src,
                        errors,
                        name.span,
                        "E0032",
                        format!("`{}` is private in module `{key}`", name.value),
                        None,
                        Some(format!(
                            "mark `{}` as `pub` in `{key}` to import it",
                            name.value
                        )),
                    );
                } else {
                    let help = did_you_mean_help(
                        &name.value,
                        exports.public.iter().chain(exports.all.iter()),
                    );
                    push_err_with_help(
                        src,
                        errors,
                        name.span,
                        "E0040",
                        format!("module `{key}` has no item `{}`", name.value),
                        None,
                        help,
                    );
                }
            }
        }

        // Qualified access binds the module path (or its alias) to the module's exported names.
        let prefix = match &import.kind {
            ImportKind::Namespace {
                alias: Some(alias), ..
            } => vec![alias.value.clone()],
            ImportKind::Namespace { path, .. } => {
                path.iter().map(|s| s.value.clone()).collect()
            }
            ImportKind::From { .. } => continue,
        };
        modules.insert(prefix, exports);
    }

    modules
}

/// Read and parse one module file into its item sets; `None` when it cannot be read or parsed.
fn read_module(path: &Path) -> Option<ModuleExports> {
    let source = fs::read_to_string(path).ok()?;
    let program = parse(&source).ok()?;
    Some(collect_exports(&program))
}

/// Collect declared top-level item names and the `pub` subset (spec §15.2).
fn collect_exports(program: &Program) -> ModuleExports {
    let mut exports = ModuleExports::default();
    for stmt in &program.stmts {
        match stmt {
            Stmt::Pub(inner) => {
                for name in item_names(inner) {
                    exports.public.insert(name.clone());
                    exports.all.insert(name);
                }
            }
            other => {
                for name in item_names(other) {
                    exports.all.insert(name);
                }
            }
        }
    }
    exports
}

/// Names bound by one top-level item (`let` patterns may bind several).
fn item_names(stmt: &Stmt) -> Vec<String> {
    match stmt {
        Stmt::Let { pat, .. } => {
            let mut names = Vec::new();
            pattern_names(pat, &mut names);
            names
        }
        Stmt::Const { name, .. }
        | Stmt::FnDef { name, .. }
        | Stmt::MathDef { name, .. }
        | Stmt::ClassDef { name, .. } => vec![name.value.clone()],
        _ => Vec::new(),
    }
}

fn pattern_names(pattern: &Pattern, out: &mut Vec<String>) {
    match pattern {
        Pattern::Binding(name) => out.push(name.value.clone()),
        Pattern::Tuple(patterns, _) | Pattern::Array(patterns, _) | Pattern::Or(patterns) => {
            for pattern in patterns {
                pattern_names(pattern, out);
            }
        }
        Pattern::Struct { fields, .. } => {
            for field in fields {
                if let Some(pattern) = &field.pat {
                    pattern_names(pattern, out);
                }
            }
        }
        Pattern::Variant { args, .. } => {
            for pattern in args {
                pattern_names(pattern, out);
            }
        }
        Pattern::Group(inner) => pattern_names(inner, out),
        Pattern::Wildcard(_) | Pattern::Literal(_) | Pattern::Range { .. } => {}
    }
}
