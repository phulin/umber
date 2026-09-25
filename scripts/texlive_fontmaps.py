#!/usr/bin/env python3
"""Build selected-release font maps with that release's official updmap.pl."""

from __future__ import annotations

import hashlib
import json
import lzma
import os
import re
import shutil
import subprocess
import tarfile
import tempfile
from collections import Counter
from pathlib import Path

import texlive
import texlive_snapshot


class FontMapError(Exception):
    """A selected release cannot produce an authenticated font map."""


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def package_record(tlpdb: Path, name: str) -> tuple[int, texlive.Identity, tuple[str, ...]]:
    """Find one package's archive identity and its listed Perl runfiles."""
    with lzma.open(tlpdb, "rt", encoding="utf-8") as stream:
        for block in stream.read().split("\n\n"):
            lines = block.splitlines()
            if not lines or lines[0] != f"name {name}":
                continue
            fields = dict(line.split(" ", 1) for line in lines if line.startswith(("revision ", "containersize ", "containerchecksum ")))
            try:
                revision = int(fields["revision"])
                size = int(fields["containersize"])
                checksum = fields["containerchecksum"]
            except (KeyError, ValueError) as error:
                raise FontMapError(f"invalid {name} archive identity in {tlpdb}") from error
            if revision < 0 or size <= 0 or size > texlive_snapshot.MAX_ARCHIVE_BYTES or not texlive.valid_digest(checksum, 128):
                raise FontMapError(f"invalid {name} archive identity in {tlpdb}")
            modules = tuple(line.strip().split(" ", 1)[0] for line in lines if line.startswith(" tlpkg/TeXLive/"))
            for module in modules:
                texlive_snapshot._safe_path(module)
            if name == "texlive.infra" and "tlpkg/TeXLive/TLUtils.pm" not in modules:
                raise FontMapError(f"{name} omits TLUtils.pm in {tlpdb}")
            return revision, texlive.Identity(size, checksum), modules
    raise FontMapError(f"missing {name} in {tlpdb}")


def validate_updmap_config(tlpdb: Path, config: Path) -> int:
    """Require the selected cfg to reflect the selected TLPDB map directives."""
    commands = {"addMap": "Map", "addMixedMap": "MixedMap", "addKanjiMap": "KanjiMap"}
    with lzma.open(tlpdb, "rt", encoding="utf-8") as stream:
        declared = Counter()
        for line in stream:
            match = re.fullmatch(r"execute (addMap|addMixedMap|addKanjiMap)\s+(\S+)\s*\n?", line)
            if match:
                declared[(commands[match[1]], match[2])] += 1
    configured = Counter()
    for line in config.read_text(encoding="utf-8").splitlines():
        pieces = line.split("#", 1)[0].split()
        if pieces and pieces[0] in commands.values():
            if len(pieces) != 2:
                raise FontMapError(f"malformed selected updmap.cfg directive: {line}")
            configured[(pieces[0], pieces[1])] += 1
    if not declared or declared != configured:
        missing = list((declared - configured).elements())[:3]
        extra = list((configured - declared).elements())[:3]
        raise FontMapError(f"selected updmap.cfg disagrees with TLPDB map directives: missing={missing}, extra={extra}")
    return sum(declared.values())


def stage_modules(snapshot_root: Path, output: Path) -> dict[str, object]:
    """Extract only listed TeX Live Perl modules from its verified archive."""
    tlpdb = snapshot_root / "tlpkg/texlive.tlpdb.xz"
    revision, expected, modules = package_record(tlpdb, "texlive.infra")
    archive_name = f"texlive.infra.r{revision}.tar.xz"
    cached = snapshot_root / "archives" / archive_name
    archive = cached if cached.is_file() else output / "dependencies" / archive_name
    if not archive.is_file():
        acquisition = json.loads((snapshot_root / "acquisition.json").read_text(encoding="utf-8"))
        database_url = str(acquisition["tlpdb"]["url"])
        if not database_url.endswith("tlpkg/texlive.tlpdb.xz"):
            raise FontMapError(f"invalid selected TLPDB URL: {database_url}")
        url = database_url.removesuffix("tlpkg/texlive.tlpdb.xz") + "archive/" + archive_name
        texlive_snapshot._download(url, archive, expected=expected, limit=texlive_snapshot.MAX_ARCHIVE_BYTES, offline=False)
    texlive.verify_file(archive, expected, "sha512", "selected texlive.infra archive")
    support = output / "fontmap-support"
    expected_modules = set(modules)
    seen: set[str] = set()
    with tarfile.open(archive, "r|xz") as source:
        for member in source:
            if member.name not in expected_modules:
                continue
            if member.name in seen or not member.isfile() or member.size > texlive_snapshot.MAX_MEMBER_BYTES:
                raise FontMapError(f"unsafe or duplicate selected Perl module: {member.name}")
            seen.add(member.name)
            destination = support / member.name
            texlive_snapshot._safe_parent(support, member.name)
            stream = source.extractfile(member)
            if stream is None:
                raise FontMapError(f"unreadable selected Perl module: {member.name}")
            with tempfile.NamedTemporaryFile("wb", dir=destination.parent, delete=False) as target:
                temporary = Path(target.name)
                shutil.copyfileobj(stream, target)
            if temporary.stat().st_size != member.size:
                temporary.unlink(missing_ok=True)
                raise FontMapError(f"truncated selected Perl module: {member.name}")
            os.replace(temporary, destination)
    if seen != expected_modules:
        raise FontMapError(f"selected texlive.infra archive omits {sorted(expected_modules - seen)[:3]}")
    return {"archive": str(archive), "archive_sha512": expected.digest, "module_count": len(modules), "support_root": str(support)}


