//! Cross-language and internal performance baseline (benches/RESULTS.md).
//!
//! Measures the steady-state execution time of a fixed set of deterministic kernels:
//!
//! - **Prima**: loaded in-process through `Evaluator::eval_file_keep_env` and invoked on a warm
//!   environment (no per-run parse/startup cost). Each kernel is measured on the authoritative AST
//!   path (`ast_call_function`), the bytecode VM (`vm_call_function`), and the default dispatch
//!   chain JIT → VM → AST (`call_function`).
//! - **Rust**: a native reference closure with the identical loop structure (upper bound), for the
//!   numeric kernels where an equivalent scalar reference is meaningful.
//! - **Python**: the CPython reference (`benches/bench_ref.py`), run as a subprocess that times its
//!   own kernel via `perf_counter` so interpreter startup is excluded.
//!
//! Symbolic and collection workloads have no cross-language reference: they establish an internal
//! baseline for the parts of the language that have no natural Python/Rust one-liner. They still
//! assert AST/VM/default correctness before timing is reported.
//!
//! Run with `cargo bench --bench bench_suite` (harness = false) or
//! `cargo run --release --bench bench_suite`. Results are written to `benches/RESULTS.md`.
//!
//! NOTE: results are only comparable on the same machine, toolchain, and build profile; the file
//! records all three. Existing numbers are historical references, not cross-machine thresholds.

use std::hint::black_box;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use prima_language::{
    prima_core::{Number, Value},
    prima_runtime::Evaluator,
};

/// Which Prima execution path a timed run uses.
#[derive(Clone, Copy, PartialEq, Eq)]
enum PrimaPath {
    /// Authoritative AST interpreter (`ast_call_function`).
    Ast,
    /// Bytecode VM (`vm_call_function`; falls back to the default dispatch when the body is outside
    /// the compiled subset).
    Vm,
    /// Default dispatch: whole-function JIT → bytecode VM → AST (`call_function`).
    Default,
}

