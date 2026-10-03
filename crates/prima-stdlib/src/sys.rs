//! `sys` module (spec §18.2 / appendix B.5 and the v2.2 expansion §18.6): cross-platform path
//! helpers (`sys::path`), environment access (`sys::env`), OS/platform queries (`sys::os`), process
//! execution (`sys::process`), filesystem metadata (`sys::fs`), and terminal queries (`sys::term`).
//!
//! # Trust boundary
//!
//! `sys::process::run` / `sys::process::exit_code` execute arbitrary commands through the platform
//! shell, and the `sys::fs` functions read arbitrary paths, all with the privileges of the host
//! process. Running untrusted `.pra` code therefore grants it that access; only run code you trust.

use std::collections::HashMap;
use std::fs;
use std::io::IsTerminal;
use std::path::Path;
use std::process::{Command, ExitStatus};
use std::time::UNIX_EPOCH;

use num_bigint::BigInt;
use prima_core::{Number, Real, Value, ValueKey};
use prima_runtime::builtin;
use prima_runtime::{Evaluator, RuntimeError};

fn arity(args: &[Value], n: usize, fname: &str) -> Result<(), RuntimeError> {
    if args.len() == n {
        Ok(())
    } else {
        Err(RuntimeError::Message(format!(
            "`{fname}` expects {n} argument(s), got {}",
            args.len()
        )))
    }
}

fn string_arg(args: &[Value], i: usize, fname: &str) -> Result<String, RuntimeError> {
    match args.get(i) {
        Some(Value::String(s)) => Ok(s.to_string()),
        Some(other) => Err(RuntimeError::Type(format!(
            "`{fname}` argument {i} must be a string, got {other:?}"
        ))),
        None => Err(RuntimeError::Message(format!(
            "`{fname}` missing argument {i}"
        ))),
    }
}

/// Wrap a success value in `Result<_, String>` (spec §16.2).
fn ok(v: Value) -> Value {
    Value::Result(Ok(Box::new(v)))
}

/// Build an error `Result<_, String>` from a message (spec §16.2).
fn err(msg: String) -> Value {
    Value::Result(Err(Box::new(msg)))
}

/// Register the `sys::path`, `sys::env`, `sys::os`, `sys::process`, `sys::fs`, and `sys::term`
/// `@builtin` implementations (spec §18.4 / §18.2 / §18.6). Each `@builtin` declaration in the
/// embedded signature modules binds to the implementation registered under its fully-qualified
/// `<module>::<name>` key (spec §18.4).
pub fn register() {
    // sys::path
    builtin!("sys::path::join", path_join);
    builtin!("sys::path::file_name", path_file_name);
    builtin!("sys::path::extension", path_extension);
    builtin!("sys::path::parent", path_parent);
    builtin!("sys::path::is_absolute", path_is_absolute);
    builtin!("sys::path::canonicalize", path_canonicalize);
    // sys::env
    builtin!("sys::env::home_dir", env_home_dir);
    builtin!("sys::env::get", env_get);
    builtin!("sys::env::args", env_args);
    builtin!("sys::env::current_dir", env_current_dir);
    // sys::os
    builtin!("sys::os::name", os_name);
    builtin!("sys::os::arch", os_arch);
    builtin!("sys::os::exit", os_exit);
    // sys::process
    builtin!("sys::process::run", process_run);
    builtin!("sys::process::exit_code", process_exit_code);
    // sys::fs
    builtin!("sys::fs::exists", fs_exists);
    builtin!("sys::fs::is_file", fs_is_file);
    builtin!("sys::fs::is_dir", fs_is_dir);
    builtin!("sys::fs::size", fs_size);
    builtin!("sys::fs::read_dir", fs_read_dir);
    builtin!("sys::fs::metadata", fs_metadata);
    // sys::term
    builtin!("sys::term::size", term_size);
    builtin!("sys::term::is_tty", term_is_tty);
}

fn path_join(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    arity(args, 2, "sys::path::join")?;
    let a = string_arg(args, 0, "sys::path::join")?;
    let b = string_arg(args, 1, "sys::path::join")?;
    let sep = std::path::MAIN_SEPARATOR;
    Ok(Value::String(format!("{a}{sep}{b}").into()))
}

fn path_file_name(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    arity(args, 1, "sys::path::file_name")?;
    let p = string_arg(args, 0, "sys::path::file_name")?;
    match Path::new(&p).file_name() {
        Some(n) if !n.is_empty() => Ok(Value::Option(Some(Box::new(Value::String(
            n.to_string_lossy().into_owned().into(),
        ))))),
        _ => Ok(Value::Option(None)),
    }
}

fn path_extension(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    arity(args, 1, "sys::path::extension")?;
    let p = string_arg(args, 0, "sys::path::extension")?;
    match Path::new(&p).extension() {
        Some(e) => Ok(Value::Option(Some(Box::new(Value::String(
            e.to_string_lossy().into_owned().into(),
        ))))),
        None => Ok(Value::Option(None)),
    }
}

