#!/usr/bin/env python3
"""Hermetic contract tests for build-cache capacity and reclamation."""

from __future__ import annotations

import importlib.util
import os
import sys
import tempfile
from pathlib import Path

MODULE_PATH = Path(__file__).with_name("build-cache-policy.py")
SPEC = importlib.util.spec_from_file_location("build_cache_policy", MODULE_PATH)
assert SPEC is not None and SPEC.loader is not None
policy = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = policy
SPEC.loader.exec_module(policy)


def expect_error(action, fragment: str) -> None:
    try:
        action()
    except policy.PolicyError as error:
        assert fragment in str(error), (fragment, str(error))
    else:
        raise AssertionError(f"expected PolicyError containing {fragment!r}")


def fake_process(
    proc: Path,
    pid: int,
    name: str,
    cwd: Path,
    environment: bytes = b"",
) -> None:
    process = proc / str(pid)
    process.mkdir()
    (process / "comm").write_text(name + "\n")
    os.symlink(cwd, process / "cwd")
    (process / "environ").write_bytes(environment)


def main() -> None:
    with tempfile.TemporaryDirectory() as raw:
        root = Path(raw) / "repo"
        root.mkdir()
        for relative in policy.CACHE_RELATIVES:
            cache = root / relative
            cache.mkdir(parents=True)
            (cache / "artifact").write_bytes(b"x" * 32)
        proc = Path(raw) / "proc"
        proc.mkdir()

        targets = policy.cache_targets(root)
        assert targets == tuple(root / relative for relative in policy.CACHE_RELATIVES)
        assert all(target.exists() for target in targets)
        removed, reclaimed = policy.reclaim(root, proc)
        assert removed == 2
        assert reclaimed == 64
        assert all(not target.exists() for target in targets)
        assert root.exists()

    with tempfile.TemporaryDirectory() as raw:
        root = Path(raw) / "repo"
        root.mkdir()
        (root / "target/debug/incremental").mkdir(parents=True)
        proc = Path(raw) / "proc"
        proc.mkdir()
        fake_process(proc, 123, "cargo", root)
        expect_error(lambda: policy.reclaim(root, proc), "123:cargo")
        assert (root / "target/debug/incremental").exists()

    with tempfile.TemporaryDirectory() as raw:
        root = Path(raw) / "repo"
        outside = Path(raw) / "elsewhere"
        root.mkdir()
        outside.mkdir()
        (root / "target/clippy").mkdir(parents=True)
        proc = Path(raw) / "proc"
        proc.mkdir()
        fake_process(
            proc,
            456,
            "rustc",
            outside,
            f"CARGO_TARGET_DIR={root / 'target'}\0".encode(),
        )
        expect_error(lambda: policy.reclaim(root, proc), "456:rustc")
        assert (root / "target/clippy").exists()

    with tempfile.TemporaryDirectory() as raw:
        root = Path(raw) / "repo"
        outside = Path(raw) / "outside"
        root.mkdir()
        outside.mkdir()
        os.symlink(outside, root / "target")
        expect_error(lambda: policy.cache_targets(root), "symlinked target")
        assert outside.exists()

    with tempfile.TemporaryDirectory() as raw:
        root = Path(raw) / "repo"
        outside = Path(raw) / "outside"
        (root / "target/debug").mkdir(parents=True)
        outside.mkdir()
        os.symlink(outside, root / "target/debug/incremental")
        expect_error(lambda: policy.cache_targets(root), "symlinked cache path")
        assert outside.exists()

    expect_error(lambda: policy.capacity(Path.cwd(), 0), "at least 1")
    assert policy.capacity(Path.cwd(), 2).required == 28 * policy.GIB
    print("test-build-cache-policy: capacity and reclamation guards passed.")


if __name__ == "__main__":
    main()
