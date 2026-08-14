use assert_cmd::Command;
use predicates::prelude::*;

#[test]
fn run_parses_example() {
    Command::cargo_bin("prima")
        .unwrap()
        .arg("run")
        .arg("examples/simple.pra")
        .assert()
        .success();
}

#[test]
fn run_reports_syntax_errors() {
    Command::cargo_bin("prima")
        .unwrap()
        .arg("run")
        .arg("examples/broken.pra")
        .assert()
        .failure()
        .stderr(predicate::str::contains("error"))
        .stderr(predicate::str::contains("-->"));
}

#[test]
fn run_broadcast_example() {
    Command::cargo_bin("prima")
        .unwrap()
        .arg("run")
        .arg("examples/broadcast.pra")
        .assert()
        .success()
        .stdout("[1, 4, 9]\n[11, 12, 13]\n[1, 2, 3, 10, 20, 30]\n[1, 4, 9]\n");
}

#[test]
fn run_tex_example() {
    Command::cargo_bin("prima")
        .unwrap()
        .arg("run")
        .arg("examples/tex_literals.pra")
        .assert()
        .success()
        .stdout("\\sqrt{2} + \\pi\n");
}

#[test]
fn run_euler_identity_example() {
    Command::cargo_bin("prima")
        .unwrap()
        .arg("run")
        .arg("examples/euler_identity.pra")
        .assert()
        .success()
        .stdout("0\n");
}

#[test]
fn run_rational_arithmetic_example() {
    Command::cargo_bin("prima")
        .unwrap()
        .arg("run")
        .arg("examples/rational_arithmetic.pra")
        .assert()
        .success()
        .stdout("\\frac{11}{15}\n1\n");
}

#[test]
fn run_differentiation_example() {
    Command::cargo_bin("prima")
        .unwrap()
        .arg("run")
        .arg("examples/differentiation.pra")
        .assert()
        .success()
        .stdout("2 x + \\cos\\left(x\\right)\n-\\left(\\sin\\left(x\\right)\\right) + 2\n2 x y\nx^{2} + 3 \\left(y^{2}\\right)\n(2 x, 2 y)\n1\n9\n");
}

#[test]
fn run_parfor_example() {
    Command::cargo_bin("prima")
        .unwrap()
        .arg("run")
        .arg("examples/parfor.pra")
        .assert()
        .success()
        .stdout("[0, 1, 4, 9, 16, 25]\n[100, 0, 104, 0, 116, 0]\n[0, 1, 2, 3] [10, 9, 8, 7]\n");
}

#[test]
fn run_parallel_broadcast_example() {
    Command::cargo_bin("prima")
        .unwrap()
        .arg("run")
        .arg("examples/parallel.pra")
        .assert()
        .success()
        .stdout("1 3996002\n1 8994002\n");
}

#[test]
fn run_all_examples_succeed() {
    let examples = [
        "broadcast.pra",
        "tex_literals.pra",
        "euler_identity.pra",
        "rational_arithmetic.pra",
        "symbolic_math.pra",
        "pipeline.pra",
        "number_literals.pra",
        "simple.pra",
        "config_fraction.pra",
        "loop_optimization.pra",
        "collapse.pra",
        "control_flow.pra",
        "try_catch.pra",
        "imports.pra",
        "for_step.pra",
        "classes.pra",
        "patterns.pra",
        "differentiation.pra",
        "parfor.pra",
        "parallel.pra",
        "console_io.pra",
        "arrays.pra",
        "dict_set.pra",
        "comprehension.pra",
        "convenience.pra",
    ];
    for name in examples {
        Command::cargo_bin("prima")
            .unwrap()
            .arg("run")
            .arg(format!("examples/{name}"))
            .assert()
            .success();
    }
}

#[test]
fn repl_evaluates_input() {
    Command::cargo_bin("prima")
        .unwrap()
        .arg("repl")
        .write_stdin("1 + 2\n:q\n")
        .assert()
        .success()
        .stdout(predicate::str::contains("3"));
}

#[test]
fn repl_multiline_completes() {
    Command::cargo_bin("prima")
        .unwrap()
        .arg("repl")
        .write_stdin("let f(x) = x^2\nf(3)\n:q\n")
        .assert()
        .success()
        .stdout(predicate::str::contains("9"));
}

#[test]
fn fmt_rewrites_file_in_place() {
    let mut file = std::env::temp_dir();
    file.push(format!("prima_fmt_{}.pra", std::process::id()));
    std::fs::write(&file, "let a=1;let b=2;").unwrap();
    Command::cargo_bin("prima")
        .unwrap()
        .arg("fmt")
        .arg("-w")
        .arg(&file)
        .assert()
        .success();
    let content = std::fs::read_to_string(&file).unwrap();
    assert!(content.contains("let a = 1;\nlet b = 2;"), "got {content:?}");
    let _ = std::fs::remove_file(&file);
}