fn path_parent(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    arity(args, 1, "sys::path::parent")?;
    let p = string_arg(args, 0, "sys::path::parent")?;
    match Path::new(&p).parent() {
        Some(par) => Ok(Value::Option(Some(Box::new(Value::String(
            par.to_string_lossy().into_owned().into(),
        ))))),
        None => Ok(Value::Option(None)),
    }
}

fn path_is_absolute(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    arity(args, 1, "sys::path::is_absolute")?;
    let p = string_arg(args, 0, "sys::path::is_absolute")?;
    Ok(Value::Bool(Path::new(&p).is_absolute()))
}

fn path_canonicalize(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    arity(args, 1, "sys::path::canonicalize")?;
    let p = string_arg(args, 0, "sys::path::canonicalize")?;
    match std::fs::canonicalize(&p) {
        Ok(c) => Ok(Value::Result(Ok(Box::new(Value::String(
            c.to_string_lossy().into_owned().into(),
        ))))),
        Err(e) => Ok(Value::Result(Err(Box::new(format!(
            "cannot canonicalize `{p}`: {e}"
        ))))),
    }
}

fn env_home_dir(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    arity(args, 0, "sys::env::home_dir")?;
    let key = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    Ok(std::env::var_os(key)
        .map(|h| {
            Value::Option(Some(Box::new(Value::String(
                h.to_string_lossy().into_owned().into(),
            ))))
        })
        .unwrap_or(Value::Option(None)))
}

fn env_get(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    arity(args, 1, "sys::env::get")?;
    let name = string_arg(args, 0, "sys::env::get")?;
    match std::env::var(&name) {
        Ok(v) => Ok(Value::Option(Some(Box::new(Value::String(v.into()))))),
        Err(_) => Ok(Value::Option(None)),
    }
}

fn env_args(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    arity(args, 0, "sys::env::args")?;
    Ok(Value::Array(
        std::env::args()
            .skip(1)
            .map(|s| Value::String(s.into()))
            .collect::<Vec<Value>>()
            .into(),
    ))
}

fn env_current_dir(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    arity(args, 0, "sys::env::current_dir")?;
    match std::env::current_dir() {
        Ok(d) => Ok(Value::String(d.to_string_lossy().into_owned().into())),
        // Mirror `input`/`read_line` I/O-error policy: no panic, return an empty string.
        Err(_) => Ok(Value::String(String::new().into())),
    }
}

fn os_name(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    arity(args, 0, "sys::os::name")?;
    Ok(Value::String(std::env::consts::OS.to_string().into()))
}

fn os_arch(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    arity(args, 0, "sys::os::arch")?;
    Ok(Value::String(std::env::consts::ARCH.to_string().into()))
}

fn os_exit(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    let code = match args.first() {
        Some(Value::Number(n)) => n.as_i64().and_then(|v| i32::try_from(v).ok()),
        _ => None,
    }
    .ok_or_else(|| RuntimeError::Type("`sys::os::exit` expects an integer exit code".into()))?;
    std::process::exit(code);
}

// ——— sys::process (spec §18.6) ———

/// Run `cmd` through the platform shell, capturing both streams. `sh -c` on unix, `cmd /C` on
/// Windows, so callers can pass a portable command line (spec §18.6).
fn run_shell(cmd: &str) -> std::io::Result<std::process::Output> {
    if cfg!(windows) {
        Command::new("cmd").arg("/C").arg(cmd).output()
    } else {
        Command::new("sh").arg("-c").arg(cmd).output()
    }
}

/// The portable exit code of a finished command (spec §18.6): the process exit status, or `None`
/// when it has none (terminated by a signal). On unix a signal termination is reported as the
/// conventional `128 + signal`.
fn status_code(status: &ExitStatus) -> Option<i32> {
    if let Some(code) = status.code() {
        Some(code)
    } else {
        #[cfg(unix)]
        {
            use std::os::unix::process::ExitStatusExt;
            status.signal().map(|s| 128 + s)
        }
        #[cfg(not(unix))]
        {
            None
        }
    }
}

fn process_run(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    arity(args, 1, "sys::process::run")?;
    let cmd = string_arg(args, 0, "sys::process::run")?;
    let output = match run_shell(&cmd) {
        Ok(output) => output,
        Err(e) => return Ok(err(format!("cannot run `{cmd}`: {e}"))),
    };
    if output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        return Ok(ok(Value::String(stdout.into())));
    }
    // Non-zero exit: report the code together with the captured stdout and stderr (spec §18.6).
    let code_text = match status_code(&output.status) {
        Some(code) => code.to_string(),
        None => "unknown".to_string(),
    };
    let mut captured = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !stderr.is_empty() {
        captured.push_str(&stderr);
    }
    Ok(err(format!(
        "command `{cmd}` exited with code {code_text}: {}",
        captured.trim_end()
    )))
}

