//! stdlib signature harvesting and call-site validation (spec §18.4, §16.2 `E0050`).
//!
//! A `Signature` is harvested from an embedded `.pra` signature module: parameter types, optional
//! return type, and the doc metadata used for the `E0050` definition note. The table is keyed on
//! fully-qualified `"module::name"` (and any `from`/alias bindings), mirroring how the runtime
//! registers and resolves them.

use std::collections::HashMap;

use prima_syntax::Span;
use prima_syntax::ast::{DocComment, Expr, ImportItem, ImportKind, Program, Spanned, Stmt, Type};
use prima_syntax::parse;

use super::TypeError;
use super::error::push_err_with_note;
use super::infer::{assignable, infer, type_name};
use super::line_col;

/// A stdlib function signature harvested from an embedded `.pra` signature module (spec §18.4):
/// parameter types, optional return type (`None` for void functions), and the doc metadata used for
/// the `E0050` definition note (spec §16.4).
#[derive(Clone)]
pub(crate) struct Signature {
    params: Vec<Type>,
    pub(crate) ret: Option<Type>,
    /// Source-level function name as declared (`Matrix::zeros` keeps its joined path).
    name: String,
    /// `///` doc comment from the signature module (spec §4.1).
    docs: Option<DocComment>,
    /// Display definition location `"<module>.pra:<line>:<col>"`.
    defined_at: String,
    /// Span of the `fn` declaration inside the signature module. Kept for future span-precise
    /// notes; the location string is derived from it at harvest time.
    #[allow(dead_code)]
    span: Span,
}

/// Collect the stdlib `@builtin pub fn` signatures reachable through the program's imports (spec
/// §15.4 import forms, §18.4 signatures). Keys are fully-qualified `"module::name"`; `from`
/// imports additionally expose the imported bare name (and any alias). Flattened `::`-joined item
/// names (e.g. `Matrix::zeros`) are keyed under the joined module path, mirroring how the runtime
/// registers and resolves them (`module.rs`/`eval.rs` `lookup_module_item_flat`).
pub(crate) fn build_signature_table(program: &Program) -> HashMap<String, Vec<Signature>> {
    let mut table: HashMap<String, Vec<Signature>> = HashMap::new();
    for imp in &program.imports {
        let segments: Vec<String> = match &imp.kind {
            ImportKind::Namespace { path, .. } | ImportKind::From { path, .. } => {
                path.iter().map(|s| s.value.clone()).collect()
            }
        };
        let module_key = segments.join("::");
        let Some(src) = crate::stdlib::get_module_source(&module_key) else {
            continue;
        };
        // Embedded sources are ours and known-good; a parse failure just yields no signatures.
        let Ok(parsed) = parse(src) else { continue };
        // Display path of the module, e.g. `linalg` → `linalg.pra`, `sys::path` → `sys/path.pra`.
        let display_file = format!("{}.pra", module_key.replace("::", "/"));
        let mut sigs = Vec::new();
        for stmt in &parsed.stmts {
            let Stmt::Pub(inner) = stmt else { continue };
            let Stmt::FnDef {
                name,
                params,
                ret,
                docs,
                span,
                ..
            } = inner.as_ref()
            else {
                continue;
            };
            let param_types = params
                .iter()
                .map(|p| {
                    p.type_ann.clone().unwrap_or_else(|| {
                        Type::User(Spanned {
                            value: "Value".into(),
                            span: p.name.span,
                        })
                    })
                })
                .collect();
            let (line, column) = line_col(src, span.start);
            sigs.push(Signature {
                params: param_types,
                ret: ret.clone(),
                name: name.value.clone(),
                docs: docs.clone(),
                defined_at: format!("{display_file}:{line}:{column}"),
                span: *span,
            });
        }
        // A name may have multiple signatures (overloads, e.g. `stats::quantile`); keep them all.
        for sig in &sigs {
            table
                .entry(format!("{module_key}::{}", sig.name))
                .or_default()
                .push(sig.clone());
        }
        match &imp.kind {
            ImportKind::From { items, .. } => {
                for sig in &sigs {
                    for item in items {
                        if let ImportItem::Name {
                            name: item_name,
                            alias,
                        } = item
                            && item_name.value == sig.name
                        {
                            // Bind exactly what the runtime binds (eval.rs `bind_imports`): the
                            // alias when present, else the item name.
                            let target = alias
                                .as_ref()
                                .map_or_else(|| item_name.value.clone(), |a| a.value.clone());
                            table.entry(target).or_default().push(sig.clone());
                        }
                    }
                }
            }
            ImportKind::Namespace { alias, .. } => {
                if let Some(a) = alias {
                    for sig in &sigs {
                        table
                            .entry(format!("{}::{}", a.value, sig.name))
                            .or_default()
                            .push(sig.clone());
                    }
                }
            }
        }
    }
    table
}

