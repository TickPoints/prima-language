//! Differential semantic gate across the three execution paths (spec §19.2/§19.5): the same program
//! must produce the same value, error code, and console output whether it runs through the AST
//! interpreter, the bytecode VM, or the default JIT → VM → AST dispatch. Path counters prove that
//! the intended engine actually ran; identical results after a silent fallback do not count as
//! coverage.
//!
//! The AST path is the *reference implementation*, not the authority: cases are chosen from the
//! specification (§6 exact numbers, §11 collections, §12 control flow, §13 policy, §16 errors), and a
//! disagreement is reported with the case name so the specification decides which side is wrong.

use std::cell::RefCell;
use std::rc::Rc;

use prima_core::{Number, Real, Value};
use prima_runtime::{Evaluator, PathStats};

/// Execution path selected by the surrounding `with config` block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    /// `vm := false`, `opt_level := O0`: authoritative AST interpreter only.
    Ast,
    /// `vm := true`, `opt_level := O1`: bytecode VM (JIT is gated at O2).
    Vm,
    /// No override: default `JIT → VM → AST` dispatch at `O2`.
    Default,
}

#[derive(Debug)]
struct Run {
    value: Option<Value>,
    /// Spec appendix C.2 runtime error code.
    error: Option<&'static str>,
    /// Full error rendering, kept for failure messages.
    error_message: Option<String>,
    /// Everything written through the evaluator sink.
    output: String,
    stats: PathStats,
}

fn mode_entries(mode: Mode) -> &'static str {
    match mode {
        Mode::Ast => "vm := false, opt_level := O0",
        Mode::Vm => "vm := true, opt_level := O1",
        Mode::Default => "",
    }
}

/// Run a program with the given policy entries active for the *whole* program, on one path.
///
/// The entries go into a leading `config {}` block (legal only at the top of the file), so they also
/// apply to calls made from top-level statements, including nested function bodies. `with config`
/// is not used here because a config block statement does not yield the value of its body to
/// [`Evaluator::eval_value`].
fn run(src: &str, policy: &str, mode: Mode) -> Run {
    // Builtin-type methods (`Array.push`, `String.len`, …) live in the embedded stdlib (spec §18.4).
    prima_stdlib::init();
    let entries = [mode_entries(mode), policy]
        .into_iter()
        .filter(|e| !e.is_empty())
        .collect::<Vec<_>>()
        .join(", ");
    let program = if entries.is_empty() {
        src.to_string()
    } else {
        format!("config {{ {entries} }}\n{src}")
    };

    let out = Rc::new(RefCell::new(String::new()));
    let sink = Rc::clone(&out);
    let mut ev = Evaluator::with_sink(move |s| sink.borrow_mut().push_str(&s));
    let result = ev.eval_value(&program);
    let (value, error, error_message) = match result {
        Ok(v) => (Some(v), None, None),
        Err(e) => (None, Some(e.code()), Some(e.to_string())),
    };
    Run {
        value,
        error,
        error_message,
        output: out.borrow().clone(),
        stats: ev.path_stats(),
    }
}

/// Value equality with IEEE NaN treated as equal (the observable "same result" for numeric paths;
/// `Value`'s derived `PartialEq` follows IEEE, under which a NaN never equals itself).
fn values_equal(a: Option<&Value>, b: Option<&Value>) -> bool {
    match (a, b) {
        (
            Some(Value::Number(Number::Real(Real::F64(x)))),
            Some(Value::Number(Number::Real(Real::F64(y)))),
        ) => x == y || (x.is_nan() && y.is_nan()),
        (
            Some(Value::Number(Number::Real(Real::F32(x)))),
            Some(Value::Number(Number::Real(Real::F32(y)))),
        ) => x == y || (x.is_nan() && y.is_nan()),
        _ => a == b,
    }
}

