//! C ABI export analysis and header generation (spec §18.4, appendix B.6).
//!
//! `@c_api::extern` marks a `pub fn` to be exported with the C calling convention; the checker
//! (`check_src`, E0071/E0072) validates the function before `collect_exports` builds prototypes.

use prima_syntax::ast::{Annotation, Program, Stmt, Type};

/// A single validated `@c_api::extern` export: the pieces of a C prototype line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CExtern {
    pub name: String,
    /// `(parameter name, C type)` pairs in declaration order.
    pub params: Vec<(String, String)>,
    /// C return type; `void` for `c_api::unit` or an absent return annotation.
    pub ret: String,
}

/// Collect the `@c_api::extern` functions of a program into C-ABI prototypes (spec §18.4).
///
/// Only `pub` functions are exported; the program must already have passed `check_src` validation
/// (E0071/E0072) — non-C-compatible types are skipped defensively.
pub fn collect_exports(program: &Program) -> Vec<CExtern> {
    let mut out = Vec::new();
    for stmt in &program.stmts {
        collect_stmt_export(stmt, &mut out);
    }
    out
}

fn collect_stmt_export(stmt: &Stmt, out: &mut Vec<CExtern>) {
    let inner = match stmt {
        Stmt::Pub(inner) => inner.as_ref(),
        _ => return,
    };
    if let Stmt::FnDef { name, params, ret, annotations, .. } = inner
        && annotations.contains(&Annotation::CApiExtern)
    {
        let params = params
            .iter()
            .filter_map(|p| {
                let ty = p.type_ann.as_ref()?;
                let c = c_type(ty)?;
                if c == "void" {
                    return None;
                }
                Some((p.name.value.clone(), c))
            })
            .collect();
        let ret = ret.as_ref().and_then(c_type).unwrap_or_else(|| "void".into());
        out.push(CExtern { name: name.value.clone(), params, ret });
    }
}

/// Map a Prima type to its C ABI representation (spec appendix B.6):
/// `c_api::int` → `int`, `c_api::cstring` → `const char*`, `c_api::unit` → `void`, etc.
/// Returns `None` for types that are not C-compatible.
pub fn c_type(t: &Type) -> Option<String> {
    let name = match t {
        Type::User(sp) => sp.value.as_str(),
        _ => return None,
    };
    let c = match name {
        "c_api::int" => "int",
        "c_api::uint" => "unsigned int",
        "c_api::long" => "long",
        "c_api::long_long" => "long long",
        "c_api::float" => "float",
        "c_api::double" => "double",
        "c_api::bool" => "bool",
        "c_api::char" => "char",
        "c_api::cstring" => "const char*",
        "c_api::ptr" => "void*",
        "c_api::unit" => "void",
        _ => return None,
    };
    Some(c.into())
}

/// Render an include-guarded C header declaring the prototypes of the given exports.
pub fn render_header(exports: &[CExtern]) -> String {
    let mut s = String::new();
    s.push_str("#ifndef PRIMA_EXPORT_H\n");
    s.push_str("#define PRIMA_EXPORT_H\n\n");
    s.push_str("#ifdef __cplusplus\nextern \"C\" {\n#endif\n");
    for e in exports {
        let params: Vec<String> = e.params.iter().map(|(name, c)| format!("{c} {name}")).collect();
        s.push_str(&format!("{} {}({});\n", e.ret, e.name, params.join(", ")));
    }
    s.push_str("\n#ifdef __cplusplus\n}\n#endif\n\n#endif /* PRIMA_EXPORT_H */\n");
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use prima_syntax::parse;

    /// Parse a type annotation into a `Type`, e.g. `"c_api::double"` from `let x: c_api::double = 0;`.
    fn ty(src: &str) -> Type {
        let program = parse(&format!("let x: {src} = 0;")).unwrap();
        match &program.stmts[0] {
            Stmt::Let { type_ann: Some(t), .. } => t.clone(),
            _ => panic!("expected a type annotation"),
        }
    }

    #[test]
    fn c_type_mapping_covers_appendix_b6() {
        let cases = [
            ("c_api::int", Some("int")),
            ("c_api::uint", Some("unsigned int")),
            ("c_api::long", Some("long")),
            ("c_api::long_long", Some("long long")),
            ("c_api::float", Some("float")),
            ("c_api::double", Some("double")),
            ("c_api::bool", Some("bool")),
            ("c_api::char", Some("char")),
            ("c_api::cstring", Some("const char*")),
            ("c_api::ptr", Some("void*")),
            ("c_api::unit", Some("void")),
        ];
        for (prima, c) in cases {
            assert_eq!(c_type(&ty(prima)).as_deref(), c, "prima type {prima}");
        }
    }

    #[test]
    fn non_c_types_map_to_none() {
        assert_eq!(c_type(&ty("Integer")), None);
        assert_eq!(c_type(&ty("F64")), None);
        assert_eq!(c_type(&ty("Array<Integer>")), None);
        assert_eq!(c_type(&Type::Bool), None);
        assert_eq!(c_type(&Type::Char), None);
    }

    #[test]
    fn render_header_guard_and_prototypes() {
        let exports = vec![
            CExtern {
                name: "add".into(),
                params: vec![("a".into(), "double".into()), ("b".into(), "double".into())],
                ret: "double".into(),
            },
            CExtern { name: "hello".into(), params: vec![("a".into(), "int".into())], ret: "void".into() },
        ];
        let expected = "#ifndef PRIMA_EXPORT_H\n#define PRIMA_EXPORT_H\n\n#ifdef __cplusplus\nextern \"C\" {\n#endif\ndouble add(double a, double b);\nvoid hello(int a);\n\n#ifdef __cplusplus\n}\n#endif\n\n#endif /* PRIMA_EXPORT_H */\n";
        assert_eq!(render_header(&exports), expected);
    }

    #[test]
    fn collect_exports_only_pub_c_api_extern() {
        let program = parse(
            "@c_api::extern\npub fn add(a: c_api::double, b: c_api::double) -> c_api::double { return a + b; }\n\
             @c_api::extern\nfn hidden(a: c_api::int) { return a; }",
        )
        .unwrap();
        let exports = collect_exports(&program);
        assert_eq!(exports.len(), 1);
        assert_eq!(exports[0].name, "add");
        assert_eq!(exports[0].ret, "double");
        assert_eq!(exports[0].params, vec![("a".into(), "double".into()), ("b".into(), "double".into())]);
    }
}
