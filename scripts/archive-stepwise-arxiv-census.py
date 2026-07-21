#!/usr/bin/env python3
"""Archive a guarded stepwise arXiv census as immutable JSON artifacts."""

import argparse
import csv
import hashlib
import json
import subprocess
from collections import defaultdict
from pathlib import Path


REFERENCE_UNCLEAN = {
    "0809.4370": "external_publisher_input",
    "quant-ph/0401158": "external_publisher_input",
    "astro-ph/9806267": "external_publisher_input",
    "1607.01424": "external_publisher_input",
    "1706.07482": "external_publisher_input",
    "hep-ph/0604209": "external_publisher_input",
    "1307.4678": "incomplete_source_bundle",
    "0906.4556": "reference_dvi_eps_workflow",
    "1209.1157": "reference_dvi_eps_workflow",
    "astro-ph/0701678": "reference_dvi_eps_workflow",
    "1309.6552": "reference_legacy_input_rejected",
    "1402.7313": "reference_legacy_input_rejected",
    "1501.07105": "reference_legacy_input_rejected",
    "1111.4238": "reference_package_era_incompatibility",
    "1806.00133": "reference_package_era_incompatibility",
    "1605.00321": "reference_package_era_incompatibility",
    "2109.04844": "reference_package_era_incompatibility",
    "0903.3324": "reference_document_error",
    "1512.00596": "reference_document_error",
    "1512.07679": "reference_document_error",
    "2001.09659": "reference_document_error",
}

BLOCKERS = {
    "umber2-65ku.61": {"2606.04708", "2409.09687", "2302.05666", "2509.18247", "1505.01466", "2408.08969"},
    "umber2-65ku.62": {"0901.0375", "cond-mat/0403603", "2212.09995", "2504.13286", "2406.18174", "math/0405422", "2509.15093", "2306.06689", "2606.13617"},
    "umber2-65ku.63": {"1910.12506", "1901.02462"},
    "umber2-65ku.64": {"1806.09227", "2210.17134"},
    "umber2-65ku.65": {"1803.04459", "2002.08666", "2601.15540", "2309.07749", "2007.04915", "2411.08642", "2301.10807", "2512.01214"},
    "umber2-65ku.66": {"2412.02819", "2502.08039", "2106.16088", "2008.12745"},
    "umber2-65ku.67": {"2511.00922", "2304.07531", "2509.26198", "2012.14835"},
    "umber2-65ku.68": {"2405.07680"},
    "umber2-65ku.69": {"1910.14440"},
    "umber2-65ku.70": {"1903.09682"},
}


def digest(path: Path) -> str:
    value = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            value.update(chunk)
    return value.hexdigest()


def last_nonempty(path: Path) -> str:
    if not path.exists():
        return ""
    return next((line for line in reversed(path.read_text(errors="replace").splitlines()) if line.strip()), "")


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("results", type=Path)
    parser.add_argument("output", type=Path)
    parser.add_argument("--sample", type=Path, required=True)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--format", type=Path, required=True)
    parser.add_argument("--distribution", type=Path, required=True)
    parser.add_argument("--mode", choices=("warm", "offline"), required=True)
    args = parser.parse_args()

    blocker_for = {paper: issue for issue, papers in BLOCKERS.items() for paper in papers}
    rows = []
    with (args.results / "summary.tsv").open(newline="") as source:
        summary = {row["id"]: row for row in csv.DictReader(source, delimiter="\t")}
    with args.sample.open(newline="") as source:
        sample = list(csv.DictReader(source, delimiter="\t"))
    if len(sample) != 100 or len(summary) != 100 or set(summary) != {row["id"] for row in sample}:
        raise SystemExit("census must contain each of the 100 sample ids exactly once")

    clusters = defaultdict(list)
    for ordinal, sample_row in enumerate(sample, 1):
        paper = sample_row["id"]
        row = summary[paper]
        complete = row["engine_status"] == "accepted" and row["finalizer_status"] == "complete"
        reference_class = REFERENCE_UNCLEAN.get(paper)
        blocker = None if complete or reference_class else blocker_for.get(paper)
        if not complete and not reference_class and blocker is None:
            raise SystemExit(f"reference-clean unresolved row lacks blocker: {paper}")
        cluster = (
            "pdf_complete"
            if complete and reference_class is None
            else f"umber_complete_{reference_class}"
            if complete
            else reference_class
            if reference_class
            else blocker
        )
        key = paper.replace("/", "_")
        engine_log = args.results / f"{key}.engine.log"
        final_log = args.results / f"{key}.finalizer.log"
        pdf = args.results / f"{key}.pdf"
        record = {
            "ordinal": ordinal,
            "id": paper,
            "category": sample_row["categories"],
            "reference_clean": reference_class is None,
            "reference_class": reference_class,
            "engine_status": row["engine_status"],
            "finalizer_status": row["finalizer_status"],
            "guard_status": int(row["guard_status"]),
            "cumulative_fuel": int(row["cumulative_fuel"]),
            "peak_rss_limit_mib": 1536,
            "timeout_seconds": 120,
            "blocker": blocker,
            "cluster": cluster,
            "engine_terminal": last_nonempty(engine_log),
            "finalizer_terminal": last_nonempty(final_log),
            "engine_log_sha256": digest(engine_log),
            "finalizer_log_sha256": digest(final_log) if final_log.exists() else None,
            "pdf_sha256": digest(pdf) if pdf.exists() else None,
        }
        rows.append(record)
        clusters[cluster].append(paper)

    args.output.mkdir(parents=True, exist_ok=True)
    commit = subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip()
    metadata = {
        "git_commit": commit,
        "mode": args.mode,
        "rows": 100,
        "engine_fuel": 100000000,
        "max_rss_mib": 1536,
        "timeout_seconds": 120,
        "binary_sha256": digest(args.binary),
        "format_sha256": digest(args.format),
        "distribution_manifest_sha256": digest(args.distribution / "manifest.json"),
        "sample_sha256": digest(args.sample),
    }
    (args.output / "metadata.json").write_text(json.dumps(metadata, indent=2, sort_keys=True) + "\n")
    with (args.output / "results.jsonl").open("w") as output:
        for row in rows:
            output.write(json.dumps(row, sort_keys=True) + "\n")
    cluster_receipt = {"accounted": 100, "clusters": {key: {"count": len(ids), "ids": ids} for key, ids in sorted(clusters.items())}}
    (args.output / "clusters.json").write_text(json.dumps(cluster_receipt, indent=2, sort_keys=True) + "\n")
    (args.output / "summary.tsv").write_bytes((args.results / "summary.tsv").read_bytes())


if __name__ == "__main__":
    main()