fn process_exit_code(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    arity(args, 1, "sys::process::exit_code")?;
    let cmd = string_arg(args, 0, "sys::process::exit_code")?;
    let output = match run_shell(&cmd) {
        Ok(output) => output,
        Err(e) => return Ok(err(format!("cannot run `{cmd}`: {e}"))),
    };
    match status_code(&output.status) {
        Some(code) => Ok(ok(Value::Number(Number::from(code)))),
        None => Ok(err(format!(
            "command `{cmd}` has no exit code (terminated by a signal)"
        ))),
    }
}

// ——— sys::fs (spec §18.6) ———

fn fs_exists(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    arity(args, 1, "sys::fs::exists")?;
    let path = string_arg(args, 0, "sys::fs::exists")?;
    Ok(Value::Bool(Path::new(&path).exists()))
}

fn fs_is_file(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    arity(args, 1, "sys::fs::is_file")?;
    let path = string_arg(args, 0, "sys::fs::is_file")?;
    Ok(Value::Bool(Path::new(&path).is_file()))
}

fn fs_is_dir(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    arity(args, 1, "sys::fs::is_dir")?;
    let path = string_arg(args, 0, "sys::fs::is_dir")?;
    Ok(Value::Bool(Path::new(&path).is_dir()))
}

fn fs_size(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    arity(args, 1, "sys::fs::size")?;
    let path = string_arg(args, 0, "sys::fs::size")?;
    Ok(match fs::metadata(&path) {
        Ok(md) => ok(Value::Number(Number::Integer(Box::new(BigInt::from(
            md.len(),
        ))))),
        Err(e) => err(format!("cannot stat `{path}`: {e}")),
    })
}

fn fs_read_dir(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    arity(args, 1, "sys::fs::read_dir")?;
    let path = string_arg(args, 0, "sys::fs::read_dir")?;
    let entries = match fs::read_dir(&path) {
        Ok(entries) => entries,
        Err(e) => return Ok(err(format!("cannot read `{path}`: {e}"))),
    };
    let names: std::io::Result<Vec<String>> = entries
        .map(|entry| entry.map(|e| e.file_name().to_string_lossy().into_owned()))
        .collect();
    match names {
        Ok(mut names) => {
            names.sort();
            let items: Vec<Value> = names.into_iter().map(|n| Value::String(n.into())).collect();
            Ok(ok(Value::Array(items.into())))
        }
        Err(e) => Ok(err(format!("cannot read `{path}`: {e}"))),
    }
}

fn fs_metadata(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    arity(args, 1, "sys::fs::metadata")?;
    let path = string_arg(args, 0, "sys::fs::metadata")?;
    let md = match fs::metadata(&path) {
        Ok(md) => md,
        Err(e) => return Ok(err(format!("cannot stat `{path}`: {e}"))),
    };
    let mut map: HashMap<ValueKey, Value> = HashMap::new();
    map.insert(
        ValueKey::Str("size".into()),
        Value::Number(Number::Integer(Box::new(BigInt::from(md.len())))),
    );
    map.insert(ValueKey::Str("is_file".into()), Value::Bool(md.is_file()));
    map.insert(ValueKey::Str("is_dir".into()), Value::Bool(md.is_dir()));
    if let Ok(modified) = md.modified() {
        let secs = match modified.duration_since(UNIX_EPOCH) {
            Ok(d) => d.as_secs_f64(),
            Err(e) => -e.duration().as_secs_f64(),
        };
        map.insert(
            ValueKey::Str("modified".into()),
            Value::Number(Number::Real(Real::F64(secs))),
        );
    }
    Ok(ok(Value::Dict(Box::new(map))))
}

// ——— sys::term (spec §18.6) ———

/// Best-effort terminal size (spec §18.6): when stdout is a TTY, the `LINES`/`COLUMNS` environment
/// variables are used if they parse as positive integers, otherwise the conventional fallback
/// `(24, 80)`. Rust's standard library exposes no portable window-size query, so environment
/// variables are the portable detection path.
fn detected_terminal_size() -> (u64, u64) {
    const DEFAULT: (u64, u64) = (24, 80);
    if !std::io::stdout().is_terminal() {
        return DEFAULT;
    }
    let parse = |name: &str| {
        std::env::var(name)
            .ok()?
            .trim()
            .parse::<u64>()
            .ok()
            .filter(|n| *n > 0)
    };
    match (parse("LINES"), parse("COLUMNS")) {
        (Some(rows), Some(cols)) => (rows, cols),
        _ => DEFAULT,
    }
}

fn term_size(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    arity(args, 0, "sys::term::size")?;
    let (rows, cols) = detected_terminal_size();
    let mut map: HashMap<ValueKey, Value> = HashMap::new();
    map.insert(
        ValueKey::Str("rows".into()),
        Value::Number(Number::Integer(Box::new(BigInt::from(rows)))),
    );
    map.insert(
        ValueKey::Str("cols".into()),
        Value::Number(Number::Integer(Box::new(BigInt::from(cols)))),
    );
    Ok(Value::Dict(Box::new(map)))
}

fn term_is_tty(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    arity(args, 0, "sys::term::is_tty")?;
    Ok(Value::Bool(std::io::stdout().is_terminal()))
}
