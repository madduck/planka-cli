use plnk_core::api::{AssigneeApi, CardApi, match_by_name};
use plnk_core::error::PlankaError;
use plnk_core::models::{
    Card, CardWithAssignees, CreateCard, ErrorFailure, FindScope, Label, MoveCard, UpdateCard,
};
use serde::Serialize;

use crate::app::OutputFormat;
use crate::commands::project::confirm_delete;
use crate::due::{DueDateRange, LocalClock, parse_due_date_input};
use crate::input::resolve_text;
use crate::names::{NameIndex, NameSource};
use crate::output::{
    render_collection_named, render_collection_with_meta_named, render_item_named, render_message,
    render_snapshot,
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

/// Keep only closed cards when `closed` is set, otherwise pass through.
///
/// This is the explicit `--closed` filter. It is separate from the implicit
/// closed-card exclusion inside [`DueDateRange::filter`], which a due query
/// applies unless `--include-closed` is given.
fn filter_closed(cards: Vec<Card>, closed: bool) -> Vec<Card> {
    if closed {
        cards.into_iter().filter(|card| card.is_closed).collect()
    } else {
        cards
    }
}

/// Attach assignees to each card by fetching its memberships.
///
/// This is the fallback enrichment path, used only when no response in hand
/// already carries the memberships (mutation responses). It costs one
/// `GET /api/cards/{id}` per card.
async fn enrich_with_members(
    client: &impl AssigneeApi,
    cards: Vec<Card>,
) -> Result<Vec<CardWithAssignees>, PlankaError> {
    let mut enriched = Vec::with_capacity(cards.len());
    for card in cards {
        let assignees = client.list_assignees(&card.id).await?;
        enriched.push(CardWithAssignees::new(card, assignees));
    }
    Ok(enriched)
}

/// Keep only closed rows when `closed` is set, otherwise pass through.
fn filter_closed_rows(rows: Vec<CardWithAssignees>, closed: bool) -> Vec<CardWithAssignees> {
    if closed {
        rows.into_iter().filter(|row| row.card.is_closed).collect()
    } else {
        rows
    }
}

/// Render one card, pairing it with its assignees when `members` is set.
async fn render_card_item(
    client: &(impl AssigneeApi + NameSource),
    card: Card,
    members: bool,
    format: OutputFormat,
    full: bool,
    names: &mut NameIndex,
) -> Result<(), PlankaError> {
    if members {
        let assignees = client.list_assignees(&card.id).await?;
        let row = CardWithAssignees::new(card, assignees);
        render_item_named(client, &row, format, full, names).await
    } else {
        render_item_named(client, &card, format, full, names).await
    }
}

/// Render card rows that already carry their assignees.
async fn render_member_rows<C>(
    client: &C,
    rows: &[CardWithAssignees],
    format: OutputFormat,
    full: bool,
    names: &mut NameIndex,
) -> Result<(), PlankaError>
where
    C: NameSource + ?Sized,
{
    render_collection_named(client, rows, format, full, names).await
}

/// Narrow member rows to those whose card matches `title`.
///
/// The house three-tier match runs over the cards; the matched IDs then filter
/// the rows so their assignees survive.
fn match_member_rows_by_name(rows: Vec<CardWithAssignees>, title: &str) -> Vec<CardWithAssignees> {
    let cards: Vec<Card> = rows.iter().map(|row| row.card.clone()).collect();
    let matched: std::collections::HashSet<&str> = match_by_name(&cards, title)
        .into_iter()
        .map(|card| card.id.as_str())
        .collect();
    rows.into_iter()
        .filter(|row| matched.contains(row.card.id.as_str()))
        .collect()
}

/// Parse position flag: "top", "bottom", or numeric value.
pub(crate) fn parse_position(pos: &str) -> Result<f64, PlankaError> {
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

async fn resolve_label_scope(
    client: &impl NameSource,
    list: Option<&str>,
    board: Option<&str>,
    labels: &[String],
) -> Result<Option<(String, Option<String>, Vec<String>)>, PlankaError> {
    if labels.is_empty() {
        return Ok(None);
    }

    let (board_id, list_id) = if let Some(list_id) = list {
        let list_item = client.get_list(list_id).await?;
        (list_item.board_id, Some(list_id.to_string()))
    } else if let Some(board_id) = board {
        (board_id.to_string(), None)
    } else {
        return Err(PlankaError::InvalidOptionValue {
            field: "--label".to_string(),
            message: "--label is only supported with --list or --board scopes because labels are board-scoped".to_string(),
        });
    };

    let board_labels = client.list_labels(&board_id).await?;
    let resolved = labels
        .iter()
        .map(|query| resolve_label_id(&board_labels, query))
        .collect::<Result<Vec<_>, _>>()?;

    Ok(Some((board_id, list_id, resolved)))
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
    client: &(impl CardApi + AssigneeApi + NameSource),
    action: crate::app::CardAction,
    format: OutputFormat,
    yes: bool,
    full: bool,
    names: &mut NameIndex,
) -> Result<(), PlankaError> {
    match action {
        crate::app::CardAction::List {
            list,
            board,
            label,
            due_after,
            due_before,
            include_closed,
            closed,
            members,
        } => {
            let board = names.resolve_optional_board(client, board, None).await?;
            let list = names
                .resolve_optional_list(client, list, board.as_deref(), None)
                .await?;
            let due_range = DueDateRange::parse(
                due_after.as_deref(),
                "--due-after",
                due_before.as_deref(),
                "--due-before",
                LocalClock::system(),
            )?;
            let board = names.resolve_optional_board(client, board, None).await?;
            let list = names
                .resolve_optional_list(client, list, board.as_deref(), None)
                .await?;
            let label_scope =
                resolve_label_scope(client, list.as_deref(), board.as_deref(), &label).await?;
            // `--closed` is an explicit filter, independent of the due window.
            // Apply it first, then tell the due filter to keep closed cards:
            // the caller asked for them, so the implicit planning exclusion
            // must not drop them again.
            if members {
                // Every scope here reads memberships from the same response
                // that supplies the cards, so `--members` costs nothing extra.
                let rows = if let Some((board_id, list_id, label_ids)) = &label_scope {
                    client
                        .list_cards_in_board_with_assignees(board_id, list_id.as_deref(), label_ids)
                        .await?
                } else if let Some(list_id) = &list {
                    client.list_cards_with_assignees(list_id).await?
                } else if let Some(board_id) = &board {
                    client
                        .list_cards_in_board_with_assignees(board_id, None, &[])
                        .await?
                } else {
                    return Err(PlankaError::MissingRequiredOption {
                        field: "--list or --board".to_string(),
                    });
                };
                let rows = filter_closed_rows(rows, closed);
                let rows =
                    due_range.filter_by_card(rows, |row| &row.card, include_closed || closed);
                render_member_rows(client, &rows, format, full, names).await?;
            } else {
                let cards = if let Some((board_id, list_id, label_ids)) = label_scope {
                    client
                        .list_cards_in_board(&board_id, list_id.as_deref(), &label_ids)
                        .await?
                } else if let Some(list_id) = list {
                    client.list_cards(&list_id).await?
                } else if let Some(board_id) = board {
                    client.list_cards_in_board(&board_id, None, &[]).await?
                } else {
                    return Err(PlankaError::MissingRequiredOption {
                        field: "--list or --board".to_string(),
                    });
                };
                let cards = filter_closed(cards, closed);
                let cards = due_range.filter(cards, include_closed || closed);
                render_collection_named(client, &cards, format, full, names).await?;
            }
        }
        crate::app::CardAction::Get { id, members } => {
            if members {
                let row = client.get_card_with_assignees(&id).await?;
                render_item_named(client, &row, format, full, names).await?;
            } else {
                let card = client.get_card(&id).await?;
                render_item_named(client, &card, format, full, names).await?;
            }
        }
        crate::app::CardAction::GetMany {
            id,
            concurrency,
            allow_missing,
            members,
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
            if members {
                let enriched = enrich_with_members(client, batch.cards).await?;
                render_collection_with_meta_named(client, &enriched, format, full, &meta, names)
                    .await?;
            } else {
                render_collection_with_meta_named(client, &batch.cards, format, full, &meta, names)
                    .await?;
            }
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
            due_after,
            due_before,
            include_closed,
            closed,
            members,
        } => {
            if title.is_none()
                && label.is_empty()
                && due_after.is_none()
                && due_before.is_none()
                && !closed
            {
                return Err(PlankaError::MissingRequiredOption {
                    field: "--title, --label, --due-after, --due-before, or --closed".to_string(),
                });
            }

            let project = names.resolve_optional_project(client, project).await?;
            let board = names
                .resolve_optional_board(client, board, project.as_deref())
                .await?;
            let list = names
                .resolve_optional_list(client, list, board.as_deref(), project.as_deref())
                .await?;
            let due_range = DueDateRange::parse(
                due_after.as_deref(),
                "--due-after",
                due_before.as_deref(),
                "--due-before",
                LocalClock::system(),
            )?;

            let label_scope =
                resolve_label_scope(client, list.as_deref(), board.as_deref(), &label).await?;
            if members {
                // Memberships come from the same snapshot that supplies the
                // cards, so `--members` costs nothing extra here either.
                let mut rows = if let Some((board_id, list_id, label_ids)) = label_scope {
                    client
                        .list_cards_in_board_with_assignees(
                            &board_id,
                            list_id.as_deref(),
                            &label_ids,
                        )
                        .await?
                } else {
                    match (list.clone(), board.clone(), project.clone()) {
                        (Some(list_id), _, _) => client.list_cards_with_assignees(&list_id).await?,
                        (_, Some(board_id), _) => {
                            client
                                .list_cards_in_board_with_assignees(&board_id, None, &[])
                                .await?
                        }
                        (_, _, Some(project_id)) => {
                            let mut all = Vec::new();
                            for board in client.list_boards(&project_id).await? {
                                all.extend(
                                    client
                                        .list_cards_in_board_with_assignees(&board.id, None, &[])
                                        .await?,
                                );
                            }
                            all
                        }
                        _ => {
                            return Err(PlankaError::MissingRequiredOption {
                                field: "--list, --board, or --project".to_string(),
                            });
                        }
                    }
                };
                if let Some(title) = title.as_deref() {
                    rows = match_member_rows_by_name(rows, title);
                }
                // `--closed` is an explicit filter, independent of the due
                // window. Apply it first, then tell the due filter to keep
                // closed cards: the caller asked for them, so the implicit
                // planning exclusion must not drop them again.
                let rows = filter_closed_rows(rows, closed);
                let rows =
                    due_range.filter_by_card(rows, |row| &row.card, include_closed || closed);
                render_member_rows(client, &rows, format, full, names).await?;
            } else {
                let cards = if let Some((board_id, list_id, label_ids)) = label_scope {
                    let scoped_cards = client
                        .list_cards_in_board(&board_id, list_id.as_deref(), &label_ids)
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
                // `--closed` is an explicit filter, independent of the due
                // window. Apply it first, then tell the due filter to keep
                // closed cards: the caller asked for them, so the implicit
                // planning exclusion must not drop them again.
                let cards = filter_closed(cards, closed);
                let cards = due_range.filter(cards, include_closed || closed);
                render_collection_named(client, &cards, format, full, names).await?;
            }
        }
        crate::app::CardAction::Create {
            list,
            title,
            description,
            position,
            members,
        } => {
            let list = names.resolve_list(client, &list, None, None).await?;
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
            render_card_item(client, card, members, format, full, names).await?;
        }
        crate::app::CardAction::Update {
            id,
            title,
            description,
            due_date,
            clear_due_date,
            members,
        } => {
            if title.is_none() && description.is_none() && due_date.is_none() && !clear_due_date {
                return Err(PlankaError::InvalidOptionValue {
                    field: "--title / --description / --due-date / --clear-due-date".to_string(),
                    message: "At least one field must be provided for update".to_string(),
                });
            }
            let desc = match description {
                Some(raw) => Some(resolve_text(&raw)?),
                None => None,
            };
            let due_date = parse_due_date_input(
                due_date.as_deref(),
                clear_due_date,
                "--due-date",
                LocalClock::system(),
            )?;
            let params = UpdateCard {
                name: title,
                description: desc,
                due_date,
                is_closed: None,
            };
            let card = client.update_card(&id, params).await?;
            render_card_item(client, card, members, format, full, names).await?;
        }
        crate::app::CardAction::Move {
            id,
            to_list,
            to_board,
            position,
            members,
        } => {
            let to_board = names.resolve_optional_board(client, to_board, None).await?;
            let to_list = names
                .resolve_list(client, &to_list, None, to_board.as_deref())
                .await?;
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
            render_card_item(client, card, members, format, full, names).await?;
        }
        crate::app::CardAction::Archive { id, members } => {
            let card = client.archive_card(&id).await?;
            render_card_item(client, card, members, format, full, names).await?;
        }
        crate::app::CardAction::Unarchive { id, members } => {
            let card = client.unarchive_card(&id).await?;
            render_card_item(client, card, members, format, full, names).await?;
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
