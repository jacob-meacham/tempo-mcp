# Tempo: In-Memory Calendar MCP Server

## Context

LLMs struggle with complex scheduling (e.g., "schedule 6 blocks around these existing blocks over the next week"). Tempo is a lightning-fast, in-memory calendar MCP server in Rust that enables a **test-retest workflow**: hydrate existing events from iCal/JSON, query free/busy, propose new events, check conflicts, iterate, commit, and export.

The key insight is that proposals are first-class: the LLM can create multiple candidate schedules, compare them against existing events, and only commit when satisfied — without ever modifying the real calendar until ready.

---

## Architecture

### File Structure

```
src/
  main.rs                 -- Entry point: tokio, tracing to stderr, stdio transport
  error.rs                -- TempoError enum with thiserror
  ical_bridge.rs          -- icalendar crate <-> domain type conversions
  server/
    mod.rs                -- TempoServer, ServerHandler impl, #[tool] methods
    types.rs              -- Tool parameter/input structs (Deserialize + JsonSchema)
    conversions.rs        -- Helpers: parse_datetime, json/gcal event conversions, error mapping
  calendar/
    mod.rs                -- CalendarStore (top-level container)
    event.rs              -- Event, EventId, RecurrenceRule, EventOccurrence
    proposal.rs           -- Proposal, ProposalId, ConflictReport, conflict detection
    time_utils.rs         -- TimeRange, overlap, free/busy, slot-finding
```

### Key Dependencies

- `rmcp` — MCP server framework with `#[tool]` macros, stdio transport
- `tokio` — async runtime
- `icalendar` — iCal parsing and generation (RFC 5545)
- `rrule` — recurring event expansion
- `chrono` / `chrono-tz` — timezone-aware datetime
- `serde` / `schemars` — serialization and JSON schema generation for tool params

---

## Core Domain Types

### Event Model (`calendar/event.rs`)

- `EventId(Uuid)` — newtype for type safety
- `Event { id, title, start: DateTime<Utc>, end: DateTime<Utc>, timezone: String, recurrence: Option<RecurrenceRule>, metadata: HashMap<String, String> }`
- `RecurrenceRule { rrule: String }` — stores raw RRULE, parsed on demand via `rrule` crate
- `EventOccurrence { event_id, title, start, end, is_recurring, metadata }` — materialized occurrence for query results
- All times stored as UTC internally; `timezone` preserved for display/export

### Time Utilities (`calendar/time_utils.rs`)

- `TimeRange { start, end }` — half-open interval `[start, end)`
- `TimeRange::overlaps(&self, other)` / `TimeRange::overlap_duration(&self, other)`
- `find_free_slots(busy, range, min_duration)` — sorted sweep to find gaps
- `compute_free_busy(occurrences, range)` → `FreeBusyResult { busy_periods, free_periods, total_busy_minutes, total_free_minutes }`

### Proposal System (`calendar/proposal.rs`)

- `ProposalId(Uuid)` — newtype
- `Proposal { id, name, events: Vec<ProposedEvent>, created_at }`
- `ConflictReport { proposal_id, has_conflicts, conflicts: Vec<Conflict> }`
- `Conflict { proposed_title, proposed_start/end, conflicting_event_id, conflicting_title, conflicting_start/end, overlap_minutes }`

### Calendar Store (`calendar/mod.rs`)

- `CalendarStore { calendars: HashMap<String, Calendar>, proposals: HashMap<ProposalId, Proposal> }`
- Calendars keyed by name (case-insensitive). A "default" calendar auto-created.
- `Calendar { name, events: HashMap<EventId, Event> }` with methods: `add_event`, `remove_event`, `occurrences_in_range`
- Recurring event expansion via `rrule` crate with safety limit of 1000 occurrences
- Wrapped in `Arc<RwLock<CalendarStore>>` (tokio async RwLock) for safe concurrent access

---

## MCP Tools

### Hydration
| Tool | Params | Returns |
|------|--------|---------|
| `load_ical` | `ical_data: String, calendar_name?: String` | `{ calendar_name, events_loaded }` |
| `load_json` | `events: Vec<{title, start, end, timezone?, rrule?, metadata?}>, calendar_name?: String` | `{ calendar_name, events_loaded, event_ids }` |
| `load_google_calendar` | `events: Vec<GCalEvent>, calendar_name?: String` | `{ calendar_name, events_loaded, events_skipped, event_ids }` |

### Querying
| Tool | Params | Returns |
|------|--------|---------|
| `list_events` | `start, end, calendar_name?` | Sorted `EventOccurrence[]` (recurring expanded) |
| `get_free_busy` | `start, end, calendar_name?` | `FreeBusyResult` with busy/free periods and totals |
| `find_available_slots` | `start, end, duration_minutes, calendar_name?` | `TimeRange[]` of free slots |

### Proposals (the key differentiator)
| Tool | Params | Returns |
|------|--------|---------|
| `propose_events` | `name: String, events: Vec<...>` | `{ proposal_id, name, event_count }` |
| `check_conflicts` | `proposal_id, calendar_name?, check_internal?: bool` | `ConflictReport` |
| `list_proposals` | *(none)* | `{ id, name, event_count, created_at }[]` |
| `withdraw_proposal` | `proposal_id` | `{ withdrawn: true }` |
| `propose_and_commit` | `name, events: Vec<...>, calendar_name?` | `{ committed, event_ids? }` or `{ committed: false, conflicts }` |

### Mutations
| Tool | Params | Returns |
|------|--------|---------|
| `commit_proposal` | `proposal_id, calendar_name?` | `{ event_ids, event_count }` |
| `add_event` | `title, start, end, timezone?, rrule?, metadata?, calendar_name?` | `{ event_id }` |
| `remove_event` | `event_id, calendar_name?` | `{ removed: true }` |
| `clear_calendar` | `calendar_name?` | `{ cleared: true }` |

