//! `time` module (spec §18.3 / appendix B.5): unix timestamps, durations (in seconds), formatting,
//! and parsing. Times are `Value::Number` unix seconds (`Number::I64` for whole seconds); durations
//! are also seconds and may be fractional (`Rational`/`F64`).

use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use prima_core::{Number, Real, Value};
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

fn number_arg(args: &[Value], i: usize, fname: &str) -> Result<Number, RuntimeError> {
    match args.get(i) {
        Some(Value::Number(n)) => Ok(n.clone()),
        Some(other) => Err(RuntimeError::Type(format!(
            "`{fname}` argument {i} must be a number, got {other:?}"
        ))),
        None => Err(RuntimeError::Message(format!("`{fname}` missing argument {i}"))),
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

/// Register the `time` namespace (spec §18.3).
pub fn register() {
    let mut items = HashMap::new();
    items.insert("now".into(), native("time::now", time_now));
    items.insert("sleep".into(), native("time::sleep", time_sleep));
    items.insert("unix_timestamp".into(), native("time::unix_timestamp", time_unix_timestamp));
    items.insert("format".into(), native("time::format", time_format));
    items.insert("parse".into(), native("time::parse", time_parse));
    // `time::Duration::from_secs` resolves via the flattened module-item lookup (see `eval::resolve_func`).
    items.insert("Duration::from_secs".into(), native("time::Duration::from_secs", duration_from_secs));
    items.insert("Duration::from_millis".into(), native("time::Duration::from_millis", duration_from_millis));
    register_namespace("time", items);
}

fn time_now(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    arity(args, 0, "time::now")?;
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    Ok(Value::Number(Number::I64(secs as i64)))
}

fn time_sleep(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    arity(args, 1, "time::sleep")?;
    let secs = number_arg(args, 0, "time::sleep")?;
    std::thread::sleep(std::time::Duration::from_secs_f64(secs.to_f64_lossy()));
    Ok(Value::Nil)
}

fn time_unix_timestamp(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    arity(args, 1, "time::unix_timestamp")?;
    let n = number_arg(args, 0, "time::unix_timestamp")?;
    let secs = n
        .as_i64()
        .filter(|v| *v >= 0)
        .ok_or_else(|| RuntimeError::Type(format!("`time::unix_timestamp` expects a non-negative integral timestamp, got {n}")))?;
    Ok(Value::Number(Number::I64(secs)))
}

fn duration_from_secs(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    arity(args, 1, "time::Duration::from_secs")?;
    let n = number_arg(args, 0, "time::Duration::from_secs")?;
    let secs = n
        .as_i64()
        .ok_or_else(|| RuntimeError::Type(format!("`time::Duration::from_secs` expects an integer, got {n}")))?;
    Ok(Value::Number(Number::from(secs)))
}

fn duration_from_millis(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    arity(args, 1, "time::Duration::from_millis")?;
    let n = number_arg(args, 0, "time::Duration::from_millis")?;
    let ms = n
        .as_i64()
        .ok_or_else(|| RuntimeError::Type(format!("`time::Duration::from_millis` expects an integer, got {n}")))?;
    if ms % 1000 == 0 {
        Ok(Value::Number(Number::from(ms / 1000)))
    } else {
        Ok(Value::Number(Number::Real(Real::F64(ms as f64 / 1000.0))))
    }
}

fn time_format(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    arity(args, 2, "time::format")?;
    let n = number_arg(args, 0, "time::format")?;
    let fmt = string_arg(args, 1, "time::format")?;
    let secs = n.to_f64_lossy().floor() as i64;
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    let (hour, minute, second) = (rem / 3600, (rem % 3600) / 60, rem % 60);

    let mut out = String::new();
    let mut chars = fmt.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '%' {
            match chars.next() {
                Some('Y') => out.push_str(&format!("{year:04}")),
                Some('m') => out.push_str(&format!("{month:02}")),
                Some('d') => out.push_str(&format!("{day:02}")),
                Some('H') => out.push_str(&format!("{hour:02}")),
                Some('M') => out.push_str(&format!("{minute:02}")),
                Some('S') => out.push_str(&format!("{second:02}")),
                Some('s') => out.push_str(&secs.to_string()),
                Some(other) => {
                    out.push('%');
                    out.push(other);
                }
                None => out.push('%'),
            }
        } else {
            out.push(c);
        }
    }
    Ok(Value::String(out))
}

fn time_parse(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    arity(args, 2, "time::parse")?;
    let s = string_arg(args, 0, "time::parse")?;
    let fmt = string_arg(args, 1, "time::parse")?;
    match parse_dt(&s, &fmt) {
        Ok(n) => Ok(Value::Result(Ok(Box::new(Value::Number(n))))),
        Err(msg) => Ok(Value::Result(Err(msg))),
    }
}

/// Parse a timestamp for the `%Y-%m-%d`, `%Y-%m-%d %H:%M:%S`, and `%s` formats (spec §18.3),
/// returning unix seconds.
fn parse_dt(s: &str, fmt: &str) -> Result<Number, String> {
    if fmt == "%s" {
        let secs: i64 = s.trim().parse().map_err(|_| format!("cannot parse `{s}` as unix seconds"))?;
        return Ok(Number::I64(secs));
    }
    let (mut year, mut month, mut day) = (1970i64, 1u32, 1u32);
    let (mut hour, mut minute, mut second) = (0u32, 0u32, 0u32);

    let s_chars: Vec<char> = s.chars().collect();
    let f_chars: Vec<char> = fmt.chars().collect();
    let (mut i, mut j) = (0usize, 0usize);
    while j < f_chars.len() {
        if f_chars[j] == '%' && j + 1 < f_chars.len() {
            match f_chars[j + 1] {
                'Y' => {
                    let (v, n) = digits(&s_chars, i, 4)?;
                    year = v;
                    i += n;
                }
                'm' => {
                    let (v, n) = digits(&s_chars, i, 2)?;
                    month = v as u32;
                    i += n;
                }
                'd' => {
                    let (v, n) = digits(&s_chars, i, 2)?;
                    day = v as u32;
                    i += n;
                }
                'H' => {
                    let (v, n) = digits(&s_chars, i, 2)?;
                    hour = v as u32;
                    i += n;
                }
                'M' => {
                    let (v, n) = digits(&s_chars, i, 2)?;
                    minute = v as u32;
                    i += n;
                }
                'S' => {
                    let (v, n) = digits(&s_chars, i, 2)?;
                    second = v as u32;
                    i += n;
                }
                other => return Err(format!("unsupported format directive `%{other}`")),
            }
            j += 2;
        } else {
            if i >= s_chars.len() || s_chars[i] != f_chars[j] {
                return Err(format!("cannot parse `{s}` with format `{fmt}`"));
            }
            i += 1;
            j += 1;
        }
    }
    if i != s_chars.len() {
        return Err(format!("cannot parse `{s}` with format `{fmt}`"));
    }
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) || hour > 23 || minute > 59 || second > 60 {
        return Err(format!("invalid date/time fields in `{s}`"));
    }
    let days = days_from_civil(year, month, day);
    let secs = days * 86_400 + (hour as i64) * 3600 + (minute as i64) * 60 + second as i64;
    Ok(Number::I64(secs))
}

/// Read up to `len` consecutive digits starting at `i`; returns the value and how many were consumed.
fn digits(chars: &[char], i: usize, len: usize) -> Result<(i64, usize), String> {
    let mut v: i64 = 0;
    let mut n = 0usize;
    while n < len && i + n < chars.len() && chars[i + n].is_ascii_digit() {
        v = v * 10 + (chars[i + n] as i64 - '0' as i64);
        n += 1;
    }
    if n == 0 {
        return Err("expected digits".into());
    }
    Ok((v, n))
}

/// Days since 1970-01-01 from a civil (proleptic Gregorian) date (Howard Hinnant's public-domain
/// algorithm). Supports years ≥ 0 (and negative years via `div_euclid` in `civil_from_days`).
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = y - if m <= 2 { 1 } else { 0 };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) as i64 + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Inverse of `days_from_civil`: days since epoch → (year, month, day) in UTC (Howard Hinnant).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}
