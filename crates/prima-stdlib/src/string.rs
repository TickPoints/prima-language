//! Native implementations of the `String` class methods (spec §18.1), registered under
//! `"String::<name>"` keys and bound to the `@builtin`/`@builtin(ON)` declarations in
//! `modules/string.pra` (spec §18.4).
//!
//! Hot methods (`split`/`replace`/`strip`/`find`/`join`) are layered `@builtin(O2)`: the `.pra`
//! fallback body in `string.pra` is the semantic authority, and these Rust implementations must
//! match it exactly (the O0/O2 consistency tests enforce this). All string methods operate on a
//! value-semantic copy; `args[0]` is the receiver.

use prima_core::{Number, Value};
use prima_runtime::builtin;
use prima_runtime::{Evaluator, RuntimeError, value_type_name};

/// Extract the `String` receiver (`args[0]`).
fn recv(args: &[Value], name: &str) -> Result<String, RuntimeError> {
    match args.first() {
        Some(Value::String(s)) => Ok(s.clone()),
        Some(other) => Err(RuntimeError::Message(format!(
            "`String.{name}` expects a string receiver, got {}",
            value_type_name(other)
        ))),
        None => Err(RuntimeError::Message(format!(
            "`String.{name}` expects a string receiver"
        ))),
    }
}

/// Method argument `i` (1-based, after the receiver) as a `String`.
fn str_arg(args: &[Value], i: usize, name: &str) -> Result<String, RuntimeError> {
    match args.get(i) {
        Some(Value::String(s)) => Ok(s.clone()),
        Some(other) => Err(RuntimeError::Message(format!(
            "`String.{name}` argument {i} must be a string, got {}",
            value_type_name(other)
        ))),
        None => Err(RuntimeError::Message(format!(
            "`String.{name}` missing argument {i}"
        ))),
    }
}

/// Method argument `i` (1-based, after the receiver) as an `i64`.
fn int_arg(args: &[Value], i: usize, name: &str) -> Result<i64, RuntimeError> {
    match args.get(i) {
        Some(Value::Number(n)) => n.as_i64().ok_or_else(|| {
            RuntimeError::Type(format!(
                "`String.{name}` argument {i} must be an integer, got {n}"
            ))
        }),
        Some(other) => Err(RuntimeError::Message(format!(
            "`String.{name}` argument {i} must be an integer, got {}",
            value_type_name(other)
        ))),
        None => Err(RuntimeError::Message(format!(
            "`String.{name}` missing argument {i}"
        ))),
    }
}

fn arity(args: &[Value], n: usize, name: &str) -> Result<(), RuntimeError> {
    let got = args.len().saturating_sub(1);
    if got == n {
        Ok(())
    } else {
        Err(RuntimeError::Message(format!(
            "`String.{name}` expects {n} argument(s), got {got}"
        )))
    }
}

// ---- primitive accessors (O0) ----

fn string_len(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    let s = recv(args, "len")?;
    Ok(Value::Number(Number::from(s.chars().count() as i64)))
}

fn string_is_empty(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    let s = recv(args, "is_empty")?;
    Ok(Value::Bool(s.is_empty()))
}

fn string_push(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    let s = recv(args, "push")?;
    arity(args, 1, "push")?;
    Ok(Value::String(format!("{s}{}", str_arg(args, 1, "push")?)))
}

fn string_insert(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    let s = recv(args, "insert")?;
    arity(args, 2, "insert")?;
    let idx = int_arg(args, 1, "insert")?;
    let sub = str_arg(args, 2, "insert")?;
    let len = s.chars().count() as i64;
    if idx < 0 || idx > len {
        return Ok(Value::Result(Err(format!(
            "insert index {idx} out of range (length {len})"
        ))));
    }
    let idx = idx as usize;
    let mut out: String = s.chars().take(idx).collect();
    out.push_str(&sub);
    out.extend(s.chars().skip(idx));
    Ok(Value::Result(Ok(Box::new(Value::String(out)))))
}

fn string_char_at(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    let s = recv(args, "char_at")?;
    arity(args, 1, "char_at")?;
    let idx = int_arg(args, 1, "char_at")?;
    if idx < 0 {
        return Ok(Value::Option(None));
    }
    match s.chars().nth(idx as usize) {
        Some(c) => Ok(Value::Option(Some(Box::new(Value::Char(c))))),
        None => Ok(Value::Option(None)),
    }
}