/// Assert that a program agrees on value/error/output across AST, VM, and the default path.
fn assert_parity(label: &str, src: &str, policy: &str) -> (Run, Run, Run) {
    let ast = run(src, policy, Mode::Ast);
    let vm = run(src, policy, Mode::Vm);
    let default = run(src, policy, Mode::Default);

    assert_eq!(
        vm.error, ast.error,
        "{label}: VM error code differs (ast={:?}, vm={:?})",
        ast.error_message, vm.error_message
    );
    assert!(
        values_equal(vm.value.as_ref(), ast.value.as_ref()),
        "{label}: VM result differs (ast={:?}, vm={:?})",
        ast.value,
        vm.value
    );
    assert_eq!(vm.output, ast.output, "{label}: VM console output differs");

    assert_eq!(
        default.error, ast.error,
        "{label}: default error code differs (ast={:?}, default={:?})",
        ast.error_message, default.error_message
    );
    assert!(
        values_equal(default.value.as_ref(), ast.value.as_ref()),
        "{label}: default result differs (ast={:?}, default={:?})",
        ast.value,
        default.value
    );
    assert_eq!(
        default.output, ast.output,
        "{label}: default console output differs"
    );
    (ast, vm, default)
}

fn int(n: i64) -> Value {
    Value::Number(Number::from(n))
}

fn assert_is_integer(v: Option<&Value>, expected: i64, label: &str) {
    assert_eq!(v, Some(&int(expected)), "{label}");
}

// ————— exact numbers (spec §6) —————

#[test]
fn parity_exact_integer_loop() {
    let src = "fn main(n: Integer) -> Integer {\n    let mut s = 0;\n    let mut i = 0;\n    while i < n { s = s * 3 + i; i += 1; }\n    s\n}\nmain(20)";
    let (_, vm, default) = assert_parity("exact integer loop", src, "");
    assert!(vm.stats.vm_hits > 0, "VM must execute: {:?}", vm.stats);
    assert!(
        default.stats.jit_hits > 0,
        "JIT must execute: {:?}",
        default.stats
    );
}

#[test]
fn parity_rational_division_stays_exact() {
    let src = "fn ratio(n: Integer) { 1 / n }\n(ratio(3), ratio(4))";
    let (ast, _, _) = assert_parity("rational division", src, "");
    let Some(Value::Tuple(items)) = ast.value.as_ref() else {
        panic!("expected a tuple, got {:?}", ast.value);
    };
    assert!(
        items
            .iter()
            .all(|v| matches!(v, Value::Number(Number::Rational(_)))),
        "expected exact rationals, got {items:?}"
    );
}

#[test]
fn parity_big_integer_overflow_deopts_but_stays_exact() {
    let src = "fn main(n: Integer) -> Integer {\n    let mut s = 0;\n    let mut i = 0;\n    while i < n { s += 9223372036854775807; i += 1; }\n    s\n}\nmain(5)";
    let (ast, _, default) = assert_parity("big integer overflow", src, "");
    assert!(
        matches!(ast.value, Some(Value::Number(Number::Integer(_)))),
        "expected a BigInt result, got {:?}",
        ast.value
    );
    assert!(
        default.stats.jit_deopts > 0,
        "the JIT must deopt on exact overflow: {:?}",
        default.stats
    );
}

#[test]
fn parity_fraction_policy_applies_on_every_path() {
    let src = "fn half(n: Integer) -> F64 { 1 / n }\nhalf(2)";
    let (ast, _, _) = assert_parity("fraction := false", src, "fraction := false");
    assert!(
        matches!(ast.value, Some(Value::Number(Number::Real(_)))),
        "expected a Real result, got {:?}",
        ast.value
    );
}

// ————— collections and aliasing (spec §11.3) —————

