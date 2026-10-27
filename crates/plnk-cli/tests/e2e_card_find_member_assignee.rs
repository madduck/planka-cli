use assert_cmd::Command;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn plnk_with_server(server_uri: &str) -> Command {
    let mut cmd = Command::cargo_bin("plnk").unwrap();
    cmd.env("PLANKA_CONFIG", "/tmp/plnk-test-nonexistent/config.toml");
    cmd.env("PLANKA_SERVER", server_uri);
    cmd.env("PLANKA_TOKEN", "test-api-key");
    cmd
}

fn card_json(id: &str, list_id: &str, name: &str) -> serde_json::Value {
    serde_json::json!({
        "id": id,
        "listId": list_id,
        "boardId": "board-1",
        "name": name,
        "description": null,
        "position": 65536.0,
        "isSubscribed": false,
        "createdAt": "2026-04-14T12:00:00Z",
        "updatedAt": null
    })
}

fn task_list_json(id: &str, card_id: &str) -> serde_json::Value {
    serde_json::json!({
        "id": id,
        "cardId": card_id,
        "name": "Checklist",
        "position": 65536.0,
        "createdAt": "2026-04-14T12:00:00Z",
        "updatedAt": null
    })
}

fn task_json(id: &str, task_list_id: &str, assignee: Option<&str>) -> serde_json::Value {
    serde_json::json!({
        "id": id,
        "taskListId": task_list_id,
        "name": "Do the thing",
        "isCompleted": false,
        "position": 65536.0,
        "assigneeUserId": assignee,
        "createdAt": "2026-04-14T12:00:00Z",
        "updatedAt": null
    })
}

fn user_json(id: &str, name: &str, username: &str) -> serde_json::Value {
    serde_json::json!({
        "id": id,
        "name": name,
        "username": username,
        "email": format!("{username}@example.com"),
        "role": "member",
        "isDeactivated": false,
        "createdAt": "2026-04-14T12:00:00Z",
        "updatedAt": null
    })
}

#[allow(clippy::needless_pass_by_value)]
fn board_snapshot(
    cards: serde_json::Value,
    card_memberships: serde_json::Value,
    task_lists: serde_json::Value,
    tasks: serde_json::Value,
) -> serde_json::Value {
    serde_json::json!({
        "item": {
            "id": "board-1",
            "name": "Work",
            "position": 65536.0,
            "projectId": "project-1",
            "createdAt": "2026-04-14T12:00:00Z",
            "updatedAt": null
        },
        "included": {
            "lists": [],
            "cards": cards,
            "labels": [],
            "cardLabels": [],
            "boardMemberships": [],
            "cardMemberships": card_memberships,
            "taskLists": task_lists,
            "tasks": tasks
        }
    })
}

fn users_response() -> serde_json::Value {
    serde_json::json!({
        "items": [
            user_json("user-1", "Jane Doe", "jane"),
            user_json("user-2", "Sam Roe", "sam")
        ]
    })
}

fn fixture_snapshot() -> serde_json::Value {
    board_snapshot(
        serde_json::json!([
            card_json("card-1", "list-1", "Member and assignee"),
            card_json("card-2", "list-1", "Member only"),
            card_json("card-3", "list-1", "Assignee only"),
            card_json("card-4", "list-1", "Untouched")
        ]),
        serde_json::json!([
            {"id": "cm-1", "cardId": "card-1", "userId": "user-1", "createdAt": "2026-04-14T12:00:00Z"},
            {"id": "cm-2", "cardId": "card-2", "userId": "user-1", "createdAt": "2026-04-14T12:00:00Z"}
        ]),
        serde_json::json!([
            task_list_json("tl-1", "card-1"),
            task_list_json("tl-3", "card-3")
        ]),
        serde_json::json!([
            task_json("task-1", "tl-1", Some("user-1")),
            task_json("task-3", "tl-3", Some("user-1"))
        ]),
    )
}

