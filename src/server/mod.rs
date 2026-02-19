mod conversions;
mod types;

pub(crate) use conversions::*;
pub(crate) use types::*;

use std::collections::HashMap;
use std::sync::Arc;

use chrono::TimeDelta;
use rmcp::{
    ErrorData as McpError, ServerHandler,
    handler::server::wrapper::Parameters,
    model::*,
    tool, tool_router,
};
use serde::Serialize;
use tokio::sync::RwLock;

use crate::calendar::CalendarStore;
use crate::calendar::event::EventId;
use crate::calendar::proposal::ProposalId;
use crate::error::TempoError;
use crate::ical_bridge;

#[derive(Clone)]
pub struct TempoServer {
    store: Arc<RwLock<CalendarStore>>,
}

impl ServerHandler for TempoServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            protocol_version: ProtocolVersion::V_2024_11_05,
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            server_info: Implementation {
                name: "tempo".into(),
                version: env!("CARGO_PKG_VERSION").into(),
                title: None,
                icons: None,
                website_url: None,
            },
            instructions: Some(
                "Tempo is a lightweight in-memory calendar server for scheduling workflows. \
                 Recommended workflow (3 steps): \
                 1) Load all calendars with load_ical/load_json/load_google_calendar (you can make multiple load calls in parallel), \
                 2) Use find_available_slots with buffer_minutes to find open windows that already account for travel time, \
                 3) Use propose_and_commit to propose, conflict-check, and commit in one step. \
                 If propose_and_commit reports conflicts, adjust times and retry. \
                 Use the EXACT start/end times returned by find_available_slots — do not invent your own times."
                    .into(),
            ),
        }
    }
}

// -- Tool implementations --

#[tool_router]
impl TempoServer {
    pub fn new() -> Self {
        Self {
            store: Arc::new(RwLock::new(CalendarStore::new())),
        }
    }

    // === Hydration ===

    #[tool(description = "Load events from iCal/ICS format into a calendar. Parses VEVENT components including RRULE recurrence rules.")]
    async fn load_ical(
        &self,
        params: Parameters<LoadIcalParams>,
    ) -> Result<CallToolResult, McpError> {
        let cal_name = params.0.calendar_name.as_deref().unwrap_or("default");
        let events = ical_bridge::parse_ical(&params.0.ical_data).map_err(tempo_err)?;
        let count = events.len();

        let mut store = self.store.write().await;
        let cal = store.get_or_create_calendar(cal_name);
        for event in events {
            cal.add_event(event);
        }

        Ok(json_text(&serde_json::json!({
            "calendar_name": cal_name,
            "events_loaded": count,
        })))
    }

    #[tool(description = "Load events from a JSON array into a calendar. Each event needs title, start (ISO 8601), end (ISO 8601). Optional: timezone, rrule, metadata.")]
    async fn load_json(
        &self,
        params: Parameters<LoadJsonParams>,
    ) -> Result<CallToolResult, McpError> {
        let cal_name = params.0.calendar_name.as_deref().unwrap_or("default");

        let mut parsed_events = Vec::new();
        for input in &params.0.events {
            parsed_events.push(json_event_to_event(input).map_err(tempo_err)?);
        }

        let mut store = self.store.write().await;
        let cal = store.get_or_create_calendar(cal_name);
        let mut ids = Vec::new();
        for event in parsed_events {
            ids.push(event.id.to_string());
            cal.add_event(event);
        }

        Ok(json_text(&serde_json::json!({
            "calendar_name": cal_name,
            "events_loaded": ids.len(),
            "event_ids": ids,
        })))
    }

    #[tool(description = "Load events from Google Calendar API JSON format. Accepts the events array as returned by Google Calendar's events.list API. Handles nested start/end objects with dateTime, timeZone fields, and timezone offset conversion.")]
    async fn load_google_calendar(
        &self,
        params: Parameters<LoadGoogleCalendarParams>,
    ) -> Result<CallToolResult, McpError> {
        let cal_name = params.0.calendar_name.as_deref().unwrap_or("default");

        let mut parsed_events = Vec::new();
        let mut skipped = 0;
        for input in &params.0.events {
            match gcal_event_to_event(input) {
                Ok(event) => parsed_events.push(event),
                Err(e) => {
                    tracing::warn!("Skipping Google Calendar event: {e}");
                    skipped += 1;
                }
            }
        }

        let mut store = self.store.write().await;
        let cal = store.get_or_create_calendar(cal_name);
        let mut ids = Vec::new();
        for event in parsed_events {
            ids.push(event.id.to_string());
            cal.add_event(event);
        }

        Ok(json_text(&serde_json::json!({
            "calendar_name": cal_name,
            "events_loaded": ids.len(),
            "events_skipped": skipped,
            "event_ids": ids,
        })))
    }

