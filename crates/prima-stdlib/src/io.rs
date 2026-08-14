//! `io` module (spec §18 / appendix B.2): file I/O, JSON serialization, and CSV serialization.
//!
//! File functions return `Result` (spec §16.2): `Ok(..)` carries the produced value, `Err(msg)`
//! carries an error message string — the `?` operator / `match` unwraps them (§15.5).
//! JSON uses `serde_json` for both directions; CSV is hand-rolled RFC 4180-ish (comma delimiter,
//! double-quoted fields with `""` escapes and embedded commas/newlines).

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use prima_core::{Number, Value, ValueKey};
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

/// Extract a `&[Value]` from a `Value::Array` argument (type error otherwise).
fn array_arg<'a>(v: &'a Value, fname: &str, i: usize) -> Result<&'a [Value], RuntimeError> {
    match v {
        Value::Array(items) => Ok(items),
        other => Err(RuntimeError::Type(format!(
            "`{fname}` argument {i} must be an array, got {other:?}"
        ))),
    }
}

/// A `String`-typed element of an array (type error otherwise).
fn array_string(elem: &Value, fname: &str, what: &str) -> Result<String, RuntimeError> {
    match elem {
        Value::String(s) => Ok(s.clone()),
        other => Err(RuntimeError::Type(format!(
            "`{fname}` {what} must be a string, got {other:?}"
        ))),
    }
}

fn ok(v: Value) -> Value {
    Value::Result(Ok(Box::new(v)))
}

fn err(msg: String) -> Value {
    Value::Result(Err(msg))
}

/// Register the `io` `@builtin` implementations (spec §18.4 / appendix B.2): file I/O, JSON, and
/// CSV helpers. Each `@builtin` declaration in the embedded `io.pra` signature module binds to the
/// implementation registered under its fully-qualified `io::<name>` key (spec §18.4).
pub fn register() {
    register_impl("io::read_file", read_file);
    register_impl("io::write_file", write_file);
    register_impl("io::read_lines", read_lines);
    register_impl("io::exists", exists);
    register_impl("io::json_parse", json_parse);
    register_impl("io::json_stringify", json_stringify);
    register_impl("io::read_json", read_json);
    register_impl("io::write_json", write_json);
    register_impl("io::csv_parse", csv_parse);
    register_impl("io::csv_stringify", csv_stringify);
    register_impl("io::read_csv", read_csv);
    register_impl("io::write_csv", write_csv);
}

// ——— file I/O (spec §18) ———

fn read_file(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    arity(args, 1, "io::read_file")?;
    let path = string_arg(args, 0, "io::read_file")?;
    Ok(match fs::read_to_string(&path) {
        Ok(s) => ok(Value::String(s)),
        Err(e) => err(format!("cannot read `{path}`: {e}")),
    })
}

fn write_file(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    arity(args, 2, "io::write_file")?;
    let path = string_arg(args, 0, "io::write_file")?;
    let content = string_arg(args, 1, "io::write_file")?;
    Ok(match fs::write(&path, content) {
        Ok(()) => ok(Value::Nil),
        Err(e) => err(format!("cannot write `{path}`: {e}")),
    })
}

fn read_lines(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    arity(args, 1, "io::read_lines")?;
    let path = string_arg(args, 0, "io::read_lines")?;
    let result = match fs::read_to_string(&path) {
        Ok(content) => {
            let lines: Vec<String> = content
                .split('\n')
                .map(|l| l.trim_end_matches('\r').to_string())
                .collect();
            // A trailing newline does not produce a final empty line (spec §18 io).
            let lines = if content.ends_with('\n') && lines.last().map(String::is_empty).unwrap_or(false) {
                &lines[..lines.len() - 1]
            } else {
                &lines[..]
            };
            ok(Value::Array(lines.iter().map(|l| Value::String(l.clone())).collect()))
        }
        Err(e) => err(format!("cannot read `{path}`: {e}")),
    };
    Ok(result)
}

fn exists(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    arity(args, 1, "io::exists")?;
    let path = string_arg(args, 0, "io::exists")?;
    Ok(Value::Bool(Path::new(&path).exists()))
}

// ——— JSON (spec §18: serialization via serde_json) ———

