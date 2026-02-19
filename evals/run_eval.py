#!/usr/bin/env python3
"""Tempo MCP Eval Runner.

Runs scheduling scenarios against bare Claude and Claude + Tempo MCP,
then outputs a comparison report.

Usage:
    python run_eval.py                     # Run all scenarios, both modes
    python run_eval.py --mode bare         # Bare model only
    python run_eval.py --mode mcp          # MCP mode only
    python run_eval.py --scenario 02       # Run only scenario 02
    python run_eval.py --model claude-sonnet-4-20250514

Environment:
    ANTHROPIC_API_KEY   - Required
    TEMPO_BINARY        - Path to tempo-mcp binary (default: target/release/tempo-mcp)
    EVAL_MODEL          - Model to use (default: claude-sonnet-4-20250514)
"""

import argparse
import asyncio
import json
import logging
import os
import sys
from datetime import datetime
from pathlib import Path

from dotenv import load_dotenv

# Load .env from evals/ directory, then project root
load_dotenv(Path(__file__).parent / ".env")
load_dotenv(Path(__file__).parent.parent / ".env")

# Ensure evals/ is on the path
sys.path.insert(0, str(Path(__file__).parent))

import anthropic

import harness

logging.basicConfig(
    level=logging.INFO,
    format="%(asctime)s %(levelname)s %(message)s",
    datefmt="%H:%M:%S",
)
logger = logging.getLogger(__name__)


def find_scenarios(filter_str: str | None = None) -> list[Path]:
    scenarios_dir = Path(__file__).parent / "scenarios"
    paths = sorted(scenarios_dir.glob("*.json"))
    if filter_str:
        paths = [p for p in paths if filter_str in p.stem]
    return paths


def print_result(result: harness.EvalResult) -> None:
    score = result.score
    print(f"\n{'='*60}")
    print(f"  {result.scenario_name} [{result.mode.upper()}]")
    print(f"{'='*60}")

    if result.error:
        print(f"  ERROR: {result.error}")
        return

    print(f"  Composite Score:  {score.composite_score}/100")
    print(f"  Correctness:      {score.correctness:.0%} ({score.conflict_count} conflicts)")
    print(f"  Completeness:     {score.completeness:.0%} ({score.total_blocks}/{score.required_blocks} blocks)")
    print(f"  Constraints:      {score.constraint_adherence:.0%}")
    print(f"  ---")
    print(f"  Wall time:        {result.wall_time_seconds:.1f}s")
    print(f"  Tokens:           {result.input_tokens} in / {result.output_tokens} out")
    print(f"  API rounds:       {result.api_rounds}")
    if result.mode == "mcp":
        print(f"  Tool calls:       {result.tool_calls}")
    print(f"  Blocks scheduled: {len(result.blocks)}")

    if score.constraint_violations:
        print(f"  Violations:")
        for v in score.constraint_violations[:10]:
            print(f"    - {v}")
        if len(score.constraint_violations) > 10:
            print(f"    ... and {len(score.constraint_violations) - 10} more")


def print_comparison(bare: harness.EvalResult, mcp: harness.EvalResult) -> None:
    print(f"\n{'='*60}")
    print(f"  COMPARISON: {bare.scenario_name}")
    print(f"{'='*60}")

    def delta(a: float, b: float) -> str:
        d = b - a
        return f"+{d:.1f}" if d > 0 else f"{d:.1f}"

    bs, ms = bare.score, mcp.score
    print(f"  {'Metric':<20} {'Bare':>10} {'MCP':>10} {'Delta':>10}")
    print(f"  {'-'*50}")
    print(f"  {'Composite':.<20} {bs.composite_score:>10.1f} {ms.composite_score:>10.1f} {delta(bs.composite_score, ms.composite_score):>10}")
    print(f"  {'Correctness':.<20} {bs.correctness:>10.0%} {ms.correctness:>10.0%} {delta(bs.correctness*100, ms.correctness*100):>10}")
    print(f"  {'Completeness':.<20} {bs.completeness:>10.0%} {ms.completeness:>10.0%} {delta(bs.completeness*100, ms.completeness*100):>10}")
    print(f"  {'Constraints':.<20} {bs.constraint_adherence:>10.0%} {ms.constraint_adherence:>10.0%} {delta(bs.constraint_adherence*100, ms.constraint_adherence*100):>10}")
    print(f"  {'Conflicts':.<20} {bs.conflict_count:>10} {ms.conflict_count:>10}")
    print(f"  {'Wall time (s)':.<20} {bare.wall_time_seconds:>10.1f} {mcp.wall_time_seconds:>10.1f} {delta(bare.wall_time_seconds, mcp.wall_time_seconds):>10}")
    print(f"  {'Total tokens':.<20} {bare.input_tokens+bare.output_tokens:>10} {mcp.input_tokens+mcp.output_tokens:>10}")