fn string_substring(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    let s = recv(args, "substring")?;
    arity(args, 2, "substring")?;
    let a = int_arg(args, 1, "substring")?;
    let b = int_arg(args, 2, "substring")?;
    if a < 0 {
        return Err(RuntimeError::Message(
            "`String.substring` start must be non-negative".into(),
        ));
    }
    let (a, b) = (a as usize, b as usize);
    let chars: Vec<char> = s.chars().collect();
    if a > chars.len() || b > chars.len() || a > b {
        return Err(RuntimeError::Message(format!(
            "invalid substring range {a}..{b} (length {})",
            chars.len()
        )));
    }
    Ok(Value::String(chars[a..b].iter().collect()))
}

fn string_contains(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    let s = recv(args, "contains")?;
    arity(args, 1, "contains")?;
    Ok(Value::Bool(s.contains(&str_arg(args, 1, "contains")?)))
}

fn string_to_upper(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    let s = recv(args, "to_upper")?;
    Ok(Value::String(s.to_uppercase()))
}

fn string_to_lower(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    let s = recv(args, "to_lower")?;
    Ok(Value::String(s.to_lowercase()))
}

fn string_repeat(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    let s = recv(args, "repeat")?;
    arity(args, 1, "repeat")?;
    let n = int_arg(args, 1, "repeat")?;
    if n < 0 {
        return Err(RuntimeError::Message(
            "`String.repeat` count must be non-negative".into(),
        ));
    }
    Ok(Value::String(s.repeat(n as usize)))
}

fn string_trim(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    let s = recv(args, "trim")?;
    Ok(Value::String(s.trim().to_string()))
}

fn string_lstrip(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    let s = recv(args, "lstrip")?;
    arity(args, 1, "lstrip")?;
    let pat: Vec<char> = str_arg(args, 1, "lstrip")?.chars().collect();
    Ok(Value::String(
        s.trim_start_matches(|c| pat.contains(&c)).to_string(),
    ))
}

fn string_rstrip(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    let s = recv(args, "rstrip")?;
    arity(args, 1, "rstrip")?;
    let pat: Vec<char> = str_arg(args, 1, "rstrip")?.chars().collect();
    Ok(Value::String(
        s.trim_end_matches(|c| pat.contains(&c)).to_string(),
    ))
}

// ---- predicates (O0) ----

fn string_is_upper(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    let s = recv(args, "is_upper")?;
    let has_cased = s.chars().any(|c| c.is_uppercase() || c.is_lowercase());
    Ok(Value::Bool(
        has_cased && s.chars().all(|c| !c.is_lowercase()),
    ))
}

fn string_is_lower(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    let s = recv(args, "is_lower")?;
    let has_cased = s.chars().any(|c| c.is_uppercase() || c.is_lowercase());
    Ok(Value::Bool(
        has_cased && s.chars().all(|c| !c.is_uppercase()),
    ))
}

fn string_is_alpha(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    let s = recv(args, "is_alpha")?;
    Ok(Value::Bool(
        !s.is_empty() && s.chars().all(|c| c.is_alphabetic()),
    ))
}

fn string_is_digit(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    let s = recv(args, "is_digit")?;
    Ok(Value::Bool(
        !s.is_empty() && s.chars().all(|c| c.is_numeric()),
    ))
}

fn string_is_alnum(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    let s = recv(args, "is_alnum")?;
    Ok(Value::Bool(
        !s.is_empty() && s.chars().all(|c| c.is_alphanumeric()),
    ))
}

fn string_is_space(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    let s = recv(args, "is_space")?;
    Ok(Value::Bool(
        !s.is_empty() && s.chars().all(|c| c.is_whitespace()),
    ))
}

fn string_is_ascii(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    let s = recv(args, "is_ascii")?;
    Ok(Value::Bool(s.is_ascii()))
}

// ---- case transforms (O0) ----

fn string_capitalize(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    let s = recv(args, "capitalize")?;
    let mut cs = s.chars();
    match cs.next() {
        Some(first) => {
            let mut out = String::new();
            out.push_str(&first.to_uppercase().collect::<String>());
            out.push_str(&cs.as_str().to_lowercase());
            Ok(Value::String(out))
        }
        None => Ok(Value::String(String::new())),
    }
}

fn string_title(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    let s = recv(args, "title")?;
    let mut out = String::new();
    let mut prev_alpha = false;
    for c in s.chars() {
        if prev_alpha {
            out.push_str(&c.to_lowercase().collect::<String>());
        } else {
            out.push_str(&c.to_uppercase().collect::<String>());
        }
        prev_alpha = c.is_alphabetic();
    }
    Ok(Value::String(out))
}