/// Look up the signatures for a `Path` callee, mirroring the runtime's flattened module-item lookup
/// (`eval.rs` `lookup_module_item_flat`): the joined segments first, then every module prefix.
pub(crate) fn lookup_call_signature<'a>(
    segments: &[Spanned<String>],
    sigs: &'a HashMap<String, Vec<Signature>>,
) -> Option<&'a Vec<Signature>> {
    if segments.is_empty() {
        return None;
    }
    let joined = segments
        .iter()
        .map(|s| s.value.as_str())
        .collect::<Vec<_>>()
        .join("::");
    if let Some(sig) = sigs.get(&joined) {
        return Some(sig);
    }
    for i in 1..segments.len() - 1 {
        let key = format!(
            "{}::{}",
            segments[..i]
                .iter()
                .map(|s| s.value.as_str())
                .collect::<Vec<_>>()
                .join("::"),
            segments[i..]
                .iter()
                .map(|s| s.value.as_str())
                .collect::<Vec<_>>()
                .join("::")
        );
        if let Some(sig) = sigs.get(&key) {
            return Some(sig);
        }
    }
    None
}

/// Whether a call matches one signature (spec §18.4): arity at most the param count (stdlib functions
/// may have optional trailing args) and every provided argument assignable (unknown types never reject).
pub(crate) fn signature_accepts(
    sig: &Signature,
    args: &[Expr],
    sigs: &HashMap<String, Vec<Signature>>,
) -> bool {
    if args.len() > sig.params.len() {
        return false;
    }
    args.iter()
        .enumerate()
        .all(|(i, arg)| assignable(&sig.params[i], &infer(arg, sigs)))
}

/// The definition note attached to an `E0050` error (spec §16.4): the rendered call signature plus
/// the definition location and, when the signature module documents it, the `///` doc text.
pub(crate) fn sig_note(name: &str, sig: &Signature) -> String {
    let params = sig
        .params
        .iter()
        .map(type_name)
        .collect::<Vec<_>>()
        .join(", ");
    let mut note = match &sig.ret {
        Some(t) => format!(
            "function `{name}({params}) -> {}` defined at {}",
            type_name(t),
            sig.defined_at
        ),
        None => format!("function `{name}({params})` defined at {}", sig.defined_at),
    };
    if let Some(docs) = &sig.docs {
        let text = docs.text();
        if !text.is_empty() {
            note.push_str(&format!("\n{text}"));
        }
    }
    note
}

/// Check a call against the harvested stdlib signatures (spec §18.4, §16.2 `E0050`): a call is valid
/// when ANY overload accepts it; positive arity and per-argument type mismatches only — unknown or
/// unresolved types never error. Each `E0050` error carries a definition note (spec §16.4).
pub(crate) fn check_call_signature(
    src: &str,
    call_span: Span,
    segments: &[Spanned<String>],
    args: &[Expr],
    errors: &mut Vec<TypeError>,
    sigs: &HashMap<String, Vec<Signature>>,
) {
    let Some(candidates) = lookup_call_signature(segments, sigs) else {
        return;
    };
    if candidates
        .iter()
        .any(|sig| signature_accepts(sig, args, sigs))
    {
        return;
    }
    let name = segments
        .iter()
        .map(|s| s.value.as_str())
        .collect::<Vec<_>>()
        .join("::");
    // Report against the best-fitting signature: the one with the most parameters that still covers
    // the given arity, else the first candidate (for the arity error).
    let chosen = candidates
        .iter()
        .filter(|sig| args.len() <= sig.params.len())
        .max_by_key(|sig| sig.params.len())
        .or_else(|| candidates.first())
        .expect("candidates is non-empty");
    if args.len() > chosen.params.len() {
        push_err_with_note(
            src,
            errors,
            call_span,
            format!(
                "function `{name}` expects {} argument(s), got {} (E0050)",
                chosen.params.len(),
                args.len()
            ),
            sig_note(&name, chosen),
        );
        return;
    }
    for (i, arg) in args.iter().enumerate().take(chosen.params.len()) {
        let got = infer(arg, sigs);
        if !assignable(&chosen.params[i], &got) {
            push_err_with_note(
                src,
                errors,
                arg.span,
                format!(
                    "argument {} of `{name}` expects {}, got {} (E0050)",
                    i + 1,
                    type_name(&chosen.params[i]),
                    got
                ),
                sig_note(&name, chosen),
            );
            return;
        }
    }
}
