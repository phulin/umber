#!/usr/bin/env python3
"""Hermetic contract tests for native-test worktree asset provisioning."""

from __future__ import annotations

import importlib.util
import subprocess
import sys
import tempfile
from pathlib import Path

MODULE_PATH = Path(__file__).with_name("native-test-assets.py")
SPEC = importlib.util.spec_from_file_location("native_test_assets", MODULE_PATH)
assert SPEC is not None and SPEC.loader is not None
assets = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(assets)


def run(*command: str, cwd: Path) -> str:
    return subprocess.run(
        command, cwd=cwd, check=True, capture_output=True, text=True
    ).stdout.strip()


def write_lock(root: Path, entries: dict[str, bytes]) -> None:
    lock = root / "tests/native-test-assets.lock"
    lock.parent.mkdir(parents=True, exist_ok=True)
    lock.write_text(
        "".join(
            f"{assets.hashlib.sha256(content).hexdigest()} {path}\n"
            for path, content in entries.items()
        )
    )


def make_repository(directory: Path, entries: dict[str, bytes]) -> tuple[Path, Path]:
    primary = directory / "primary"
    primary.mkdir()
    run("git", "init", "-q", cwd=primary)
    run("git", "config", "user.email", "test@example.invalid", cwd=primary)
    run("git", "config", "user.name", "Test", cwd=primary)
    write_lock(primary, entries)
    (primary / ".gitignore").write_text("/third_party\n/tests/corpus/e2e/*.dvi\n")
    run("git", "add", ".gitignore", "tests/native-test-assets.lock", cwd=primary)
    run("git", "commit", "-q", "-m", "fixture", cwd=primary)
    for relative, content in entries.items():
        path = primary / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(content)
    worktree = directory / "worktree"
    run("git", "worktree", "add", "-q", "-b", "test-worktree", str(worktree), cwd=primary)
    return primary, worktree


def expect_error(action, fragment: str) -> None:
    try:
        action()
    except assets.AssetError as error:
        assert fragment in str(error), (fragment, str(error))
    else:
        raise AssertionError(f"expected AssetError containing {fragment!r}")


def main() -> None:
    entries = {
        "third_party/corpus/story.tex": b"story\n",
        "tests/corpus/e2e/story.expected.dvi": b"dvi bytes\n",
    }
    with tempfile.TemporaryDirectory() as raw_directory:
        primary, worktree = make_repository(Path(raw_directory), entries)

        copied = assets.provision(worktree)
        assert copied == len(entries)
        assert assets.provision(worktree) == 0
        for relative, content in entries.items():
            destination = worktree / relative
            assert destination.read_bytes() == content
            assert not destination.is_symlink()
            assert destination.stat().st_ino != (primary / relative).stat().st_ino
        assert run("git", "status", "--short", cwd=worktree) == ""

        changed = worktree / "third_party/corpus/story.tex"
        changed.chmod(0o644)
        changed.write_bytes(b"changed\n")
        expect_error(lambda: assets.provision(worktree), "existing asset")

    with tempfile.TemporaryDirectory() as raw_directory:
        primary, worktree = make_repository(Path(raw_directory), entries)
        (primary / "third_party/corpus/story.tex").unlink()
        expect_error(
            lambda: assets.provision(worktree),
            "Run scripts/setup-conformance-tests.sh in that checkout",
        )

    with tempfile.TemporaryDirectory() as raw_directory:
        primary = Path(raw_directory)
        write_lock(primary, {"../escape": b"no"})
        expect_error(lambda: assets.read_lock(primary), "unsafe or duplicate")

    print("test-native-test-assets: provisioning guards passed.")


if __name__ == "__main__":
    try:
        main()
    except (AssertionError, OSError, subprocess.CalledProcessError) as error:
        print(f"test-native-test-assets: FAILED: {error}", file=sys.stderr)
        sys.exit(1)