#[test]
fn parity_array_mutation_and_negative_index() {
    let src = "fn main(n: Integer) -> Integer {\n    let mut a = [];\n    for i in 0..n { a.push(i); }\n    a[-1] += a[0];\n    a[1..3] = [20, 30];\n    let mut s = 0;\n    for i in 0..n { s += a[i]; }\n    s + a[-1]\n}\nmain(5)";
    // Slice assignment and negative-index compound assignment both belong to the VM/JIT subset.
    let (_, vm, default) = assert_parity("array mutation", src, "");
    assert!(
        vm.stats.vm_hits > 0,
        "VM must execute the array kernel: {:?}",
        vm.stats
    );
    assert!(
        default.stats.jit_hits > 0,
        "JIT must execute the array kernel: {:?}",
        default.stats
    );
}

#[test]
fn parity_copy_on_write_aliasing() {
    let src = "fn main(n: Integer) -> Integer {\n    let mut a = [];\n    for i in 0..n { a.push(i); }\n    let b = a;\n    b[0] = 100;\n    let mut s = 0;\n    for i in 0..n { s += a[i]; }\n    s + b[0]\n}\nmain(50)";
    let (ast, vm, default) = assert_parity("copy-on-write aliasing", src, "");
    assert_is_integer(ast.value.as_ref(), 1225 + 100, "CoW sum");
    assert!(vm.stats.vm_hits > 0);
    assert!(default.stats.jit_hits > 0);
}

#[test]
fn parity_array_alias_push_stays_independent() {
    // The same value semantics apply to growth: after `let b = a`, pushing to `b` must not change
    // `a`'s length or contents (spec §11.3); the JIT copies the buffer when the alias is bound.
    let src = "fn main(n: Integer) -> Integer {\n    let mut a = [];\n    for i in 0..n { a.push(i); }\n    let b = a;\n    b.push(99);\n    a.len() + b.len() * 10\n}\nmain(4)";
    let (ast, _, default) = assert_parity("array alias push", src, "");
    assert_is_integer(ast.value.as_ref(), 4 + 5 * 10, "alias push lengths");
    assert!(default.stats.jit_hits > 0);
}

#[test]
fn parity_index_compound_assignment_jit() {
    // `a[i] += v` (and negative indices) must run natively in the whole-function JIT, not fall back.
    let src = "fn main(n: Integer) -> Integer {\n    let mut a = [0, 0, 0, 0];\n    let mut i = 0;\n    while i < n { a[i % 4] += i; i += 1; }\n    a[-1] += 100;\n    a[0] + a[1] + a[2] + a[3]\n}\nmain(10)";
    let (ast, vm, default) = assert_parity("index compound assignment", src, "");
    assert_is_integer(ast.value.as_ref(), 145, "compound index sum");
    assert!(
        vm.stats.vm_hits > 0,
        "VM must execute index compound assignment: {:?}",
        vm.stats
    );
    assert!(
        default.stats.jit_hits > 0,
        "JIT must execute index compound assignment: {:?}",
        default.stats
    );
}

#[test]
fn parity_slice_assignment_variants() {
    // Grow, insert-at-front, and truncate-with-open-end must all keep the JIT's value semantics and
    // be compiled on both fast paths (spec §11.3).
    let src = "fn main(n: Integer) -> Integer {\n    let mut a = [];\n    for i in 0..n { a.push(i); }\n    a[1..3] = [10, 20, 30];\n    a[0..0] = [7];\n    a[-1..] = [];\n    let mut s = 0;\n    for i in 0..a.len() { s += a[i]; }\n    s + a.len() * 1000\n}\nmain(5)";
    let (_, vm, default) = assert_parity("slice assignment variants", src, "");
    assert!(
        vm.stats.vm_hits > 0,
        "VM must execute slice assignment: {:?}",
        vm.stats
    );
    assert!(
        default.stats.jit_hits > 0,
        "JIT must execute slice assignment: {:?}",
        default.stats
    );
}