    // === Querying ===

    #[tool(description = "List all event occurrences within a time range. Expands recurring events into individual occurrences. Returns events sorted by start time.")]
    async fn list_events(
        &self,
        params: Parameters<ListEventsParams>,
    ) -> Result<CallToolResult, McpError> {
        let start = parse_datetime(&params.0.start).map_err(tempo_err)?;
        let end = parse_datetime(&params.0.end).map_err(tempo_err)?;

        let store = self.store.read().await;
        let occs = store
            .occurrences_in_range(start, end, params.0.calendar_name.as_deref())
            .map_err(tempo_err)?;

        Ok(json_text(&occs))
    }

    #[tool(description = "Get free/busy analysis for a time range. Returns busy periods (with event titles), free periods, and total minutes for each.")]
    async fn get_free_busy(
        &self,
        params: Parameters<GetFreeBusyParams>,
    ) -> Result<CallToolResult, McpError> {
        let start = parse_datetime(&params.0.start).map_err(tempo_err)?;
        let end = parse_datetime(&params.0.end).map_err(tempo_err)?;

        let store = self.store.read().await;
        let result = store
            .free_busy(start, end, params.0.calendar_name.as_deref())
            .map_err(tempo_err)?;

        Ok(json_text(&result))
    }

    #[tool(description = "Find available time slots of at least the specified duration within a time range. Returns slots sorted by start time. Use buffer_minutes to account for travel time between events.")]
    async fn find_available_slots(
        &self,
        params: Parameters<FindAvailableSlotsParams>,
    ) -> Result<CallToolResult, McpError> {
        let start = parse_datetime(&params.0.start).map_err(tempo_err)?;
        let end = parse_datetime(&params.0.end).map_err(tempo_err)?;
        let buffer = TimeDelta::minutes(params.0.buffer_minutes.unwrap_or(0) as i64);
        let effective_duration = TimeDelta::minutes(params.0.duration_minutes as i64) + buffer + buffer;

        let store = self.store.read().await;
        let raw_slots = store
            .find_available_slots(start, end, effective_duration, params.0.calendar_name.as_deref())
            .map_err(tempo_err)?;

        // Shrink each slot by buffer on both sides so the caller can use times directly
        let slots: Vec<_> = if buffer > TimeDelta::zero() {
            raw_slots
                .into_iter()
                .filter_map(|s| {
                    let shrunk_start = s.start + buffer;
                    let shrunk_end = s.end - buffer;
                    if shrunk_end > shrunk_start {
                        Some(crate::calendar::time_utils::TimeRange::new(shrunk_start, shrunk_end))
                    } else {
                        None
                    }
                })
                .collect()
        } else {
            raw_slots
        };

        Ok(json_text(&slots))
    }

    // === Proposals ===

    #[tool(description = "Create a proposal with one or more events WITHOUT committing them. Use check_conflicts to verify before committing.")]
    async fn propose_events(
        &self,
        params: Parameters<ProposeEventsParams>,
    ) -> Result<CallToolResult, McpError> {
        let mut proposed = Vec::new();
        for input in &params.0.events {
            proposed.push(json_event_to_proposed(input).map_err(tempo_err)?);
        }

        let mut store = self.store.write().await;
        let proposal_id = store.create_proposal(params.0.name.clone(), proposed);

        Ok(json_text(&serde_json::json!({
            "proposal_id": proposal_id.to_string(),
            "name": params.0.name,
            "event_count": params.0.events.len(),
        })))
    }

    #[tool(description = "Check a proposal for conflicts against existing calendar events. Returns a detailed conflict report. Does NOT modify the calendar.")]
    async fn check_conflicts(
        &self,
        params: Parameters<CheckConflictsParams>,
    ) -> Result<CallToolResult, McpError> {
        let proposal_id = uuid::Uuid::parse_str(&params.0.proposal_id)
            .map(ProposalId)
            .map_err(|e| {
                tempo_err(TempoError::InvalidInput(format!(
                    "Invalid proposal ID: {}",
                    e
                )))
            })?;

        let check_internal = params.0.check_internal.unwrap_or(true);

        let store = self.store.read().await;
        let report = store
            .check_conflicts(&proposal_id, params.0.calendar_name.as_deref(), check_internal)
            .map_err(tempo_err)?;

        Ok(json_text(&report))
    }

