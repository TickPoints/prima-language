//! `@builtin` and `@c_api::extern` interop validation (spec §18.4): static checks E0055/E0056/
//! E0071/E0072 plus the C ABI export list and header rendering.

use prima_runtime::capi::{CExtern, collect_exports, render_header};
use prima_runtime::check::check_src;
use prima_syntax::parse;

fn messages(src: &str) -> Vec<String> {
    check_src(src).into_iter().map(|e| e.message).collect()
}

#[test]
fn c_api_extern_pub_export_passes_validation_and_collects() {
    let src = "@c_api::extern\npub fn add(a: c_api::double, b: c_api::double) -> c_api::double { return a + b; }";
    assert!(
        check_src(src).is_empty(),
        "expected no errors, got {:?}",
        messages(src)
    );
    let exports = collect_exports(&parse(src).unwrap());
    assert_eq!(
        exports,
        vec![CExtern {
            name: "add".into(),
            params: vec![("a".into(), "double".into()), ("b".into(), "double".into())],
            ret: "double".into(),
        }]
    );
}

#[test]
fn c_api_extern_render_header_is_valid_c() {
    let src = "@c_api::extern\npub fn add(a: c_api::double, b: c_api::double) -> c_api::double { return a + b; }";
    let header = render_header(&collect_exports(&parse(src).unwrap()));
    assert!(header.starts_with("#ifndef PRIMA_EXPORT_H\n#define PRIMA_EXPORT_H\n"));
    assert!(header.contains("double add(double a, double b);"));
    assert!(header.contains("extern \"C\" {"));
    assert!(header.ends_with("#endif /* PRIMA_EXPORT_H */\n"));
}

#[test]
fn non_pub_c_api_extern_is_e0072() {
    let errs = messages("@c_api::extern\nfn hidden(a: c_api::int) { return a; }");
    assert_eq!(errs.len(), 1, "expected exactly one error, got {errs:?}");
    assert!(errs[0].contains("E0072"));
}

#[test]
fn non_c_compatible_param_is_e0071() {
    let errs = messages("@c_api::extern\npub fn bad(a: Integer) -> c_api::int { return 0; }");
    assert_eq!(errs.len(), 1, "expected exactly one error, got {errs:?}");
    assert!(errs[0].contains("E0071"));
}

#[test]
fn c_api_unit_as_parameter_is_e0071() {
    let errs = messages("@c_api::extern\npub fn bad(u: c_api::unit) -> c_api::int { return 0; }");
    assert_eq!(errs.len(), 1, "expected exactly one error, got {errs:?}");
    assert!(errs[0].contains("E0071"));
}

#[test]
fn c_api_unit_return_is_allowed() {
    assert!(
        check_src("@c_api::extern\npub fn hello(a: c_api::int) -> c_api::unit { return; }")
            .is_empty()
    );
}

#[test]
fn builtin_fn_with_body_is_e0056() {
    let errs = messages("@builtin fn sqrt2(x) { return x; }");
    assert_eq!(errs.len(), 1, "expected exactly one error, got {errs:?}");
    assert!(errs[0].contains("E0056"));
}

#[test]
fn unregistered_builtin_fn_is_e0055() {
    let errs = messages("@builtin fn zzzzz_unknown(x);");
    assert_eq!(errs.len(), 1, "expected exactly one error, got {errs:?}");
    assert!(errs[0].contains("E0055"));
    assert!(errs[0].contains("zzzzz_unknown"));
}

#[test]
fn registered_builtin_signature_only_passes() {
    assert!(check_src("@builtin fn sqrt(x);").is_empty());
}

#[test]
fn builtin_class_is_e0055() {
    let errs = messages("@builtin class Foo { x: Integer }");
    assert_eq!(errs.len(), 1, "expected exactly one error, got {errs:?}");
    assert!(errs[0].contains("E0055"));
    assert!(errs[0].contains("Foo"));
}
