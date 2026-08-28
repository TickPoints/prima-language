//! `prima compile --emit-c-abi` (spec §18.4/§19.3): build a real shared library that re-exports
//! every `@c_api::extern` function of a `.pra` file as a C-callable symbol, plus its C header.
//!
//! Approach (decided by the maintainer): generate a temporary *shell crate* — a `cdylib` whose only
//! code is one `extern "C"` wrapper per export. Each wrapper marshals the C arguments into
//! `prima_core::Value`s, invokes the interpreter through [`prima_runtime::capi::call_file_export`],
//! and converts the result back to the C ABI. The exported `.pra` file is re-evaluated per process;
//! the runtime caches the evaluated module per thread so repeated calls do not re-parse it.
//!
//! The generated `Cargo.toml` points at the workspace's `prima-runtime`/`prima-core` by absolute
//! path; their own path/`workspace` dependencies resolve against the real crate manifests, so the
//! shell crate builds even though it lives outside the workspace.

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

/// C type string → Rust wrapper signature type. All types come from `std::os::raw`, so the
/// generated crate needs no external dependencies beyond `prima-runtime`/`prima-core`.
fn c_type_rust(cty: &str) -> String {
    match cty {
        "int" => "c_int".into(),
        "unsigned int" => "c_uint".into(),
        "long" => "c_long".into(), // LP64: 64-bit; `c_long` follows the target ABI
        "long long" => "c_longlong".into(),
        "float" => "c_float".into(),
        "double" => "c_double".into(),
        "bool" => "bool".into(),
        "char" => "c_char".into(),
        "const char*" => "*const c_char".into(),
        "void*" => "*mut c_void".into(),
        "void" => "()".into(),
        other => other.into(),
    }
}

/// Rust type for the default value in a wrapper's error path (0/false/null).
fn c_type_default(cty: &str) -> String {
    match cty {
        "bool" => "false".into(),
        "const char*" => "std::ptr::null()".into(),
        "void*" => "std::ptr::null_mut()".into(),
        "void" => "()".into(),
        "double" | "float" => "0.0".into(),
        _ => "0".into(),
    }
}

/// Convert an already-called `Value` to the C return type.
fn c_return_expr(cty: &str) -> String {
    match cty {
        "bool" => "value_f64(&v) != 0.0".into(),
        "const char*" => "value_cstr(&v)".into(),
        "void*" => "std::ptr::null_mut()".into(),
        "void" => "()".into(),
        _ => format!("value_f64(&v) as {}", c_type_rust(cty)),
    }
}

/// Marshal a single C argument into a `Value` (spec §18.4): numerics cross lossily as `f64`,
/// `const char*` becomes a `String`; an opaque `void*` cannot be represented and becomes `Nil`.
fn c_arg_push(name: &str, cty: &str) -> String {
    match cty {
        "bool" => {
            format!(
                "    args.push(Value::Number(Number::Real(Real::F64(if {name} {{ 1.0 }} else {{ 0.0 }}))));\n"
            )
        }
        "const char*" => format!("    args.push(Value::String(cstr_arg({name})));\n"),
        "void*" => format!("    let _ = {name};\n    args.push(Value::Nil);\n"),
        _ => format!("    args.push(Value::Number(Number::Real(Real::F64({name} as f64))));\n"),
    }
}

/// Body that dispatches `call_file_export` and converts the result (or reports an error).
fn call_body(e: &prima_runtime::capi::CExtern) -> String {
    let call = format!(
        "call_file_export(Path::new(PRIMA_SRC_PATH), {:?}, args)",
        e.name
    );
    if e.ret == "void" {
        format!(
            "    if let Err(e) = {call} {{\n        eprintln!(\"prima: export `{}` failed: {{e}}\");\n    }}\n",
            e.name
        )
    } else {
        // `void*` has no value conversion, so the success arm discards the returned `Value`.
        let (ok_pat, ok_expr) = if e.ret == "void*" {
            ("Ok(_)", "std::ptr::null_mut()".to_string())
        } else {
            ("Ok(v)", c_return_expr(&e.ret))
        };
        format!(
            "    match {call} {{\n        {ok_pat} => {ok_expr},\n        Err(e) => {{ eprintln!(\"prima: export `{}` failed: {{e}}\"); {} }}\n    }}\n",
            e.name,
            c_type_default(&e.ret),
        )
    }
}