#[tokio::test]
async fn find_by_assignee_matches_task_assignment_only() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/api/boards/board-1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture_snapshot()))
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/api/users"))
        .respond_with(ResponseTemplate::new(200).set_body_json(users_response()))
        .mount(&server)
        .await;

    let output = plnk_with_server(&server.uri())
        .args([
            "card",
            "find",
            "--board",
            "board-1",
            "--assignee",
            "jane",
            "--output",
            "json",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let json: serde_json::Value = serde_json::from_slice(&output).unwrap();
    let ids = json["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|card| card["id"].as_str().unwrap())
        .collect::<Vec<_>>();

    assert_eq!(ids, vec!["card-1", "card-3"]);
    assert_eq!(json["meta"]["count"], 2);
}

#[tokio::test]
async fn find_by_member_matches_card_membership_only() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/api/boards/board-1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture_snapshot()))
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/api/users"))
        .respond_with(ResponseTemplate::new(200).set_body_json(users_response()))
        .mount(&server)
        .await;

    let output = plnk_with_server(&server.uri())
        .args([
            "card", "find", "--board", "board-1", "--member", "user-1", "--output", "json",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let json: serde_json::Value = serde_json::from_slice(&output).unwrap();
    let ids = json["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|card| card["id"].as_str().unwrap())
        .collect::<Vec<_>>();

    assert_eq!(ids, vec!["card-1", "card-2"]);
}

#[tokio::test]
async fn find_with_member_and_assignee_uses_and_semantics() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/api/boards/board-1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture_snapshot()))
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/api/users"))
        .respond_with(ResponseTemplate::new(200).set_body_json(users_response()))
        .mount(&server)
        .await;

    let output = plnk_with_server(&server.uri())
        .args([
            "card",
            "find",
            "--board",
            "board-1",
            "--member",
            "jane",
            "--assignee",
            "jane",
            "--output",
            "json",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let json: serde_json::Value = serde_json::from_slice(&output).unwrap();
    let ids = json["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|card| card["id"].as_str().unwrap())
        .collect::<Vec<_>>();

    assert_eq!(ids, vec!["card-1"]);
}

#[tokio::test]
async fn find_unknown_user_reports_no_match() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/api/users"))
        .respond_with(ResponseTemplate::new(200).set_body_json(users_response()))
        .mount(&server)
        .await;

    let output = plnk_with_server(&server.uri())
        .args([
            "card",
            "find",
            "--board",
            "board-1",
            "--assignee",
            "nobody",
            "--output",
            "json",
        ])
        .assert()
        .code(4)
        .get_output()
        .stdout
        .clone();

    let json: serde_json::Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(json["error"]["type"], "ResourceNotFound");
    assert!(
        json["error"]["message"]
            .as_str()
            .unwrap()
            .contains("No user matching 'nobody'")
    );
}

#[tokio::test]
async fn find_with_member_requires_board_or_list_scope() {
    let server = MockServer::start().await;

    let output = plnk_with_server(&server.uri())
        .args([
            "card",
            "find",
            "--project",
            "project-1",
            "--member",
            "jane",
            "--output",
            "json",
        ])
        .assert()
        .code(2)
        .get_output()
        .stdout
        .clone();

    let json: serde_json::Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(json["error"]["type"], "InvalidOptionValue");
    assert!(
        json["error"]["message"]
            .as_str()
            .unwrap()
            .contains("--list or --board")
    );
}

#[tokio::test]
async fn find_by_assignee_or_member_unions_both_relations() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/api/boards/board-1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture_snapshot()))
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/api/users"))
        .respond_with(ResponseTemplate::new(200).set_body_json(users_response()))
        .mount(&server)
        .await;

    let output = plnk_with_server(&server.uri())
        .args([
            "card",
            "find",
            "--board",
            "board-1",
            "--assignee-or-member",
            "jane",
            "--output",
            "json",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let json: serde_json::Value = serde_json::from_slice(&output).unwrap();
    let ids = json["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|card| card["id"].as_str().unwrap())
        .collect::<Vec<_>>();

    // card-1 both roles, card-2 member only, card-3 assignee only.
    assert_eq!(ids, vec!["card-1", "card-2", "card-3"]);
}

#[tokio::test]
async fn find_assignee_or_member_rejects_combination_with_member() {
    let server = MockServer::start().await;

    let output = plnk_with_server(&server.uri())
        .args([
            "card",
            "find",
            "--board",
            "board-1",
            "--member",
            "jane",
            "--assignee-or-member",
            "jane",
            "--output",
            "json",
        ])
        .assert()
        .code(2)
        .get_output()
        .stdout
        .clone();

    let json: serde_json::Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(json["error"]["type"], "InvalidOptionValue");
    assert!(
        json["error"]["message"]
            .as_str()
            .unwrap()
            .contains("cannot be combined")
    );
}

#[tokio::test]
async fn find_assignee_or_member_requires_board_or_list_scope() {
    let server = MockServer::start().await;

    let output = plnk_with_server(&server.uri())
        .args([
            "card",
            "find",
            "--project",
            "project-1",
            "--assignee-or-member",
            "jane",
            "--output",
            "json",
        ])
        .assert()
        .code(2)
        .get_output()
        .stdout
        .clone();

    let json: serde_json::Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(json["error"]["type"], "InvalidOptionValue");
    assert!(
        json["error"]["message"]
            .as_str()
            .unwrap()
            .contains("--list or --board")
    );
}
