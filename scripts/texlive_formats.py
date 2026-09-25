#!/usr/bin/env python3
"""Prepare stable LaTeX formats from one explicitly selected TeX Live snapshot."""

from __future__ import annotations

import argparse
import datetime as dt
import hashlib
import json
import lzma
import os
import re
import shlex
import shutil
import subprocess
import sys
import tempfile
import time
from pathlib import Path

import texlive_fontmaps


class FormatPreparationError(Exception):
    """A selected release could not produce an auditable format pair."""


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def atomic_json(path: Path, data: object) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.NamedTemporaryFile("w", encoding="utf-8", dir=path.parent, delete=False) as stream:
        temporary = Path(stream.name)
        json.dump(data, stream, indent=2, sort_keys=True)
        stream.write("\n")
    os.replace(temporary, path)


def atomic_copy(source: Path, destination: Path) -> None:
    destination.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.NamedTemporaryFile("wb", dir=destination.parent, delete=False) as stream:
        temporary = Path(stream.name)
        with source.open("rb") as input_file:
            shutil.copyfileobj(input_file, stream)
    os.replace(temporary, destination)


def source_epoch(receipt: dict[str, object]) -> int:
    raw = receipt.get("snapshot_date")
    if not isinstance(raw, str) or not re.fullmatch(r"\d{4}-\d{2}-\d{2}", raw):
        raise FormatPreparationError("acquisition receipt lacks a snapshot_date")
    try:
        return int(dt.datetime.combine(dt.date.fromisoformat(raw), dt.time(), dt.timezone.utc).timestamp())
    except ValueError as error:
        raise FormatPreparationError(f"invalid snapshot_date: {raw}") from error


def tlpdb_release_year(tlpdb: Path) -> int:
    """Read the upstream release marker from its 00texlive.config package."""
    inside = False
    with lzma.open(tlpdb, "rt", encoding="utf-8") as stream:
        for line in stream:
            if line.startswith("name "):
                if inside:
                    break
                inside = line.strip() == "name 00texlive.config"
            elif inside and line.startswith("depend release/"):
                raw = line.removeprefix("depend release/").strip()
                if re.fullmatch(r"20\d{2}", raw):
                    return int(raw)
    raise FormatPreparationError(f"TLPDB lacks an unambiguous 00texlive.config release year: {tlpdb}")


def addhyphen_records(tlpdb: Path) -> list[tuple[str, dict[str, str]]]:
    """Read package-owned AddHyphen directives in TLPDB order."""
    package = ""
    records: list[tuple[str, dict[str, str]]] = []
    opener = lzma.open if tlpdb.suffix == ".xz" else open
    with opener(tlpdb, "rt", encoding="utf-8") as stream:
        for line in stream:
            if line.startswith("name "):
                package = line[5:].strip()
            elif line.startswith("execute AddHyphen "):
                fields = shlex.split(line[len("execute AddHyphen "):])
                values = dict(field.split("=", 1) for field in fields if "=" in field)
                if not package or not values.get("name") or not values.get("file"):
                    raise FormatPreparationError(f"invalid AddHyphen directive in {tlpdb}")
                if "dat" in values.get("databases", "dat,def,lua").split(","):
                    records.append((package, values))
    if not records:
        raise FormatPreparationError(f"no AddHyphen directives in {tlpdb}")
    return records


def language_dat(texmf: Path, tlpdb: Path, output: Path) -> dict[str, object]:
    source = texmf / "tex/generic/config/language.us"
    if not source.is_file():
        raise FormatPreparationError(f"missing selected language.us: {source}")
    records = addhyphen_records(tlpdb)
    lines = [source.read_text(encoding="utf-8").rstrip("\n"), ""]
    for package, record in records:
        lines.extend((f"% from {package}:", f"{record['name']} {record['file']}"))
        for synonym in record.get("synonyms", "").split(","):
            if synonym:
                lines.append(f"={synonym}")
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text("\n".join(lines) + "\n", encoding="utf-8")
    return {
        "generator": "tlpdb-AddHyphen-language-dat-v1",
        "language_us_sha256": sha256(source),
        "tlpdb_sha256": sha256(tlpdb),
        "addhyphen_count": len(records),
        "sha256": sha256(output),
    }


