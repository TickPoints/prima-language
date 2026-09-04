//! Free helper functions for the evaluator: source spans, environment/parfor analysis, numeric and
//! string formatting helpers, and diagnostic-note construction (spec §16.4/§17.2/§18.1).
//!
//! This module holds only *stateless* helpers — functions that take their inputs by argument and do
//! not require interpreter state. The interpreter's own methods live in `super::eval`.

use std::collections::HashSet;

use num_bigint::BigInt;
use prima_core::{Number, Real, Value};
use prima_syntax::ast::{
    AssignOp, Block, ComprehensionClause, Expr, ExprKind, ImplOp, IndexItem, Literal, Pattern,
    Spanned, Stmt, Type, UnOp,
};
use prima_syntax::error::SyntaxError;

use crate::class::MethodDef;
use crate::config::{Config, Domain, OverloadPolicy, UndefinedHandling};
use crate::error::RuntimeError;

/// Source span of a statement, used to locate errors (spec §16.4).
pub(crate) fn stmt_span(stmt: &Stmt) -> prima_syntax::Span {
    match stmt {
        Stmt::Let { span, .. }
        | Stmt::Const { span, .. }
        | Stmt::FnDef { span, .. }
        | Stmt::MathDef { span, .. }
        | Stmt::ClassDef { span, .. }
        | Stmt::Impl { span, .. }
        | Stmt::Assign { span, .. }
        | Stmt::For { span, .. }
        | Stmt::ParFor { span, .. }
        | Stmt::While { span, .. }
        | Stmt::If { span, .. }
        | Stmt::IfLet { span, .. }
        | Stmt::WhileLet { span, .. }
        | Stmt::Match { span, .. }
        | Stmt::Return { span, .. }
        | Stmt::WithConfig { span, .. } => *span,
        Stmt::Expr(e) => e.span,
        Stmt::Pub(inner) => stmt_span(inner),
    }
}

/// Process-wide defaults at the bottom of the policy stack (spec §4.6).
pub(crate) static DEFAULT_CONFIG: Config = Config {
    domain: Domain::Complex,
    undefined_handling: UndefinedHandling::Strict,
    custom_rules: Vec::new(),
    fraction: true,
    broadcast: true,
    loop_optimization: true,
    simplify_level: 2,
    opt_level: crate::config::OptLevel::O2,
    num_to_big: true,
    print_format: crate::config::PrintFormat::Latex,
    overload_policy: OverloadPolicy::Warn,
};

/// Minimum array length for which a `@parallel` MFn broadcast is split across rayon threads (spec §17.1);
/// smaller arrays keep the sequential path to avoid thread-spawn overhead.
pub(crate) const PARALLEL_BROADCAST_THRESHOLD: usize = 1024;

pub(crate) fn path_key(segments: &[Spanned<String>]) -> String {
    segments
        .iter()
        .map(|s| s.value.as_str())
        .collect::<Vec<_>>()
        .join("::")
}

/// Map arguments to `f64` when every argument is a non-complex number; otherwise `None` (spec §19.2:
/// only numeric scalars participate in the JIT hot path).
pub(crate) fn numeric_args(args: &[Value]) -> Option<Vec<f64>> {
    let mut out = Vec::with_capacity(args.len());
    for a in args {
        if let Value::Number(n) = a {
            if n.is_complex() {
                return None;
            }
            out.push(n.to_f64_lossy());
        } else {
            return None;
        }
    }
    Some(out)
}

