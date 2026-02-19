"""Eval harness: runs scheduling tasks with bare model and model+Tempo MCP.

Connects to the Tempo MCP server as a subprocess, gets tool definitions,
and runs Claude with/without tools to compare scheduling accuracy.
"""

import asyncio
import json
import logging
import os
import time
from dataclasses import dataclass, field
from datetime import datetime
from pathlib import Path
from typing import Any

import anthropic
from mcp import ClientSession, StdioServerParameters
from mcp.client.stdio import stdio_client

from scoring import ExistingEvent, ScheduledBlock, ScoreBreakdown, parse_iso, score_scenario

logger = logging.getLogger(__name__)

TEMPO_BINARY = os.environ.get(
    "TEMPO_BINARY",
    str(Path(__file__).parent.parent / "target" / "release" / "tempo-mcp"),
)

MODEL = os.environ.get("EVAL_MODEL", "claude-sonnet-4-20250514")
MAX_TOKENS = 8192
MAX_TOOL_ROUNDS = 30


@dataclass
class EvalResult:
    scenario_name: str
    mode: str  # "bare" or "mcp"
    score: ScoreBreakdown = field(default_factory=ScoreBreakdown)
    blocks: list[dict] = field(default_factory=list)
    wall_time_seconds: float = 0.0
    input_tokens: int = 0
    output_tokens: int = 0
    tool_calls: int = 0
    api_rounds: int = 0
    raw_response: str = ""
    error: str | None = None


def load_scenario(path: str) -> dict:
    with open(path) as f:
        return json.load(f)


TIMEZONES = ["America/New_York", "America/Chicago", "America/Los_Angeles", "Europe/London"]

VTIMEZONE_BLOCKS = {
    "America/New_York": (
        "BEGIN:VTIMEZONE\nTZID:America/New_York\n"
        "BEGIN:STANDARD\nDTSTART:19701101T020000\nRRULE:FREQ=YEARLY;BYMONTH=11;BYDAY=1SU\n"
        "TZOFFSETFROM:-0400\nTZOFFSETTO:-0500\nTZNAME:EST\nEND:STANDARD\n"
        "BEGIN:DAYLIGHT\nDTSTART:19700308T020000\nRRULE:FREQ=YEARLY;BYMONTH=3;BYDAY=2SU\n"
        "TZOFFSETFROM:-0500\nTZOFFSETTO:-0400\nTZNAME:EDT\nEND:DAYLIGHT\n"
        "END:VTIMEZONE"
    ),
    "America/Chicago": (
        "BEGIN:VTIMEZONE\nTZID:America/Chicago\n"
        "BEGIN:STANDARD\nDTSTART:19701101T020000\nRRULE:FREQ=YEARLY;BYMONTH=11;BYDAY=1SU\n"
        "TZOFFSETFROM:-0500\nTZOFFSETTO:-0600\nTZNAME:CST\nEND:STANDARD\n"
        "BEGIN:DAYLIGHT\nDTSTART:19700308T020000\nRRULE:FREQ=YEARLY;BYMONTH=3;BYDAY=2SU\n"
        "TZOFFSETFROM:-0600\nTZOFFSETTO:-0500\nTZNAME:CDT\nEND:DAYLIGHT\n"
        "END:VTIMEZONE"
    ),
    "America/Los_Angeles": (
        "BEGIN:VTIMEZONE\nTZID:America/Los_Angeles\n"
        "BEGIN:STANDARD\nDTSTART:19701101T020000\nRRULE:FREQ=YEARLY;BYMONTH=11;BYDAY=1SU\n"
        "TZOFFSETFROM:-0700\nTZOFFSETTO:-0800\nTZNAME:PST\nEND:STANDARD\n"
        "BEGIN:DAYLIGHT\nDTSTART:19700308T020000\nRRULE:FREQ=YEARLY;BYMONTH=3;BYDAY=2SU\n"
        "TZOFFSETFROM:-0800\nTZOFFSETTO:-0700\nTZNAME:PDT\nEND:DAYLIGHT\n"
        "END:VTIMEZONE"
    ),
    "Europe/London": (
        "BEGIN:VTIMEZONE\nTZID:Europe/London\n"
        "BEGIN:STANDARD\nDTSTART:19701025T020000\nRRULE:FREQ=YEARLY;BYMONTH=10;BYDAY=-1SU\n"
        "TZOFFSETFROM:+0100\nTZOFFSETTO:+0000\nTZNAME:GMT\nEND:STANDARD\n"
        "BEGIN:DAYLIGHT\nDTSTART:19700329T010000\nRRULE:FREQ=YEARLY;BYMONTH=3;BYDAY=-1SU\n"
        "TZOFFSETFROM:+0000\nTZOFFSETTO:+0100\nTZNAME:BST\nEND:DAYLIGHT\n"
        "END:VTIMEZONE"
    ),
}

