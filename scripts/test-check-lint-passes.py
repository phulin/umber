#!/usr/bin/env python3
"""Proves the coverage guards in `check-lint-passes.py` actually fail.

The clippy gate's value rests entirely on these guards rejecting a drifted
declaration, a stale quarantine, and an undecided feature. Each case below
feeds the checker synthetic data and requires a `Failure`; none of them
compiles anything, so this runs in well under a second as the first step of
the clippy gate.
"""

from __future__ import annotations

import importlib.util
import sys
from pathlib import Path

SCRIPT = Path(__file__).resolve().parent / "check-lint-passes.py"
spec = importlib.util.spec_from_file_location("check_lint_passes", SCRIPT)
assert spec is not None and spec.loader is not None
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)

failures: list[str] = []


def expect_failure(what: str, call) -> None:
    try:
        call()
    except module.Failure:
        return
    failures.append(f"{what}: expected a Failure, got none")


def expect_success(what: str, call) -> None:
    try:
        call()
    except module.Failure as failure:
        failures.append(f"{what}: unexpected Failure: {failure}")


def diagnostics(counts: dict[tuple[str, str], int]) -> object:
    collected = module.Diagnostics()
    for key, count in counts.items():
        for _ in range(count):
            collected.record(key, "", quarantined=True)
    return collected


PASS = {
    "name": "example",
    "summary": "synthetic",
    "args": (),
    "select": "workspace",
    "features": {"tex-command": ["profiling"]},
    "quarantine": {("tex-command", "unused_variables"): (9, "umber2-johp.200")},
}

expect_success(
    "declared resolution matches",
    lambda: module.check_features(PASS, {"tex-command": {"profiling"}}),
)
expect_failure(
    "a package resolving a feature the declaration omits",
    lambda: module.check_features(PASS, {"tex-command": {"profiling"}, "umber": {"shadow"}}),
)
expect_failure(
    "a package no longer resolving its declared feature",
    lambda: module.check_features(PASS, {"tex-command": set()}),
)
expect_failure(
    "a declared package the pass never built",
    lambda: module.check_features(PASS, {}),
)

expect_success(
    "quarantine count matching exactly",
    lambda: module.check_quarantine(PASS, diagnostics({("tex-command", "unused_variables"): 9})),
)
expect_failure(
    "a quarantined lint that stopped firing",
    lambda: module.check_quarantine(PASS, diagnostics({})),
)
expect_failure(
    "a quarantined lint firing more than its recorded count",
    lambda: module.check_quarantine(PASS, diagnostics({("tex-command", "unused_variables"): 10})),
)
expect_failure(
    "a lint with no quarantine entry",
    lambda: module.check_quarantine(
        PASS,
        diagnostics({("tex-command", "unused_variables"): 9, ("umber", "dead_code"): 1}),
    ),
)

MEMBERS = {
    "tex-command": {"features": {"profiling": []}},
    "umber": {"features": {"default": [], "shadow": []}},
}
COVERED = {"tex-command": {"profiling"}, "umber": set()}
OUT_OF_SCOPE = {"umber/shadow": "no routine build enables it"}

expect_success(
    "every feature either linted or declared out of scope",
    lambda: module.check_coverage(MEMBERS, COVERED, OUT_OF_SCOPE),
)
expect_failure(
    "a workspace member no pass lints",
    lambda: module.check_coverage(MEMBERS, {"tex-command": {"profiling"}}, OUT_OF_SCOPE),
)
expect_failure(
    "a new feature nobody decided coverage for",
    lambda: module.check_coverage(
        {**MEMBERS, "new-crate": {"features": {"brand-new": []}}},
        {**COVERED, "new-crate": set()},
        OUT_OF_SCOPE,
    ),
)
expect_failure(
    "an out-of-scope entry for a feature that is in fact linted",
    lambda: module.check_coverage(MEMBERS, {**COVERED, "umber": {"shadow"}}, OUT_OF_SCOPE),
)
expect_failure(
    "an out-of-scope entry for a feature that no longer exists",
    lambda: module.check_coverage(
        {**MEMBERS, "umber": {"features": {"default": []}}},
        COVERED,
        OUT_OF_SCOPE,
    ),
)

if failures:
    sys.stderr.write("test-check-lint-passes: FAILED\n")
    sys.stderr.write("\n".join(f"  {failure}" for failure in failures) + "\n")
    sys.exit(1)
print("test-check-lint-passes: coverage guards fail as designed.")
