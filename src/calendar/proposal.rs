use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;
use uuid::Uuid;

use super::event::{EventId, EventOccurrence, RecurrenceRule};
use super::time_utils::TimeRange;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ProposalId(pub Uuid);

impl ProposalId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl fmt::Display for ProposalId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProposedEvent {
    pub title: String,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub timezone: String,
    pub recurrence: Option<RecurrenceRule>,
    pub metadata: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Proposal {
    pub id: ProposalId,
    pub name: String,
    pub events: Vec<ProposedEvent>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ConflictReport {
    pub proposal_id: ProposalId,
    pub has_conflicts: bool,
    pub conflicts: Vec<Conflict>,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Conflict {
    pub proposed_event_title: String,
    pub proposed_start: DateTime<Utc>,
    pub proposed_end: DateTime<Utc>,
    pub conflicting_event_id: Option<EventId>,
    pub conflicting_event_title: String,
    pub conflicting_start: DateTime<Utc>,
    pub conflicting_end: DateTime<Utc>,
    pub overlap_minutes: i64,
}

/// Detect conflicts between proposed occurrences and existing calendar occurrences.
/// Uses pairwise comparison: O(N*M + N^2) where N=proposed, M=existing.
pub fn detect_conflicts(
    proposed: &[EventOccurrence],
    existing: &[EventOccurrence],
    check_internal: bool,
) -> Vec<Conflict> {
    let mut conflicts = Vec::new();

    // Check proposed vs existing
    for prop in proposed {
        let prop_range = TimeRange::new(prop.start, prop.end);
        for exist in existing {
            let exist_range = TimeRange::new(exist.start, exist.end);
            let overlap = prop_range.overlap_duration(&exist_range);
            if overlap.num_minutes() > 0 {
                conflicts.push(Conflict {
                    proposed_event_title: prop.title.clone(),
                    proposed_start: prop.start,
                    proposed_end: prop.end,
                    conflicting_event_id: Some(exist.event_id),
                    conflicting_event_title: exist.title.clone(),
                    conflicting_start: exist.start,
                    conflicting_end: exist.end,
                    overlap_minutes: overlap.num_minutes(),
                });
            }
        }
    }

    // Check proposed vs proposed (internal conflicts)
    if check_internal && proposed.len() > 1 {
        for i in 0..proposed.len() {
            for j in (i + 1)..proposed.len() {
                let a_range = TimeRange::new(proposed[i].start, proposed[i].end);
                let b_range = TimeRange::new(proposed[j].start, proposed[j].end);
                let overlap = a_range.overlap_duration(&b_range);
                if overlap.num_minutes() > 0 {
                    conflicts.push(Conflict {
                        proposed_event_title: proposed[i].title.clone(),
                        proposed_start: proposed[i].start,
                        proposed_end: proposed[i].end,
                        conflicting_event_id: None,
                        conflicting_event_title: proposed[j].title.clone(),
                        conflicting_start: proposed[j].start,
                        conflicting_end: proposed[j].end,
                        overlap_minutes: overlap.num_minutes(),
                    });
                }
            }
        }
    }

    conflicts
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::calendar::event::EventId;
    use chrono::TimeZone;

    fn utc(year: i32, month: u32, day: u32, hour: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(year, month, day, hour, 0, 0).unwrap()
    }

    fn make_occurrence(title: &str, start: DateTime<Utc>, end: DateTime<Utc>) -> EventOccurrence {
        EventOccurrence {
            event_id: EventId::new(),
            title: title.to_string(),
            start,
            end,
            is_recurring: false,
            metadata: Default::default(),
        }
    }

    #[test]
    fn no_conflicts_when_no_overlap() {
        let proposed = vec![make_occurrence("New", utc(2025, 1, 1, 14), utc(2025, 1, 1, 15))];
        let existing = vec![make_occurrence("Old", utc(2025, 1, 1, 9), utc(2025, 1, 1, 10))];
        let conflicts = detect_conflicts(&proposed, &existing, true);
        assert!(conflicts.is_empty());
    }

    #[test]
    fn detects_overlap_with_existing() {
        let proposed = vec![make_occurrence("New", utc(2025, 1, 1, 9), utc(2025, 1, 1, 11))];
        let existing = vec![make_occurrence("Old", utc(2025, 1, 1, 10), utc(2025, 1, 1, 12))];
        let conflicts = detect_conflicts(&proposed, &existing, true);
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].overlap_minutes, 60);
        assert_eq!(conflicts[0].proposed_event_title, "New");
        assert_eq!(conflicts[0].conflicting_event_title, "Old");
    }

    #[test]
    fn detects_internal_conflicts() {
        let proposed = vec![
            make_occurrence("A", utc(2025, 1, 1, 9), utc(2025, 1, 1, 11)),
            make_occurrence("B", utc(2025, 1, 1, 10), utc(2025, 1, 1, 12)),
        ];
        let conflicts = detect_conflicts(&proposed, &[], true);
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].overlap_minutes, 60);
    }

    #[test]
    fn skips_internal_conflicts_when_disabled() {
        let proposed = vec![
            make_occurrence("A", utc(2025, 1, 1, 9), utc(2025, 1, 1, 11)),
            make_occurrence("B", utc(2025, 1, 1, 10), utc(2025, 1, 1, 12)),
        ];
        let conflicts = detect_conflicts(&proposed, &[], false);
        assert!(conflicts.is_empty());
    }

    #[test]
    fn adjacent_events_no_conflict() {
        let proposed = vec![make_occurrence("New", utc(2025, 1, 1, 10), utc(2025, 1, 1, 11))];
        let existing = vec![make_occurrence("Old", utc(2025, 1, 1, 9), utc(2025, 1, 1, 10))];
        let conflicts = detect_conflicts(&proposed, &existing, true);
        assert!(conflicts.is_empty());
    }
}