def stable_paths(texmf: Path) -> tuple[Path, Path, Path]:
    base = texmf / "tex/latex/base"
    kernel = texmf / "tex/latex/l3kernel"
    found = [
        path.parent for path in (texmf / "tex/latex").rglob("latex.ini")
        if "latex-dev" not in path.parts and (path.parent / "pdflatex.ini").is_file()
    ]
    if len(found) != 1:
        raise FormatPreparationError(f"expected one selected stable LaTeX ini directory, found {found}")
    ini = found[0]
    for path in (base / "latex.ltx", kernel / "expl3-code.tex"):
        if not path.is_file():
            raise FormatPreparationError(f"missing stable format source: {path}")
    return base, kernel, ini


def reference_environment(texmf: Path, config: Path, epoch: int, work: Path) -> dict[str, str]:
    base, kernel, ini = stable_paths(texmf)
    paths = [Path("."), config, base, kernel, ini, texmf / "tex/generic/babel", texmf / "tex/generic/tex-ini-files", texmf / "tex"]
    env = {
        "HOME": str(work / "home"),
        "LC_ALL": "C",
        "TMPDIR": str(work / "tmp"),
        "SOURCE_DATE_EPOCH": str(epoch),
        "FORCE_SOURCE_DATE": "1",
        "TEXMFCNF": str(texmf / "web2c"),
        "WEB2C": str(texmf / "web2c"),
        "TEXMFROOT": str(texmf),
        "TEXMFDIST": str(texmf),
        "TEXMFLOCAL": str(work / "empty"),
        "TEXMFHOME": str(work / "home"),
        "TEXMFSYSVAR": str(work / "sysvar"),
        "TEXMFSYSCONFIG": str(work / "sysconfig"),
        "TEXMFVAR": str(work / "var"),
        "TEXMFCONFIG": str(work / "config"),
        "TEXMFCACHE": str(work / "cache"),
        "VARTEXFONTS": str(work / "fonts"),
        "TEXINPUTS": ":".join(str(path) for path in paths[:-1]) + f":{paths[-1]}//",
        "TEXFONTS": f"{texmf / 'fonts/tfm'}//",
        "TFMFONTS": f"{texmf / 'fonts/tfm'}//",
    }
    for name in ("home", "tmp", "empty", "sysvar", "sysconfig", "var", "config", "cache", "fonts"):
        (work / name).mkdir(parents=True, exist_ok=True)
    return env


def run_guarded(repo: Path, binary: Path, args: list[str], *, cwd: Path, env: dict[str, str], stdout: Path, stderr: Path, timeout: int = 600, rss: int = 2048) -> None:
    command = [sys.executable, str(repo / "scripts/run-umber-guarded.py"), "--timeout-seconds", str(timeout), "--max-rss-mib", str(rss), "--term-grace-seconds", "5", "--", str(binary), *args]
    with stdout.open("wb") as out, stderr.open("wb") as err:
        result = subprocess.run(command, cwd=cwd, env=env, stdout=out, stderr=err, check=False)
    if result.returncode:
        raise FormatPreparationError(f"format command failed ({result.returncode}); see {stdout} and {stderr}")


def recorder_inputs(run: Path, texmf: Path, config: Path, fls: Path) -> list[Path]:
    used: set[Path] = set()
    for line in fls.read_text(encoding="utf-8").splitlines():
        if not line.startswith("INPUT "):
            continue
        path = Path(line[6:])
        if not path.is_absolute():
            path = run / path
        path = path.resolve()
        if path in (Path("/dev/null"), (run / "texsys.aux").resolve()):
            continue
        if not (path.is_relative_to(texmf) or path.is_relative_to(config)):
            raise FormatPreparationError(f"reference opened input outside selected snapshot: {path}")
        if "latex-dev" in path.parts:
            raise FormatPreparationError(f"reference selected latex-dev input: {path}")
        used.add(path)
    required = [texmf / "tex/latex/base/latex.ltx", texmf / "tex/latex/l3kernel/expl3-code.tex", config / "language.dat"]
    for path in required:
        if path.resolve() not in used:
            raise FormatPreparationError(f"reference did not consume required stable source: {path}")
    return sorted(used)


