#!/usr/bin/env python3
"""Deterministic checks for the repository's GitHub Actions coverage contract."""

from pathlib import Path
import sys


ROOT = Path(__file__).resolve().parent.parent
QUALITY = ROOT / ".github/workflows/quality.yml"
DEFERRED = ROOT / ".github/workflows/deferred-tiers.yml"
CHECK_AND_TEST = ROOT / "scripts/check-and-test.sh"


def require(condition: bool, message: str) -> None:
    if not condition:
        raise AssertionError(message)


def workflow_text(path: Path) -> str:
    require(path.is_file(), f"missing workflow: {path.relative_to(ROOT)}")
    text = path.read_text(encoding="utf-8")
    require("\ton:" not in text, f"{path.name}: tabs are not valid indentation")
    return text


def main() -> int:
    quality = workflow_text(QUALITY)
    deferred = workflow_text(DEFERRED)
    check_and_test = CHECK_AND_TEST.read_text(encoding="utf-8")

    require("\non:\n  pull_request:\n  push:\n" in quality,
            "quality workflow must run for every pull request and main push")
    require("\n    paths:" not in quality and "\n    paths-ignore:" not in quality,
            "quality workflow must not filter paths")
    require("run: scripts/check.sh" in quality,
            "quality workflow must invoke the canonical format/lint gate")
    require("run: python3 scripts/test-ci-workflows.py" in quality,
            "quality workflow must enforce this coverage contract")
    require("scripts/check-and-test.sh" not in deferred,
            "deferred workflow must not duplicate format/lint and native gates")
    require("run: cargo test --quiet --tests" in deferred,
            "deferred workflow must retain native correctness coverage")
    require("scripts/check-wasm.sh" in deferred,
            "deferred workflow must retain the browser WASM gate")

    prebuild = "cargo test --quiet --tests --no-run"
    guarded_test = (
        "python3 scripts/run-umber-guarded.py \\\n"
        "  --timeout-seconds 1800 --max-rss-mib 6144 --term-grace-seconds 5 -- \\\n"
        "  cargo test --quiet --tests &"
    )
    require(prebuild in check_and_test,
            "check-and-test must prebuild the complete native suite")
    require(guarded_test in check_and_test,
            "check-and-test must run tests under the 6 GiB process-group guard")
    require(check_and_test.index(prebuild) < check_and_test.index(guarded_test),
            "check-and-test must finish the test build before guarded execution")
    require(
        check_and_test.index(guarded_test)
        < check_and_test.index("scripts/check.sh &"),
        "check-and-test must start guarded tests before the concurrent quality gate",
    )

    print("test-ci-workflows: PASS")
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except AssertionError as error:
        print(f"test-ci-workflows: FAIL: {error}", file=sys.stderr)
        sys.exit(1)