/// Collect every single-segment variable path referenced in `e` (spec §17.2 read set): used to bind
/// read-only outer values into `parfor` task environments without touching the non-`Send` env chain.
pub(crate) fn collect_read_names(e: &Expr, out: &mut HashSet<String>) {
    match &e.kind {
        ExprKind::Path { segments } if segments.len() == 1 => {
            out.insert(segments[0].value.clone());
        }
        ExprKind::Call { callee, args } => {
            collect_read_names(callee, out);
            for a in args {
                collect_read_names(a, out);
            }
        }
        ExprKind::MethodCall { receiver, args, .. } => {
            collect_read_names(receiver, out);
            for a in args {
                collect_read_names(a, out);
            }
        }
        ExprKind::Field { receiver, .. } => collect_read_names(receiver, out),
        ExprKind::StructLiteral { fields, base, .. } => {
            if let Some(b) = base {
                collect_read_names(b, out);
            }
            for f in fields {
                if let Some(v) = &f.value {
                    collect_read_names(v, out);
                }
            }
        }
        ExprKind::Index { base, index } => {
            collect_read_names(base, out);
            for it in &index.items {
                match it {
                    IndexItem::Elem(e) => collect_read_names(e, out),
                    IndexItem::Slice { start, end } => {
                        if let Some(s) = start {
                            collect_read_names(s, out);
                        }
                        if let Some(s) = end {
                            collect_read_names(s, out);
                        }
                    }
                }
            }
        }
        ExprKind::Binary { lhs, rhs, .. } => {
            collect_read_names(lhs, out);
            collect_read_names(rhs, out);
        }
        ExprKind::Unary { operand, .. } | ExprKind::Try(operand) => {
            collect_read_names(operand, out)
        }
        ExprKind::Array(items) | ExprKind::Tuple(items) => {
            for it in items {
                collect_read_names(it, out);
            }
        }
        ExprKind::Lambda { body, .. } => collect_read_names(body, out),
        ExprKind::Match { scrutinee, arms } => {
            collect_read_names(scrutinee, out);
            for a in arms {
                if let Some(g) = &a.guard {
                    collect_read_names(g, out);
                }
                collect_read_names(&a.body, out);
            }
        }
        ExprKind::Dict(entries) => {
            for (k, v) in entries {
                collect_read_names(k, out);
                collect_read_names(v, out);
            }
        }
        ExprKind::Set(items) => {
            for it in items {
                collect_read_names(it, out);
            }
        }
        ExprKind::Comprehension {
            output, clauses, ..
        } => {
            collect_read_names(output, out);
            for c in clauses {
                match c {
                    ComprehensionClause::For { iter, .. } => collect_read_names(iter, out),
                    ComprehensionClause::If { cond } => collect_read_names(cond, out),
                }
            }
        }
        ExprKind::KeyValue { key, value } => {
            collect_read_names(key, out);
            collect_read_names(value, out);
        }
        _ => {}
    }
}

/// One unit of work inside a `parfor` iteration body (spec §17.2): either an index-slot assignment
/// (`A[i] = …`/`A[i] += …`) or a pure function call evaluated for effect.
#[derive(Clone)]
pub(crate) enum ParforStep {
    Assign(ParforWrite),
    Eval(Expr),
}

#[derive(Clone)]
pub(crate) struct ParforWrite {
    pub(crate) array: String,
    pub(crate) index: Expr,
    pub(crate) op: AssignOp,
    pub(crate) value: Expr,
}

/// One index write produced by a `parfor` iteration: (array name, index, merged value).
pub(crate) type ParforWriteVec = Vec<(String, usize, Value)>;

/// Static side-effect check for a `parfor` body (spec §17.2, `E0082`): only index-slot assignments
/// (`A[i] = …`/`A[i] += …`/`A[i] -= …`) and pure function calls are allowed; anything else (external
/// variable assignment, `let`, `print`, class mutation, …) is an error.
pub(crate) fn check_parfor_body(body: &Block) -> Result<Vec<ParforStep>, RuntimeError> {
    let mut steps = Vec::new();
    for stmt in &body.stmts {
        match stmt {
            Stmt::Assign {
                target, op, value, ..
            } => {
                if let ExprKind::Index { base, index } = &target.kind
                    && let ExprKind::Path { segments } = &base.kind
                    && segments.len() == 1
                    && index.items.len() == 1
                    && let IndexItem::Elem(idx) = &index.items[0]
                {
                    steps.push(ParforStep::Assign(ParforWrite {
                        array: segments[0].value.clone(),
                        index: idx.clone(),
                        op: *op,
                        value: value.clone(),
                    }));
                } else {
                    return crate::error::err(
                        "parfor iteration body may only assign to index slots `A[i]` (E0082)",
                    );
                }
            }
            Stmt::Expr(e) => steps.push(ParforStep::Eval(e.clone())),
            _ => {
                return crate::error::err(
                    "parfor iteration body must be side-effect free (E0082): only index-slot assignments and pure calls allowed",
                );
            }
        }
    }
    Ok(steps)
}

/// Operator-overload registry key: `"<class>::<Op>"` (spec §18.5; `ImplOp` has no `Hash`).
pub(crate) fn overload_key(class: &str, op: ImplOp) -> String {
    let name = match op {
        ImplOp::Add => "Add",
        ImplOp::Sub => "Sub",
        ImplOp::Mul => "Mul",
        ImplOp::Div => "Div",
        ImplOp::Rem => "Rem",
        ImplOp::Neg => "Neg",
        ImplOp::Eq => "Eq",
        ImplOp::Cmp => "Cmp",
        ImplOp::Index => "Index",
    };
    format!("{class}::{name}")
}

