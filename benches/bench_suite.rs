//! Cross-language performance benchmark suite (bench/RESULTS.md).
//!
//! Measures the steady-state execution time of a fixed set of deterministic kernels across three
//! implementations of the *same* semantics:
//!
//! - **Prima**: loaded in-process through `Evaluator::eval_file_keep_env` and invoked via
//!   `call_function` on a warm environment (no per-run parse/startup cost; the symbol + numeric
//!   layers stay resident). This is the number the language reports.
//! - **Rust**: a native reference closure with the identical loop structure (upper bound).
//! - **Python**: the CPython reference (`benches/bench_ref.py`), run as a subprocess that times its
//!   own kernel via `perf_counter` so interpreter startup is excluded (best-effort parity with the
//!   in-process Prima/Rust measurement).
//!
//! Run with `cargo bench --bench bench_suite` (harness = false) or `cargo run --release --bench bench_suite`.
//! Results are written to `benches/RESULTS.md` (`--update` to regenerate; default prints the table).

use std::hint::black_box;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use prima_language::{
    prima_core::{Number, Value},
    prima_runtime::Evaluator,
};

/// Per-workload configuration: a Prima `.pra` kernel file, a Python reference name, the parameter,
/// and a Rust reference closure returning `f64` (kernels are scalar-valued).
struct Workload {
    name: &'static str,
    pra: &'static str,
    py: &'static str,
    n: i64,
    rust: fn(i64) -> f64,
}

/// Rudimentary median-of-runs timing: run `warmup` untimed iterations, then time `samples` runs and
/// return the median wall-clock per run.
fn time_median(mut func: impl FnMut(), samples: u32, warmup: u32) -> Duration {
    for _ in 0..warmup {
        func();
        black_box(());
    }
    let mut times = Vec::with_capacity(samples as usize);
    for _ in 0..samples {
        let t0 = Instant::now();
        func();
        times.push(t0.elapsed());
        black_box(());
    }
    times.sort();
    times[times.len() / 2]
}
/// Time one Prima workload at a fixed parameter `n`. When `vm` is true, the kernel is run through
/// the bytecode VM (spec §19.5) via `Evaluator::vm_call_function`; otherwise the AST interpreter runs
/// it via `call_function`. Both run in-process and warm, so parsing/module loading are excluded.
/// Returns the median wall time and the kernel's result value for cross-language verification.
fn time_prima(pra: &str, n: i64, samples: u32, warmup: u32, vm: bool) -> (Duration, f64) {
    let mut ev = Evaluator::with_sink(|_| {});
    let env = ev
        .eval_file_keep_env(Path::new(pra))
        .expect("kernel file must parse");
    let arg = Value::Number(Number::from(n));
    let mut result = 0.0f64;
    let d = time_median(
        || {
            let v = if vm {
                ev.vm_call_function(&env, "main", vec![arg.clone()])
            } else {
                ev.call_function(&env, "main", vec![arg.clone()])
            }
            .expect("kernel must run");
            result = value_to_f64(&v);
        },
        samples,
        warmup,
    );
    (d, result)
}

/// Extract a scalar `f64` from a kernel's return `Value` for cross-language comparison.
fn value_to_f64(v: &Value) -> f64 {
    match v {
        Value::Number(n) => n.to_f64_lossy(),
        other => panic!("kernel returned a non-scalar value: {other:?}"),
    }
}

/// Time the native Rust reference at a fixed parameter `n`.
fn time_rust(f: fn(i64) -> f64, n: i64, samples: u32, warmup: u32) -> Duration {
    time_median(
        || {
            let _ = black_box(f(black_box(n)));
        },
        samples,
        warmup,
    )
}