ATTENDEES = [
    "alice@company.com", "bob@company.com", "charlie@company.com",
    "diana@company.com", "eve@company.com", "frank@company.com",
    "grace@company.com", "heidi@company.com",
]

DESCRIPTIONS = [
    "Weekly sync to align on priorities and blockers.",
    "Please review the attached agenda before the meeting.",
    "Zoom link: https://zoom.us/j/123456789\\nDial-in: +1-555-0100",
    "Conference Room B - 3rd Floor",
    "Action items from last week will be reviewed.",
    "Bring your laptop for the live demo portion.",
    "",
]

LOCATIONS = [
    "Conference Room A", "Conference Room B", "Zoom",
    "Google Meet", "Room 301", "Main Boardroom", "",
]


def add_realistic_noise(ical_data: str, seed: int = 42) -> str:
    """Add realistic calendar noise to clean iCal data.

    Simulates what real calendar APIs (Google Calendar, Outlook) produce:
    VTIMEZONE blocks, UIDs, CREATED/LAST-MODIFIED timestamps, ATTENDEE,
    DESCRIPTION, LOCATION, VALARM, SEQUENCE, STATUS, ORGANIZER fields.
    Some events get converted from UTC to local timezone representation.
    """
    import hashlib
    import random

    rng = random.Random(seed)
    lines = ical_data.replace("\r\n", "\n").split("\n")
    output = []
    used_tzs: set[str] = set()
    event_idx = 0
    in_event = False
    event_lines: list[str] = []

    for line in lines:
        stripped = line.strip()

        if stripped == "BEGIN:VCALENDAR":
            output.append(stripped)
            output.append("PRODID:-//Google Inc//Google Calendar 70.9054//EN")
            output.append("VERSION:2.0")
            output.append("CALSCALE:GREGORIAN")
            output.append("METHOD:PUBLISH")
            output.append("X-WR-CALNAME:Work Calendar")
            output.append("X-WR-TIMEZONE:America/New_York")
            continue
        elif stripped == "VERSION:2.0":
            continue  # already added above
        elif stripped == "BEGIN:VEVENT":
            in_event = True
            event_lines = [stripped]
            continue
        elif stripped == "END:VEVENT" and in_event:
            event_lines.append(stripped)
            # Process this event with noise
            noisy = _noisify_event(event_lines, event_idx, rng, used_tzs)
            output.extend(noisy)
            event_idx += 1
            in_event = False
            continue
        elif in_event:
            event_lines.append(stripped)
            continue
        elif stripped == "END:VCALENDAR":
            # Insert VTIMEZONE blocks before closing
            for tz in sorted(used_tzs):
                if tz in VTIMEZONE_BLOCKS:
                    output.extend(VTIMEZONE_BLOCKS[tz].split("\n"))
            output.append(stripped)
            continue
        else:
            output.append(stripped)

    return "\n".join(output)


