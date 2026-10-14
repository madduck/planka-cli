//! ISO 8601 parsing and strict due-date range matching for `card find`.
//!
//! Planka stores `dueDate` as an ISO 8601 string and offers no server-side
//! filter by due date, so filtering is a client-side predicate applied after
//! the candidate set is fetched. Comparison is done on the underlying UTC
//! instant; comparing the raw strings would misorder date-only values,
//! fractional seconds, and non-UTC offsets.

use plnk_core::error::PlankaError;
use plnk_core::models::Card;
use time::OffsetDateTime;
use time::UtcOffset;
use time::format_description::well_known::Rfc3339;

/// A strictly-open due-date interval: `after < due < before`.
///
/// An unset bound is unbounded on that side. A range with both bounds unset
/// matches every card, including cards without a due date, so the predicate
/// can be applied unconditionally.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DueDateRange {
    after: Option<OffsetDateTime>,
    before: Option<OffsetDateTime>,
}

impl DueDateRange {
    /// Build a range from optional raw ISO 8601 bounds.
    ///
    /// `after_field` and `before_field` name the originating CLI flags so a
    /// parse failure maps to the right `InvalidOptionValue` field.
    pub fn parse(
        after: Option<&str>,
        after_field: &str,
        before: Option<&str>,
        before_field: &str,
    ) -> Result<Self, PlankaError> {
        Ok(Self {
            after: after.map(|raw| parse_bound(raw, after_field)).transpose()?,
            before: before
                .map(|raw| parse_bound(raw, before_field))
                .transpose()?,
        })
    }

    /// Keep only cards whose due date falls strictly inside the range.
    ///
    /// Cards without a due date, or with an unparseable one, never match a
    /// bounded range. An unbounded range keeps everything.
    #[must_use]
    pub fn filter(&self, mut cards: Vec<Card>) -> Vec<Card> {
        if self.after.is_none() && self.before.is_none() {
            return cards;
        }
        cards.retain(|card| self.matches(card));
        cards
    }

    fn matches(&self, card: &Card) -> bool {
        let Some(raw) = card.due_date.as_deref() else {
            return false;
        };
        let due = match parse_iso8601_utc(raw) {
            Ok(due) => due,
            Err(reason) => {
                eprintln!(
                    "Warning: card {} has an unparseable dueDate '{raw}': {reason}; \
                     excluded from due-date filtering",
                    card.id
                );
                return false;
            }
        };
        let due = due.unix_timestamp_nanos();
        self.after
            .is_none_or(|after| after.unix_timestamp_nanos() < due)
            && self
                .before
                .is_none_or(|before| due < before.unix_timestamp_nanos())
    }
}

fn parse_bound(raw: &str, field: &str) -> Result<OffsetDateTime, PlankaError> {
    parse_iso8601_utc(raw).map_err(|reason| PlankaError::InvalidOptionValue {
        field: field.to_string(),
        message: format!(
            "'{raw}' is not a valid ISO 8601 timestamp (expected YYYY-MM-DD or \
             YYYY-MM-DDThh:mm[:ss[.fff]][Z|±hh:mm]): {reason}"
        ),
    })
}

/// Parse an ISO 8601 timestamp to a UTC instant.
///
/// Accepts a bare date (`YYYY-MM-DD`, interpreted as midnight UTC) and
/// RFC 3339 date-times. A date-time without an offset is treated as UTC.
pub fn parse_iso8601_utc(raw: &str) -> Result<OffsetDateTime, String> {
    // RFC 3339 requires a `T` between date and time, so a bare date cannot be
    // parsed directly. Normalise it to midnight UTC first.
    let candidate = if raw.len() == 10 && !raw.contains(['T', 't', ' ']) {
        format!("{raw}T00:00:00Z")
    } else {
        raw.to_string()
    };

    OffsetDateTime::parse(&candidate, &Rfc3339)
        .or_else(|_| OffsetDateTime::parse(&format!("{candidate}Z"), &Rfc3339))
        .map_err(|error| error.to_string())
}

