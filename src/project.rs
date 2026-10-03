//! Project-root discovery for the CLI (spec §20 project layout).
//!
//! A Prima project is a directory containing `prima.toml` (the manifest marker) and/or the
//! canonical entry `src/main.pra`. Commands that accept an optional file resolve it against the
//! nearest ancestor project root, so `prima run` works from any subdirectory.

use std::path::{Path, PathBuf};

use anyhow::{Context, bail};

/// Project manifest marker (spec §20).
pub const MANIFEST: &str = "prima.toml";
/// Canonical entry path relative to the project root (spec §20).
pub const ENTRY_REL: &str = "src/main.pra";
/// Directory scanned by `prima test` inside a project (spec §20).
pub const TEST_DIR: &str = "src";
/// Fallback test directory outside a project (keeps the toolchain repo's own fixtures runnable).
pub const FALLBACK_TEST_DIR: &str = "examples";

/// Walk up from `start` (a directory or a file path) to the nearest ancestor that looks like a
/// Prima project. A directory qualifies when it holds `prima.toml` or `src/main.pra`.
pub fn find_root(start: &Path) -> Option<PathBuf> {
    let mut dir: &Path = if start.is_dir() {
        start
    } else {
        start.parent()?
    };
    loop {
        if dir.join(MANIFEST).is_file() || dir.join(ENTRY_REL).is_file() {
            return Some(dir.to_path_buf());
        }
        dir = dir.parent()?;
    }
}

/// Resolve an explicit path or, when omitted, the entry file of the nearest project root.
///
/// The error message points the user at `prima new` when no project is found, so a bare
/// `prima run` outside a project fails helpfully instead of with a usage error.
pub fn resolve_entry(explicit: Option<&Path>, cwd: &Path) -> anyhow::Result<PathBuf> {
    if let Some(path) = explicit {
        return Ok(path.to_path_buf());
    }
    let root = find_root(cwd).with_context(|| {
        format!(
            "not inside a Prima project (no `{MANIFEST}` or `{ENTRY_REL}` found above {}); \
             pass a file or run `prima new <name>`",
            cwd.display()
        )
    })?;
    let entry = root.join(ENTRY_REL);
    if !entry.is_file() {
        bail!(
            "project root {} has no `{ENTRY_REL}` (spec §20)",
            root.display()
        );
    }
    Ok(entry)
}

/// The directory `prima test` scans when no path is given: the project `src/` when inside a
/// project, otherwise the toolchain's `examples/` fallback.
pub fn resolve_test_dir(cwd: &Path) -> PathBuf {
    match find_root(cwd) {
        Some(root) => root.join(TEST_DIR),
        None => PathBuf::from(FALLBACK_TEST_DIR),
    }
}
