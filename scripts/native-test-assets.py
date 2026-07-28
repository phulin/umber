#!/usr/bin/env python3
"""Provision the mandatory native suite's pinned, gitignored assets.

The source checkout is resolved from Git's worktree registry.  Only paths in
`tests/native-test-assets.lock` may be copied, and every source and destination
is checked against that committed SHA-256 before use.  Copies are used instead
of symlinks or hard links so a test in one worktree cannot mutate the owning
checkout's evidence.
"""

from __future__ import annotations

import hashlib
import os
import subprocess
import tempfile
from pathlib import Path


class AssetError(Exception):
    """Pinned native-suite assets could not be safely provisioned."""


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def read_lock(repo_root: Path) -> dict[Path, str]:
    lock_path = repo_root / "tests/native-test-assets.lock"
    assets: dict[Path, str] = {}
    for number, raw_line in enumerate(lock_path.read_text().splitlines(), 1):
        line = raw_line.strip()
        if not line or line.startswith("#"):
            continue
        fields = line.split()
        if len(fields) != 2:
            raise AssetError(f"{lock_path}:{number}: expected SHA-256 and path")
        expected, raw_path = fields
        path = Path(raw_path)
        if (
            len(expected) != 64
            or any(character not in "0123456789abcdef" for character in expected)
            or path.is_absolute()
            or ".." in path.parts
            or path in assets
        ):
            raise AssetError(f"{lock_path}:{number}: unsafe or duplicate asset entry")
        assets[path] = expected
    if not assets:
        raise AssetError(f"{lock_path}: asset allowlist is empty")
    return assets


def git_output(repo_root: Path, *arguments: str) -> str:
    try:
        return subprocess.run(
            ["git", "-C", str(repo_root), *arguments],
            check=True,
            capture_output=True,
            text=True,
        ).stdout.strip()
    except (OSError, subprocess.CalledProcessError) as error:
        raise AssetError(f"could not inspect Git worktree metadata: {error}") from error


def owning_checkout(repo_root: Path) -> Path:
    common_dir = Path(
        git_output(repo_root, "rev-parse", "--path-format=absolute", "--git-common-dir")
    ).resolve()
    records = git_output(repo_root, "worktree", "list", "--porcelain").splitlines()
    candidates = [
        Path(line.removeprefix("worktree ")).resolve()
        for line in records
        if line.startswith("worktree ")
    ]
    for candidate in candidates:
        candidate_common = Path(
            git_output(candidate, "rev-parse", "--path-format=absolute", "--git-common-dir")
        ).resolve()
        if candidate_common == common_dir and (candidate / ".git").is_dir():
            return candidate
    raise AssetError(
        "Git's worktree registry has no primary checkout for shared directory "
        f"{common_dir}"
    )


def verify(path: Path, expected: str, role: str) -> None:
    if not path.is_file() or path.is_symlink():
        raise AssetError(f"{role} is not a regular file: {path}")
    actual = sha256(path)
    if actual != expected:
        raise AssetError(
            f"SHA-256 mismatch for {role} {path}: expected {expected}, got {actual}"
        )


def copy_verified(source: Path, destination: Path, expected: str) -> None:
    destination.parent.mkdir(parents=True, exist_ok=True)
    descriptor, temporary_name = tempfile.mkstemp(
        prefix=f".{destination.name}.", dir=destination.parent
    )
    temporary = Path(temporary_name)
    try:
        with os.fdopen(descriptor, "wb") as output, source.open("rb") as input_file:
            for chunk in iter(lambda: input_file.read(1024 * 1024), b""):
                output.write(chunk)
            output.flush()
            os.fsync(output.fileno())
        verify(temporary, expected, "copied asset")
        os.chmod(temporary, 0o444)
        os.replace(temporary, destination)
    finally:
        temporary.unlink(missing_ok=True)


def provision(repo_root: Path) -> int:
    repo_root = repo_root.resolve()
    assets = read_lock(repo_root)
    missing: list[tuple[Path, str]] = []
    for relative, expected in assets.items():
        destination = repo_root / relative
        if destination.exists() or destination.is_symlink():
            verify(destination, expected, "existing asset")
        else:
            missing.append((relative, expected))
    if not missing:
        return 0

    owner = owning_checkout(repo_root)
    if owner == repo_root:
        paths = "\n  ".join(str(path) for path, _ in missing)
        raise AssetError(
            "the primary checkout is missing mandatory native-suite assets:\n"
            f"  {paths}\n"
            "Materialize them there with scripts/setup-conformance-tests.sh, "
            "then rerun scripts/run-native-tests.py."
        )

    absent = [relative for relative, _ in missing if not (owner / relative).is_file()]
    if absent:
        paths = "\n  ".join(str(path) for path in absent)
        raise AssetError(
            f"the owning checkout {owner} is missing mandatory native-suite assets:\n"
            f"  {paths}\n"
            "Run scripts/setup-conformance-tests.sh in that checkout, then rerun "
            "scripts/run-native-tests.py in this worktree."
        )

    for relative, expected in missing:
        source = owner / relative
        verify(source, expected, "owning asset")
        copy_verified(source, repo_root / relative, expected)
    return len(missing)