/// Whether a `let` pattern can fail to match (spec §4.4): only bindings/wildcards/grouped-tuples of
/// irrefutable patterns are irrefutable; anything else requires `if let`/`match` (`E0053`).
pub(crate) fn pattern_is_refutable(p: &Pattern) -> bool {
    match p {
        Pattern::Wildcard(_) | Pattern::Binding(_) => false,
        Pattern::Tuple(pats, _) | Pattern::Array(pats, _) => pats.iter().any(pattern_is_refutable),
        // Struct patterns with `..` or binding-only fields never fail; a field with an explicit refutable sub-pattern does.
        Pattern::Struct { fields, .. } => fields.iter().any(|f| match &f.pat {
            None => false,
            Some(sub) => pattern_is_refutable(sub),
        }),
        Pattern::Group(inner) => pattern_is_refutable(inner),
        Pattern::Or(pats) => pats.iter().any(pattern_is_refutable),
        _ => true,
    }
}

/// Short display name of a value's type, for error messages.
pub fn value_type_name(v: &Value) -> String {
    match v {
        Value::Nil => "nil".into(),
        Value::Number(_) => "number".into(),
        Value::Bool(_) => "bool".into(),
        Value::Char(_) => "char".into(),
        Value::String(_) => "string".into(),
        Value::Array(_) => "array".into(),
        Value::Dict(_) => "dict".into(),
        Value::Set(_) => "set".into(),
        Value::Expr(_) => "expr".into(),
        Value::Symbol(_) => "symbol".into(),
        Value::Class(_) => "class".into(),
        Value::Option(_) => "option".into(),
        Value::Result(_) => "result".into(),
        Value::Tuple(_) => "tuple".into(),
        Value::Indeterminate(_) => "indeterminate".into(),
        Value::Undefined => "undefined".into(),
        Value::Error(_) => "error".into(),
        Value::JitFunction(_) => "jit function".into(),
    }
}

/// Source-level name of a type for diagnostic signatures (spec §16.4); `Type::User` carries its text.
fn render_ty(t: &Type) -> String {
    match t {
        Type::Number => "Number".into(),
        Type::Integer => "Integer".into(),
        Type::Rational => "Rational".into(),
        Type::F64 => "F64".into(),
        Type::F32 => "F32".into(),
        Type::I8 => "I8".into(),
        Type::I16 => "I16".into(),
        Type::I32 => "I32".into(),
        Type::I64 => "I64".into(),
        Type::I128 => "I128".into(),
        Type::U8 => "U8".into(),
        Type::U16 => "U16".into(),
        Type::U32 => "U32".into(),
        Type::U64 => "U64".into(),
        Type::U128 => "U128".into(),
        Type::Isize => "Isize".into(),
        Type::Usize => "Usize".into(),
        Type::Complex => "Complex".into(),
        Type::Expr => "Expr".into(),
        Type::Symbol => "Symbol".into(),
        Type::Bool => "Bool".into(),
        Type::String => "String".into(),
        Type::Char => "Char".into(),
        Type::Array(inner) => format!("Array<{}>", render_ty(inner)),
        Type::Matrix(inner) => format!("Matrix<{}>", render_ty(inner)),
        Type::Tuple(ts) => format!(
            "Tuple<{}>",
            ts.iter().map(render_ty).collect::<Vec<_>>().join(", ")
        ),
        Type::Option(inner) => format!("Option<{}>", render_ty(inner)),
        Type::Result(a, b) => format!("Result<{}, {}>", render_ty(a), render_ty(b)),
        Type::Fn { params, ret } => format!(
            "Fn({}) -> {}",
            params.iter().map(render_ty).collect::<Vec<_>>().join(", "),
            render_ty(ret)
        ),
        Type::MFn { params, ret } => format!(
            "MFn({}) -> {}",
            params.iter().map(render_ty).collect::<Vec<_>>().join(", "),
            render_ty(ret)
        ),
        Type::SelfType => "Self".into(),
        Type::User(sp) => sp.value.clone(),
    }
}

