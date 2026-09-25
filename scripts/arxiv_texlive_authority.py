"""Authenticate the declared TeX Live year and exact format/runtime closure."""

from __future__ import annotations

import datetime as dt
import json
import os
import re
from pathlib import Path

from arxiv_corpus import sha256_file
from texlive import ahash64_file

SUPPORTED_ENGINES = ("latex", "pdflatex")


def fail(message: str) -> None:
    raise SystemExit(message)


def identity(path: Path) -> dict[str, object]:
    return {"path": str(path), "bytes": path.stat().st_size, "sha256": sha256_file(path)}


def authority(preparation: Path, rows: list[dict]) -> tuple[dict, dict]:
    from texlive_snapshot import verify_snapshot
    from texlive_reference_runtime import verify_reference_runtime
    from texlive_fontmaps import verify_fontmaps

    receipt = json.loads(preparation.read_text())
    if receipt.get("schema") != 1 or not isinstance(receipt.get("years"), dict):
        fail("invalid TeX Live preparation receipt")
    needed = {(row["year"], row["compiler"]) for row in rows
              if row["compiler"] in SUPPORTED_ENGINES}
    selected: dict[str, dict] = {}
    verified_years = set()
    for year, engine in sorted(needed):
        year_record = receipt["years"].get(year)
        if not isinstance(year_record, dict):
            fail(f"prepared TeX Live year is missing: {year}")
        runtime = Path(year_record["runtime_root"])
        runtime_receipt = Path(year_record["runtime_receipt"])
        fmt = year_record.get("formats", {}).get(engine)
        if not isinstance(fmt, dict):
            fail(f"prepared {year} {engine} format is missing")
        paths = {name: Path(fmt[name]) for name in (
            "reference_binary", "reference_format", "reference_format_receipt",
            "umber_distribution", "umber_format", "umber_format_receipt")}
        if not runtime.is_dir() or not runtime_receipt.is_file():
            fail(f"prepared {year} runtime is missing")
        if year not in verified_years:
            snapshot = verify_snapshot(int(year), runtime.parent.parent)
            if (snapshot.root.resolve() != runtime.parent.resolve()
                    or snapshot.receipt.resolve() != runtime_receipt.resolve()
                    or runtime.name != "texmf-dist"):
                fail(f"prepared {year} runtime path differs from verified snapshot")
            verified_years.add(year)
        for name in ("reference_binary", "reference_format", "reference_format_receipt",
                     "umber_format", "umber_format_receipt"):
            if not paths[name].is_file():
                fail(f"prepared {year} {engine} {name} is missing")
        if not os.access(paths["reference_binary"], os.X_OK):
            fail(f"prepared {year} {engine} reference binary is not executable")
        manifest = paths["umber_distribution"] / "manifest.json"
        if not manifest.is_file() or json.loads(manifest.read_text()).get("schema") != 8:
            fail(f"prepared {year} {engine} distribution manifest is missing")
        digest = fmt["distribution_ahash64"]
        if not isinstance(digest, str) or not re.fullmatch(r"[0-9a-f]{16}", digest):
            fail(f"invalid prepared {year} {engine} distribution digest")
        if ahash64_file(manifest) != digest:
            fail(f"prepared {year} {engine} distribution digest changed")
        umber_format_hash = sha256_file(paths["umber_format"])
        if fmt.get("distribution_manifest_sha256") != sha256_file(manifest):
            fail(f"prepared {year} {engine} distribution manifest changed")
        umber_format_receipt = json.loads(paths["umber_format_receipt"].read_text())
        if (fmt.get("umber_format_sha256") != umber_format_hash
                or umber_format_receipt.get("format", {}).get("sha256") != umber_format_hash):
            fail(f"prepared {year} {engine} Umber format differs from receipt")
        format_receipt = json.loads(paths["reference_format_receipt"].read_text())
        if (format_receipt.get("format", {}).get("sha256") != sha256_file(paths["reference_format"])
                or format_receipt.get("engine", {}).get("sha256") != sha256_file(paths["reference_binary"])):
            fail(f"prepared {year} {engine} reference format receipt differs")
        arguments = format_receipt.get("engine", {}).get("arguments")
        if (not isinstance(arguments, list) or f"-progname={engine}" not in arguments
                or f"-jobname={engine}" not in arguments):
            fail(f"prepared {year} {engine} reference format engine differs")
        year_receipt = json.loads(runtime_receipt.read_text())
        snapshot_date = year_receipt.get("snapshot_date")
        try:
            epoch = int(dt.datetime.combine(dt.date.fromisoformat(snapshot_date), dt.time(),
                                            dt.timezone.utc).timestamp())
        except (TypeError, ValueError) as error:
            fail(f"prepared {year} snapshot date is invalid: {error}")
        if (fmt.get("source_date_epoch") != epoch
                or format_receipt.get("source_date_epoch") != epoch
                or umber_format_receipt.get("source_date_epoch") != epoch
                or umber_format_receipt.get("engine") != engine):
            fail(f"prepared {year} {engine} format source clock or engine differs")
        inputs = format_receipt.get("inputs")
        if not isinstance(inputs, list) or not inputs:
            fail(f"prepared {year} {engine} reference input list is missing")
        format_root = paths["reference_format"].parent.resolve()
        generated_config = format_root / "generated-config"
        allowed = (runtime.resolve(), generated_config)
        seen_inputs = set()
        for record in inputs:
            if not isinstance(record, dict) or not isinstance(record.get("path"), str):
                fail(f"prepared {year} {engine} reference input record is invalid")
            path = Path(record["path"])
            if (not path.is_absolute() or not any(path.resolve().is_relative_to(root) for root in allowed)
                    or "latex-dev" in path.parts or not path.is_file()
                    or record.get("bytes") != path.stat().st_size
                    or record.get("sha256") != sha256_file(path)):
                fail(f"prepared {year} {engine} reference input differs: {path}")
            seen_inputs.add(path.resolve())
        required_inputs = {runtime / "tex/latex/base/latex.ltx",
                           runtime / "tex/latex/l3kernel/expl3-code.tex",
                           generated_config / "language.dat"}
        if not {path.resolve() for path in required_inputs} <= seen_inputs:
            fail(f"prepared {year} {engine} stable LaTeX inputs are missing")
        admission = Path(str(umber_format_receipt.get("input_admissions", "")))
        if (not admission.is_file()
                or umber_format_receipt.get("input_admissions_sha256") != sha256_file(admission)):
            fail(f"prepared {year} {engine} Umber input admissions differ")
        view = verify_reference_runtime(runtime.parent, year_record["reference_runtime"])
        fontmaps = verify_fontmaps(runtime.parent, year_record["fontmaps"])
        key = f"{year}/{engine}"
        selected[key] = {"runtime_root": str(runtime), "runtime_receipt": identity(runtime_receipt),
                         "generated_config": str(generated_config),
                         "reference_runtime": year_record["reference_runtime"],
                         "reference_view": str(view),
                         "fontmaps": year_record["fontmaps"],
                         "generated_fontmaps": str(fontmaps),
                         "reference_binary": identity(paths["reference_binary"]),
                         "reference_format": identity(paths["reference_format"]),
                         "reference_format_receipt": identity(paths["reference_format_receipt"]),
                         "umber_distribution": str(paths["umber_distribution"]),
                         "distribution_manifest": identity(manifest),
                         "distribution_ahash64": digest,
                         "source_date_epoch": epoch,
                         "umber_format": identity(paths["umber_format"]),
                         "umber_format_receipt": identity(paths["umber_format_receipt"])}
    return receipt, selected

