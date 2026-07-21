#!/usr/bin/env python3
"""Exact, safe materialization and identity for pinned arXiv source bundles."""

from __future__ import annotations

import argparse
import gzip
import hashlib
import json
import os
import shutil
import tarfile
import tempfile
from pathlib import Path, PurePosixPath


def fail(message: str) -> None:
    raise ValueError(message)


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def _safe_name(raw: str) -> str:
    path = PurePosixPath(raw)
    if raw == "" or path.is_absolute() or ".." in path.parts:
        fail(f"unsafe archive member path: {raw!r}")
    normalized = path.as_posix()
    if normalized in (".", ""):
        fail(f"empty archive member path: {raw!r}")
    return normalized


def archive_members(archive: Path) -> list[dict[str, str | int]]:
    """Return the normalized regular-file inventory and reject ambiguous archives."""
    members: list[dict[str, str | int]] = []
    seen: set[str] = set()
    if tarfile.is_tarfile(archive):
        with tarfile.open(archive, "r:*") as source:
            for member in source:
                name = _safe_name(member.name)
                if member.isdir():
                    continue
                if not member.isfile():
                    fail(f"unsupported non-file archive member: {name}")
                if name in seen:
                    fail(f"duplicate archive member: {name}")
                seen.add(name)
                extracted = source.extractfile(member)
                if extracted is None:
                    fail(f"cannot read archive member: {name}")
                data = extracted.read()
                members.append({
                    "path": name,
                    "bytes": len(data),
                    "sha256": hashlib.sha256(data).hexdigest(),
                })
    else:
        try:
            with gzip.open(archive, "rb") as source:
                data = source.read()
        except (gzip.BadGzipFile, OSError) as error:
            fail(f"unsupported arXiv source bundle {archive}: {error}")
        members.append({
            "path": "main.tex",
            "bytes": len(data),
            "sha256": hashlib.sha256(data).hexdigest(),
        })
    return sorted(members, key=lambda member: str(member["path"]).encode())


def member_manifest_bytes(members: list[dict[str, str | int]]) -> bytes:
    return (json.dumps(members, ensure_ascii=False, separators=(",", ":")) + "\n").encode()


def source_identity(archive: Path, entrypoint: str) -> dict[str, str | int]:
    members = archive_members(archive)
    return {
        "archive_sha256": sha256_file(archive),
        "member_manifest_sha256": hashlib.sha256(member_manifest_bytes(members)).hexdigest(),
        "member_count": len(members),
        "entrypoint": entrypoint,
    }


def verify_view(archive: Path, view: Path) -> list[dict[str, str | int]]:
    expected = archive_members(archive)
    if not view.is_dir():
        fail(f"extracted source view is missing: {view}")
    actual_paths: list[str] = []
    for item in view.rglob("*"):
        if item.is_symlink() or (not item.is_file() and not item.is_dir()):
            fail(f"unsupported extracted-view entry: {item}")
        if item.is_file():
            actual_paths.append(item.relative_to(view).as_posix())
    expected_paths = [str(member["path"]) for member in expected]
    if sorted(actual_paths, key=str.encode) != expected_paths:
        missing = sorted(set(expected_paths) - set(actual_paths))
        extra = sorted(set(actual_paths) - set(expected_paths))
        fail(f"extracted source view inventory differs: missing={missing}, extra={extra}")
    for member in expected:
        path = view / str(member["path"])
        if path.stat().st_size != member["bytes"] or sha256_file(path) != member["sha256"]:
            fail(f"extracted source member differs from archive: {member['path']}")
    return expected


def view_members(view: Path) -> list[dict[str, str | int]]:
    if not view.is_dir():
        fail(f"extracted source view is missing: {view}")
    members = []
    for item in view.rglob("*"):
        if item.is_symlink() or (not item.is_file() and not item.is_dir()):
            fail(f"unsupported extracted-view entry: {item}")
        if item.is_file():
            members.append({
                "path": item.relative_to(view).as_posix(),
                "bytes": item.stat().st_size,
                "sha256": sha256_file(item),
            })
    return sorted(members, key=lambda member: str(member["path"]).encode())


