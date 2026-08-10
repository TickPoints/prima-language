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
