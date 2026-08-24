#!/usr/bin/env python3
"""Shared, authenticated TeX Live acquisition and provisioning primitives."""

from __future__ import annotations

import hashlib
import json
import os
import shutil
import subprocess
import tempfile
import urllib.error
import urllib.request
from dataclasses import dataclass
from pathlib import Path
from urllib.parse import urljoin

SOURCE_LOCK = Path("tests/texlive-source.lock")
DEFAULT_SOURCE_CACHE = Path("third_party/texlive-source")
DEFAULT_ROOT_URL = (
    "https://assets.umber.ink/texlive/texlive-20260301/manifest-v3.json"
)
DEFAULT_ROOT_SHA256 = (
    "43a31da364e4607957a38da10dabff227657d607d1845d502204adfd5d002e4b"
)
MAX_MANIFEST_BYTES = 32 * 1024 * 1024
CHUNK_BYTES = 1024 * 1024


class TexliveError(Exception):
    """A pinned TeX Live input could not be acquired or verified."""


@dataclass(frozen=True)
class Identity:
    bytes: int
    digest: str


@dataclass(frozen=True)
class SourceArchive:
    distribution: str
    filename: str
    identity: Identity
    url: str
    extracted: tuple[tuple[Path, Identity], ...]


@dataclass(frozen=True)
class RuntimeSource:
    kind: str
    virtual_path: Path
    identity: Identity
    destination: Path | None = None

    @property
    def key(self) -> str:
        return f"{self.kind}:{self.virtual_path.name}"


def _safe_relative(raw: str, *, label: str) -> Path:
    path = Path(raw)
    if path.is_absolute() or not path.parts or ".." in path.parts or "\\" in raw:
        raise TexliveError(f"unsafe {label}: {raw}")
    return path


def valid_digest(value: str, length: int) -> bool:
    return len(value) == length and all(character in "0123456789abcdef" for character in value)


def _safe_download_url(url: str) -> bool:
    return url.startswith(("https://", "http://127.0.0.1:", "http://localhost:"))


def hash_file(path: Path, algorithm: str = "sha256") -> str:
    digest = hashlib.new(algorithm)
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(CHUNK_BYTES), b""):
            digest.update(chunk)
    return digest.hexdigest()


def verify_file(path: Path, identity: Identity, algorithm: str, label: str) -> None:
    if not path.is_file():
        raise TexliveError(f"missing {label}: {path}")
    actual_bytes = path.stat().st_size
    if actual_bytes != identity.bytes:
        raise TexliveError(
            f"length mismatch for {label} {path}: expected {identity.bytes}, got {actual_bytes}"
        )
    actual_digest = hash_file(path, algorithm)
    if actual_digest != identity.digest:
        raise TexliveError(
            f"{algorithm.upper()} mismatch for {label} {path}: "
            f"expected {identity.digest}, got {actual_digest}"
        )


def read_source_lock(path: Path) -> SourceArchive:
    distribution = ""
    archive: tuple[str, Identity, str] | None = None
    extracted: list[tuple[Path, Identity]] = []
    for number, raw_line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        fields = raw_line.split()
        if not fields or fields[0].startswith("#"):
            continue
        if fields[0] == "distribution" and len(fields) == 2:
            distribution = fields[1]
        elif fields[0] == "archive" and len(fields) == 5:
            filename, raw_bytes, digest, url = fields[1:]
            if not _safe_download_url(url) or not valid_digest(digest, 128):
                raise TexliveError(f"{path}:{number}: invalid archive record")
            archive = (filename, Identity(int(raw_bytes), digest), url)
        elif fields[0] == "source" and len(fields) == 4:
            relative = _safe_relative(fields[1], label="source path")
            if not valid_digest(fields[3], 64):
                raise TexliveError(f"{path}:{number}: invalid source digest")
            extracted.append((relative, Identity(int(fields[2]), fields[3])))
        else:
            raise TexliveError(f"{path}:{number}: invalid source-lock record")
    if not distribution or archive is None:
        raise TexliveError(f"{path}: missing distribution or archive record")
    return SourceArchive(distribution, archive[0], archive[1], archive[2], tuple(extracted))