/// `serde_json::Value` → Prima `Value` (spec §5 / §11.6): object → `Dict` with string `ValueKey`s,
/// array → `Array`, string → `String`, integer → `Number::Integer`, other numbers → `Number::Real`,
/// bool → `Bool`, null → `Nil`.
fn json_to_value(v: serde_json::Value) -> Value {
    match v {
        serde_json::Value::Null => Value::Nil,
        serde_json::Value::Bool(b) => Value::Bool(b),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Value::Number(Number::from(i))
            } else {
                // u64/big integers beyond i64 and all floats: fall back to the inexact layer (spec §6.1).
                Value::Number(Number::from(n.as_f64().unwrap_or(0.0)))
            }
        }
        serde_json::Value::String(s) => Value::String(s),
        serde_json::Value::Array(items) => Value::Array(items.into_iter().map(json_to_value).collect()),
        serde_json::Value::Object(map) => {
            let d: HashMap<ValueKey, Value> = map.into_iter().map(|(k, v)| (ValueKey::Str(k), json_to_value(v))).collect();
            Value::Dict(d)
        }
    }
}

/// Prima `Value` → `serde_json::Value` (reverse of `json_to_value`). Dict keys must be strings;
/// only Nil/Bool/Number/String/Array/Dict serialize. NaN/Inf numbers are rejected (not valid JSON).
fn value_to_json(v: &Value) -> Result<serde_json::Value, String> {
    match v {
        Value::Nil => Ok(serde_json::Value::Null),
        Value::Bool(b) => Ok(serde_json::Value::Bool(*b)),
        Value::String(s) => Ok(serde_json::Value::String(s.clone())),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(serde_json::Value::Number(i.into()))
            } else {
                let f = n.to_f64_lossy();
                serde_json::Number::from_f64(f)
                    .map(serde_json::Value::Number)
                    .ok_or_else(|| format!("cannot serialize non-finite number {n} as JSON"))
            }
        }
        Value::Array(items) => items
            .iter()
            .map(value_to_json)
            .collect::<Result<Vec<_>, _>>()
            .map(serde_json::Value::Array),
        Value::Dict(map) => {
            let mut obj = serde_json::Map::new();
            for (k, val) in map {
                let ValueKey::Str(s) = k else {
                    return Err(format!("JSON object keys must be strings, got {k:?}"));
                };
                obj.insert(s.clone(), value_to_json(val)?);
            }
            Ok(serde_json::Value::Object(obj))
        }
        other => Err(format!("cannot serialize {other:?} as JSON")),
    }
}

fn json_parse(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    arity(args, 1, "io::json_parse")?;
    let s = string_arg(args, 0, "io::json_parse")?;
    Ok(match serde_json::from_str::<serde_json::Value>(&s) {
        Ok(v) => ok(json_to_value(v)),
        Err(e) => err(format!("invalid JSON: {e}")),
    })
}

fn json_stringify(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    arity(args, 1, "io::json_stringify")?;
    let v = args[0].clone();
    let j = value_to_json(&v).map_err(RuntimeError::Message)?;
    Ok(ok(Value::String(j.to_string())))
}

fn read_json(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    arity(args, 1, "io::read_json")?;
    let path = string_arg(args, 0, "io::read_json")?;
    Ok(match fs::read_to_string(&path) {
        Ok(s) => match serde_json::from_str::<serde_json::Value>(&s) {
            Ok(v) => ok(json_to_value(v)),
            Err(e) => err(format!("invalid JSON in `{path}`: {e}")),
        },
        Err(e) => err(format!("cannot read `{path}`: {e}")),
    })
}

fn write_json(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    arity(args, 2, "io::write_json")?;
    let path = string_arg(args, 0, "io::write_json")?;
    let v = args[1].clone();
    let j = value_to_json(&v).map_err(RuntimeError::Message)?;
    Ok(match fs::write(&path, j.to_string()) {
        Ok(()) => ok(Value::Nil),
        Err(e) => err(format!("cannot write `{path}`: {e}")),
    })
}

// ——— CSV (hand-rolled, RFC 4180-ish: comma delimiter, quoted fields with `""` escapes) ———

