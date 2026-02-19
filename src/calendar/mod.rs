pub mod event;
pub mod proposal;
pub mod time_utils;

use std::collections::HashMap;

use chrono::{DateTime, TimeDelta, Utc};

use crate::error::TempoError;
use event::{Event, EventId, EventOccurrence};
use proposal::{
    ConflictReport, Proposal, ProposalId, ProposedEvent, detect_conflicts,
};
use time_utils::{FreeBusyResult, TimeRange, compute_free_busy, find_free_slots};

const MAX_RECURRENCE_OCCURRENCES: u16 = 1000;

#[derive(Debug)]
pub struct Calendar {
    #[allow(dead_code)] // stored for debugging/display
    name: String,
    events: HashMap<EventId, Event>,
}

impl Calendar {
    pub fn new(name: String) -> Self {
        Self {
            name,
            events: HashMap::new(),
        }
    }

    pub fn add_event(&mut self, event: Event) -> EventId {
        let id = event.id;
        self.events.insert(id, event);
        id
    }

    pub fn remove_event(&mut self, id: &EventId) -> Option<Event> {
        self.events.remove(id)
    }

    pub fn events(&self) -> impl Iterator<Item = &Event> {
        self.events.values()
    }

    pub fn clear(&mut self) {
        self.events.clear();
    }

    pub fn occurrences_in_range(
        &self,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> Result<Vec<EventOccurrence>, TempoError> {
        let mut occurrences = Vec::new();
        for event in self.events.values() {
            let mut event_occs = expand_event(event, start, end)?;
            occurrences.append(&mut event_occs);
        }
        occurrences.sort_by_key(|o| o.start);
        Ok(occurrences)
    }
}

/// Expand an event (possibly recurring) into concrete occurrences within a range.
fn expand_event(
    event: &Event,
    range_start: DateTime<Utc>,
    range_end: DateTime<Utc>,
) -> Result<Vec<EventOccurrence>, TempoError> {
    let Some(ref recurrence) = event.recurrence else {
        // Non-recurring: include if it overlaps the range
        let event_range = TimeRange::new(event.start, event.end);
        let query_range = TimeRange::new(range_start, range_end);
        if event_range.overlaps(&query_range) {
            return Ok(vec![event.to_occurrence()]);
        }
        return Ok(vec![]);
    };

    let duration = event.end - event.start;

    let rrule_str = format!(
        "DTSTART:{}\nRRULE:{}",
        event.start.format("%Y%m%dT%H%M%SZ"),
        recurrence.rrule
    );

    let rrule_set: rrule::RRuleSet = rrule_str
        .parse()
        .map_err(|e| TempoError::InvalidRrule(format!("{}: {}", recurrence.rrule, e)))?;

    // Convert DateTime<Utc> to DateTime<rrule::Tz> for the rrule API
    let tz_start = range_start.with_timezone(&rrule::Tz::UTC);
    let tz_end = range_end.with_timezone(&rrule::Tz::UTC);

    let result = rrule_set
        .after(tz_start)
        .before(tz_end)
        .all(MAX_RECURRENCE_OCCURRENCES);

    Ok(result
        .dates
        .into_iter()
        .map(|dt| {
            let start_utc = dt.with_timezone(&Utc);
            EventOccurrence {
                event_id: event.id,
                title: event.title.clone(),
                start: start_utc,
                end: start_utc + duration,
                is_recurring: true,
                metadata: event.metadata.clone(),
            }
        })
        .collect())
}

#[derive(Debug)]
pub struct CalendarStore {
    calendars: HashMap<String, Calendar>,
    proposals: HashMap<ProposalId, Proposal>,
}

impl CalendarStore {
    pub fn new() -> Self {
        let mut calendars = HashMap::new();
        calendars.insert("default".to_string(), Calendar::new("default".to_string()));
        Self {
            calendars,
            proposals: HashMap::new(),
        }
    }

    /// Get or create a calendar by name (case-insensitive).
    pub fn get_or_create_calendar(&mut self, name: &str) -> &mut Calendar {
        let key = name.to_lowercase();
        self.calendars
            .entry(key.clone())
            .or_insert_with(|| Calendar::new(key))
    }

    pub fn get_calendar(&self, name: &str) -> Option<&Calendar> {
        self.calendars.get(&name.to_lowercase())
    }

    pub fn get_calendar_mut(&mut self, name: &str) -> Option<&mut Calendar> {
        self.calendars.get_mut(&name.to_lowercase())
    }

    /// Collect occurrences across all calendars or a specific one.
    pub fn occurrences_in_range(
        &self,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
        calendar_name: Option<&str>,
    ) -> Result<Vec<EventOccurrence>, TempoError> {
        let mut all = Vec::new();
        match calendar_name {
            Some(name) => {
                let cal = self
                    .get_calendar(name)
                    .ok_or_else(|| TempoError::CalendarNotFound(name.to_string()))?;
                all = cal.occurrences_in_range(start, end)?;
            }
            None => {
                for cal in self.calendars.values() {
                    let mut occs = cal.occurrences_in_range(start, end)?;
                    all.append(&mut occs);
                }
                all.sort_by_key(|o| o.start);
            }
        }
        Ok(all)
    }

