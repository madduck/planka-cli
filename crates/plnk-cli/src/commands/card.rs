use plnk_core::api::{CardApi, LabelApi, ListApi, UserApi, match_by_name};
use plnk_core::error::PlankaError;
use plnk_core::models::{CreateCard, ErrorFailure, FindScope, Label, MoveCard, UpdateCard, User};
use serde::Serialize;

use crate::app::OutputFormat;
use crate::commands::project::confirm_delete;
use crate::input::resolve_text;
use crate::output::{
    render_collection, render_collection_with_meta, render_item, render_message, render_snapshot,
};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CardGetManyMeta {
    count: usize,
    requested_count: usize,
    found_count: usize,
    missing_count: usize,
    missing_ids: Vec<String>,
    concurrency: usize,
    allow_missing: bool,
}

/// Parse position flag: "top", "bottom", or numeric value.
fn parse_position(pos: &str) -> Result<f64, PlankaError> {
    match pos.to_lowercase().as_str() {
        "top" => Ok(0.0),
        "bottom" => Ok(f64::MAX),
        other => other
            .parse::<f64>()
            .map_err(|_| PlankaError::InvalidOptionValue {
                field: "--position".to_string(),
                message: format!("Must be 'top', 'bottom', or a number, got '{other}'"),
            }),
    }
}

fn resolve_label_id(labels: &[Label], query: &str) -> Result<String, PlankaError> {
    if labels.iter().any(|label| label.id == query) {
        return Ok(query.to_string());
    }

    let matched = match_by_name(labels, query);
    match matched.len() {
        0 => Err(PlankaError::NotFoundMessage {
            message: format!(
                "No label matching '{query}' was found on this board. \
                 Use 'plnk label list --board <boardId>' to inspect available labels \
                 or pass a label ID."
            ),
        }),
        1 => Ok(matched[0].id.clone()),
        _ => {
            let candidates = matched
                .iter()
                .map(|label| {
                    let name = label.name.as_deref().unwrap_or("<unnamed>");
                    format!("{name} ({})", label.id)
                })
                .collect::<Vec<_>>()
                .join(", ");
            Err(PlankaError::InvalidOptionValue {
                field: "--label".to_string(),
                message: format!(
                    "Label '{query}' matched multiple labels on this board. \
                     Be more specific or use a label ID. Matches: {candidates}"
                ),
            })
        }
    }
}

/// Resolve a user query (ID, name, or username) to a single user ID.
///
/// Resolution is server-wide rather than board-scoped: a task assignee need not
/// be a board member, and the board snapshot's `included.users` only carries
/// board members plus card creators, so it cannot be trusted as the candidate
/// set.
fn resolve_user_id(users: &[User], query: &str) -> Result<String, PlankaError> {
    if users.iter().any(|user| user.id == query) {
        return Ok(query.to_string());
    }

    if let Some(user) = users
        .iter()
        .find(|user| user.username.as_deref() == Some(query))
    {
        return Ok(user.id.clone());
    }

    let matched = match_by_name(users, query);
    match matched.len() {
        0 => Err(PlankaError::NotFoundMessage {
            message: format!(
                "No user matching '{query}' was found. \
                 Use 'plnk user list' to inspect available users or pass a user ID."
            ),
        }),
        1 => Ok(matched[0].id.clone()),
        _ => {
            let candidates = matched
                .iter()
                .map(|user| {
                    let handle = user.username.as_deref().map_or_else(
                        || user.email.clone().unwrap_or_default(),
                        |username| format!("@{username}"),
                    );
                    format!("{handle} ({})", user.id)
                })
                .collect::<Vec<_>>()
                .join(", ");
            Err(PlankaError::InvalidOptionValue {
                field: "--member/--assignee".to_string(),
                message: format!(
                    "User '{query}' matched multiple users. \
                     Be more specific or use a user ID. Matches: {candidates}"
                ),
            })
        }
    }
}

/// Board context for narrowing a card search from a board snapshot.
struct BoardScope {
    board_id: String,
    list_id: Option<String>,
    label_ids: Vec<String>,
    member_ids: Vec<String>,
    assignee_ids: Vec<String>,
    either_user_role: bool,
}