/// Parse RFC 4180-ish CSV text into rows of string fields. Quoted fields may contain commas and
/// newlines; `""` inside a quoted field is a literal quote; `\r\n` line endings are normalized.
fn csv_parse_text(s: &str) -> Result<Vec<Vec<String>>, String> {
    if s.is_empty() {
        return Ok(Vec::new());
    }
    let bytes = s.as_bytes();
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut row: Vec<String> = Vec::new();
    let mut field = String::new();
    let mut in_quotes = false;
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if in_quotes {
            match c {
                b'"' => {
                    if i + 1 < bytes.len() && bytes[i + 1] == b'"' {
                        field.push('"');
                        i += 2;
                    } else {
                        in_quotes = false;
                        i += 1;
                    }
                }
                b'\n' => {
                    field.push('\n');
                    i += 1;
                }
                b'\r' => {
                    if i + 1 < bytes.len() && bytes[i + 1] == b'\n' {
                        field.push('\n');
                        i += 2;
                    } else {
                        field.push('\r');
                        i += 1;
                    }
                }
                _ => {
                    field.push(c as char);
                    i += 1;
                }
            }
        } else {
            match c {
                b'"' => {
                    in_quotes = true;
                    i += 1;
                }
                b',' => {
                    row.push(std::mem::take(&mut field));
                    i += 1;
                }
                b'\n' => {
                    row.push(std::mem::take(&mut field));
                    rows.push(std::mem::take(&mut row));
                    i += 1;
                }
                b'\r' => {
                    if i + 1 < bytes.len() && bytes[i + 1] == b'\n' {
                        i += 1; // the following '\n' terminates the record
                    } else {
                        field.push('\r');
                        i += 1;
                    }
                }
                _ => {
                    field.push(c as char);
                    i += 1;
                }
            }
        }
    }
    if in_quotes {
        return Err("unterminated quoted field in CSV input".into());
    }
    row.push(field);
    rows.push(row);
    // Drop trailing empty records produced by one or more final newlines.
    while rows.len() > 1 {
        let last = rows.last().expect("rows non-empty");
        if last.len() == 1 && last[0].is_empty() {
            rows.pop();
        } else {
            break;
        }
    }
    Ok(rows)
}

/// Serialize rows of string fields to CSV text; quote fields that contain a comma, quote, or newline.
fn csv_stringify_text(rows: &[Vec<String>]) -> String {
    let mut out = String::new();
    for row in rows {
        for (i, f) in row.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            if f.contains(',') || f.contains('"') || f.contains('\n') {
                out.push('"');
                out.push_str(&f.replace('"', "\"\""));
                out.push('"');
            } else {
                out.push_str(f);
            }
        }
        out.push('\n');
    }
    out
}

fn csv_parse(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    arity(args, 1, "io::csv_parse")?;
    let s = string_arg(args, 0, "io::csv_parse")?;
    Ok(match csv_parse_text(&s) {
        Ok(rows) => ok(Value::Array(
            rows.into_iter()
                .map(|r| Value::Array(r.into_iter().map(Value::String).collect()))
                .collect(),
        )),
        Err(e) => err(e),
    })
}

fn csv_stringify(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    arity(args, 1, "io::csv_stringify")?;
    let rows = array_arg(&args[0], "io::csv_stringify", 0)?;
    let mut out_rows: Vec<Vec<String>> = Vec::with_capacity(rows.len());
    for (r, row) in rows.iter().enumerate() {
        let fields = array_arg(row, "io::csv_stringify", 0)?;
        let mut out_row = Vec::with_capacity(fields.len());
        for (c, f) in fields.iter().enumerate() {
            out_row.push(array_string(f, "io::csv_stringify", &format!("row {r} field {c}"))?);
        }
        out_rows.push(out_row);
    }
    Ok(ok(Value::String(csv_stringify_text(&out_rows))))
}

fn read_csv(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    arity(args, 1, "io::read_csv")?;
    let path = string_arg(args, 0, "io::read_csv")?;
    Ok(match fs::read_to_string(&path) {
        Ok(s) => match csv_parse_text(&s) {
            Ok(rows) => ok(Value::Array(
                rows.into_iter()
                    .map(|r| Value::Array(r.into_iter().map(Value::String).collect()))
                    .collect(),
            )),
            Err(e) => err(format!("invalid CSV in `{path}`: {e}")),
        },
        Err(e) => err(format!("cannot read `{path}`: {e}")),
    })
}

fn write_csv(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    arity(args, 2, "io::write_csv")?;
    let path = string_arg(args, 0, "io::write_csv")?;
    let rows = array_arg(&args[1], "io::write_csv", 1)?;
    let mut out_rows: Vec<Vec<String>> = Vec::with_capacity(rows.len());
    for (r, row) in rows.iter().enumerate() {
        let fields = array_arg(row, "io::write_csv", 1)?;
        let mut out_row = Vec::with_capacity(fields.len());
        for (c, f) in fields.iter().enumerate() {
            out_row.push(array_string(f, "io::write_csv", &format!("row {r} field {c}"))?);
        }
        out_rows.push(out_row);
    }
    Ok(match fs::write(&path, csv_stringify_text(&out_rows)) {
        Ok(()) => ok(Value::Nil),
        Err(e) => err(format!("cannot write `{path}`: {e}")),
    })
}