def _download(url: str, destination: Path, identity: Identity, algorithm: str, offline: bool) -> None:
    if destination.exists():
        verify_file(destination, identity, algorithm, "cached object")
        return
    if offline:
        raise TexliveError(f"missing {destination} while running --offline")
    destination.parent.mkdir(parents=True, exist_ok=True)
    request = urllib.request.Request(url, headers={"User-Agent": "umber-texlive/1"})
    descriptor, temporary_name = tempfile.mkstemp(
        prefix=f".{destination.name}.", dir=destination.parent
    )
    temporary = Path(temporary_name)
    try:
        digest = hashlib.new(algorithm)
        total = 0
        try:
            with urllib.request.urlopen(request, timeout=60) as response, os.fdopen(
                descriptor, "wb"
            ) as output:
                descriptor = -1
                while chunk := response.read(CHUNK_BYTES):
                    total += len(chunk)
                    if total > identity.bytes:
                        raise TexliveError(f"download length exceeds pin for {url}")
                    digest.update(chunk)
                    output.write(chunk)
                output.flush()
                os.fsync(output.fileno())
        except urllib.error.URLError as error:
            raise TexliveError(f"download failed for {url}: {error}") from error
        if total != identity.bytes or digest.hexdigest() != identity.digest:
            raise TexliveError(f"download identity mismatch for {url}")
        os.replace(temporary, destination)
    finally:
        if descriptor >= 0:
            os.close(descriptor)
        temporary.unlink(missing_ok=True)


def _verify_extracted(source_dir: Path, pin: SourceArchive) -> None:
    if not (source_dir / "configure").is_file():
        raise TexliveError(f"incomplete extracted TeX Live source: {source_dir}")
    for relative, identity in pin.extracted:
        verify_file(source_dir / relative, identity, "sha256", "extracted source")


def ensure_source_cache(cache_root: Path, lock_path: Path, offline: bool = False) -> Path:
    pin = read_source_lock(lock_path)
    cache_root.mkdir(parents=True, exist_ok=True)
    archive = cache_root / pin.filename
    _download(pin.url, archive, pin.identity, "sha512", offline)
    source_dir = cache_root / "src"
    needs_extraction = not source_dir.exists()
    if not needs_extraction:
        try:
            _verify_extracted(source_dir, pin)
        except TexliveError:
            needs_extraction = True
    if needs_extraction:
        temporary = Path(tempfile.mkdtemp(prefix=".src.", dir=cache_root))
        stale = cache_root / ".src.replaced"
        try:
            subprocess.run(
                ["tar", "-xJf", str(archive), "-C", str(temporary), "--strip-components=1"],
                check=True,
            )
            _verify_extracted(temporary, pin)
            if source_dir.exists() or source_dir.is_symlink():
                if stale.exists():
                    shutil.rmtree(stale)
                os.replace(source_dir, stale)
            try:
                os.replace(temporary, source_dir)
            except OSError:
                if stale.exists():
                    os.replace(stale, source_dir)
                raise
            if stale.exists():
                shutil.rmtree(stale)
        except (OSError, subprocess.CalledProcessError) as error:
            raise TexliveError(f"could not extract {archive}: {error}") from error
        finally:
            if temporary.exists():
                shutil.rmtree(temporary)
    _verify_extracted(source_dir, pin)
    return cache_root


def _git_output(repo_root: Path, *arguments: str) -> str:
    try:
        return subprocess.run(
            ["git", "-C", str(repo_root), *arguments],
            check=True,
            capture_output=True,
            text=True,
        ).stdout.strip()
    except (OSError, subprocess.CalledProcessError) as error:
        raise TexliveError(f"could not inspect Git worktree metadata: {error}") from error


def repository_root(path: Path) -> Path:
    return Path(
        _git_output(path, "rev-parse", "--path-format=absolute", "--show-toplevel")
    ).resolve()