/// Per-workload configuration. `py`/`rust` are `None` for symbolic and collection workloads, which
/// have no cross-language reference with identical semantics.
struct Workload {
    name: &'static str,
    pra: &'static str,
    py: Option<&'static str>,
    n: i64,
    rust: Option<fn(i64) -> f64>,
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

/// Time one Prima workload at a fixed parameter `n` on `path`. The environment is warm, so parsing
/// and module loading are excluded. Returns the median wall time and the kernel's result value for
/// cross-path correctness checks.
fn time_prima(pra: &str, n: i64, samples: u32, warmup: u32, path: PrimaPath) -> (Duration, f64) {
    let mut ev = Evaluator::with_sink(|_| {});
    let env = ev
        .eval_file_keep_env(Path::new(pra))
        .expect("kernel file must parse");
    let arg = Value::Number(Number::from(n));
    let mut result = 0.0f64;
    let d = time_median(
        || {
            let v = match path {
                PrimaPath::Ast => ev.ast_call_function(&env, "main", vec![arg.clone()]),
                PrimaPath::Vm => ev.vm_call_function(&env, "main", vec![arg.clone()]),
                PrimaPath::Default => ev.call_function(&env, "main", vec![arg.clone()]),
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
            py: Some("sumsq"),
            n: 200_000,
            rust: Some(ref_sumsq),
        },
        Workload {
            name: "pi",
            pra: "benches/workloads/pi.pra",
            py: Some("pi"),
            n: 100_000,
            rust: Some(ref_pi),
        },
        Workload {
            name: "fib",
            pra: "benches/workloads/fib.pra",
            py: Some("fib"),
            n: 30,
            rust: Some(ref_fib),
        },
        Workload {
            name: "sieve",
            pra: "benches/workloads/sieve.pra",
            py: Some("sieve"),
            n: 5_000,
            rust: Some(ref_sieve),
        },
        Workload {
            name: "dot",
            pra: "benches/workloads/dot.pra",
            py: Some("dot"),
            n: 3_000,
            rust: Some(ref_dot),
        },
        Workload {
            name: "poly",
            pra: "benches/workloads/poly.pra",
            py: Some("poly"),
            n: 50_000,
            rust: Some(ref_poly),
        },
        // Internal baseline: symbolic work has no natural Python/Rust scalar reference.
        Workload {
            name: "simplify",
            pra: "benches/workloads/simplify.pra",
            py: None,
            n: 2_000,
            rust: None,
        },
        Workload {
            name: "derivative",
            pra: "benches/workloads/derivative.pra",
            py: None,
            n: 2_000,
            rust: None,
        },
        // Internal baseline: collections exercise the AST/VM array, dict, and string paths.
        // Kept small enough to stay a baseline rather than an O(n²) stress test: the AST array
        // `push` and dict index-assignment paths copy on write, so large sizes dominate wall time.
        Workload {
            name: "sort",
            pra: "benches/workloads/sort.pra",
            py: None,
            n: 5_000,
            rust: None,
        },
        Workload {
            name: "dict",
            pra: "benches/workloads/dict.pra",
            py: None,
            n: 2_000,
            rust: None,
        },
        Workload {
            name: "strings",
            pra: "benches/workloads/strings.pra",
            py: None,
            n: 5_000,
            rust: None,
        },
    ]
}

fn fmt_ns(d: Duration) -> String {
    format!("{:.0} ns", d.as_nanos())
}

fn fmt_opt_ns(d: Option<Duration>) -> String {
    d.map_or_else(|| "-".to_string(), fmt_ns)
}

fn fmt_mult(a: Duration, b: Option<Duration>) -> String {
    b.map_or_else(
        || "-".to_string(),
        |b| {
            let x = a.as_secs_f64() / b.as_secs_f64();
            // Sub-0.1 multipliers would round to "0.0×" and hide which side is faster.
            if x < 0.1 {
                format!("{x:.2}×")
            } else {
                format!("{x:.1}×")
            }
        },
    )
}

/// CPU model for the results header (Linux `/proc/cpuinfo`; falls back to the architecture).
fn cpu_model() -> String {
    if let Ok(text) = std::fs::read_to_string("/proc/cpuinfo")
        && let Some(line) = text
            .lines()
            .find(|l| l.starts_with("model name") || l.starts_with("Model"))
        && let Some(value) = line.split(':').nth(1)
    {
        return value.trim().to_string();
    }
    std::env::consts::ARCH.to_string()
}

/// `rustc -V`, or `-` when the toolchain is not on `PATH`.
fn rustc_version() -> String {
    Command::new("rustc")
        .arg("-V")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|| "-".to_string())
}

/// Short commit hash, or `-` outside a git checkout. A dirty worktree is marked so results are not
/// mistaken for a tagged revision.
fn commit_hash() -> String {
    let mut hash = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|| "-".to_string());
    if let Ok(status) = Command::new("git").args(["status", "--porcelain"]).output()
        && !status.stdout.is_empty()
    {
        hash.push_str("-dirty");
    }
    hash
}

