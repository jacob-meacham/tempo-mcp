use std::collections::HashMap;

use chrono::{DateTime, Utc};
use rmcp::{ErrorData as McpError, model::*};
use serde::Serialize;

use crate::calendar::event::{Event, EventId, RecurrenceRule};
use crate::calendar::proposal::ProposedEvent;
use crate::error::TempoError;
use super::types::{GCalEvent, JsonEventInput};

pub(crate) fn parse_datetime(s: &str) -> Result<DateTime<Utc>, TempoError> {
    // Try RFC 3339 first (with timezone offset)
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Ok(dt.with_timezone(&Utc));
    }
    // Try without timezone (assume UTC)
    if let Ok(naive) = chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S") {
        return Ok(naive.and_utc());
    }
    Err(TempoError::InvalidTimeRange(format!(
        "Cannot parse datetime: '{}'. Use ISO 8601 format.",
        s
    )))
}

/// Shared parsing logic for JsonEventInput fields.
struct ParsedEventInput {
    title: String,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    timezone: String,
    recurrence: Option<RecurrenceRule>,
    metadata: HashMap<String, String>,
}

fn parse_json_event_input(input: &JsonEventInput) -> Result<ParsedEventInput, TempoError> {
    let start = parse_datetime(&input.start)?;
    let end = parse_datetime(&input.end)?;
    if end <= start {
        return Err(TempoError::InvalidTimeRange(
            "End time must be after start time".to_string(),
        ));
    }
    Ok(ParsedEventInput {
        title: input.title.clone(),
        start,
        end,
        timezone: input.timezone.clone().unwrap_or_else(|| "UTC".to_string()),
        recurrence: input.rrule.as_ref().map(|r| RecurrenceRule {
            rrule: r.clone(),
        }),
        metadata: input.metadata.clone().unwrap_or_default(),
    })
}

pub(crate) fn json_event_to_proposed(
    input: &JsonEventInput,
) -> Result<ProposedEvent, TempoError> {
    let parsed = parse_json_event_input(input)?;
    Ok(ProposedEvent {
        title: parsed.title,
        start: parsed.start,
        end: parsed.end,
        timezone: parsed.timezone,
        recurrence: parsed.recurrence,
        metadata: parsed.metadata,
    })
}

pub(crate) fn json_event_to_event(
    input: &JsonEventInput,
) -> Result<Event, TempoError> {
    let parsed = parse_json_event_input(input)?;
    Ok(Event {
        id: EventId::new(),
        title: parsed.title,
        start: parsed.start,
        end: parsed.end,
        timezone: parsed.timezone,
        recurrence: parsed.recurrence,
        metadata: parsed.metadata,
    })
}

pub(crate) fn gcal_event_to_event(input: &GCalEvent) -> Result<Event, TempoError> {
    let start_str = input
        .start
        .date_time
        .as_deref()
        .or(input.start.date.as_deref())
        .ok_or_else(|| TempoError::InvalidInput("Missing start dateTime".to_string()))?;
    let end_str = input
        .end
        .date_time
        .as_deref()
        .or(input.end.date.as_deref())
        .ok_or_else(|| TempoError::InvalidInput("Missing end dateTime".to_string()))?;

    let start = parse_datetime(start_str)?;
    let end = parse_datetime(end_str)?;
    if end <= start {
        return Err(TempoError::InvalidTimeRange(
            "End time must be after start time".to_string(),
        ));
    }

    let title = input
        .summary
        .clone()
        .unwrap_or_else(|| "Busy".to_string());
    let timezone = input
        .start
        .time_zone
        .clone()
        .unwrap_or_else(|| "UTC".to_string());

    let mut metadata = HashMap::new();
    if let Some(ref id) = input.id {
        metadata.insert("google_calendar_id".to_string(), id.clone());
    }
    if let Some(ref desc) = input.description {
        metadata.insert("description".to_string(), desc.clone());
    }
    if let Some(ref loc) = input.location {
        metadata.insert("location".to_string(), loc.clone());
    }

    Ok(Event {
        id: EventId::new(),
        title,
        start,
        end,
        timezone,
        recurrence: None,
        metadata,
    })
}

pub(crate) fn tempo_err(e: TempoError) -> McpError {
    let code = match &e {
        TempoError::CalendarNotFound(_)
        | TempoError::EventNotFound(_)
        | TempoError::ProposalNotFound(_) => ErrorCode::RESOURCE_NOT_FOUND,
        _ => ErrorCode::INVALID_PARAMS,
    };
    McpError::new(code, e.to_string(), None::<serde_json::Value>)
}

