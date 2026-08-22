//! Module graph resolution (spec §15.3): maps `.pra` files and directories into a dependency graph by module path.
//!
//! This module only does filesystem resolution and parsing (parse-only); it performs no evaluation.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use prima_syntax::ast::{Import, ImportKind, Program};
use prima_syntax::parse;

/// Compilation unit (spec §15.3): one `.pra` file = one module body.
#[derive(Debug)]
pub struct ModuleUnit {
    /// Module path (excluding the root), e.g. `["linalg"]` or `["linalg", "fft"]`; empty for the root module.
    pub path: Vec<String>,
    /// Absolute path of the module file.
    pub file: PathBuf,
    /// The parsed module body.
    pub program: Program,
    /// The resolved imports (including the absolute paths of the target files).
    pub imports: Vec<ResolvedImport>,
}

/// A fully resolved import (spec §15.1). `host == true` marks a Rust-hosted stdlib namespace
/// (spec §18) that has no backing file; `embedded == true` marks an embedded stdlib signature
/// module (spec §18.4) whose source is compiled in; `file` is empty for host imports and a
/// `<stdlib>/…` marker for embedded ones.
#[derive(Debug)]
pub struct ResolvedImport {
    pub path: Vec<String>,
    pub file: PathBuf,
    pub kind: ImportKind,
    pub host: bool,
    pub embedded: bool,
}

/// Module dependency graph (spec §15.3 file mapping); resolution only, no evaluation.
#[derive(Debug)]
pub struct ModuleGraph {
    /// The root module (run entry point, empty path).
    pub root: ModuleUnit,
    /// All reachable dependency modules (excluding the root), in dependency order (imported modules precede their importers).
    pub deps: Vec<ModuleUnit>,
}

impl ModuleGraph {
    /// Load all reachable modules with `root_file` as the root module; imports resolve relative to `root_file`'s directory.
    /// Returns an `Err(String)` describing: missing file / cycle / any module failing to parse.
    pub fn load(root_file: &Path) -> Result<ModuleGraph, String> {
        let root_file = fs::canonicalize(root_file).map_err(|e| {
            format!("cannot access root module `{}`: {e}", root_file.display())
        })?;
        let mut loader = Loader::default();
        let root = loader.load_module(Vec::new(), &root_file)?;
        Ok(ModuleGraph { root, deps: loader.deps })
    }
}

/// Module loader: collects dependencies via DFS post-order, using the "done/loading" sets for dedup and cycle detection.
#[derive(Default)]
struct Loader {
    /// Modules already loaded (canonical absolute paths), used for dedup.
    done: HashSet<PathBuf>,
    /// Embedded stdlib signature modules already loaded (joined paths), used for dedup.
    /// Embedded modules never import, so this set needs no cycle detection.
    done_embedded: HashSet<String>,
    /// Stack of modules currently loading (canonical absolute paths), used for cycle detection.
    stack: Vec<(Vec<String>, PathBuf)>,
    /// Dependency-order result (post-order: imported modules precede their importers).
    deps: Vec<ModuleUnit>,
}