def _noisify_event(
    event_lines: list[str], idx: int, rng: random.Random, used_tzs: set[str]
) -> list[str]:
    """Add realistic noise fields to a single VEVENT block."""
    import hashlib

    result = ["BEGIN:VEVENT"]

    # Generate a realistic UID
    uid_hash = hashlib.md5(f"event-{idx}-{rng.randint(0, 99999)}".encode()).hexdigest()
    result.append(f"UID:{uid_hash[:8]}-{uid_hash[8:12]}-{uid_hash[12:16]}-{uid_hash[16:20]}-{uid_hash[20:32]}@google.com")

    # Add CREATED and LAST-MODIFIED
    created_day = rng.randint(1, 15)
    result.append(f"CREATED:20250{rng.randint(1,9):01d}{created_day:02d}T{rng.randint(8,18):02d}{rng.randint(0,59):02d}00Z")
    result.append(f"LAST-MODIFIED:20250115T{rng.randint(8,18):02d}{rng.randint(0,59):02d}00Z")
    result.append(f"SEQUENCE:{rng.randint(0, 3)}")
    result.append("STATUS:CONFIRMED")
    result.append("TRANSP:OPAQUE")

    # Decide whether to convert this event to a local timezone (30% chance)
    convert_tz = rng.random() < 0.3
    chosen_tz = rng.choice(TIMEZONES) if convert_tz else None
    has_rrule = any("RRULE:" in l for l in event_lines)

    for line in event_lines:
        if line.startswith("BEGIN:VEVENT") or line.startswith("END:VEVENT"):
            continue

        # Convert UTC times to local timezone representation for some events
        # But NOT for events with RRULE (that would break expansion semantics)
        if chosen_tz and not has_rrule and (line.startswith("DTSTART:") or line.startswith("DTEND:")):
            # Convert DTSTART:20250120T090000Z → DTSTART;TZID=America/New_York:20250120T040000
            key = line.split(":")[0]
            val = line.split(":", 1)[1]
            if val.endswith("Z"):
                # Parse UTC time, convert to local
                from datetime import datetime, timezone, timedelta
                utc_str = val.rstrip("Z")
                utc_dt = datetime.strptime(utc_str, "%Y%m%dT%H%M%S").replace(tzinfo=timezone.utc)
                # Apply offset (simplified: Jan is standard time)
                offsets = {
                    "America/New_York": -5, "America/Chicago": -6,
                    "America/Los_Angeles": -8, "Europe/London": 0,
                }
                offset_h = offsets.get(chosen_tz, 0)
                local_dt = utc_dt + timedelta(hours=offset_h)
                result.append(f"{key};TZID={chosen_tz}:{local_dt.strftime('%Y%m%dT%H%M%S')}")
                used_tzs.add(chosen_tz)
                continue

        result.append(line)

    # Add noisy fields
    if rng.random() < 0.6:
        desc = rng.choice(DESCRIPTIONS)
        if desc:
            result.append(f"DESCRIPTION:{desc}")
    if rng.random() < 0.5:
        loc = rng.choice(LOCATIONS)
        if loc:
            result.append(f"LOCATION:{loc}")
    if rng.random() < 0.4:
        organizer = rng.choice(ATTENDEES)
        result.append(f"ORGANIZER;CN={organizer.split('@')[0].title()}:mailto:{organizer}")
    # Add 1-3 attendees
    num_attendees = rng.randint(1, 3)
    for _ in range(num_attendees):
        att = rng.choice(ATTENDEES)
        status = rng.choice(["ACCEPTED", "TENTATIVE", "NEEDS-ACTION"])
        result.append(f"ATTENDEE;CUTYPE=INDIVIDUAL;ROLE=REQ-PARTICIPANT;PARTSTAT={status};CN={att.split('@')[0].title()};X-NUM-GUESTS=0:mailto:{att}")
    # Add VALARM (reminder)
    if rng.random() < 0.5:
        result.append("BEGIN:VALARM")
        result.append("ACTION:DISPLAY")
        result.append(f"TRIGGER:-PT{rng.choice([5, 10, 15, 30])}M")
        result.append("DESCRIPTION:Reminder")
        result.append("END:VALARM")

    result.append("END:VEVENT")
    return result


def format_calendar_as_text(ical_data: str) -> str:
    """Convert iCal to a human-readable text block for bare-model prompts."""
    lines = []
    current_event: dict[str, str] = {}
    for line in ical_data.replace("\r\n", "\n").split("\n"):
        line = line.strip()
        if line == "BEGIN:VEVENT":
            current_event = {}
        elif line == "END:VEVENT":
            summary = current_event.get("SUMMARY", "Untitled")
            start = current_event.get("DTSTART", "?")
            end = current_event.get("DTEND", "?")
            rrule = current_event.get("RRULE", "")
            # Format datetime for readability
            try:
                s = parse_iso(start.replace("Z", "+00:00") if "T" in start else start)
                e = parse_iso(end.replace("Z", "+00:00") if "T" in end else end)
                time_str = f"{s.strftime('%a %b %d %H:%M')}-{e.strftime('%H:%M')} UTC"
            except Exception:
                time_str = f"{start} - {end}"
            entry = f"  - {summary}: {time_str}"
            if rrule:
                entry += f" (recurring: {rrule})"
            lines.append(entry)
        elif ":" in line and current_event is not None:
            key, _, val = line.partition(":")
            # Strip params like DTSTART;VALUE=DATE
            key = key.split(";")[0]
            current_event[key] = val
    return "\n".join(lines) if lines else "(no events)"


