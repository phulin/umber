#!/usr/bin/env python3
"""Run every clippy pass the repository lints, and prove what each one covered.

`cargo clippy` lints one feature resolution per invocation, and Cargo unifies
features across everything a single invocation selects.  The whole-workspace
`--all-targets` pass therefore lints `tex-command` and `tex-exec` with
`instrumentation` enabled, because `tools/tex-command-stream` depends on that
feature and `tex-exec`'s dev-dependencies enable it.  No invocation of that
shape can ever lint the resolution the shipped binary is built in, so warnings
that exist only there -- `cargo build -p umber`, `cargo run-dev -p umber`,
`cargo test -p umber --test it` -- were invisible to the gate while being
visible to every agent who typed one of those commands.

This script closes that gap structurally rather than by convention:

* every pass in `PASSES` runs on every invocation, so the gate is the union of
  the resolutions, not one of them;
* each pass declares the exact feature set it expects Cargo to resolve for each
  workspace package, and the declaration is checked against Cargo's own
  `compiler-artifact` records, so a manifest edit that silently moves a package
  into a different resolution fails the gate instead of changing what the gate
  covers;
* every feature declared by a workspace member must either be enabled in a pass
  that lints its owner, or be listed in `UNCOVERED_ENABLED_FEATURES` with a
  reason.  A new feature is uncovered by construction, so adding one without
  deciding its coverage fails the gate;
* every workspace member must be linted (not merely compiled) by some pass.

Known-dirty configurations are quarantined per pass, per package, per lint,
with an exact count and an issue id.  A quarantine that stops firing fails the
gate as loudly as a new warning does, so it cannot outlive its issue.
"""

from __future__ import annotations

import json
import os
import subprocess
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent

# Feature sets are written exactly as Cargo resolves them, including `default`.
# A package absent from a pass's `features` map must resolve with no features.
PASSES = (
    {
        "name": "union",
        # `--workspace` rather than the workspace's default members: a test
        # target only the exhaustive selection builds is still a target this
        # repository compiles, and one selected by no pass is one the lint
        # policy does not actually apply to (umber2-johp.201).
        "summary": "every workspace member, all targets, dev-dependency feature union",
        "args": ("--workspace", "--all-targets", "--features", "umber/instrumentation"),
        "select": "workspace",
        "features": {
            "parity-harness": ["trip-instrumentation"],
            "tex-command": ["instrumentation"],
            "tex-exec": ["instrumentation"],
            "tex-state": ["default", "testing"],
            "umber": ["default", "instrumentation"],
        },
        "quarantine": {},
    },
    {
        "name": "shipping",
        "summary": "every workspace member's lib and bin targets, no dev-dependencies",
        # `tex-command-stream` is the one member that forces `instrumentation`
        # on a dependency, so excluding it is what makes this pass resolve the
        # features a released `umber` is built with.  It is linted by the union
        # pass instead.
        "args": ("--workspace", "--lib", "--bins", "--exclude", "tex-command-stream"),
        "select": "workspace",
        "exclude": ("tex-command-stream",),
        "features": {
            "tex-state": ["default"],
            "umber": ["default"],
        },
        "quarantine": {},
    },
)

# Features no pass above enables, with the reason each is out of scope for this
# gate.  Every entry must name a feature some workspace member declares, and
# every declared feature that no pass enables must appear here.
UNCOVERED_ENABLED_FEATURES = {
    "tex-expand/shadow": "verification-only mirror; linted by scripts/check-tools.sh only through `umber`",
    "tex-expand/profiling-stats": "profiling-only counters; enabled by scripts/check-tools.sh through `umber`",
    "tex-exec/profiling-stats": "profiling-only counters; enabled by scripts/check-tools.sh through `umber`",
    "tex-lex/profiling-stats": "profiling-only counters; enabled by scripts/check-tools.sh through `umber`",
    "tex-state/shadow": "verification-only mirror; enabled by scripts/check-tools.sh through `umber`",
    "tex-state/profiling-stats": "profiling-only counters; enabled by scripts/check-tools.sh through `umber`",
    "tex-out/dvi-tools": "opt-in tool binary; linted by scripts/check-tools.sh",
    "umber/shadow": "verification-only mirror; no routine build enables it",
    "umber/profiling-runner": "opt-in profiling binary; linted by scripts/check-tools.sh",
    "umber/profiling-stats": "opt-in profiling binary; linted by scripts/check-tools.sh",
    "parity-harness/reference-tools": "opt-in reference tooling; linted by scripts/check-tools.sh",
}


class Failure(Exception):
    """A coverage or lint failure with a ready-to-print explanation."""