impl Loader {
    fn load_module(&mut self, path: Vec<String>, file: &Path) -> Result<ModuleUnit, String> {
        let file = fs::canonicalize(file)
            .map_err(|e| format!("cannot access module `{}`: {e}", file.display()))?;

        if let Some(idx) = self.stack.iter().position(|(_, f)| *f == file) {
            return Err(self.cycle_error(idx));
        }

        let label = display_path(&path, &file);
        let src = fs::read_to_string(&file)
            .map_err(|e| format!("cannot read module `{label}` (`{}`): {e}", file.display()))?;
        let program = parse(&src).map_err(|errs| {
            let details = errs.iter().map(|e| e.to_string()).collect::<Vec<_>>().join(", ");
            format!("module `{label}` (`{}`) failed to parse: {details}", file.display())
        })?;

        let dir = file.parent().expect("canonicalized module file has a parent directory");
        let mut imports = Vec::new();

        self.stack.push((path.clone(), file.clone()));
        for imp in &program.imports {
            let segments = import_segments(imp);
            let key = segments.join("::");
            match self.resolve_import(dir, &segments) {
                Resolution::File(resolved) => {
                    let target = fs::canonicalize(&resolved).map_err(|e| {
                        format!("cannot access module `{key}` (`{}`): {e}", resolved.display())
                    })?;
                    if !self.done.contains(&target) {
                        let unit = self.load_module(segments.clone(), &target)?;
                        self.deps.push(unit);
                    }
                    imports.push(ResolvedImport {
                        path: segments,
                        file: target,
                        kind: imp.kind.clone(),
                        host: false,
                        embedded: false,
                    });
                }
                // An embedded stdlib signature module (spec §18.4): the `.pra` source is compiled in,
                // never on disk, so parse it directly. Embedded modules declare no imports, so they
                // can never form a cycle; dedup is by joined path (the synthetic `file` is not real).
                Resolution::Embedded { path } => {
                    if !self.done_embedded.contains(&key) {
                        let src = crate::stdlib::get_module_source(&key)
                            .expect("embedded stdlib source present at resolution time");
                        let program = parse(src).map_err(|errs| {
                            let details = errs.iter().map(|e| e.to_string()).collect::<Vec<_>>().join(", ");
                            format!("embedded stdlib module `{key}` failed to parse: {details}")
                        })?;
                        let file = embedded_file(&path);
                        let unit = ModuleUnit { path, file, program, imports: Vec::new() };
                        self.deps.push(unit);
                        self.done_embedded.insert(key.clone());
                    }
                    imports.push(ResolvedImport {
                        path: segments.clone(),
                        file: embedded_file(&segments),
                        kind: imp.kind.clone(),
                        host: false,
                        embedded: true,
                    });
                }
                // A Rust-hosted stdlib namespace (spec §18): no file on disk, no dependency.
                Resolution::Host { path } => {
                    imports.push(ResolvedImport {
                        path,
                        file: PathBuf::new(),
                        kind: imp.kind.clone(),
                        host: true,
                        embedded: false,
                    });
                }
                Resolution::None => {
                    return Err(format!(
                        "module `{key}` not found relative to `{}` (tried `{}.pra` and `{}/main.pra`)",
                        dir.display(),
                        segments.join("/"),
                        segments.join("/")
                    ));
                }
            }
        }
        self.stack.pop();
        self.done.insert(file.clone());

        Ok(ModuleUnit { path, file, program, imports })
    }

    /// Resolve one import path: (1) an embedded stdlib signature source, (2) a registered Rust-hosted
    /// namespace, (3) a filesystem file relative to the importing module, else `None`
    /// (spec §18.4 / §18 / §15.3). Registered stdlib paths take precedence over local files — like
    /// Rust's `std`, a stdlib module name is reserved (a local `linalg.pra` cannot shadow `import linalg`).
    fn resolve_import(&self, dir: &Path, segments: &[String]) -> Resolution {
        let key = segments.join("::");
        if crate::stdlib::get_module_source(&key).is_some() {
            Resolution::Embedded { path: segments.to_vec() }
        } else if crate::stdlib::has_namespace(&key) {
            Resolution::Host { path: segments.to_vec() }
        } else if let Some(file) = resolve(dir, segments) {
            Resolution::File(file)
        } else {
            Resolution::None
        }
    }

    /// Build the cycle error message, e.g. `import cycle: a -> a::b -> a`.
    fn cycle_error(&self, idx: usize) -> String {
        let mut chain: Vec<String> = self.stack[idx..]
            .iter()
            .map(|(p, f)| display_path(p, f))
            .collect();
        chain.push(display_path(&self.stack[idx].0, &self.stack[idx].1));
        format!("import cycle: {}", chain.join(" -> "))
    }
}

/// Module path segments (spec §15.3 file mapping: `a::b::c` → `[a, b, c]`).
fn import_segments(imp: &Import) -> Vec<String> {
    match &imp.kind {
        ImportKind::Namespace { path, .. } | ImportKind::From { path, .. } => {
            path.iter().map(|s| s.value.clone()).collect()
        }
    }
}

/// Candidate files tried in order: `<dir>/a/b/c.pra`, `<dir>/a/b/c/main.pra` (spec §15.3).
fn resolve(dir: &Path, segments: &[String]) -> Option<PathBuf> {
    let base = segments.iter().fold(dir.to_path_buf(), |p, s| p.join(s));
    let file = base.with_extension("pra");
    if file.is_file() {
        Some(file)
    } else {
        let main = base.join("main.pra");
        main.is_file().then_some(main)
    }
}

