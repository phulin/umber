#!/usr/bin/env python3
"""Deterministic checks for the repository's GitHub Actions coverage contract."""

from pathlib import Path
import sys


ROOT = Path(__file__).resolve().parent.parent
QUALITY = ROOT / ".github/workflows/quality.yml"
DEFERRED = ROOT / ".github/workflows/deferred-tiers.yml"


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

    print("test-ci-workflows: PASS")
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except AssertionError as error:
        print(f"test-ci-workflows: FAIL: {error}", file=sys.stderr)
        sys.exit(1)