/// C type string → the `std::os::raw` name it needs imported (or `None` for `bool`/`void`).
fn raw_type_for(cty: &str) -> Option<&'static str> {
    match cty {
        "int" => Some("c_int"),
        "unsigned int" => Some("c_uint"),
        "long" => Some("c_long"),
        "long long" => Some("c_longlong"),
        "float" => Some("c_float"),
        "double" => Some("c_double"),
        "char" => Some("c_char"),
        "const char*" => Some("c_char"),
        "void*" => Some("c_void"),
        _ => None, // bool / void / opaque
    }
}

/// The `use std::os::raw::{...};` line restricted to the types the wrappers actually use
/// (`c_char` is always included: the string marshalling helpers use it).
fn raw_type_import_line(exports: &[prima_runtime::capi::CExtern]) -> String {
    let mut used: Vec<&'static str> = exports
        .iter()
        .flat_map(|e| {
            e.params
                .iter()
                .map(|(_, cty)| cty.as_str())
                .chain(std::iter::once(e.ret.as_str()))
        })
        .filter_map(raw_type_for)
        .collect();
    if !used.contains(&"c_char") {
        used.push("c_char");
    }
    used.sort_unstable();
    used.dedup();
    format!("use std::os::raw::{{{}}};\n", used.join(", "))
}

/// One `#[unsafe(no_mangle)] pub extern "C"` wrapper for a single export.
fn gen_wrapper(e: &prima_runtime::capi::CExtern) -> String {
    let params: Vec<String> = e
        .params
        .iter()
        .map(|(name, cty)| format!("{name}: {}", c_type_rust(cty)))
        .collect();
    let ret = c_type_rust(&e.ret);
    let mut s = String::new();
    s.push_str("#[unsafe(no_mangle)]\n");
    s.push_str(&format!(
        "pub extern \"C\" fn {}({}) -> {ret} {{\n",
        e.name,
        params.join(", ")
    ));
    s.push_str("    let mut args = Vec::new();\n");
    for (name, cty) in &e.params {
        s.push_str(&c_arg_push(name, cty));
    }
    s.push_str(&call_body(e));
    s.push_str("}\n");
    s
}

/// The full `src/lib.rs` of the shell crate.
fn gen_lib_source(exports: &[prima_runtime::capi::CExtern], src_path: &str) -> String {
    let mut s = String::new();
    s.push_str("//! C ABI export shell (generated by `prima compile --emit-c-abi`).\n");
    s.push_str("//!\n");
    s.push_str(
        "//! Each `#[unsafe(no_mangle)] pub extern \"C\"` wrapper marshals the C arguments into\n",
    );
    s.push_str("//! `prima_core::Value`s, evaluates the exporting `.pra` module through\n");
    s.push_str(
        "//! `prima_runtime::capi::call_file_export` (spec §18.4), and converts the result back\n",
    );
    s.push_str("//! to the C ABI. Numeric values cross the boundary lossily as `f64`.\n\n");
    s.push_str("use std::cell::RefCell;\n");
    s.push_str("use std::ffi::{CStr, CString};\n");
    s.push_str(&raw_type_import_line(exports));
    s.push_str("use std::path::Path;\n\n");
    s.push_str("use prima_core::{Number, Real, Value};\n");
    s.push_str("use prima_runtime::capi::call_file_export;\n\n");
    s.push_str(
        "/// Absolute path of the `.pra` module whose exports this library wraps. If the file is\n",
    );
    s.push_str("/// moved after compilation the wrappers fail at call time (message on stderr, default value).\n");
    s.push_str(&format!("const PRIMA_SRC_PATH: &str = {src_path:?};\n\n"));
    s.push_str(
        "// String results must outlive the wrapper's return frame; keep the buffers in a\n",
    );
    s.push_str(
        "// thread-local that is replaced on every call (previous buffers stay alive until the\n",
    );
    s.push_str("// next call).\n");
    s.push_str("thread_local! {\n");
    s.push_str(
        "    static CSTR_KEEP: RefCell<Vec<CString>> = const { RefCell::new(Vec::new()) };\n",
    );
    s.push_str("}\n\n");
    s.push_str("fn value_f64(v: &Value) -> f64 {\n");
    s.push_str("    match v {\n");
    s.push_str("        Value::Number(n) => n.to_f64_lossy(),\n");
    s.push_str("        _ => 0.0,\n");
    s.push_str("    }\n");
    s.push_str("}\n\n");
    s.push_str("fn value_cstr(v: &Value) -> *const c_char {\n");
    s.push_str("    if let Value::String(s) = v {\n");
    s.push_str("        let c = match CString::new(s.clone()) {\n");
    s.push_str("            Ok(c) => c,\n");
    s.push_str("            Err(_) => CString::new(\"\").expect(\"the empty string is always NUL-terminated\"),\n");
    s.push_str("        };\n");
    s.push_str("        let p = c.as_ptr();\n");
    s.push_str("        CSTR_KEEP.with(|k| k.borrow_mut().push(c));\n");
    s.push_str("        p\n");
    s.push_str("    } else {\n");
    s.push_str("        std::ptr::null()\n");
    s.push_str("    }\n");
    s.push_str("}\n\n");
    s.push_str("fn cstr_arg(p: *const c_char) -> String {\n");
    s.push_str("    if p.is_null() {\n");
    s.push_str("        String::new()\n");
    s.push_str("    } else {\n");
    s.push_str("        // SAFETY: the caller passes a NUL-terminated C string.\n");
    s.push_str("        unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()\n");
    s.push_str("    }\n");
    s.push_str("}\n");
    for e in exports {
        s.push('\n');
        s.push_str(&gen_wrapper(e));
    }
    s
}