async def main():
    parser = argparse.ArgumentParser(description="Tempo MCP Eval Runner")
    parser.add_argument("--mode", choices=["bare", "mcp", "both"], default="both")
    parser.add_argument("--scenario", type=str, default=None, help="Filter scenarios by name/number")
    parser.add_argument("--model", type=str, default=None, help="Override model")
    parser.add_argument("--output", type=str, default=None, help="Output JSON file for results")
    args = parser.parse_args()

    if args.model:
        os.environ["EVAL_MODEL"] = args.model
        harness.MODEL = args.model

    # Check API key
    if not os.environ.get("ANTHROPIC_API_KEY"):
        print("ERROR: Set ANTHROPIC_API_KEY environment variable")
        sys.exit(1)

    # Check tempo binary for MCP mode
    if args.mode in ("mcp", "both"):
        if not Path(harness.TEMPO_BINARY).exists():
            print(f"ERROR: Tempo binary not found at {harness.TEMPO_BINARY}")
            print("Run: cargo build --release")
            sys.exit(1)

    client = anthropic.Anthropic()
    scenarios = find_scenarios(args.scenario)

    if not scenarios:
        print("No scenarios found.")
        sys.exit(1)

    print(f"Running {len(scenarios)} scenario(s) in {args.mode} mode")
    print(f"Model: {harness.MODEL}")
    print(f"Tempo: {harness.TEMPO_BINARY}")

    all_results: list[dict] = []

    for scenario_path in scenarios:
        scenario = harness.load_scenario(str(scenario_path))
        logger.info("Running: %s [%s]", scenario["name"], scenario["difficulty"])

        bare_result = None
        mcp_result = None

        if args.mode in ("bare", "both"):
            logger.info("  Bare model...")
            bare_result = harness.run_bare_eval(scenario, client)
            print_result(bare_result)

        if args.mode in ("mcp", "both"):
            logger.info("  MCP model...")
            mcp_result = await harness.run_mcp_eval(scenario, client)
            print_result(mcp_result)

        if bare_result and mcp_result:
            print_comparison(bare_result, mcp_result)

        # Collect results for JSON output
        for r in [bare_result, mcp_result]:
            if r:
                all_results.append({
                    "scenario": r.scenario_name,
                    "mode": r.mode,
                    "composite_score": r.score.composite_score,
                    "correctness": r.score.correctness,
                    "completeness": r.score.completeness,
                    "constraint_adherence": r.score.constraint_adherence,
                    "conflict_count": r.score.conflict_count,
                    "blocks": r.blocks,
                    "wall_time_seconds": r.wall_time_seconds,
                    "input_tokens": r.input_tokens,
                    "output_tokens": r.output_tokens,
                    "tool_calls": r.tool_calls,
                    "api_rounds": r.api_rounds,
                    "error": r.error,
                    "violations": r.score.constraint_violations,
                })

    # Summary
    if args.mode == "both" and len(scenarios) > 1:
        print(f"\n{'='*60}")
        print(f"  AGGREGATE SUMMARY")
        print(f"{'='*60}")
        for mode in ["bare", "mcp"]:
            mode_results = [r for r in all_results if r["mode"] == mode]
            if mode_results:
                avg_score = sum(r["composite_score"] for r in mode_results) / len(mode_results)
                avg_correct = sum(r["correctness"] for r in mode_results) / len(mode_results)
                total_conflicts = sum(r["conflict_count"] for r in mode_results)
                total_time = sum(r["wall_time_seconds"] for r in mode_results)
                avg_time = total_time / len(mode_results)
                print(f"  {mode.upper():>5}: avg_score={avg_score:.1f}  avg_correctness={avg_correct:.0%}  total_conflicts={total_conflicts}  avg_time={avg_time:.1f}s  total_time={total_time:.1f}s")

    # Write results
    output_path = args.output or str(
        Path(__file__).parent / "results" / f"eval_{datetime.now().strftime('%Y%m%d_%H%M%S')}.json"
    )
    Path(output_path).parent.mkdir(parents=True, exist_ok=True)
    with open(output_path, "w") as f:
        json.dump(all_results, f, indent=2, default=str)
    print(f"\nResults written to {output_path}")


if __name__ == "__main__":
    asyncio.run(main())
