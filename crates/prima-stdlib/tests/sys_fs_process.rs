//! `sys::process`, `sys::fs`, and `sys::term` integration tests (spec §18.6).
//!
//! Filesystem tests use a self-cleaning temporary directory. `tempfile` is not a dev-dependency of
//! `prima-stdlib`, so a minimal RAII helper over `std::env::temp_dir` is used instead.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use prima_core::{Value, ValueKey};
use prima_runtime::Evaluator;

/// Evaluate an in-memory program that imports Rust-hosted stdlib namespaces (spec §18).
fn eval(src: &str) -> Value {
    prima_stdlib::init();
    Evaluator::new().eval_value(src).expect("eval failed")
}

/// Unwrap a `Value::Result(Ok(v))`; panic on `Err`/non-Result.
fn ok_of(v: Value) -> Value {
    match v {
        Value::Result(Ok(inner)) => *inner,
        other => panic!("expected Result<Ok>, got {other:?}"),
    }
}

/// Expect a `Value::Result(Err(msg))` and return the message.
fn err_of(v: Value) -> String {
    match v {
        Value::Result(Err(msg)) => (*msg).clone(),
        other => panic!("expected Result<Err>, got {other:?}"),
    }
}

/// The `i64` payload of a `Value::Number`, or a panic.
fn int_of(v: &Value) -> i64 {
    match v {
        Value::Number(n) => n
            .as_i64()
            .unwrap_or_else(|| panic!("expected an integral Number, got {n}")),
        other => panic!("expected Number, got {other:?}"),
    }
}

