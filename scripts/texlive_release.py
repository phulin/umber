#!/usr/bin/env python3
"""Authenticated acquisition of the complete pinned TeX Live release tree."""

from __future__ import annotations

import hashlib
import os
import re
import shutil
import subprocess
import tempfile
import urllib.error
import urllib.request
from dataclasses import dataclass
from pathlib import Path
from urllib.parse import urljoin

import texlive

RUNTIME_SOURCE_LOCK = Path("tests/texlive-runtime-source.lock")
SNAPSHOT_LOCK = Path("tests/texlive-snapshot.lock")
CHUNK_BYTES = 1024 * 1024


@dataclass(frozen=True)
class ReleaseArchive:
    filename: str
    identity: texlive.Identity


@dataclass(frozen=True)
class IsoSlice:
    filename: str
    iso_bytes: int
    offset: int
    identity: texlive.Identity


@dataclass(frozen=True)
class ReleaseSource:
    distribution: str
    archive: ReleaseArchive
    package_database: IsoSlice


def _basename(raw: str, *, label: str) -> str:
    path = Path(raw)
    if path.name != raw or raw in ("", ".", ".."):
        raise texlive.TexliveError(f"invalid {label}: {raw}")
    return raw


def _nonnegative(raw: str, *, label: str) -> int:
    try:
        value = int(raw)
    except ValueError as error:
        raise texlive.TexliveError(f"invalid {label}: {raw}") from error
    if value < 0:
        raise texlive.TexliveError(f"invalid {label}: {raw}")
    return value


def read_release_source(path: Path) -> ReleaseSource:
    distribution = ""
    archive: ReleaseArchive | None = None
    package_database: IsoSlice | None = None
    for number, raw_line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        fields = raw_line.split()
        if not fields or fields[0].startswith("#"):
            continue
        if fields[0] == "distribution" and len(fields) == 2:
            distribution = _basename(fields[1], label="distribution")
        elif fields[0] == "archive" and len(fields) == 4:
            filename = _basename(fields[1], label="archive filename")
            size = _nonnegative(fields[2], label="archive length")
            if archive is not None or size == 0 or not texlive.valid_digest(fields[3], 128):
                raise texlive.TexliveError(f"{path}:{number}: invalid archive record")
            archive = ReleaseArchive(filename, texlive.Identity(size, fields[3]))
        elif fields[0] == "iso-slice" and len(fields) == 6:
            filename = _basename(fields[1], label="ISO filename")
            iso_bytes = _nonnegative(fields[2], label="ISO length")
            offset = _nonnegative(fields[3], label="ISO slice offset")
            size = _nonnegative(fields[4], label="ISO slice length")
            if (
                package_database is not None
                or size == 0
                or offset + size > iso_bytes
                or not texlive.valid_digest(fields[5], 128)
            ):
                raise texlive.TexliveError(f"{path}:{number}: invalid ISO slice record")
            package_database = IsoSlice(
                filename, iso_bytes, offset, texlive.Identity(size, fields[5])
            )
        else:
            raise texlive.TexliveError(f"{path}:{number}: invalid runtime-source record")
    if not distribution or archive is None or package_database is None:
        raise texlive.TexliveError(f"{path}: incomplete runtime-source lock")
    return ReleaseSource(distribution, archive, package_database)


def _mirrors(raw_mirrors: tuple[str, ...], offline: bool) -> tuple[str, ...]:
    mirrors: list[str] = []
    for raw in raw_mirrors:
        mirror = raw.rstrip("/") + "/"
        if not mirror.startswith(("https://", "http://127.0.0.1:", "http://localhost:")):
            raise texlive.TexliveError(f"unsafe TeX Live mirror: {raw}")
        if mirror in mirrors:
            raise texlive.TexliveError(f"duplicate TeX Live mirror: {raw}")
        mirrors.append(mirror)
    if not mirrors and not offline:
        raise texlive.TexliveError("runtime-source requires at least one --mirror")
    return tuple(mirrors)


def _artifact_urls(mirrors: tuple[str, ...], filename: str) -> tuple[str, ...]:
    return tuple(urljoin(mirror, filename) for mirror in mirrors)