/// The shell crate's `Cargo.toml`; `runtime_path`/`core_path` are absolute so the temp crate can
/// depend on the real crates, whose own relative path deps still resolve against their manifests.
fn gen_cargo_toml(crate_name: &str, runtime_path: &Path, core_path: &Path) -> String {
    format!(
        "[package]\n\
         name = \"{crate_name}\"\n\
         version = \"0.1.0\"\n\
         edition = \"2024\"\n\
         description = \"C ABI export shell generated by `prima compile --emit-c-abi`\"\n\
         publish = false\n\
         \n\
         [lib]\n\
         crate-type = [\"cdylib\"]\n\
         \n\
         [dependencies]\n\
         prima-runtime = {{ path = {:?}, version = \"0.1.0\" }}\n\
         prima-core = {{ path = {:?}, version = \"0.1.0\" }}\n",
        runtime_path.to_string_lossy(),
        core_path.to_string_lossy()
    )
}

/// A crate name must be a Rust identifier: ASCII alnum/underscore, not starting with a digit.
fn sanitize_crate_name(s: &str) -> String {
    let mut out: String = s
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if out.is_empty() || out.as_bytes()[0].is_ascii_digit() {
        out.insert_str(0, "prima_");
    }
    out
}

/// Drop a leading `lib` from a file stem (`libhello` → `hello`) so the Rust crate mirrors the
/// conventional `lib<name>` cdylib artifact name.
fn strip_lib(stem: &str) -> String {
    stem.strip_prefix("lib").unwrap_or(stem).to_string()
}

/// `base` with its extension replaced by `ext` (appended when there is none), for the header.
fn with_extension(base: &Path, ext: &str) -> PathBuf {
    let mut p = base.to_path_buf();
    p.set_extension(ext);
    p
}

/// The shared library extension for the host platform.
fn shared_lib_ext() -> &'static str {
    if cfg!(target_os = "macos") {
        ".dylib"
    } else if cfg!(target_os = "windows") {
        ".dll"
    } else {
        ".so"
    }
}

/// Target path for the built library: `base` if it already carries the platform extension,
/// otherwise `base` + extension.
fn lib_target_path(base: &Path) -> PathBuf {
    let ext = shared_lib_ext();
    let name = base.to_string_lossy();
    if name.ends_with(ext) {
        base.to_path_buf()
    } else {
        PathBuf::from(format!("{name}{ext}"))
    }
}

