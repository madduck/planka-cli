//! E2E tests for `--members`: pairing cards with their assigned users.
//!
//! List and find scopes read memberships from the same board snapshot or
//! list-cards response that supplies the cards, so `--members` adds no extra
//! request there. The tests accordingly mount memberships inside the list /
//! board response, not on a per-card `GET /api/cards/{id}`.

use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn plnk_with_server(server_uri: &str) -> Command {
    let mut cmd = Command::cargo_bin("plnk").unwrap();
    cmd.env("PLANKA_CONFIG", "/tmp/plnk-test-nonexistent/config.toml");
    cmd.env("PLANKA_SERVER", server_uri);
    cmd.env("PLANKA_TOKEN", "test-api-key");
    cmd
}

fn card(id: &str, name: &str) -> serde_json::Value {
    json!({
        "id": id,
        "listId": "list-1",
        "boardId": "board-1",
        "name": name,
        "description": null,
        "position": 65536.0,
        "isSubscribed": false,
        "createdAt": "2026-01-01T12:00:00Z",
        "updatedAt": null
    })
}

fn membership(id: &str, card_id: &str, user_id: &str) -> serde_json::Value {
    json!({
        "id": id,
        "cardId": card_id,
        "userId": user_id,
        "createdAt": "2026-01-01T12:00:00Z",
        "updatedAt": null
    })
}

/// A list-cards response carrying the given cards and their memberships.
fn list_cards_body(cards: serde_json::Value, memberships: serde_json::Value) -> serde_json::Value {
    json!({
        "items": cards,
        "included": { "cardLabels": [], "cardMemberships": memberships }
    })
}

/// A board snapshot carrying the given cards and their memberships.
fn board_body(cards: serde_json::Value, memberships: serde_json::Value) -> serde_json::Value {
    json!({
        "item": {
            "id": "board-1",
            "projectId": "project-1",
            "name": "Work",
            "position": 65536.0,
            "createdAt": "2026-01-01T12:00:00Z",
            "updatedAt": null
        },
        "included": {
            "lists": [],
            "cards": cards,
            "labels": [],
            "cardLabels": [],
            "cardMemberships": memberships,
            "boardMemberships": []
        }
    })
}

/// A `GET /api/cards/{id}` snapshot — used by the mutation and get paths,
/// which have no memberships in hand.
fn card_snapshot(card: serde_json::Value, memberships: serde_json::Value) -> serde_json::Value {
    json!({
        "item": card,
        "included": {
            "taskLists": [],
            "tasks": [],
            "cardLabels": [],
            "cardMemberships": memberships,
            "attachments": [],
            "customFieldGroups": [],
            "customFields": [],
            "customFieldValues": []
        }
    })
}

async fn mount_list_cards(server: &MockServer, body: serde_json::Value) {
    Mock::given(method("GET"))
        .and(path("/api/lists/list-1/cards"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(server)
        .await;
}

async fn mount_card_snapshot(server: &MockServer, card_id: &str, snapshot: serde_json::Value) {
    Mock::given(method("GET"))
        .and(path(format!("/api/cards/{card_id}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(snapshot))
        .mount(server)
        .await;
}

async fn mount_user(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/api/users/user-1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "item": {"id":"user-1","name":"Alice Smith","username":"alice","role":"editor",
                     "isDeactivated":false,"createdAt":"2026-01-01T00:00:00Z","updatedAt":null}
        })))
        .mount(server)
        .await;
}

/// The project → board → list enumeration `--names` needs to resolve a
/// scope given as an ID (it still refreshes the name caches).
async fn mount_scope_for_names(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/api/projects"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "items": [{"id":"1","name":"P","createdAt":"2026-01-01T00:00:00Z","updatedAt":null}]
        })))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/projects/1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "item": {"id":"1","name":"P","createdAt":"2026-01-01T00:00:00Z","updatedAt":null},
            "included": {"boards": [{"id":"board-1","projectId":"1","name":"Work","position":65536.0,
                                     "createdAt":"2026-01-01T00:00:00Z","updatedAt":null}],
                         "projectManagers": [], "baseCustomFieldGroups": []}
        })))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/boards/board-1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "item": {"id":"board-1","projectId":"1","name":"Work","position":65536.0,
                     "createdAt":"2026-01-01T00:00:00Z","updatedAt":null},
            "included": {
                "lists": [{"id":"list-1","boardId":"board-1","name":"Todo","position":65536.0,
                           "type":"active","createdAt":"2026-01-01T00:00:00Z","updatedAt":null}],
                "cards": [], "labels": [], "cardLabels": [], "cardMemberships": [],
                "boardMemberships": []
            }
        })))
        .mount(server)
        .await;
}

