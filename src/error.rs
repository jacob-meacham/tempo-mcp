use thiserror::Error;

#[derive(Debug, Error)]
pub enum TempoError {
    #[error("Calendar not found: {0}")]
    CalendarNotFound(String),

    #[error("Event not found: {0}")]
    EventNotFound(String),

    #[error("Proposal not found: {0}")]
    ProposalNotFound(String),

    #[error("Invalid iCal data: {0}")]
    InvalidIcal(String),

    #[error("Invalid RRULE: {0}")]
    InvalidRrule(String),

    #[error("Invalid time range: {0}")]
    InvalidTimeRange(String),

    #[error("Invalid input: {0}")]
    InvalidInput(String),
}