/// Parse a due-date input and normalise it to RFC 3339 in UTC.
///
/// Used by `card update --due-date` so the value sent to Planka is
/// deterministic regardless of the offset the caller supplied.
pub fn normalize_due_date(raw: &str, field: &str) -> Result<String, PlankaError> {
    let parsed = parse_bound(raw, field)?;
    parsed
        .to_offset(UtcOffset::UTC)
        .format(&Rfc3339)
        .map_err(|error| PlankaError::InvalidOptionValue {
            field: field.to_string(),
            message: error.to_string(),
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card(due: Option<&str>) -> Card {
        Card {
            id: "card-1".to_string(),
            list_id: "list-1".to_string(),
            board_id: "board-1".to_string(),
            name: "Card".to_string(),
            description: None,
            position: 65_536.0,
            due_date: due.map(std::string::ToString::to_string),
            is_due_completed: None,
            is_closed: false,
            is_subscribed: false,
            creator_user_id: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: None,
        }
    }

    fn nanos(raw: &str) -> i128 {
        parse_iso8601_utc(raw).unwrap().unix_timestamp_nanos()
    }

    #[test]
    fn date_only_is_midnight_utc() {
        assert_eq!(nanos("2026-04-20"), nanos("2026-04-20T00:00:00Z"));
    }

    #[test]
    fn missing_offset_is_utc() {
        assert_eq!(nanos("2026-04-20T12:30:00"), nanos("2026-04-20T12:30:00Z"));
    }

    #[test]
    fn explicit_offset_is_respected() {
        assert_eq!(
            nanos("2026-04-20T02:30:00+02:00"),
            nanos("2026-04-20T00:30:00Z")
        );
    }

    #[test]
    fn fractional_seconds_are_respected() {
        assert_eq!(
            nanos("2026-04-20T00:00:00.500Z"),
            nanos("2026-04-20T00:00:00Z") + 500_000_000
        );
    }

    #[test]
    fn lowercase_rfc3339_markers_are_accepted() {
        assert_eq!(nanos("2026-04-20t00:00:00z"), nanos("2026-04-20T00:00:00Z"));
    }

    #[test]
    fn invalid_timestamp_is_rejected() {
        assert!(parse_iso8601_utc("not-a-date").is_err());
        assert!(parse_iso8601_utc("").is_err());
        assert!(parse_iso8601_utc("2026-13-40").is_err());
    }

    #[test]
    fn unbounded_range_keeps_null_due_dates() {
        let range = DueDateRange::parse(None, "--due-after", None, "--due-before").unwrap();
        let cards = range.filter(vec![card(None), card(Some("2026-04-20T00:00:00Z"))]);
        assert_eq!(cards.len(), 2);
    }

    #[test]
    fn bounded_range_drops_null_due_dates() {
        let range =
            DueDateRange::parse(Some("2026-01-01"), "--due-after", None, "--due-before").unwrap();
        let cards = range.filter(vec![card(None), card(Some("2026-04-20T00:00:00Z"))]);
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0].id, "card-1");
    }

    #[test]
    fn after_bound_is_strict() {
        let range = DueDateRange::parse(
            Some("2026-04-20T00:00:00Z"),
            "--due-after",
            None,
            "--due-before",
        )
        .unwrap();
        let cards = range.filter(vec![
            card(Some("2026-04-19T23:59:59Z")),
            card(Some("2026-04-20T00:00:00Z")),
            card(Some("2026-04-20T00:00:01Z")),
        ]);
        assert_eq!(
            cards
                .iter()
                .map(|c| c.due_date.as_deref().unwrap())
                .collect::<Vec<_>>(),
            vec!["2026-04-20T00:00:01Z"]
        );
    }

    #[test]
    fn before_bound_is_strict() {
        let range = DueDateRange::parse(
            None,
            "--due-after",
            Some("2026-04-20T00:00:00Z"),
            "--due-before",
        )
        .unwrap();
        let cards = range.filter(vec![
            card(Some("2026-04-19T23:59:59Z")),
            card(Some("2026-04-20T00:00:00Z")),
            card(Some("2026-04-20T00:00:01Z")),
        ]);
        assert_eq!(
            cards
                .iter()
                .map(|c| c.due_date.as_deref().unwrap())
                .collect::<Vec<_>>(),
            vec!["2026-04-19T23:59:59Z"]
        );
    }

    #[test]
    fn both_bounds_combine() {
        let range = DueDateRange::parse(
            Some("2026-04-20"),
            "--due-after",
            Some("2026-04-22"),
            "--due-before",
        )
        .unwrap();
        let cards = range.filter(vec![
            card(Some("2026-04-20T00:00:00Z")),
            card(Some("2026-04-21T12:00:00Z")),
            card(Some("2026-04-22T00:00:00Z")),
            card(None),
        ]);
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0].due_date.as_deref(), Some("2026-04-21T12:00:00Z"));
    }

    #[test]
    fn unparseable_due_date_is_excluded() {
        let range =
            DueDateRange::parse(Some("2026-01-01"), "--due-after", None, "--due-before").unwrap();
        let cards = range.filter(vec![
            card(Some("2026-04-20T00:00:00Z")),
            card(Some("not-a-timestamp")),
        ]);
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0].due_date.as_deref(), Some("2026-04-20T00:00:00Z"));
    }

    #[test]
    fn normalize_due_date_uses_utc() {
        assert_eq!(
            normalize_due_date("2026-04-20", "--due-date").unwrap(),
            "2026-04-20T00:00:00Z"
        );
        assert_eq!(
            normalize_due_date("2026-04-20T02:30:00+02:00", "--due-date").unwrap(),
            "2026-04-20T00:30:00Z"
        );
    }

    #[test]
    fn normalize_due_date_rejects_invalid() {
        let error = normalize_due_date("nope", "--due-date").unwrap_err();
        assert_eq!(error.error_type(), "InvalidOptionValue");
        assert!(error.to_string().contains("--due-date"));
    }

    #[test]
    fn parse_error_names_the_flag() {
        let error =
            DueDateRange::parse(Some("nope"), "--due-after", None, "--due-before").unwrap_err();
        assert_eq!(error.error_type(), "InvalidOptionValue");
        assert!(error.to_string().contains("--due-after"));
        assert_eq!(error.exit_code(), 2);
    }
}
