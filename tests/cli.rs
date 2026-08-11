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
        .stdout("[1, 4, 9]\n[11, 12, 13]\n[11, 22, 33]\n");
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

#[test]
fn unimplemented_subcommands_fail() {
    Command::cargo_bin("prima")
        .unwrap()
        .arg("repl")
        .assert()
        .failure()
        .stderr(predicate::str::contains("not implemented"));
}