/// Render a method signature as `name(self, args) -> Ret` for diagnostic notes (spec §16.4),
/// omitting the `-> Ret` suffix when the method has no return type.
fn method_signature(name: &str, m: &MethodDef) -> String {
    let params = m
        .params
        .iter()
        .map(|p| {
            if p.is_self {
                "self".to_string()
            } else {
                match &p.type_ann {
                    Some(t) => format!("{}: {}", p.name.value, render_ty(t)),
                    None => p.name.value.clone(),
                }
            }
        })
        .collect::<Vec<_>>()
        .join(", ");
    match &m.ret {
        Some(t) => format!("{name}({params}) -> {}", render_ty(t)),
        None => format!("{name}({params})"),
    }
}

/// A diagnostic note for a runtime `MethodDef` (spec §16.4): `method \`<sig>\`` plus the `///` doc
/// text when present. Only emitted when the method carries a doc comment — the runtime class model
/// records no definition location, so the `defined at` suffix is omitted here (registry/native docs
/// carry theirs; see [`native_method_error`]).
pub(crate) fn method_note(name: &str, m: &MethodDef) -> Option<String> {
    let docs = m.docs.as_ref()?;
    let mut note = format!("method `{}`", method_signature(name, m));
    let text = docs.text();
    if !text.is_empty() {
        note.push_str(&format!("\n{text}"));
    }
    Some(note)
}

/// A diagnostic note for a registry `MethodDoc` (spec §16.4): `method \`<sig>\` defined at <loc>`
/// plus the doc text when present.
pub(crate) fn method_doc_note(doc: &crate::docs::MethodDoc) -> String {
    let mut note = format!("method `{}` defined at {}", doc.sig, doc.defined_at);
    if let Some(text) = &doc.doc
        && !text.is_empty()
    {
        note.push_str(&format!("\n{text}"));
    }
    note
}

/// Attach diagnostic notes/help to an error, or return it unchanged when there is nothing to add.
pub(crate) fn with_notes(
    error: RuntimeError,
    notes: impl IntoIterator<Item = String>,
    help: Option<String>,
) -> RuntimeError {
    let notes: Vec<String> = notes.into_iter().collect();
    if notes.is_empty() && help.is_none() {
        error
    } else {
        RuntimeError::WithNotes {
            notes,
            help,
            error: Box::new(error),
        }
    }
}

/// Wrap a failed native-method call (`String`/`Array`/`Dict`/`Set`, spec §18.1/§11.3/§11.6) with a
/// doc note from the process-global registry (spec §16.4). A known method uses its own doc; an
/// unknown method falls back to the class-level doc plus a `did you mean` from the documented
/// methods. The registry is seeded by `prima-stdlib` at startup and empty in runtime unit tests.
pub(crate) fn native_method_error(class: &str, name: &str, err: RuntimeError) -> RuntimeError {
    let mut notes = Vec::new();
    let mut help = None;
    if let Some(doc) = crate::docs::get_doc(&format!("{class}::{name}")) {
        // Known method: attach its own definition + doc (spec §16.4).
        notes.push(method_doc_note(&doc));
    } else if let Some(cand) = did_you_mean(
        name,
        crate::docs::class_methods(class)
            .iter()
            .map(|d| d.name.as_str()),
    ) && cand != name
    {
        // Unknown method: point at the nearest documented method (its sig + doc) and suggest it.
        help = Some(format!("did you mean `{cand}`?"));
        if let Some(doc) = crate::docs::get_doc(&format!("{class}::{cand}")) {
            notes.push(method_doc_note(&doc));
        } else if let Some(doc) = crate::docs::get_doc(class) {
            notes.push(method_doc_note(&doc));
        }
    } else if let Some(doc) = crate::docs::get_doc(class) {
        notes.push(method_doc_note(&doc));
    }
    with_notes(err, notes, help)
}

/// Best `did you mean` candidate among `candidates` for `attempted` (spec §16.4): the closest name
/// by Levenshtein distance when it is within 2, else any name sharing a non-empty prefix with the
/// attempted one. `None` when no plausible candidate exists.
pub(crate) fn did_you_mean<'a>(
    attempted: &str,
    candidates: impl Iterator<Item = &'a str>,
) -> Option<String> {
    let mut best: Option<(usize, &str)> = None;
    let mut prefixed: Option<&str> = None;
    for c in candidates {
        if c == attempted {
            continue;
        }
        let d = levenshtein(attempted, c);
        if best.as_ref().is_none_or(|(bd, _)| d < *bd) {
            best = Some((d, c));
        }
        if common_prefix_len(attempted, c) > 0 && prefixed.is_none() {
            prefixed = Some(c);
        }
    }
    match best {
        Some((d, c)) if d <= 2 => Some(c.to_string()),
        Some((_, c)) if common_prefix_len(attempted, c) > 0 => Some(c.to_string()),
        _ => prefixed.map(str::to_string),
    }
}