def build_reference(repo: Path, texmf: Path, config: Path, binary: Path, engine: str, epoch: int, output: Path) -> dict[str, object]:
    work = output / f"reference-{engine}-work"
    work.mkdir(parents=True, exist_ok=True)
    env = reference_environment(texmf, config, epoch, work)
    arguments = ["-ini", "-etex", "-enc", f"-progname={engine}", f"-jobname={engine}", "-translate-file=cp227.tcx", "-recorder", f"{engine}.ini"]
    for stale in (work / f"{engine}.fmt", work / f"{engine}.fls"):
        stale.unlink(missing_ok=True)
    run_guarded(repo, binary, arguments, cwd=work, env=env, stdout=work / "terminal.txt", stderr=work / "stderr.txt")
    fmt = work / f"{engine}.fmt"
    fls = work / f"{engine}.fls"
    if not fmt.is_file() or not fls.is_file():
        raise FormatPreparationError(f"reference omitted {engine} format or recorder")
    inputs = recorder_inputs(work, texmf, config, fls)
    published = output / f"reference-{engine}.fmt"
    atomic_copy(fmt, published)
    receipt = {
        "schema": 1,
        "engine": {"name": "pdfTeX", "version": "1.40.29", "sha256": sha256(binary), "arguments": arguments},
        "source_date_epoch": epoch,
        "format": {"bytes": published.stat().st_size, "sha256": sha256(published)},
        "inputs": [{"path": str(path), "bytes": path.stat().st_size, "sha256": sha256(path)} for path in inputs],
    }
    receipt_path = output / f"reference-{engine}.json"
    atomic_json(receipt_path, receipt)
    return receipt


def publish_local_support(texmf: Path, publisher: Path, output: Path, year: int) -> tuple[Path, str]:
    """Provide an explicit selected-release fallback catalogue for local TEXMF runs."""
    staged = output / "distribution-input"
    source = stable_paths(texmf)[2] / "latex.ini"
    target = staged / source.relative_to(texmf)
    target.parent.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(source, target)
    tree = subprocess.run([str(publisher), "--tree-ahash64", str(staged)], capture_output=True, text=True, check=True).stdout.strip()
    configuration = output / "distribution-config.json"
    atomic_json(configuration, {
        "schema": 8,
        "distribution": f"texlive-{year}-local-format-support",
        "objectsBaseUrl": "https://example.invalid/texlive/objects/",
        "shardBits": 0,
        "roots": [{"name": "selected-release-sentinel", "path": str(staged), "treeAhash64": tree}],
        "formats": [],
    })
    distribution = output / f"distribution-{sha256(source)[:16]}"
    manifest = distribution / "manifest.json"
    if manifest.exists():
        result = subprocess.run([str(publisher), "--verify-sharded", str(distribution)], capture_output=True, text=True, check=False)
        if result.returncode:
            raise FormatPreparationError(f"existing selected-release support distribution failed verification: {result.stderr.strip()}")
        existing = json.loads(manifest.read_text(encoding="utf-8"))
        if existing.get("distribution") != f"texlive-{year}-local-format-support":
            raise FormatPreparationError(f"existing selected-release support distribution belongs to another year: {manifest}")
    else:
        result = subprocess.run([str(publisher), str(configuration), str(distribution)], capture_output=True, text=True, check=False)
        if result.returncode:
            raise FormatPreparationError(f"selected-release support distribution failed: {result.stderr.strip()}")
    digest = subprocess.run([str(publisher), "--file-ahash64", str(manifest)], capture_output=True, text=True, check=True).stdout.strip()
    return distribution, digest


def hardlink_tree(source: Path, destination: Path, *, skip_top_level: frozenset[str] = frozenset()) -> int:
    count = 0
    for parent, directories, filenames in os.walk(source):
        relative = Path(parent).relative_to(source)
        if relative == Path("."):
            directories[:] = [name for name in directories if name not in skip_top_level]
        directories.sort()
        for name in sorted(filenames):
            original = Path(parent) / name
            if original.is_symlink() or not original.is_file():
                raise FormatPreparationError(f"selected runtime has a nonregular source file: {original}")
            target = destination / relative / name
            target.parent.mkdir(parents=True, exist_ok=True)
            os.link(original, target)
            count += 1
    return count


