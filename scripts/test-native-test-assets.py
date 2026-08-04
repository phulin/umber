#!/usr/bin/env python3
"""Hermetic contract tests for worktree asset provisioning."""

from __future__ import annotations

import hashlib
import importlib.util
import subprocess
import sys
import tempfile
from pathlib import Path

MODULE_PATH = Path(__file__).with_name("provision.py")
SPEC = importlib.util.spec_from_file_location("provision", MODULE_PATH)
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
            f"{hashlib.sha256(content).hexdigest()} {path}\n"
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
    archive = b"pinned source archive\n"
    configure = b"configure\n"
    tex_web = b"tex.web\n"
    source_lock = primary / "tests/texlive-source.lock"
    source_lock.write_text(
        "distribution fixture-source\n"
        f"archive fixture-source.tar.xz {len(archive)} {hashlib.sha512(archive).hexdigest()} https://example.invalid/fixture-source.tar.xz\n"
        f"source configure {len(configure)} {hashlib.sha256(configure).hexdigest()}\n"
        f"source texk/web2c/tex.web {len(tex_web)} {hashlib.sha256(tex_web).hexdigest()}\n"
    )
    (primary / ".gitignore").write_text(
        "/target\n/third_party\n/tests/corpus/e2e/*.dvi\n"
    )
    run(
        "git",
        "add",
        ".gitignore",
        "tests/native-test-assets.lock",
        "tests/texlive-source.lock",
        cwd=primary,
    )
    run("git", "commit", "-q", "-m", "fixture", cwd=primary)
    for relative, content in entries.items():
        path = primary / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(content)
    source_cache = primary / "third_party/texlive-source"
    (source_cache / "src/texk/web2c").mkdir(parents=True)
    (source_cache / "fixture-source.tar.xz").write_bytes(archive)
    (source_cache / "src/configure").write_bytes(configure)
    (source_cache / "src/texk/web2c/tex.web").write_bytes(tex_web)
    worktree = directory / "worktree"
    run(
        "git",
        "worktree",
        "add",
        "-q",
        "-b",
        "test-worktree",
        str(worktree),
        cwd=primary,
    )
    return primary, worktree


def expect_error(action, fragment: str) -> None:
    try:
        action()
    except assets.ProvisionError as error:
        assert fragment in str(error), (fragment, str(error))
    else:
        raise AssertionError(f"expected ProvisionError containing {fragment!r}")


def main() -> None:
    entries = {
        "third_party/corpus/story.tex": b"story\n",
        "tests/corpus/e2e/story.expected.dvi": b"dvi bytes\n",
        "target/trip-oracles/trip/initex-command.jsonl": b"initex commands\n",
        "target/trip-oracles/trip/format-loaded-command.jsonl": b"loaded commands\n",
    }
    with tempfile.TemporaryDirectory() as raw_directory:
        primary, worktree = make_repository(Path(raw_directory), entries)

        copied = assets.provision_worktree(worktree)
        assert copied == len(entries)
        assert assets.provision_worktree(worktree) == 0
        assert (worktree / "third_party/texlive-source/src").is_symlink()
        assert (worktree / "third_party/texlive-source/fixture-source.tar.xz").is_symlink()
        for relative, content in entries.items():
            destination = worktree / relative
            assert destination.read_bytes() == content
            assert not destination.is_symlink()
            assert destination.stat().st_ino != (primary / relative).stat().st_ino
            assert destination.stat().st_mode & 0o222 == 0
        assert run("git", "status", "--short", cwd=worktree) == ""

        completed = subprocess.run(
            [sys.executable, str(MODULE_PATH), "worktree", str(worktree)],
            check=False,
            capture_output=True,
            text=True,
        )
        assert completed.returncode == 0, completed.stderr
        assert "PASS: 0 asset(s) provisioned" in completed.stdout

        changed = worktree / "third_party/corpus/story.tex"
        changed.chmod(0o644)
        changed.write_bytes(b"changed\n")
        expect_error(lambda: assets.provision_worktree(worktree), "existing asset")

    with tempfile.TemporaryDirectory() as raw_directory:
        primary, worktree = make_repository(Path(raw_directory), entries)
        namespaced_target = worktree / "target/audit-issue-target"
        copied = assets.provision_worktree(worktree, namespaced_target)
        assert copied == len(entries)
        for relative, content in entries.items():
            path = Path(relative)
            destination = (
                namespaced_target.joinpath(*path.parts[1:])
                if path.parts[0] == "target"
                else worktree / path
            )
            assert destination.read_bytes() == content
        assert not (worktree / "target/trip-oracles").exists()
        expect_error(
            lambda: assets.provision_worktree(worktree, worktree.parent / "outside"),
            "outside the worktree",
        )

    with tempfile.TemporaryDirectory() as raw_directory:
        primary, worktree = make_repository(Path(raw_directory), entries)
        (primary / "third_party/corpus/story.tex").unlink()
        expect_error(
            lambda: assets.provision_worktree(worktree),
            "Run python3 scripts/provision.py worktree",
        )

    with tempfile.TemporaryDirectory() as raw_directory:
        primary = Path(raw_directory)
        write_lock(primary, {"../escape": b"no"})
        expect_error(lambda: assets.read_native_asset_lock(primary), "unsafe")

    print("test-native-test-assets: PASS")


if __name__ == "__main__":
    try:
        main()
    except (AssertionError, OSError, subprocess.CalledProcessError) as error:
        print(f"test-native-test-assets: FAIL: {error}", file=sys.stderr)
        sys.exit(1)