/// A fresh per-run temp dir for the shell crate, under the OS temp dir.
fn make_temp_dir() -> std::io::Result<PathBuf> {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    let dir = std::env::temp_dir().join(format!("prima-cabi-{}-{nanos}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// Locate the built `cdylib` in `target_dir/release`. Rust emits `lib<name>.so`/`lib<name>.dylib`
/// on Unix and `<name>.dll` on Windows (MSVC), so match both prefixed and unprefixed stems.
fn find_artifact(target_dir: &Path, crate_name: &str) -> Option<PathBuf> {
    let ext = &shared_lib_ext()[1..];
    let entries = std::fs::read_dir(target_dir.join("release")).ok()?;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some((stem, e)) = name.split_once('.') else {
            continue;
        };
        if e != ext {
            continue;
        }
        if stem == crate_name || stem == format!("lib{crate_name}") {
            return Some(entry.path());
        }
    }
    None
}

/// Build the shell crate with `cargo build --release`; try `--offline` first (locked/vendored
/// deps), falling back to a normal build when the dependency graph needs updating.
fn build_shell_crate(temp: &Path, target_dir: &Path) -> bool {
    let manifest = temp.join("Cargo.toml");
    let mut first = Command::new("cargo");
    first
        .arg("build")
        .arg("--release")
        .arg("--offline")
        .arg("--manifest-path")
        .arg(&manifest)
        .env("CARGO_TARGET_DIR", target_dir);
    if first.status().is_ok_and(|s| s.success()) {
        return true;
    }
    let mut second = Command::new("cargo");
    second
        .arg("build")
        .arg("--release")
        .arg("--manifest-path")
        .arg(&manifest)
        .env("CARGO_TARGET_DIR", target_dir);
    second.status().is_ok_and(|s| s.success())
}

/// `prima compile --emit-c-abi`: build a shared library + C header for the file's `@c_api::extern`
/// exports (spec §18.4/§19.3). Both `--emit-c-abi` and `--emit-headers` are honored together:
/// this path always writes the header.
pub(crate) fn run(file: &Path, output: Option<&Path>) -> ExitCode {
    let source = match crate::read_src(file) {
        Ok(s) => s,
        Err(code) => return code,
    };
    let program = match prima_syntax::parse(&source) {
        Ok(p) => p,
        Err(errors) => {
            crate::diagnostics::report_syntax_errors(file, &source, &errors);
            return ExitCode::FAILURE;
        }
    };
    let exports = prima_runtime::capi::collect_exports(&program);
    if exports.is_empty() {
        crate::diagnostics::print_colored_error("no `@c_api::extern` exports found");
        return ExitCode::FAILURE;
    }

    let base = output
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("libprima_export"));
    let stem = base
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "prima_export".into());
    let crate_name = sanitize_crate_name(&strip_lib(&stem));

    // C header (spec §18.4).
    let header_path = with_extension(&base, "h");
    if let Err(e) = std::fs::write(&header_path, prima_runtime::capi::render_header(&exports)) {
        crate::diagnostics::print_colored_error(&format!(
            "cannot write {}: {e}",
            header_path.display()
        ));
        return ExitCode::FAILURE;
    }

    // Shell crate sources. The embedded source path is the canonicalized absolute path, so the
    // runtime can re-evaluate the module by path on first call.
    let src_path = std::fs::canonicalize(file).unwrap_or_else(|_| file.to_path_buf());
    let src_path = src_path.to_string_lossy().into_owned();
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let runtime_path = manifest_dir.join("crates/prima-runtime");
    let core_path = manifest_dir.join("crates/prima-core");

    let temp = match make_temp_dir() {
        Ok(t) => t,
        Err(e) => {
            crate::diagnostics::print_colored_error(&format!("cannot create temp dir: {e}"));
            return ExitCode::FAILURE;
        }
    };
    let write = |rel: &str, content: &str| -> bool {
        let p = temp.join(rel);
        if let Some(parent) = p.parent()
            && std::fs::create_dir_all(parent).is_err()
        {
            return false;
        }
        std::fs::write(p, content).is_ok()
    };
    if !write(
        "Cargo.toml",
        &gen_cargo_toml(&crate_name, &runtime_path, &core_path),
    ) || !write("src/lib.rs", &gen_lib_source(&exports, &src_path))
    {
        crate::diagnostics::print_colored_error(
            "cannot write the export shell crate to the temp dir",
        );
        let _ = std::fs::remove_dir_all(&temp);
        return ExitCode::FAILURE;
    }

    // Build (release, offline-first) inside the temp dir so the workspace `target/` is untouched.
    let target_dir = temp.join("target");
    if !build_shell_crate(&temp, &target_dir) {
        crate::diagnostics::print_colored_error(&format!(
            "failed to build the export shell crate for {} (see `cargo` output above)",
            file.display()
        ));
        let _ = std::fs::remove_dir_all(&temp);
        return ExitCode::FAILURE;
    }

    let artifact = match find_artifact(&target_dir, &crate_name) {
        Some(a) => a,
        None => {
            crate::diagnostics::print_colored_error(&format!(
                "no cdylib artifact for crate `{crate_name}` found in {}",
                target_dir.display()
            ));
            let _ = std::fs::remove_dir_all(&temp);
            return ExitCode::FAILURE;
        }
    };

    let lib_path = lib_target_path(&base);
    if let Err(e) = std::fs::copy(&artifact, &lib_path) {
        crate::diagnostics::print_colored_error(&format!(
            "cannot copy {} to {}: {e}",
            artifact.display(),
            lib_path.display()
        ));
        let _ = std::fs::remove_dir_all(&temp);
        return ExitCode::FAILURE;
    }
    let _ = std::fs::remove_dir_all(&temp);

    println!("wrote {}", lib_path.display());
    println!("wrote {}", header_path.display());
    ExitCode::SUCCESS
}