#[test]
fn parity_invalid_slice_range_errors_on_every_path() {
    let src = "fn main() -> Integer {\n    let mut a = [1, 2, 3];\n    a[2..1] = [];\n    a[0]\n}\nmain()";
    let (ast, _, _) = assert_parity("invalid slice range", src, "");
    assert_eq!(ast.error, Some("R0011"));
}

#[test]
fn parity_multidimensional_indexing() {
    // Matrix values are nested arrays; `A[i, j]`, row/column selections, and submatrices share one
    // selection implementation in the AST and the VM (spec §11.3).
    let src = "fn main() -> Integer {\n    let A = [[1, 2, 3], [4, 5, 6], [7, 8, 9]];\n    let e = A[0, 1];\n    let row = A[0, ..];\n    let col = A[.., 1];\n    let sub = A[0..2, 1..3];\n    e + row[0] + col[1] + sub[1][1]\n}\nmain()";
    let (ast, vm, default) = assert_parity("multidimensional indexing", src, "");
    assert_is_integer(ast.value.as_ref(), 2 + 1 + 5 + 6, "matrix selection");
    assert!(
        vm.stats.vm_hits > 0,
        "VM must execute multi-dimensional indexing: {:?}",
        vm.stats
    );
    assert!(
        default.stats.vm_hits > 0,
        "default path falls back to the VM for multi-dimensional indexing: {:?}",
        default.stats
    );
}

#[test]
fn parity_nested_fn_without_capture_runs_in_vm() {
    // A nested `fn` is bound through the environment; the VM delegates just that definition to the
    // AST and keeps the rest of the body compiled (spec §19.5).
    let src = "fn main(n: Integer) -> Integer {\n    fn helper(x: Integer) -> Integer { x * 2 }\n    let mut s = 0;\n    let mut i = 0;\n    while i < n { s += helper(i); i += 1; }\n    s\n}\nmain(5)";
    let (ast, vm, _) = assert_parity("nested fn", src, "");
    assert_is_integer(ast.value.as_ref(), 20, "nested helper sum");
    assert!(
        vm.stats.vm_hits > 0 && vm.stats.vm_fallbacks == 0,
        "non-capturing nested fn must keep the body in the VM: {:?}",
        vm.stats
    );
}

#[test]
fn nested_fn_binding_does_not_leak_out_of_the_call() {
    // A nested `fn` is call-local in the AST; the VM must not bind it in the module environment.
    let src =
        "fn main() -> Integer { fn helper() -> Integer { 1 } helper() }\nlet a = main();\nhelper()";
    let (ast, vm, default) = assert_parity("nested fn scope", src, "");
    assert!(ast.error.is_some(), "helper must not be visible after main");
    assert_eq!(ast.error, Some("R0011"));
    assert_eq!(vm.error, ast.error);
    assert_eq!(default.error, ast.error);
}

#[test]
fn parity_nested_fn_capture_takes_vm_fallback() {
    // Capturing an enclosing local is outside the compiled subset; the AST fallback must still agree.
    let src = "fn main(n: Integer) -> Integer {\n    fn add(x: Integer) -> Integer { x + n }\n    add(2)\n}\nmain(5)";
    let (ast, vm, _) = assert_parity("nested fn capture", src, "");
    assert_is_integer(ast.value.as_ref(), 7, "capturing nested fn");
    assert!(
        vm.stats.vm_fallbacks > 0 && vm.stats.ast_calls > 0,
        "capturing nested fn must take the documented fallback: {:?}",
        vm.stats
    );
}

#[test]
fn parity_out_of_bounds_error_matches() {
    let src = "fn main() -> Integer {\n    let a = [1, 2];\n    a[5]\n}\nmain()";
    let (ast, _, _) = assert_parity("out-of-bounds", src, "");
    assert_eq!(ast.error, Some("R0003"));
}

// ————— control flow (spec §10.2, §12) —————