/// UTC timestamp (`date -u`), or `-` when the command is unavailable.
fn utc_now() -> String {
    Command::new("date")
        .args(["-u", "+%Y-%m-%dT%H:%M:%SZ"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|| "-".to_string())
}

fn environment_header() -> String {
    let profile = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    format!(
        "- CPU: {}\n- OS: {} {}\n- Toolchain: {}\n- Build: `{}` (cargo bench)\n- Commit: `{}`\n- Measured: {}\n- Samples: {SAMPLES} timed runs after {WARMUP} warmup runs per combination\n\n",
        cpu_model(),
        std::env::consts::OS,
        std::env::consts::ARCH,
        rustc_version(),
        profile,
        commit_hash(),
        utc_now()
    )
}

fn main() {
    prima_language::prima_stdlib::init();
    let py_file = PathBuf::from("benches/bench_ref.py");
    let wl = workloads();
    let mut table = String::new();
    table.push_str("# Prima execution-path and cross-language baseline\n\n");
    table.push_str(
        "Deterministic, scalar-valued kernels measured in steady state: Prima runs in-process\n",
    );
    table.push_str(
        "(warm environment, so parsing and module loading are excluded); Python times its own\n",
    );
    table.push_str("kernel with `perf_counter` so interpreter startup is excluded too.\n\n");
    table.push_str(
        "**Columns**: `AST` is the authoritative interpreter, `VM` forces the bytecode VM, and\n",
    );
    table.push_str(
        "`default` is the JIT → VM → AST dispatch. `AST/default` and `VM/default` are how many\n",
    );
    table.push_str("times faster the default path is than each interpreter (higher = the fast path helps more).\n");
    table.push_str("`Python ×` and `Rust ×` are the multiplier by which the reference is *faster* than Prima\n");
    table.push_str("(1.0× = equal; below 1 means Prima is faster).\n\n");
    table.push_str(
        "Numeric kernels have Python/Rust references; `-` marks symbolic and collection\n",
    );
    table.push_str("workloads that only establish an internal before/after baseline.\n\n");
    table.push_str("**Comparability**: these results are only meaningful on the same machine, toolchain, and\n");
    table.push_str("build profile. Existing numbers are historical references, not cross-machine thresholds.\n\n");
    table.push_str(&environment_header());
    table.push_str("| workload | n | AST (ns) | VM (ns) | default (ns) | Python (ns) | Rust (ns) | AST/default | VM/default | Python × | Rust × |\n");
    table.push_str("|---|---|---|---|---|---|---|---|---|---|---|\n");

    println!(
        "running {} workloads ({SAMPLES} samples, {WARMUP} warmup)...",
        wl.len()
    );

    for w in &wl {
        let (ast, ast_v) = time_prima(w.pra, w.n, SAMPLES, WARMUP, PrimaPath::Ast);
        let (vm, vm_v) = time_prima(w.pra, w.n, SAMPLES, WARMUP, PrimaPath::Vm);
        let (default, default_v) = time_prima(w.pra, w.n, SAMPLES, WARMUP, PrimaPath::Default);

        // Cross-path correctness: the three Prima paths must agree before timing is reported.
        let name = w.name;
        assert!(
            (ast_v - default_v).abs() / ast_v.abs().max(1e-12) < 1e-6,
            "{name}: ast={ast_v} default={default_v} disagree"
        );
        assert!(
            (vm_v - default_v).abs() / vm_v.abs().max(1e-12) < 1e-6,
            "{name}: vm={vm_v} default={default_v} disagree"
        );

        let (py, rust) = match (w.py, w.rust) {
            (Some(py_name), Some(rust_fn)) => {
                let (py_t, py_v) = time_python(&py_file, py_name, w.n, SAMPLES);
                let rust_v = rust_fn(w.n);
                let rust_t = time_rust(rust_fn, w.n, SAMPLES, WARMUP);
                // Cross-language correctness: the three implementations of the same kernel must agree.
                assert!(
                    (default_v - py_v).abs() / default_v.abs().max(1e-12) < 1e-6,
                    "{name}: prima={default_v} python={py_v} disagree"
                );
                assert!(
                    (default_v - rust_v).abs() / default_v.abs().max(1e-12) < 1e-6,
                    "{name}: prima={default_v} rust={rust_v} disagree"
                );
                (Some(py_t), Some(rust_t))
            }
            _ => (None, None),
        };

        let vm_gain = vm.as_secs_f64() / default.as_secs_f64();
        let ast_gain = ast.as_secs_f64() / default.as_secs_f64();
        println!(
            "{:<10} ast={:<14} vm={:<14} default={:<14} py={:<14} rust={:<14} ast/default={:.1}×",
            name,
            fmt_ns(ast),
            fmt_ns(vm),
            fmt_ns(default),
            fmt_opt_ns(py),
            fmt_opt_ns(rust),
            ast_gain
        );
        table.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} | {} | {:.1}× | {:.1}× | {} | {} |\n",
            name,
            w.n,
            fmt_ns(ast),
            fmt_ns(vm),
            fmt_ns(default),
            fmt_opt_ns(py),
            fmt_opt_ns(rust),
            ast_gain,
            vm_gain,
            fmt_mult(default, py),
            fmt_mult(default, rust)
        ));
    }

    std::fs::write("benches/RESULTS.md", &table).expect("cannot write benches/RESULTS.md");
    println!("wrote benches/RESULTS.md");
}
