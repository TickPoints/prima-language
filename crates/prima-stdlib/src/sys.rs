//! `sys` module (spec §18.2 / appendix B.5): cross-platform path helpers (`sys::path`),
//! environment access (`sys::env`), and OS/platform queries (`sys::os`).

use std::collections::HashMap;
use std::path::Path;

use prima_core::Value;
use prima_runtime::stdlib::register_namespace;
use prima_runtime::{Evaluator, Function, NamespaceItem, RuntimeError};

type Native = fn(&mut Evaluator, &[Value]) -> Result<Value, RuntimeError>;

fn native(name: &'static str, call: Native) -> NamespaceItem {
    NamespaceItem::Func(Function::Native { name, call })
}

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

/// Register `sys::path`, `sys::env`, and `sys::os` (spec §18.2).
pub fn register() {
    let mut path = HashMap::new();
    path.insert("join".into(), native("sys::path::join", path_join));
    path.insert("file_name".into(), native("sys::path::file_name", path_file_name));
    path.insert("extension".into(), native("sys::path::extension", path_extension));
    path.insert("parent".into(), native("sys::path::parent", path_parent));
    path.insert("is_absolute".into(), native("sys::path::is_absolute", path_is_absolute));
    path.insert("canonicalize".into(), native("sys::path::canonicalize", path_canonicalize));
    register_namespace("sys::path", path);

    let mut env = HashMap::new();
    env.insert("home_dir".into(), native("sys::env::home_dir", env_home_dir));
    env.insert("get".into(), native("sys::env::get", env_get));
    env.insert("args".into(), native("sys::env::args", env_args));
    env.insert("current_dir".into(), native("sys::env::current_dir", env_current_dir));
    register_namespace("sys::env", env);

    let mut os = HashMap::new();
    os.insert("name".into(), native("sys::os::name", os_name));
    os.insert("arch".into(), native("sys::os::arch", os_arch));
    os.insert("exit".into(), native("sys::os::exit", os_exit));
    register_namespace("sys::os", os);
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
