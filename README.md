# Tempo

**A lightning-fast, in-memory calendar MCP server that gives LLMs superpowers for scheduling.**

LLMs aren't great at complex scheduling. They hallucinate conflicts, lose track of recurring events, and can't reliably reason about overlapping time ranges. Tempo fixes this by giving any MCP-compatible model a structured **propose-check-commit** workflow — so it can load real calendars, find open slots, propose events, verify zero conflicts, and only commit when everything checks out. Runs as just a single Rust binary over stdio.

## Why Tempo?

Scheduling is deceptively hard for LLMs:

- **Dense calendars** with 20+ events cause context overload and missed conflicts
- **Recurring events** (daily standups, weekly 1:1s) need to be expanded to check for overlaps
- **Multi-calendar awareness** (work + personal) means reasoning across separate event sets
- **Buffer time** (travel, context-switching) adds invisible constraints

Tempo offloads all of this to a purpose-built engine. The model calls tools instead of doing mental math, and gets deterministic answers instead of best guesses.

## Quickstart

### Build

```sh
cargo build --release
```

### Add to your MCP client

<details>
<summary><b>Claude Desktop</b></summary>

Add to `~/Library/Application Support/Claude/claude_desktop_config.json` (macOS) or `%APPDATA%\Claude\claude_desktop_config.json` (Windows):

```json
{
  "mcpServers": {
    "tempo": {
      "command": "/absolute/path/to/tempo-mcp/target/release/tempo-mcp"
    }
  }
}
```
</details>

<details>
<summary><b>Claude Code</b></summary>

```sh
claude mcp add tempo /absolute/path/to/tempo-mcp/target/release/tempo-mcp
```
</details>

### Use it

> "Here's my calendar for this week (paste iCal or JSON). Schedule three 1-hour focus blocks, avoiding conflicts, with 15 minutes buffer between meetings."

Tempo handles the rest — the model will load your events, find open slots, and propose a conflict-free schedule.

## How It Works

Tempo enables a **test-retest workflow** where models iterate on scheduling decisions before committing:

```
Load calendars ──> Query availability ──> Propose events ──> Check conflicts ──> Commit
       │                                       │                    │
       │                                       └── Adjust & retry ──┘
       │
  iCal / JSON / Google Calendar
```

**Proposals are first-class**. The model can create candidate schedules, check them against existing events, and only commit when satisfied — without modifying anything until it's ready.

### Recommended 3-step workflow

1. **Load** all calendars with `load_ical`, `load_json`, or `load_google_calendar` (parallel calls OK)
2. **Find slots** with `find_available_slots` using `buffer_minutes` for travel/transition time
3. **Commit** with `propose_and_commit` — proposes, conflict-checks, and commits atomically

## Tools

### Hydration — Load existing events

| Tool | Description |
|------|-------------|
| `load_ical` | Load events from iCal/ICS format (supports RRULE recurrence) |
| `load_json` | Load events from a JSON array |
| `load_google_calendar` | Load events directly from Google Calendar API format |

### Querying — Understand the schedule

| Tool | Description |
|------|-------------|
| `list_events` | List all occurrences in a time range (recurring events expanded) |
| `get_free_busy` | Get busy/free periods with totals |
| `find_available_slots` | Find open slots of a given duration, with optional buffer time |

### Proposals — Schedule with confidence

| Tool | Description |
|------|-------------|
| `propose_events` | Create a named proposal without committing |
| `check_conflicts` | Verify a proposal against existing events |
| `list_proposals` | See all pending proposals |
| `withdraw_proposal` | Discard a proposal |
| `propose_and_commit` | Propose + conflict-check + commit in one step **(recommended)** |

### Mutations — Direct calendar edits

| Tool | Description |
|------|-------------|
| `commit_proposal` | Commit a previously-created proposal |
| `add_event` | Add a single event directly (bypasses proposal workflow) |
| `remove_event` | Remove an event by ID |
| `clear_calendar` | Remove all events from a calendar |

### Export — Get results out

| Tool | Description |
|------|-------------|
| `export_ical` | Export as iCal/ICS string |
| `export_json` | Export as JSON array |

## Multi-Calendar Support

Tempo supports multiple named calendars (e.g., "work", "personal"). Calendar names are case-insensitive and auto-created on first use.

- **Read operations** with no calendar specified query across **all** calendars
- **Write operations** default to the `"default"` calendar
- Conflict checks can span all calendars, so a proposal is verified against your entire schedule

## Development

```sh
cargo build          # compile (debug)
cargo build --release  # compile (optimized — use this for evals)
cargo test           # run all 44 unit tests
cargo clippy         # lint (zero warnings)
```

## Evals

Tempo ships with a benchmark suite that compares bare Claude vs. Claude + Tempo on hard scheduling tasks. Seven scenarios test increasing complexity — from a light week with 5 meetings to dense multi-calendar setups with recurring events and constraints.

### Running evals

```sh
# Setup
cd evals
python -m venv .venv && source .venv/bin/activate
pip install -e .
cp ../.env.example ../.env  # add your ANTHROPIC_API_KEY

# Build the server (evals require the release binary)
cargo build --release

# Run
python run_eval.py                     # all scenarios, bare vs. MCP
python run_eval.py --mode mcp          # MCP only
python run_eval.py --scenario 02       # specific scenario
python run_eval.py --model claude-sonnet-4-20250514
```

### Scoring

Each run is scored on a 0-100 composite:

| Dimension | Weight | What it measures |
|-----------|--------|-----------------|
| Correctness | 40% | Zero conflicts with existing events |
| Completeness | 25% | All requested blocks actually scheduled |
| Constraint adherence | 20% | Custom rules satisfied (time windows, buffers, spread) |
| Efficiency | 15% | Bonus for perfect correctness + completeness |

### Results

Bare Claude Sonnet 4 vs. Claude Sonnet 4 + Tempo (composite score, 0–100):

| Scenario | Bare | + Tempo | Conflicts (bare → MCP) |
|----------|-----:|--------:|-----------------------:|
| 01 — Simple scheduling | 100.0 | 100.0 | 0 → 0 |
| 02 — Dense calendar | 45.0 | 98.4 | 7 → 0 |
| 03 — Recurring event maze | 61.0 | 100.0 | 3 → 0 |
| 04 — Multi-calendar | 52.9 | 98.7 | 4 → 0 |
| 05 — Constraint-heavy | 99.8 | 100.0 | 0 → 0 |
| 06 — Real-world Google Cal | 54.7 | 100.0 | 3 → 0 |
| 07 — Pick from options | 71.5 | 100.0 | 1 → 0 |
| **Average** | **69.3** | **99.6** | **18 → 0** |

Tempo eliminates scheduling conflicts entirely and raises the average composite score from 69 to 99.6. The bare model struggles most on dense calendars (scenario 02) and multi-source coordination (scenarios 04, 06) where it must reason across 20+ events.

### Scenarios

| # | Scenario | Difficulty | What it tests |
|---|----------|-----------|---------------|
| 01 | Simple scheduling | Easy | 5 events, schedule 3 focus blocks |
| 02 | Dense calendar | Hard | 20+ events, schedule 6 blocks |
| 03 | Recurring conflicts | Hard | Daily standups, weekly 1:1s |
| 04 | Multi-calendar | Hard | Work + personal calendars |
| 05 | Constraint-heavy | Hard | Time windows, no back-to-back, lunch buffers |
| 06 | Real-world | Hard | Mixed timezones, Google Calendar JSON |
| 07 | Pick from options | Hard | Multiple calendar configurations |

## License

MIT