def runtime_priority_paths(texmf: Path, references: list[dict[str, object]]) -> list[Path]:
    """Return the authenticated format-source paths that win runtime aliases."""
    priority: set[Path] = set()
    for receipt in references:
        for record in receipt["inputs"]:
            source = Path(str(record["path"]))
            if not source.is_relative_to(texmf / "tex") and not source.is_relative_to(texmf / "fonts"):
                continue
            if not source.is_file() or source.stat().st_size != record["bytes"] or sha256(source) != record["sha256"]:
                raise FormatPreparationError(f"reference source changed since capture: {source}")
            if "latex-dev" in source.parts:
                raise FormatPreparationError(f"reference selected latex-dev runtime input: {source}")
            priority.add(source)
    return sorted(priority)


def publish_full_runtime(year: int, snapshot_root: Path, output: Path, publisher: Path) -> tuple[Path, str]:
    """Pack all selected TeX/font runfiles, with format inputs winning aliases."""
    texmf = snapshot_root / "texmf-dist"
    config = output / "generated-config/language.dat"
    fontmaps = json.loads((output / "fontmaps.json").read_text(encoding="utf-8"))
    generated = texlive_fontmaps.verify_fontmaps(snapshot_root, fontmaps)
    pdftex_map = generated / "fonts/map/pdftex/updmap/pdftex.map"
    reference = [json.loads((output / f"reference-{engine}.json").read_text(encoding="utf-8")) for engine in ("latex", "pdflatex")]
    priority_paths = runtime_priority_paths(texmf, reference)
    acquisition = snapshot_root / "acquisition.json"
    priority_identity = "\n".join(str(path.relative_to(texmf)) for path in priority_paths)
    identity = hashlib.sha256(("stable-runtime-layout-v5:" + sha256(acquisition) + sha256(config) + sha256(pdftex_map) + priority_identity).encode("utf-8")).hexdigest()[:16]
    distribution = output / f"runtime-distribution-{identity}"
    manifest = distribution / "manifest.json"
    if manifest.is_file():
        subprocess.run([str(publisher), "--verify-sharded", str(distribution)], check=True, stdout=subprocess.DEVNULL)
        digest = subprocess.run([str(publisher), "--file-ahash64", str(manifest)], capture_output=True, text=True, check=True).stdout.strip()
        return distribution, digest
    started = time.monotonic()
    with tempfile.TemporaryDirectory(prefix="runtime-publish.", dir=output) as raw:
        scratch = Path(raw)
        priority = scratch / "priority"
        ini = stable_paths(texmf)[2].relative_to(texmf)
        layers = [
            ("latex-base", "tex/latex/base", frozenset()),
            ("l3kernel", "tex/latex/l3kernel", frozenset()),
            ("latex-ini", str(ini), frozenset()),
            ("latex", "tex/latex", frozenset()),
            ("generic", "tex/generic", frozenset()),
            ("plain", "tex/plain", frozenset()),
            ("other-tex", "tex", frozenset({"latex", "generic", "plain", "latex-dev"})),
        ]
        layers.extend((f"font-{area}", f"fonts/{area}", frozenset()) for area in ("tfm", "afm", "enc", "map", "opentype", "pk", "type1", "truetype", "vf"))
        count = 0
        root_layers: list[tuple[str, Path]] = []
        for label, area, skipped in layers:
            source = texmf / area
            if not source.is_dir():
                continue
            tree = scratch / label
            count += hardlink_tree(source, tree / area, skip_top_level=skipped)
            root_layers.append((label, tree))
        for source in priority_paths:
            relative = source.relative_to(texmf)
            target = priority / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            os.link(source, target)
        language = priority / "tex/generic/config/language.dat"
        language.parent.mkdir(parents=True, exist_ok=True)
        language.unlink(missing_ok=True)
        shutil.copyfile(config, language)
        generated_map = priority / "fonts/map/pdftex/updmap/pdftex.map"
        generated_map.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(pdftex_map, generated_map)
        roots = []
        for label, tree in [("format-input-priority", priority), *root_layers]:
            digest = subprocess.run([str(publisher), "--tree-ahash64", str(tree)], capture_output=True, text=True, check=True).stdout.strip()
            roots.append({"name": label, "path": str(tree), "treeAhash64": digest})
        configuration = scratch / "publish.json"
        atomic_json(configuration, {"schema": 8, "distribution": f"texlive-{year}-stable-runtime", "objectsBaseUrl": "https://example.invalid/texlive/objects/", "shardBits": 12, "roots": roots, "formats": []})
        staged = scratch / "publication"
        log = output / "runtime-publication.log"
        with log.open("wb") as stream:
            result = subprocess.run([str(publisher), str(configuration), str(staged)], stdout=stream, stderr=subprocess.STDOUT, check=False)
        if result.returncode:
            raise FormatPreparationError(f"full selected-year runtime publication failed; see {log}")
        subprocess.run([str(publisher), "--verify-sharded", str(staged)], check=True, stdout=subprocess.DEVNULL)
        os.replace(staged, distribution)
    digest = subprocess.run([str(publisher), "--file-ahash64", str(manifest)], capture_output=True, text=True, check=True).stdout.strip()
    atomic_json(output / "runtime-distribution.json", {"schema": 3, "year": year, "selection": "generated-config,generated-fontmaps,latex-base,l3kernel,latex-ini,latex,generic,plain,other-tex,fonts", "source_receipt_sha256": sha256(acquisition), "language_dat_sha256": sha256(config), "pdftex_map_sha256": sha256(pdftex_map), "priority_paths": [str(path.relative_to(texmf)) for path in priority_paths], "runfiles_linked": count, "manifest_sha256": sha256(manifest), "manifest_ahash64": digest, "elapsed_seconds": round(time.monotonic() - started, 3)})
    return distribution, digest


