#!/usr/bin/env python3
"""Smoke tests for the scoring module."""

import sys
from datetime import datetime, timezone

sys.path.insert(0, ".")

from scoring import (
    ExistingEvent,
    ScheduledBlock,
    overlaps,
    score_scenario,
)

UTC = timezone.utc


def dt(month, day, hour, minute=0):
    return datetime(2025, month, day, hour, minute, tzinfo=UTC)


def test_overlaps():
    # Overlapping
    assert overlaps(dt(1, 20, 9), dt(1, 20, 11), dt(1, 20, 10), dt(1, 20, 12))
    # Adjacent — no overlap
    assert not overlaps(dt(1, 20, 9), dt(1, 20, 10), dt(1, 20, 10), dt(1, 20, 11))
    # Disjoint
    assert not overlaps(dt(1, 20, 9), dt(1, 20, 10), dt(1, 20, 14), dt(1, 20, 15))
    print("  overlaps: PASS")


def test_no_conflicts():
    blocks = [ScheduledBlock("Focus", dt(1, 20, 14), dt(1, 20, 15))]
    existing = [ExistingEvent("Meeting", dt(1, 20, 9), dt(1, 20, 10))]
    constraints = {"required_block_count": 1, "no_conflicts": True}
    score = score_scenario(blocks, existing, constraints)
    assert score.correctness == 1.0
    assert score.completeness == 1.0
    assert score.conflict_count == 0
    print("  no_conflicts: PASS")


def test_with_conflict():
    blocks = [ScheduledBlock("Focus", dt(1, 20, 9), dt(1, 20, 10, 30))]
    existing = [ExistingEvent("Meeting", dt(1, 20, 10), dt(1, 20, 11))]
    constraints = {"required_block_count": 1}
    score = score_scenario(blocks, existing, constraints)
    assert score.correctness == 0.0
    assert score.conflict_count == 1
    print("  with_conflict: PASS")


def test_completeness():
    blocks = [ScheduledBlock("A", dt(1, 20, 9), dt(1, 20, 10))]
    constraints = {"required_block_count": 3}
    score = score_scenario(blocks, [], constraints)
    assert abs(score.completeness - 1 / 3) < 0.01
    print("  completeness: PASS")


def test_working_hours_violation():
    blocks = [ScheduledBlock("Late", dt(1, 20, 18), dt(1, 20, 19))]
    constraints = {
        "required_block_count": 1,
        "working_hours": {"start_hour": 9, "end_hour": 17},
    }
    score = score_scenario(blocks, [], constraints)
    assert score.constraint_adherence < 1.0
    assert any("WORKING_HOURS" in v for v in score.constraint_violations)
    print("  working_hours_violation: PASS")


def test_composite_perfect():
    blocks = [
        ScheduledBlock("A", dt(1, 20, 10), dt(1, 20, 11)),
        ScheduledBlock("B", dt(1, 21, 10), dt(1, 21, 11)),
    ]
    constraints = {
        "required_block_count": 2,
        "working_hours": {"start_hour": 9, "end_hour": 17},
    }
    score = score_scenario(blocks, [], constraints)
    assert score.composite_score == 100.0
    print("  composite_perfect: PASS")


def test_custom_rule_max_per_day():
    blocks = [
        ScheduledBlock("A", dt(1, 20, 9), dt(1, 20, 10)),
        ScheduledBlock("B", dt(1, 20, 11), dt(1, 20, 12)),
    ]
    constraints = {
        "required_block_count": 2,
        "custom_rules": [{"type": "max_per_day", "value": 1}],
    }
    score = score_scenario(blocks, [], constraints)
    assert any("MAX_PER_DAY" in v for v in score.constraint_violations)
    print("  custom_rule_max_per_day: PASS")


if __name__ == "__main__":
    print("Running scoring tests...")
    test_overlaps()
    test_no_conflicts()
    test_with_conflict()
    test_completeness()
    test_working_hours_violation()
    test_composite_perfect()
    test_custom_rule_max_per_day()
    print("\nAll scoring tests passed!")