def primary_checkout(repo_root: Path) -> Path:
    common = Path(
        _git_output(repo_root, "rev-parse", "--path-format=absolute", "--git-common-dir")
    ).resolve()
    for line in _git_output(repo_root, "worktree", "list", "--porcelain").splitlines():
        if not line.startswith("worktree "):
            continue
        candidate = Path(line.removeprefix("worktree ")).resolve()
        candidate_common = Path(
            _git_output(candidate, "rev-parse", "--path-format=absolute", "--git-common-dir")
        ).resolve()
        if candidate_common == common and (candidate / ".git").is_dir():
            return candidate
    raise TexliveError(f"Git has no primary checkout for shared directory {common}")


def _replace_with_symlink(destination: Path, source: Path, pin: SourceArchive) -> None:
    if destination.is_symlink() and destination.resolve() == source.resolve():
        return
    if destination.exists() or destination.is_symlink():
        if destination.name == pin.filename:
            verify_file(destination, pin.identity, "sha512", "worktree source archive")
            destination.unlink()
        elif destination.name == "src":
            if destination.is_symlink():
                destination.unlink()
            else:
                if not (destination / "configure").is_file() or not (
                    destination / "texk/web2c/tex.web"
                ).is_file():
                    raise TexliveError(
                        f"refusing to replace unrecognized source tree: {destination}"
                    )
                shutil.rmtree(destination)
        else:
            raise TexliveError(f"refusing to replace unexpected source path: {destination}")
    destination.symlink_to(source, target_is_directory=source.is_dir())


def provision_source(repo_root: Path, offline: bool = False) -> Path:
    repo_root = repository_root(repo_root)
    primary = primary_checkout(repo_root)
    lock_path = primary / SOURCE_LOCK
    pin = read_source_lock(lock_path)
    primary_cache = primary / DEFAULT_SOURCE_CACHE
    ensure_source_cache(primary_cache, lock_path, offline)
    if repo_root == primary:
        return primary_cache
    local_cache = repo_root / DEFAULT_SOURCE_CACHE
    local_cache.mkdir(parents=True, exist_ok=True)
    _replace_with_symlink(local_cache / pin.filename, primary_cache / pin.filename, pin)
    _replace_with_symlink(local_cache / "src", primary_cache / "src", pin)
    return local_cache


def read_runtime_sources(path: Path, require_destinations: bool = False) -> list[RuntimeSource]:
    records: list[RuntimeSource] = []
    keys: set[str] = set()
    destinations: set[Path] = set()
    for number, raw_line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        fields = raw_line.split()
        if not fields or fields[0].startswith("#") or fields[0] == "distribution":
            continue
        if fields[0] != "source" or len(fields) not in (5, 6):
            raise TexliveError(f"{path}:{number}: invalid runtime source record")
        kind = fields[1]
        virtual = _safe_relative(fields[2], label="virtual path")
        digest = fields[4]
        destination = _safe_relative(fields[5], label="destination") if len(fields) == 6 else None
        if not valid_digest(digest, 64):
            raise TexliveError(f"{path}:{number}: invalid SHA-256")
        record = RuntimeSource(kind, virtual, Identity(int(fields[3]), digest), destination)
        if record.key in keys or destination is not None and destination in destinations:
            raise TexliveError(f"{path}:{number}: duplicate key or destination")
        keys.add(record.key)
        if destination is not None:
            destinations.add(destination)
        records.append(record)
    if not records or require_destinations and any(record.destination is None for record in records):
        raise TexliveError(f"{path}: empty source list or missing destination")
    return records