def umber_input_areas(texmf: Path, config: Path, reference: dict[str, object]) -> list[Path]:
    base, kernel, ini = stable_paths(texmf)
    areas = [config, base, kernel, ini]
    for record in reference["inputs"]:
        path = Path(str(record["path"]))
        if path.is_relative_to(texmf) and path.parent not in areas and path.parent != texmf / "web2c":
            areas.append(path.parent)
    return areas


def umber_environment(texmf: Path, config: Path, reference: dict[str, object], epoch: int, work: Path) -> dict[str, str]:
    areas = umber_input_areas(texmf, config, reference)
    font_areas = [Path(str(record["path"])).parent for record in reference["inputs"] if "/fonts/tfm/" in str(record["path"])]
    env = os.environ.copy()
    for name in ("TEXINPUTS", "TEXFONTS", "TFMFONTS", "SOURCE_DATE_EPOCH", "FORCE_SOURCE_DATE"):
        env.pop(name, None)
    env.update({
        "LC_ALL": "C",
        "SOURCE_DATE_EPOCH": str(epoch),
        "FORCE_SOURCE_DATE": "1",
        "TEXINPUTS": os.pathsep.join(str(path) for path in areas),
        "TEXFONTS": os.pathsep.join(str(path) for path in dict.fromkeys(font_areas)),
        "UMBER_ENGINE_FUEL": "10000000000",
    })
    return env


def verify_umber_inputs(reference: dict[str, object], admission: Path, publisher: Path, texmf: Path, config: Path, engine: str) -> list[dict[str, object]]:
    """Bind consumed inputs to common reference bytes or selected-source paths."""
    import latex_input_admissions

    authorized: dict[str, latex_input_admissions.Identity] = {}
    for record in reference["inputs"]:
        path = Path(str(record["path"]))
        if not (path.is_relative_to(texmf / "tex") or path.is_relative_to(texmf / "fonts/tfm") or path.is_relative_to(config)):
            continue
        key = f"{'tfm' if path.suffix == '.tfm' else 'tex'}:{path.name}"
        digest = subprocess.run([str(publisher), "--file-ahash64", str(path)], capture_output=True, text=True, check=True).stdout.strip()
        identity = (int(record["bytes"]), digest)
        if key in authorized and authorized[key] != identity:
            raise FormatPreparationError(f"reference inputs have conflicting request key {key}")
        authorized[key] = identity
    main, files = latex_input_admissions.read_receipt(admission)
    if authorized.get(f"tex:{engine}.ini") != main:
        raise FormatPreparationError(f"Umber {engine} main input differs from selected reference source")
    extras: list[dict[str, object]] = []
    areas = umber_input_areas(texmf, config, reference)
    for status, key, observed in files:
        if status != "used":
            continue
        if key in authorized:
            if authorized[key] != observed:
                raise FormatPreparationError(f"Umber consumed {key} with bytes different from clean reference")
            continue
        kind, name = key.split(":", 1)
        if kind != "tex" or Path(name).name != name:
            raise FormatPreparationError(f"Umber consumed unrecognized selected-source input {key}")
        matches = [area / name for area in areas if (area / name).is_file()]
        if not matches:
            raise FormatPreparationError(f"Umber consumed {key} outside selected source search areas")
        path = matches[0]
        if path.is_symlink() or not (path.is_relative_to(texmf) or path.is_relative_to(config)):
            raise FormatPreparationError(f"Umber consumed unsafe selected-source input {path}")
        digest = subprocess.run([str(publisher), "--file-ahash64", str(path)], capture_output=True, text=True, check=True).stdout.strip()
        if observed != (path.stat().st_size, digest):
            raise FormatPreparationError(f"Umber consumed {key} with bytes different from selected source {path}")
        extras.append({"key": key, "path": str(path), "bytes": path.stat().st_size, "sha256": sha256(path), "ahash64": digest})
    return extras


