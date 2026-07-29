#!/usr/bin/env python3
"""Record and report what the repository's deferred test tiers actually ran.

Six named tiers are deliberately outside the routine gate because they need
tools, corpora, or a browser the routine gate must not depend on:
`scripts/check-tools.sh`, `scripts/check-wasm.sh`, and
`scripts/check-hb-shape-fixtures.sh`, and the three explicit LaTeX tiers.
Moving them into `scripts/check.sh` or
the routine `cargo test --tests` suite would make the fast path depend on wasm-pack,
Firefox, ripgrep, and three extra dependency trees, which is precisely why they
are separate (`umber2-johp.211`).

Being separate is not the defect.  The defect (`umber2-johp.213`) was that the
routine gates *asserted* those tiers cover what they exclude while nothing
recorded whether any of them had ever run.  A green routine run therefore read
as a statement about coverage it had no evidence for -- the same shape as
`umber2-johp.121`, `.168`, `.201`, and `.211`.

So each tier writes a stamp here when it finishes, naming the commit, the tree
state, and the census of steps that ran, and every routine gate prints what
those stamps say.  A stamp cannot be produced without running the tier: the
tier runner writes it from its own step accounting, a partial run records a
partial census, and a run that could not execute a step records `BLOCKED` with
the reason rather than a pass.  "The tier ran" therefore stops being something
a reader assumes and becomes something a run prints.

Stamps live in `.tier-stamps/` (gitignored) because they are facts about one
working tree, not about the branch: a tier that passed in a different checkout
says nothing about this one.

Exit status:

  0  the report was produced, or the stamp was recorded
  2  the request named a tier that is not in the registry
  5  `--require-attempted` and some tier has never run in this checkout
"""

from __future__ import annotations

import argparse
import dataclasses
import datetime
import json
import subprocess
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
STAMP_DIR = REPO_ROOT / ".tier-stamps"

EXIT_OK = 0
EXIT_UNKNOWN_TIER = 2
EXIT_NEVER_RUN = 5

# Recorded states a tier run may end in.  There is no "skipped": a tier that
# could not run its work records BLOCKED, which is never a success.
STATUS_PASS = "PASS"
STATUS_FAIL = "FAIL"
STATUS_BLOCKED = "BLOCKED"
RECORDABLE_STATUSES = (STATUS_PASS, STATUS_FAIL, STATUS_BLOCKED)

# Reported states, which add the ones only a reader can determine: whether the
# recorded run covers the tree in front of them, and whether it covered the
# whole tier.
STATE_PASSED = "PASSED"
STATE_PARTIAL = "PARTIAL"
STATE_STALE = "STALE"
STATE_BLOCKED = "BLOCKED"
STATE_FAILED = "FAILED"
STATE_NEVER_RUN = "NEVER-RUN"

# States in which the tier's coverage claim is currently backed by evidence for
# this tree.  Everything else is a claim with nothing behind it.
SATISFIED_STATES = frozenset({STATE_PASSED})


@dataclasses.dataclass(frozen=True)
class TierSpec:
    """A tier the routine gates defer coverage to."""

    command: str
    covers: str


TIERS: dict[str, TierSpec] = {
    "check-tools.sh": TierSpec(
        command="scripts/check-tools.sh",
        covers=(
            "the three `[workspace] exclude` directories, parity-harness in its "
            "reference-tools resolution, and the opt-in clippy features"
        ),
    ),
    "check-wasm.sh": TierSpec(
        command="scripts/check-wasm.sh",
        covers="umber-wasm's #[wasm_bindgen_test] suite and the browser package",
    ),
    "check-hb-shape-fixtures.sh": TierSpec(
        command="scripts/check-hb-shape-fixtures.sh",
        covers="the rustybuzz cross-check against C HarfBuzz",
    ),
    "check-latex-corpus.sh": TierSpec(
        command="scripts/check-latex-corpus.sh",
        covers="the pinned native LaTeX base-class corpus and runtime closure",
    ),
    "check-latex-wasm.sh": TierSpec(
        command="scripts/check-latex-wasm.sh",
        covers="the pinned LaTeX native/WASM article parity build",
    ),
    "check-latex-parity.sh": TierSpec(
        command="scripts/check-latex-parity.sh",
        covers="the pinned upstream LaTeX2e DVI parity cohort",
    ),
}


@dataclasses.dataclass(frozen=True)
class Position:
    """Where a recorded run sits relative to the tree being reported on.

    `dirty_now` is the working tree's state at report time, not at record time.
    Both matter and neither implies the other: a run recorded against a modified
    tree never described a committed state, and a run recorded against a clean
    tree stops describing the tree the moment someone edits it.
    """

    phrase: str
    at_head: bool
    dirty_now: bool = False