    pub fn free_busy(
        &self,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
        calendar_name: Option<&str>,
    ) -> Result<FreeBusyResult, TempoError> {
        let occs = self.occurrences_in_range(start, end, calendar_name)?;
        let range = TimeRange::new(start, end);
        Ok(compute_free_busy(&occs, &range))
    }

    pub fn find_available_slots(
        &self,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
        min_duration: TimeDelta,
        calendar_name: Option<&str>,
    ) -> Result<Vec<TimeRange>, TempoError> {
        let occs = self.occurrences_in_range(start, end, calendar_name)?;
        let busy: Vec<TimeRange> = occs
            .iter()
            .map(|o| TimeRange::new(o.start, o.end))
            .collect();
        let range = TimeRange::new(start, end);
        Ok(find_free_slots(&busy, &range, min_duration))
    }

    // -- Proposal methods --

    pub fn create_proposal(&mut self, name: String, events: Vec<ProposedEvent>) -> ProposalId {
        let id = ProposalId::new();
        let proposal = Proposal {
            id,
            name,
            events,
            created_at: Utc::now(),
        };
        self.proposals.insert(id, proposal);
        id
    }

    pub fn list_proposals(&self) -> Vec<&Proposal> {
        self.proposals.values().collect()
    }

    pub fn withdraw_proposal(&mut self, id: &ProposalId) -> Option<Proposal> {
        self.proposals.remove(id)
    }

    pub fn check_conflicts(
        &self,
        proposal_id: &ProposalId,
        calendar_name: Option<&str>,
        check_internal: bool,
    ) -> Result<ConflictReport, TempoError> {
        let proposal = self
            .proposals
            .get(proposal_id)
            .ok_or_else(|| TempoError::ProposalNotFound(proposal_id.to_string()))?;

        // Convert proposed events to occurrences for conflict detection
        let proposed_occs: Vec<EventOccurrence> = proposal
            .events
            .iter()
            .map(|pe| EventOccurrence {
                event_id: EventId::new(),
                title: pe.title.clone(),
                start: pe.start,
                end: pe.end,
                is_recurring: pe.recurrence.is_some(),
                metadata: pe.metadata.clone(),
            })
            .collect();

        // Find the time range spanning all proposed events
        let Some((range_start, range_end)) = proposed_time_bounds(&proposal.events) else {
            return Ok(ConflictReport {
                proposal_id: *proposal_id,
                has_conflicts: false,
                conflicts: vec![],
            });
        };
        let existing = self.occurrences_in_range(range_start, range_end, calendar_name)?;

        let conflicts = detect_conflicts(&proposed_occs, &existing, check_internal);

        Ok(ConflictReport {
            proposal_id: *proposal_id,
            has_conflicts: !conflicts.is_empty(),
            conflicts,
        })
    }