pub(crate) fn json_text<T: Serialize>(value: &T) -> CallToolResult {
    let json = serde_json::to_string_pretty(value)
        .unwrap_or_else(|e| serde_json::json!({"error": e.to_string()}).to_string());
    CallToolResult::success(vec![Content::text(json)])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_datetime_rfc3339_utc() {
        let dt = parse_datetime("2025-01-15T09:00:00Z").unwrap();
        assert_eq!(dt.to_rfc3339(), "2025-01-15T09:00:00+00:00");
    }

    #[test]
    fn parse_datetime_rfc3339_with_offset() {
        let dt = parse_datetime("2025-01-15T09:00:00-05:00").unwrap();
        assert_eq!(dt.to_rfc3339(), "2025-01-15T14:00:00+00:00");
    }

    #[test]
    fn parse_datetime_naive_assumes_utc() {
        let dt = parse_datetime("2025-01-15T09:00:00").unwrap();
        assert_eq!(dt.to_rfc3339(), "2025-01-15T09:00:00+00:00");
    }

    #[test]
    fn parse_datetime_invalid_returns_error() {
        let result = parse_datetime("not-a-date");
        assert!(result.is_err());
    }

    #[test]
    fn json_event_to_event_basic() {
        let input = JsonEventInput {
            title: "Test".to_string(),
            start: "2025-01-15T09:00:00Z".to_string(),
            end: "2025-01-15T10:00:00Z".to_string(),
            timezone: Some("America/New_York".to_string()),
            rrule: None,
            metadata: None,
        };
        let event = json_event_to_event(&input).unwrap();
        assert_eq!(event.title, "Test");
        assert_eq!(event.timezone, "America/New_York");
    }

    #[test]
    fn json_event_to_event_end_before_start_is_error() {
        let input = JsonEventInput {
            title: "Bad".to_string(),
            start: "2025-01-15T10:00:00Z".to_string(),
            end: "2025-01-15T09:00:00Z".to_string(),
            timezone: None,
            rrule: None,
            metadata: None,
        };
        assert!(json_event_to_event(&input).is_err());
    }

    #[test]
    fn json_event_to_proposed_basic() {
        let input = JsonEventInput {
            title: "Proposed".to_string(),
            start: "2025-01-15T09:00:00Z".to_string(),
            end: "2025-01-15T10:00:00Z".to_string(),
            timezone: None,
            rrule: Some("FREQ=DAILY;COUNT=3".to_string()),
            metadata: None,
        };
        let proposed = json_event_to_proposed(&input).unwrap();
        assert_eq!(proposed.title, "Proposed");
        assert_eq!(proposed.timezone, "UTC");
        assert!(proposed.recurrence.is_some());
    }

    #[test]
    fn gcal_event_to_event_with_datetime() {
        let input = GCalEvent {
            id: Some("gcal123".to_string()),
            summary: Some("GCal Meeting".to_string()),
            start: super::super::types::GCalDateTime {
                date_time: Some("2025-01-15T09:00:00-05:00".to_string()),
                date: None,
                time_zone: Some("America/New_York".to_string()),
            },
            end: super::super::types::GCalDateTime {
                date_time: Some("2025-01-15T10:00:00-05:00".to_string()),
                date: None,
                time_zone: None,
            },
            description: Some("A meeting".to_string()),
            location: Some("Room 101".to_string()),
            status: None,
        };
        let event = gcal_event_to_event(&input).unwrap();
        assert_eq!(event.title, "GCal Meeting");
        assert_eq!(event.timezone, "America/New_York");
        assert_eq!(event.metadata.get("google_calendar_id").unwrap(), "gcal123");
        assert_eq!(event.metadata.get("description").unwrap(), "A meeting");
        assert_eq!(event.metadata.get("location").unwrap(), "Room 101");
    }

    #[test]
    fn gcal_event_to_event_missing_start_is_error() {
        let input = GCalEvent {
            id: None,
            summary: None,
            start: super::super::types::GCalDateTime {
                date_time: None,
                date: None,
                time_zone: None,
            },
            end: super::super::types::GCalDateTime {
                date_time: Some("2025-01-15T10:00:00Z".to_string()),
                date: None,
                time_zone: None,
            },
            description: None,
            location: None,
            status: None,
        };
        assert!(gcal_event_to_event(&input).is_err());
    }

    #[test]
    fn tempo_err_maps_not_found_to_resource_not_found() {
        let err = tempo_err(TempoError::CalendarNotFound("test".to_string()));
        assert_eq!(err.code, ErrorCode::RESOURCE_NOT_FOUND);
    }

    #[test]
    fn tempo_err_maps_event_not_found_to_resource_not_found() {
        let err = tempo_err(TempoError::EventNotFound("test".to_string()));
        assert_eq!(err.code, ErrorCode::RESOURCE_NOT_FOUND);
    }

    #[test]
    fn tempo_err_maps_invalid_input_to_invalid_params() {
        let err = tempo_err(TempoError::InvalidInput("bad".to_string()));
        assert_eq!(err.code, ErrorCode::INVALID_PARAMS);
    }
}