@dataclasses.dataclass(frozen=True)
class Report:
    tier: str
    state: str
    line: str

    @property
    def satisfied(self) -> bool:
        return self.state in SATISFIED_STATES


def describe(tier: str, stamp: dict | None, position: Position) -> Report:
    """Classify one tier's recorded evidence.  Pure; the self-test drives it."""
    spec = TIERS[tier]
    if stamp is None:
        return Report(
            tier,
            STATE_NEVER_RUN,
            f"TIER: {tier}: {STATE_NEVER_RUN} - no run recorded in this checkout"
            f"; nothing here has exercised {spec.covers}",
        )

    total = stamp["steps_total"]
    selected = stamp["steps_selected"]
    ran = stamp["steps_ran"]
    failed = stamp["steps_failed"]
    blocked = stamp["steps_blocked"]
    blockers = stamp["blockers"]
    census = f"{ran} of {total} steps ran"
    when = f"{position.phrase} on {stamp['recorded_at']}"
    if stamp["dirty"]:
        when += ", against a modified tree"
    elif position.dirty_now:
        when += ", before the working tree was modified"

    if stamp["status"] == STATUS_FAIL:
        named = ", ".join(failed) or "unnamed steps"
        return Report(
            tier,
            STATE_FAILED,
            f"TIER: {tier}: {STATE_FAILED} {when} - {census}, "
            f"{len(failed)} failed: {named}",
        )

    if stamp["status"] == STATUS_BLOCKED:
        named = "; ".join(blockers) or "reason not recorded"
        return Report(
            tier,
            STATE_BLOCKED,
            f"TIER: {tier}: {STATE_BLOCKED} {when} - {census}, "
            f"{len(blocked)} could not run: {named}",
        )

    if selected < total:
        return Report(
            tier,
            STATE_PARTIAL,
            f"TIER: {tier}: {STATE_PARTIAL} {when} - {census}; a named-step run "
            f"is not evidence for the {total - selected} steps it did not select",
        )

    if position.at_head and not stamp["dirty"] and not position.dirty_now:
        return Report(
            tier,
            STATE_PASSED,
            f"TIER: {tier}: {STATE_PASSED} {when} - {census}",
        )

    return Report(
        tier,
        STATE_STALE,
        f"TIER: {tier}: {STATE_STALE} - passed {when} - {census}; "
        f"re-run {spec.command} to make it evidence for this tree",
    )


def summarize(reports: list[Report]) -> str:
    """One line stating how many deferred tiers currently have evidence."""
    counts: dict[str, int] = {}
    for report in reports:
        counts[report.state] = counts.get(report.state, 0) + 1
    census = ", ".join(
        f"{counts[state]} {state.lower()}"
        for state in (
            STATE_PASSED,
            STATE_PARTIAL,
            STATE_STALE,
            STATE_BLOCKED,
            STATE_FAILED,
            STATE_NEVER_RUN,
        )
        if state in counts
    )
    satisfied = sum(1 for report in reports if report.satisfied)
    return (
        f"TIER SUMMARY: {satisfied} of {len(reports)} deferred tiers have "
        f"passed on this tree ({census})"
    )


def git(*arguments: str) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        ["git", *arguments],
        cwd=REPO_ROOT,
        check=False,
        capture_output=True,
        text=True,
    )


def head_commit() -> str | None:
    result = git("rev-parse", "HEAD")
    return result.stdout.strip() if result.returncode == 0 else None


def tree_is_dirty() -> bool:
    result = git("status", "--porcelain")
    return bool(result.stdout.strip()) if result.returncode == 0 else True


def position_of(commit: str, head: str | None, dirty_now: bool) -> Position:
    """Locate a recorded commit relative to HEAD, without trusting either."""
    if head is None:
        return Position(f"at {commit[:8]}, in a checkout with no HEAD", False, dirty_now)
    if commit == head:
        return Position("at HEAD", True, dirty_now)
    if git("cat-file", "-e", f"{commit}^{{commit}}").returncode != 0:
        return Position(
            f"at {commit[:8]}, a commit this checkout does not have", False, dirty_now
        )
    if git("merge-base", "--is-ancestor", commit, head).returncode == 0:
        distance = git("rev-list", "--count", f"{commit}..{head}").stdout.strip()
        return Position(
            f"at {commit[:8]}, {distance} commits before HEAD", False, dirty_now
        )
    return Position(
        f"at {commit[:8]}, which is not an ancestor of HEAD", False, dirty_now
    )


def stamp_path(tier: str) -> Path:
    return STAMP_DIR / f"{tier}.json"