    /// Commit a proposal: move its events into the target calendar.
    pub fn commit_proposal(
        &mut self,
        proposal_id: &ProposalId,
        calendar_name: &str,
    ) -> Result<Vec<EventId>, TempoError> {
        let proposal = self
            .proposals
            .remove(proposal_id)
            .ok_or_else(|| TempoError::ProposalNotFound(proposal_id.to_string()))?;

        let cal = self.get_or_create_calendar(calendar_name);
        let mut ids = Vec::with_capacity(proposal.events.len());

        for pe in proposal.events {
            let event = Event {
                id: EventId::new(),
                title: pe.title,
                start: pe.start,
                end: pe.end,
                timezone: pe.timezone,
                recurrence: pe.recurrence,
                metadata: pe.metadata,
            };
            ids.push(cal.add_event(event));
        }

        Ok(ids)
    }
}

/// Find the min start and max end across all proposed events.
fn proposed_time_bounds(events: &[ProposedEvent]) -> Option<(DateTime<Utc>, DateTime<Utc>)> {
    let start = events.iter().map(|e| e.start).min()?;
    let end = events.iter().map(|e| e.end).max()?;
    Some((start, end))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn utc(year: i32, month: u32, day: u32, hour: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(year, month, day, hour, 0, 0).unwrap()
    }

    fn make_event(title: &str, start: DateTime<Utc>, end: DateTime<Utc>) -> Event {
        Event {
            id: EventId::new(),
            title: title.to_string(),
            start,
            end,
            timezone: "UTC".to_string(),
            recurrence: None,
            metadata: Default::default(),
        }
    }

    #[test]
    fn calendar_add_and_query() {
        let mut cal = Calendar::new("test".to_string());
        cal.add_event(make_event("Meeting", utc(2025, 1, 1, 9), utc(2025, 1, 1, 10)));
        cal.add_event(make_event("Lunch", utc(2025, 1, 1, 12), utc(2025, 1, 1, 13)));

        let occs = cal
            .occurrences_in_range(utc(2025, 1, 1, 0), utc(2025, 1, 2, 0))
            .unwrap();
        assert_eq!(occs.len(), 2);
        assert_eq!(occs[0].title, "Meeting");
        assert_eq!(occs[1].title, "Lunch");
    }

    #[test]
    fn calendar_query_filters_by_range() {
        let mut cal = Calendar::new("test".to_string());
        cal.add_event(make_event("Morning", utc(2025, 1, 1, 9), utc(2025, 1, 1, 10)));
        cal.add_event(make_event("Afternoon", utc(2025, 1, 1, 14), utc(2025, 1, 1, 15)));

        let occs = cal
            .occurrences_in_range(utc(2025, 1, 1, 8), utc(2025, 1, 1, 11))
            .unwrap();
        assert_eq!(occs.len(), 1);
        assert_eq!(occs[0].title, "Morning");
    }

    #[test]
    fn calendar_remove_event() {
        let mut cal = Calendar::new("test".to_string());
        let event = make_event("ToRemove", utc(2025, 1, 1, 9), utc(2025, 1, 1, 10));
        let id = event.id;
        cal.add_event(event);
        assert!(cal.remove_event(&id).is_some());
        let occs = cal
            .occurrences_in_range(utc(2025, 1, 1, 0), utc(2025, 1, 2, 0))
            .unwrap();
        assert!(occs.is_empty());
    }

    #[test]
    fn store_default_calendar_exists() {
        let store = CalendarStore::new();
        assert!(store.get_calendar("default").is_some());
    }

    #[test]
    fn store_get_or_create_is_case_insensitive() {
        let mut store = CalendarStore::new();
        store.get_or_create_calendar("Work");
        assert!(store.get_calendar("work").is_some());
        assert!(store.get_calendar("WORK").is_some());
    }

    #[test]
    fn store_proposal_workflow() {
        let mut store = CalendarStore::new();

        // Add an existing event
        let cal = store.get_or_create_calendar("default");
        cal.add_event(make_event("Existing", utc(2025, 1, 1, 9), utc(2025, 1, 1, 10)));

        // Create a non-conflicting proposal
        let proposal_id = store.create_proposal(
            "Option A".to_string(),
            vec![ProposedEvent {
                title: "New Meeting".to_string(),
                start: utc(2025, 1, 1, 14),
                end: utc(2025, 1, 1, 15),
                timezone: "UTC".to_string(),
                recurrence: None,
                metadata: Default::default(),
            }],
        );

        // Check no conflicts
        let report = store.check_conflicts(&proposal_id, None, true).unwrap();
        assert!(!report.has_conflicts);

        // Commit
        let ids = store.commit_proposal(&proposal_id, "default").unwrap();
        assert_eq!(ids.len(), 1);

        // Verify event is in calendar
        let occs = store
            .occurrences_in_range(utc(2025, 1, 1, 0), utc(2025, 1, 2, 0), Some("default"))
            .unwrap();
        assert_eq!(occs.len(), 2);
    }

    #[test]
    fn find_available_slots_with_buffer() {
        // Given a calendar with an event 9-10 and we search 8-12 with 30min buffer,
        // the raw search uses effective_duration = requested + 2*buffer,
        // and the caller shrinks slots by buffer on both sides.
        // Here we test the store-level find_available_slots without buffer logic
        // (buffer shrinking is done in the server layer).
        let mut store = CalendarStore::new();
        let cal = store.get_or_create_calendar("default");
        cal.add_event(make_event("Meeting", utc(2025, 1, 1, 9), utc(2025, 1, 1, 10)));

        // Find 30-min slots in 8-12 range
        let slots = store
            .find_available_slots(
                utc(2025, 1, 1, 8),
                utc(2025, 1, 1, 12),
                TimeDelta::minutes(30),
                Some("default"),
            )
            .unwrap();
        assert_eq!(slots.len(), 2);
        // 8:00-9:00 (1 hour) and 10:00-12:00 (2 hours)
        assert_eq!(slots[0].start, utc(2025, 1, 1, 8));
        assert_eq!(slots[0].end, utc(2025, 1, 1, 9));
        assert_eq!(slots[1].start, utc(2025, 1, 1, 10));
        assert_eq!(slots[1].end, utc(2025, 1, 1, 12));
    }

    #[test]
    fn proposal_not_found_returns_error() {
        let store = CalendarStore::new();
        let fake_id = ProposalId::new();
        let result = store.check_conflicts(&fake_id, None, true);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, crate::error::TempoError::ProposalNotFound(_)));
    }

    #[test]
    fn store_proposal_detects_conflicts() {
        let mut store = CalendarStore::new();

        let cal = store.get_or_create_calendar("default");
        cal.add_event(make_event("Existing", utc(2025, 1, 1, 9), utc(2025, 1, 1, 10)));

        let proposal_id = store.create_proposal(
            "Conflicting".to_string(),
            vec![ProposedEvent {
                title: "Overlap".to_string(),
                start: utc(2025, 1, 1, 9),
                end: utc(2025, 1, 1, 11),
                timezone: "UTC".to_string(),
                recurrence: None,
                metadata: Default::default(),
            }],
        );

        let report = store.check_conflicts(&proposal_id, None, true).unwrap();
        assert!(report.has_conflicts);
        assert_eq!(report.conflicts.len(), 1);
        assert_eq!(report.conflicts[0].overlap_minutes, 60);
    }
}