/// Resolve `--list`/`--board` plus the board-scoped `--label`, `--member`,
/// `--assignee`, and `--assignee-or-member` filters into one scope.
///
/// Returns `None` when no board-scoped filter was requested, leaving the caller
/// free to use a cheaper per-scope path. A `--list` scope is promoted to its
/// board because all the filters are computed from a board snapshot.
async fn resolve_board_scope(
    client: &(impl LabelApi + ListApi + UserApi),
    list: Option<&str>,
    board: Option<&str>,
    labels: &[String],
    members: &[String],
    assignees: &[String],
    either_users: &[String],
) -> Result<Option<BoardScope>, PlankaError> {
    if !either_users.is_empty() && (!members.is_empty() || !assignees.is_empty()) {
        return Err(PlankaError::InvalidOptionValue {
            field: "--assignee-or-member".to_string(),
            message: "--assignee-or-member cannot be combined with --member or --assignee; \
                      it is shorthand for both at once"
                .to_string(),
        });
    }

    let has_user_filter = !members.is_empty() || !assignees.is_empty() || !either_users.is_empty();

    if labels.is_empty() && !has_user_filter {
        return Ok(None);
    }

    let (board_id, list_id) = if let Some(list_id) = list {
        let list_item = client.get_list(list_id).await?;
        (list_item.board_id, Some(list_id.to_string()))
    } else if let Some(board_id) = board {
        (board_id.to_string(), None)
    } else {
        let field = if labels.is_empty() {
            "--member/--assignee/--assignee-or-member"
        } else {
            "--label"
        };
        return Err(PlankaError::InvalidOptionValue {
            field: field.to_string(),
            message: format!(
                "{field} is only supported with --list or --board scopes because \
                 labels and members are board-scoped"
            ),
        });
    };

    let label_ids = if labels.is_empty() {
        Vec::new()
    } else {
        let board_labels = client.list_labels(&board_id).await?;
        labels
            .iter()
            .map(|query| resolve_label_id(&board_labels, query))
            .collect::<Result<Vec<_>, _>>()?
    };

    // `--assignee-or-member` feeds both dimensions and asks for their union.
    let (member_queries, assignee_queries) = if either_users.is_empty() {
        (members, assignees)
    } else {
        (either_users, either_users)
    };

    let (member_ids, assignee_ids) = if has_user_filter {
        let users = client.list_users().await?;
        (
            member_queries
                .iter()
                .map(|query| resolve_user_id(&users, query))
                .collect::<Result<Vec<_>, _>>()?,
            assignee_queries
                .iter()
                .map(|query| resolve_user_id(&users, query))
                .collect::<Result<Vec<_>, _>>()?,
        )
    } else {
        (Vec::new(), Vec::new())
    };

    Ok(Some(BoardScope {
        board_id,
        list_id,
        label_ids,
        member_ids,
        assignee_ids,
        either_user_role: !either_users.is_empty(),
    }))
}

fn build_get_many_meta(
    found_count: usize,
    requested_count: usize,
    missing_ids: Vec<String>,
    concurrency: usize,
    allow_missing: bool,
) -> CardGetManyMeta {
    CardGetManyMeta {
        count: found_count,
        requested_count,
        found_count,
        missing_count: missing_ids.len(),
        missing_ids,
        concurrency,
        allow_missing,
    }
}

fn missing_cards_error(
    requested_count: usize,
    found_count: usize,
    missing_ids: Vec<String>,
) -> PlankaError {
    let noun = if missing_ids.len() == 1 {
        "card was"
    } else {
        "cards were"
    };
    PlankaError::BatchNotFound {
        message: format!("{} requested {noun} not found", missing_ids.len()),
        resource_type: "card".to_string(),
        missing_ids,
        requested_count,
        found_count,
    }
}

fn fatal_batch_error(requested_count: usize, failures: Vec<ErrorFailure>) -> PlankaError {
    if failures
        .iter()
        .any(|failure| failure.error_type == "AuthenticationFailed")
    {
        PlankaError::BatchAuthenticationFailed {
            message: "Batch card fetch failed".to_string(),
            requested_count,
            failures,
        }
    } else {
        PlankaError::BatchApiError {
            message: "Batch card fetch failed".to_string(),
            requested_count,
            failures,
        }
    }
}