def _download_resumable(
    urls: tuple[str, ...], destination: Path, identity: texlive.Identity, offline: bool
) -> None:
    if destination.exists():
        texlive.verify_file(destination, identity, "sha512", "cached release archive")
        return
    if offline:
        raise texlive.TexliveError(f"missing {destination} while running --offline")
    destination.parent.mkdir(parents=True, exist_ok=True)
    partial = destination.with_name(f".{destination.name}.part")
    if partial.exists() and partial.stat().st_size > identity.bytes:
        partial.unlink()
    if partial.exists() and partial.stat().st_size == identity.bytes:
        texlive.verify_file(partial, identity, "sha512", "completed release archive")
        os.replace(partial, destination)
        return
    failures: list[str] = []
    for url in urls:
        offset = partial.stat().st_size if partial.exists() else 0
        headers = {"User-Agent": "umber-texlive/1"}
        if offset:
            headers["Range"] = f"bytes={offset}-{identity.bytes - 1}"
        request = urllib.request.Request(url, headers=headers)
        try:
            with urllib.request.urlopen(request, timeout=60) as response:
                status = response.getcode()
                if offset:
                    expected = f"bytes {offset}-{identity.bytes - 1}/{identity.bytes}"
                    if status != 206 or response.headers.get("Content-Range") != expected:
                        failures.append(f"{url}: mirror cannot resume the pinned archive")
                        continue
                elif status == 206 and not _content_range(
                    response, 0, identity.bytes - 1, identity.bytes
                ):
                    failures.append(f"{url}: mirror returned an invalid full-file range")
                    continue
                elif status not in (200, 206):
                    failures.append(f"{url}: unexpected HTTP status {status}")
                    continue
                mode = "ab" if offset else "wb"
                with partial.open(mode) as output:
                    shutil.copyfileobj(response, output, CHUNK_BYTES)
        except (OSError, urllib.error.URLError) as error:
            failures.append(f"{url}: {error}")
            continue
        size = partial.stat().st_size
        if size < identity.bytes:
            failures.append(f"{url}: incomplete download ({size}/{identity.bytes} bytes)")
            continue
        if size > identity.bytes:
            partial.unlink()
            failures.append(f"{url}: response exceeded pinned length")
            continue
        try:
            texlive.verify_file(partial, identity, "sha512", "downloaded release archive")
        except texlive.TexliveError as error:
            partial.unlink()
            failures.append(f"{url}: {error}")
            continue
        os.replace(partial, destination)
        return
    raise texlive.TexliveError(
        f"all mirrors failed for {destination.name}: {'; '.join(failures)}"
    )


def _content_range(response, start: int, end: int, total: int) -> bool:
    raw = response.headers.get("Content-Range", "")
    match = re.fullmatch(r"bytes ([0-9]+)-([0-9]+)/([0-9]+)", raw)
    return match is not None and tuple(map(int, match.groups())) == (start, end, total)


def _download_iso_slice(
    urls: tuple[str, ...], destination: Path, pin: IsoSlice, offline: bool
) -> None:
    if destination.exists():
        texlive.verify_file(destination, pin.identity, "sha512", "cached package database")
        return
    if offline:
        raise texlive.TexliveError(f"missing {destination} while running --offline")
    destination.parent.mkdir(parents=True, exist_ok=True)
    end = pin.offset + pin.identity.bytes - 1
    failures: list[str] = []
    for url in urls:
        request = urllib.request.Request(
            url,
            headers={
                "User-Agent": "umber-texlive/1",
                "Range": f"bytes={pin.offset}-{end}",
            },
        )
        descriptor, temporary_name = tempfile.mkstemp(
            prefix=f".{destination.name}.", dir=destination.parent
        )
        temporary = Path(temporary_name)
        try:
            try:
                with urllib.request.urlopen(request, timeout=60) as response:
                    if response.getcode() != 206 or not _content_range(
                        response, pin.offset, end, pin.iso_bytes
                    ):
                        failures.append(f"{url}: mirror did not return the pinned ISO range")
                        continue
                    digest = hashlib.sha512()
                    total = 0
                    with os.fdopen(descriptor, "wb") as output:
                        descriptor = -1
                        for chunk in iter(lambda: response.read(CHUNK_BYTES), b""):
                            output.write(chunk)
                            digest.update(chunk)
                            total += len(chunk)
                if total != pin.identity.bytes or digest.hexdigest() != pin.identity.digest:
                    failures.append(f"{url}: ISO slice identity mismatch")
                    continue
                os.replace(temporary, destination)
                return
            except (OSError, urllib.error.URLError) as error:
                failures.append(f"{url}: {error}")
        finally:
            if descriptor >= 0:
                os.close(descriptor)
            temporary.unlink(missing_ok=True)
    raise texlive.TexliveError(
        f"all mirrors failed for {destination.name}: {'; '.join(failures)}"
    )