def build_umber(repo: Path, texmf: Path, config: Path, binary: Path, publisher: Path, engine: str, epoch: int, output: Path, reference: dict[str, object], distribution: Path, distribution_digest: str, year: int, *, reuse_existing_capture: bool = False) -> dict[str, object]:
    work = output / f"umber-{engine}-work"
    work.mkdir(parents=True, exist_ok=True)
    env = umber_environment(texmf, config, reference, epoch, work)
    fmt = work / f"{engine}.fmt"
    admission = work / "build.inputs"
    entry = stable_paths(texmf)[2] / f"{engine}.ini"
    arguments = ["run", f"--{engine}", "--distribution", str(distribution), "--distribution-ahash64", distribution_digest, "--offline", str(entry), "--format-out", str(fmt), "--input-records-out", str(admission)]
    if reuse_existing_capture:
        if not fmt.is_file() or not admission.is_file() or not (work / "terminal.txt").is_file() or (work / "stderr.txt").read_bytes():
            raise FormatPreparationError(f"no successful {engine} capture available to audit")
    else:
        fmt.unlink(missing_ok=True)
        admission.unlink(missing_ok=True)
        run_guarded(repo, binary, arguments, cwd=work, env=env, stdout=work / "terminal.txt", stderr=work / "stderr.txt")
    if not fmt.is_file() or fmt.read_bytes()[:8] != b"UMBRFMT\0":
        raise FormatPreparationError(f"Umber omitted valid {engine} native format")
    if "! " in (work / "terminal.txt").read_text(encoding="utf-8", errors="replace"):
        raise FormatPreparationError(f"Umber {engine} format emitted a TeX diagnostic")
    extras = verify_umber_inputs(reference, admission, publisher, texmf, config, engine)
    published = output / f"umber-{engine}.fmt"
    atomic_copy(fmt, published)
    receipt = {
        "schema": 1,
        "year": year,
        "engine": engine,
        "binary_sha256": sha256(binary),
        "capture_reused": reuse_existing_capture,
        "arguments": arguments,
        "source_date_epoch": epoch,
        "format": {"bytes": published.stat().st_size, "sha256": sha256(published)},
        "distribution_ahash64": distribution_digest,
        "distribution_manifest_sha256": sha256(distribution / "manifest.json"),
        "input_admissions": str(admission),
        "input_admissions_sha256": sha256(admission),
        "selected_source_inputs_beyond_reference": extras,
        "texinputs": env["TEXINPUTS"].split(os.pathsep),
        "texfonts": env["TEXFONTS"].split(os.pathsep) if env["TEXFONTS"] else [],
    }
    atomic_json(output / f"umber-{engine}.json", receipt)
    return receipt