def read_runtime_requests(
    path: Path,
) -> tuple[set[str], dict[str, Identity], set[str]]:
    """Read positive keys and negative catalogue assertions from runtime receipts."""
    requested: set[str] = set()
    expected: dict[str, Identity] = {}
    unavailable: set[str] = set()
    for number, raw_line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        if raw_line == "umber-pdf-font-closure-v1":
            continue
        if raw_line.startswith(("resolved\t", "unavailable\t")):
            fields = raw_line.split("\t")
            if fields[0] == "unavailable" and len(fields) == 4:
                key = fields[3]
                if ":" not in key:
                    raise TexliveError(f"{path}:{number}: invalid PDF font closure key")
                if key in requested or key in unavailable:
                    raise TexliveError(
                        f"{path}:{number}: duplicate runtime request key {key}"
                    )
                unavailable.add(key)
                continue
            if fields[0] != "resolved" or len(fields) != 7:
                raise TexliveError(f"{path}:{number}: invalid PDF font closure receipt record")
            key = fields[3]
            if ":" not in key or not valid_digest(fields[6], 64):
                raise TexliveError(f"{path}:{number}: invalid PDF font closure identity")
            identity = Identity(int(fields[5]), fields[6])
            previous = expected.setdefault(key, identity)
            if previous != identity:
                raise TexliveError(f"{path}:{number}: conflicting identity for {key}")
            if key in requested or key in unavailable:
                raise TexliveError(f"{path}:{number}: duplicate runtime request key {key}")
            requested.add(key)
            continue
        fields = raw_line.split()
        if not fields or fields[0].startswith("#") or fields[0] in {
            "distribution",
            "distribution_sha256",
            "format_schema",
            "source_date_epoch",
        }:
            continue
        if len(fields) == 1 and ":" in fields[0]:
            key = fields[0]
        elif fields[0] in {
            "source",
            "local",
            "pdflatex-source",
            "pdflatex-local",
        } and len(fields) == 4:
            virtual = _safe_relative(fields[1], label="virtual path")
            if not valid_digest(fields[3], 64):
                raise TexliveError(f"{path}:{number}: invalid SHA-256")
            kind = "tfm" if virtual.suffix == ".tfm" else "tex"
            key = f"{kind}:{virtual.name}"
            identity = Identity(int(fields[2]), fields[3])
            if fields[0] in {"source", "pdflatex-source"}:
                previous = expected.setdefault(key, identity)
                if previous != identity:
                    raise TexliveError(f"{path}:{number}: conflicting identity for {key}")
        elif fields[0] == "source" and len(fields) in (5, 6):
            virtual = _safe_relative(fields[2], label="virtual path")
            if not valid_digest(fields[4], 64):
                raise TexliveError(f"{path}:{number}: invalid SHA-256")
            key = f"{fields[1]}:{virtual.name}"
            identity = Identity(int(fields[3]), fields[4])
            previous = expected.setdefault(key, identity)
            if previous != identity:
                raise TexliveError(f"{path}:{number}: conflicting identity for {key}")
        else:
            raise TexliveError(f"{path}:{number}: invalid runtime request record")
        if key in requested or key in unavailable:
            raise TexliveError(f"{path}:{number}: duplicate runtime request key {key}")
        requested.add(key)
    if not requested and not unavailable:
        raise TexliveError(f"{path}: empty runtime request list")
    return requested, expected, unavailable


def _json_document(data: bytes, label: str) -> dict:
    try:
        value = json.loads(data)
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise TexliveError(f"invalid {label}: {error}") from error
    if not isinstance(value, dict):
        raise TexliveError(f"invalid {label}: expected an object")
    return value


def _read_verified_bytes(path: Path, identity: Identity) -> bytes | None:
    if not path.exists():
        return None
    verify_file(path, identity, "sha256", "cached snapshot object")
    return path.read_bytes()


def _acquire_snapshot_object(url: str, path: Path, identity: Identity, offline: bool) -> bytes:
    data = _read_verified_bytes(path, identity)
    if data is None:
        _download(url, path, identity, "sha256", offline)
        data = path.read_bytes()
    return data


def _seed_snapshot_object(
    path: Path, identity: Identity, object_roots: tuple[Path, ...]
) -> bytes | None:
    name = f"sha256-{identity.digest}"
    for root in object_roots:
        candidate = root / name
        if not candidate.exists():
            continue
        verify_file(candidate, identity, "sha256", "seed snapshot object")
        data = candidate.read_bytes()
        _write_atomic(path, data)
        return data
    return None


