#!/usr/bin/env python3
"""Run the routine native correctness suite and prove what it executed.

`cargo test --tests` selects the workspace's *default* members.  Every other
member -- the nine `bib-*` crates, `umber-wasm`, `umber-interrupt`, `refexec`,
`profile-analyzer` -- therefore had its test targets executed by no routine
command, and a green run said nothing whatever about them (`umber2-johp.211`).
That is the same shape as `umber2-johp.201` for clippy and `umber2-johp.121`
for `tools/tex-command-stream`: the reassuring output was real, it just did not
mean what it appeared to mean.

This script makes the covered set a property of the command rather than of
whoever typed it:

* the selection is `--workspace` minus `EXCLUDED_PACKAGES`, so a member added
  to `Cargo.toml` is covered by construction rather than by remembering;
* every exclusion carries a reason and the exact command that does run it, and
  an exclusion naming a package the workspace no longer has fails the run, so a
  stale declaration cannot outlive the thing it excused;
* the number of test binaries Cargo's own manifests say the selection has is
  computed up front and compared against the number that actually reported, so
  a run that quietly executed fewer binaries than it built fails instead of
  passing;
* the run ends in a `VERDICT:` line naming the packages, the binaries, and the
  passed/failed/ignored totals.  Success is a positive statement about what
  ran, not the absence of complaints.

Exit status:

  0  PASS      every selected test binary ran and every test in it passed
  1  FAIL      a test failed, or Cargo could not build or run the selection
  2  COVERAGE  the exclusion declaration does not match the workspace
  3  SHORT     fewer test binaries reported than the selection declares
"""

from __future__ import annotations

import json
import re
import subprocess
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent

EXIT_PASS = 0
EXIT_FAIL = 1
EXIT_COVERAGE = 2
EXIT_SHORT = 3

# Workspace members this host suite deliberately does not select, each with the
# command that does run its tests.  An entry here is a claim that the package's
# tests run somewhere else; it is not permission to leave them unrun.
EXCLUDED_PACKAGES = {
    "umber-wasm": (
        "its tests are `#[wasm_bindgen_test]`, which registers no test on a "
        "host target: selecting it here builds a cdylib and runs exactly zero "
        "tests. `scripts/check-wasm.sh` runs them for real with "
        "`wasm-pack test --headless --firefox crates/umber-wasm`."
    ),
}

# Cargo target kinds that `--tests` builds in test mode.  Integration tests are
# kind `test`; a library or binary target is additionally built as a unit-test
# binary unless its manifest sets `test = false`.
TESTABLE_KINDS = frozenset(
    {"lib", "rlib", "dylib", "cdylib", "staticlib", "proc-macro", "bin", "test"}
)

RESULT_LINE = re.compile(
    r"^test result: (?P<outcome>\w+)\. (?P<passed>\d+) passed; (?P<failed>\d+) failed; "
    r"(?P<ignored>\d+) ignored"
)


class CoverageError(Exception):
    """The exclusion declaration and the workspace disagree."""


def workspace_members() -> dict[str, list[dict]]:
    """Every workspace member, mapped to its Cargo targets."""
    metadata = json.loads(
        subprocess.run(
            ["cargo", "metadata", "--no-deps", "--format-version", "1"],
            cwd=REPO_ROOT,
            check=True,
            capture_output=True,
            text=True,
        ).stdout
    )
    return {package["name"]: package["targets"] for package in metadata["packages"]}


def expected_test_binaries(targets: list[dict]) -> int:
    return sum(
        1
        for target in targets
        if target.get("test") and TESTABLE_KINDS.intersection(target["kind"])
    )