    #[tool(description = "List all current proposals with their names and event counts.")]
    async fn list_proposals(&self) -> Result<CallToolResult, McpError> {
        let store = self.store.read().await;
        let proposals = store.list_proposals();

        let list: Vec<serde_json::Value> = proposals
            .iter()
            .map(|p| {
                serde_json::json!({
                    "proposal_id": p.id.to_string(),
                    "name": p.name,
                    "event_count": p.events.len(),
                    "created_at": p.created_at.to_rfc3339(),
                })
            })
            .collect();

        Ok(json_text(&list))
    }

    #[tool(description = "Withdraw (delete) a proposal. The proposed events are discarded.")]
    async fn withdraw_proposal(
        &self,
        params: Parameters<WithdrawProposalParams>,
    ) -> Result<CallToolResult, McpError> {
        let proposal_id = uuid::Uuid::parse_str(&params.0.proposal_id)
            .map(ProposalId)
            .map_err(|e| {
                tempo_err(TempoError::InvalidInput(format!(
                    "Invalid proposal ID: {}",
                    e
                )))
            })?;

        let mut store = self.store.write().await;
        store
            .withdraw_proposal(&proposal_id)
            .ok_or_else(|| tempo_err(TempoError::ProposalNotFound(params.0.proposal_id.clone())))?;

        Ok(json_text(&serde_json::json!({ "withdrawn": true })))
    }

    // === Mutations ===

    #[tool(description = "Commit a proposal, adding all its events to the calendar. The proposal is consumed. Returns assigned event IDs.")]
    async fn commit_proposal(
        &self,
        params: Parameters<CommitProposalParams>,
    ) -> Result<CallToolResult, McpError> {
        let proposal_id = uuid::Uuid::parse_str(&params.0.proposal_id)
            .map(ProposalId)
            .map_err(|e| {
                tempo_err(TempoError::InvalidInput(format!(
                    "Invalid proposal ID: {}",
                    e
                )))
            })?;
        let cal_name = params.0.calendar_name.as_deref().unwrap_or("default");

        let mut store = self.store.write().await;
        let ids = store
            .commit_proposal(&proposal_id, cal_name)
            .map_err(tempo_err)?;

        let id_strings: Vec<String> = ids.iter().map(|id| id.to_string()).collect();

        Ok(json_text(&serde_json::json!({
            "event_ids": id_strings,
            "event_count": id_strings.len(),
        })))
    }

    #[tool(description = "Propose events, check for conflicts, and commit in a single step. If conflict-free, commits immediately and returns event IDs. If conflicts found, returns the conflict details without committing. This is the recommended way to finalize a schedule.")]
    async fn propose_and_commit(
        &self,
        params: Parameters<ProposeAndCommitParams>,
    ) -> Result<CallToolResult, McpError> {
        let mut proposed = Vec::new();
        for input in &params.0.events {
            proposed.push(json_event_to_proposed(input).map_err(tempo_err)?);
        }

        let cal_name = params.0.calendar_name.as_deref().unwrap_or("default");
        let mut store = self.store.write().await;

        // Create proposal
        let proposal_id = store.create_proposal(params.0.name.clone(), proposed);

        // Check conflicts
        let report = store
            .check_conflicts(&proposal_id, Some(cal_name), true)
            .map_err(tempo_err)?;

        if report.has_conflicts {
            // Withdraw and return conflicts
            store.withdraw_proposal(&proposal_id);
            Ok(json_text(&serde_json::json!({
                "committed": false,
                "conflicts": report.conflicts,
            })))
        } else {
            // Commit
            let ids = store
                .commit_proposal(&proposal_id, cal_name)
                .map_err(tempo_err)?;
            let id_strings: Vec<String> = ids.iter().map(|id| id.to_string()).collect();
            Ok(json_text(&serde_json::json!({
                "committed": true,
                "event_count": id_strings.len(),
                "event_ids": id_strings,
            })))
        }
    }

