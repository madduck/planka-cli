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

#[allow(clippy::needless_pass_by_value)]
fn card_json(id: &str, name: &str, due_date: serde_json::Value) -> serde_json::Value {
    json!({
        "id": id,
        "listId": "list-1",
        "boardId": "board-1",
        "name": name,
        "description": null,
        "position": 65536.0,
        "dueDate": due_date,
        "isSubscribed": false,
        "createdAt": "2026-01-01T12:00:00Z",
        "updatedAt": null
    })
}

#[allow(clippy::needless_pass_by_value)]
fn board_snapshot(
    cards: serde_json::Value,
    labels: serde_json::Value,
    card_labels: serde_json::Value,
) -> serde_json::Value {
    json!({
        "item": {
            "id": "board-1",
            "name": "Work",
            "position": 65536.0,
            "projectId": "project-1",
            "createdAt": "2026-01-01T12:00:00Z",
            "updatedAt": null
        },
        "included": {
            "lists": [],
            "cards": cards,
            "labels": labels,
            "cardLabels": card_labels,
            "boardMemberships": []
        }
    })
}

/// Five cards spanning the boundary dates used across the tests:
/// before (Apr 10), on (Apr 20), inside (Apr 25), after (May 1), and no due date.
fn due_fixture() -> serde_json::Value {
    json!([
        card_json(
            "card-before",
            "Card before",
            json!("2026-04-10T00:00:00.000Z")
        ),
        card_json(
            "card-boundary",
            "Card boundary",
            json!("2026-04-20T00:00:00.000Z")
        ),
        card_json(
            "card-inside",
            "Card inside",
            json!("2026-04-25T12:00:00.000Z")
        ),
        card_json(
            "card-after",
            "Card after",
            json!("2026-05-01T00:00:00.000Z")
        ),
        card_json(
            "card-null",
            "Card without due date",
            serde_json::Value::Null
        )
    ])
}