def plan(members: dict[str, list[dict]]) -> tuple[list[str], int]:
    """Resolve the selection, or explain why the declaration is wrong."""
    unknown = sorted(set(EXCLUDED_PACKAGES) - set(members))
    if unknown:
        raise CoverageError(
            "EXCLUDED_PACKAGES names packages that are not workspace members: "
            + ", ".join(unknown)
            + "\nRemove the stale entries, or fix the names."
        )

    selected = sorted(set(members) - set(EXCLUDED_PACKAGES))
    if not selected:
        raise CoverageError("the selection is empty; every member is excluded.")

    binaries = sum(expected_test_binaries(members[name]) for name in selected)
    if binaries == 0:
        raise CoverageError(
            "the selection declares no test binaries at all; refusing to report "
            "a suite that cannot fail."
        )
    return selected, binaries


def run_cargo(extra_args: list[str]) -> tuple[int, list[re.Match[str]]]:
    """Stream `cargo test` to our stdout while tallying its result lines."""
    command = ["cargo", "test", "--tests", "--quiet", "--workspace"]
    for name in sorted(EXCLUDED_PACKAGES):
        command += ["--exclude", name]
    command += extra_args

    print(f"run-native-tests: {' '.join(command)}", flush=True)
    process = subprocess.Popen(
        command,
        cwd=REPO_ROOT,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
        bufsize=1,
    )
    results: list[re.Match[str]] = []
    assert process.stdout is not None
    for line in process.stdout:
        sys.stdout.write(line)
        sys.stdout.flush()
        match = RESULT_LINE.match(line)
        if match is not None:
            results.append(match)
    return process.wait(), results


def verdict(
    status: int,
    results: list[re.Match[str]],
    packages: int,
    expected_binaries: int,
) -> tuple[int, str]:
    """Classify a finished run into an exit code and its one-line verdict."""
    passed = sum(int(match["passed"]) for match in results)
    failed = sum(int(match["failed"]) for match in results)
    ignored = sum(int(match["ignored"]) for match in results)
    ran = len(results)
    census = (
        f"{packages} packages, {ran}/{expected_binaries} test binaries, "
        f"{passed} passed, {failed} failed, {ignored} ignored"
    )

    if status != 0 or failed > 0:
        return EXIT_FAIL, f"run-native-tests: VERDICT: FAIL - {census}"
    if ran < expected_binaries:
        return EXIT_SHORT, (
            f"run-native-tests: VERDICT: SHORT - {census}\n"
            "Cargo reported fewer test binaries than the selected manifests "
            "declare. Something ran less than it claims to have run."
        )
    if ran > expected_binaries:
        return EXIT_COVERAGE, (
            f"run-native-tests: VERDICT: COVERAGE - {census}\n"
            "More test binaries reported than the selected manifests declare, "
            "so TESTABLE_KINDS no longer models what `--tests` builds and the "
            "binary count has stopped being evidence of anything."
        )
    return EXIT_PASS, f"run-native-tests: VERDICT: PASS - {census}"


def main(argv: list[str]) -> int:
    # The guards below are checked against synthetic inputs first, for the same
    # reason the clippy gate self-tests `check-lint-passes.py`: a coverage
    # check nobody has watched fail proves nothing when it stays quiet.
    self_test = subprocess.run(
        [sys.executable, str(REPO_ROOT / "scripts" / "test-run-native-tests.py")],
        cwd=REPO_ROOT,
        check=False,
    )
    if self_test.returncode != 0:
        print(
            "\nrun-native-tests: VERDICT: COVERAGE - the suite's own guards are "
            "broken; its verdict would mean nothing.",
            file=sys.stderr,
        )
        return EXIT_COVERAGE

    members = workspace_members()
    try:
        selected, expected_binaries = plan(members)
    except CoverageError as error:
        print(f"\nrun-native-tests: VERDICT: COVERAGE - {error}", file=sys.stderr)
        return EXIT_COVERAGE

    status, results = run_cargo(argv)

    if EXCLUDED_PACKAGES:
        print("\nrun-native-tests: not selected here:")
        for name, reason in sorted(EXCLUDED_PACKAGES.items()):
            print(f"  - {name}: {reason}")

    code, line = verdict(status, results, len(selected), expected_binaries)
    print(f"\n{line}", file=sys.stdout if code == EXIT_PASS else sys.stderr)
    return code


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