def load_stamp(tier: str) -> dict | None:
    path = stamp_path(tier)
    if not path.is_file():
        return None
    try:
        stamp = json.loads(path.read_text())
    except (OSError, json.JSONDecodeError):
        return None
    return stamp if stamp.get("tier") == tier else None


def collect(tiers: list[str]) -> list[Report]:
    head = head_commit()
    dirty_now = tree_is_dirty()
    reports = []
    for tier in tiers:
        stamp = load_stamp(tier)
        position = (
            Position("with no commit recorded", False, dirty_now)
            if stamp is None
            else position_of(stamp["commit"], head, dirty_now)
        )
        reports.append(describe(tier, stamp, position))
    return reports


def self_test() -> bool:
    """Prove the classifier still rejects what it must, before reporting."""
    return (
        subprocess.run(
            [sys.executable, str(REPO_ROOT / "scripts" / "test_tier_stamp.py")],
            cwd=REPO_ROOT,
            check=False,
        ).returncode
        == 0
    )


def do_report(arguments: argparse.Namespace) -> int:
    unknown = sorted(set(arguments.tiers) - set(TIERS))
    if unknown:
        print(
            f"tier_stamp: no such tier: {', '.join(unknown)}; "
            f"the registry holds {', '.join(TIERS)}",
            file=sys.stderr,
        )
        return EXIT_UNKNOWN_TIER
    if not arguments.no_self_test and not self_test():
        print(
            "TIER SUMMARY: the tier classifier's own guards are broken; its "
            "report would mean nothing",
            file=sys.stderr,
        )
        return EXIT_UNKNOWN_TIER
    reports = collect(arguments.tiers or list(TIERS))
    for report in reports:
        print(report.line)
    print(summarize(reports))

    if not arguments.require_attempted:
        return EXIT_OK

    # Only NEVER-RUN is refused, and deliberately so.  Refusing BLOCKED or
    # FAILED as well would make honestly recording a bad outcome worse for the
    # author than never running the tier, which is an incentive to leave it
    # unrun -- the defect itself.  This condition cannot be satisfied without
    # invoking the tier, because only the tier runner writes a stamp.
    never = [report.tier for report in reports if report.state == STATE_NEVER_RUN]
    if not never:
        return EXIT_OK
    print(
        "\ntier_stamp: never run in this checkout: "
        + ", ".join(never)
        + "\nRun "
        + ", ".join(TIERS[tier].command for tier in never)
        + " once. A tier that reports BLOCKED because a tool is absent is an "
        "answer; a tier nobody has invoked is not.",
        file=sys.stderr,
    )
    return EXIT_NEVER_RUN


def do_record(arguments: argparse.Namespace) -> int:
    stamp = {
        "schema": 1,
        "tier": arguments.tier,
        "status": arguments.status,
        "commit": head_commit() or "",
        "dirty": tree_is_dirty(),
        "recorded_at": datetime.datetime.now(datetime.UTC).strftime(
            "%Y-%m-%dT%H:%M:%SZ"
        ),
        "steps_total": arguments.total,
        "steps_selected": arguments.selected,
        "steps_ran": arguments.ran,
        "steps_failed": arguments.failed,
        "steps_blocked": arguments.blocked,
        "blockers": arguments.blocker,
    }
    STAMP_DIR.mkdir(parents=True, exist_ok=True)
    stamp_path(arguments.tier).write_text(json.dumps(stamp, indent=2) + "\n")
    return EXIT_OK


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    subcommands = parser.add_subparsers(dest="subcommand", required=True)

    report = subcommands.add_parser("report", help="print each tier's evidence")
    report.add_argument("tiers", nargs="*", default=[])
    report.add_argument("--no-self-test", action="store_true")
    report.add_argument(
        "--require-attempted",
        action="store_true",
        help="exit 5 if any tier has never run in this checkout",
    )
    report.set_defaults(handler=do_report)

    record = subcommands.add_parser("record", help="record a finished tier run")
    record.add_argument("tier", choices=list(TIERS))
    record.add_argument("--status", required=True, choices=RECORDABLE_STATUSES)
    record.add_argument("--total", required=True, type=int)
    record.add_argument("--selected", required=True, type=int)
    record.add_argument("--ran", required=True, type=int)
    record.add_argument("--failed", action="append", default=[])
    record.add_argument("--blocked", action="append", default=[])
    record.add_argument("--blocker", action="append", default=[])
    record.set_defaults(handler=do_record)

    return parser


def main(argv: list[str]) -> int:
    arguments = build_parser().parse_args(argv)
    return arguments.handler(arguments)


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
