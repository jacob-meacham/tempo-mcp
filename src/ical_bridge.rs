use chrono::{DateTime, NaiveDateTime, Utc};
use icalendar::{Calendar as IcalCalendar, CalendarComponent, CalendarDateTime, Component, DatePerhapsTime, EventLike};

use crate::calendar::event::{Event, EventId, RecurrenceRule};
use crate::error::TempoError;

/// Parse an iCal string into domain Events.
pub fn parse_ical(ical_data: &str) -> Result<Vec<Event>, TempoError> {
    let calendar: IcalCalendar = ical_data
        .parse()
        .map_err(|e| TempoError::InvalidIcal(format!("Parse error: {}", e)))?;

    let mut events = Vec::new();
    for component in &calendar.components {
        if let CalendarComponent::Event(ical_event) = component {
            let event = ical_event_to_domain(ical_event)?;
            events.push(event);
        }
    }
    Ok(events)
}

fn ical_event_to_domain(ical_event: &icalendar::Event) -> Result<Event, TempoError> {
    let title = ical_event
        .get_summary()
        .unwrap_or("(untitled)")
        .to_string();

    let start = extract_datetime(ical_event.get_start(), "DTSTART")?;
    let end = extract_datetime(ical_event.get_end(), "DTEND")
        .unwrap_or(start + chrono::Duration::hours(1));

    let rrule = ical_event
        .property_value("RRULE")
        .map(|s| RecurrenceRule {
            rrule: s.to_string(),
        });

    Ok(Event {
        id: EventId::new(),
        title,
        start,
        end,
        timezone: "UTC".to_string(),
        recurrence: rrule,
        metadata: Default::default(),
    })
}

fn extract_datetime(
    dpt: Option<DatePerhapsTime>,
    field_name: &str,
) -> Result<DateTime<Utc>, TempoError> {
    match dpt {
        Some(DatePerhapsTime::DateTime(cdt)) => match cdt {
            CalendarDateTime::Utc(utc) => Ok(utc),
            CalendarDateTime::Floating(naive) => Ok(naive.and_utc()),
            CalendarDateTime::WithTimezone { date_time, .. } => Ok(date_time.and_utc()),
        },
        Some(DatePerhapsTime::Date(d)) => {
            // All-day event: midnight to midnight UTC
            let naive = NaiveDateTime::new(d, chrono::NaiveTime::from_hms_opt(0, 0, 0).unwrap());
            Ok(naive.and_utc())
        }
        None => Err(TempoError::InvalidIcal(format!(
            "Missing {} field",
            field_name
        ))),
    }
}

/// Export domain Events to an iCal string.
pub fn events_to_ical(events: &[Event]) -> String {
    let mut cal = IcalCalendar::new();
    cal.name("Tempo Calendar");

    for event in events {
        let mut ical_event = icalendar::Event::new();
        ical_event.summary(&event.title);
        ical_event.starts(event.start);
        ical_event.ends(event.end);
        ical_event.uid(&event.id.0.to_string());

        if let Some(ref recurrence) = event.recurrence {
            ical_event.add_property("RRULE", &recurrence.rrule);
        }

        cal.push(ical_event.done());
    }

    cal.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple_ical() {
        let ical = "BEGIN:VCALENDAR\r\n\
            VERSION:2.0\r\n\
            BEGIN:VEVENT\r\n\
            SUMMARY:Team Standup\r\n\
            DTSTART:20250115T090000Z\r\n\
            DTEND:20250115T093000Z\r\n\
            END:VEVENT\r\n\
            END:VCALENDAR\r\n";

        let events = parse_ical(ical).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].title, "Team Standup");
        assert_eq!(events[0].start.hour(), 9);
        assert_eq!(events[0].end.minute(), 30);
    }

    #[test]
    fn parse_ical_with_rrule() {
        let ical = "BEGIN:VCALENDAR\r\n\
            VERSION:2.0\r\n\
            BEGIN:VEVENT\r\n\
            SUMMARY:Daily Standup\r\n\
            DTSTART:20250115T090000Z\r\n\
            DTEND:20250115T091500Z\r\n\
            RRULE:FREQ=DAILY;COUNT=5\r\n\
            END:VEVENT\r\n\
            END:VCALENDAR\r\n";

        let events = parse_ical(ical).unwrap();
        assert_eq!(events.len(), 1);
        assert!(events[0].recurrence.is_some());
        assert_eq!(events[0].recurrence.as_ref().unwrap().rrule, "FREQ=DAILY;COUNT=5");
    }

    #[test]
    fn parse_multiple_events() {
        let ical = "BEGIN:VCALENDAR\r\n\
            VERSION:2.0\r\n\
            BEGIN:VEVENT\r\n\
            SUMMARY:Meeting A\r\n\
            DTSTART:20250115T090000Z\r\n\
            DTEND:20250115T100000Z\r\n\
            END:VEVENT\r\n\
            BEGIN:VEVENT\r\n\
            SUMMARY:Meeting B\r\n\
            DTSTART:20250115T140000Z\r\n\
            DTEND:20250115T150000Z\r\n\
            END:VEVENT\r\n\
            END:VCALENDAR\r\n";

        let events = parse_ical(ical).unwrap();
        assert_eq!(events.len(), 2);
    }

    #[test]
    fn export_and_reparse_round_trip() {
        let ical = "BEGIN:VCALENDAR\r\n\
            VERSION:2.0\r\n\
            BEGIN:VEVENT\r\n\
            SUMMARY:Round Trip Test\r\n\
            DTSTART:20250115T100000Z\r\n\
            DTEND:20250115T110000Z\r\n\
            END:VEVENT\r\n\
            END:VCALENDAR\r\n";

        let events = parse_ical(ical).unwrap();
        let exported = events_to_ical(&events);
        let reparsed = parse_ical(&exported).unwrap();

        assert_eq!(events.len(), reparsed.len());
        assert_eq!(events[0].title, reparsed[0].title);
        assert_eq!(events[0].start, reparsed[0].start);
        assert_eq!(events[0].end, reparsed[0].end);
    }

    #[test]
    fn parse_invalid_ical_produces_no_events() {
        let result = parse_ical("not valid ical data");
        // The icalendar crate is lenient — it may parse empty or return an error
        match result {
            Ok(events) => assert!(events.is_empty(), "Expected no events from invalid iCal"),
            Err(_) => {} // Error is also acceptable
        }
    }

    use chrono::Timelike;
}