def materialize(archive: Path, destination: Path) -> list[dict[str, str | int]]:
    """Atomically publish a new exact archive-derived view."""
    expected = archive_members(archive)
    destination.parent.mkdir(parents=True, exist_ok=True)
    temporary = Path(tempfile.mkdtemp(prefix=f".{destination.name}.new-", dir=destination.parent))
    try:
        if tarfile.is_tarfile(archive):
            with tarfile.open(archive, "r:*") as source:
                for member in source:
                    name = _safe_name(member.name)
                    if member.isdir():
                        (temporary / name).mkdir(parents=True, exist_ok=True)
                        continue
                    if not member.isfile():
                        fail(f"unsupported non-file archive member: {name}")
                    target = temporary / name
                    target.parent.mkdir(parents=True, exist_ok=True)
                    extracted = source.extractfile(member)
                    if extracted is None:
                        fail(f"cannot read archive member: {name}")
                    with target.open("xb") as output:
                        shutil.copyfileobj(extracted, output)
        else:
            with gzip.open(archive, "rb") as source, (temporary / "main.tex").open("xb") as output:
                shutil.copyfileobj(source, output)
        members = verify_view(archive, temporary)
        if members != expected:
            fail("archive inventory changed during materialization")
        if destination.exists():
            fail(f"refusing to replace existing destination without an explicit backup: {destination}")
        os.replace(temporary, destination)
        return members
    finally:
        if temporary.exists():
            shutil.rmtree(temporary)


def replace_view(archive: Path, view: Path, backup: Path, manifest: Path) -> None:
    """Recoverably replace a view, preserving a hashed inventory of old bytes."""
    if not view.is_dir():
        fail(f"extracted source view is missing: {view}")
    if backup.exists():
        fail(f"backup already exists: {backup}")
    old_members = view_members(view)
    archive_inventory = archive_members(archive)
    expected = {str(member["path"]): member for member in archive_inventory}
    observed = {str(member["path"]): member for member in old_members}
    backup.parent.mkdir(parents=True, exist_ok=True)
    manifest.parent.mkdir(parents=True, exist_ok=True)
    if view.parent.stat().st_dev != backup.parent.stat().st_dev:
        fail("source view and backup must be on the same filesystem")
    staged = view.with_name(f".{view.name}.replacement-{os.getpid()}")
    if staged.exists():
        fail(f"replacement staging path already exists: {staged}")
    materialize(archive, staged)
    os.replace(view, backup)
    try:
        os.replace(staged, view)
    except BaseException:
        os.replace(backup, view)
        raise
    receipt = {
        "schema": 1,
        "kind": "extracted-view-pre-pristine-backup",
        "source_archive": {
            "path": str(archive.resolve()),
            "sha256": sha256_file(archive),
            "member_manifest_sha256": hashlib.sha256(
                member_manifest_bytes(archive_inventory)
            ).hexdigest(),
        },
        "backup_path": str(backup.resolve()),
        "old_members": old_members,
        "extra_paths": sorted(set(observed) - set(expected)),
        "missing_paths": sorted(set(expected) - set(observed)),
        "changed_paths": sorted(
            name for name in set(expected) & set(observed)
            if expected[name] != observed[name]
        ),
        "provenance": (
            "Observed in the formerly mutable shared extraction before exact "
            "archive regeneration; non-archive producers and versions are unknown."
        ),
    }
    temporary = manifest.with_name(manifest.name + ".tmp")
    temporary.write_text(json.dumps(receipt, indent=2, sort_keys=True) + "\n")
    os.replace(temporary, manifest)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest="action", required=True)
    for action in ("verify", "materialize"):
        command = subparsers.add_parser(action)
        command.add_argument("archive", type=Path)
        command.add_argument("view", type=Path)
    identity = subparsers.add_parser("identity")
    identity.add_argument("archive", type=Path)
    identity.add_argument("entrypoint")
    replace = subparsers.add_parser("replace")
    replace.add_argument("archive", type=Path)
    replace.add_argument("view", type=Path)
    replace.add_argument("backup", type=Path)
    replace.add_argument("manifest", type=Path)
    arguments = parser.parse_args()
    try:
        if arguments.action == "verify":
            members = verify_view(arguments.archive, arguments.view)
            print(member_manifest_bytes(members).decode(), end="")
        elif arguments.action == "materialize":
            members = materialize(arguments.archive, arguments.view)
            print(member_manifest_bytes(members).decode(), end="")
        elif arguments.action == "replace":
            replace_view(arguments.archive, arguments.view, arguments.backup, arguments.manifest)
        else:
            print(json.dumps(source_identity(arguments.archive, arguments.entrypoint), sort_keys=True))
    except ValueError as error:
        raise SystemExit(str(error)) from error


if __name__ == "__main__":
    main()