def _seed_snapshot_shard(
    path: Path, digest: str, object_roots: tuple[Path, ...]
) -> bytes | None:
    name = f"sha256-{digest}"
    for root in object_roots:
        candidate = root / name
        if not candidate.exists():
            continue
        data = candidate.read_bytes()
        if len(data) > MAX_MANIFEST_BYTES or hashlib.sha256(data).hexdigest() != digest:
            raise TexliveError(f"seed snapshot shard failed verification: {candidate}")
        _write_atomic(path, data)
        return data
    return None


def _write_atomic(path: Path, data: bytes) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    descriptor, temporary_name = tempfile.mkstemp(prefix=f".{path.name}.", dir=path.parent)
    temporary = Path(temporary_name)
    try:
        with os.fdopen(descriptor, "wb") as output:
            output.write(data)
            output.flush()
            os.fsync(output.fileno())
        os.replace(temporary, path)
    finally:
        temporary.unlink(missing_ok=True)


def _shard_index(key: str, bits: int) -> int:
    value = int.from_bytes(hashlib.sha256(key.encode()).digest()[:2], "big")
    return value >> (16 - bits) if bits else 0


def materialize_snapshot(
    output: Path,
    *,
    root_url: str = DEFAULT_ROOT_URL,
    root_sha256: str = DEFAULT_ROOT_SHA256,
    source_root_path: Path | None = None,
    object_roots: tuple[Path, ...] = (),
    texmf_roots: tuple[Path, ...] = (),
    formats_requested: tuple[str, ...] = (),
    keys: tuple[str, ...] = (),
    lock_paths: tuple[Path, ...] = (),
    offline: bool = False,
) -> dict[str, int | str]:
    if source_root_path is None and not root_url.startswith(("https://", "http://127.0.0.1:", "http://localhost:")):
        raise TexliveError("root URL must use HTTPS")
    if not valid_digest(root_sha256, 64):
        raise TexliveError("invalid root SHA-256")
    object_roots = tuple(root.resolve() for root in object_roots)
    texmf_roots = tuple(root.resolve() for root in texmf_roots)
    for root in (*object_roots, *texmf_roots):
        if not root.is_dir():
            raise TexliveError(f"snapshot seed root is not a directory: {root}")
    output = output.resolve()
    source_root_data = None
    if source_root_path is not None:
        source_root_path = source_root_path.resolve()
        source_root_data = source_root_path.read_bytes()
        if (
            len(source_root_data) > MAX_MANIFEST_BYTES
            or hashlib.sha256(source_root_data).hexdigest() != root_sha256
        ):
            raise TexliveError(f"root manifest identity mismatch for {source_root_path}")
    root_path = output / "manifest-v3.json"
    if root_path.exists():
        data = root_path.read_bytes()
        if len(data) > MAX_MANIFEST_BYTES or hashlib.sha256(data).hexdigest() != root_sha256:
            raise TexliveError(f"existing root manifest failed verification: {root_path}")
        root_data = data
    elif source_root_data is not None:
        root_data = source_root_data
        _write_atomic(root_path, root_data)
    else:
        if offline:
            raise TexliveError(f"missing {root_path} while running --offline")
        request = urllib.request.Request(root_url, headers={"User-Agent": "umber-texlive/1"})
        try:
            with urllib.request.urlopen(request, timeout=60) as response:
                root_data = response.read(MAX_MANIFEST_BYTES + 1)
        except urllib.error.URLError as error:
            raise TexliveError(f"download failed for {root_url}: {error}") from error
        if len(root_data) > MAX_MANIFEST_BYTES or hashlib.sha256(root_data).hexdigest() != root_sha256:
            raise TexliveError(f"root manifest identity mismatch for {root_url}")
        _write_atomic(root_path, root_data)
    root = _json_document(root_data, "root manifest")
    bits, shards = root.get("shardBits"), root.get("shards")
    formats, objects_url = root.get("formats"), root.get("objectsBaseUrl")
    if root.get("schema") != 3:
        raise TexliveError("root manifest is not schema 3")
    if (
        not isinstance(bits, int)
        or not 0 <= bits <= 16
        or not isinstance(shards, list)
        or len(shards) != 1 << bits
        or any(not isinstance(digest, str) or not valid_digest(digest, 64) for digest in shards)
    ):
        raise TexliveError("invalid shard inventory")
    if not isinstance(formats, dict) or not isinstance(objects_url, str) or not objects_url.startswith("https://") and not objects_url.startswith(("http://127.0.0.1:", "http://localhost:")):
        raise TexliveError("invalid formats or objectsBaseUrl")
    requested = set(keys)
    expected: dict[str, Identity] = {}
    unavailable: set[str] = set()
    for lock_path in lock_paths:
        lock_keys, lock_expected, lock_unavailable = read_runtime_requests(lock_path)
        if requested.intersection(lock_unavailable) or unavailable.intersection(lock_keys):
            raise TexliveError(f"conflicting runtime request outcome in {lock_path}")
        requested.update(lock_keys)
        unavailable.update(lock_unavailable)
        for key, identity in lock_expected.items():
            previous = expected.setdefault(key, identity)
            if previous != identity:
                raise TexliveError(f"conflicting locked identity for {key}")
    objects: dict[str, Identity] = {}
    object_virtuals: dict[str, Path] = {}
    views: dict[Path, str] = {}
    for name in formats_requested:
        record = formats.get(name)
        closure = record.get("inputClosure") if isinstance(record, dict) else None
        if not isinstance(closure, dict) or closure.get("schema") != 1 or not isinstance(closure.get("keys"), list):
            raise TexliveError(f"unknown or invalid format: {name}")
        requested.update(closure["keys"])
        objects[record["object"]] = Identity(record["bytes"], record["sha256"])
    if requested.intersection(unavailable):
        key = min(requested.intersection(unavailable))
        raise TexliveError(f"conflicting positive and unavailable runtime key: {key}")
    selected_shard_indices = {
        _shard_index(key, bits) for key in requested.union(unavailable)
    }
    shard_documents: dict[int, dict] = {}
    for index, digest in enumerate(shards):
        identity = Identity(MAX_MANIFEST_BYTES, digest)
        name = f"sha256-{digest}"
        path = output / "objects" / name
        if path.exists():
            data = path.read_bytes()
            if len(data) > MAX_MANIFEST_BYTES or hashlib.sha256(data).hexdigest() != digest:
                raise TexliveError(f"existing shard failed verification: {path}")
        else:
            data = _seed_snapshot_shard(path, digest, object_roots)
            if data is None:
                if offline:
                    raise TexliveError(f"missing {path} while running --offline")
                request = urllib.request.Request(urljoin(objects_url, name), headers={"User-Agent": "umber-texlive/1"})
                with urllib.request.urlopen(request, timeout=60) as response:
                    data = response.read(MAX_MANIFEST_BYTES + 1)
                if len(data) > MAX_MANIFEST_BYTES or hashlib.sha256(data).hexdigest() != digest:
                    raise TexliveError(f"shard identity mismatch: {name}")
                _write_atomic(path, data)
        shard = _json_document(data, f"shard {index}")
        if shard.get("schema") != 1 or shard.get("distribution") != root.get("distribution") or shard.get("index") != index:
            raise TexliveError(f"shard {index} identity mismatch")
        files = shard.get("files")
        if not isinstance(files, dict) or any(
            not isinstance(key, str) or _shard_index(key, bits) != index
            for key in files
        ):
            raise TexliveError(f"shard {index} contains a noncanonical lookup key")
        if index in selected_shard_indices:
            shard_documents[index] = shard
    for key in sorted(unavailable):
        shard = shard_documents[_shard_index(key, bits)]
        if key in shard["files"]:
            raise TexliveError(
                f"receipt declares a key unavailable but its canonical shard contains it: {key}"
            )
    for key in sorted(requested):
        record = shard_documents[_shard_index(key, bits)].get("files", {}).get(key)
        if not isinstance(record, dict):
            raise TexliveError(f"requested key is absent from its canonical shard: {key}")
        identity = Identity(record.get("bytes"), record.get("sha256"))
        if key in expected and expected[key] != identity:
            raise TexliveError(f"requested key differs from pinned lock identity: {key}")
        objects[record["object"]] = identity
        virtual = str(record.get("virtualPath", "")).removeprefix("/texlive/")
        virtual_path = _safe_relative(virtual, label=f"virtual path for {key}")
        views[virtual_path] = record["object"]
        object_virtuals.setdefault(record["object"], virtual_path)
    total = 0
    for name, identity in sorted(objects.items()):
        if name != f"sha256-{identity.digest}" or identity.bytes < 0:
            raise TexliveError(f"invalid object record: {name}")
        object_path = output / "objects" / name
        data = _read_verified_bytes(object_path, identity)
        if data is None:
            virtual = object_virtuals.get(name)
            if virtual is not None:
                for texmf_root in texmf_roots:
                    candidate = texmf_root / virtual
                    if not candidate.exists():
                        continue
                    verify_file(candidate, identity, "sha256", "seed TEXMF object")
                    data = candidate.read_bytes()
                    _write_atomic(object_path, data)
                    break
        if data is None:
            data = _seed_snapshot_object(object_path, identity, object_roots)
        if data is None:
            _acquire_snapshot_object(urljoin(objects_url, name), object_path, identity, offline)
        total += identity.bytes
    for virtual, name in sorted(views.items(), key=lambda item: str(item[0])):
        source = output / "objects" / name
        destination = output / "texmf-dist" / virtual
        destination.parent.mkdir(parents=True, exist_ok=True)
        if destination.exists():
            if destination.read_bytes() != source.read_bytes():
                raise TexliveError(f"existing TEXMF view differs: {destination}")
        else:
            try:
                os.link(source, destination)
            except OSError:
                shutil.copyfile(source, destination)
    return {
        "root_sha256": root_sha256,
        "shards": len(shards),
        "keys": len(requested),
        "unavailable_keys": len(unavailable),
        "payload_objects": len(objects),
        "payload_bytes": total,
        "texmf_files": len(views),
        "output": str(output),
    }


