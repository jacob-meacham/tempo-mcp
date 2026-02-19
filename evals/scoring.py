"""Scoring logic for Tempo eval scenarios.

Evaluates a proposed schedule against scenario constraints.
Returns a breakdown of scores across multiple dimensions.
"""

from dataclasses import dataclass, field
from datetime import datetime, timedelta, timezone
from typing import Any


@dataclass
class ScheduledBlock:
    title: str
    start: datetime
    end: datetime

    @property
    def duration_minutes(self) -> float:
        return (self.end - self.start).total_seconds() / 60

    @property
    def date_key(self) -> str:
        return self.start.strftime("%Y-%m-%d")


@dataclass
class ExistingEvent:
    title: str
    start: datetime
    end: datetime


@dataclass
class ScoreBreakdown:
    # Core dimensions
    correctness: float = 0.0  # 0-1: no conflicts with existing events
    completeness: float = 0.0  # 0-1: required blocks scheduled
    constraint_adherence: float = 0.0  # 0-1: custom rules satisfied
    # Detail
    total_blocks: int = 0
    required_blocks: int = 0
    conflict_count: int = 0
    constraint_violations: list[str] = field(default_factory=list)

    @property
    def composite_score(self) -> float:
        """Weighted composite: correctness 40%, completeness 25%, constraints 20%, plus 15% bonus for perfection."""
        base = (
            self.correctness * 0.40
            + self.completeness * 0.25
            + self.constraint_adherence * 0.20
        )
        # Bonus 15%: awarded proportionally to overall perfection
        if self.correctness == 1.0 and self.completeness == 1.0:
            base += self.constraint_adherence * 0.15
        return round(base * 100, 1)


def parse_iso(s: str) -> datetime:
    """Parse ISO 8601 datetime string to timezone-aware datetime."""
    s = s.rstrip("Z") + "+00:00" if s.endswith("Z") else s
    return datetime.fromisoformat(s)


def overlaps(a_start: datetime, a_end: datetime, b_start: datetime, b_end: datetime) -> bool:
    """Half-open interval overlap: [start, end)"""
    return a_start < b_end and b_start < a_end


def overlap_minutes(a_start: datetime, a_end: datetime, b_start: datetime, b_end: datetime) -> float:
    o_start = max(a_start, b_start)
    o_end = min(a_end, b_end)
    if o_start < o_end:
        return (o_end - o_start).total_seconds() / 60
    return 0.0


