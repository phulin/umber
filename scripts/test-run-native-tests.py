#!/usr/bin/env python3
"""Synthetic-input proof that every guard in `run-native-tests.py` can fail.

A coverage check that cannot fail is worth less than no check at all, which is
the whole point of `umber2-johp.211`. Each case here drives one guard with
inputs it must reject, so the guard's silence during a real run is evidence.
"""

from __future__ import annotations

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import importlib

runner = importlib.import_module("run-native-tests")


def target(name: str, kind: str, testable: bool) -> dict:
    return {"name": name, "kind": [kind], "test": testable}


LIB = [target("thing", "lib", True)]
LIB_AND_IT = LIB + [target("it", "test", True)]

failures: list[str] = []


def expect(condition: bool, description: str) -> None:
    if not condition:
        failures.append(description)


def parsed(passed: int, failed: int, ignored: int):
    line = (
        f"test result: ok. {passed} passed; {failed} failed; {ignored} ignored; "
        "0 measured; 0 filtered out; finished in 0.01s\n"
    )
    match = runner.RESULT_LINE.match(line)
    assert match is not None, "RESULT_LINE no longer matches cargo's own format"
    return match


REAL_TIER = "check-wasm.sh"


def deferral(tier: str = REAL_TIER, reason: str = "reason") -> "runner.Deferral":
    return runner.Deferral(tier=tier, reason=reason)


def run_plan(members: dict, excluded: dict, workspaces: dict | None = None):
    saved_packages = runner.EXCLUDED_PACKAGES
    saved_workspaces = runner.EXCLUDED_WORKSPACES
    runner.EXCLUDED_PACKAGES = excluded
    runner.EXCLUDED_WORKSPACES = {} if workspaces is None else workspaces
    try:
        return runner.plan(members, list(runner.EXCLUDED_WORKSPACES))
    finally:
        runner.EXCLUDED_PACKAGES = saved_packages
        runner.EXCLUDED_WORKSPACES = saved_workspaces


def raises_coverage(members: dict, excluded: dict, workspaces: dict | None = None) -> bool:
    try:
        run_plan(members, excluded, workspaces)
    except runner.CoverageError:
        return True
    return False


def raises_workspace_coverage(declared: list[str], workspaces: dict) -> bool:
    saved = runner.EXCLUDED_WORKSPACES
    runner.EXCLUDED_WORKSPACES = workspaces
    try:
        runner.check_excluded_workspaces(declared)
    except runner.CoverageError:
        return True
    finally:
        runner.EXCLUDED_WORKSPACES = saved
    return False


# The regex is the load-bearing link between cargo's output and every count
# below, so prove it reads a real line before trusting any of them.
sample = parsed(356, 0, 939)
expect(sample["passed"] == "356", "RESULT_LINE misreads the passed count")
expect(sample["ignored"] == "939", "RESULT_LINE misreads the ignored count")

# A member added to Cargo.toml is selected without anyone remembering to.
selected, binaries = run_plan({"a": LIB, "b": LIB_AND_IT}, {})
expect(selected == ["a", "b"], "a new workspace member was not selected")
expect(binaries == 3, "test binaries were miscounted from the manifests")

# `test = false` targets and non-test kinds are not counted as binaries.
_, binaries = run_plan(
    {"a": LIB + [target("bench", "bench", False), target("ex", "example", False)]}, {}
)
expect(binaries == 1, "a non-test target was counted as a test binary")

# An exclusion that outlived its package fails rather than being ignored.
expect(
    raises_coverage({"a": LIB}, {"gone": deferral()}),
    "a stale exclusion did not fail the run",
)
# ... and one that still names a member is honoured.
selected, _ = run_plan({"a": LIB, "b": LIB}, {"b": deferral()})
expect(selected == ["a"], "a live exclusion was not applied")

# A directory pushed out of the workspace must name the gate that runs it.
expect(
    raises_workspace_coverage(["tools/new-thing"], {}),
    "an undeclared `[workspace] exclude` directory did not fail the run",
)
expect(
    raises_workspace_coverage([], {"tools/gone": "check-tools.sh"}),
    "a declaration for a directory no longer excluded did not fail the run",
)
expect(
    not raises_workspace_coverage(["tools/thing"], {"tools/thing": "check-tools.sh"}),
    "a correct `[workspace] exclude` declaration was rejected",
)

# An exclusion may only defer to a tier that stamps and reports its own runs;
# deferring to anything else is a claim nothing can check (`umber2-johp.213`).
expect(
    raises_coverage({"a": LIB, "b": LIB}, {"b": deferral(tier="check-nothing.sh")}),
    "an exclusion deferring to an unregistered tier did not fail the run",
)
expect(
    raises_coverage(
        {"a": LIB}, {}, {"tools/thing": "check-nothing.sh"}
    ),
    "a `[workspace] exclude` deferring to an unregistered tier did not fail",
)
expect(
    set(runner.EXCLUDED_WORKSPACES.values())
    | {entry.tier for entry in runner.EXCLUDED_PACKAGES.values()}
    <= set(runner.tier_stamp.TIERS),
    "the live exclusion declarations name a tier outside the registry",
)

# A selection that cannot fail is not allowed to report success.
expect(raises_coverage({"a": LIB}, {"a": deferral()}), "an empty selection passed")
expect(
    raises_coverage({"a": [target("thing", "lib", False)]}, {}),
    "a selection with no test binaries at all passed",
)

# Verdict classification.
ok = [parsed(10, 0, 2), parsed(5, 0, 0)]
expect(
    runner.verdict(0, ok, 2, 2)[0] == runner.EXIT_PASS,
    "a clean run did not report PASS",
)
expect(
    runner.verdict(0, [parsed(10, 1, 0), parsed(5, 0, 0)], 2, 2)[0] == runner.EXIT_FAIL,
    "a failing test did not report FAIL",
)
expect(
    runner.verdict(101, ok, 2, 2)[0] == runner.EXIT_FAIL,
    "a nonzero cargo status did not report FAIL",
)
expect(
    runner.verdict(0, ok[:1], 2, 2)[0] == runner.EXIT_SHORT,
    "a run that executed fewer binaries than it built did not report SHORT",
)
expect(
    runner.verdict(0, [], 2, 2)[0] == runner.EXIT_SHORT,
    "a run that executed nothing at all did not report SHORT",
)
expect(
    runner.verdict(0, ok, 2, 1)[0] == runner.EXIT_COVERAGE,
    "an unmodelled extra test binary did not report COVERAGE",
)

# The verdict line must state the census, not merely a word.
_, line = runner.verdict(0, ok, 2, 2)
expect("VERDICT: PASS" in line, "the passing verdict is not labelled")
expect(
    "2 packages, 2/2 test binaries, 15 passed, 0 failed, 2 ignored" in line,
    "the passing verdict does not state what ran",
)

# A PASS must never read as a statement about the tiers this suite defers to.
_, line = runner.verdict(0, ok, 2, 2, "deferred tiers: 0 of 3 passed on this tree")
expect(
    line.endswith("; deferred tiers: 0 of 3 passed on this tree"),
    "the verdict does not state the deferred tiers' evidence",
)

if failures:
    print("test-run-native-tests: FAILED:", file=sys.stderr)
    for failure in failures:
        print(f"  - {failure}", file=sys.stderr)
    sys.exit(1)

print("test-run-native-tests: all guards fail when they should.")
