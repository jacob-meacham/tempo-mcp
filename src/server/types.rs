use std::collections::HashMap;

use schemars::JsonSchema;
use serde::Deserialize;

#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct JsonEventInput {
    #[schemars(description = "Event title")]
    pub(crate) title: String,
    #[schemars(description = "Start time (ISO 8601, e.g. '2025-01-15T09:00:00Z')")]
    pub(crate) start: String,
    #[schemars(description = "End time (ISO 8601)")]
    pub(crate) end: String,
    #[schemars(description = "IANA timezone name (e.g. 'America/New_York'). Defaults to 'UTC'.")]
    pub(crate) timezone: Option<String>,
    #[schemars(description = "RRULE recurrence string (e.g. 'FREQ=WEEKLY;BYDAY=MO,WE,FR')")]
    pub(crate) rrule: Option<String>,
    #[schemars(description = "Key-value metadata")]
    pub(crate) metadata: Option<HashMap<String, String>>,
}

// -- Tool parameter structs --

#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct LoadIcalParams {
    #[schemars(description = "The iCal/ICS data as a string")]
    pub(crate) ical_data: String,
    #[schemars(description = "Calendar name. Creates or adds to existing. Defaults to 'default'.")]
    pub(crate) calendar_name: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct LoadJsonParams {
    #[schemars(description = "Array of event objects")]
    pub(crate) events: Vec<JsonEventInput>,
    #[schemars(description = "Calendar name. Defaults to 'default'.")]
    pub(crate) calendar_name: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct ListEventsParams {
    #[schemars(description = "Start of time range (ISO 8601)")]
    pub(crate) start: String,
    #[schemars(description = "End of time range (ISO 8601)")]
    pub(crate) end: String,
    #[schemars(description = "Calendar name. If omitted, queries all calendars.")]
    pub(crate) calendar_name: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct GetFreeBusyParams {
    #[schemars(description = "Start of time range (ISO 8601)")]
    pub(crate) start: String,
    #[schemars(description = "End of time range (ISO 8601)")]
    pub(crate) end: String,
    #[schemars(description = "Calendar name. If omitted, considers all calendars.")]
    pub(crate) calendar_name: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct FindAvailableSlotsParams {
    #[schemars(description = "Start of search range (ISO 8601)")]
    pub(crate) start: String,
    #[schemars(description = "End of search range (ISO 8601)")]
    pub(crate) end: String,
    #[schemars(description = "Minimum slot duration in minutes")]
    pub(crate) duration_minutes: u32,
    #[schemars(description = "Calendar name. If omitted, considers all calendars.")]
    pub(crate) calendar_name: Option<String>,
    #[schemars(description = "Buffer minutes to reserve on each side of existing events (e.g. for travel time). Returned slots will already account for these buffers. Defaults to 0.")]
    pub(crate) buffer_minutes: Option<u32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct ProposeEventsParams {
    #[schemars(description = "A descriptive name for this proposal (e.g. 'Option A: morning blocks')")]
    pub(crate) name: String,
    #[schemars(description = "Array of proposed events")]
    pub(crate) events: Vec<JsonEventInput>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct CheckConflictsParams {
    #[schemars(description = "The proposal ID to check")]
    pub(crate) proposal_id: String,
    #[schemars(description = "Calendar name to check against. If omitted, checks all.")]
    pub(crate) calendar_name: Option<String>,
    #[schemars(description = "Also check for conflicts among proposed events themselves. Defaults to true.")]
    pub(crate) check_internal: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct WithdrawProposalParams {
    #[schemars(description = "The proposal ID to withdraw")]
    pub(crate) proposal_id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct CommitProposalParams {
    #[schemars(description = "The proposal ID to commit")]
    pub(crate) proposal_id: String,
    #[schemars(description = "Calendar name to add events to. Defaults to 'default'.")]
    pub(crate) calendar_name: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct ProposeAndCommitParams {
    #[schemars(description = "A descriptive name for this proposal (e.g. 'Weekly focus blocks')")]
    pub(crate) name: String,
    #[schemars(description = "Array of proposed events")]
    pub(crate) events: Vec<JsonEventInput>,
    #[schemars(description = "Calendar name to check against and commit to. Defaults to 'default'.")]
    pub(crate) calendar_name: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct AddEventParams {
    #[schemars(description = "Event title")]
    pub(crate) title: String,
    #[schemars(description = "Start time (ISO 8601)")]
    pub(crate) start: String,
    #[schemars(description = "End time (ISO 8601)")]
    pub(crate) end: String,
    #[schemars(description = "IANA timezone (defaults to 'UTC')")]
    pub(crate) timezone: Option<String>,
    #[schemars(description = "RRULE recurrence string")]
    pub(crate) rrule: Option<String>,
    #[schemars(description = "Key-value metadata")]
    pub(crate) metadata: Option<HashMap<String, String>>,
    #[schemars(description = "Calendar name. Defaults to 'default'.")]
    pub(crate) calendar_name: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct RemoveEventParams {
    #[schemars(description = "The event ID to remove")]
    pub(crate) event_id: String,
    #[schemars(description = "Calendar name. Defaults to 'default'.")]
    pub(crate) calendar_name: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct ClearCalendarParams {
    #[schemars(description = "Calendar name to clear. Defaults to 'default'.")]
    pub(crate) calendar_name: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct ExportParams {
    #[schemars(description = "Calendar name. Defaults to 'default'.")]
    pub(crate) calendar_name: Option<String>,
}

// -- Google Calendar API types --

#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct GCalDateTime {
    #[schemars(description = "ISO 8601 datetime with offset (e.g. '2025-01-20T09:00:00-07:00')")]
    #[serde(alias = "dateTime")]
    pub(crate) date_time: Option<String>,
    #[schemars(description = "Date only for all-day events (e.g. '2025-01-20')")]
    pub(crate) date: Option<String>,
    #[schemars(description = "IANA timezone (e.g. 'America/Denver')")]
    #[serde(alias = "timeZone")]
    pub(crate) time_zone: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct GCalEvent {
    #[schemars(description = "Event ID")]
    pub(crate) id: Option<String>,
    #[schemars(description = "Event title/summary")]
    pub(crate) summary: Option<String>,
    #[schemars(description = "Start time")]
    pub(crate) start: GCalDateTime,
    #[schemars(description = "End time")]
    pub(crate) end: GCalDateTime,
    #[schemars(description = "Event description")]
    pub(crate) description: Option<String>,
    #[schemars(description = "Event location")]
    pub(crate) location: Option<String>,
    #[schemars(description = "Event status")]
    #[allow(dead_code)] // accepted from Google Calendar API but not used
    pub(crate) status: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct LoadGoogleCalendarParams {
    #[schemars(description = "Array of Google Calendar event objects (as returned by the Google Calendar API). Each event has summary, start: {dateTime, timeZone}, end: {dateTime, timeZone}.")]
    pub(crate) events: Vec<GCalEvent>,
    #[schemars(description = "Calendar name. Defaults to 'default'.")]
    pub(crate) calendar_name: Option<String>,
}