### Export
| Tool | Params | Returns |
|------|--------|---------|
| `export_ical` | `calendar_name?` | iCal string |
| `export_json` | `calendar_name?` | JSON event array |

### Design Notes

- `calendar_name` defaults to "default" everywhere; omitting it queries all calendars for read ops, uses default for writes
- No "update event" tool — use remove + add (keeps API surface small)
- `propose_and_commit` is the recommended happy path: proposes, checks conflicts, and commits atomically. If conflicts exist, the proposal is auto-withdrawn and conflicts returned.
- `commit_proposal` does NOT re-check conflicts; the LLM decides when to check
- `load_google_calendar` accepts the Google Calendar API `events.list` format directly; failed events are skipped gracefully
- Conflict detection uses pairwise comparison: O(N*M + N^2) where N=proposed, M=existing

---

## Test-Retest Workflow

The intended usage pattern for an LLM:

1. **Hydrate** — `load_ical` or `load_json` to populate existing events
2. **Query** — `list_events`, `get_free_busy`, `find_available_slots` to understand the schedule
3. **Propose** — `propose_events` with candidate events
4. **Check** — `check_conflicts` to find overlaps
5. **Iterate** — `withdraw_proposal` and `propose_events` again if conflicts found
6. **Commit** — `commit_proposal` when satisfied
7. **Export** — `export_ical` or `export_json` to get the final calendar

---

## Error Handling

```rust
pub enum TempoError {
    CalendarNotFound(String),
    EventNotFound(String),
    ProposalNotFound(String),
    InvalidIcal(String),
    InvalidRrule(String),
    InvalidTimeRange(String),
    InvalidInput(String),
}
```

Not-found variants (`CalendarNotFound`, `EventNotFound`, `ProposalNotFound`) map to MCP `RESOURCE_NOT_FOUND` (`-32002`). All other variants map to `INVALID_PARAMS` (`-32602`).

---

## Implementation Order

1. Bootstrap — CLAUDE.md + skill symlinks
2. `cargo init` + Cargo.toml
3. `error.rs`
4. `calendar/event.rs` + `calendar/time_utils.rs` — with unit tests
5. `calendar/mod.rs` + `calendar/proposal.rs` — with unit tests
6. `ical_bridge.rs` — with unit tests
7. `server.rs` + `main.rs` — wire everything together
8. Integration tests
9. End-to-end verification

---

## Evals Framework

### Purpose

Demonstrate that Tempo MCP materially improves LLM scheduling accuracy and speed. Compare a bare model (no tools) against the same model equipped with Tempo on hard scheduling tasks.

### Eval Structure

```
evals/
  scenarios/
    01_simple_scheduling.json    -- Schedule 3 blocks in a mostly-free week
    02_dense_calendar.json       -- Schedule 6 blocks around 20+ existing meetings
    03_recurring_conflicts.json  -- Navigate around daily standups, weekly 1:1s
    04_multi_calendar.json       -- Work + personal calendar, respect both
    05_constraint_heavy.json     -- "Only mornings", "not back-to-back", "at least 1hr lunch"
  harness.py                     -- Eval runner (calls Claude API)
  scoring.py                     -- Automated scoring logic
  results/                       -- Output directory for run results
```

### Scenario Format

Each scenario JSON contains:

```json
{
  "name": "Dense calendar with 6 new blocks",
  "description": "Schedule six 1-hour focus blocks across Mon-Fri...",
  "existing_events_ical": "BEGIN:VCALENDAR...",
  "task_prompt": "Schedule six 1-hour focus blocks this week (Mon Jan 20 - Fri Jan 24). Avoid conflicts with existing meetings. Prefer mornings. No back-to-back blocks.",
  "constraints": {
    "required_block_count": 6,
    "min_duration_minutes": 60,
    "date_range": ["2025-01-20T00:00:00Z", "2025-01-24T23:59:59Z"],
    "no_conflicts": true,
    "custom_rules": [
      {"type": "prefer_time_range", "start_hour": 8, "end_hour": 12},
      {"type": "no_back_to_back"}
    ]
  },
  "difficulty": "hard"
}
```

### Eval Modes

1. **Bare model** — The LLM receives the task prompt, existing events as text, and must output a JSON schedule. No tools available. Must do all conflict reasoning in-context.

2. **With Tempo MCP** — The LLM has access to all Tempo tools. It loads events, queries free/busy, proposes, checks conflicts, and iterates until satisfied.

### Scoring Dimensions

| Dimension | Weight | How scored |
|-----------|--------|------------|
| **Correctness** | 40% | No conflicts with existing events (binary per block) |
| **Completeness** | 25% | All requested blocks actually scheduled |
| **Constraint adherence** | 20% | Custom rules satisfied (preferences, timing) |
| **Efficiency** | 15% | Token usage and wall-clock time |

### Scoring Logic (`scoring.py`)

- Parse the LLM's output (JSON schedule or committed Tempo events)
- Load existing events + proposed schedule into Tempo
- Run `check_conflicts` to verify zero overlaps (ground truth)
- Check each constraint rule programmatically
- Compute composite score 0-100

### Expected Results

The hypothesis is that the bare model will:
- Miss conflicts (especially with recurring events expanded into the query range)
- Fail at higher density (> 15 existing events)
- Take more tokens/time for the same accuracy
- Degrade sharply on constraint-heavy scenarios

With Tempo:
- Perfect conflict detection (tool-verified, not hallucinated)
- Scale to any calendar density (expansion happens server-side)
- Faster convergence via propose → check → iterate loop
- Constraints validated rather than assumed