def expand_recurring_for_scoring(ical_data: str, range_start: str, range_end: str) -> list[ExistingEvent]:
    """Expand iCal events (including RRULE) into concrete occurrences for scoring.

    Uses a simple Python expansion since we need this for bare-model scoring
    where Tempo isn't involved.
    """
    from dateutil.rrule import rrulestr

    events: list[ExistingEvent] = []
    rs = parse_iso(range_start)
    re_ = parse_iso(range_end)

    current: dict[str, str] = {}
    for line in ical_data.replace("\r\n", "\n").split("\n"):
        line = line.strip()
        if line == "BEGIN:VEVENT":
            current = {}
        elif line == "END:VEVENT":
            summary = current.get("SUMMARY", "Untitled")
            dtstart_str = current.get("DTSTART", "")
            dtend_str = current.get("DTEND", "")
            rrule_str = current.get("RRULE", "")

            try:
                start = parse_iso(dtstart_str)
                end = parse_iso(dtend_str)
            except Exception:
                continue

            duration = end - start

            if rrule_str:
                try:
                    rule = rrulestr(
                        f"RRULE:{rrule_str}",
                        dtstart=start.replace(tzinfo=None),
                    )
                    for occ in rule.between(
                        rs.replace(tzinfo=None),
                        re_.replace(tzinfo=None),
                        inc=True,
                    ):
                        from datetime import timezone
                        occ_start = occ.replace(tzinfo=timezone.utc)
                        events.append(ExistingEvent(
                            title=summary,
                            start=occ_start,
                            end=occ_start + duration,
                        ))
                except Exception as exc:
                    logger.warning("Failed to expand RRULE for %s: %s", summary, exc)
                    # Fall back to single occurrence
                    if rs <= start <= re_ or rs <= end <= re_:
                        events.append(ExistingEvent(title=summary, start=start, end=end))
            else:
                if rs <= start <= re_ or rs <= end <= re_:
                    events.append(ExistingEvent(title=summary, start=start, end=end))
        elif ":" in line:
            key, _, val = line.partition(":")
            key = key.split(";")[0]
            current[key] = val

    return events


def expand_gcal_json_for_scoring(gcal_data: dict, range_start: str, range_end: str) -> list[ExistingEvent]:
    """Expand Google Calendar JSON events into ExistingEvent objects for scoring.

    Parses the nested start.dateTime/end.dateTime format with timezone offsets
    and converts to UTC-aware datetimes.
    """
    events: list[ExistingEvent] = []
    rs = parse_iso(range_start)
    re_ = parse_iso(range_end)

    for gcal_event in gcal_data.get("events", []):
        summary = gcal_event.get("summary", "Untitled")
        start_obj = gcal_event.get("start", {})
        end_obj = gcal_event.get("end", {})

        start_str = start_obj.get("dateTime") or start_obj.get("date")
        end_str = end_obj.get("dateTime") or end_obj.get("date")

        if not start_str or not end_str:
            continue

        try:
            start = parse_iso(start_str)
            end = parse_iso(end_str)
        except Exception:
            continue

        if rs <= start <= re_ or rs <= end <= re_:
            events.append(ExistingEvent(title=summary, start=start, end=end))

    return events