#[test]
fn parity_control_flow_and_early_return() {
    let src = "fn main(n: Integer) -> Integer {\n    let mut total = 0;\n    for i in 0..n {\n        if i % 2 == 0 { total += i; } else { total += i * 2; }\n        if total > 100 { return total; }\n    }\n    total\n}\nmain(200)";
    assert_parity("control flow", src, "");
}

#[test]
fn parity_tail_recursion() {
    let src = "fn sum(n: Integer, acc: Integer) -> Integer {\n    if n == 0 { return acc; }\n    return sum(n - 1, acc + n);\n}\nsum(10, 0)";
    assert_parity("tail recursion", src, "");
}

#[test]
fn parity_loop_optimization_closed_form() {
    let src = "fn main(n: Integer) -> Integer {\n    let mut s = 0;\n    for i in 0..n { s += i; }\n    s\n}\nmain(1000)";
    let (ast, _, _) = assert_parity("loop closed form", src, "");
    assert_is_integer(ast.value.as_ref(), 499_500, "closed-form sum");
}

// ————— configuration policy (spec §13) —————

#[test]
fn parity_broadcast_policy_error() {
    // `broadcast := false` disables both pure-function broadcasting and `Array ∘ scalar` (spec §11.4).
    let call =
        "let f(x) = x + 1;\nfn main(v: Array<Integer>) -> Integer { f(v).len() }\nmain([1, 2, 3])";
    let (ast, _, _) = assert_parity("broadcast := false (call)", call, "broadcast := false");
    assert_eq!(ast.error, Some("R0011"));

    let scalar = "fn main(a: Array<Integer>) -> Integer { (a + 1).len() }\nmain([1, 2, 3])";
    let (ast, _, _) = assert_parity("broadcast := false (scalar)", scalar, "broadcast := false");
    assert_eq!(ast.error, Some("R0011"));
}

#[test]
fn parity_undefined_handling_custom() {
    let src = "fn main(n: Integer) -> Integer { n / n }\nmain(0)";
    let (ast, _, _) = assert_parity(
        "undefined_handling custom",
        src,
        "undefined_handling := custom { 0/0 := 7 }",
    );
    assert_is_integer(ast.value.as_ref(), 7, "custom 0/0");
}

#[test]
fn parity_domain_real_rejects_negative_fractional_power() {
    // `(-1)^0.5` is the policy-checked power path (spec §6.5/§9.9); the JIT cannot lower `Pow`, so
    // this exercises the VM/AST policy split.
    let src = "fn main(n: Integer) -> F64 { to_f64(n) ^ 0.5 }\nmain(-1)";
    let (ast, _, _) = assert_parity("domain := real", src, "domain := real");
    assert_eq!(ast.error, Some("R0005"));

    // Constant folding must honor the policy too: `sqrt(-1.0)` may not silently become NaN.
    let sqrt = "fn main() -> F64 { sqrt(-1.0) }\nmain()";
    let (ast, _, _) = assert_parity("domain := real (sqrt)", sqrt, "domain := real");
    assert_eq!(ast.error, Some("R0005"));

    let ln = "fn main() -> F64 { ln(-1.0) }\nmain()";
    let (ast, _, _) = assert_parity("domain := real (ln)", ln, "domain := real");
    assert_eq!(ast.error, Some("R0005"));
}

// ————— builtin types (spec §18.1) —————

#[test]
fn parity_dict_and_set_operations() {
    let src = "fn main(n: Integer) -> Integer {\n    let mut d = {};\n    for i in 0..n { d[i] = i * i; }\n    let s = {1, 2, 2, 3};\n    let mut total = 0;\n    for i in 0..n { total += d[i]; }\n    total + s.len()\n}\nmain(10)";
    assert_parity("dict and set operations", src, "");
}