#[allow(clippy::too_many_lines)]
pub async fn execute(
    client: &(impl CardApi + LabelApi + ListApi + UserApi),
    action: crate::app::CardAction,
    format: OutputFormat,
    yes: bool,
    full: bool,
) -> Result<(), PlankaError> {
    match action {
        crate::app::CardAction::List { list, board, label } => {
            let cards = if let Some(scope) = resolve_board_scope(
                client,
                list.as_deref(),
                board.as_deref(),
                &label,
                &[],
                &[],
                &[],
            )
            .await?
            {
                client
                    .list_cards_in_board(
                        &scope.board_id,
                        scope.list_id.as_deref(),
                        &scope.label_ids,
                        &[],
                        &[],
                        false,
                    )
                    .await?
            } else if let Some(list_id) = list {
                client.list_cards(&list_id).await?
            } else if let Some(board_id) = board {
                client
                    .list_cards_in_board(&board_id, None, &[], &[], &[], false)
                    .await?
            } else {
                return Err(PlankaError::MissingRequiredOption {
                    field: "--list or --board".to_string(),
                });
            };
            render_collection(&cards, format, full)?;
        }
        crate::app::CardAction::Get { id } => {
            let card = client.get_card(&id).await?;
            render_item(&card, format, full)?;
        }
        crate::app::CardAction::GetMany {
            id,
            concurrency,
            allow_missing,
        } => {
            let batch = client.get_many_cards(id, usize::from(concurrency)).await?;
            let found_count = batch.found_count();
            let failures = batch
                .failures
                .into_iter()
                .map(|failure| ErrorFailure {
                    id: failure.id,
                    error_type: failure.error_type,
                    message: failure.message,
                })
                .collect::<Vec<_>>();

            if !failures.is_empty() {
                return Err(fatal_batch_error(batch.requested_count, failures));
            }

            if !allow_missing && !batch.missing_ids.is_empty() {
                return Err(missing_cards_error(
                    batch.requested_count,
                    found_count,
                    batch.missing_ids,
                ));
            }

            if allow_missing
                && !batch.missing_ids.is_empty()
                && matches!(format, OutputFormat::Table | OutputFormat::Markdown)
            {
                eprintln!(
                    "Warning: {} requested card(s) were missing: {}",
                    batch.missing_ids.len(),
                    batch.missing_ids.join(", ")
                );
            }

            let meta = build_get_many_meta(
                found_count,
                batch.requested_count,
                batch.missing_ids,
                batch.concurrency,
                allow_missing,
            );
            render_collection_with_meta(&batch.cards, format, full, &meta)?;
        }
        crate::app::CardAction::Snapshot { id } => {
            let snapshot = client.get_card_snapshot(&id).await?;
            render_snapshot(&snapshot, format)?;
        }
        crate::app::CardAction::Find {
            list,
            board,
            project,
            title,
            label,
            member,
            assignee,
            assignee_or_member,
        } => {
            if title.is_none()
                && label.is_empty()
                && member.is_empty()
                && assignee.is_empty()
                && assignee_or_member.is_empty()
            {
                return Err(PlankaError::MissingRequiredOption {
                    field: "--title, --label, --member, --assignee, or --assignee-or-member"
                        .to_string(),
                });
            }

            let cards = if let Some(scope) = resolve_board_scope(
                client,
                list.as_deref(),
                board.as_deref(),
                &label,
                &member,
                &assignee,
                &assignee_or_member,
            )
            .await?
            {
                let scoped_cards = client
                    .list_cards_in_board(
                        &scope.board_id,
                        scope.list_id.as_deref(),
                        &scope.label_ids,
                        &scope.member_ids,
                        &scope.assignee_ids,
                        scope.either_user_role,
                    )
                    .await?;
                if let Some(title) = title.as_deref() {
                    match_by_name(&scoped_cards, title)
                        .into_iter()
                        .cloned()
                        .collect()
                } else {
                    scoped_cards
                }
            } else {
                let scope = if let Some(list_id) = list {
                    FindScope::List(list_id)
                } else if let Some(board_id) = board {
                    FindScope::Board(board_id)
                } else if let Some(project_id) = project {
                    FindScope::Project(project_id)
                } else {
                    return Err(PlankaError::MissingRequiredOption {
                        field: "--list, --board, or --project".to_string(),
                    });
                };
                client
                    .find_cards(scope, title.as_deref().unwrap_or(""))
                    .await?
            };
            render_collection(&cards, format, full)?;
        }
        crate::app::CardAction::Create {
            list,
            title,
            description,
            position,
        } => {
            let desc = match description {
                Some(raw) => Some(resolve_text(&raw)?),
                None => None,
            };
            let pos = match position {
                Some(p) => parse_position(&p)?,
                None => 65536.0,
            };
            let params = CreateCard {
                list_id: list.clone(),
                name: title,
                description: desc,
                card_type: "project".to_string(),
                position: pos,
            };
            let card = client.create_card(&list, params).await?;
            render_item(&card, format, full)?;
        }
        crate::app::CardAction::Update {
            id,
            title,
            description,
        } => {
            if title.is_none() && description.is_none() {
                return Err(PlankaError::InvalidOptionValue {
                    field: "--title / --description".to_string(),
                    message: "At least one field must be provided for update".to_string(),
                });
            }
            let desc = match description {
                Some(raw) => Some(resolve_text(&raw)?),
                None => None,
            };
            let params = UpdateCard {
                name: title,
                description: desc,
                due_date: None,
                is_closed: None,
            };
            let card = client.update_card(&id, params).await?;
            render_item(&card, format, full)?;
        }
        crate::app::CardAction::Move {
            id,
            to_list,
            to_board,
            position,
        } => {
            let pos = match position {
                Some(p) => parse_position(&p)?,
                None => 65536.0,
            };
            let params = MoveCard {
                board_id: to_board,
                list_id: to_list,
                position: pos,
            };
            let card = client.move_card(&id, params).await?;
            render_item(&card, format, full)?;
        }
        crate::app::CardAction::Archive { id } => {
            let card = client.archive_card(&id).await?;
            render_item(&card, format, full)?;
        }
        crate::app::CardAction::Unarchive { id } => {
            let card = client.unarchive_card(&id).await?;
            render_item(&card, format, full)?;
        }
        crate::app::CardAction::Delete { id } => {
            if !yes && !confirm_delete("card", &id) {
                render_message("Aborted.", format)?;
                return Ok(());
            }
            client.delete_card(&id).await?;
            render_message("Card deleted.", format)?;
        }
        // Label and Assignee subcommands are dispatched in main.rs
        crate::app::CardAction::Label(_)
        | crate::app::CardAction::Assignee(_)
        | crate::app::CardAction::Field(_) => {
            unreachable!("card label/assignee/field dispatched in main.rs")
        }
    }
    Ok(())
}