def parse_blocks_from_text(text: str) -> list[ScheduledBlock]:
    """Extract scheduled blocks from Claude's bare-model text response.

    Looks for JSON in the response, either a top-level array or an object
    with a schedule/blocks key.
    """
    # Try to find JSON in the response
    import re

    # Look for JSON code blocks first
    json_match = re.search(r"```(?:json)?\s*\n(.*?)\n```", text, re.DOTALL)
    if json_match:
        raw = json_match.group(1)
    else:
        # Try to find a JSON array or object directly
        for start_char, end_char in [("[", "]"), ("{", "}")]:
            idx = text.find(start_char)
            if idx >= 0:
                # Find matching bracket
                depth = 0
                for i in range(idx, len(text)):
                    if text[i] == start_char:
                        depth += 1
                    elif text[i] == end_char:
                        depth -= 1
                    if depth == 0:
                        raw = text[idx:i+1]
                        break
                else:
                    continue
                break
        else:
            return []

    try:
        data = json.loads(raw)
    except json.JSONDecodeError:
        return []

    # Normalize: could be a list or an object with a key
    if isinstance(data, dict):
        for key in ["schedule", "blocks", "events", "focus_blocks"]:
            if key in data and isinstance(data[key], list):
                data = data[key]
                break
        else:
            data = [data]

    if not isinstance(data, list):
        return []

    blocks = []
    for item in data:
        if not isinstance(item, dict):
            continue
        title = item.get("title", item.get("name", item.get("summary", "Focus Block")))
        start_str = item.get("start", item.get("start_time", ""))
        end_str = item.get("end", item.get("end_time", ""))
        if not start_str or not end_str:
            continue
        try:
            blocks.append(ScheduledBlock(
                title=str(title),
                start=parse_iso(str(start_str)),
                end=parse_iso(str(end_str)),
            ))
        except Exception:
            continue

    return blocks


# -- Bare model eval --

def run_bare_eval(scenario: dict, client: anthropic.Anthropic) -> EvalResult:
    """Run a scheduling task with the bare model (no tools)."""
    result = EvalResult(scenario_name=scenario["name"], mode="bare")

    # Pass noisy raw iCal — simulates what a real calendar API returns
    # (VTIMEZONE blocks, mixed timezones, UIDs, attendees, descriptions, etc.)
    noisy_ical = add_realistic_noise(scenario["existing_events_ical"], seed=42)
    ical_sections = f"Work calendar (iCal format):\n```\n{noisy_ical}\n```"
    if "existing_events_personal_ical" in scenario:
        noisy_personal = add_realistic_noise(scenario["existing_events_personal_ical"], seed=43)
        ical_sections += f"\n\nPersonal calendar (iCal format):\n```\n{noisy_personal}\n```"
    if "existing_events_shared_ical" in scenario:
        noisy_shared = add_realistic_noise(scenario["existing_events_shared_ical"], seed=44)
        ical_sections += f"\n\nShared team calendar (iCal format):\n```\n{noisy_shared}\n```"
    if "existing_events_gcal_json" in scenario:
        gcal_raw = json.dumps(scenario["existing_events_gcal_json"], indent=2)
        ical_sections += f"\n\nPersonal calendar (Google Calendar API JSON format):\n```json\n{gcal_raw}\n```"

    prompt = f"""Here is my current calendar data:

{ical_sections}

{scenario['task_prompt']}

IMPORTANT: Return your proposed schedule as a JSON array of objects, each with "title" (string), "start" (ISO 8601 with Z suffix), and "end" (ISO 8601 with Z suffix). Put the JSON in a ```json code block."""

    t0 = time.time()
    try:
        response = client.messages.create(
            model=MODEL,
            max_tokens=MAX_TOKENS,
            messages=[{"role": "user", "content": prompt}],
        )
        result.wall_time_seconds = time.time() - t0
        result.input_tokens = response.usage.input_tokens
        result.output_tokens = response.usage.output_tokens
        result.api_rounds = 1

        text = response.content[0].text if response.content else ""
        result.raw_response = text
        blocks = parse_blocks_from_text(text)
        result.blocks = [{"title": b.title, "start": b.start.isoformat(), "end": b.end.isoformat()} for b in blocks]

        # Score
        constraints = scenario["constraints"]
        existing = expand_recurring_for_scoring(
            scenario["existing_events_ical"],
            constraints.get("date_range_start", "2025-01-20T00:00:00Z"),
            constraints.get("date_range_end", "2025-01-24T23:59:59Z"),
        )
        for extra_key in ["existing_events_personal_ical", "existing_events_shared_ical"]:
            if extra_key in scenario:
                existing += expand_recurring_for_scoring(
                    scenario[extra_key],
                    constraints.get("date_range_start", "2025-01-20T00:00:00Z"),
                    constraints.get("date_range_end", "2025-01-24T23:59:59Z"),
                )
        if "existing_events_gcal_json" in scenario:
            existing += expand_gcal_json_for_scoring(
                scenario["existing_events_gcal_json"],
                constraints.get("date_range_start", "2025-01-20T00:00:00Z"),
                constraints.get("date_range_end", "2025-01-24T23:59:59Z"),
            )

        result.score = score_scenario(blocks, existing, constraints)

    except Exception as e:
        result.wall_time_seconds = time.time() - t0
        result.error = str(e)

    return result