def score_scenario(
    blocks: list[ScheduledBlock],
    existing: list[ExistingEvent],
    constraints: dict[str, Any],
) -> ScoreBreakdown:
    """Score a proposed schedule against a scenario's constraints."""
    result = ScoreBreakdown()
    result.required_blocks = constraints.get("required_block_count", 0)
    result.total_blocks = len(blocks)

    # -- Completeness --
    if result.required_blocks > 0:
        result.completeness = min(1.0, result.total_blocks / result.required_blocks)
    else:
        result.completeness = 1.0

    # -- Correctness: no conflicts with existing events --
    conflict_count = 0
    for block in blocks:
        for event in existing:
            if overlaps(block.start, block.end, event.start, event.end):
                conflict_count += 1
                result.constraint_violations.append(
                    f"CONFLICT: '{block.title}' ({block.start.strftime('%a %H:%M')}-{block.end.strftime('%H:%M')}) "
                    f"overlaps '{event.title}' ({event.start.strftime('%a %H:%M')}-{event.end.strftime('%H:%M')})"
                )
    result.conflict_count = conflict_count
    if len(blocks) > 0:
        result.correctness = max(0.0, 1.0 - conflict_count / len(blocks))
    else:
        result.correctness = 0.0

    # -- Constraint adherence --
    violations = 0
    total_checks = 0

    # Working hours check
    wh = constraints.get("working_hours")
    if wh:
        for block in blocks:
            total_checks += 1
            bh_start = block.start.hour
            bh_end = block.end.hour + (1 if block.end.minute > 0 else 0)
            if bh_start < wh["start_hour"] or bh_end > wh["end_hour"]:
                violations += 1
                result.constraint_violations.append(
                    f"WORKING_HOURS: '{block.title}' at {block.start.strftime('%H:%M')}-{block.end.strftime('%H:%M')} outside {wh['start_hour']}:00-{wh['end_hour']}:00"
                )

    # Duration check
    min_dur = constraints.get("min_duration_minutes")
    max_dur = constraints.get("max_duration_minutes")
    for block in blocks:
        if min_dur is not None:
            total_checks += 1
            if block.duration_minutes < min_dur - 1:  # 1 min tolerance
                violations += 1
                result.constraint_violations.append(
                    f"DURATION: '{block.title}' is {block.duration_minutes:.0f}min, minimum is {min_dur}min"
                )
        if max_dur is not None:
            total_checks += 1
            if block.duration_minutes > max_dur + 1:
                violations += 1
                result.constraint_violations.append(
                    f"DURATION: '{block.title}' is {block.duration_minutes:.0f}min, maximum is {max_dur}min"
                )

    # Date range check
    range_start = constraints.get("date_range_start")
    range_end = constraints.get("date_range_end")
    if range_start and range_end:
        rs = parse_iso(range_start)
        re = parse_iso(range_end)
        for block in blocks:
            total_checks += 1
            if block.start < rs or block.end > re:
                violations += 1
                result.constraint_violations.append(
                    f"DATE_RANGE: '{block.title}' is outside {rs.date()}-{re.date()}"
                )

    # Custom rules
    custom_rules = constraints.get("custom_rules", [])
    for rule in custom_rules:
        rule_type = rule.get("type")

        if rule_type == "time_window":
            for block in blocks:
                total_checks += 1
                if block.start.hour < rule["start_hour"] or block.end.hour > rule["end_hour"]:
                    violations += 1
                    result.constraint_violations.append(
                        f"TIME_WINDOW: '{block.title}' at {block.start.strftime('%H:%M')} outside {rule['start_hour']}:00-{rule['end_hour']}:00"
                    )

        elif rule_type == "min_buffer_minutes":
            buffer = timedelta(minutes=rule["value"])
            all_events = [(e.start, e.end, e.title) for e in existing]
            all_events += [(b.start, b.end, b.title) for b in blocks]
            for block in blocks:
                for ev_start, ev_end, ev_title in all_events:
                    if ev_start == block.start and ev_end == block.end:
                        continue  # skip self
                    total_checks += 1
                    # Check if the gap between block and event is < buffer
                    if block.end <= ev_start:
                        gap = ev_start - block.end
                    elif ev_end <= block.start:
                        gap = block.start - ev_end
                    else:
                        continue  # overlapping, handled by conflict check
                    if gap < buffer:
                        violations += 1
                        result.constraint_violations.append(
                            f"BUFFER: '{block.title}' has only {gap.total_seconds()/60:.0f}min gap to '{ev_title}', need {rule['value']}min"
                        )

        elif rule_type == "max_per_day":
            max_val = rule["value"]
            from collections import Counter
            day_counts = Counter(b.date_key for b in blocks)
            for day, count in day_counts.items():
                total_checks += 1
                if count > max_val:
                    violations += 1
                    result.constraint_violations.append(
                        f"MAX_PER_DAY: {count} blocks on {day}, max is {max_val}"
                    )

        elif rule_type == "spread_across_days":
            total_checks += 1
            unique_days = len(set(b.date_key for b in blocks))
            min_days = rule.get("min_days", 1)
            if unique_days < min_days:
                violations += 1
                result.constraint_violations.append(
                    f"SPREAD: blocks on {unique_days} days, need at least {min_days}"
                )

        elif rule_type == "blocked_dates":
            blocked = set(rule.get("dates", []))
            for block in blocks:
                total_checks += 1
                block_date = block.start.strftime("%Y-%m-%d")
                if block_date in blocked:
                    violations += 1
                    result.constraint_violations.append(
                        f"BLOCKED_DATE: '{block.title}' on {block_date} is on a blocked date ({rule.get('description', '')})"
                    )

    if total_checks > 0:
        result.constraint_adherence = max(0.0, 1.0 - violations / total_checks)
    else:
        result.constraint_adherence = 1.0

    return result