#[test]
fn parity_dict_and_set_mutations_run_in_vm() {
    let src = "fn main(n: Integer) -> Integer {\n    let mut d = {};\n    let mut i = 0;\n    while i < n { d.insert(i, i * 2); i += 1; }\n    let mut s = 0;\n    let mut j = 0;\n    while j < n { s += d[j]; j += 1; }\n    d.remove(0);\n    let mut set = {1, 2};\n    set.add(3);\n    set.discard(2);\n    set.update({4, 5});\n    s + d.len() + set.len()\n}\nmain(5)";
    let (ast, vm, default) = assert_parity("dict/set mutation", src, "");
    assert_is_integer(ast.value.as_ref(), 20 + 4 + 4, "dict/set mutation result");
    assert!(
        vm.stats.vm_hits > 0,
        "mutating dict/set methods must run in the VM: {:?}",
        vm.stats
    );
    assert!(
        default.stats.vm_hits > 0,
        "default path must run the dict/set body in the VM: {:?}",
        default.stats
    );
}

#[test]
fn parity_string_methods() {
    let src = "fn main(s: String) -> String { \"-\".join(s.split(\",\")) }\nmain(\"a,b,c\")";
    let (ast, _, _) = assert_parity("string methods", src, "");
    assert_eq!(ast.value, Some(Value::String("a-b-c".into())));
}

#[test]
fn parity_class_method_call() {
    let src = "class Counter {\n    value: Integer,\n    pub fn new(v: Integer) -> Self { Counter { value: v } },\n    pub fn get(self) -> Integer { self.value }\n}\nfn main(n: Integer) -> Integer { Counter::new(n).get() }\nmain(7)";
    let (ast, vm, _) = assert_parity("class method call", src, "");
    assert_is_integer(ast.value.as_ref(), 7, "class method result");
    // The method body (`self.value`) now runs in the VM; class construction/struct literals still
    // take the documented fallback.
    assert!(
        vm.stats.vm_hits > 0,
        "class method body must run in the VM: {:?}",
        vm.stats
    );
    assert!(
        vm.stats.vm_fallbacks > 0,
        "class construction must take the documented VM fallback: {:?}",
        vm.stats
    );
}

#[test]
fn parity_class_method_field_arithmetic_in_vm() {
    let src = "class Point {\n    x: Integer,\n    y: Integer,\n    pub fn new(x: Integer, y: Integer) -> Self { Point { x, y } },\n    pub fn sum(self) -> Integer { self.x + self.y }\n}\nfn main(n: Integer) -> Integer {\n    let p = Point::new(n, 2);\n    let mut s = 0;\n    let mut i = 0;\n    while i < n { s += p.sum(); i += 1; }\n    s\n}\nmain(4)";
    let (ast, vm, default) = assert_parity("class field arithmetic", src, "");
    assert_is_integer(ast.value.as_ref(), 4 * 6, "field arithmetic result");
    assert!(
        vm.stats.vm_hits > 0,
        "class method with field reads must run in the VM: {:?}",
        vm.stats
    );
    let _ = default;
}

// ————— error propagation (spec §16.3) —————

#[test]
fn parity_try_on_err_returns_from_function() {
    let src = "fn inner() -> Result<Integer, Error> { Err(\"bad\") }\nfn main() -> Result<Integer, Error> { let v = inner()?; Ok(v + 1) }\nmain()";
    assert_parity("? on Err", src, "");
}

#[test]
fn parity_try_on_none_returns_none() {
    let src = "fn main() -> Option<Integer> { let x = get([1], 5)?; Some(x) }\nmain()";
    assert_parity("? on None", src, "");
}

#[test]
fn parity_nested_error_code_propagates() {
    let src =
        "fn inner() -> Integer { let a = [1]; a[9] }\nfn main() -> Integer { inner() + 1 }\nmain()";
    let (ast, _, _) = assert_parity("nested error", src, "");
    assert_eq!(ast.error, Some("R0003"));
}

// ————— output and side effects (spec §18.1) —————

