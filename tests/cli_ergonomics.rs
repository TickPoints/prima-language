//! CLI ergonomics (spec §20): machine-readable diagnostics, project scaffolding, root discovery,
//! color control, and shell completions.

use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::tempdir;

fn prima() -> Command {
    Command::cargo_bin("prima").unwrap()
}

#[test]
fn json_diagnostics_are_ndjson_on_stderr() {
    let output = prima()
        .args(["--json", "run", "examples/broken.pra"])
        .output()
        .expect("spawn prima");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    let mut saw_diagnostic = false;
    for line in stderr.lines().filter(|l| !l.trim().is_empty()) {
        let value: serde_json::Value =
            serde_json::from_str(line).unwrap_or_else(|e| panic!("not JSON: {line:?} ({e})"));
        assert!(value.get("severity").is_some(), "missing severity: {line}");
        assert!(value.get("code").is_some(), "missing code: {line}");
        assert!(value.get("message").is_some(), "missing message: {line}");
        assert!(value.get("spans").is_some(), "missing spans: {line}");
        // The syntax error must carry a code in the appendix C range.
        let code = value["code"].as_str().unwrap_or_default();
        assert!(code.starts_with('E'), "expected an E-code, got {code:?}");
        saw_diagnostic = true;
    }
    assert!(saw_diagnostic, "expected at least one JSON diagnostic");
}

#[test]
fn color_never_emits_no_ansi_escapes() {
    let output = prima()
        .args(["--color", "never", "run", "examples/broken.pra"])
        .output()
        .expect("spawn prima");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!stderr.contains('\u{1b}'), "unexpected ANSI escape: {stderr:?}");
    assert!(stderr.contains("E"), "expected a coded header: {stderr:?}");
}

#[test]
fn new_scaffolds_a_runnable_project() {
    let dir = tempdir().unwrap();
    prima()
        .current_dir(dir.path())
        .args(["new", "demo"])
        .assert()
        .success()
        .stdout(predicate::str::contains("demo"));

    let project = dir.path().join("demo");
    assert!(project.join("prima.toml").is_file());
    assert!(project.join("config.toml").is_file());
    assert!(project.join(".gitignore").is_file());
    assert!(project.join("src/main.pra").is_file());

    // Running with no file argument discovers the project entry (spec §20).
    prima()
        .current_dir(&project)
        .arg("run")
        .assert()
        .success()
        .stdout(predicate::str::contains("hello from demo"));
}

#[test]
fn init_refuses_an_existing_project() {
    let dir = tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(dir.path().join("prima.toml"), "[package]\n").unwrap();
    prima()
        .current_dir(dir.path())
        .arg("init")
        .assert()
        .failure()
        .stderr(predicate::str::contains("already a Prima project"));
}

#[test]
fn completions_emit_a_script() {
    prima()
        .args(["completions", "bash"])
        .assert()
        .success()
        .stdout(predicate::str::contains("prima"));
}

#[test]
fn test_json_reports_ndjson_events_and_summary() {
    let dir = tempdir().unwrap();
    std::fs::write(dir.path().join("good.pra"), "println(\"hi\");\n").unwrap();
    // A file that does not parse is skipped, not failed.
    std::fs::write(dir.path().join("fixture.pra"), "let x = ;\n").unwrap();

    let output = prima()
        .args(["--json", "test"])
        .arg(dir.path())
        .output()
        .expect("spawn prima");
    // Test events go to stderr so the programs' stdout stays clean (here: "hi").
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("hi"), "program output expected: {stdout:?}");

    let stderr = String::from_utf8_lossy(&output.stderr);
    let events: Vec<serde_json::Value> = stderr
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).unwrap_or_else(|e| panic!("not JSON: {l:?} ({e})")))
        .collect();
    assert!(
        events
            .iter()
            .any(|v| v["type"] == "test" && v["status"] == "ok"),
        "expected an ok event: {events:?}"
    );
    assert!(
        events
            .iter()
            .any(|v| v["type"] == "test" && v["status"] == "skip"),
        "expected a skip event: {events:?}"
    );
    let summary = events
        .iter()
        .find(|v| v["type"] == "summary")
        .expect("summary event");
    assert_eq!(summary["passed"], 1);
    assert_eq!(summary["failed"], 0);
    assert_eq!(summary["skipped"], 1);
}

#[test]
fn test_quiet_suppresses_passing_lines_but_keeps_summary() {
    let dir = tempdir().unwrap();
    std::fs::write(dir.path().join("good.pra"), "println(\"hi\");\n").unwrap();
    prima()
        .args(["--quiet", "test"])
        .arg(dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("1 passed, 0 failed"));
}