def publish_prepared_year(year: int, snapshot_root: Path, output_root: Path, reference_binary: Path, publisher: Path, preparation_path: Path | None = None) -> dict[str, object]:
    """Publish one complete, source-bound year without disturbing other years."""
    acquisition = snapshot_root / "acquisition.json"
    source = json.loads(acquisition.read_text(encoding="utf-8"))
    tlpdb = snapshot_root / "tlpkg/texlive.tlpdb.xz"
    if source.get("year") != year or tlpdb_release_year(tlpdb) != year:
        raise FormatPreparationError(f"selected {year} snapshot has conflicting upstream release authority")
    epoch = source_epoch(source)
    output = output_root / str(year)
    formats: dict[str, object] = {}
    for engine in ("latex", "pdflatex"):
        reference_format = output / f"reference-{engine}.fmt"
        reference_receipt = output / f"reference-{engine}.json"
        umber_format = output / f"umber-{engine}.fmt"
        umber_receipt = output / f"umber-{engine}.json"
        ref = json.loads(reference_receipt.read_text(encoding="utf-8"))
        native = json.loads(umber_receipt.read_text(encoding="utf-8"))
        if ref["engine"]["sha256"] != sha256(reference_binary) or ref["format"]["sha256"] != sha256(reference_format):
            raise FormatPreparationError(f"stale {year} {engine} reference format receipt")
        for record in ref["inputs"]:
            source_path = Path(str(record["path"]))
            if not (source_path.is_relative_to(snapshot_root / "texmf-dist") or source_path.is_relative_to(output / "generated-config")):
                raise FormatPreparationError(f"foreign {year} {engine} reference input: {source_path}")
            if (source_path.is_symlink() or not source_path.is_file() or source_path.stat().st_size != record["bytes"]
                    or sha256(source_path) != record["sha256"]):
                raise FormatPreparationError(f"stale {year} {engine} reference input: {source_path}")
        if native["format"]["sha256"] != sha256(umber_format) or native["source_date_epoch"] != epoch:
            raise FormatPreparationError(f"stale {year} {engine} Umber format receipt")
        arguments = native["arguments"]
        distribution = Path(arguments[arguments.index("--distribution") + 1])
        digest = subprocess.run([str(publisher), "--file-ahash64", str(distribution / "manifest.json")], capture_output=True, text=True, check=True).stdout.strip()
        if digest != native["distribution_ahash64"]:
            raise FormatPreparationError(f"stale {year} {engine} selected-release distribution")
        formats[engine] = {
            "reference_binary": str(reference_binary),
            "reference_format": str(reference_format),
            "reference_format_receipt": str(reference_receipt),
            "umber_format": str(umber_format),
            "umber_format_sha256": native["format"]["sha256"],
            "umber_format_receipt": str(umber_receipt),
            "source_date_epoch": epoch,
        }
    runtime_distribution, runtime_digest = publish_full_runtime(year, snapshot_root, output, publisher)
    runtime_manifest = runtime_distribution / "manifest.json"
    for row in formats.values():
        row["umber_distribution"] = str(runtime_distribution)
        row["distribution_ahash64"] = runtime_digest
        row["distribution_manifest_sha256"] = sha256(runtime_manifest)
    fontmaps = json.loads((output / "fontmaps.json").read_text(encoding="utf-8"))
    fontmaps_receipt = output / f"fontmaps-{fontmaps['recipe_sha256'][:24]}.json"
    if json.loads(fontmaps_receipt.read_text(encoding="utf-8")) != fontmaps:
        raise FormatPreparationError(f"selected font-map receipt changed: {fontmaps_receipt}")
    texlive_fontmaps.verify_fontmaps(snapshot_root, fontmaps)
    fontmaps["receipt"] = str(fontmaps_receipt)
    fontmaps["receipt_sha256"] = sha256(fontmaps_receipt)
    row = {"runtime_root": str(snapshot_root / "texmf-dist"), "runtime_receipt": str(acquisition), "fontmaps": fontmaps, "formats": formats}
    receipt_path = preparation_path or output_root / "preparation.json"
    prepared: dict[str, object] = {}
    if receipt_path.is_file():
        previous = json.loads(receipt_path.read_text(encoding="utf-8"))
        if previous.get("schema") != 1 or not isinstance(previous.get("years"), dict):
            raise FormatPreparationError(f"invalid existing preparation receipt: {receipt_path}")
        prepared.update(previous["years"])
    prepared[str(year)] = row
    atomic_json(receipt_path, {"schema": 1, "years": prepared})
    return row


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--years", help="comma-separated selected years; defaults to all snapshot years")
    parser.add_argument("--snapshot-root", type=Path, default=Path("target/texlive-years"), help="read-only acquired snapshot root")
    parser.add_argument("--output-root", type=Path, default=Path("target/texlive-formats"), help="generated format and receipt root")
    parser.add_argument("--preparation", type=Path, help="preparation receipt destination; defaults to OUTPUT_ROOT/preparation.json")
    parser.add_argument("--runtime-only", action="store_true", help="reuse verified format images and publish updated runtime configuration")
    parser.add_argument("--reference-binary", type=Path, required=True)
    parser.add_argument("--kpsewhich", type=Path, default=Path("third_party/texlive-source/build-pdftex14029-20260301/texk/kpathsea/kpsewhich"), help="built kpathsea lookup utility from the project TeX Live source")
    parser.add_argument("--umber", type=Path, help="Umber binary, required unless --runtime-only")
    parser.add_argument("--publisher", type=Path, required=True)
    args = parser.parse_args()
    repo = Path(__file__).resolve().parent.parent
    cache = args.snapshot_root.resolve()
    output_root = args.output_root.resolve()
    try:
        import texlive_snapshot

        years = [int(piece) for piece in args.years.split(",")] if args.years else sorted(texlive_snapshot.SNAPSHOT_DATES)
        if not years or len(set(years)) != len(years):
            raise FormatPreparationError("--years must contain distinct years")
        for year in years:
            if year not in texlive_snapshot.SNAPSHOT_DATES:
                raise FormatPreparationError(f"unsupported format year: {year}")
        if not args.runtime_only and args.umber is None:
            raise FormatPreparationError("--umber is required unless --runtime-only is used")
        binaries = (args.reference_binary, args.publisher, args.kpsewhich)
        if args.umber is not None and not args.runtime_only:
            binaries += (args.umber,)
        for binary in binaries:
            if not binary.resolve().is_file():
                raise FormatPreparationError(f"missing binary: {binary}")
        version = subprocess.run([str(args.reference_binary.resolve()), "--version"], capture_output=True, text=True, check=True).stdout.splitlines()[0]
        if "pdfTeX" not in version or "1.40.29" not in version:
            raise FormatPreparationError(f"reference binary is not pdfTeX 1.40.29: {version}")
        for year in years:
            snapshot = texlive_snapshot.verify_snapshot(year, cache)
            root = Path(snapshot.root).resolve()
            acquisition = Path(snapshot.receipt).resolve()
            source = json.loads(acquisition.read_text(encoding="utf-8"))
            if source.get("year") != year:
                raise FormatPreparationError(f"acquisition receipt year disagrees with selector {year}: {acquisition}")
            tlpdb = root / "tlpkg/texlive.tlpdb.xz"
            release_year = tlpdb_release_year(tlpdb)
            if release_year != year:
                raise FormatPreparationError(f"selected {year} snapshot contains upstream TeX Live {release_year}: {tlpdb}")
            epoch = source_epoch(source)
            texmf = root / "texmf-dist"
            stable_paths(texmf)
            output = output_root / str(year)
            output.mkdir(parents=True, exist_ok=True)
            config = output / "generated-config"
            config_receipt = language_dat(texmf, tlpdb, config / "language.dat")
            atomic_json(output / "generated-config.json", config_receipt)
            texlive_fontmaps.prepare_fontmaps(root, output, args.kpsewhich)
            if not args.runtime_only:
                distribution, digest = publish_local_support(texmf, args.publisher.resolve(), output, year)
                for engine in ("latex", "pdflatex"):
                    reference = build_reference(repo, texmf, config, args.reference_binary.resolve(), engine, epoch, output)
                    build_umber(repo, texmf, config, args.umber.resolve(), args.publisher.resolve(), engine, epoch, output, reference, distribution, digest, year)
            preparation = args.preparation.resolve() if args.preparation else None
            publish_prepared_year(year, root, output_root, args.reference_binary.resolve(), args.publisher.resolve(), preparation)
            print(f"prepared reference and Umber formats for TeX Live {year}: {output}")
    except (OSError, ValueError, subprocess.CalledProcessError, FormatPreparationError, texlive_fontmaps.FontMapError) as error:
        print(f"texlive_formats.py: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