def _verify_installed(
    root: Path,
    package_database: Path,
    package_identity: texlive.Identity,
    snapshot_lock: Path,
    distribution: str,
) -> None:
    if root.is_symlink() or not (root / "texmf-dist").is_dir():
        raise texlive.TexliveError(f"incomplete runtime source: {root}")
    actual_distribution, _ = texlive.verify_runtime_tree(root / "texmf-dist", snapshot_lock)
    if actual_distribution != distribution:
        raise texlive.TexliveError(
            f"runtime source distribution {distribution} differs from {actual_distribution}"
        )
    texlive.verify_file(package_database, package_identity, "sha512", "package database")


def ensure_runtime_source(
    repo_root: Path,
    raw_mirrors: tuple[str, ...],
    offline: bool = False,
    *,
    release_lock: Path | None = None,
    snapshot_lock: Path | None = None,
) -> Path:
    repo_root = texlive.repository_root(repo_root)
    primary = texlive.primary_checkout(repo_root)
    release_lock = release_lock or primary / RUNTIME_SOURCE_LOCK
    snapshot_lock = snapshot_lock or primary / SNAPSHOT_LOCK
    pin = read_release_source(release_lock)
    mirrors = _mirrors(raw_mirrors, offline)
    third_party = primary / "third_party"
    archive = third_party / pin.archive.filename
    root = third_party / pin.distribution
    package_cache = third_party / f"{pin.distribution}.tlpdb"
    package_database = root / "tlpkg/texlive.tlpdb"
    _download_resumable(
        _artifact_urls(mirrors, pin.archive.filename), archive, pin.archive.identity, offline
    )
    if not package_cache.exists() and package_database.is_file():
        try:
            texlive.verify_file(
                package_database,
                pin.package_database.identity,
                "sha512",
                "installed package database",
            )
            shutil.copyfile(package_database, package_cache)
        except texlive.TexliveError:
            pass
    _download_iso_slice(
        _artifact_urls(mirrors, pin.package_database.filename),
        package_cache,
        pin.package_database,
        offline,
    )
    try:
        _verify_installed(
            root,
            package_database,
            pin.package_database.identity,
            snapshot_lock,
            pin.distribution,
        )
        return root
    except texlive.TexliveError:
        pass

    third_party.mkdir(parents=True, exist_ok=True)
    temporary = Path(tempfile.mkdtemp(prefix=f".{pin.distribution}.", dir=third_party))
    stale = third_party / f".{pin.distribution}.replaced"
    try:
        subprocess.run(
            ["tar", "-xJf", str(archive), "-C", str(temporary), "--strip-components=1"],
            check=True,
        )
        staged_database = temporary / "tlpkg/texlive.tlpdb"
        staged_database.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(package_cache, staged_database)
        _verify_installed(
            temporary,
            staged_database,
            pin.package_database.identity,
            snapshot_lock,
            pin.distribution,
        )
        if stale.exists():
            raise texlive.TexliveError(f"stale interrupted replacement requires review: {stale}")
        if root.exists() or root.is_symlink():
            os.replace(root, stale)
        try:
            os.replace(temporary, root)
        except OSError:
            if stale.exists():
                os.replace(stale, root)
            raise
        if stale.exists():
            shutil.rmtree(stale)
    except (OSError, subprocess.CalledProcessError) as error:
        raise texlive.TexliveError(f"could not install {pin.distribution}: {error}") from error
    finally:
        if temporary.exists():
            shutil.rmtree(temporary)
    _verify_installed(
        root,
        package_database,
        pin.package_database.identity,
        snapshot_lock,
        pin.distribution,
    )
    return root
