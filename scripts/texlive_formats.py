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
from pathlib import Path


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
    ini = texmf / "tex/latex/tex-ini-files"
    for path in (base / "latex.ltx", kernel / "expl3-code.tex", ini / "latex.ini", ini / "pdflatex.ini"):
        if not path.is_file():
            raise FormatPreparationError(f"missing stable format source: {path}")
    return base, kernel, ini


def reference_environment(texmf: Path, config: Path, epoch: int, work: Path) -> dict[str, str]:
    base, kernel, ini = stable_paths(texmf)
    paths = [config, base, kernel, ini, texmf / "tex/generic/babel", texmf / "tex/generic/tex-ini-files", texmf / "tex"]
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
    source = texmf / "tex/latex/tex-ini-files/latex.ini"
    target = staged / "tex/latex/tex-ini-files/latex.ini"
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


def umber_environment(texmf: Path, config: Path, reference: dict[str, object], epoch: int, work: Path) -> dict[str, str]:
    base, kernel, ini = stable_paths(texmf)
    areas = [config, base, kernel, ini]
    for record in reference["inputs"]:
        path = Path(str(record["path"]))
        if path.is_relative_to(texmf) and path.parent not in areas and path.parent != texmf / "web2c":
            areas.append(path.parent)
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


def build_umber(repo: Path, texmf: Path, config: Path, binary: Path, engine: str, epoch: int, output: Path, reference: dict[str, object], distribution: Path, distribution_digest: str, year: int) -> dict[str, object]:
    work = output / f"umber-{engine}-work"
    work.mkdir(parents=True, exist_ok=True)
    env = umber_environment(texmf, config, reference, epoch, work)
    fmt = work / f"{engine}.fmt"
    admission = work / "build.inputs"
    fmt.unlink(missing_ok=True)
    admission.unlink(missing_ok=True)
    arguments = ["run", f"--{engine}", "--distribution", str(distribution), "--distribution-ahash64", distribution_digest, "--offline", str(texmf / f"tex/latex/tex-ini-files/{engine}.ini"), "--format-out", str(fmt), "--input-records-out", str(admission)]
    run_guarded(repo, binary, arguments, cwd=work, env=env, stdout=work / "terminal.txt", stderr=work / "stderr.txt")
    if not fmt.is_file() or fmt.read_bytes()[:8] != b"UMBRFMT\0":
        raise FormatPreparationError(f"Umber omitted valid {engine} native format")
    if "! " in (work / "terminal.txt").read_text(encoding="utf-8", errors="replace"):
        raise FormatPreparationError(f"Umber {engine} format emitted a TeX diagnostic")
    published = output / f"umber-{engine}.fmt"
    atomic_copy(fmt, published)
    receipt = {
        "schema": 1,
        "year": year,
        "engine": engine,
        "binary_sha256": sha256(binary),
        "arguments": arguments,
        "source_date_epoch": epoch,
        "format": {"bytes": published.stat().st_size, "sha256": sha256(published)},
        "distribution_ahash64": distribution_digest,
        "distribution_manifest_sha256": sha256(distribution / "manifest.json"),
        "input_admissions": str(admission),
        "input_admissions_sha256": sha256(admission),
        "texinputs": env["TEXINPUTS"].split(os.pathsep),
        "texfonts": env["TEXFONTS"].split(os.pathsep) if env["TEXFONTS"] else [],
    }
    atomic_json(output / f"umber-{engine}.json", receipt)
    return receipt


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--years", help="comma-separated selected years; defaults to all snapshot years")
    parser.add_argument("--cache-root", type=Path, default=Path("target/texlive-years"))
    parser.add_argument("--reference-binary", type=Path, required=True)
    parser.add_argument("--umber", type=Path, required=True)
    parser.add_argument("--publisher", type=Path, required=True)
    parser.add_argument("--offline", action="store_true")
    args = parser.parse_args()
    repo = Path(__file__).resolve().parent.parent
    cache = args.cache_root.resolve()
    try:
        import texlive_snapshot

        years = [int(piece) for piece in args.years.split(",")] if args.years else sorted(texlive_snapshot.SNAPSHOT_DATES)
        if not years or len(set(years)) != len(years):
            raise FormatPreparationError("--years must contain distinct years")
        prepared: dict[str, object] = {}
        for year in years:
            if year not in texlive_snapshot.SNAPSHOT_DATES:
                raise FormatPreparationError(f"unsupported format year: {year}")
        for binary in (args.reference_binary, args.umber, args.publisher):
            if not binary.resolve().is_file():
                raise FormatPreparationError(f"missing binary: {binary}")
        version = subprocess.run([str(args.reference_binary.resolve()), "--version"], capture_output=True, text=True, check=True).stdout.splitlines()[0]
        if "pdfTeX" not in version or "1.40.29" not in version:
            raise FormatPreparationError(f"reference binary is not pdfTeX 1.40.29: {version}")
        for year in years:
            snapshot = texlive_snapshot.ensure_snapshot(year, cache, offline=args.offline)
            root = Path(snapshot.root).resolve()
            acquisition = Path(snapshot.receipt).resolve()
            source = json.loads(acquisition.read_text(encoding="utf-8"))
            if source.get("year") != year:
                raise FormatPreparationError(f"acquisition receipt year disagrees with selector {year}: {acquisition}")
            epoch = source_epoch(source)
            texmf = root / "texmf-dist"
            stable_paths(texmf)
            output = root / "formats"
            output.mkdir(parents=True, exist_ok=True)
            config = output / "generated-config"
            config_receipt = language_dat(texmf, root / "tlpkg/texlive.tlpdb.xz", config / "language.dat")
            atomic_json(output / "generated-config.json", config_receipt)
            distribution, digest = publish_local_support(texmf, args.publisher.resolve(), output, year)
            formats: dict[str, object] = {}
            for engine in ("latex", "pdflatex"):
                reference = build_reference(repo, texmf, config, args.reference_binary.resolve(), engine, epoch, output)
                umber = build_umber(repo, texmf, config, args.umber.resolve(), engine, epoch, output, reference, distribution, digest, year)
                formats[engine] = {
                    "reference_binary": str(args.reference_binary.resolve()),
                    "reference_format": str(output / f"reference-{engine}.fmt"),
                    "reference_format_receipt": str(output / f"reference-{engine}.json"),
                    "umber_distribution": str(distribution),
                    "distribution_ahash64": digest,
                    "umber_format": str(output / f"umber-{engine}.fmt"),
                    "umber_format_sha256": umber["format"]["sha256"],
                    "umber_format_receipt": str(output / f"umber-{engine}.json"),
                    "source_date_epoch": epoch,
                }
            prepared[str(year)] = {"runtime_root": str(texmf), "runtime_receipt": str(acquisition), "formats": formats}
            print(f"prepared reference and Umber formats for TeX Live {year}: {output}")
        atomic_json(cache / "preparation.json", {"schema": 1, "years": prepared})
    except (OSError, ValueError, subprocess.CalledProcessError, FormatPreparationError) as error:
        print(f"texlive_formats.py: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