/// Number of leading characters the two strings share.
fn common_prefix_len(a: &str, b: &str) -> usize {
    a.chars().zip(b.chars()).take_while(|(x, y)| x == y).count()
}

/// Classic dynamic-programming Levenshtein edit distance.
fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0usize; b.len() + 1];
    for (i, ca) in a.iter().enumerate() {
        cur[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            cur[j + 1] = (prev[j + 1] + 1)
                .min(cur[j] + 1)
                .min(prev[j] + usize::from(ca != cb));
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

/// Arity guard for builtin functions (spec §16): wrong argument counts are `Message` errors.
pub(crate) fn check_arity(name: &str, args: &[Value], n: usize) -> Result<(), RuntimeError> {
    if args.len() == n {
        Ok(())
    } else {
        crate::error::err(format!(
            "`{name}` expects {n} argument(s), got {}",
            args.len()
        ))
    }
}

/// Every element of an array must be numeric for elementwise operations/broadcast (spec §11.4, `R0009`).
pub(crate) fn require_numeric_array(elems: &[Value]) -> Result<Vec<Number>, RuntimeError> {
    let mut out = Vec::with_capacity(elems.len());
    for e in elems {
        match e {
            Value::Number(n) => out.push(n.clone()),
            _ => return crate::error::err("array elements must be numeric (R0009)"),
        }
    }
    Ok(out)
}

/// Normalize an index against a length: negative indices count from the end; out-of-range → `None`
/// (spec §11.3, `R0003`).
pub(crate) fn normalize_index(idx: i64, len: usize) -> Option<usize> {
    let len_i = len as i64;
    let i = if idx < 0 { len_i + idx } else { idx };
    if i < 0 || i >= len_i {
        None
    } else {
        Some(i as usize)
    }
}

/// Normalize an insert position: `idx == len` is allowed (append); out-of-range → `None`.
pub(crate) fn normalize_insert(idx: i64, len: usize) -> Option<usize> {
    let len_i = len as i64;
    let i = if idx < 0 { len_i + idx } else { idx };
    if i < 0 || i > len_i {
        None
    } else {
        Some(i as usize)
    }
}

/// Remainder for the `%` operator (spec §11.4 elementwise Mod): exact for integers, f64 otherwise.
pub(crate) fn number_mod(x: &Number, y: &Number) -> Result<Number, RuntimeError> {
    if y.is_zero() {
        return crate::error::err("modulo by zero");
    }
    if let (Some(a), Some(b)) = (x.as_bigint(), y.as_bigint()) {
        return Ok(Number::Integer(a % b));
    }
    Ok(Number::Real(Real::F64(x.to_f64_lossy() % y.to_f64_lossy())))
}

/// Mutating array methods (spec §11.3): these require the receiver to be a single-segment path.
pub(crate) fn is_mutating_array_method(name: &str) -> bool {
    matches!(
        name,
        "push" | "pop" | "append" | "extend" | "insert" | "remove" | "clear" | "sort" | "reverse"
    )
}

/// Mutating dict methods (spec §11.6).
pub(crate) fn is_mutating_dict_method(name: &str) -> bool {
    matches!(
        name,
        "insert" | "remove" | "clear" | "update" | "setdefault" | "popitem"
    )
}

/// Mutating set methods (spec §11.6).
pub(crate) fn is_mutating_set_method(name: &str) -> bool {
    matches!(
        name,
        "add" | "remove" | "discard" | "pop" | "clear" | "update"
    )
}

/// Map numeric method names to their collapse-family builtin (spec §9): `x.to_f64()`, `x.rounded(3)`, `x.truncated()`, `x.abs()`.
pub(crate) fn numeric_method_name(name: &str) -> String {
    match name {
        "to_f64" => "to_f64".into(),
        "to_f32" => "to_f32".into(),
        "to_i8" => "to_i8".into(),
        "to_i16" => "to_i16".into(),
        "to_i32" => "to_i32".into(),
        "to_i64" => "to_i64".into(),
        "to_i128" => "to_i128".into(),
        "to_u8" => "to_u8".into(),
        "to_u16" => "to_u16".into(),
        "to_u32" => "to_u32".into(),
        "to_u64" => "to_u64".into(),
        "to_u128" => "to_u128".into(),
        "to_isize" => "to_isize".into(),
        "to_usize" => "to_usize".into(),
        "to_bigint" => "to_bigint".into(),
        "to_rational" => "to_rational".into(),
        "rounded" => "rounded_f64".into(),
        "truncated" => "truncated_i32".into(),
        _ => name.into(),
    }
}

/// Apply an f-string `:spec` refinement (spec §18.1): a `.N` precision formats float values to N
/// decimal places; `[[fill]align][width]` pads/aligns the rendered text (Python `format`
/// mini-language subset, with the leading-`0` zero-pad flag). Unknown or non-numeric forms are
/// left untouched rather than breaking the interpolation.
pub(crate) fn apply_spec(v: &Value, text: &str, spec: Option<&str>) -> String {
    let Some(spec) = spec else {
        return text.to_owned();
    };
    let spec: Vec<char> = spec.trim().chars().collect();
    if spec.is_empty() {
        return text.to_owned();
    }
    let mut i = 0;
    let mut fill = ' ';
    let mut align = None;
    if let Some(&c) = spec.first() {
        if matches!(c, '<' | '>' | '^') {
            align = Some(c);
            i = 1;
        } else if let Some(&a) = spec.get(1)
            && matches!(a, '<' | '>' | '^')
        {
            fill = c;
            align = Some(a);
            i = 2;
        }
    }
    if spec.get(i) == Some(&'0') {
        fill = '0';
        i += 1;
    }
    let mut width = 0usize;
    while let Some(&c) = spec.get(i) {
        if c.is_ascii_digit() {
            width = width * 10 + (c as usize - b'0' as usize);
            i += 1;
        } else {
            break;
        }
    }
    let mut precision = None;
    if spec.get(i) == Some(&'.') {
        i += 1;
        let mut p = 0usize;
        let mut any = false;
        while let Some(&c) = spec.get(i) {
            if c.is_ascii_digit() {
                p = p * 10 + (c as usize - b'0' as usize);
                any = true;
                i += 1;
            } else {
                break;
            }
        }
        if any {
            precision = Some(p);
        }
    }
    let mut body = text.to_owned();
    if let Some(p) = precision
        && let Value::Number(n) = v
    {
        match n {
            Number::Real(Real::F64(f)) => body = format!("{f:.p$}"),
            Number::Real(Real::F32(f)) => body = format!("{f:.p$}"),
            _ => {}
        }
    }
    if body.len() >= width {
        return body;
    }
    let pad = width - body.len();
    let fill = fill.to_string();
    // Python `format` default alignment: right for numbers (and zero-padding implies right),
    // left otherwise.
    let is_number = matches!(v, Value::Number(_));
    let align = align
        .or(if fill == "0" || is_number {
            Some('>')
        } else {
            None
        })
        .unwrap_or('<');
    match align {
        '>' => format!("{}{}", fill.repeat(pad), body),
        '^' => {
            let left = pad / 2;
            let right = pad - left;
            format!("{}{}{}", fill.repeat(left), body, fill.repeat(right))
        }
        _ => format!("{body}{}", fill.repeat(pad)),
    }
}

pub(crate) fn is_zero_literal(e: &Expr) -> bool {
    matches!(&e.kind, ExprKind::Literal(Literal::Integer(s)) if s == "0")
}

pub(crate) fn literal_value(e: &Expr) -> Option<Value> {
    match &e.kind {
        ExprKind::Literal(Literal::Integer(s)) => s
            .parse::<BigInt>()
            .ok()
            .map(Number::Integer)
            .map(Value::Number),
        ExprKind::Literal(Literal::Bool(b)) => Some(Value::Bool(*b)),
        ExprKind::Literal(Literal::String { value, .. }) => Some(Value::String(value.clone())),
        ExprKind::Unary {
            op: UnOp::Neg,
            operand,
        } => literal_value(operand).map(|v| match v {
            Value::Number(n) => Value::Number(-n),
            other => other,
        }),
        _ => None,
    }
}

pub(crate) fn syntax_err(e: SyntaxError) -> RuntimeError {
    RuntimeError::Message(format!("syntax error: {}", e.message))
}

pub(crate) fn syntax_errors(errors: Vec<SyntaxError>) -> RuntimeError {
    match errors.first() {
        Some(e) => syntax_err(e.clone()),
        None => RuntimeError::Message("syntax error".into()),
    }
}