/// Time the Python reference. We run `python3 bench_ref.py <name> <n> --time`, which times its own
/// kernel inside the process (excluding interpreter startup), and parse the `seconds=` value. It also
/// prints `result=`, which we check against Prima/Rust. The script does one timed run per
/// invocation; we run it a few times and take the median.
fn time_python(py_file: &Path, name: &str, n: i64, samples: u32) -> (Duration, f64) {
    let mut runs = Vec::with_capacity(samples as usize);
    let mut result = 0.0f64;
    for _ in 0..samples {
        let out = Command::new("python3")
            .arg(py_file)
            .arg(name)
            .arg(n.to_string())
            .arg("--time")
            .output()
            .expect("python3 must be available");
        let stdout = String::from_utf8_lossy(&out.stdout);
        let secs = stdout
            .split("seconds=")
            .nth(1)
            .and_then(|s| s.split_whitespace().next())
            .and_then(|s| s.parse::<f64>().ok())
            .unwrap_or_else(|| panic!("cannot parse python timing for {name}: {stdout}"));
        let r = stdout
            .split("result=")
            .nth(1)
            .and_then(|s| s.split_whitespace().next())
            .and_then(|s| s.parse::<f64>().ok())
            .unwrap_or_else(|| panic!("cannot parse python result for {name}: {stdout}"));
        result = r;
        runs.push(Duration::from_secs_f64(secs));
    }
    runs.sort();
    (runs[runs.len() / 2], result)
}

// --- Native Rust reference kernels (same semantics as benches/workloads/*.pra). ---
//
// Each arithmetic kernel inserts a `black_box` on the accumulator inside its loop so the compiler
// cannot collapse the loop into a closed form (LLVM otherwise reduces e.g. `sum i*i` to an O(1) sum
// formula); this keeps every reference an honest O(n) execution that benchmarks steady-state Rust.

fn ref_sumsq(n: i64) -> f64 {
    let mut s: i64 = 0;
    let mut i: i64 = 0;
    while i < n {
        s = s.wrapping_add(i.wrapping_mul(i));
        i += 1;
        black_box(s);
    }
    s as f64
}

fn ref_pi(n: i64) -> f64 {
    let mut acc = 0.0;
    let mut sign = 1.0;
    let mut i: i64 = 0;
    while i < n {
        let t = (2 * i + 1) as f64;
        acc += sign / t;
        sign = -sign;
        i += 1;
        black_box(acc);
    }
    4.0 * acc
}

fn ref_fib(n: i64) -> f64 {
    if n <= 1 {
        return n as f64;
    }
    let mut a: i64 = 0;
    let mut b: i64 = 1;
    let mut i: i64 = 2;
    while i <= n {
        let s = a + b;
        a = b;
        b = s;
        i += 1;
        black_box(b);
    }
    b as f64
}

fn ref_sieve(n: i64) -> f64 {
    let mut prime = vec![true; (n + 1) as usize];
    prime[0] = false;
    prime[1] = false;
    let mut i: i64 = 2;
    while i * i <= n {
        if prime[i as usize] {
            let mut j = i * i;
            while j <= n {
                prime[j as usize] = false;
                j += i;
            }
        }
        i += 1;
        black_box(i);
    }
    let mut count = 0usize;
    for &b in &prime {
        count += b as usize;
        black_box(count);
    }
    count as f64
}

fn ref_dot(n: i64) -> f64 {
    let x: Vec<f64> = (0..n).map(|i| (i % 13) as f64).collect();
    let y: Vec<f64> = (0..n).map(|i| ((i * 3 + 1) % 17) as f64).collect();
    let mut s = 0.0;
    for (a, b) in x.iter().zip(&y) {
        s += a * b;
        black_box(s);
    }
    s
}

fn ref_poly(n: i64) -> f64 {
    let mut acc = 0.0;
    let mut i: i64 = 0;
    while i < n {
        let t = i as f64 / 7.0;
        let r = (((1.0 * t + 2.0) * t + 3.0) * t + 4.0) * t + 5.0;
        acc += r;
        i += 1;
        black_box(acc);
    }
    acc
}

const SAMPLES: u32 = 9;
const WARMUP: u32 = 3;

fn workloads() -> Vec<Workload> {
    vec![
        Workload {
            name: "sumsq",
            pra: "benches/workloads/sumsq.pra",
            py: "sumsq",
            n: 200_000,
            rust: ref_sumsq,
        },
        Workload {
            name: "pi",
            pra: "benches/workloads/pi.pra",
            py: "pi",
            n: 100_000,
            rust: ref_pi,
        },
        Workload {
            name: "fib",
            pra: "benches/workloads/fib.pra",
            py: "fib",
            n: 30,
            rust: ref_fib,
        },
        Workload {
            name: "sieve",
            pra: "benches/workloads/sieve.pra",
            py: "sieve",
            n: 5_000,
            rust: ref_sieve,
        },
        Workload {
            name: "dot",
            pra: "benches/workloads/dot.pra",
            py: "dot",
            n: 3_000,
            rust: ref_dot,
        },
        Workload {
            name: "poly",
            pra: "benches/workloads/poly.pra",
            py: "poly",
            n: 50_000,
            rust: ref_poly,
        },
    ]
}