def stage_runtime_sources(snapshot: Path, lock_path: Path, destination_root: Path) -> int:
    records = read_runtime_sources(lock_path, require_destinations=True)
    copied = 0
    for record in records:
        assert record.destination is not None
        source = snapshot / "texmf-dist" / record.virtual_path
        verify_file(source, record.identity, "sha256", "materialized TeX Live source")
        destination = destination_root / record.destination
        if destination.exists():
            verify_file(destination, record.identity, "sha256", "staged TeX Live source")
            continue
        destination.parent.mkdir(parents=True, exist_ok=True)
        descriptor, temporary_name = tempfile.mkstemp(prefix=f".{destination.name}.", dir=destination.parent)
        temporary = Path(temporary_name)
        try:
            with os.fdopen(descriptor, "wb") as output, source.open("rb") as input_file:
                shutil.copyfileobj(input_file, output)
                output.flush()
                os.fsync(output.fileno())
            verify_file(temporary, record.identity, "sha256", "staged TeX Live source")
            os.chmod(temporary, 0o444)
            os.replace(temporary, destination)
        finally:
            temporary.unlink(missing_ok=True)
        copied += 1
    return copied


def verify_runtime_tree(texmf_dist: Path, lock_path: Path) -> tuple[str, str]:
    distribution = ""
    tree_digest = ""
    for number, raw_line in enumerate(lock_path.read_text(encoding="utf-8").splitlines(), 1):
        fields = raw_line.split()
        if not fields or fields[0].startswith("#"):
            continue
        if fields[0] == "distribution" and len(fields) == 2:
            distribution = fields[1]
        elif fields[0] == "tree_sha256" and len(fields) == 2:
            tree_digest = fields[1]
        elif fields[0] == "source" and len(fields) == 4:
            relative = _safe_relative(fields[1], label="snapshot source")
            verify_file(texmf_dist / relative, Identity(int(fields[2]), fields[3]), "sha256", "snapshot source")
        else:
            raise TexliveError(f"{lock_path}:{number}: invalid snapshot-lock record")
    if not distribution or not valid_digest(tree_digest, 64):
        raise TexliveError(f"{lock_path}: missing distribution or tree digest")
    return distribution, tree_digest
