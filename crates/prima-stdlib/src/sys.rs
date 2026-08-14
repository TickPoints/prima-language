//! `sys` module (spec §18.2 / appendix B.5): cross-platform path helpers (`sys::path`),
//! environment access (`sys::env`), and OS/platform queries (`sys::os`).

use std::path::Path;

use prima_core::Value;
use prima_runtime::stdlib::register_impl;
use prima_runtime::{Evaluator, RuntimeError};

fn arity(args: &[Value], n: usize, fname: &str) -> Result<(), RuntimeError> {
    if args.len() == n {
        Ok(())
    } else {
        Err(RuntimeError::Message(format!("`{fname}` expects {n} argument(s), got {}", args.len())))
    }
}

fn string_arg(args: &[Value], i: usize, fname: &str) -> Result<String, RuntimeError> {
    match args.get(i) {
        Some(Value::String(s)) => Ok(s.clone()),
        Some(other) => Err(RuntimeError::Type(format!(
            "`{fname}` argument {i} must be a string, got {other:?}"
        ))),
        None => Err(RuntimeError::Message(format!("`{fname}` missing argument {i}"))),
    }
}

/// Register the `sys::path`, `sys::env`, and `sys::os` `@builtin` implementations (spec §18.4 /
/// §18.2). Each `@builtin` declaration in the embedded `sys_path.pra` / `sys_env.pra` / `sys_os.pra`
/// signature modules binds to the implementation registered under its fully-qualified
/// `sys::path::<name>` / `sys::env::<name>` / `sys::os::<name>` key (spec §18.4).
pub fn register() {
    // sys::path
    register_impl("sys::path::join", path_join);
    register_impl("sys::path::file_name", path_file_name);
    register_impl("sys::path::extension", path_extension);
    register_impl("sys::path::parent", path_parent);
    register_impl("sys::path::is_absolute", path_is_absolute);
    register_impl("sys::path::canonicalize", path_canonicalize);
    // sys::env
    register_impl("sys::env::home_dir", env_home_dir);
    register_impl("sys::env::get", env_get);
    register_impl("sys::env::args", env_args);
    register_impl("sys::env::current_dir", env_current_dir);
    // sys::os
    register_impl("sys::os::name", os_name);
    register_impl("sys::os::arch", os_arch);
    register_impl("sys::os::exit", os_exit);
}

fn path_join(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    arity(args, 2, "sys::path::join")?;
    let a = string_arg(args, 0, "sys::path::join")?;
    let b = string_arg(args, 1, "sys::path::join")?;
    let sep = std::path::MAIN_SEPARATOR;
    Ok(Value::String(format!("{a}{sep}{b}")))
}

fn path_file_name(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    arity(args, 1, "sys::path::file_name")?;
    let p = string_arg(args, 0, "sys::path::file_name")?;
    match Path::new(&p).file_name() {
        Some(n) if !n.is_empty() => {
            Ok(Value::Option(Some(Box::new(Value::String(n.to_string_lossy().into_owned())))))
        }
        _ => Ok(Value::Option(None)),
    }
}

fn path_extension(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    arity(args, 1, "sys::path::extension")?;
    let p = string_arg(args, 0, "sys::path::extension")?;
    match Path::new(&p).extension() {
        Some(e) => Ok(Value::Option(Some(Box::new(Value::String(e.to_string_lossy().into_owned()))))),
        None => Ok(Value::Option(None)),
    }
}

fn path_parent(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    arity(args, 1, "sys::path::parent")?;
    let p = string_arg(args, 0, "sys::path::parent")?;
    match Path::new(&p).parent() {
        Some(par) => Ok(Value::Option(Some(Box::new(Value::String(par.to_string_lossy().into_owned()))))),
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
        Ok(c) => Ok(Value::Result(Ok(Box::new(Value::String(c.to_string_lossy().into_owned()))))),
        Err(e) => Ok(Value::Result(Err(format!("cannot canonicalize `{p}`: {e}")))),
    }
}

fn env_home_dir(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    arity(args, 0, "sys::env::home_dir")?;
    let key = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    Ok(std::env::var_os(key)
        .map(|h| Value::Option(Some(Box::new(Value::String(h.to_string_lossy().into_owned())))))
        .unwrap_or(Value::Option(None)))
}

fn env_get(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    arity(args, 1, "sys::env::get")?;
    let name = string_arg(args, 0, "sys::env::get")?;
    match std::env::var(&name) {
        Ok(v) => Ok(Value::Option(Some(Box::new(Value::String(v))))),
        Err(_) => Ok(Value::Option(None)),
    }
}

fn env_args(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    arity(args, 0, "sys::env::args")?;
    Ok(Value::Array(std::env::args().skip(1).map(Value::String).collect()))
}

fn env_current_dir(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    arity(args, 0, "sys::env::current_dir")?;
    match std::env::current_dir() {
        Ok(d) => Ok(Value::String(d.to_string_lossy().into_owned())),
        // Mirror `input`/`read_line` I/O-error policy: no panic, return an empty string.
        Err(_) => Ok(Value::String(String::new())),
    }
}

fn os_name(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    arity(args, 0, "sys::os::name")?;
    Ok(Value::String(std::env::consts::OS.to_string()))
}

fn os_arch(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    arity(args, 0, "sys::os::arch")?;
    Ok(Value::String(std::env::consts::ARCH.to_string()))
}

fn os_exit(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    let code = match args.first() {
        Some(Value::Number(n)) => n.as_i64().and_then(|v| i32::try_from(v).ok()),
        _ => None,
    }
    .ok_or_else(|| RuntimeError::Type("`sys::os::exit` expects an integer exit code".into()))?;
    std::process::exit(code);
}
