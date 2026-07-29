#!/usr/bin/env python3
"""Synthetic-input proof that `scripts/tier_stamp.py` classifies honestly.

`tier_stamp.py report` runs this before printing anything, for the same reason
the clippy gate self-tests `check-lint-passes.py` and the native suite
self-tests `check-lint-passes.py`: a coverage check nobody has watched fail
is worth no more than no check at all, and this one exists specifically to stop
an unproven coverage claim from reading as a proven one.

Every case below is a state in which the tier's claim is *not* backed for the
tree in front of the reader.  Exactly one shape may report PASSED.
"""

from __future__ import annotations

import importlib.util
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
_SPEC = importlib.util.spec_from_file_location(
    "tier_stamp", REPO_ROOT / "scripts" / "tier_stamp.py"
)
assert _SPEC is not None and _SPEC.loader is not None
tier_stamp = importlib.util.module_from_spec(_SPEC)
sys.modules["tier_stamp"] = tier_stamp
_SPEC.loader.exec_module(tier_stamp)

TIER = "check-tools.sh"
AT_HEAD = tier_stamp.Position("at HEAD", True)
AT_HEAD_EDITED = tier_stamp.Position("at HEAD", True, dirty_now=True)
BEHIND = tier_stamp.Position("at 5e2d2799, 12 commits before HEAD", False)
UNRELATED = tier_stamp.Position("at 5e2d2799, which is not an ancestor of HEAD", False)


def stamp(**overrides) -> dict:
    base = {
        "schema": 1,
        "tier": TIER,
        "status": tier_stamp.STATUS_PASS,
        "commit": "5e2d2799",
        "dirty": False,
        "recorded_at": "2026-07-27T23:40:11Z",
        "steps_total": 12,
        "steps_selected": 12,
        "steps_ran": 12,
        "steps_failed": [],
        "steps_blocked": [],
        "blockers": [],
    }
    base.update(overrides)
    return base


def check(label: str, report, expected_state: str, must_mention: list[str]) -> None:
    if report.state != expected_state:
        raise AssertionError(
            f"{label}: classified {report.state}, expected {expected_state}"
        )
    if expected_state in tier_stamp.SATISFIED_STATES and not report.satisfied:
        raise AssertionError(f"{label}: {expected_state} must count as satisfied")
    if expected_state not in tier_stamp.SATISFIED_STATES and report.satisfied:
        raise AssertionError(
            f"{label}: {expected_state} must not count as evidence of coverage"
        )
    for fragment in must_mention:
        if fragment not in report.line:
            raise AssertionError(f"{label}: report line omits {fragment!r}: {report.line}")


def main() -> int:
    cases = [
        (
            "a tier that has never run must say so, not read as absent news",
            None,
            AT_HEAD,
            tier_stamp.STATE_NEVER_RUN,
            ["NEVER-RUN", "no run recorded"],
        ),
        (
            "a whole clean run at HEAD is the one shape that backs the claim",
            stamp(),
            AT_HEAD,
            tier_stamp.STATE_PASSED,
            ["PASSED", "12 of 12 steps ran"],
        ),
        (
            "a run whose prerequisites were missing is not a pass",
            stamp(
                status=tier_stamp.STATUS_BLOCKED,
                steps_ran=10,
                steps_blocked=["arXiv entrypoint selection"],
                blockers=["rg is not installed"],
            ),
            AT_HEAD,
            tier_stamp.STATE_BLOCKED,
            ["BLOCKED", "10 of 12 steps ran", "rg is not installed"],
        ),
        (
            "a failed run must name the steps that failed",
            stamp(status=tier_stamp.STATUS_FAIL, steps_failed=["oracle regeneration"]),
            AT_HEAD,
            tier_stamp.STATE_FAILED,
            ["FAILED", "oracle regeneration"],
        ),
        (
            "a named-step run is not evidence for the steps it did not select",
            stamp(steps_selected=3, steps_ran=3),
            AT_HEAD,
            tier_stamp.STATE_PARTIAL,
            ["PARTIAL", "3 of 12 steps ran", "did not select"],
        ),
        (
            "a pass recorded against a modified tree is not evidence for it",
            stamp(dirty=True),
            AT_HEAD,
            tier_stamp.STATE_STALE,
            ["STALE", "modified tree"],
        ),
        (
            "a pass stops being evidence the moment the tree is edited under it",
            stamp(),
            AT_HEAD_EDITED,
            tier_stamp.STATE_STALE,
            ["STALE", "before the working tree was modified"],
        ),
        (
            "a pass from an ancestor commit is stale, and must say how stale",
            stamp(),
            BEHIND,
            tier_stamp.STATE_STALE,
            ["STALE", "12 commits before HEAD"],
        ),
        (
            "a pass from an unrelated commit is stale, and must say so",
            stamp(),
            UNRELATED,
            tier_stamp.STATE_STALE,
            ["STALE", "not an ancestor of HEAD"],
        ),
    ]

    for label, recorded, position, expected_state, fragments in cases:
        check(label, tier_stamp.describe(TIER, recorded, position), expected_state, fragments)

    # The summary must count only the shapes that back the claim, so a run
    # cannot report full deferred-tier coverage without every tier passing.
    reports = [
        tier_stamp.describe(TIER, stamp(), AT_HEAD),
        tier_stamp.describe(TIER, stamp(dirty=True), AT_HEAD),
        tier_stamp.describe(TIER, None, AT_HEAD),
    ]
    summary = tier_stamp.summarize(reports)
    if "1 of 3 deferred tiers have passed" not in summary:
        raise AssertionError(f"summary miscounts satisfied tiers: {summary}")
    for expected in ("1 passed", "1 stale", "1 never-run"):
        if expected not in summary:
            raise AssertionError(f"summary omits {expected!r}: {summary}")

    # Every registry entry must name a command a reader can actually type.
    for name, spec in tier_stamp.TIERS.items():
        if not (REPO_ROOT / spec.command).is_file():
            raise AssertionError(f"{name} names a command that does not exist")

    print(f"test_tier_stamp: {len(cases)} classifier cases and the summary hold")
    return 0


if __name__ == "__main__":
    sys.exit(main())