def prepare_fontmaps(snapshot_root: Path, output: Path, kpsewhich: Path) -> dict[str, object]:
    """Generate installed-style maps outside the immutable selected snapshot."""
    snapshot_root = snapshot_root.resolve()
    output = output.resolve()
    kpsewhich = kpsewhich.resolve()
    if not kpsewhich.is_file():
        raise FontMapError(f"missing kpsewhich: {kpsewhich}")
    texmf = snapshot_root / "texmf-dist"
    tlpdb = snapshot_root / "tlpkg/texlive.tlpdb.xz"
    script = texmf / "scripts/texlive/updmap.pl"
    config = texmf / "web2c/updmap.cfg"
    if not script.is_file() or not config.is_file():
        raise FontMapError(f"missing selected updmap script or config in {texmf}")
    directive_count = validate_updmap_config(tlpdb, config)
    support = stage_modules(snapshot_root, output)
    recipe = {
        "generator": "selected-texlive-updmap-pl-v1",
        "tlpdb_sha256": sha256(tlpdb), "updmap_script_sha256": sha256(script),
        "updmap_config_sha256": sha256(config),
        "texlive_infra_archive_sha512": support["archive_sha512"],
        "kpsewhich_sha256": sha256(kpsewhich),
    }
    recipe_sha256 = hashlib.sha256(json.dumps(recipe, sort_keys=True).encode("utf-8")).hexdigest()
    generated = output / "generated-fontmaps" / recipe_sha256[:24]
    pdftex_map = generated / "fonts/map/pdftex/updmap/pdftex.map"
    command = ["perl", str(script), "--sys", "--nohash", f"--cnffile={config}"]
    private = output / "fontmap-private" / recipe_sha256[:24]
    work_map = private / "sysvar/fonts/map/pdftex/updmap/pdftex.map"
    for name in ("home", "local", "config", "sysconfig", "tmp"):
        (private / name).mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="fontmap-build.", dir=output) as raw:
        scratch = Path(raw)
        env = {
            "HOME": str(private / "home"), "LC_ALL": "C", "PATH": str(kpsewhich.parent) + os.pathsep + "/usr/bin:/bin",
            "TMPDIR": str(private / "tmp"), "TEXMFCNF": str(texmf / "web2c"),
            "TEXMFROOT": str(support["support_root"]), "TEXMFDIST": str(texmf),
            "TEXMFLOCAL": str(private / "local"), "TEXMFHOME": str(private / "home"),
            "TEXMFSYSVAR": str(private / "sysvar"), "TEXMFVAR": str(private / "sysvar"),
            "TEXMFSYSCONFIG": str(private / "sysconfig"), "TEXMFCONFIG": str(private / "config"),
            "TEXFONTMAPS": f"{private / 'sysvar/fonts/map'}//:{texmf / 'fonts/map'}//",
        }
        result = subprocess.run(command, cwd=private / "tmp", env=env, capture_output=True, text=True, timeout=600, check=False)
        log = output / f"fontmap-updmap-{recipe_sha256[:24]}.log"
        if not log.exists():
            log.write_text(result.stdout + "\n--- stderr ---\n" + result.stderr, encoding="utf-8")
        if result.returncode:
            raise FontMapError(f"selected updmap.pl failed ({result.returncode}); see {log}")
        if not work_map.is_file() or work_map.stat().st_size == 0:
            raise FontMapError(f"selected updmap.pl omitted pdftex.map; see {log}")
        # updmap creates this alias as a symlink. Publish its generated target
        # bytes as the one regular file admitted to the runtime catalogue.
        staged = scratch / "publish/fonts/map/pdftex/updmap/pdftex.map"
        staged.parent.mkdir(parents=True)
        shutil.copyfile(work_map, staged)
        if generated.exists():
            if not pdftex_map.is_file() or pdftex_map.is_symlink() or sha256(pdftex_map) != sha256(staged):
                raise FontMapError(f"previous selected font-map output differs: {pdftex_map}")
        else:
            generated.parent.mkdir(parents=True, exist_ok=True)
            os.replace(scratch / "publish", generated)
    record = {
        "schema": 1, "generator": "selected-texlive-updmap-pl-v1",
        "recipe_sha256": recipe_sha256, "generated_root": str(generated), "pdftex_map": str(pdftex_map),
        "pdftex_map_sha256": sha256(pdftex_map), "pdftex_map_bytes": pdftex_map.stat().st_size,
        "tlpdb_sha256": recipe["tlpdb_sha256"], "updmap_script_sha256": recipe["updmap_script_sha256"],
        "updmap_config_sha256": recipe["updmap_config_sha256"], "map_directive_count": directive_count,
        "texlive_infra_archive_sha512": support["archive_sha512"],
        "texlive_infra_module_count": support["module_count"],
        "kpsewhich_sha256": recipe["kpsewhich_sha256"], "command": command,
    }
    receipt = output / f"fontmaps-{recipe_sha256[:24]}.json"
    if receipt.is_file() and json.loads(receipt.read_text(encoding="utf-8")) != record:
        raise FontMapError(f"previous selected font-map receipt differs: {receipt}")
    with tempfile.NamedTemporaryFile("w", encoding="utf-8", dir=output, delete=False) as stream:
        temporary = Path(stream.name)
        json.dump(record, stream, indent=2, sort_keys=True)
        stream.write("\n")
    if receipt.is_file():
        temporary.unlink()
    else:
        os.replace(temporary, receipt)
    current = output / "fontmaps.json"
    with tempfile.NamedTemporaryFile("wb", dir=output, delete=False) as stream:
        temporary = Path(stream.name)
        with receipt.open("rb") as source:
            shutil.copyfileobj(source, stream)
    os.replace(temporary, current)
    return record