fn string_swapcase(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    let s = recv(args, "swapcase")?;
    let mut out = String::new();
    for c in s.chars() {
        if c.is_uppercase() {
            out.push_str(&c.to_lowercase().collect::<String>());
        } else if c.is_lowercase() {
            out.push_str(&c.to_uppercase().collect::<String>());
        } else {
            out.push(c);
        }
    }
    Ok(Value::String(out))
}

fn string_casefold(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    let s = recv(args, "casefold")?;
    Ok(Value::String(s.to_lowercase()))
}

// ---- layered hot methods (O2): must match the `.pra` fallback bodies in `string.pra` ----

fn string_split(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    let s = recv(args, "split")?;
    arity(args, 1, "split")?;
    let sep = str_arg(args, 1, "split")?;
    if sep.is_empty() {
        let parts: Vec<Value> = s.chars().map(|c| Value::String(c.to_string())).collect();
        return Ok(Value::Array(parts));
    }
    let parts: Vec<Value> = s
        .split(&sep)
        .map(|p| Value::String(p.to_string()))
        .collect();
    Ok(Value::Array(parts))
}

fn string_replace(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    let s = recv(args, "replace")?;
    arity(args, 2, "replace")?;
    let old = str_arg(args, 1, "replace")?;
    let new = str_arg(args, 2, "replace")?;
    if old.is_empty() {
        return Ok(Value::String(s));
    }
    Ok(Value::String(s.replace(&old, &new)))
}

fn string_strip(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    let s = recv(args, "strip")?;
    arity(args, 1, "strip")?;
    let pat: Vec<char> = str_arg(args, 1, "strip")?.chars().collect();
    Ok(Value::String(
        s.trim_matches(|c| pat.contains(&c)).to_string(),
    ))
}

fn string_find(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    let s = recv(args, "find")?;
    arity(args, 1, "find")?;
    let pat = str_arg(args, 1, "find")?;
    match s.find(&pat) {
        Some(i) => Ok(Value::Option(Some(Box::new(Value::Number(Number::from(
            s[..i].chars().count() as i64,
        )))))),
        None => Ok(Value::Option(None)),
    }
}

fn string_join(_ev: &mut Evaluator, args: &[Value]) -> Result<Value, RuntimeError> {
    let s = recv(args, "join")?;
    arity(args, 1, "join")?;
    let parts = match &args[1] {
        Value::Array(parts) => parts,
        other => {
            return Err(RuntimeError::Message(format!(
                "`String.join` expects an array of strings, got {}",
                value_type_name(other)
            )));
        }
    };
    let mut out = String::new();
    for (i, p) in parts.iter().enumerate() {
        if i > 0 {
            out.push_str(&s);
        }
        match p {
            Value::String(p) => out.push_str(p),
            _ => {
                return Err(RuntimeError::Message(
                    "`String.join` requires an array of strings".into(),
                ));
            }
        }
    }
    Ok(Value::String(out))
}

/// Register every `String::<name>` implementation (spec §18.1/§18.4).
pub fn register() {
    // primitive accessors and case transforms (O0)
    builtin!("String::len", string_len);
    builtin!("String::is_empty", string_is_empty);
    builtin!("String::push", string_push);
    builtin!("String::insert", string_insert);
    builtin!("String::char_at", string_char_at);
    builtin!("String::substring", string_substring);
    builtin!("String::contains", string_contains);
    builtin!("String::to_upper", string_to_upper);
    builtin!("String::to_lower", string_to_lower);
    builtin!("String::repeat", string_repeat);
    builtin!("String::trim", string_trim);
    builtin!("String::lstrip", string_lstrip);
    builtin!("String::rstrip", string_rstrip);
    builtin!("String::is_upper", string_is_upper);
    builtin!("String::is_lower", string_is_lower);
    builtin!("String::is_alpha", string_is_alpha);
    builtin!("String::is_digit", string_is_digit);
    builtin!("String::is_alnum", string_is_alnum);
    builtin!("String::is_space", string_is_space);
    builtin!("String::is_ascii", string_is_ascii);
    builtin!("String::capitalize", string_capitalize);
    builtin!("String::title", string_title);
    builtin!("String::swapcase", string_swapcase);
    builtin!("String::casefold", string_casefold);
    // layered hot methods (O2)
    builtin!("String::split", string_split, O2);
    builtin!("String::replace", string_replace, O2);
    builtin!("String::strip", string_strip, O2);
    builtin!("String::find", string_find, O2);
    builtin!("String::join", string_join, O2);
}
