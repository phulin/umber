#!/usr/bin/env python3
"""Exercise the registered deferred entry points without their optional tools.

The test deliberately removes Cargo from PATH. Every new LaTeX entry point
must then record a BLOCKED attempt, emit a verdict, and be discoverable from
the common registry rather than quietly treating the missing tool as success.
"""

from __future__ import annotations

import importlib.util
import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
SPEC = importlib.util.spec_from_file_location(
    "tier_stamp", REPO_ROOT / "scripts" / "tier_stamp.py"
)
assert SPEC is not None and SPEC.loader is not None
tier_stamp = importlib.util.module_from_spec(SPEC)
sys.modules["tier_stamp"] = tier_stamp
SPEC.loader.exec_module(tier_stamp)

LATEX_TIERS = (
    "check-latex-corpus.sh",
    "check-latex-wasm.sh",
    "check-latex-parity.sh",
)


def main() -> int:
    stamp_dir = REPO_ROOT / ".tier-stamps"
    backup = REPO_ROOT / ".tier-stamps-johp-279-test-backup"
    if backup.exists():
        raise AssertionError(f"refusing to replace test backup: {backup}")
    if stamp_dir.exists():
        stamp_dir.rename(backup)

    try:
        with tempfile.TemporaryDirectory(prefix="johp-279-deferred-tiers-") as raw_bin:
            test_bin = Path(raw_bin)
            for command in ("dirname", "git", "python3"):
                source = shutil.which(command)
                if source is None:
                    raise AssertionError(f"test host has no {command}")
                (test_bin / command).symlink_to(source)
            environment = os.environ | {"PATH": str(test_bin)}
            for tier in LATEX_TIERS:
                spec = tier_stamp.TIERS.get(tier)
                if spec is None:
                    raise AssertionError(f"{tier} is not registered")
                if not (REPO_ROOT / spec.command).is_file():
                    raise AssertionError(f"{tier} command is absent: {spec.command}")

                result = subprocess.run(
                    ["/bin/bash", spec.command],
                    cwd=REPO_ROOT,
                    env=environment,
                    check=False,
                    capture_output=True,
                    text=True,
                )
                output = result.stdout + result.stderr
                if result.returncode != 4:
                    raise AssertionError(f"{tier} returned {result.returncode}: {output}")
                if "VERDICT: BLOCKED" not in output or "cargo" not in output:
                    raise AssertionError(f"{tier} hid its missing prerequisite: {output}")
                stamp = tier_stamp.load_stamp(tier)
                if stamp is None or stamp["status"] != tier_stamp.STATUS_BLOCKED:
                    raise AssertionError(f"{tier} did not write a BLOCKED stamp: {stamp}")
                report = tier_stamp.describe(tier, stamp, tier_stamp.Position("at HEAD", True))
                if report.state != tier_stamp.STATE_BLOCKED:
                    raise AssertionError(f"{tier} stamp was not discoverable as BLOCKED")
    finally:
        shutil.rmtree(stamp_dir, ignore_errors=True)
        if backup.exists():
            backup.rename(stamp_dir)

    print("test-deferred-tier-entrypoints: registration, stamps, blockers, and verdicts hold")
    return 0


if __name__ == "__main__":
    sys.exit(main())
