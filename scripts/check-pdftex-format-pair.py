#!/usr/bin/env python3
"""Prove that clean pdfTeX and Umber load the same pinned pdfLaTeX program."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import shutil
import subprocess
import sys
from pathlib import Path


class PairingError(Exception):
    """The two formats cannot be accepted as one LaTeX program pair."""


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as input_file:
        for chunk in iter(lambda: input_file.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def canonical(path: Path, role: str, *, directory: bool = False) -> Path:
    path = path.resolve()
    valid = path.is_dir() if directory else path.is_file()
    if not valid:
        raise PairingError(f"missing {role}: {path}")
    return path


def find_manifest(distribution: Path) -> Path:
    for name in ("manifest-v9.json", "manifest-v8.json", "manifest.json"):
        candidate = distribution / name
        if candidate.is_file():
            return candidate
    raise PairingError(f"distribution has no root manifest: {distribution}")


def run_guarded(
    guard: Path,
    command: list[str],
    directory: Path,
    environment: dict[str, str],
    stdout: Path,
    stderr: Path,
) -> None:
    with stdout.open("wb") as out, stderr.open("wb") as err:
        result = subprocess.run(
            [
                sys.executable,
                str(guard),
                "--timeout-seconds",
                "120",
                "--max-rss-mib",
                "1536",
                "--term-grace-seconds",
                "2",
                "--",
                *command,
            ],
            cwd=directory,
            env=environment,
            stdout=out,
            stderr=err,
            check=False,
        )
    if result.returncode != 0:
        raise PairingError(
            f"guarded {' '.join(command[:1])} probe exited {result.returncode}; "
            f"see {stdout} and {stderr}"
        )


def markers(path: Path) -> list[str]:
    prefixes = (
        "UMBER-FORMAT-PAIR-LATEX=",
        "UMBER-FORMAT-PAIR-THEOREM-KEY=",
    )
    found = [
        line.strip()
        for line in path.read_text(encoding="utf-8", errors="replace").splitlines()
        if line.startswith(prefixes)
    ]
    expected = [
        "UMBER-FORMAT-PAIR-LATEX=2026-06-01",
        "UMBER-FORMAT-PAIR-THEOREM-KEY=proposition",
    ]
    if found != expected:
        raise PairingError(f"unexpected format-pair markers in {path}: {found}")
    return found


def normalized_dvi(path: Path) -> bytes:
    pre = 247
    comment_len_offset = 14
    comment_offset = 15
    normalized_comment = b"umber normalized dvi banner"

    data = bytearray(path.read_bytes())
    if not data or data[0] != pre or len(data) <= comment_len_offset:
        raise PairingError(f"DVI is missing a valid preamble: {path}")
    comment_end = comment_offset + data[comment_len_offset]
    if comment_end > len(data):
        raise PairingError(f"DVI preamble comment is truncated: {path}")
    for index in range(comment_offset, comment_end):
        normalized_index = index - comment_offset
        data[index] = (
            normalized_comment[normalized_index]
            if normalized_index < len(normalized_comment)
            else ord(" ")
        )
    return bytes(data)


def parse_args(arguments: list[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repository", type=Path, default=Path.cwd())
    parser.add_argument("--reference-pdftex", type=Path)
    parser.add_argument("--reference-format", type=Path)
    parser.add_argument("--umber", type=Path)
    parser.add_argument("--umber-format", type=Path)
    parser.add_argument("--distribution", type=Path, required=True)
    parser.add_argument("--distribution-ahash64", required=True)
    parser.add_argument("--texmf-dist", type=Path)
    parser.add_argument("--output-dir", type=Path)
    return parser.parse_args(arguments)


def main(arguments: list[str] | None = None) -> int:
    args = parse_args(arguments)
    repository = canonical(args.repository, "repository", directory=True)
    target = repository / "target"
    reference_pdftex = canonical(
        args.reference_pdftex
        or target / "pdftex14029-oracle/bin/umber-pdftex14029-oracle-clean",
        "clean pdfTeX oracle",
    )
    reference_format = canonical(
        args.reference_format
        or target / "pdftex14029-reference-format/pdflatex.fmt",
        "clean pdfTeX format",
    )
    reference_receipt = canonical(
        reference_format.with_name("pdflatex-format.json"),
        "clean pdfTeX format receipt",
    )
    umber = canonical(args.umber or target / "debug/umber", "Umber binary")
    distribution = canonical(args.distribution, "distribution", directory=True)
    distribution_manifest = find_manifest(distribution)
    root = json.loads(distribution_manifest.read_text(encoding="utf-8"))
    format_record = root.get("formats", {}).get("pdflatex")
    if not isinstance(format_record, dict):
        raise PairingError("distribution has no pdflatex format record")
    if args.umber_format is None:
        object_name = format_record.get("object")
        if not isinstance(object_name, str):
            raise PairingError("distribution pdflatex record has no object")
        umber_format = distribution / "objects" / object_name
    else:
        umber_format = args.umber_format
    umber_format = canonical(umber_format, "Umber format")
    texmf_dist = canonical(
        args.texmf_dist
        or repository / "third_party/texlive-20260301-texmf/texmf-dist",
        "pinned texmf-dist",
        directory=True,
    )
    lock = canonical(repository / "tests/latex-source.lock", "LaTeX source lock")
    lock_metadata = {
        fields[0]: fields[1]
        for line in lock.read_text(encoding="utf-8").splitlines()
        if len(fields := line.split()) == 2
    }
    receipt = json.loads(reference_receipt.read_text(encoding="utf-8"))
    if receipt.get("format", {}).get("sha256") != sha256(reference_format):
        raise PairingError("clean pdfTeX format differs from its receipt")
    if receipt.get("source", {}).get("lockSha256") != sha256(lock):
        raise PairingError("clean pdfTeX format receipt names a different source lock")
    for key, receipt_key in (
        ("distribution", "distribution"),
        ("distribution_ahash64", "distributionAhash64"),
    ):
        if receipt.get("source", {}).get(receipt_key) != lock_metadata.get(key):
            raise PairingError(f"clean pdfTeX receipt disagrees on {key}")
    if (
        format_record.get("sourceDistribution")
        != lock_metadata.get("distribution")
        or format_record.get("sourceManifestAhash64")
        != lock_metadata.get("distribution_ahash64")
    ):
        raise PairingError("Umber format record names a different source distribution")

    output = (args.output_dir or target / "pdftex-format-pair").resolve()
    if output.exists():
        raise PairingError(f"output directory already exists: {output}")
    reference_run = output / "reference"
    umber_run = output / "umber"
    reference_run.mkdir(parents=True)
    umber_run.mkdir(parents=True)
    probe = repository / "tests/latex/format-pairing.tex"
    shutil.copyfile(probe, reference_run / "format-pairing.tex")
    shutil.copyfile(probe, umber_run / "format-pairing.tex")
    guard = repository / "scripts/run-umber-guarded.py"
    epoch = lock_metadata["source_date_epoch"]
    texinputs = ":".join(
        (
            ".",
            f"{texmf_dist}/tex/latex-dev//",
            f"{texmf_dist}/tex/latex//",
            f"{texmf_dist}/tex/generic//",
        )
    )
    reference_environment = {
        "HOME": str(reference_run),
        "LC_ALL": "C",
        "SOURCE_DATE_EPOCH": epoch,
        "FORCE_SOURCE_DATE": "1",
        "TEXMFCNF": f"{texmf_dist.parent}:{texmf_dist / 'web2c'}",
        "TEXMFROOT": str(texmf_dist.parent),
        "TEXMFDIST": str(texmf_dist),
        "TEXINPUTS": texinputs,
        "TEXFONTS": f"{texmf_dist}/fonts/tfm//",
    }
    run_guarded(
        guard,
        [
            str(reference_pdftex),
            f"--fmt={reference_format}",
            "--progname=pdflatex-dev",
            "--output-format=dvi",
            "--interaction=nonstopmode",
            "--halt-on-error",
            "--jobname=format-pairing",
            "format-pairing.tex",
        ],
        reference_run,
        reference_environment,
        reference_run / "terminal.txt",
        reference_run / "stderr.txt",
    )
    umber_environment = os.environ.copy()
    umber_environment.update(
        {
            "LC_ALL": "C",
            "SOURCE_DATE_EPOCH": epoch,
            "FORCE_SOURCE_DATE": "1",
        }
    )
    run_guarded(
        guard,
        [
            str(umber),
            "run",
            "--pdflatex",
            "--format",
            str(umber_format),
            "--distribution",
            str(distribution),
            "--distribution-ahash64",
            args.distribution_ahash64,
            "--offline",
            "--dvi",
            "format-pairing.dvi",
            "format-pairing.tex",
        ],
        umber_run,
        umber_environment,
        umber_run / "terminal.txt",
        umber_run / "stderr.txt",
    )
    reference_markers = markers(reference_run / "terminal.txt")
    umber_markers = markers(umber_run / "terminal.txt")
    if reference_markers != umber_markers:
        raise PairingError("clean pdfTeX and Umber macro markers differ")
    reference_dvi = normalized_dvi(reference_run / "format-pairing.dvi")
    umber_dvi = normalized_dvi(umber_run / "format-pairing.dvi")
    if reference_dvi != umber_dvi:
        common = min(len(reference_dvi), len(umber_dvi))
        offset = next(
            (
                index
                for index in range(common)
                if reference_dvi[index] != umber_dvi[index]
            ),
            common,
        )
        reference_byte = reference_dvi[offset] if offset < len(reference_dvi) else None
        umber_byte = umber_dvi[offset] if offset < len(umber_dvi) else None
        raise PairingError(
            "clean pdfTeX and Umber normalized DVI differ at byte "
            f"{offset}: reference={reference_byte} Umber={umber_byte} "
            f"(lengths {len(reference_dvi)} and {len(umber_dvi)})"
        )
    marker_bytes = ("\n".join(reference_markers) + "\n").encode()
    dvi_sha256 = hashlib.sha256(reference_dvi).hexdigest()
    result = {
        "schema": 1,
        "sourceLockSha256": sha256(lock),
        "distribution": root.get("distribution"),
        "distributionRootSha256": sha256(distribution_manifest),
        "reference": {
            "binarySha256": sha256(reference_pdftex),
            "formatSha256": sha256(reference_format),
        },
        "umber": {
            "binarySha256": sha256(umber),
            "formatSha256": sha256(umber_format),
        },
        "macroMarkers": reference_markers,
        "macroFingerprintSha256": hashlib.sha256(marker_bytes).hexdigest(),
        "normalizedDviSha256": dvi_sha256,
    }
    (output / "pairing.json").write_text(
        json.dumps(result, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )
    print(
        "pdfTeX/Umber format pairing: PASS "
        f"macro-sha256={result['macroFingerprintSha256']} "
        f"dvi-sha256={dvi_sha256} receipt={output / 'pairing.json'}"
    )
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, ValueError, json.JSONDecodeError, PairingError) as error:
        print(f"check-pdftex-format-pair.py: {error}", file=sys.stderr)
        raise SystemExit(1)
