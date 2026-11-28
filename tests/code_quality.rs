//! Quality regression guards (CI): forbid reintroducing catch-all error paths or unfinished
//! placeholders, and keep the hot-path error type compact.

use std::fs;
use std::path::{Path, PathBuf};

/// Patterns that must not reappear in production sources. Each entry is
/// `(needle, replacement advice)`.
const FORBIDDEN: &[(&str, &str)] = &[
    (
        "RuntimeError::Message",
        "use a typed `RuntimeError` variant (or the `Type` category)",
    ),
    (
        "RuntimeError::Coded",
        "use `CompileDiagnostic` or a dedicated runtime variant",
    ),
    ("CoreError::Other", "use a typed `CoreError` variant"),
    (
        "fn coded(",
        "add a dedicated runtime-error variant instead of a code+message carrier",
    ),
    ("dbg!", "remove the debug macro before merging"),
    ("todo!", "implement the case or return a typed error"),
    (
        "unimplemented!",
        "implement the case or return a typed error",
    ),
];

fn rust_sources(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let skip = matches!(
                path.file_name().and_then(|name| name.to_str()),
                Some("target" | "tests" | "benches" | "examples")
            );
            if !skip {
                rust_sources(&path, out);
            }
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

#[test]
fn production_sources_have_no_catch_alls_or_placeholders() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    rust_sources(&root.join("src"), &mut files);
    rust_sources(&root.join("crates"), &mut files);
    files.sort();

    let mut hits = Vec::new();
    for file in &files {
        let text = fs::read_to_string(file).expect("read Rust source");
        for (line_no, line) in text.lines().enumerate() {
            for (needle, advice) in FORBIDDEN {
                if line.contains(needle) {
                    let rel = file.strip_prefix(root).unwrap_or(file);
                    hits.push(format!(
                        "{}:{}: `{needle}` ({advice})",
                        rel.display(),
                        line_no + 1
                    ));
                }
            }
        }
    }
    assert!(
        hits.is_empty(),
        "quality guards failed:\n{}",
        hits.join("\n")
    );
}

#[test]
fn runtime_error_payload_stays_compact() {
    // `Result<Value, RuntimeError>` is returned from every evaluation step; a growing error
    // payload would inflate the success path.
    assert!(
        std::mem::size_of::<prima_runtime::RuntimeError>() <= 64,
        "RuntimeError grew to {} bytes",
        std::mem::size_of::<prima_runtime::RuntimeError>()
    );
}