async fn mount_board(server: &MockServer, snapshot: serde_json::Value) {
    Mock::given(method("GET"))
        .and(path("/api/boards/board-1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(snapshot))
        .mount(server)
        .await;
}

fn ids_from(stdout: &[u8]) -> Vec<String> {
    let json: serde_json::Value = serde_json::from_slice(stdout).unwrap();
    json["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|card| card["id"].as_str().unwrap().to_string())
        .collect()
}

#[tokio::test]
async fn card_find_due_after_excludes_boundary_and_null() {
    let server = MockServer::start().await;
    mount_board(&server, board_snapshot(due_fixture(), json!([]), json!([]))).await;

    let output = plnk_with_server(&server.uri())
        .args([
            "card",
            "find",
            "--board",
            "board-1",
            "--due-after",
            "2026-04-20",
            "--output",
            "json",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    assert_eq!(ids_from(&output), vec!["card-inside", "card-after"]);
}

#[tokio::test]
async fn card_find_due_before_excludes_boundary_and_null() {
    let server = MockServer::start().await;
    mount_board(&server, board_snapshot(due_fixture(), json!([]), json!([]))).await;

    let output = plnk_with_server(&server.uri())
        .args([
            "card",
            "find",
            "--board",
            "board-1",
            "--due-before",
            "2026-04-20",
            "--output",
            "json",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    assert_eq!(ids_from(&output), vec!["card-before"]);
}

#[tokio::test]
async fn card_find_due_between_combines_bounds_strictly() {
    let server = MockServer::start().await;
    mount_board(&server, board_snapshot(due_fixture(), json!([]), json!([]))).await;

    let output = plnk_with_server(&server.uri())
        .args([
            "card",
            "find",
            "--board",
            "board-1",
            "--due-after",
            "2026-04-10",
            "--due-before",
            "2026-05-01",
            "--output",
            "json",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    assert_eq!(ids_from(&output), vec!["card-boundary", "card-inside"]);
}

#[tokio::test]
async fn card_find_due_filter_combines_with_title() {
    let server = MockServer::start().await;
    mount_board(&server, board_snapshot(due_fixture(), json!([]), json!([]))).await;

    let output = plnk_with_server(&server.uri())
        .args([
            "card",
            "find",
            "--board",
            "board-1",
            "--title",
            "Card",
            "--due-before",
            "2026-04-25",
            "--output",
            "json",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    assert_eq!(ids_from(&output), vec!["card-before", "card-boundary"]);
}

#[tokio::test]
async fn card_find_due_filter_combines_with_label() {
    let server = MockServer::start().await;
    let snapshot = board_snapshot(
        json!([
            card_json(
                "card-inside",
                "Card inside",
                json!("2026-04-25T12:00:00.000Z")
            ),
            card_json(
                "card-after",
                "Card after",
                json!("2026-05-01T00:00:00.000Z")
            ),
            card_json(
                "card-null",
                "Card without due date",
                serde_json::Value::Null
            )
        ]),
        json!([{
            "id": "label-red",
            "boardId": "board-1",
            "name": "Urgent",
            "color": "berry-red",
            "position": 65536.0,
            "createdAt": "2026-01-01T12:00:00Z",
            "updatedAt": null
        }]),
        json!([
            {"id": "cl-1", "cardId": "card-inside", "labelId": "label-red", "createdAt": "2026-01-01T12:00:00Z"},
            {"id": "cl-2", "cardId": "card-after", "labelId": "label-red", "createdAt": "2026-01-01T12:00:00Z"}
        ]),
    );
    mount_board(&server, snapshot).await;

    let output = plnk_with_server(&server.uri())
        .args([
            "card",
            "find",
            "--board",
            "board-1",
            "--label",
            "Urgent",
            "--due-after",
            "2026-04-30",
            "--output",
            "json",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    assert_eq!(ids_from(&output), vec!["card-after"]);
}

#[tokio::test]
async fn card_find_without_due_flags_keeps_null_due_dates() {
    let server = MockServer::start().await;
    mount_board(&server, board_snapshot(due_fixture(), json!([]), json!([]))).await;

    let output = plnk_with_server(&server.uri())
        .args([
            "card", "find", "--board", "board-1", "--title", "Card", "--output", "json",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let ids = ids_from(&output);
    assert!(ids.contains(&"card-null".to_string()));
    assert_eq!(ids.len(), 5);
}

#[tokio::test]
async fn card_find_invalid_due_timestamp_exits_2() {
    let server = MockServer::start().await;

    let output = plnk_with_server(&server.uri())
        .args([
            "card",
            "find",
            "--board",
            "board-1",
            "--due-after",
            "not-a-date",
            "--output",
            "json",
        ])
        .assert()
        .failure()
        .code(2)
        .get_output()
        .stdout
        .clone();

    let json: serde_json::Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(json["error"]["type"], "InvalidOptionValue");
    assert_eq!(json["error"]["field"], "--due-after");
    assert!(
        json["error"]["message"]
            .as_str()
            .unwrap()
            .contains("not-a-date")
    );
}

#[tokio::test]
async fn card_find_warns_on_unparseable_due_date() {
    let server = MockServer::start().await;
    mount_board(
        &server,
        board_snapshot(
            json!([
                card_json("card-ok", "Card ok", json!("2026-04-25T12:00:00.000Z")),
                card_json("card-bad", "Card bad", json!("not-a-timestamp"))
            ]),
            json!([]),
            json!([]),
        ),
    )
    .await;

    let output = plnk_with_server(&server.uri())
        .args([
            "card",
            "find",
            "--board",
            "board-1",
            "--due-after",
            "2026-04-01",
            "--output",
            "json",
        ])
        .assert()
        .success()
        .stderr(predicate::str::contains("unparseable dueDate"))
        .stderr(predicate::str::contains("card-bad"))
        .get_output()
        .stdout
        .clone();

    assert_eq!(ids_from(&output), vec!["card-ok"]);
}

#[tokio::test]
async fn card_list_due_before_filters_cards() {
    let server = MockServer::start().await;
    mount_board(&server, board_snapshot(due_fixture(), json!([]), json!([]))).await;

    let output = plnk_with_server(&server.uri())
        .args([
            "card",
            "list",
            "--board",
            "board-1",
            "--due-before",
            "2026-04-20",
            "--output",
            "json",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    assert_eq!(ids_from(&output), vec!["card-before"]);
}

#[tokio::test]
async fn card_list_due_between_combines_bounds_strictly() {
    let server = MockServer::start().await;
    mount_board(&server, board_snapshot(due_fixture(), json!([]), json!([]))).await;

    let output = plnk_with_server(&server.uri())
        .args([
            "card",
            "list",
            "--board",
            "board-1",
            "--due-after",
            "2026-04-10",
            "--due-before",
            "2026-05-01",
            "--output",
            "json",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    assert_eq!(ids_from(&output), vec!["card-boundary", "card-inside"]);
}

#[tokio::test]
async fn card_list_without_due_flags_keeps_null_due_dates() {
    let server = MockServer::start().await;
    mount_board(&server, board_snapshot(due_fixture(), json!([]), json!([]))).await;

    let output = plnk_with_server(&server.uri())
        .args(["card", "list", "--board", "board-1", "--output", "json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let ids = ids_from(&output);
    assert!(ids.contains(&"card-null".to_string()));
    assert_eq!(ids.len(), 5);
}

#[tokio::test]
async fn card_list_invalid_due_timestamp_exits_2() {
    let server = MockServer::start().await;

    let output = plnk_with_server(&server.uri())
        .args([
            "card",
            "list",
            "--board",
            "board-1",
            "--due-before",
            "not-a-date",
            "--output",
            "json",
        ])
        .assert()
        .failure()
        .code(2)
        .get_output()
        .stdout
        .clone();

    let json: serde_json::Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(json["error"]["type"], "InvalidOptionValue");
    assert_eq!(json["error"]["field"], "--due-before");
}

/// Relative bounds are resolved against the process's local time. Pin `TZ` so
/// the fixture and the parser agree regardless of the machine running the test.
fn plnk_with_tz(server_uri: &str, tz: &str) -> Command {
    let mut cmd = plnk_with_server(server_uri);
    cmd.env("TZ", tz);
    cmd
}

/// A card due far enough in the past that `--due-after today` must exclude it,
/// and one due far enough ahead that it must be included. The dates are derived
/// from the real clock on purpose: the point is only to prove that relative
/// words reach the filter, not to test the calendar arithmetic, which the unit
/// tests cover against a frozen instant.
#[tokio::test]
async fn card_find_accepts_relative_due_bounds() {
    let server = MockServer::start().await;
    let today = time::OffsetDateTime::now_utc().date();
    let long_ago = today - time::Duration::days(400);
    let long_ahead = today + time::Duration::days(400);

    let iso = |date: time::Date| format!("{date}T12:00:00.000Z");
    mount_board(
        &server,
        board_snapshot(
            json!([
                card_json("card-past", "Card past", json!(iso(long_ago))),
                card_json("card-future", "Card future", json!(iso(long_ahead))),
                card_json(
                    "card-null",
                    "Card without due date",
                    serde_json::Value::Null
                )
            ]),
            json!([]),
            json!([]),
        ),
    )
    .await;

    let output = plnk_with_tz(&server.uri(), "UTC")
        .args([
            "card",
            "find",
            "--board",
            "board-1",
            "--due-after",
            "today",
            "--output",
            "json",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    assert_eq!(ids_from(&output), vec!["card-future"]);
}

#[tokio::test]
async fn card_find_relative_bounds_combine() {
    let server = MockServer::start().await;
    let today = time::OffsetDateTime::now_utc().date();
    let soon = today + time::Duration::days(3);
    let long_ahead = today + time::Duration::days(400);

    mount_board(
        &server,
        board_snapshot(
            json!([
                card_json(
                    "card-soon",
                    "Card soon",
                    json!(format!("{soon}T12:00:00.000Z"))
                ),
                card_json(
                    "card-far",
                    "Card far",
                    json!(format!("{long_ahead}T12:00:00.000Z"))
                )
            ]),
            json!([]),
            json!([]),
        ),
    )
    .await;

    let output = plnk_with_tz(&server.uri(), "UTC")
        .args([
            "card",
            "find",
            "--board",
            "board-1",
            "--due-after",
            "today",
            "--due-before",
            "next month",
            "--output",
            "json",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    assert_eq!(ids_from(&output), vec!["card-soon"]);
}

#[tokio::test]
async fn card_list_accepts_relative_due_bounds() {
    let server = MockServer::start().await;
    let today = time::OffsetDateTime::now_utc().date();
    let long_ago = today - time::Duration::days(400);

    mount_board(
        &server,
        board_snapshot(
            json!([card_json(
                "card-past",
                "Card past",
                json!(format!("{long_ago}T12:00:00.000Z"))
            )]),
            json!([]),
            json!([]),
        ),
    )
    .await;

    let output = plnk_with_tz(&server.uri(), "UTC")
        .args([
            "card",
            "list",
            "--board",
            "board-1",
            "--due-before",
            "yesterday",
            "--output",
            "json",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    assert_eq!(ids_from(&output), vec!["card-past"]);
}

#[tokio::test]
async fn relative_due_bound_error_mentions_the_grammar() {
    let server = MockServer::start().await;

    let output = plnk_with_server(&server.uri())
        .args([
            "card",
            "find",
            "--board",
            "board-1",
            "--due-before",
            "next friday",
            "--output",
            "json",
        ])
        .assert()
        .failure()
        .code(2)
        .get_output()
        .stdout
        .clone();

    let json: serde_json::Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(json["error"]["type"], "InvalidOptionValue");
    assert_eq!(json["error"]["field"], "--due-before");
    let message = json["error"]["message"].as_str().unwrap();
    assert!(message.contains("relative date"), "got: {message}");
    assert!(message.contains("ISO 8601"), "got: {message}");
}
