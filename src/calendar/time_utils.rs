use chrono::{DateTime, TimeDelta, Utc};
use serde::{Deserialize, Serialize};

use super::event::EventOccurrence;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct TimeRange {
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
}

impl TimeRange {
    pub fn new(start: DateTime<Utc>, end: DateTime<Utc>) -> Self {
        debug_assert!(start <= end, "TimeRange start ({start}) must be <= end ({end})");
        Self { start, end }
    }

    /// Half-open interval overlap: [start, end)
    pub fn overlaps(&self, other: &TimeRange) -> bool {
        self.start < other.end && other.start < self.end
    }

    pub fn overlap_duration(&self, other: &TimeRange) -> TimeDelta {
        let overlap_start = self.start.max(other.start);
        let overlap_end = self.end.min(other.end);
        if overlap_start < overlap_end {
            overlap_end - overlap_start
        } else {
            TimeDelta::zero()
        }
    }

    pub fn duration(&self) -> TimeDelta {
        self.end - self.start
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct BusyPeriod {
    pub range: TimeRange,
    pub event_titles: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct FreeBusyResult {
    pub busy_periods: Vec<BusyPeriod>,
    pub free_periods: Vec<TimeRange>,
    pub total_busy_minutes: i64,
    pub total_free_minutes: i64,
}

/// Merge overlapping time ranges into non-overlapping sorted ranges.
fn merge_ranges(ranges: &[TimeRange]) -> Vec<TimeRange> {
    if ranges.is_empty() {
        return vec![];
    }
    let mut sorted: Vec<TimeRange> = ranges.to_vec();
    sorted.sort_by_key(|r| r.start);

    let mut merged: Vec<TimeRange> = vec![sorted[0]];
    for r in &sorted[1..] {
        let last = merged.last_mut().unwrap();
        if r.start <= last.end {
            last.end = last.end.max(r.end);
        } else {
            merged.push(*r);
        }
    }
    merged
}

/// Find free slots of at least `min_duration` within `search_range`,
/// given a set of busy periods.
pub fn find_free_slots(
    busy_periods: &[TimeRange],
    search_range: &TimeRange,
    min_duration: TimeDelta,
) -> Vec<TimeRange> {
    let merged = merge_ranges(busy_periods);

    let mut free = Vec::new();
    let mut cursor = search_range.start;

    for period in &merged {
        if period.start > cursor {
            let gap = TimeRange::new(cursor, period.start.min(search_range.end));
            if gap.duration() >= min_duration {
                free.push(gap);
            }
        }
        cursor = cursor.max(period.end);
        if cursor >= search_range.end {
            break;
        }
    }

    // Trailing free slot
    if cursor < search_range.end {
        let gap = TimeRange::new(cursor, search_range.end);
        if gap.duration() >= min_duration {
            free.push(gap);
        }
    }

    free
}

/// Compute free/busy breakdown for a time range given event occurrences.
pub fn compute_free_busy(
    occurrences: &[EventOccurrence],
    range: &TimeRange,
) -> FreeBusyResult {
    if occurrences.is_empty() {
        return FreeBusyResult {
            busy_periods: vec![],
            free_periods: vec![*range],
            total_busy_minutes: 0,
            total_free_minutes: range.duration().num_minutes(),
        };
    }

    // Collect all busy ranges clipped to the search range
    let mut busy_ranges: Vec<(TimeRange, String)> = occurrences
        .iter()
        .filter_map(|occ| {
            let occ_range = TimeRange::new(occ.start, occ.end);
            if occ_range.overlaps(range) {
                let clipped = TimeRange::new(
                    occ.start.max(range.start),
                    occ.end.min(range.end),
                );
                Some((clipped, occ.title.clone()))
            } else {
                None
            }
        })
        .collect();

    busy_ranges.sort_by_key(|(r, _)| r.start);

    // Build busy periods with merged ranges and associated titles
    let mut busy_periods: Vec<BusyPeriod> = Vec::new();
    for (r, title) in &busy_ranges {
        if let Some(last) = busy_periods.last_mut() {
            if r.start <= last.range.end {
                last.range.end = last.range.end.max(r.end);
                if !last.event_titles.contains(title) {
                    last.event_titles.push(title.clone());
                }
                continue;
            }
        }
        busy_periods.push(BusyPeriod {
            range: *r,
            event_titles: vec![title.clone()],
        });
    }

    let busy_only: Vec<TimeRange> = busy_periods.iter().map(|bp| bp.range).collect();
    let free_periods = find_free_slots(&busy_only, range, TimeDelta::zero());

    let total_busy_minutes: i64 = busy_periods
        .iter()
        .map(|bp| bp.range.duration().num_minutes())
        .sum();
    let total_free_minutes = range.duration().num_minutes() - total_busy_minutes;

    FreeBusyResult {
        busy_periods,
        free_periods,
        total_busy_minutes,
        total_free_minutes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn utc(year: i32, month: u32, day: u32, hour: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(year, month, day, hour, 0, 0).unwrap()
    }

    fn mins(n: i64) -> TimeDelta {
        TimeDelta::minutes(n)
    }

    // -- TimeRange::overlaps --

    #[test]
    fn overlapping_ranges_detected() {
        let a = TimeRange::new(utc(2025, 1, 1, 9), utc(2025, 1, 1, 11));
        let b = TimeRange::new(utc(2025, 1, 1, 10), utc(2025, 1, 1, 12));
        assert!(a.overlaps(&b));
        assert!(b.overlaps(&a));
    }

    #[test]
    fn adjacent_ranges_do_not_overlap() {
        let a = TimeRange::new(utc(2025, 1, 1, 9), utc(2025, 1, 1, 10));
        let b = TimeRange::new(utc(2025, 1, 1, 10), utc(2025, 1, 1, 11));
        assert!(!a.overlaps(&b));
        assert!(!b.overlaps(&a));
    }

    #[test]
    fn disjoint_ranges_do_not_overlap() {
        let a = TimeRange::new(utc(2025, 1, 1, 9), utc(2025, 1, 1, 10));
        let b = TimeRange::new(utc(2025, 1, 1, 14), utc(2025, 1, 1, 15));
        assert!(!a.overlaps(&b));
    }

    #[test]
    fn contained_range_overlaps() {
        let outer = TimeRange::new(utc(2025, 1, 1, 8), utc(2025, 1, 1, 17));
        let inner = TimeRange::new(utc(2025, 1, 1, 10), utc(2025, 1, 1, 12));
        assert!(outer.overlaps(&inner));
        assert!(inner.overlaps(&outer));
    }

    // -- TimeRange::overlap_duration --

    #[test]
    fn overlap_duration_partial() {
        let a = TimeRange::new(utc(2025, 1, 1, 9), utc(2025, 1, 1, 11));
        let b = TimeRange::new(utc(2025, 1, 1, 10), utc(2025, 1, 1, 12));
        assert_eq!(a.overlap_duration(&b), mins(60));
    }

    #[test]
    fn overlap_duration_no_overlap() {
        let a = TimeRange::new(utc(2025, 1, 1, 9), utc(2025, 1, 1, 10));
        let b = TimeRange::new(utc(2025, 1, 1, 10), utc(2025, 1, 1, 11));
        assert_eq!(a.overlap_duration(&b), TimeDelta::zero());
    }

    // -- find_free_slots --

    #[test]
    fn free_slots_with_no_busy_periods() {
        let range = TimeRange::new(utc(2025, 1, 1, 8), utc(2025, 1, 1, 17));
        let slots = find_free_slots(&[], &range, mins(30));
        assert_eq!(slots.len(), 1);
        assert_eq!(slots[0], range);
    }

    #[test]
    fn free_slots_between_busy_periods() {
        let range = TimeRange::new(utc(2025, 1, 1, 8), utc(2025, 1, 1, 17));
        let busy = vec![
            TimeRange::new(utc(2025, 1, 1, 9), utc(2025, 1, 1, 10)),
            TimeRange::new(utc(2025, 1, 1, 14), utc(2025, 1, 1, 15)),
        ];
        let slots = find_free_slots(&busy, &range, mins(30));
        assert_eq!(slots.len(), 3);
        assert_eq!(slots[0], TimeRange::new(utc(2025, 1, 1, 8), utc(2025, 1, 1, 9)));
        assert_eq!(slots[1], TimeRange::new(utc(2025, 1, 1, 10), utc(2025, 1, 1, 14)));
        assert_eq!(slots[2], TimeRange::new(utc(2025, 1, 1, 15), utc(2025, 1, 1, 17)));
    }

    #[test]
    fn free_slots_filters_by_min_duration() {
        let range = TimeRange::new(utc(2025, 1, 1, 8), utc(2025, 1, 1, 12));
        let busy = vec![
            TimeRange::new(utc(2025, 1, 1, 8), utc(2025, 1, 1, 9)),
            // 15 min gap
            TimeRange::new(utc(2025, 1, 1, 9), utc(2025, 1, 1, 10)),
            // 2 hour gap
        ];
        // With 60 min minimum, only the trailing 2h slot qualifies
        let slots = find_free_slots(&busy, &range, mins(60));
        assert_eq!(slots.len(), 1);
        assert_eq!(slots[0], TimeRange::new(utc(2025, 1, 1, 10), utc(2025, 1, 1, 12)));
    }

    #[test]
    fn free_slots_with_overlapping_busy_periods() {
        let range = TimeRange::new(utc(2025, 1, 1, 8), utc(2025, 1, 1, 17));
        let busy = vec![
            TimeRange::new(utc(2025, 1, 1, 9), utc(2025, 1, 1, 11)),
            TimeRange::new(utc(2025, 1, 1, 10), utc(2025, 1, 1, 12)),
        ];
        let slots = find_free_slots(&busy, &range, mins(30));
        assert_eq!(slots.len(), 2);
        assert_eq!(slots[0], TimeRange::new(utc(2025, 1, 1, 8), utc(2025, 1, 1, 9)));
        assert_eq!(slots[1], TimeRange::new(utc(2025, 1, 1, 12), utc(2025, 1, 1, 17)));
    }

    #[test]
    fn fully_booked_returns_no_slots() {
        let range = TimeRange::new(utc(2025, 1, 1, 9), utc(2025, 1, 1, 10));
        let busy = vec![TimeRange::new(utc(2025, 1, 1, 8), utc(2025, 1, 1, 11))];
        let slots = find_free_slots(&busy, &range, mins(1));
        assert!(slots.is_empty());
    }

    // -- compute_free_busy --

    #[test]
    fn free_busy_with_no_events() {
        let range = TimeRange::new(utc(2025, 1, 1, 8), utc(2025, 1, 1, 17));
        let result = compute_free_busy(&[], &range);
        assert!(result.busy_periods.is_empty());
        assert_eq!(result.free_periods.len(), 1);
        assert_eq!(result.total_busy_minutes, 0);
        assert_eq!(result.total_free_minutes, 540); // 9 hours
    }

    #[test]
    fn free_busy_with_events() {
        let range = TimeRange::new(utc(2025, 1, 1, 8), utc(2025, 1, 1, 12));
        let occurrences = vec![
            EventOccurrence {
                event_id: super::super::event::EventId::new(),
                title: "Meeting".to_string(),
                start: utc(2025, 1, 1, 9),
                end: utc(2025, 1, 1, 10),
                is_recurring: false,
                metadata: Default::default(),
            },
        ];
        let result = compute_free_busy(&occurrences, &range);
        assert_eq!(result.busy_periods.len(), 1);
        assert_eq!(result.busy_periods[0].event_titles, vec!["Meeting"]);
        assert_eq!(result.total_busy_minutes, 60);
        assert_eq!(result.total_free_minutes, 180);
    }
}