/// Escape a path so it can be embedded in a Prima `"..."` literal (spec §18.1): Windows
/// backslashes must be doubled and quotes escaped.
fn primed_str(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// A self-cleaning temporary directory.
struct TempDir(std::path::PathBuf);

impl TempDir {
    fn new(tag: &str) -> TempDir {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let path = std::env::temp_dir().join(format!(
            "prima-sys-{tag}-{}-{nanos}-{n}",
            std::process::id()
        ));
        std::fs::create_dir_all(&path).expect("create temp dir");
        TempDir(path)
    }

    /// The directory path as a Prima string literal body (escaped).
    fn prima(&self) -> String {
        primed_str(&self.0.to_string_lossy())
    }

    /// The path of `name` inside the directory, as a Prima string literal body (escaped).
    fn prima_child(&self, name: &str) -> String {
        primed_str(&self.0.join(name).to_string_lossy())
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

// ——— sys::fs ———

#[test]
fn fs_exists_and_kind_predicates() {
    let dir = TempDir::new("kind");
    std::fs::write(dir.0.join("file.txt"), b"hi").expect("write file");
    std::fs::create_dir(dir.0.join("sub")).expect("create dir");

    let file = dir.prima_child("file.txt");
    let sub = dir.prima_child("sub");
    let missing = dir.prima_child("missing");

    assert_eq!(
        eval(&format!("import sys::fs;\nsys::fs::exists(\"{file}\")")),
        Value::Bool(true)
    );
    assert_eq!(
        eval(&format!("import sys::fs;\nsys::fs::is_file(\"{file}\")")),
        Value::Bool(true)
    );
    assert_eq!(
        eval(&format!("import sys::fs;\nsys::fs::is_dir(\"{file}\")")),
        Value::Bool(false)
    );
    assert_eq!(
        eval(&format!("import sys::fs;\nsys::fs::is_dir(\"{sub}\")")),
        Value::Bool(true)
    );
    assert_eq!(
        eval(&format!("import sys::fs;\nsys::fs::exists(\"{missing}\")")),
        Value::Bool(false)
    );
}

#[test]
fn fs_size_reports_byte_length() {
    let dir = TempDir::new("size");
    let content = "hello world";
    std::fs::write(dir.0.join("f.txt"), content).expect("write file");
    let f = dir.prima_child("f.txt");

    let v = ok_of(eval(&format!("import sys::fs;\nsys::fs::size(\"{f}\")")));
    assert_eq!(int_of(&v), content.len() as i64);

    let missing = dir.prima_child("nope.txt");
    let msg = err_of(eval(&format!(
        "import sys::fs;\nsys::fs::size(\"{missing}\")"
    )));
    assert!(!msg.is_empty());
}

#[test]
fn fs_read_dir_is_sorted() {
    let dir = TempDir::new("readdir");
    for name in ["b.txt", "a.txt", "c.txt"] {
        std::fs::write(dir.0.join(name), b"x").expect("write file");
    }
    std::fs::create_dir(dir.0.join("d")).expect("create dir");

    let v = ok_of(eval(&format!(
        "import sys::fs;\nsys::fs::read_dir(\"{}\")",
        dir.prima()
    )));
    match v {
        Value::Array(items) => {
            let names: Vec<String> = items.with(|xs| {
                xs.iter()
                    .map(|x| match x {
                        Value::String(s) => s.to_string(),
                        other => panic!("expected a String entry, got {other:?}"),
                    })
                    .collect()
            });
            assert_eq!(names, vec!["a.txt", "b.txt", "c.txt", "d"]);
        }
        other => panic!("expected Array, got {other:?}"),
    }

    let missing = dir.prima_child("nope");
    let msg = err_of(eval(&format!(
        "import sys::fs;\nsys::fs::read_dir(\"{missing}\")"
    )));
    assert!(!msg.is_empty());
}

#[test]
fn fs_metadata_reports_size_and_kind() {
    let dir = TempDir::new("metadata");
    let content = b"abcd";
    std::fs::write(dir.0.join("f.bin"), content).expect("write file");
    let f = dir.prima_child("f.bin");

    let v = ok_of(eval(&format!(
        "import sys::fs;\nsys::fs::metadata(\"{f}\")"
    )));
    match v {
        Value::Dict(d) => {
            let size = d
                .get(&ValueKey::Str("size".into()))
                .expect("`size` key present");
            assert_eq!(int_of(size), content.len() as i64);
            assert_eq!(
                d.get(&ValueKey::Str("is_file".into())),
                Some(&Value::Bool(true))
            );
            assert_eq!(
                d.get(&ValueKey::Str("is_dir".into())),
                Some(&Value::Bool(false))
            );
            // `modified` is optional (omitted when the platform cannot report it); when present it
            // must be a non-negative unix-seconds number.
            if let Some(modified) = d.get(&ValueKey::Str("modified".into())) {
                match modified {
                    Value::Number(n) => assert!(n.to_f64_lossy() > 0.0),
                    other => panic!("`modified` must be an F64, got {other:?}"),
                }
            }
        }
        other => panic!("expected Dict, got {other:?}"),
    }
}

// ——— sys::process ———

#[test]
fn process_run_captures_stdout() {
    let v = ok_of(eval(
        "import sys::process;\nsys::process::run(\"echo prima\")",
    ));
    match v {
        Value::String(s) => assert_eq!(s.trim(), "prima"),
        other => panic!("expected String, got {other:?}"),
    }
}

#[test]
fn process_run_nonzero_is_err_with_code() {
    let cmd = if cfg!(windows) { "exit /b 3" } else { "exit 3" };
    let msg = err_of(eval(&format!(
        "import sys::process;\nsys::process::run(\"{cmd}\")"
    )));
    assert!(msg.contains('3'), "error should carry the exit code: {msg}");
}

#[test]
fn process_exit_code_zero_and_nonzero() {
    let zero = ok_of(eval(
        "import sys::process;\nsys::process::exit_code(\"echo ok\")",
    ));
    assert_eq!(int_of(&zero), 0);

    let cmd = if cfg!(windows) { "exit /b 7" } else { "exit 7" };
    let seven = ok_of(eval(&format!(
        "import sys::process;\nsys::process::exit_code(\"{cmd}\")"
    )));
    assert_eq!(int_of(&seven), 7);
}

// ——— sys::term ———

#[test]
fn term_size_has_positive_rows_and_cols() {
    let v = eval("import sys::term;\nsys::term::size()");
    match v {
        Value::Dict(d) => {
            for key in ["rows", "cols"] {
                match d.get(&ValueKey::Str(key.into())) {
                    Some(value) => assert!(int_of(value) > 0, "`{key}` must be positive"),
                    other => panic!("`{key}` missing, got {other:?}"),
                }
            }
        }
        other => panic!("expected Dict, got {other:?}"),
    }
}

#[test]
fn term_is_tty_returns_bool() {
    assert!(matches!(
        eval("import sys::term;\nsys::term::is_tty()"),
        Value::Bool(_)
    ));
}