#[test]
fn fmt_check_detects_unformatted() {
    let mut file = std::env::temp_dir();
    file.push(format!("prima_fmt_check_{}.pra", std::process::id()));
    std::fs::write(&file, "let a=1;let b=2;").unwrap();
    Command::cargo_bin("prima")
        .unwrap()
        .arg("fmt")
        .arg("--check")
        .arg(&file)
        .assert()
        .failure();
    Command::cargo_bin("prima")
        .unwrap()
        .arg("fmt")
        .arg("-w")
        .arg(&file)
        .assert()
        .success();
    Command::cargo_bin("prima")
        .unwrap()
        .arg("fmt")
        .arg("--check")
        .arg(&file)
        .assert()
        .success();
    let _ = std::fs::remove_file(&file);
}

#[test]
fn test_runs_examples() {
    Command::cargo_bin("prima")
        .unwrap()
        .arg("test")
        .arg("examples/")
        .assert()
        .success()
        .stdout(predicate::str::contains("passed"));
}

#[test]
fn test_reports_failure() {
    let mut dir = std::env::temp_dir();
    dir.push(format!("prima_test_fail_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("bad.pra"), "let x = 1/0;\n").unwrap();
    Command::cargo_bin("prima")
        .unwrap()
        .arg("test")
        .arg(&dir)
        .assert()
        .failure()
        .stdout(predicate::str::contains("FAIL"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn doc_lists_definitions() {
    Command::cargo_bin("prima")
        .unwrap()
        .arg("doc")
        .arg("examples/classes.pra")
        .assert()
        .success()
        .stdout(predicate::str::contains("class"))
        .stdout(predicate::str::contains("increment"));
}

#[test]
fn check_deny_promotes_warning() {
    // Newline-separated statements trigger W0001 (deprecated separator, spec §16.5).
    let mut file = std::env::temp_dir();
    file.push(format!("prima_deny_{}.pra", std::process::id()));
    std::fs::write(&file, "let a = 1\nlet b = 2\n").unwrap();
    Command::cargo_bin("prima")
        .unwrap()
        .arg("check")
        .arg(&file)
        .assert()
        .success()
        .stderr(predicate::str::contains("W0001"));
    Command::cargo_bin("prima")
        .unwrap()
        .arg("check")
        .arg("--deny")
        .arg("W0001")
        .arg(&file)
        .assert()
        .failure()
        .stderr(predicate::str::contains("W0001"));
    let _ = std::fs::remove_file(&file);
}

#[test]
fn check_reports_type_errors() {
    let src = "let x: F64 = sqrt(2);\n";
    let mut file = std::env::temp_dir();
    file.push(format!("prima_check_{}.pra", std::process::id()));
    std::fs::write(&file, src).unwrap();
    Command::cargo_bin("prima")
        .unwrap()
        .arg("check")
        .arg(&file)
        .assert()
        .failure()
        .stderr(predicate::str::contains("type mismatch"))
        .stderr(predicate::str::contains("-->"));
    let _ = std::fs::remove_file(&file);
}

#[test]
fn run_reports_located_runtime_errors() {
    let src = "let a = 1;\nlet b = a + (1/0);\n";
    let mut file = std::env::temp_dir();
    file.push(format!("prima_run_err_{}.pra", std::process::id()));
    std::fs::write(&file, src).unwrap();
    Command::cargo_bin("prima")
        .unwrap()
        .arg("run")
        .arg(&file)
        .assert()
        .failure()
        .stderr(predicate::str::contains("division by zero"))
        .stderr(predicate::str::contains("-->"));
    let _ = std::fs::remove_file(&file);
}

#[test]
fn check_passes_clean_file() {
    let src = "let y: F64 = to_f64(sqrt(2));\n";
    let mut file = std::env::temp_dir();
    file.push(format!("prima_check_ok_{}.pra", std::process::id()));
    std::fs::write(&file, src).unwrap();
    Command::cargo_bin("prima")
        .unwrap()
        .arg("check")
        .arg(&file)
        .assert()
        .success();
    let _ = std::fs::remove_file(&file);
}

#[test]
fn parse_dumps_ast() {
    Command::cargo_bin("prima")
        .unwrap()
        .arg("parse")
        .arg("examples/simple.pra")
        .assert()
        .success()
        .stdout(predicate::str::contains("Program {"));
}

#[test]
fn run_missing_file_fails() {
    Command::cargo_bin("prima")
        .unwrap()
        .arg("run")
        .arg("does_not_exist.pra")
        .assert()
        .failure();
}
