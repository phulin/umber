"""Recorder-audited clean-pdfTeX format construction."""

from __future__ import annotations

import hashlib
import json
import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

import texlive


class ReferenceFormatError(Exception):
    """The reference format could not be built from the locked closure."""


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as input_file:
        for chunk in iter(lambda: input_file.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def format_lock_records(
    repo_root: Path,
) -> tuple[dict[str, str], list[dict[str, object]]]:
    lock = repo_root / "tests/latex-source.lock"
    metadata: dict[str, str] = {}
    records: list[dict[str, object]] = []
    selected = {
        "source",
        "local",
        "pdflatex-source",
        "pdflatex-local",
        "pdftex-reference-source",
    }
    for number, raw_line in enumerate(
        lock.read_text(encoding="utf-8").splitlines(), 1
    ):
        fields = raw_line.split()
        if not fields or fields[0].startswith("#"):
            continue
        if fields[0] in {
            "distribution",
            "distribution_ahash64",
            "source_date_epoch",
        }:
            if len(fields) != 2 or fields[0] in metadata:
                raise ReferenceFormatError(
                    f"{lock}:{number}: malformed format metadata"
                )
            metadata[fields[0]] = fields[1]
            continue
        if fields[0] not in selected:
            continue
        if len(fields) != 4:
            raise ReferenceFormatError(
                f"{lock}:{number}: malformed format input record"
            )
        relative = texlive._safe_relative(fields[1], label="format input path")
        try:
            length = int(fields[2])
        except ValueError as error:
            raise ReferenceFormatError(
                f"{lock}:{number}: invalid format input length"
            ) from error
        identity = texlive.Identity(length, fields[3])
        records.append(
            {
                "kind": fields[0],
                "path": relative.as_posix(),
                "bytes": identity.bytes,
                "sha256": identity.digest,
            }
        )
    required = {"distribution", "distribution_ahash64", "source_date_epoch"}
    if metadata.keys() < required:
        raise ReferenceFormatError(f"{lock}: missing format metadata")
    if not records:
        raise ReferenceFormatError(f"{lock}: no selected format input records")
    return metadata, records


def stage_input_root(
    repo_root: Path,
    texmf_dist: Path,
    destination: Path,
    records: list[dict[str, object]],
) -> set[Path]:
    staged: set[Path] = set()
    for record in records:
        kind = str(record["kind"])
        relative = Path(str(record["path"]))
        identity = texlive.Identity(int(record["bytes"]), str(record["sha256"]))
        if kind in {"source", "pdflatex-source", "pdftex-reference-source"}:
            source = texmf_dist / relative
            staged_relative = relative
        else:
            source = repo_root / relative
            staged_relative = Path("tex") / relative.name
        texlive.verify_file(source, identity, "sha256", "locked format input")
        if staged_relative in staged:
            raise ReferenceFormatError(
                f"duplicate staged format input path: {staged_relative}"
            )
        staged.add(staged_relative)
        output = destination / staged_relative
        output.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(source, output)
    return staged


def publish_generated_file(source: Path, destination: Path) -> None:
    destination.parent.mkdir(parents=True, exist_ok=True)
    descriptor, temporary_name = tempfile.mkstemp(
        prefix=f".{destination.name}.", dir=destination.parent
    )
    temporary = Path(temporary_name)
    try:
        with os.fdopen(descriptor, "wb") as output, source.open("rb") as input_file:
            shutil.copyfileobj(input_file, output)
            output.flush()
            os.fsync(output.fileno())
        os.replace(temporary, destination)
    finally:
        temporary.unlink(missing_ok=True)


def build(
    repo_root: Path,
    target_dir: Path,
    *,
    texmf_dist: Path | None = None,
    pdftex: Path | None = None,
) -> dict[str, object]:
    """Build pdfTeX's format from the exact Umber pdfLaTeX input closure."""
    target_dir = target_dir.resolve()
    texmf_dist = (
        texmf_dist
        or repo_root / "third_party/texlive-20260301-texmf/texmf-dist"
    ).resolve()
    pdftex = (
        pdftex
        or target_dir / "pdftex14029-oracle/bin/umber-pdftex14029-oracle-clean"
    ).resolve()
    if not texmf_dist.is_dir():
        raise ReferenceFormatError(f"missing pinned texmf-dist root: {texmf_dist}")
    if not pdftex.is_file() or pdftex.is_symlink():
        raise ReferenceFormatError(f"missing clean pdfTeX 1.40.29 oracle: {pdftex}")
    metadata, records = format_lock_records(repo_root)
    output_root = target_dir / "pdftex14029-reference-format"
    output_root.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="build.", dir=output_root) as raw_temp:
        temporary = Path(raw_temp)
        staged = temporary / "texmf-dist"
        stage_input_root(repo_root, texmf_dist, staged, records)
        run = temporary / "run"
        generated = temporary / "generated"
        run.mkdir()
        for name in (
            "home",
            "config",
            "sysconfig",
            "var",
            "cache",
            "tmp",
            "fonts",
        ):
            (generated / name).mkdir(parents=True)
        source_date_epoch = metadata["source_date_epoch"]
        relative_staged = Path("../texmf-dist")
        relative_generated = Path("../generated")
        environment = {
            "HOME": str(relative_generated / "home"),
            "LC_ALL": "C",
            "TMPDIR": str(relative_generated / "tmp"),
            "SOURCE_DATE_EPOCH": source_date_epoch,
            "FORCE_SOURCE_DATE": "1",
            "TEXMFCNF": str(relative_staged / "web2c"),
            "WEB2C": str(relative_staged / "web2c"),
            "TEXMFROOT": str(relative_staged),
            "TEXMFDIST": str(relative_staged),
            "TEXMFLOCAL": str(relative_generated / "local"),
            "TEXMFHOME": str(relative_generated / "home"),
            "TEXMFSYSVAR": str(relative_generated / "sysvar"),
            "TEXMFSYSCONFIG": str(relative_generated / "sysconfig"),
            "TEXMFVAR": str(relative_generated / "var"),
            "TEXMFCONFIG": str(relative_generated / "config"),
            "TEXMFCACHE": str(relative_generated / "cache"),
            "VARTEXFONTS": str(relative_generated / "fonts"),
            "TEXINPUTS": f".:{relative_staged / 'tex'}//",
            "TEXFONTS": f"{relative_staged / 'fonts/tfm'}//",
            "TFMFONTS": f"{relative_staged / 'fonts/tfm'}//",
        }
        arguments = [
            "-ini",
            "-etex",
            "-enc",
            "-progname=pdflatex-dev",
            "-jobname=pdflatex",
            "-translate-file=cp227.tcx",
            "-recorder",
            "pdflatex.ini",
        ]
        command = [
            sys.executable,
            str(repo_root / "scripts/run-umber-guarded.py"),
            "--timeout-seconds",
            "120",
            "--max-rss-mib",
            "1536",
            "--term-grace-seconds",
            "2",
            "--",
            str(pdftex),
            *arguments,
        ]
        with (run / "terminal.txt").open("wb") as stdout, (
            run / "stderr.txt"
        ).open("wb") as stderr:
            try:
                subprocess.run(
                    command,
                    cwd=run,
                    env=environment,
                    stdout=stdout,
                    stderr=stderr,
                    check=True,
                )
            except (OSError, subprocess.CalledProcessError) as error:
                raise ReferenceFormatError(
                    f"clean pdfTeX format build failed: {error}"
                ) from error
        format_path = run / "pdflatex.fmt"
        recorder = run / "pdflatex.fls"
        if not format_path.is_file() or not recorder.is_file():
            raise ReferenceFormatError(
                "clean pdfTeX format build omitted its format or recorder"
            )
        allowed = {(staged / relative).resolve() for relative in stage_input_root_paths(records)}
        observed = validate_recorder(run, recorder, allowed)
        required_opened = {
            staged / "tex/latex/tex-ini-files/pdflatex.ini",
            staged / "tex/pdftexconfig.tex",
            staged / "tex/latex-dev/base/latex.ltx",
            staged
            / "tex/latex-dev/firstaid/latex2e-first-aid-for-external-files.ltx",
            staged / "web2c/texmf.cnf",
        }
        missing = sorted({path.resolve() for path in required_opened} - observed)
        if missing:
            rendered = ", ".join(path.name for path in missing)
            raise ReferenceFormatError(
                f"clean pdfTeX format omitted required inputs: {rendered}"
            )
        terminal = (run / "terminal.txt").read_text(encoding="utf-8")
        if str(relative_staged / "web2c/cp227.tcx") not in terminal:
            raise ReferenceFormatError(
                "clean pdfTeX format did not load the locked cp227.tcx"
            )
        receipt: dict[str, object] = {
            "schema": 1,
            "format": {
                "bytes": format_path.stat().st_size,
                "sha256": sha256_file(format_path),
            },
            "engine": {
                "name": "pdfTeX",
                "version": "1.40.29",
                "profile": "clean-initex-etex-eight-bit",
                "sha256": sha256_file(pdftex),
                "arguments": arguments,
            },
            "source": {
                "distribution": metadata["distribution"],
                "distributionAhash64": metadata["distribution_ahash64"],
                "sourceDateEpoch": int(source_date_epoch),
                "lockSha256": sha256_file(repo_root / "tests/latex-source.lock"),
                "records": records,
            },
            "recorder": {
                "uniqueLockedInputs": len(observed),
                "unusedAllowedInputs": sorted(
                    path.relative_to(staged).as_posix() for path in allowed - observed
                ),
            },
        }
        receipt_path = temporary / "pdflatex-format.json"
        receipt_path.write_text(
            json.dumps(receipt, indent=2, sort_keys=True) + "\n",
            encoding="utf-8",
        )
        publish_generated_file(format_path, output_root / "pdflatex.fmt")
        publish_generated_file(receipt_path, output_root / "pdflatex-format.json")
    return receipt


def stage_input_root_paths(records: list[dict[str, object]]) -> set[Path]:
    paths: set[Path] = set()
    for record in records:
        relative = Path(str(record["path"]))
        if record["kind"] in {"local", "pdflatex-local"}:
            relative = Path("tex") / relative.name
        paths.add(relative)
    return paths


def validate_recorder(run: Path, recorder: Path, allowed: set[Path]) -> set[Path]:
    observed: set[Path] = set()
    for raw_line in recorder.read_text(encoding="utf-8").splitlines():
        if not raw_line.startswith("INPUT "):
            continue
        path = Path(raw_line.removeprefix("INPUT "))
        if not path.is_absolute():
            path = run / path
        canonical = path.resolve()
        if canonical in {Path("/dev/null"), (run / "texsys.aux").resolve()}:
            continue
        if canonical not in allowed:
            raise ReferenceFormatError(
                "clean pdfTeX format opened input outside the locked closure: "
                f"{canonical}"
            )
        observed.add(canonical)
    return observed