class Diagnostics:
    """Every workspace diagnostic of one pass, counted by package and lint.

    A quarantined lint's renderings are held back rather than printed, so a
    clean run prints no warning text at all: a warning on screen under a green
    verdict is exactly the ambiguity this gate exists to remove.  They are
    printed if the count stops matching, because then they are news.
    """

    def __init__(self) -> None:
        self.counts: dict[tuple[str, str], int] = {}
        self.held: dict[tuple[str, str], list[str]] = {}

    def record(self, key: tuple[str, str], rendering: str, *, quarantined: bool) -> None:
        self.counts[key] = self.counts.get(key, 0) + 1
        if quarantined:
            self.held.setdefault(key, []).append(rendering)
        elif rendering:
            sys.stderr.write(rendering)
            sys.stderr.flush()


def cargo_metadata() -> dict:
    completed = subprocess.run(
        ["cargo", "metadata", "--no-deps", "--format-version", "1"],
        cwd=REPO_ROOT,
        capture_output=True,
        text=True,
        check=True,
    )
    return json.loads(completed.stdout)


def run_pass(spec: dict, member_names: dict[str, str]) -> tuple[dict[str, set[str]], "Diagnostics"]:
    """Runs one clippy pass, returning its resolved features and its diagnostics."""

    # No `-D warnings` on the command line.  Denial happens in this script
    # instead, from the parsed diagnostic stream, because no rustc level flag
    # expresses "deny everything except this lint in this one package":
    # `-D warnings -W <lint>` still denies, and `-A <lint>` stops the
    # diagnostic being emitted at all, which would make a stale quarantine
    # entry indistinguishable from a live one.  Counting is strictly stronger
    # than `-D warnings`, which never applied to a workspace crate in
    # dependency position.
    #
    # A quarantined lint is downgraded to warn for this pass so the compilation
    # survives long enough to report every diagnostic, including the ones a
    # manifest `deny` would otherwise turn into a hard error.  That downgrade
    # loses no strictness: an occurrence outside the quarantine's package, or
    # beyond its recorded count, still fails below.
    lint_flags = []
    for code in sorted({code for _, code in spec["quarantine"]}):
        lint_flags += ["-W", code]
    rendered = "json-diagnostic-rendered-ansi" if sys.stderr.isatty() else "json"
    command = [
        "cargo",
        "clippy",
        "--quiet",
        f"--message-format={rendered}",
        *spec["args"],
        *(["--", *lint_flags] if lint_flags else []),
    ]
    print(f"--- lint pass: {spec['name']} ({spec['summary']})", flush=True)
    print(f"    {' '.join(command)}", flush=True)

    features: dict[str, set[str]] = {}
    diagnostics = Diagnostics()
    process = subprocess.Popen(
        command,
        cwd=REPO_ROOT,
        stdout=subprocess.PIPE,
        text=True,
        bufsize=1,
    )
    assert process.stdout is not None
    for line in process.stdout:
        line = line.strip()
        if not line.startswith("{"):
            if line:
                print(line, flush=True)
            continue
        record = json.loads(line)
        package = member_names.get(record.get("package_id", ""))
        reason = record.get("reason")
        if reason == "compiler-artifact" and package is not None:
            features.setdefault(package, set()).update(record.get("features") or [])
        elif reason == "compiler-message":
            message = record["message"]
            if message["level"] not in ("warning", "error"):
                continue
            rendering = message.get("rendered") or ""
            if package is None:
                sys.stderr.write(rendering)
                sys.stderr.flush()
                continue
            code = (message.get("code") or {}).get("code")
            key = (package, code or "<uncoded>")
            diagnostics.record(key, rendering, quarantined=key in spec["quarantine"])
    status = process.wait()
    if status != 0:
        raise Failure(f"lint pass {spec['name']!r} failed: cargo exited {status}")
    for (package, lint), (expected, issue) in sorted(spec["quarantine"].items()):
        print(
            f"    quarantined: {package} {lint} x{expected} ({issue}); "
            f"observed {diagnostics.counts.get((package, lint), 0)}",
            flush=True,
        )
    return features, diagnostics


def check_features(spec: dict, observed: dict[str, set[str]]) -> None:
    expected = {name: set(values) for name, values in spec["features"].items()}
    mismatched = [
        (name, sorted(values), sorted(expected.get(name, set())))
        for name, values in sorted(observed.items())
        if values != expected.get(name, set())
    ]
    missing = sorted(set(expected) - set(observed))
    if not mismatched and not missing:
        return
    lines = [
        f"lint pass {spec['name']!r} did not resolve the features it declares.",
        "Cargo's resolution changed, so this pass no longer covers what its",
        "declaration in scripts/check-lint-passes.py says it covers. Decide",
        "which resolution the gate should lint, then update the declaration.",
    ]
    for name, actual, want in mismatched:
        lines.append(f"  {name}: resolved {actual}, declared {want}")
    for name in missing:
        lines.append(f"  {name}: declared {sorted(expected[name])}, but the pass never built it")
    raise Failure("\n".join(lines))