    #[tool(description = "Add a single event directly to a calendar (bypassing the proposal workflow). Use for simple additions where conflict checking isn't needed.")]
    async fn add_event(
        &self,
        params: Parameters<AddEventParams>,
    ) -> Result<CallToolResult, McpError> {
        let input = JsonEventInput {
            title: params.0.title,
            start: params.0.start,
            end: params.0.end,
            timezone: params.0.timezone,
            rrule: params.0.rrule,
            metadata: params.0.metadata,
        };
        let event = json_event_to_event(&input).map_err(tempo_err)?;
        let event_id = event.id;
        let cal_name = params.0.calendar_name.as_deref().unwrap_or("default");

        let mut store = self.store.write().await;
        let cal = store.get_or_create_calendar(cal_name);
        cal.add_event(event);

        Ok(json_text(&serde_json::json!({
            "event_id": event_id.to_string(),
        })))
    }

    #[tool(description = "Remove an event from a calendar by its ID.")]
    async fn remove_event(
        &self,
        params: Parameters<RemoveEventParams>,
    ) -> Result<CallToolResult, McpError> {
        let event_id = uuid::Uuid::parse_str(&params.0.event_id)
            .map(EventId)
            .map_err(|e| {
                tempo_err(TempoError::InvalidInput(format!(
                    "Invalid event ID: {}",
                    e
                )))
            })?;
        let cal_name = params.0.calendar_name.as_deref().unwrap_or("default");

        let mut store = self.store.write().await;
        let cal = store
            .get_calendar_mut(cal_name)
            .ok_or_else(|| tempo_err(TempoError::CalendarNotFound(cal_name.to_string())))?;

        cal.remove_event(&event_id)
            .ok_or_else(|| tempo_err(TempoError::EventNotFound(params.0.event_id.clone())))?;

        Ok(json_text(&serde_json::json!({ "removed": true })))
    }

    #[tool(description = "Remove all events from a calendar. Proposals are NOT affected.")]
    async fn clear_calendar(
        &self,
        params: Parameters<ClearCalendarParams>,
    ) -> Result<CallToolResult, McpError> {
        let cal_name = params.0.calendar_name.as_deref().unwrap_or("default");

        let mut store = self.store.write().await;
        let cal = store
            .get_calendar_mut(cal_name)
            .ok_or_else(|| tempo_err(TempoError::CalendarNotFound(cal_name.to_string())))?;

        cal.clear();

        Ok(json_text(&serde_json::json!({ "cleared": true })))
    }

    // === Export ===

    #[tool(description = "Export a calendar as iCal/ICS format string. Includes all events with recurrence rules.")]
    async fn export_ical(
        &self,
        params: Parameters<ExportParams>,
    ) -> Result<CallToolResult, McpError> {
        let cal_name = params.0.calendar_name.as_deref().unwrap_or("default");

        let store = self.store.read().await;
        let cal = store
            .get_calendar(cal_name)
            .ok_or_else(|| tempo_err(TempoError::CalendarNotFound(cal_name.to_string())))?;

        let events: Vec<_> = cal.events().cloned().collect();
        let ical_str = ical_bridge::events_to_ical(&events);

        Ok(CallToolResult::success(vec![Content::text(ical_str)]))
    }

    #[tool(description = "Export a calendar as a JSON array of events.")]
    async fn export_json(
        &self,
        params: Parameters<ExportParams>,
    ) -> Result<CallToolResult, McpError> {
        let cal_name = params.0.calendar_name.as_deref().unwrap_or("default");

        let store = self.store.read().await;
        let cal = store
            .get_calendar(cal_name)
            .ok_or_else(|| tempo_err(TempoError::CalendarNotFound(cal_name.to_string())))?;

        #[derive(Serialize)]
        struct ExportedEvent {
            id: String,
            title: String,
            start: String,
            end: String,
            timezone: String,
            rrule: Option<String>,
            metadata: HashMap<String, String>,
        }

        let exported: Vec<ExportedEvent> = cal
            .events()
            .map(|e| ExportedEvent {
                id: e.id.to_string(),
                title: e.title.clone(),
                start: e.start.to_rfc3339(),
                end: e.end.to_rfc3339(),
                timezone: e.timezone.clone(),
                rrule: e.recurrence.as_ref().map(|r| r.rrule.clone()),
                metadata: e.metadata.clone(),
            })
            .collect();

        Ok(json_text(&exported))
    }
}

impl TempoServer {
    pub fn into_router(self) -> rmcp::handler::server::router::Router<Self> {
        let mut router = rmcp::handler::server::router::Router::new(self);
        router.tool_router = Self::tool_router();
        router
    }
}