fn fmt_ns(d: Duration) -> String {
    format!("{:.0} ns", d.as_nanos())
}

fn main() {
    prima_language::prima_stdlib::init();
    let py_file = PathBuf::from("benches/bench_ref.py");
    let wl = workloads();
    let mut table = String::new();
    table.push_str("# Prima vs Python vs Rust — benchmark results\n\n");
    table.push_str("Deterministic, scalar-valued kernels measured in steady state: Prima and Rust run in-process\n");
    table.push_str("(warm interpreter/native, so parsing and module loading are excluded); Python times its own\n");
    table.push_str(
        "kernel with `perf_counter` so interpreter startup is excluded too. Times are medians of\n",
    );
    table.push_str("repeated runs; the `Python ×` and `Rust ×` columns are the multiplier by which the reference\n");
    table.push_str(
        "implementation is *faster* than Prima (1.0× = equal, higher = reference wins).\n\n",
    );
    table.push_str("NOTE: this is the AST-interpreter baseline. `vm := true` (bytecode VM, spec §19.5) and the\n");
    table.push_str("JIT hot path (spec §19.2) are the mechanisms targeted at closing the gap to Python/Rust;\n");
    table.push_str(
        "see the milestone notes and docs/IMPLEMENTATION-zh_CN.md §5 for the tracked deltas.\n\n",
    );
    table.push_str(
        "Regenerate with `cargo bench --bench bench_suite` (see benches/bench_suite.rs).\n\n",
    );
    table.push_str(
        "The `Prima VM` column runs the same kernel through the bytecode VM (spec §19.5);\n",
    );
    table.push_str(
        "`VM/AST ×` is how much faster the VM is than the AST interpreter on that kernel.\n\n",
    );
    table.push_str("| workload | n | Prima AST (ns) | Prima VM (ns) | Python (ns) | Rust (ns) | VM/AST × | Python × | Rust × |\n");
    table.push_str("|---|---|---|---|---|---|---|---|---|\n");

    println!(
        "running {} workloads ({SAMPLES} samples, {WARMUP} warmup)...",
        wl.len()
    );

    for w in &wl {
        let (p, pv) = time_prima(w.pra, w.n, SAMPLES, WARMUP, false);
        let (pv_t, pv_v) = time_prima(w.pra, w.n, SAMPLES, WARMUP, true);
        let (py, pyv) = time_python(&py_file, w.py, w.n, SAMPLES);
        let rv = (w.rust)(w.n);
        let r = time_rust(w.rust, w.n, SAMPLES, WARMUP);

        // Cross-language correctness: the three implementations of the same kernel must agree.
        let name = w.name;
        let prim_vs_py = (pv - pyv).abs() / pv.abs().max(1e-12);
        let prim_vs_rust = (pv - rv).abs() / pv.abs().max(1e-12);
        assert!(
            prim_vs_py < 1e-6,
            "{}: prima={pv} python={pyv} disagree",
            name
        );
        assert!(
            prim_vs_rust < 1e-6,
            "{}: prima={pv} rust={rv} disagree",
            name
        );
        assert!(
            (pv - pv_v).abs() / pv.abs().max(1e-12) < 1e-6,
            "{}: ast={pv} vm={pv_v} disagree",
            name
        );

        let vm_ast = p.as_secs_f64() / pv_t.as_secs_f64();
        let py_mult = p.as_secs_f64() / py.as_secs_f64();
        let rust_mult = p.as_secs_f64() / r.as_secs_f64();
        println!(
            "{:<8} ast={:<14} vm={:<14} python={:<14} rust={:<14} vm/ast={:.1}×",
            w.name,
            fmt_ns(p),
            fmt_ns(pv_t),
            fmt_ns(py),
            fmt_ns(r),
            vm_ast
        );
        table.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} | {:.1}× | {:.1}× | {:.2}× |\n",
            w.name,
            w.n,
            fmt_ns(p),
            fmt_ns(pv_t),
            fmt_ns(py),
            fmt_ns(r),
            vm_ast,
            py_mult,
            rust_mult
        ));
    }

    std::fs::write("benches/RESULTS.md", &table).expect("cannot write benches/RESULTS.md");
    println!("wrote benches/RESULTS.md");
}