def check_quarantine(spec: dict, diagnostics: Diagnostics) -> None:
    counts = diagnostics.counts
    problems = []
    for key, (expected, issue) in sorted(spec["quarantine"].items()):
        package, lint = key
        actual = counts.get(key, 0)
        if actual == expected:
            continue
        for rendering in diagnostics.held.get(key, ()):
            sys.stderr.write(rendering)
        sys.stderr.flush()
        if actual == 0:
            problems.append(
                f"  {package}: {lint} no longer fires in this pass. Delete the"
                f" quarantine entry and close {issue}."
            )
        else:
            problems.append(
                f"  {package}: {lint} fired {actual} times, quarantine expects"
                f" {expected} ({issue}). Fix the new occurrences; never raise"
                f" the count to match."
            )
    unexpected = sorted(
        f"  {package}: {lint} fired {count} times"
        for (package, lint), count in counts.items()
        if (package, lint) not in spec["quarantine"]
    )
    if not problems and not unexpected:
        return
    lines = [f"lint pass {spec['name']!r} is not clean."]
    if unexpected:
        lines.append(
            "Every diagnostic from a workspace crate fails this gate, including"
            " one from a crate in dependency position:"
        )
        lines.extend(unexpected)
    lines.extend(problems)
    raise Failure("\n".join(lines))


def check_coverage(
    members: dict[str, dict],
    linted: dict[str, set[str]],
    out_of_scope: dict[str, str] = UNCOVERED_ENABLED_FEATURES,
) -> None:
    """Every member is linted somewhere, and every feature's enabled state is decided."""

    never_linted = sorted(name for name in members if name not in linted)
    if never_linted:
        raise Failure(
            "these workspace members are compiled but never linted by any pass:\n"
            + "\n".join(f"  {name}" for name in never_linted)
            + "\nAdd them to a pass's selection, or the gate silently skips them."
        )

    enabled_somewhere = {
        f"{name}/{feature}" for name, features in linted.items() for feature in features
    }
    declared = {
        f"{name}/{feature}"
        for name, package in members.items()
        for feature in package["features"]
        if feature != "default"
    }
    uncovered = sorted(declared - enabled_somewhere)
    undeclared = sorted(name for name in uncovered if name not in out_of_scope)
    stale = sorted(
        name for name in out_of_scope if name not in declared or name in enabled_somewhere
    )
    problems = []
    if undeclared:
        problems.append(
            "no lint pass enables these features, and they are not listed as out"
            " of scope:\n"
            + "\n".join(f"  {name}" for name in undeclared)
            + "\nEither enable the feature in a pass, or record in"
            " UNCOVERED_ENABLED_FEATURES why the gate does not lint it."
        )
    if stale:
        problems.append(
            "UNCOVERED_ENABLED_FEATURES lists features that no longer exist or"
            " are now linted:\n" + "\n".join(f"  {name}" for name in stale)
        )
    if problems:
        raise Failure("\n\n".join(problems))


def main() -> int:
    metadata = cargo_metadata()
    members = {package["name"]: package for package in metadata["packages"]}
    member_names = {package["id"]: package["name"] for package in metadata["packages"]}
    default_members = {
        member_names[package_id] for package_id in metadata["workspace_default_members"]
    }

    linted: dict[str, set[str]] = {}
    failures: list[str] = []
    for spec in PASSES:
        try:
            observed, diagnostics = run_pass(spec, member_names)
        except Failure as failure:
            failures.append(str(failure))
            continue
        selected = default_members if spec["select"] == "default-members" else set(members)
        selected -= set(spec.get("exclude", ()))
        for name in selected:
            linted.setdefault(name, set()).update(observed.get(name, set()) - {"default"})
        for check, argument in ((check_features, observed), (check_quarantine, diagnostics)):
            try:
                check(spec, argument)
            except Failure as failure:
                failures.append(str(failure))

    if not failures:
        try:
            check_coverage(members, linted)
        except Failure as failure:
            failures.append(str(failure))

    if failures:
        sys.stderr.write("\ncheck-lint-passes: FAILED\n\n")
        sys.stderr.write("\n\n".join(failures) + "\n")
        return 1
    print(
        f"\ncheck-lint-passes: {len(PASSES)} lint passes clean; "
        f"{len(linted)} workspace members linted.",
        flush=True,
    )
    return 0


if __name__ == "__main__":
    os.environ.setdefault("CARGO_TERM_COLOR", "always" if sys.stderr.isatty() else "never")
    try:
        sys.exit(main())
    except Failure as failure:  # pragma: no cover - defensive
        sys.stderr.write(f"check-lint-passes: {failure}\n")
        sys.exit(1)
