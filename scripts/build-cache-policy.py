#!/usr/bin/env python3
"""Report build-cache capacity and optionally reclaim this checkout's caches."""

from __future__ import annotations

import argparse
import os
import shutil
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path

GIB = 1024**3
JOB_BUDGET_GIB = 12
RESERVE_GIB = 4
CACHE_RELATIVES = (Path("target/debug/incremental"), Path("target/clippy"))
BUILD_PROCESSES = {"cargo", "rustc", "rustdoc", "clippy-driver"}


class PolicyError(Exception):
    """A policy or safety check failed."""


@dataclass(frozen=True)
class Capacity:
    available: int
    required: int

    @property
    def sufficient(self) -> bool:
        return self.available >= self.required


def repository_root(start: Path) -> Path:
    result = subprocess.run(
        ["git", "rev-parse", "--show-toplevel"],
        cwd=start,
        check=False,
        capture_output=True,
        text=True,
    )
    if result.returncode != 0:
        raise PolicyError("not inside a Git checkout")
    return Path(result.stdout.strip()).resolve()


def cache_targets(root: Path) -> tuple[Path, ...]:
    """Return exact, non-symlinked cache targets contained by root."""
    root = root.resolve()
    target = root / "target"
    if target.is_symlink():
        raise PolicyError(f"refusing symlinked target directory: {target}")
    paths = []
    for relative in CACHE_RELATIVES:
        candidate = root / relative
        current = root
        for component in relative.parts:
            current /= component
            if current.is_symlink():
                raise PolicyError(f"refusing symlinked cache path: {current}")
        resolved = candidate.resolve(strict=False)
        if not resolved.is_relative_to(root):
            raise PolicyError(f"cache path escapes checkout: {candidate}")
        paths.append(candidate)
    return tuple(paths)


def directory_size(path: Path) -> int:
    if not path.exists():
        return 0
    total = 0
    for base, directories, files in os.walk(path, followlinks=False):
        base_path = Path(base)
        for name in directories + files:
            entry = base_path / name
            if entry.is_symlink():
                total += entry.lstat().st_size
            elif entry.is_file():
                total += entry.stat().st_size
    return total


def capacity(root: Path, jobs: int) -> Capacity:
    if jobs < 1:
        raise PolicyError("--jobs must be at least 1")
    available = shutil.disk_usage(root).free
    required = (jobs * JOB_BUDGET_GIB + RESERVE_GIB) * GIB
    return Capacity(available, required)


def active_build_processes(root: Path, proc: Path = Path("/proc")) -> list[str]:
    """Find Cargo-family processes that can own this checkout's target."""
    root = root.resolve()
    target = root / "target"
    active = []
    if not proc.is_dir():
        raise PolicyError("cannot verify active build processes: /proc is unavailable")
    for process in proc.iterdir():
        if not process.name.isdigit():
            continue
        try:
            name = (process / "comm").read_text().strip()
            cwd = (process / "cwd").resolve(strict=True)
            environment = (process / "environ").read_bytes().split(b"\0")
        except (FileNotFoundError, PermissionError, OSError):
            continue
        if name not in BUILD_PROCESSES:
            continue
        configured_target = None
        for entry in environment:
            if entry.startswith(b"CARGO_TARGET_DIR="):
                raw_target = os.fsdecode(entry.partition(b"=")[2])
                configured_target = Path(raw_target)
                if not configured_target.is_absolute():
                    configured_target = cwd / configured_target
                configured_target = configured_target.resolve(strict=False)
                break
        owns_target = configured_target == target or (
            configured_target is not None and configured_target.is_relative_to(target)
        )
        if cwd == root or cwd.is_relative_to(root) or owns_target:
            active.append(f"{process.name}:{name}")
    return sorted(active)


def reclaim(root: Path, proc: Path = Path("/proc")) -> tuple[int, int]:
    targets = cache_targets(root)
    active = active_build_processes(root, proc)
    if active:
        raise PolicyError(
            "refusing reclamation while build processes are active: "
            + ", ".join(active)
        )
    reclaimed = 0
    removed = 0
    for target in targets:
        if not target.exists():
            continue
        if not target.is_dir():
            raise PolicyError(f"cache target is not a directory: {target}")
        reclaimed += directory_size(target)
        shutil.rmtree(target)
        removed += 1
    return removed, reclaimed


def gibibytes(value: int) -> str:
    return f"{value / GIB:.1f} GiB"


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description=(
            "Check capacity for isolated worktree builds. The default is "
            "report-only; --reclaim deletes only this checkout's exact "
            "debug incremental and clippy caches."
        )
    )
    parser.add_argument("--jobs", type=int, default=1, help="new jobs planned")
    parser.add_argument(
        "--reclaim",
        action="store_true",
        help="reclaim exact cache targets after process and path checks",
    )
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    try:
        root = repository_root(Path.cwd())
        targets = cache_targets(root)
        before = capacity(root, args.jobs)
        print(f"checkout: {root}")
        print("mode: " + ("reclaim" if args.reclaim else "report-only"))
        for target in targets:
            print(f"cache: {target} ({gibibytes(directory_size(target))})")
        print(
            f"capacity: {gibibytes(before.available)} available; "
            f"{gibibytes(before.required)} required "
            f"for {args.jobs} job(s)"
        )
        if args.reclaim:
            removed, reclaimed = reclaim(root)
            print(f"reclaimed: {gibibytes(reclaimed)} from {removed} cache(s)")
        final = capacity(root, args.jobs)
        if not final.sufficient:
            print(
                f"REFUSE: need {gibibytes(final.required - final.available)} "
                "more free space",
                file=sys.stderr,
            )
            return 1
        print("PASS: capacity floor satisfied")
        return 0
    except PolicyError as error:
        print(f"REFUSE: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
