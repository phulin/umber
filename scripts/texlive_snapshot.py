#!/usr/bin/env python3
"""Acquire dated TeX Live runfiles from authenticated upstream package archives.

The compressed TLPDB is retained as the source inventory. Its SHA-512 is
recorded on first acquisition; every package archive is checked against the
length and SHA-512 declared by that database before any member is extracted.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import lzma
import os
import re
import shutil
import sys
import tarfile
import tempfile
import threading
import urllib.error
import urllib.request
from concurrent.futures import ThreadPoolExecutor
from dataclasses import dataclass
from pathlib import Path, PurePosixPath
from urllib.parse import urljoin, urlsplit

import texlive

SNAPSHOT_DATES = {2023: "2023-05-21", 2024: "2024-03-14", 2025: "2025-08-03", 2026: "2026-03-02"}
DEFAULT_CACHE_ROOT = Path("target/texlive-years")
MAX_DATABASE_BYTES = 16 * 1024 * 1024
MAX_EXPANDED_DATABASE_BYTES = 128 * 1024 * 1024
MAX_ARCHIVE_BYTES = 512 * 1024 * 1024
MAX_MEMBER_BYTES = 512 * 1024 * 1024
MAX_PACKAGE_FILES = 20_000
MAX_RUNTIME_FILES = 500_000
MAX_RUNTIME_BYTES = 32 * 1024 * 1024 * 1024
DEFAULT_MAX_DOWNLOAD_BYTES = 4 * 1024 * 1024 * 1024
MAX_WORKERS = 8
CHUNK_BYTES = 1024 * 1024
USER_AGENT = "curl/8.5.0"  # The historical archive rejects Python's default user agent.


@dataclass(frozen=True)
class Package:
    name: str
    revision: int
    identity: texlive.Identity
    # (archive member name, installed path relative to snapshot root)
    runfiles: tuple[tuple[str, str], ...]
    all_members: tuple[str, ...]

    @property
    def archive_name(self) -> str:
        return f"{self.name}.r{self.revision}.tar.xz"


@dataclass(frozen=True)
class SnapshotResult:
    root: Path
    receipt: Path
    snapshot_date: str
    tlpdb_identity: texlive.Identity
    package_count: int
    file_count: int


def _safe_path(raw: str) -> str:
    path = PurePosixPath(raw)
    if (
        not raw
        or raw.startswith("/")
        or "\\" in raw
        or any(ord(character) < 32 for character in raw)
        or any(part in ("", ".", "..") for part in raw.split("/"))
        or str(path) != raw
    ):
        raise texlive.TexliveError(f"unsafe TeX Live path: {raw!r}")
    return raw


def _runtime_path(raw: str) -> str | None:
    _safe_path(raw)
    if raw.startswith("RELOC/"):
        return "texmf-dist/" + raw.removeprefix("RELOC/")
    if raw.startswith("texmf-dist/"):
        return raw
    return None


def _archive_member(raw: str, relocated: bool) -> str:
    return raw.removeprefix("RELOC/") if relocated else raw


def parse_tlpdb(compressed: bytes, *, year: int | None = None) -> tuple[Package, ...]:
    """Select every platform-independent package with installable TEXMF runfiles."""
    if len(compressed) > MAX_DATABASE_BYTES:
        raise texlive.TexliveError("TLPDB exceeds compressed byte limit")
    decoder = lzma.LZMADecompressor()
    try:
        raw = decoder.decompress(compressed, max_length=MAX_EXPANDED_DATABASE_BYTES + 1)
    except lzma.LZMAError as error:
        raise texlive.TexliveError(f"invalid compressed TLPDB: {error}") from error
    if len(raw) > MAX_EXPANDED_DATABASE_BYTES or not decoder.eof or decoder.unused_data:
        raise texlive.TexliveError("TLPDB exceeds expanded byte limit or has trailing data")
    try:
        database = raw.decode("utf-8")
    except UnicodeError as error:
        raise texlive.TexliveError("TLPDB is not UTF-8") from error
    if year is not None:
        releases = [line.removeprefix("depend release/")
                    for block in database.split("\n\n")
                    if "name 00texlive.config" in block.splitlines()
                    for line in block.splitlines() if line.startswith("depend release/")]
        if releases != [str(year)]:
            raise texlive.TexliveError(f"TLPDB release mismatch: requested {year}, declared {releases}")
    packages: list[Package] = []
    names: set[str] = set()
    installed: set[str] = set()
    total_files = 0
    total_bytes = 0
    for block in database.split("\n\n"):
        if not block.strip():
            continue
        fields: dict[str, str] = {}
        runfiles: list[str] = []
        in_runfiles = False
        for line in block.splitlines():
            if line.startswith(" "):
                if in_runfiles:
                    runfiles.append(line.strip().split(" ", 1)[0])
                continue
            key, separator, value = line.partition(" ")
            if not separator:
                raise texlive.TexliveError(f"invalid TLPDB field: {line[:80]}")
            if key in ("name", "revision", "category", "relocated", "containersize", "containerchecksum", "binfiles"):
                if key in fields and key != "binfiles":
                    raise texlive.TexliveError(f"duplicate TLPDB {key} field")
                fields[key] = value
            in_runfiles = key == "runfiles"
        name = fields.get("name", "")
        if not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9_.+-]*", name):
            raise texlive.TexliveError(f"invalid TLPDB package name: {name!r}")
        if name in names:
            raise texlive.TexliveError(f"duplicate TLPDB package: {name}")
        names.add(name)
        # This synthetic installer image has no downloadable container. Its
        # generated maps/configuration are built by the installer, not an
        # upstream package archive.
        if name == "00texlive.image" or "binfiles" in fields or not runfiles or fields.get("category") not in ("Package", "TLCore"):
            continue
        relocated = fields.get("relocated") == "1"
        selected: list[tuple[str, str]] = []
        all_members: list[str] = []
        for raw_path in runfiles:
            _safe_path(raw_path)
            all_members.append(_archive_member(raw_path, relocated))
            target = _runtime_path(raw_path)
            if target is not None:
                selected.append((_archive_member(raw_path, relocated), target))
        if not selected:
            continue
        try:
            revision = int(fields["revision"])
            size = int(fields["containersize"])
            checksum = fields["containerchecksum"]
        except (KeyError, ValueError) as error:
            raise texlive.TexliveError(f"missing archive identity for {name}") from error
        if revision < 0 or size <= 0 or size > MAX_ARCHIVE_BYTES or not texlive.valid_digest(checksum, 128):
            raise texlive.TexliveError(f"invalid archive identity for {name}")
        if len(selected) > MAX_PACKAGE_FILES:
            raise texlive.TexliveError(f"too many runfiles for {name}")
        for member, target in selected:
            _safe_path(member)
            if target in installed:
                raise texlive.TexliveError(f"duplicate installed runfile: {target}")
            installed.add(target)
        total_files += len(selected)
        total_bytes += size
        if total_files > MAX_RUNTIME_FILES or total_bytes > MAX_RUNTIME_BYTES:
            raise texlive.TexliveError("TLPDB runtime profile exceeds limits")
        if len(all_members) != len(set(all_members)):
            raise texlive.TexliveError(f"duplicate archive runfile in {name}")
        packages.append(Package(name, revision, texlive.Identity(size, checksum), tuple(selected), tuple(all_members)))
    if not packages:
        raise texlive.TexliveError("TLPDB contains no installable runtime packages")
    return tuple(packages)


def _source_url(year: int, mirror: str | None) -> str:
    if year not in SNAPSHOT_DATES:
        raise texlive.TexliveError(f"unsupported TeX Live year: {year}")
    if mirror is None:
        date = SNAPSHOT_DATES[year].replace("-", "/")
        return f"https://texlive.info/tlnet-archive/{date}/tlnet/"
    split = urlsplit(mirror)
    if split.scheme != "https" and not (split.scheme == "http" and split.hostname in ("localhost", "127.0.0.1")):
        raise texlive.TexliveError(f"unsafe TeX Live mirror URL: {mirror}")
    if split.username or split.password or split.query or split.fragment:
        raise texlive.TexliveError(f"unsafe TeX Live mirror URL: {mirror}")
    return mirror.rstrip("/") + "/"


def _download(url: str, destination: Path, *, expected: texlive.Identity | None, limit: int, offline: bool) -> texlive.Identity:
    if destination.exists():
        if expected is None:
            if destination.stat().st_size > limit:
                raise texlive.TexliveError(f"cached object exceeds byte limit: {destination}")
            return texlive.Identity(destination.stat().st_size, texlive.hash_file(destination, "sha512"))
        texlive.verify_file(destination, expected, "sha512", "cached archive")
        return expected
    if offline:
        raise texlive.TexliveError(f"missing {destination} while running offline")
    destination.parent.mkdir(parents=True, exist_ok=True)
    partial = destination.with_name(f".{destination.name}.part")
    if partial.exists() and partial.stat().st_size > limit:
        partial.unlink()
    for attempt in range(3):
        offset = partial.stat().st_size if partial.exists() else 0
        if expected is not None and offset == expected.bytes:
            texlive.verify_file(partial, expected, "sha512", "completed archive")
            os.replace(partial, destination)
            return expected
        headers = {"User-Agent": USER_AGENT}
        if offset:
            headers["Range"] = f"bytes={offset}-"
        request = urllib.request.Request(url, headers=headers)
        try:
            with urllib.request.urlopen(request, timeout=60) as response:
                status = response.getcode()
                if offset and status == 206:
                    content_range = response.headers.get("Content-Range", "")
                    match = re.fullmatch(r"bytes ([0-9]+)-([0-9]+)/([0-9]+)", content_range)
                    if (
                        match is None
                        or int(match.group(1)) != offset
                        or int(match.group(2)) < offset
                        or int(match.group(3)) > limit
                        or (expected is not None and int(match.group(3)) != expected.bytes)
                    ):
                        raise texlive.TexliveError(f"invalid resume response from {url}")
                    mode = "ab"
                elif status == 200:
                    mode = "wb"
                else:
                    raise texlive.TexliveError(f"unexpected HTTP status {status} from {url}")
                total = offset if mode == "ab" else 0
                with partial.open(mode) as output:
                    for chunk in iter(lambda: response.read(CHUNK_BYTES), b""):
                        total += len(chunk)
                        if total > limit:
                            raise texlive.TexliveError(f"download exceeds byte limit: {url}")
                        output.write(chunk)
            if expected is None or total == expected.bytes:
                break
            if total > expected.bytes:
                raise texlive.TexliveError(f"download exceeds declared length: {url}")
            if attempt == 2:
                raise texlive.TexliveError(f"short archive download from {url}: {total}/{expected.bytes} bytes")
        except (OSError, urllib.error.URLError) as error:
            if attempt == 2:
                raise texlive.TexliveError(f"could not download {url}: {error}") from error
    size = partial.stat().st_size
    if expected is not None and size != expected.bytes:
        raise texlive.TexliveError(f"archive length mismatch for {url}: expected {expected.bytes}, got {size}")
    identity = texlive.Identity(size, texlive.hash_file(partial, "sha512"))
    if expected is not None and identity != expected:
        partial.unlink()
        raise texlive.TexliveError(f"archive SHA-512 mismatch for {url}")
    os.replace(partial, destination)
    return identity


def _atomic_json(path: Path, value: dict) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    fd, name = tempfile.mkstemp(prefix=f".{path.name}.", dir=path.parent)
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as output:
            json.dump(value, output, sort_keys=True, indent=2)
            output.write("\n")
        os.replace(name, path)
    finally:
        Path(name).unlink(missing_ok=True)


def _safe_parent(root: Path, relative: str) -> Path:
    current = root
    if current.is_symlink():
        raise texlive.TexliveError(f"symlink snapshot root: {root}")
    current.mkdir(parents=True, exist_ok=True)
    for part in PurePosixPath(relative).parts[:-1]:
        current = current / part
        if current.is_symlink():
            raise texlive.TexliveError(f"symlink runtime directory: {current}")
        current.mkdir(exist_ok=True)
    return current


def _extract_package(archive: Path, root: Path, package: Package) -> tuple[tuple[str, ...], int]:
    expected = dict(package.runfiles)
    allowed = set(package.all_members)
    seen: set[str] = set()
    written: list[str] = []
    extracted_bytes = 0
    try:
        with tarfile.open(archive, "r|xz") as source:
            for member in source:
                name = _safe_path(member.name.rstrip("/") if member.isdir() else member.name)
                if name in seen:
                    raise texlive.TexliveError(f"duplicate member {name} in {archive.name}")
                seen.add(name)
                if member.isdir():
                    continue
                if not member.isfile():
                    raise texlive.TexliveError(f"nonregular member {name} in {archive.name}")
                if member.size > MAX_MEMBER_BYTES:
                    raise texlive.TexliveError(f"oversized member {name} in {archive.name}")
                if name == f"tlpkg/tlpobj/{package.name}.tlpobj":
                    continue
                if name not in allowed:
                    raise texlive.TexliveError(f"unlisted archive member {name} in {archive.name}")
                target_name = expected.get(name)
                if target_name is None:
                    continue
                target = root / target_name
                _safe_parent(root, target_name)
                fd, temporary_name = tempfile.mkstemp(prefix=f".{target.name}.", dir=target.parent)
                try:
                    stream = source.extractfile(member)
                    if stream is None:
                        raise texlive.TexliveError(f"cannot read member {name}")
                    with os.fdopen(fd, "wb") as output:
                        fd = -1
                        shutil.copyfileobj(stream, output, CHUNK_BYTES)
                    if Path(temporary_name).stat().st_size != member.size:
                        raise texlive.TexliveError(f"short member {name} in {archive.name}")
                    os.replace(temporary_name, target)
                finally:
                    if fd >= 0:
                        os.close(fd)
                    Path(temporary_name).unlink(missing_ok=True)
                written.append(target_name)
                extracted_bytes += member.size
    except (tarfile.TarError, lzma.LZMAError, OSError) as error:
        raise texlive.TexliveError(f"cannot extract {archive}: {error}") from error
    if allowed != seen.intersection(allowed):
        missing = sorted(allowed - seen)
        raise texlive.TexliveError(f"archive {archive.name} omits {missing[:3]}")
    return tuple(written), extracted_bytes


def _package_inventory(root: Path, package: Package) -> tuple[int, str]:
    digest = hashlib.sha256()
    total = 0
    for _, relative in sorted(package.runfiles, key=lambda item: item[1]):
        file = root / relative
        if not file.is_file() or file.is_symlink():
            raise texlive.TexliveError(f"missing or unsafe installed runfile: {file}")
        size = file.stat().st_size
        total += size
        digest.update(f"{relative}\t{size}\t{texlive.hash_file(file)}\n".encode())
    return total, digest.hexdigest()


def _reuse_archive(root: Path, cache_root: Path, year: int, package: Package) -> None:
    destination = root / "archives" / package.archive_name
    if destination.exists():
        return
    for other_year in SNAPSHOT_DATES:
        candidate = cache_root / str(other_year) / "archives" / package.archive_name
        if not candidate.is_file():
            continue
        try:
            texlive.verify_file(candidate, package.identity, "sha512", "shared snapshot archive")
        except texlive.TexliveError:
            continue
        destination.parent.mkdir(parents=True, exist_ok=True)
        try:
            os.link(candidate, destination)
        except OSError:
            shutil.copyfile(candidate, destination)
        return


def _inventory(root: Path, packages: tuple[Package, ...], path: Path) -> tuple[int, str]:
    expected = sorted(target for package in packages for _, target in package.runfiles)
    fd, temporary_name = tempfile.mkstemp(prefix=f".{path.name}.", dir=path.parent)
    aggregate = hashlib.sha256()
    total_bytes = 0
    try:
        with os.fdopen(fd, "wb") as output:
            for relative in expected:
                file = root / relative
                if not file.is_file() or file.is_symlink():
                    raise texlive.TexliveError(f"missing or unsafe installed runfile: {file}")
                total_bytes += file.stat().st_size
                if total_bytes > MAX_RUNTIME_BYTES:
                    raise texlive.TexliveError("installed runtime exceeds byte limit")
                record = f"{relative}\t{file.stat().st_size}\t{texlive.hash_file(file)}\n".encode()
                output.write(record)
                aggregate.update(record)
        os.replace(temporary_name, path)
    finally:
        Path(temporary_name).unlink(missing_ok=True)
    return len(expected), aggregate.hexdigest()


def _verify_tree_shape(root: Path, expected: set[str]) -> None:
    runtime = root / "texmf-dist"
    if root.is_symlink() or runtime.is_symlink() or not runtime.is_dir():
        raise texlive.TexliveError(f"missing or unsafe runtime directory: {runtime}")
    actual: set[str] = set()
    for directory, directories, files in os.walk(runtime, followlinks=False):
        for name in directories:
            child = Path(directory) / name
            if child.is_symlink():
                raise texlive.TexliveError(f"symlink runtime directory: {child}")
        for name in files:
            child = Path(directory) / name
            if child.is_symlink() or not child.is_file():
                raise texlive.TexliveError(f"unsafe runtime file: {child}")
            relative = child.relative_to(root).as_posix()
            if relative not in expected:
                raise texlive.TexliveError(f"unlisted runtime file: {child}")
            actual.add(relative)
    if actual != expected:
        missing = sorted(expected - actual)
        raise texlive.TexliveError(f"runtime tree omits {missing[:3]}")


def verify_snapshot(year: int, cache_root: Path = DEFAULT_CACHE_ROOT) -> SnapshotResult:
    root = cache_root / str(year)
    receipt_path = root / "acquisition.json"
    try:
        receipt = json.loads(receipt_path.read_text(encoding="utf-8"))
        pin = receipt["tlpdb"]
        identity = texlive.Identity(pin["bytes"], pin["sha512"])
    except (OSError, KeyError, TypeError, ValueError, json.JSONDecodeError) as error:
        raise texlive.TexliveError(f"missing or invalid snapshot receipt: {receipt_path}") from error
    if receipt.get("year") != year or receipt.get("snapshot_date") != SNAPSHOT_DATES.get(year):
        raise texlive.TexliveError(f"snapshot identity mismatch: {receipt_path}")
    database_path = root / "tlpkg/texlive.tlpdb.xz"
    texlive.verify_file(database_path, identity, "sha512", "snapshot TLPDB")
    packages = parse_tlpdb(database_path.read_bytes(), year=year)
    if len(packages) != receipt.get("package_count"):
        raise texlive.TexliveError("snapshot package count mismatch")
    for package in packages:
        texlive.verify_file(root / "archives" / package.archive_name, package.identity, "sha512", "snapshot package")
    expected_files = sorted(target for package in packages for _, target in package.runfiles)
    _verify_tree_shape(root, set(expected_files))
    owners = {target: package.archive_name for package in packages for _, target in package.runfiles}
    package_digests = {package.archive_name: hashlib.sha256() for package in packages}
    package_bytes = {package.archive_name: 0 for package in packages}
    inventory_path = root / "runtime.files"
    digest = hashlib.sha256()
    count = 0
    try:
        with inventory_path.open("rb") as source:
            for expected in expected_files:
                line = source.readline()
                digest.update(line)
                parts = line.decode("utf-8").rstrip("\n").split("\t")
                if len(parts) != 3 or parts[0] != expected:
                    raise texlive.TexliveError(f"invalid runtime inventory at {expected}")
                file = root / expected
                if file.is_symlink() or not file.is_file() or file.stat().st_size != int(parts[1]) or texlive.hash_file(file) != parts[2]:
                    raise texlive.TexliveError(f"runtime file changed: {file}")
                owner = owners[expected]
                package_digests[owner].update(line)
                package_bytes[owner] += int(parts[1])
                count += 1
            if source.read(1):
                raise texlive.TexliveError("runtime inventory has extra records")
    except (OSError, UnicodeError, ValueError) as error:
        raise texlive.TexliveError(f"invalid runtime inventory: {error}") from error
    if count != receipt.get("file_count") or digest.hexdigest() != receipt.get("runtime_inventory_sha256"):
        raise texlive.TexliveError("runtime inventory identity mismatch")
    for package in packages:
        marker_path = root / ".complete" / f"{package.archive_name}.json"
        try:
            marker = json.loads(marker_path.read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError) as error:
            raise texlive.TexliveError(f"invalid package marker: {marker_path}") from error
        if marker != {
            "archive_sha512": package.identity.digest,
            "installed_bytes": package_bytes[package.archive_name],
            "member_inventory_sha256": package_digests[package.archive_name].hexdigest(),
        }:
            raise texlive.TexliveError(f"package member identity mismatch: {marker_path}")
    return SnapshotResult(root, receipt_path, SNAPSHOT_DATES[year], identity, len(packages), count)


def ensure_snapshot(
    year: int,
    cache_root: Path = DEFAULT_CACHE_ROOT,
    *,
    offline: bool = False,
    max_download_bytes: int = DEFAULT_MAX_DOWNLOAD_BYTES,
    mirror: str | None = None,
    workers: int = 4,
    archive_caches: tuple[Path, ...] = (),
) -> SnapshotResult:
    """Download and install one dated, complete platform-independent runtime."""
    base_url = _source_url(year, mirror)
    root = cache_root / str(year)
    receipt = root / "acquisition.json"
    if receipt.exists():
        return verify_snapshot(year, cache_root)
    database_path = root / "tlpkg/texlive.tlpdb.xz"
    database_url = urljoin(base_url, "tlpkg/texlive.tlpdb.xz")
    identity_path = root / "tlpkg/texlive.tlpdb.identity.json"
    database_pin = None
    if identity_path.exists():
        try:
            pinned = json.loads(identity_path.read_text(encoding="utf-8"))
            database_pin = texlive.Identity(pinned["bytes"], pinned["sha512"])
            if not isinstance(database_pin.bytes, int) or database_pin.bytes <= 0 or not texlive.valid_digest(database_pin.digest, 128):
                raise ValueError("invalid TLPDB identity")
        except (OSError, KeyError, TypeError, ValueError, json.JSONDecodeError) as error:
            raise texlive.TexliveError(f"invalid retained TLPDB identity: {identity_path}") from error
    elif database_path.exists() and not offline:
        # An interrupted metadata fetch without its retained identity cannot
        # supply the authority for subsequent package hashes.
        database_path.unlink()
    database_identity = _download(database_url, database_path, expected=database_pin, limit=MAX_DATABASE_BYTES, offline=offline)
    try:
        packages = parse_tlpdb(database_path.read_bytes(), year=year)
    except texlive.TexliveError:
        if database_pin is None:
            database_path.unlink(missing_ok=True)
        raise
    if database_pin is None:
        _atomic_json(identity_path, {"url": database_url, "bytes": database_identity.bytes, "sha512": database_identity.digest})
    selected_bytes = sum(package.identity.bytes for package in packages)
    cached_bytes = sum(package.identity.bytes for package in packages if (root / "archives" / package.archive_name).is_file())
    if selected_bytes - cached_bytes > max_download_bytes:
        raise texlive.TexliveError(f"snapshot requires {selected_bytes - cached_bytes} download bytes, limit is {max_download_bytes}")
    if workers < 1 or workers > MAX_WORKERS:
        raise texlive.TexliveError(f"workers must be between 1 and {MAX_WORKERS}")
    print(f"TeX Live {year} {SNAPSHOT_DATES[year]}: {len(packages)} packages, {selected_bytes} archive bytes ({cached_bytes} cached)", file=sys.stderr)
    budget_lock = threading.Lock()
    extracted_total = 0
    def install(package: Package) -> None:
        nonlocal extracted_total
        archive = root / "archives" / package.archive_name
        for archive_cache in (cache_root, *archive_caches):
            _reuse_archive(root, archive_cache, year, package)
        _download(urljoin(base_url, f"archive/{package.archive_name}"), archive, expected=package.identity, limit=package.identity.bytes, offline=offline)
        marker = root / ".complete" / f"{package.archive_name}.json"
        valid_marker = False
        if marker.exists():
            try:
                marker_value = json.loads(marker.read_text(encoding="utf-8"))
                installed_bytes, member_digest = _package_inventory(root, package)
                valid_marker = marker_value == {
                    "archive_sha512": package.identity.digest,
                    "installed_bytes": installed_bytes,
                    "member_inventory_sha256": member_digest,
                }
            except (OSError, ValueError, json.JSONDecodeError, texlive.TexliveError):
                pass
        if not valid_marker:
            _extract_package(archive, root, package)
            installed_bytes, member_digest = _package_inventory(root, package)
            _atomic_json(marker, {
                "archive_sha512": package.identity.digest,
                "installed_bytes": installed_bytes,
                "member_inventory_sha256": member_digest,
            })
        with budget_lock:
            extracted_total += installed_bytes
            if extracted_total > MAX_RUNTIME_BYTES:
                raise texlive.TexliveError("installed runtime exceeds byte limit")
    with ThreadPoolExecutor(max_workers=workers) as pool:
        for index, _ in enumerate(pool.map(install, packages), 1):
            if index % 250 == 0:
                print(f"TeX Live {year}: installed {index}/{len(packages)} packages", file=sys.stderr)
    inventory_path = root / "runtime.files"
    file_count, inventory_sha256 = _inventory(root, packages, inventory_path)
    _atomic_json(receipt, {
        "schema": 1,
        "year": year,
        "snapshot_date": SNAPSHOT_DATES[year],
        "runtime_root": ".",
        "tlpdb": {"url": database_url, "bytes": database_identity.bytes, "sha512": database_identity.digest},
        "package_count": len(packages),
        "file_count": file_count,
        "selected_archive_bytes": selected_bytes,
        "runtime_inventory": "runtime.files",
        "runtime_inventory_sha256": inventory_sha256,
    })
    return verify_snapshot(year, cache_root)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=("acquire", "verify"))
    parser.add_argument("--year", type=int, required=True, choices=SNAPSHOT_DATES)
    parser.add_argument("--cache-root", type=Path, default=DEFAULT_CACHE_ROOT)
    parser.add_argument("--offline", action="store_true")
    parser.add_argument("--archive-cache", type=Path, action="append", default=[], help="Read-only cache root for authenticated package reuse; repeatable")
    parser.add_argument("--max-download-bytes", type=int, default=DEFAULT_MAX_DOWNLOAD_BYTES)
    parser.add_argument("--workers", type=int, default=4)
    parser.add_argument("--mirror", help="HTTPS archive base; localhost HTTP allowed for hermetic tests")
    args = parser.parse_args(argv)
    try:
        if args.command == "verify":
            result = verify_snapshot(args.year, args.cache_root)
        else:
            result = ensure_snapshot(args.year, args.cache_root, offline=args.offline, max_download_bytes=args.max_download_bytes, mirror=args.mirror, workers=args.workers, archive_caches=tuple(args.archive_cache))
    except texlive.TexliveError as error:
        parser.exit(1, f"texlive_snapshot: {error}\n")
    print(json.dumps({"runtime_root": str(result.root.resolve()), "runtime_receipt": str(result.receipt.resolve()), "snapshot_date": result.snapshot_date, "tlpdb_sha512": result.tlpdb_identity.digest, "package_count": result.package_count, "file_count": result.file_count}, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
