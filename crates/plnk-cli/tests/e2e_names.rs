//! E2E tests for the `--names` mode plumbing.
//!
//! Resource-specific resolution and decoration live in the commits that add
//! each resource. This file covers the mode flags themselves.

use assert_cmd::Command;
use predicates::prelude::*;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn plnk() -> Command {
    let mut cmd = Command::cargo_bin("plnk").unwrap();
    cmd.env("PLANKA_CONFIG", "/tmp/plnk-test-nonexistent/config.toml");
    cmd.env_remove("PLNK_PROJECT");
    cmd.env_remove("PLNK_BOARD");
    cmd.env_remove("PLNK_LIST");
    cmd
}

fn plnk_with_server(server_uri: &str) -> Command {
    let mut cmd = plnk();
    cmd.env("PLANKA_SERVER", server_uri);
    cmd.env("PLANKA_TOKEN", "test-api-key");
    cmd
}

#[test]
fn names_and_no_names_conflict() {
    plnk()
        .args(["--names", "--no-names", "project", "list"])
        .assert()
        .failure()
        .code(2)
        .stderr(predicate::str::contains("cannot be used with"));
}

#[tokio::test]
async fn names_flag_does_not_change_a_command_without_mappings() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/projects"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "items": [{"id":"1","name":"P","createdAt":"2026-01-01T00:00:00Z","updatedAt":null}]
        })))
        .mount(&server)
        .await;

    let output = plnk_with_server(&server.uri())
        .args(["--names", "project", "list", "--output", "json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let json: serde_json::Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(json["success"], true);
    assert_eq!(json["data"][0]["name"], "P");
}

// ─── Project: input resolution, JSON decoration, text substitution ────────

fn project_list_body() -> serde_json::Value {
    serde_json::json!({
        "items": [
            {"id":"1","name":"Platform","createdAt":"2026-01-01T00:00:00Z","updatedAt":null},
        ]
    })
}

fn project_snapshot_body() -> serde_json::Value {
    serde_json::json!({
        "item": {"id":"1","name":"Platform","createdAt":"2026-01-01T00:00:00Z","updatedAt":null},
        "included": {
            "boards": [
                {"id":"10","projectId":"1","name":"Sprint","position":65536,
                 "createdAt":"2026-01-01T00:00:00Z","updatedAt":null}
            ]
        }
    })
}

#[tokio::test]
async fn project_name_resolves_and_decorates_json() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/projects"))
        .respond_with(ResponseTemplate::new(200).set_body_json(project_list_body()))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/projects/1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(project_snapshot_body()))
        .mount(&server)
        .await;

    let output = plnk_with_server(&server.uri())
        .args([
            "--names",
            "board",
            "list",
            "--project",
            "Platform",
            "--output",
            "json",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let json: serde_json::Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(json["data"][0]["projectId"], "1");
    assert_eq!(json["data"][0]["projectName"], "Platform");
}

#[tokio::test]
async fn project_name_is_substituted_in_table() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/projects"))
        .respond_with(ResponseTemplate::new(200).set_body_json(project_list_body()))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/projects/1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(project_snapshot_body()))
        .mount(&server)
        .await;

    plnk_with_server(&server.uri())
        .args(["--names", "board", "list", "--project", "Platform"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Platform"))
        .stdout(predicate::str::contains("| 1 |").not());
}

#[tokio::test]
async fn ambiguous_project_name_lists_candidates() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/projects"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "items": [
                {"id":"1","name":"Platform One","createdAt":"2026-01-01T00:00:00Z","updatedAt":null},
                {"id":"2","name":"Platform Two","createdAt":"2026-01-01T00:00:00Z","updatedAt":null}
            ]
        })))
        .mount(&server)
        .await;

    plnk_with_server(&server.uri())
        .args(["--names", "board", "list", "--project", "Platform"])
        .assert()
        .failure()
        .code(2)
        .stderr(predicate::str::contains("Platform One (1)"))
        .stderr(predicate::str::contains("Platform Two (2)"));
}

#[tokio::test]
async fn unknown_project_name_exits_not_found() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/projects"))
        .respond_with(ResponseTemplate::new(200).set_body_json(project_list_body()))
        .mount(&server)
        .await;

    plnk_with_server(&server.uri())
        .args(["--names", "board", "list", "--project", "Nope"])
        .assert()
        .failure()
        .code(4);
}
