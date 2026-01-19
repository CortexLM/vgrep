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
fn test_serve_refuses_insecure_non_loopback_without_tls_or_override() {
    let home = tempfile::tempdir().unwrap();

    vgrep()
        .env("HOME", home.path())
        .env("VGREP_HOST", "0.0.0.0")
        .arg("serve")
        .assert()
        .failure()
        .stderr(predicate::str::contains("Refusing to bind to non-loopback"));
}

#[test]
fn test_serve_allows_insecure_override() {
    let home = tempfile::tempdir().unwrap();

    vgrep()
        .env("HOME", home.path())
        .env("VGREP_HOST", "0.0.0.0")
        .env("VGREP_ALLOW_INSECURE_HTTP", "true")
        .arg("serve")
        .assert()
        .failure()
        .stderr(predicate::str::contains("Embedding model not found"));
}
