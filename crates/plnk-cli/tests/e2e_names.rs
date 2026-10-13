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
