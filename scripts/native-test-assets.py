#!/usr/bin/env python3
"""Copy pinned native-test assets from the primary checkout into a worktree."""

from __future__ import annotations

import argparse
import hashlib
import os
import subprocess
import sys
import tempfile
from pathlib import Path

LOCK = Path("tests/native-test-assets.lock")


class ProvisionError(Exception):
    """The worktree could not be provisioned safely."""


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def read_lock(repo_root: Path) -> dict[Path, str]:
    lock_path = repo_root / LOCK
    assets: dict[Path, str] = {}
    for number, raw_line in enumerate(lock_path.read_text().splitlines(), 1):
        line = raw_line.strip()
        if not line or line.startswith("#"):
            continue
        fields = line.split()
        if len(fields) != 2:
            raise ProvisionError(
                f"{lock_path}:{number}: expected SHA-256 and path"
            )
        expected, raw_path = fields
        path = Path(raw_path)
        if (
            len(expected) != 64
            or any(character not in "0123456789abcdef" for character in expected)
            or path.is_absolute()
            or ".." in path.parts
            or path in assets
        ):
            raise ProvisionError(
                f"{lock_path}:{number}: unsafe or duplicate asset entry"
            )
        assets[path] = expected
    if not assets:
        raise ProvisionError(f"{lock_path}: asset allowlist is empty")
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
        raise ProvisionError(
            f"could not inspect Git worktree metadata: {error}"
        ) from error


def repository_root(path: Path) -> Path:
    return Path(
        git_output(path, "rev-parse", "--path-format=absolute", "--show-toplevel")
    ).resolve()


def owning_checkout(repo_root: Path) -> Path:
    common_dir = Path(
        git_output(
            repo_root, "rev-parse", "--path-format=absolute", "--git-common-dir"
        )
    ).resolve()
    for line in git_output(repo_root, "worktree", "list", "--porcelain").splitlines():
        if not line.startswith("worktree "):
            continue
        candidate = Path(line.removeprefix("worktree ")).resolve()
        candidate_common = Path(
            git_output(
                candidate,
                "rev-parse",
                "--path-format=absolute",
                "--git-common-dir",
            )
        ).resolve()
        if candidate_common == common_dir and (candidate / ".git").is_dir():
            return candidate
    raise ProvisionError(
        "Git's worktree registry has no primary checkout for shared directory "
        f"{common_dir}"
    )


def verify(path: Path, expected: str, role: str) -> None:
    if not path.is_file() or path.is_symlink():
        raise ProvisionError(f"{role} is not a regular file: {path}")
    actual = sha256(path)
    if actual != expected:
        raise ProvisionError(
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


def asset_destination(
    repo_root: Path, relative: Path, target_dir: Path | None
) -> Path:
    if target_dir is not None and relative.parts[0] == "target":
        return target_dir.joinpath(*relative.parts[1:])
    return repo_root / relative


def provision(repo_root: Path, target_dir: Path | None = None) -> int:
    repo_root = repository_root(repo_root)
    if target_dir is not None:
        target_dir = target_dir.resolve()
        if not target_dir.is_relative_to(repo_root):
            raise ProvisionError(
                f"target directory is outside the destination worktree: {target_dir}"
            )
    assets = read_lock(repo_root)
    missing: list[tuple[Path, str]] = []
    for relative, expected in assets.items():
        destination = asset_destination(repo_root, relative, target_dir)
        if destination.exists() or destination.is_symlink():
            verify(destination, expected, "existing asset")
        else:
            missing.append((relative, expected))
    if not missing:
        return 0

    owner = owning_checkout(repo_root)
    if owner == repo_root:
        paths = "\n  ".join(str(path) for path, _ in missing)
        raise ProvisionError(
            "the primary checkout is missing pinned native-test assets:\n"
            f"  {paths}\n"
            "Run scripts/setup-conformance-tests.sh there first."
        )

    absent = [relative for relative, _ in missing if not (owner / relative).is_file()]
    if absent:
        paths = "\n  ".join(str(path) for path in absent)
        raise ProvisionError(
            f"the primary checkout {owner} is missing pinned native-test assets:\n"
            f"  {paths}\n"
            "Run scripts/setup-conformance-tests.sh there first."
        )

    for relative, expected in missing:
        source = owner / relative
        verify(source, expected, "primary asset")
        copy_verified(
            source,
            asset_destination(repo_root, relative, target_dir),
            expected,
        )
    return len(missing)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "worktree",
        type=Path,
        help="path within the linked worktree to provision",
    )
    parser.add_argument(
        "--target-dir",
        type=Path,
        help="checkout-local destination replacing target/ for locked target assets",
    )
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    try:
        repo_root = repository_root(args.worktree)
        copied = provision(repo_root, args.target_dir)
    except (OSError, ProvisionError) as error:
        print(f"native-test-assets: FAIL: {error}", file=sys.stderr)
        return 1
    print(
        f"native-test-assets: PASS: {copied} asset(s) copied into "
        f"{repo_root}"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