#[test]
fn parity_print_side_effects_happen_exactly_once() {
    let src = "fn main(n: Integer) -> Integer {\n    let mut s = 0;\n    for i in 0..n { print(i); s += i; }\n    s\n}\nmain(4)";
    let (ast, vm, default) = assert_parity("print side effects", src, "");
    // Four values, each written exactly once (a fallback re-run would duplicate them).
    assert_eq!(
        ast.output, "0123",
        "expected each loop value printed once, got {:?}",
        ast.output
    );
    assert!(
        vm.stats.vm_hits > 0,
        "VM must execute the print loop: {:?}",
        vm.stats
    );
    assert!(
        default.stats.jit_ineligible > 0,
        "impure body must not be JIT-compiled: {:?}",
        default.stats
    );
}

#[test]
fn parity_print_before_error_is_not_duplicated() {
    let src = "fn main() -> Integer {\n    print(1);\n    let a = [0];\n    a[3]\n}\nmain()";
    let (ast, _, _) = assert_parity("print before error", src, "");
    assert_eq!(
        ast.output.matches('1').count(),
        1,
        "output={:?}",
        ast.output
    );
    assert_eq!(ast.error, Some("R0003"));
}

#[test]
fn parity_global_mutation_from_host_fn() {
    let src = "let mut g = 0;\nfn bump() -> Integer { g += 1; g }\nfn main(n: Integer) -> Integer { let mut s = 0; for i in 0..n { s += bump(); } s }\nlet s = main(5);\n(s, g)";
    let (ast, _, _) = assert_parity("global mutation", src, "");
    assert_eq!(
        ast.value,
        Some(Value::Tuple(vec![int(15), int(5)].into_boxed_slice()))
    );
}

// ————— path coverage: intentional fallbacks (spec §19.5) —————

#[test]
fn vm_runs_kernels_and_falls_back_for_unsupported_bodies() {
    let kernel = "fn main(n: Integer) -> Integer {\n    let mut s = 0;\n    let mut i = 0;\n    while i < n { s += i * i; i += 1; }\n    s\n}\nmain(100)";
    let vm = run(kernel, "", Mode::Vm);
    assert!(
        vm.stats.vm_hits > 0,
        "pure kernel must run in the VM: {:?}",
        vm.stats
    );
    assert_eq!(vm.stats.jit_hits, 0, "JIT is gated at O2");
    assert_eq!(vm.stats.ast_calls, 0, "the VM must not silently fall back");

    // A nested `fn` that captures an enclosing local is outside the compiled subset (spec §19.5), so
    // the VM must decline it and the AST interpreter must still produce the value.
    let unsupported = "fn main(n: Integer) -> Integer {\n    fn inner() -> Integer { n }\n    inner()\n}\nmain(3)";
    let fallback = run(unsupported, "", Mode::Vm);
    assert!(
        fallback.stats.vm_fallbacks > 0 && fallback.stats.ast_calls > 0,
        "capturing nested fn must take the documented AST fallback: {:?}",
        fallback.stats
    );
}

#[test]
fn default_path_prefers_jit_then_falls_back() {
    let numeric = "fn main(n: Integer) -> Integer {\n    let mut s = 0;\n    let mut i = 0;\n    while i < n { s += i * i; i += 1; }\n    s\n}\nmain(100)";
    let jit = run(numeric, "", Mode::Default);
    assert!(
        jit.stats.jit_hits > 0,
        "numeric kernel must run natively: {:?}",
        jit.stats
    );

    // `print` is outside the pure JIT subset, so the default dispatch declines it and the VM runs.
    let vm_path = "fn main(n: Integer) -> Integer {\n    print(n);\n    n + 1\n}\nmain(3)";
    let vm = run(vm_path, "", Mode::Default);
    assert!(
        vm.stats.jit_ineligible > 0 && vm.stats.vm_hits > 0,
        "default path must fall back to the VM: {:?}",
        vm.stats
    );
}