def verify_fontmaps(snapshot_root: Path, record: dict[str, object]) -> Path:
    """Validate a prepared map against its selected snapshot and receipt."""
    snapshot_root = snapshot_root.resolve()
    texmf = snapshot_root / "texmf-dist"
    generated = Path(str(record["generated_root"]))
    pdftex_map = generated / "fonts/map/pdftex/updmap/pdftex.map"
    checks = {
        "tlpdb_sha256": snapshot_root / "tlpkg/texlive.tlpdb.xz",
        "updmap_script_sha256": texmf / "scripts/texlive/updmap.pl",
        "updmap_config_sha256": texmf / "web2c/updmap.cfg",
        "pdftex_map_sha256": pdftex_map,
    }
    recipe = {key: record.get(key) for key in (
        "generator", "tlpdb_sha256", "updmap_script_sha256", "updmap_config_sha256",
        "texlive_infra_archive_sha512", "kpsewhich_sha256",
    )}
    recipe_sha256 = hashlib.sha256(json.dumps(recipe, sort_keys=True).encode("utf-8")).hexdigest()
    if (
        record.get("schema") != 1
        or record.get("generator") != "selected-texlive-updmap-pl-v1"
        or record.get("recipe_sha256") != recipe_sha256
        or generated.is_symlink()
        or generated != generated.parent.parent / "generated-fontmaps" / recipe_sha256[:24]
        or record.get("pdftex_map") != str(pdftex_map)
    ):
        raise FontMapError("invalid selected font-map receipt")
    if "receipt" in record or "receipt_sha256" in record:
        receipt = Path(str(record.get("receipt", "")))
        expected = {key: value for key, value in record.items() if key not in ("receipt", "receipt_sha256")}
        if (
            not receipt.is_file()
            or receipt.is_symlink()
            or generated.parent.parent != receipt.parent
            or record.get("receipt_sha256") != sha256(receipt)
            or json.loads(receipt.read_text(encoding="utf-8")) != expected
        ):
            raise FontMapError(f"selected font-map receipt changed: {receipt}")
    files = []
    for directory, subdirectories, names in os.walk(generated, followlinks=False):
        for name in subdirectories:
            if (Path(directory) / name).is_symlink():
                raise FontMapError(f"unsafe selected font-map directory: {Path(directory) / name}")
        for name in names:
            files.append(Path(directory) / name)
    if files != [pdftex_map]:
        raise FontMapError(f"selected font-map tree has unexpected files: {generated}")
    for key, path in checks.items():
        if path.is_symlink() or not path.is_file() or record.get(key) != sha256(path):
            raise FontMapError(f"selected font-map identity changed: {path}")
    if record.get("pdftex_map_bytes") != pdftex_map.stat().st_size:
        raise FontMapError(f"selected font-map length changed: {pdftex_map}")
    return generated
