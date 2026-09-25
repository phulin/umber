#!/usr/bin/env python3
"""Independent, opt-in MuPDF raster/text comparison of recorded arXiv PDFs."""
from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
import math
import re
import resource
import subprocess
import sys
from pathlib import Path

SCHEMA = "arxiv-pdf-render-v1"
DPI = 144
MAX_PAGE_PIXELS = 32_000_000


def identity(path: Path) -> dict:
    with path.open("rb") as source:
        return {"bytes": path.stat().st_size, "sha256": hashlib.file_digest(source, "sha256").hexdigest()}


def check_artifact(record: dict) -> Path:
    path = Path(record["path"])
    observed = identity(path)
    if any(record.get(key) != value for key, value in observed.items()):
        raise ValueError(f"PDF differs from corpus receipt: {path}")
    return path


def pixel_difference(left: bytes, right: bytes, width: int) -> dict:
    if len(left) != len(right):
        return {"different_raster_sizes": True}
    count = delta = 0
    min_x, min_y, max_x, max_y = width, len(left) // (3 * width), 0, 0
    for offset in range(0, len(left), 3):
        if left[offset:offset + 3] == right[offset:offset + 3]:
            continue
        y, x = divmod(offset // 3, width)
        count += 1
        min_x, min_y = min(min_x, x), min(min_y, y)
        max_x, max_y = max(max_x, x + 1), max(max_y, y + 1)
        delta = max(delta, *(abs(left[offset + c] - right[offset + c]) for c in range(3)))
    return ({"changed_pixels": count, "bounds": [min_x, min_y, max_x, max_y],
             "max_channel_delta": delta} if count else {"changed_pixels": 0})



def compare(reference: Path, umber: Path) -> dict:
    import pymupdf

    pymupdf.TOOLS.mupdf_display_errors(False)
    pymupdf.TOOLS.mupdf_display_warnings(False)
    report = {"schema": SCHEMA, "dpi": DPI, "consumer": list(pymupdf.version),
              "reference": identity(reference), "umber": identity(umber), "pages": [],
              "first_raster_difference": None}
    with pymupdf.open(reference) as expected, pymupdf.open(umber) as actual:
        for label, document in (("reference", expected), ("Umber", actual)):
            if document.is_repaired:
                raise ValueError(f"MuPDF repaired the {label} PDF; comparison is not a validity pass")
        if len(expected) < 1 or len(actual) < 1:
            raise ValueError("empty PDF")
        report["reference_pages"], report["umber_pages"] = len(expected), len(actual)
        for number, (left, right) in enumerate(zip(expected, actual), 1):
            records, samples = [], []
            for page in (left, right):
                rect = page.rect
                if (not all(math.isfinite(v) for v in rect) or rect.width <= 0 or rect.height <= 0
                        or math.ceil(rect.width * 2) * math.ceil(rect.height * 2) > MAX_PAGE_PIXELS):
                    raise ValueError(f"page {number} exceeds raster geometry budget")
                pixmap = page.get_pixmap(matrix=pymupdf.Matrix(2, 2), colorspace=pymupdf.csRGB,
                                        alpha=False, annots=True)
                samples.append(pixmap.samples)
                records.append({"media_box": list(page.mediabox), "crop_box": list(page.cropbox),
                                "rotation": page.rotation, "width": pixmap.width, "height": pixmap.height,
                                "rgb_sha256": hashlib.sha256(samples[-1]).hexdigest(),
                                "text_sha256": hashlib.sha256(page.get_text().encode()).hexdigest()})
            a, b = records
            raster_equal = all(a[key] == b[key] for key in a if key != "text_sha256")
            if not raster_equal and report["first_raster_difference"] is None:
                report["first_raster_difference"] = {"page": number,
                    **(pixel_difference(samples[0], samples[1], a["width"])
                       if (a["width"], a["height"]) == (b["width"], b["height"])
                       else {"different_raster_sizes": True})}
            report["pages"].append({"page": number, "reference": a, "umber": b,
                                    "raster_equal": raster_equal,
                                    "text_equal": a["text_sha256"] == b["text_sha256"]})
        same_count = len(expected) == len(actual)
        report["raster_equal"] = same_count and all(p["raster_equal"] for p in report["pages"])
        report["text_equal"] = same_count and all(p["text_equal"] for p in report["pages"])
        report["status"] = "equal" if report["raster_equal"] and report["text_equal"] else "different"
    return report


def eligible_pair(row: dict) -> tuple[Path, Path] | None:
    sides = [row.get(name, {}) for name in ("reference", "umber")]
    if any(side.get("exit_status") != 0 or not side.get("pdf") for side in sides):
        return None
    return tuple(check_artifact(side["pdf"]) for side in sides)


def summarize(reports: list[dict]) -> dict:
    counts = {status: sum(r["status"] == status for r in reports)
              for status in sorted({r["status"] for r in reports})}
    qualified = len(reports) - counts.get("ineligible", 0)
    verdict = "FAIL" if counts.get("different", 0) or counts.get("error", 0) else (
        "PARTIAL" if counts.get("unavailable", 0) or not qualified else "PASS")
    return {"counts": counts, "verdict": verdict, "reference_qualified_rows": qualified,
            "compared_rows": counts.get("equal", 0) + counts.get("different", 0),
            "raster_equal_rows": sum(r.get("raster_equal", False) for r in reports),
            "text_equal_rows": sum(r.get("text_equal", False) for r in reports)}


def corpus_rows(results: Path) -> list[tuple[Path, dict]]:
    summary = json.loads((results / "summary.json").read_text())
    rows = [(path, json.loads(path.read_text()))
            for path in sorted((results / "rows").glob("*/result.json"))]
    expected = summary.get("sample_rows")
    qualified = sum(row.get("reference", {}).get("status") == "PDF-success" for _, row in rows)
    if (summary.get("output_format") != "pdf" or not isinstance(expected, int) or expected < 1
            or len(rows) != expected or summary.get("reference_rows_recorded") != expected
            or qualified != summary.get("pdf_eligible_rows")):
        raise ValueError("corpus coverage differs from summary; finish and verify reference qualification first")
    return rows


def memory_limit() -> None:
    resource.setrlimit(resource.RLIMIT_AS, (1536 * 1024 * 1024,) * 2)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--results", type=Path)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--pair", nargs=2, type=Path, help=argparse.SUPPRESS)
    args = parser.parse_args()
    if args.pair:
        try:
            report = compare(*args.pair)
        except Exception as error:
            report = {"schema": SCHEMA, "status": "error", "error": str(error)}
        print(json.dumps(report, sort_keys=True))
        return 2 if report["status"] == "error" else 0
    if not args.results or not args.output:
        parser.error("--results and --output are required")
    if importlib.util.find_spec("pymupdf") is None:
        parser.error("PyMuPDF is required; run with the Python environment documented in docs/arxiv_dvi_cohort.md")
    if args.output.exists() and any(args.output.iterdir()):
        parser.error("output directory already contains evidence; choose a new --output directory")
    rows = corpus_rows(args.results)
    args.output.mkdir(parents=True, exist_ok=True)
    script = Path(__file__).resolve()
    reports = []
    for receipt, row in rows:
        if not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9._-]*", row["id"]) or row["id"] != receipt.parent.name:
            raise ValueError("invalid corpus row identifier")
        pair = eligible_pair(row)
        if pair is None:
            status = "unavailable" if row.get("reference", {}).get("status") == "PDF-success" else "ineligible"
            report = {"status": status, "corpus_status": row["status"]}
        else:
            try:
                completed = subprocess.run([sys.executable, str(script), "--pair", *map(str, pair)],
                                           capture_output=True, text=True, timeout=120,
                                           preexec_fn=memory_limit, check=False)
                report = json.loads(completed.stdout)
                if report.get("schema") != SCHEMA or completed.returncode not in (0, 2):
                    raise ValueError(f"invalid consumer result: exit {completed.returncode}")
                if report["status"] != "error":
                    for name, path in zip(("reference", "umber"), pair):
                        if report[name] != identity(path):
                            raise ValueError("consumer input identity differs")
            except (ValueError, subprocess.TimeoutExpired) as error:
                report = {"status": "error", "error": str(error)}
        report.update(id=row["id"], corpus_receipt=identity(receipt))
        (args.output / f"{row['id']}.json").write_text(json.dumps(report, indent=2, sort_keys=True) + "\n")
        reports.append(report)
        print(row["id"], report["status"], flush=True)
    if not reports:
        raise ValueError("no corpus receipts found")
    summary = {"schema": SCHEMA, "dpi": DPI, "script": identity(script), "rows": len(reports),
               **summarize(reports)}
    (args.output / "summary.json").write_text(json.dumps(summary, indent=2, sort_keys=True) + "\n")
    print(json.dumps(summary, sort_keys=True))
    return {"PASS": 0, "FAIL": 1, "PARTIAL": 4}[summary["verdict"]]


if __name__ == "__main__":
    raise SystemExit(main())
