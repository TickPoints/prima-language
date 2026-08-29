//! Populate the runtime doc registry for the native builtin classes (`String`/`Array`/`Dict`/`Set`/
//! `Number`/`Char`/`Tuple`/`Option`/`Result`, spec §18.1) from their embedded `core::<class>` modules.
//!
//! Each `modules/*.pra` source is the single source of truth: its `///` comments and signatures are
//! parsed once at startup and registered under `<Class>::<method>` keys (plus the class-level
//! `<Class>` key) so diagnostics can attach a signature, definition location, and doc note to a
//! failed call (spec §16.4) and `prima doc` lists them offline (spec §20).

use prima_runtime::docs::{MethodDoc, register_doc};
use prima_syntax::ast::{ClassMemberKind, Stmt, Type};
use prima_syntax::parse;

/// All embedded builtin-class modules, as `(class name, display path, source)`.
const CLASS_MODULES: &[(&str, &str, &str)] = &[
    (
        "String",
        "core/string.pra",
        include_str!("modules/string.pra"),
    ),
    ("Array", "core/array.pra", include_str!("modules/array.pra")),
    ("Dict", "core/dict.pra", include_str!("modules/dict.pra")),
    ("Set", "core/set.pra", include_str!("modules/set.pra")),
    (
        "Number",
        "core/number.pra",
        include_str!("modules/number.pra"),
    ),
    ("Char", "core/char.pra", include_str!("modules/char.pra")),
    ("Tuple", "core/tuple.pra", include_str!("modules/tuple.pra")),
    (
        "Option",
        "core/option.pra",
        include_str!("modules/option.pra"),
    ),
    (
        "Result",
        "core/result.pra",
        include_str!("modules/result.pra"),
    ),
];

/// Parse every embedded builtin-class module and register one `MethodDoc` per member.
///
/// A parse failure is non-fatal: `init` must never panic on a broken embedded source, so the
/// registry is simply left empty for that module.
pub fn register() {
    for (class, display, src) in CLASS_MODULES {
        let Ok(program) = parse(src) else { continue };
        for stmt in &program.stmts {
            let Stmt::ClassDef {
                name,
                members,
                docs,
                ..
            } = stmt
            else {
                continue;
            };
            if name.value != *class {
                continue;
            }
            // Class-level doc (spec §4.1): the `///` above `class <Class>`.
            register_doc(
                *class,
                MethodDoc {
                    name: (*class).into(),
                    sig: (*class).into(),
                    doc: docs.as_ref().map(|d| d.text()),
                    defined_at: format!("{display}:1:1"),
                },
            );
            for member in members {
                let ClassMemberKind::Method {
                    name, params, ret, ..
                } = &member.kind
                else {
                    continue;
                };
                let key = format!("{class}::{}", name.value);
                register_doc(
                    key,
                    MethodDoc {
                        name: name.value.clone(),
                        sig: render_sig(&name.value, params, ret),
                        doc: member.docs.as_ref().map(|d| d.text()),
                        defined_at: line_col(display, src, member.span.start),
                    },
                );
            }
        }
    }
}

/// Render a method signature, e.g. `to_upper(self)` or `len(self) -> Integer`. A method without a
/// declared return type is rendered with `-> Self` (methods chain, spec §4.5).
fn render_sig(name: &str, params: &[prima_syntax::ast::Param], ret: &Option<Type>) -> String {
    let mut sig = String::new();
    sig.push_str(name);
    sig.push('(');
    for (i, p) in params.iter().enumerate() {
        if i > 0 {
            sig.push_str(", ");
        }
        if p.is_self {
            sig.push_str("self");
        } else {
            sig.push_str(&p.name.value);
            if let Some(t) = &p.type_ann {
                sig.push_str(": ");
                render_type(t, &mut sig);
            }
        }
    }
    sig.push(')');
    match ret {
        Some(t) => {
            sig.push_str(" -> ");
            render_type(t, &mut sig);
        }
        None => sig.push_str(" -> Self"),
    }
    sig
}

/// Render a `Type` back to source text (mirrors the printer in `src/fmt.rs`, kept local so the
/// doc registry does not depend on the CLI crate).
fn render_type(t: &Type, out: &mut String) {
    match t {
        Type::Number => out.push_str("Number"),
        Type::Integer => out.push_str("Integer"),
        Type::Rational => out.push_str("Rational"),
        Type::F64 => out.push_str("F64"),
        Type::F32 => out.push_str("F32"),
        Type::I8 => out.push_str("I8"),
        Type::I16 => out.push_str("I16"),
        Type::I32 => out.push_str("I32"),
        Type::I64 => out.push_str("I64"),
        Type::I128 => out.push_str("I128"),
        Type::U8 => out.push_str("U8"),
        Type::U16 => out.push_str("U16"),
        Type::U32 => out.push_str("U32"),
        Type::U64 => out.push_str("U64"),
        Type::U128 => out.push_str("U128"),
        Type::Isize => out.push_str("Isize"),
        Type::Usize => out.push_str("Usize"),
        Type::Complex => out.push_str("Complex"),
        Type::Expr => out.push_str("Expr"),
        Type::Symbol => out.push_str("Symbol"),
        Type::Bool => out.push_str("Bool"),
        Type::String => out.push_str("String"),
        Type::Char => out.push_str("Char"),
        Type::Array(t) => {
            out.push_str("Array<");
            render_type(t, out);
            out.push('>');
        }
        Type::Matrix(t) => {
            out.push_str("Matrix<");
            render_type(t, out);
            out.push('>');
        }
        Type::Tuple(ts) => {
            out.push_str("Tuple<");
            render_type_list(ts, out);
            out.push('>');
        }
        Type::Option(t) => {
            out.push_str("Option<");
            render_type(t, out);
            out.push('>');
        }
        Type::Result(a, b) => {
            out.push_str("Result<");
            render_type(a, out);
            out.push_str(", ");
            render_type(b, out);
            out.push('>');
        }
        Type::Fn { params, ret } => {
            out.push_str("Fn(");
            render_type_list(params, out);
            out.push_str(") -> ");
            render_type(ret, out);
        }
        Type::MFn { params, ret } => {
            out.push_str("MFn(");
            render_type_list(params, out);
            out.push_str(") -> ");
            render_type(ret, out);
        }
        Type::SelfType => out.push_str("Self"),
        Type::User(name) => out.push_str(&name.value),
    }
}

fn render_type_list(types: &[Type], out: &mut String) {
    for (i, t) in types.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        render_type(t, out);
    }
}

/// Render `<display>:<line>:<col>` for a byte offset (both 1-based): line counts newlines before
/// the offset, column counts characters since the last newline.
fn line_col(display: &str, src: &str, offset: u32) -> String {
    let offset = (offset as usize).min(src.len());
    let before = &src[..offset];
    let line = before.bytes().filter(|&b| b == b'\n').count() + 1;
    let col = before
        .rfind('\n')
        .map(|i| before[i + 1..].chars().count() + 1)
        .unwrap_or(before.chars().count() + 1);
    format!("{display}:{line}:{col}")
}
