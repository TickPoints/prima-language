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
        .stderr(predicate::str::contains("error"));
}

#[test]
fn run_phase1_milestones() {
    Command::cargo_bin("prima")
        .unwrap()
        .arg("run")
        .arg("examples/phase1.pra")
        .assert()
        .success()
        .stdout("[1, 4, 9]\n\\sqrt{2} + \\pi\n0\n");
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
