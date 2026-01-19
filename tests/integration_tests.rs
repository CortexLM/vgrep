use assert_cmd::Command;
use predicates::prelude::*;

#[allow(deprecated)]
fn vgrep() -> Command {
    Command::cargo_bin("vgrep").unwrap()
}

#[test]
fn test_help() {
    vgrep()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("semantic"));
}

#[test]
fn test_version() {
    vgrep()
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains("vgrep"));
}

#[test]
fn test_status() {
    vgrep()
        .arg("status")
        .assert()
        .success()
        .stdout(predicate::str::contains("Status"));
}

#[test]
fn test_models_info() {
    vgrep()
        .args(["models", "info"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Qwen3-Embedding"));
}

#[test]
fn test_models_list() {
    vgrep()
        .args(["models", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Embedding"));
}

#[test]
fn test_config_show() {
    vgrep()
        .args(["config", "show"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Chunk size"));
}

#[test]
fn test_search_empty_query_should_fail() {
    vgrep()
        .arg("search")
        .arg("")
        .assert()
        .failure()
        .stderr(predicate::str::contains("Search query cannot be empty"));
}

#[test]
fn test_search_whitespace_query_should_fail() {
    vgrep()
        .arg("search")
        .arg("   ")
        .assert()
        .failure()
        .stderr(predicate::str::contains("Search query cannot be empty"));
}