# -- MCP model eval --

async def run_mcp_eval(scenario: dict, client: anthropic.Anthropic) -> EvalResult:
    """Run a scheduling task with Claude + Tempo MCP tools."""
    result = EvalResult(scenario_name=scenario["name"], mode="mcp")

    server_params = StdioServerParameters(
        command=TEMPO_BINARY,
        args=[],
        env=None,
    )

    t0 = time.time()
    try:
        async with stdio_client(server_params) as (read, write):
            async with ClientSession(read, write) as session:
                await session.initialize()

                # Get tools from Tempo
                tools_response = await session.list_tools()
                anthropic_tools = [
                    {
                        "name": tool.name,
                        "description": tool.description or "",
                        "input_schema": tool.inputSchema,
                    }
                    for tool in tools_response.tools
                ]

                # Build the prompt — tell the model to use the tools
                calendar_names = ["default"]
                load_instructions_parts = ["Load my calendar using load_ical."]
                if "existing_events_personal_ical" in scenario:
                    calendar_names = ["work", "personal"]
                    load_instructions_parts = [
                        "Load my work calendar using load_ical with calendar_name='work'.",
                        "Load my personal calendar using load_ical with calendar_name='personal'.",
                    ]
                if "existing_events_shared_ical" in scenario:
                    calendar_names.append("shared")
                    load_instructions_parts.append(
                        "Load the shared team calendar using load_ical with calendar_name='shared'."
                    )
                if "existing_events_gcal_json" in scenario:
                    calendar_names.append("personal_gcal")
                    load_instructions_parts.append(
                        "Load my personal Google Calendar using load_google_calendar with "
                        "calendar_name='personal_gcal'. The Google Calendar JSON data is provided below."
                    )

                load_instructions = (
                    "First, load all calendars (you can make multiple load calls in a single response):\n"
                    + "\n".join(f"- {part}" for part in load_instructions_parts)
                )

                load_instructions += (
                    "\n\nThen follow this workflow:\n"
                    "1. Use find_available_slots (with buffer_minutes for travel time if needed) to find open windows\n"
                    "2. Use propose_and_commit to propose, check conflicts, and commit in a single call\n"
                    "3. If propose_and_commit reports conflicts, adjust times and retry\n"
                    "\nIMPORTANT: Use the EXACT start/end times returned by find_available_slots. "
                    "Do not adjust or invent your own times."
                )

                ical_content = f"Work calendar iCal data:\n```\n{scenario['existing_events_ical']}\n```"
                if "existing_events_personal_ical" in scenario:
                    ical_content += f"\n\nPersonal calendar iCal data:\n```\n{scenario['existing_events_personal_ical']}\n```"
                if "existing_events_shared_ical" in scenario:
                    ical_content += f"\n\nShared team calendar iCal data:\n```\n{scenario['existing_events_shared_ical']}\n```"
                if "existing_events_gcal_json" in scenario:
                    gcal_json_str = json.dumps(scenario["existing_events_gcal_json"], indent=2)
                    ical_content += f"\n\nPersonal Google Calendar JSON data (pass this to load_google_calendar):\n```json\n{gcal_json_str}\n```"

                prompt = f"""{scenario['task_prompt']}

{load_instructions}

{ical_content}"""

                messages: list[dict[str, Any]] = [{"role": "user", "content": prompt}]

                # Conversation loop
                for round_num in range(MAX_TOOL_ROUNDS):
                    response = client.messages.create(
                        model=MODEL,
                        max_tokens=MAX_TOKENS,
                        messages=messages,
                        tools=anthropic_tools,
                    )

                    result.input_tokens += response.usage.input_tokens
                    result.output_tokens += response.usage.output_tokens
                    result.api_rounds += 1

                    if response.stop_reason == "end_turn":
                        # Extract final text
                        for block in response.content:
                            if hasattr(block, "text"):
                                result.raw_response += block.text
                        break

                    # Process tool calls
                    messages.append({"role": "assistant", "content": response.content})
                    tool_results = []

                    for block in response.content:
                        if block.type == "tool_use":
                            result.tool_calls += 1
                            try:
                                mcp_result = await session.call_tool(block.name, block.input)
                                tool_text = ""
                                for content in mcp_result.content:
                                    if hasattr(content, "text"):
                                        tool_text += content.text
                                tool_results.append({
                                    "type": "tool_result",
                                    "tool_use_id": block.id,
                                    "content": tool_text,
                                })
                            except Exception as e:
                                tool_results.append({
                                    "type": "tool_result",
                                    "tool_use_id": block.id,
                                    "content": f"Error: {e}",
                                    "is_error": True,
                                })

                    if tool_results:
                        messages.append({"role": "user", "content": tool_results})
                    else:
                        break

                result.wall_time_seconds = time.time() - t0

                # Extract blocks from tool call results
                # Look through messages for committed events or export_json results
                blocks = extract_blocks_from_conversation(messages)
                result.blocks = [
                    {"title": b.title, "start": b.start.isoformat(), "end": b.end.isoformat()}
                    for b in blocks
                ]

                # Score
                constraints = scenario["constraints"]
                existing = expand_recurring_for_scoring(
                    scenario["existing_events_ical"],
                    constraints.get("date_range_start", "2025-01-20T00:00:00Z"),
                    constraints.get("date_range_end", "2025-01-24T23:59:59Z"),
                )
                for extra_key in ["existing_events_personal_ical", "existing_events_shared_ical"]:
                    if extra_key in scenario:
                        existing += expand_recurring_for_scoring(
                            scenario[extra_key],
                            constraints.get("date_range_start", "2025-01-20T00:00:00Z"),
                            constraints.get("date_range_end", "2025-01-24T23:59:59Z"),
                        )
                if "existing_events_gcal_json" in scenario:
                    existing += expand_gcal_json_for_scoring(
                        scenario["existing_events_gcal_json"],
                        constraints.get("date_range_start", "2025-01-20T00:00:00Z"),
                        constraints.get("date_range_end", "2025-01-24T23:59:59Z"),
                    )

                result.score = score_scenario(blocks, existing, constraints)

    except Exception as e:
        result.wall_time_seconds = time.time() - t0
        result.error = str(e)
        logger.exception("MCP eval failed for %s", scenario["name"])

    return result