/// One resolved import target (spec §15.3 / §18.4 / §18): a file, an embedded stdlib signature
/// source, a Rust-hosted namespace, or nothing.
enum Resolution {
    File(PathBuf),
    Embedded { path: Vec<String> },
    Host { path: Vec<String> },
    None,
}

/// Synthetic marker path for an embedded stdlib module, e.g. `<stdlib>/linalg` (spec §18.4).
/// Distinct from any real file path so the module loader and diagnostics can recognise it.
pub(crate) fn embedded_file(path: &[String]) -> PathBuf {
    PathBuf::from(format!("<stdlib>/{}", path.join("::")))
}

/// Module display name: the file path for the root module (empty path), otherwise the `::`-joined path.
fn display_path(path: &[String], file: &Path) -> String {
    if path.is_empty() {
        file.to_string_lossy().into_owned()
    } else {
        path.join("::")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Unique temp directory, cleaned up on Drop, so parallel tests never interfere with each other.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> TempDir {
            static COUNTER: AtomicUsize = AtomicUsize::new(0);
            let path = std::env::temp_dir().join(format!(
                "prima-module-{}-{}-{}",
                std::process::id(),
                tag,
                COUNTER.fetch_add(1, Ordering::SeqCst),
            ));
            fs::create_dir_all(&path).unwrap();
            TempDir(fs::canonicalize(&path).unwrap())
        }

        fn dir(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn write(dir: &Path, rel: &str, content: &str) -> PathBuf {
        let path = dir.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(&path, content).unwrap();
        path
    }

    fn abs(p: &Path) -> PathBuf {
        fs::canonicalize(p).unwrap()
    }

    #[test]
    fn single_import_resolution() {
        let tmp = TempDir::new("single");
        let root = write(tmp.dir(), "main.pra", "import a;\nx = 1;\n");
        write(tmp.dir(), "a.pra", "y = 2;\n");

        let g = ModuleGraph::load(&root).unwrap();
        assert!(g.root.path.is_empty());
        assert_eq!(g.root.imports.len(), 1);
        let imp = &g.root.imports[0];
        assert_eq!(imp.path, ["a".to_string()]);
        assert_eq!(imp.file, abs(&tmp.dir().join("a.pra")));
        assert_eq!(g.deps.len(), 1);
        assert_eq!(g.deps[0].path, ["a".to_string()]);
        assert_eq!(g.deps[0].file, abs(&tmp.dir().join("a.pra")));
    }

    #[test]
    fn directory_module_resolution() {
        let tmp = TempDir::new("dir-module");
        let root = write(tmp.dir(), "main.pra", "import util\n");
        write(tmp.dir(), "util/main.pra", "z = 3\n");

        let g = ModuleGraph::load(&root).unwrap();
        assert_eq!(g.deps.len(), 1);
        assert_eq!(g.deps[0].path, ["util".to_string()]);
        assert_eq!(g.deps[0].file, abs(&tmp.dir().join("util/main.pra")));
        assert_eq!(g.root.imports[0].file, abs(&tmp.dir().join("util/main.pra")));
    }

    #[test]
    fn nested_path_resolution() {
        let tmp = TempDir::new("nested");
        let root = write(tmp.dir(), "main.pra", "import linalg::fft\n");
        write(tmp.dir(), "linalg/fft.pra", "w = 4\n");

        let g = ModuleGraph::load(&root).unwrap();
        assert_eq!(g.deps.len(), 1);
        assert_eq!(g.deps[0].path, ["linalg".to_string(), "fft".to_string()]);
        assert_eq!(g.deps[0].file, abs(&tmp.dir().join("linalg/fft.pra")));
        assert_eq!(g.root.imports[0].path, ["linalg".to_string(), "fft".to_string()]);
    }

    #[test]
    fn missing_file_error() {
        let tmp = TempDir::new("missing");
        let root = write(tmp.dir(), "main.pra", "import nope\n");

        let err = ModuleGraph::load(&root).unwrap_err();
        assert!(err.contains("nope"), "unexpected error: {err}");
        assert!(err.contains("nope.pra"), "unexpected error: {err}");
    }

    #[test]
    fn cycle_detection_error() {
        let tmp = TempDir::new("cycle");
        let root = write(tmp.dir(), "main.pra", "import a\n");
        write(tmp.dir(), "a.pra", "import b\n");
        write(tmp.dir(), "b.pra", "import a\n");

        let err = ModuleGraph::load(&root).unwrap_err();
        assert!(err.contains("import cycle"), "unexpected error: {err}");
        assert!(err.contains("a"), "unexpected error: {err}");
        assert!(err.contains("b"), "unexpected error: {err}");
    }

    #[test]
    fn dependency_ordering_and_dedup() {
        let tmp = TempDir::new("ordering");
        let root = write(tmp.dir(), "main.pra", "import a;\nimport b;\n");
        write(tmp.dir(), "a.pra", "import c;\n");
        write(tmp.dir(), "b.pra", "import c;\n");
        write(tmp.dir(), "c.pra", "");

        let g = ModuleGraph::load(&root).unwrap();
        assert_eq!(g.deps.len(), 3, "shared dependency `c` must appear once");
        let pos = |name: &str| g.deps.iter().position(|d| d.path == [name.to_string()]).unwrap();
        let (ia, ib, ic) = (pos("a"), pos("b"), pos("c"));
        assert!(ic < ia, "`c` must precede importer `a`: {:?}", g.deps);
        assert!(ic < ib, "`c` must precede importer `b`: {:?}", g.deps);
    }

    #[test]
    fn host_namespace_import_resolves_without_file() {
        // The registry is a process-global `OnceLock`; use a uniquely-named namespace and do not
        // assume any registration ordering across tests.
        crate::stdlib::register_namespace("testhost_x", std::collections::HashMap::new());
        let tmp = TempDir::new("host");
        let root = write(tmp.dir(), "main.pra", "import testhost_x\n");

        let g = ModuleGraph::load(&root).unwrap();
        assert_eq!(g.root.imports.len(), 1);
        let imp = &g.root.imports[0];
        assert!(imp.host, "host import must be flagged, got {imp:?}");
        assert!(!imp.embedded, "host import must not be flagged as embedded, got {imp:?}");
        assert!(imp.file.as_os_str().is_empty(), "host import must not map to a file");
        assert_eq!(g.deps.len(), 0, "host import must not load a file dependency");
    }

    #[test]
    fn embedded_source_import_resolves_as_dep() {
        // The source registry is global; use a unique module name for this test.
        crate::stdlib::register_module_source("testem", "@builtin pub fn answer() -> Integer;");
        let tmp = TempDir::new("embedded");
        let root = write(tmp.dir(), "main.pra", "import testem\n");

        let g = ModuleGraph::load(&root).unwrap();
        assert_eq!(g.root.imports.len(), 1);
        let imp = &g.root.imports[0];
        assert!(imp.embedded, "embedded import must be flagged, got {imp:?}");
        assert!(!imp.host, "embedded import must not be flagged as host, got {imp:?}");
        assert_eq!(imp.file, embedded_file(&["testem".to_string()]), "embedded import must carry the synthetic marker");
        assert_eq!(g.deps.len(), 1, "embedded import must produce a dependency unit");
        let dep = &g.deps[0];
        assert_eq!(dep.path, ["testem".to_string()]);
        assert_eq!(dep.file, embedded_file(&["testem".to_string()]));
        assert!(dep.imports.is_empty(), "embedded modules declare no imports");
        assert_eq!(dep.program.stmts.len(), 1, "embedded source must parse to its `@builtin` declaration");
    }

    #[test]
    fn embedded_source_import_dedups() {
        crate::stdlib::register_module_source("testem2", "@builtin pub fn f() -> Integer;");
        let tmp = TempDir::new("embedded-dedup");
        let root = write(tmp.dir(), "main.pra", "import testem2;\nimport testem2;\n");

        let g = ModuleGraph::load(&root).unwrap();
        assert_eq!(g.deps.len(), 1, "duplicate embedded imports must dedup to a single dependency");
        assert!(g.root.imports.iter().all(|i| i.embedded));
    }
}
