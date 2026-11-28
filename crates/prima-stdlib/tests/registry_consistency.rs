//! `@builtin` signature/registration consistency (spec §18.4).
//!
//! Every `@builtin` declaration in an embedded `.pra` signature module must have a Rust
//! implementation registered under the matching key, and every registered implementation must
//! correspond to a declaration. This prevents a signature from silently failing at runtime
//! (`E0055`) and catches stale registrations after a `.pra` rename.

use std::collections::BTreeSet;

use prima_runtime::stdlib::{all_module_sources, registered_impl_keys};
use prima_syntax::ast::{ClassMemberKind, Stmt};
use prima_syntax::parse;

#[test]
fn declared_builtins_are_registered() {
    prima_stdlib::init();
    let mut declared_top = BTreeSet::new();
    let mut declared_all = BTreeSet::new();
    for (module, source) in all_module_sources() {
        let program = parse(source).unwrap_or_else(|errors| {
            panic!("embedded stdlib module `{module}` does not parse: {errors:?}")
        });
        for stmt in &program.stmts {
            collect_keys(module.as_str(), stmt, &mut declared_all, &mut declared_top);
        }
    }
    let registered: BTreeSet<String> = registered_impl_keys().into_iter().collect();

    // Top-level `@builtin` functions must have a registered Rust implementation.
    let missing: Vec<&String> = declared_top.difference(&registered).collect();
    assert!(
        missing.is_empty(),
        "`@builtin` declarations without a Rust implementation: {missing:#?}"
    );

    // Every registration must correspond to a `.pra` declaration (top-level or class method).
    let stale: Vec<&String> = registered.difference(&declared_all).collect();
    assert!(
        stale.is_empty(),
        "registered `@builtin` implementations without a `.pra` declaration: {stale:#?}"
    );
}

/// Collect all builtin keys and the subset declared as top-level `fn`/`let` items.
fn collect_keys(
    module: &str,
    stmt: &Stmt,
    all: &mut BTreeSet<String>,
    top_level: &mut BTreeSet<String>,
) {
    match stmt {
        Stmt::Pub(inner) => collect_keys(module, inner, all, top_level),
        Stmt::FnDef {
            name, annotations, ..
        } => {
            if annotations.iter().any(|a| a.is_builtin()) {
                let key = format!("{module}::{}", name.value);
                all.insert(key.clone());
                top_level.insert(key);
            }
        }
        Stmt::MathDef {
            name, annotations, ..
        } => {
            if annotations.iter().any(|a| a.is_builtin()) {
                let key = format!("{module}::{}", name.value);
                all.insert(key.clone());
                top_level.insert(key);
            }
        }
        Stmt::ClassDef { name, members, .. } => {
            for member in members {
                let ClassMemberKind::Method {
                    name: method,
                    annotations,
                    ..
                } = &member.kind
                else {
                    continue;
                };
                if annotations.iter().any(|a| a.is_builtin()) {
                    all.insert(format!("{}::{}", name.value, method.value));
                }
            }
        }
        _ => {}
    }
}

#[test]
fn declared_builtin_tiers_match_registered_metadata() {
    prima_stdlib::init();
    let mut mismatched = Vec::new();
    for (module, source) in all_module_sources() {
        let program = parse(source).expect("embedded stdlib module parses");
        for stmt in &program.stmts {
            collect_tier_mismatches(&module, stmt, &mut mismatched);
        }
    }
    assert!(mismatched.is_empty(), "{mismatched:#?}");
}

fn collect_tier_mismatches(module: &str, stmt: &Stmt, out: &mut Vec<String>) {
    match stmt {
        Stmt::Pub(inner) => collect_tier_mismatches(module, inner, out),
        Stmt::FnDef {
            name, annotations, ..
        } => check_tier(module, &name.value, annotations, out),
        Stmt::MathDef {
            name, annotations, ..
        } => check_tier(module, &name.value, annotations, out),
        Stmt::ClassDef { name, members, .. } => {
            for member in members {
                let ClassMemberKind::Method {
                    name: method,
                    annotations,
                    ..
                } = &member.kind
                else {
                    continue;
                };
                check_tier(
                    &format!("{}::{}", name.value, method.value),
                    "",
                    annotations,
                    out,
                );
            }
        }
        _ => {}
    }
}

fn check_tier(
    key: &str,
    name: &str,
    annotations: &[prima_syntax::ast::Annotation],
    out: &mut Vec<String>,
) {
    let declared = annotations
        .iter()
        .find(|a| a.is_builtin())
        .map(|a| a.builtin_level());
    let Some(declared) = declared else {
        return;
    };
    let key = if name.is_empty() {
        key.to_string()
    } else {
        format!("{key}::{name}")
    };
    let registered = prima_runtime::stdlib::get_impl_level(&key);
    if registered != declared {
        out.push(format!(
            "`{key}`: `.pra` declares `O{declared}`, Rust registration records `O{registered}`"
        ));
    }
}