def extract_blocks_from_conversation(messages: list[dict]) -> list[ScheduledBlock]:
    """Extract scheduled blocks from the MCP conversation.

    Uses the LAST propose_events or propose_and_commit call only, since earlier
    proposals may have been withdrawn and retried. If the model proposed → withdrew
    → re-proposed, only the final proposal represents the committed schedule.
    """
    last_proposal_events: list[dict] = []

    for msg in messages:
        if msg.get("role") != "assistant":
            continue
        content = msg.get("content", [])
        if not isinstance(content, list):
            continue

        for item in content:
            # Assistant content blocks are pydantic objects from anthropic SDK
            item_type = getattr(item, "type", None) or (item.get("type") if isinstance(item, dict) else None)
            if item_type != "tool_use":
                continue
            item_name = getattr(item, "name", None) or (item.get("name") if isinstance(item, dict) else None)
            if item_name not in ("propose_events", "propose_and_commit"):
                continue
            item_input = getattr(item, "input", None) or (item.get("input", {}) if isinstance(item, dict) else {})

            # Replace — only keep the last proposal
            last_proposal_events = item_input.get("events", [])

    blocks: list[ScheduledBlock] = []
    for event in last_proposal_events:
        if not isinstance(event, dict):
            continue
        start_str = event.get("start", "")
        end_str = event.get("end", "")
        if not start_str or not end_str:
            continue
        try:
            blocks.append(ScheduledBlock(
                title=event.get("title", "Focus Block"),
                start=parse_iso(str(start_str)),
                end=parse_iso(str(end_str)),
            ))
        except Exception:
            continue

    return blocks
