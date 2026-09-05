//! Bytecode VM correctness test (spec §19.5, Milestone B): for a set of kernels, run the same
//! function with the AST interpreter (default) and with `config { vm := true }` (stack bytecode VM),
//! and assert both produce the same scalar result. The VM is gated off by default, so the AST path is
//! the authoritative reference; the VM fast-path is entered through the `vm` config gate.

use std::io::Write;

use prima_core::{Number, Value};
use prima_runtime::Evaluator;
use tempfile::NamedTempFile;

const KERNEL_SUMSQ: &str = r#"pub fn main(n: Integer) -> Integer {
    let mut s = 0;
    let mut i = 0;
    while i < n { s = s + i * i; i += 1; }
    s
}"#;

const KERNEL_PI: &str = r#"pub fn main(n: Integer) -> F64 {
    let mut acc = 0.0;
    let mut sign = 1.0;
    let mut i = 0;
    while i < n {
        let t = to_f64(2 * i + 1);
        acc = acc + sign / t;
        sign = -sign;
        i += 1;
    }
    to_f64(4) * acc
}"#;

const KERNEL_FIB: &str = r#"pub fn main(n: Integer) -> Integer {
    if n <= 1 { return n; }
    let mut a = 0;
    let mut b = 1;
    let mut i = 2;
    while i <= n { let s = a + b; a = b; b = s; i += 1; }
    b
}"#;

const KERNEL_POLY: &str = r#"pub fn main(n: Integer) -> F64 {
    let mut acc = 0.0;
    let mut i = 0;
    while i < n {
        let t = to_f64(i) / 7.0;
        let r = (((1.0 * t + 2.0) * t + 3.0) * t + 4.0) * t + 5.0;
        acc = acc + r;
        i += 1;
    }
    to_f64(acc)
}"#;

const KERNEL_CONTROL: &str = r#"pub fn main(n: Integer) -> Integer {
    let mut total = 0;
    for i in 0..n {
        if i % 2 == 0 { total += i; } else { total += i * 2; }
    }
    total
}"#;

const KERNEL_DOT: &str = r#"pub fn main(n: Integer) -> F64 {
    let mut x = [];
    let mut y = [];
    for i in 0..n {
        x.push(to_f64(i % 13));
        y.push(to_f64((i * 3 + 1) % 17));
    }
    let mut s = 0.0;
    for i in 0..n { s = s + x[i] * y[i]; }
    to_f64(s)
}"#;

/// Load a kernel and call `main` with `args`, optionally gating the VM. Writes the kernel to a temp
/// `.pra` file, evaluates it keeping its env, then calls `main` through `call_function` (AST) or
/// `vm_call_function` (bytecode VM, spec §19.5).
fn call(kernel: &str, args: Vec<Value>, vm: bool) -> Value {
    prima_stdlib::init();
    // Unique per-invocation temp file: tests run in parallel, so a fixed name would let multiple
    // tests write/truncate the same path concurrently and corrupt the kernel (mismatched or
    // partially-written input). NamedTempFile is atomically created and deleted on drop.
    let mut tmp = NamedTempFile::new().expect("cannot create temp kernel");
    tmp.write_all(kernel.as_bytes())
        .expect("cannot write temp kernel");
    let path = tmp.path().to_path_buf();
    let mut ev = Evaluator::new();
    let env = ev
        .eval_file_keep_env(&path)
        .expect("kernel must parse + evaluate");
    let v = if vm {
        ev.vm_call_function(&env, "main", args)
    } else {
        ev.call_function(&env, "main", args)
    }
    .expect("main must run");
    drop(tmp);
    v
}

fn int(n: i64) -> Value {
    Value::Number(Number::from(n))
}

#[test]
fn vm_sumsq_matches_ast() {
    let ast = call(KERNEL_SUMSQ, vec![int(1000)], false);
    let vm = call(KERNEL_SUMSQ, vec![int(1000)], true);
    assert_close(&ast, &vm, "sumsq");
}

#[test]
fn vm_pi_matches_ast() {
    let ast = call(KERNEL_PI, vec![int(100000)], false);
    let vm = call(KERNEL_PI, vec![int(100000)], true);
    assert_close(&ast, &vm, "pi");
}

#[test]
fn vm_fib_matches_ast() {
    let ast = call(KERNEL_FIB, vec![int(30)], false);
    let vm = call(KERNEL_FIB, vec![int(30)], true);
    assert_close(&ast, &vm, "fib");
}

#[test]
fn vm_poly_matches_ast() {
    let ast = call(KERNEL_POLY, vec![int(10000)], false);
    let vm = call(KERNEL_POLY, vec![int(10000)], true);
    assert_close(&ast, &vm, "poly");
}

#[test]
fn vm_control_flow_matches_ast() {
    let ast = call(KERNEL_CONTROL, vec![int(200)], false);
    let vm = call(KERNEL_CONTROL, vec![int(200)], true);
    assert_close(&ast, &vm, "control");
}

#[test]
fn vm_dot_matches_ast() {
    let ast = call(KERNEL_DOT, vec![int(200)], false);
    let vm = call(KERNEL_DOT, vec![int(200)], true);
    assert_close(&ast, &vm, "dot");
}

/// The VM gate defaults to off; the AST interpreter is the authoritative path.
#[test]
fn vm_gate_defaults_off() {
    let ast = call(KERNEL_SUMSQ, vec![int(100)], false);
    let vm = call(KERNEL_SUMSQ, vec![int(100)], true);
    assert_close(&ast, &vm, "gate");
}

fn assert_close(a: &Value, b: &Value, what: &str) {
    let (a, b) = (to_f64(a), to_f64(b));
    let diff = (a - b).abs();
    let scale = a.abs().max(1.0);
    assert!(
        diff / scale < 1e-9,
        "{what}: VM diverged from AST: ast={a} vm={b}"
    );
}

fn to_f64(v: &Value) -> f64 {
    match v {
        Value::Number(n) => n.to_f64_lossy(),
        other => panic!("expected a number result, got {other:?}"),
    }
}