#[tokio::test]
async fn list_without_members_omits_the_field() {
    let server = MockServer::start().await;
    mount_list_cards(
        &server,
        list_cards_body(json!([card("card-1", "Fix bug")]), json!([])),
    )
    .await;

    let output = plnk_with_server(&server.uri())
        .args(["card", "list", "--list", "list-1", "--output", "json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let json: serde_json::Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(json["data"][0]["id"], "card-1");
    assert!(json["data"][0].get("assignees").is_none());
}

#[tokio::test]
async fn list_with_members_reads_memberships_from_the_same_response() {
    let server = MockServer::start().await;
    mount_list_cards(
        &server,
        list_cards_body(
            json!([card("card-1", "Fix bug"), card("card-2", "Add feature")]),
            json!([membership("m1", "card-1", "user-1")]),
        ),
    )
    .await;

    let output = plnk_with_server(&server.uri())
        .args([
            "card",
            "list",
            "--list",
            "list-1",
            "--members",
            "--output",
            "json",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let json: serde_json::Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(json["data"][0]["id"], "card-1");
    assert_eq!(json["data"][0]["assignees"][0]["userId"], "user-1");
    // A card with no assignees still carries the (empty) field.
    assert_eq!(json["data"][1]["assignees"], json!([]));
}

#[tokio::test]
async fn list_with_members_on_a_board_reads_memberships_from_the_snapshot() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/boards/board-1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(board_body(
            json!([card("card-1", "Fix bug")]),
            json!([membership("m1", "card-1", "user-1")]),
        )))
        .mount(&server)
        .await;

    let output = plnk_with_server(&server.uri())
        .args([
            "card",
            "list",
            "--board",
            "board-1",
            "--members",
            "--output",
            "json",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let json: serde_json::Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(json["data"][0]["assignees"][0]["userId"], "user-1");
}

#[tokio::test]
async fn find_with_members_reads_memberships_from_the_same_response() {
    let server = MockServer::start().await;
    mount_list_cards(
        &server,
        list_cards_body(
            json!([card("card-1", "Fix auth bug")]),
            json!([membership("m1", "card-1", "user-1")]),
        ),
    )
    .await;

    let output = plnk_with_server(&server.uri())
        .args([
            "card",
            "find",
            "--list",
            "list-1",
            "--title",
            "Fix auth",
            "--members",
            "--output",
            "json",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let json: serde_json::Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(json["data"][0]["assignees"][0]["userId"], "user-1");
}

#[tokio::test]
async fn find_with_members_across_a_project_reads_board_snapshots() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/projects/project-1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "item": {"id":"project-1","name":"P","createdAt":"2026-01-01T00:00:00Z","updatedAt":null},
            "included": {
                "boards": [{"id":"board-1","projectId":"project-1","name":"Work","position":65536.0,
                           "createdAt":"2026-01-01T00:00:00Z","updatedAt":null}],
                "projectManagers": [], "baseCustomFieldGroups": []
            }
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/boards/board-1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(board_body(
            json!([card("card-1", "Fix auth bug")]),
            json!([membership("m1", "card-1", "user-1")]),
        )))
        .mount(&server)
        .await;

    let output = plnk_with_server(&server.uri())
        .args([
            "card",
            "find",
            "--project",
            "project-1",
            "--title",
            "Fix auth",
            "--members",
            "--output",
            "json",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let json: serde_json::Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(json["data"][0]["assignees"][0]["userId"], "user-1");
}

#[tokio::test]
async fn members_resolve_usernames_with_names() {
    let server = MockServer::start().await;
    mount_scope_for_names(&server).await;
    mount_list_cards(
        &server,
        list_cards_body(
            json!([card("card-1", "Fix bug")]),
            json!([membership("m1", "card-1", "user-1")]),
        ),
    )
    .await;
    mount_user(&server).await;

    let output = plnk_with_server(&server.uri())
        .args([
            "--names",
            "card",
            "list",
            "--list",
            "list-1",
            "--members",
            "--output",
            "json",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let json: serde_json::Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(json["data"][0]["assignees"][0]["userName"], "alice");
}

#[tokio::test]
async fn members_render_usernames_in_a_table() {
    let server = MockServer::start().await;
    mount_scope_for_names(&server).await;
    mount_list_cards(
        &server,
        list_cards_body(
            json!([card("card-1", "Fix bug")]),
            json!([membership("m1", "card-1", "user-1")]),
        ),
    )
    .await;
    mount_user(&server).await;

    plnk_with_server(&server.uri())
        .args(["--names", "card", "list", "--list", "list-1", "--members"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Assignees"))
        .stdout(predicate::str::contains("alice"));
}

#[tokio::test]
async fn get_many_with_members_includes_assignees() {
    let server = MockServer::start().await;
    mount_card_snapshot(
        &server,
        "card-1",
        card_snapshot(
            card("card-1", "Fix bug"),
            json!([membership("m1", "card-1", "user-1")]),
        ),
    )
    .await;

    let output = plnk_with_server(&server.uri())
        .args([
            "card",
            "get-many",
            "--id",
            "card-1",
            "--members",
            "--output",
            "json",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let json: serde_json::Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(json["data"][0]["id"], "card-1");
    assert_eq!(json["data"][0]["assignees"][0]["userId"], "user-1");
}

#[tokio::test]
async fn get_with_members_includes_assignees() {
    let server = MockServer::start().await;
    mount_card_snapshot(
        &server,
        "card-1",
        card_snapshot(
            card("card-1", "Fix bug"),
            json!([membership("m1", "card-1", "user-1")]),
        ),
    )
    .await;

    let output = plnk_with_server(&server.uri())
        .args(["card", "get", "card-1", "--members", "--output", "json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let json: serde_json::Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(json["data"]["id"], "card-1");
    assert_eq!(json["data"]["assignees"][0]["userId"], "user-1");
}
